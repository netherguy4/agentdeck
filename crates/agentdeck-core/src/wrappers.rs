//! Per-instance terminal wrappers (`claude@second`) and `.desktop` launchers.
//!
//! A wrapper is a small POSIX sh script that pins the instance's home dir
//! (`CLAUDE_CONFIG_DIR` for Claude, `CODEX_HOME` for Codex) and execs the
//! harness binary, so each account starts from a terminal without agentdeck.
//! Every file agentdeck writes carries a marker (a comment line in scripts,
//! `X-Agentdeck-Instance` in desktop files); install refuses to overwrite
//! files without our marker, and remove only deletes files with it.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::changes::{Change, plan_write_file, plan_write_script};
use crate::registry::{Harness, Instance};

/// Script comment line; the full marker is this prefix plus the instance id.
pub const MARKER_PREFIX: &str = "# managed by agentdeck: ";
/// Desktop file key carrying the owning instance id.
pub const DESKTOP_KEY: &str = "X-Agentdeck-Instance";

/// `~/.local/bin`, or `$AGENTDECK_BIN_DIR` when set (tests inject temp dirs).
pub fn bin_dir(home: &Path) -> PathBuf {
    std::env::var_os("AGENTDECK_BIN_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".local/bin"))
}

/// `~/.local/share/applications`, or `$AGENTDECK_APPLICATIONS_DIR` when set.
pub fn apps_dir(home: &Path) -> PathBuf {
    std::env::var_os("AGENTDECK_APPLICATIONS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".local/share/applications"))
}

/// The primary account of a harness already runs without an env override, so
/// it gets no wrapper: the id is the harness name itself (`codex`), or T3's
/// legacy default Claude id (`claudeAgent`).
pub fn is_primary(inst: &Instance) -> bool {
    inst.id == inst.harness.as_str()
        || (inst.harness == Harness::Claude && inst.id == "claudeAgent")
}

/// Default wrapper file name (`claude_second` -> `claude@second`), or `None`
/// for the primary instance. Only claude/codex harnesses are supported (the
/// opencode/agy login model needs its own adapter first).
pub fn default_wrapper_name(inst: &Instance) -> anyhow::Result<Option<String>> {
    check_harness(inst)?;
    if is_primary(inst) {
        return Ok(None);
    }
    let base = inst.harness.as_str();
    let mut suffix = inst.id.as_str();
    for prefix in [format!("{base}_"), format!("{base}-")] {
        if let Some(rest) = suffix.strip_prefix(&prefix) {
            suffix = rest;
            break;
        }
    }
    if suffix.is_empty() {
        anyhow::bail!(
            "cannot derive a wrapper name from instance id {:?}",
            inst.id
        );
    }
    Ok(Some(format!("{base}@{suffix}")))
}

/// Resolve the wrapper file name: `--name` override wins, else the default.
/// `None` means the instance is primary and gets no wrapper.
pub fn resolve_wrapper_name(
    inst: &Instance,
    name_override: Option<&str>,
) -> anyhow::Result<Option<String>> {
    check_harness(inst)?;
    if let Some(name) = name_override {
        if name.is_empty() || name.contains('/') || name.contains('\0') || name.contains('\n') {
            anyhow::bail!("bad wrapper name {name:?}: must be a plain file name");
        }
        return Ok(Some(name.to_string()));
    }
    default_wrapper_name(inst)
}

fn check_harness(inst: &Instance) -> anyhow::Result<()> {
    match inst.harness {
        Harness::Claude | Harness::Codex => Ok(()),
        other => anyhow::bail!(
            "wrappers for harness {} are not supported yet (instance {:?})",
            other.as_str(),
            inst.id
        ),
    }
}

fn env_var(inst: &Instance) -> &'static str {
    match inst.harness {
        Harness::Claude => "CLAUDE_CONFIG_DIR",
        Harness::Codex => "CODEX_HOME",
        // Checked by check_harness before any caller gets here.
        Harness::Opencode | Harness::Agy => unreachable!(),
    }
}

fn binary(inst: &Instance) -> String {
    inst.binary
        .clone()
        .unwrap_or_else(|| inst.harness.as_str().to_string())
}

/// Single-quote a path for POSIX sh (`'` -> `'\''`).
fn sh_quote(path: &Path) -> String {
    format!("'{}'", path.display().to_string().replace('\'', "'\\''"))
}

/// Render the wrapper script. `home` must already be tilde-expanded.
pub fn script_contents(inst: &Instance, home: &Path) -> anyhow::Result<Vec<u8>> {
    check_harness(inst)?;
    Ok(format!(
        "#!/bin/sh\n{MARKER_PREFIX}{id}\nexport {var}={home}\nexec {bin} \"$@\"\n",
        id = inst.id,
        var = env_var(inst),
        home = sh_quote(home),
        bin = binary(inst),
    )
    .into_bytes())
}

/// Render the `.desktop` launcher for an installed wrapper script.
pub fn desktop_contents(inst: &Instance, wrapper_path: &Path) -> Vec<u8> {
    let name = if inst.display_name.is_empty() {
        &inst.id
    } else {
        &inst.display_name
    };
    format!(
        "[Desktop Entry]\nName={name}\nExec={exec}\nTerminal=true\nType=Application\n{DESKTOP_KEY}={id}\n",
        exec = wrapper_path.display(),
        id = inst.id,
    )
    .into_bytes()
}

fn marker_line(id: &str) -> String {
    format!("{MARKER_PREFIX}{id}")
}

fn desktop_line(id: &str) -> String {
    format!("{DESKTOP_KEY}={id}")
}

/// True when `bytes` contain exactly the `marker` line.
fn has_line(bytes: &[u8], marker: &str) -> bool {
    bytes
        .split(|b| *b == b'\n')
        .map(|l| l.strip_suffix(b"\r").unwrap_or(l))
        .any(|l| l == marker.as_bytes())
}

fn read_existing(path: &Path) -> Option<Vec<u8>> {
    // Cap the read: marker lines sit at the top of our small generated files.
    let meta = fs::symlink_metadata(path).ok()?;
    if meta.file_type().is_dir() || meta.len() > 64 * 1024 {
        return None;
    }
    fs::read(path).ok()
}

/// Plan installing the wrapper (+ desktop file) for a registry instance.
/// Pure reads plus returned `Change`s; the caller applies them (or prints
/// them for `--dry-run`). `home` must already be tilde-expanded.
pub fn plan_install(
    inst: &Instance,
    home: &Path,
    bin_dir: &Path,
    apps_dir: &Path,
    name_override: Option<&str>,
    no_desktop: bool,
) -> anyhow::Result<Vec<Change>> {
    let Some(name) = resolve_wrapper_name(inst, name_override)? else {
        anyhow::bail!(
            "instance {:?} is the primary {} account and needs no wrapper",
            inst.id,
            inst.harness.as_str()
        );
    };
    let wrapper_path = bin_dir.join(&name);
    let script = script_contents(inst, home)?;
    if let Some(old) = read_existing(&wrapper_path)
        && !has_line(&old, &marker_line(&inst.id))
    {
        anyhow::bail!(
            "refusing to overwrite {}: not managed by agentdeck for {:?}",
            wrapper_path.display(),
            inst.id
        );
    }
    let mut changes = Vec::new();
    let old = read_existing(&wrapper_path);
    changes.extend(plan_write_script(
        wrapper_path.clone(),
        old.as_deref(),
        &script,
    ));
    if !no_desktop {
        let desktop_path = apps_dir.join(format!("{name}.desktop"));
        let desktop = desktop_contents(inst, &wrapper_path);
        if let Some(old) = read_existing(&desktop_path)
            && !has_line(&old, &desktop_line(&inst.id))
        {
            anyhow::bail!(
                "refusing to overwrite {}: not managed by agentdeck for {:?}",
                desktop_path.display(),
                inst.id
            );
        }
        validate_desktop(&desktop)?;
        let old = read_existing(&desktop_path);
        changes.extend(plan_write_file(desktop_path, old.as_deref(), &desktop));
    }
    Ok(changes)
}

/// Plan removing every wrapper/desktop file carrying this instance's marker
/// (found by scanning both dirs, so `--name` renames are caught too).
pub fn plan_remove(id: &str, bin_dir: &Path, apps_dir: &Path) -> anyhow::Result<Vec<Change>> {
    let mut changes = Vec::new();
    changes.extend(
        files_with_line(bin_dir, &marker_line(id), None)
            .into_iter()
            .map(|path| Change::Remove { path }),
    );
    changes.extend(
        files_with_line(apps_dir, &desktop_line(id), Some("desktop"))
            .into_iter()
            .map(|path| Change::Remove { path }),
    );
    if changes.is_empty() {
        anyhow::bail!("no managed wrappers found for {id:?}");
    }
    Ok(changes)
}

fn files_with_line(dir: &Path, line: &str, extension: Option<&str>) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if extension.is_some_and(|ext| path.extension().is_none_or(|e| e != ext)) {
            continue;
        }
        if entry.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        if read_existing(&path).is_some_and(|b| has_line(&b, line)) {
            out.push(path);
        }
    }
    out.sort();
    out
}

/// Validate desktop content with `desktop-file-validate` when installed.
pub fn validate_desktop(contents: &[u8]) -> anyhow::Result<()> {
    if std::process::Command::new("desktop-file-validate")
        .arg("--version")
        .output()
        .is_err()
    {
        return Ok(());
    }
    static N: AtomicU64 = AtomicU64::new(0);
    let tmp = std::env::temp_dir().join(format!(
        "agentdeck-desktop-validate-{}-{}.desktop",
        std::process::id(),
        N.fetch_add(1, Ordering::SeqCst)
    ));
    fs::write(&tmp, contents)?;
    let out = std::process::Command::new("desktop-file-validate")
        .arg(&tmp)
        .output();
    let _ = fs::remove_file(&tmp);
    let out = out?;
    if !out.status.success() {
        let detail = String::from_utf8_lossy(&out.stderr);
        anyhow::bail!("generated .desktop file failed validation: {detail}");
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileState {
    Missing,
    Ours,
    Foreign,
}

impl FileState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Missing => "missing",
            Self::Ours => "installed",
            Self::Foreign => "foreign",
        }
    }
}

#[derive(Debug, Clone)]
pub struct WrapperStatus {
    pub id: String,
    pub name: Option<String>,
    pub wrapper: FileState,
    pub desktop: FileState,
}

fn file_state(path: &Path, line: &str) -> FileState {
    match read_existing(path) {
        None if fs::symlink_metadata(path).is_err() => FileState::Missing,
        // Exists but unreadable/too big/a dir: not ours, never touch it.
        None => FileState::Foreign,
        Some(old) if has_line(&old, line) => FileState::Ours,
        Some(_) => FileState::Foreign,
    }
}

/// One row per registry instance for `wrapper list`.
pub fn status(
    instances: &[Instance],
    bin_dir: &Path,
    apps_dir: &Path,
) -> anyhow::Result<Vec<WrapperStatus>> {
    let mut rows = Vec::new();
    for inst in instances {
        let name = resolve_wrapper_name(inst, None).unwrap_or(None);
        let Some(ref name) = name else {
            rows.push(WrapperStatus {
                id: inst.id.clone(),
                name: None,
                wrapper: FileState::Missing,
                desktop: FileState::Missing,
            });
            continue;
        };
        let wrapper_path = bin_dir.join(name);
        let desktop_path = apps_dir.join(format!("{name}.desktop"));
        rows.push(WrapperStatus {
            id: inst.id.clone(),
            name: Some(name.clone()),
            wrapper: file_state(&wrapper_path, &marker_line(&inst.id)),
            desktop: file_state(&desktop_path, &desktop_line(&inst.id)),
        });
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::changes::apply_changes;
    use std::os::unix::fs::PermissionsExt;

    static N: AtomicU64 = AtomicU64::new(0);

    struct Guard(PathBuf);
    impl Drop for Guard {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    struct Fx {
        _g: Guard,
        bin: PathBuf,
        apps: PathBuf,
        home: PathBuf,
    }

    fn fixture() -> Fx {
        let dir = std::env::temp_dir().join(format!(
            "agentdeck-wrappers-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = fs::remove_dir_all(&dir);
        Fx {
            _g: Guard(dir.clone()),
            bin: dir.join("bin"),
            apps: dir.join("apps"),
            home: PathBuf::from("/fake/.claude-second"),
        }
    }

    fn inst(id: &str, harness: Harness) -> Instance {
        Instance {
            id: id.to_string(),
            harness,
            display_name: format!("{id} display"),
            home: "/fake/home".to_string(),
            binary: None,
            enabled: true,
            t3: true,
            extra: Default::default(),
        }
    }

    #[test]
    fn naming_and_content() {
        assert_eq!(
            default_wrapper_name(&inst("claude_second", Harness::Claude))
                .unwrap()
                .as_deref(),
            Some("claude@second")
        );
        assert_eq!(
            default_wrapper_name(&inst("codex-second", Harness::Codex))
                .unwrap()
                .as_deref(),
            Some("codex@second")
        );
        // No prefix: the whole id becomes the suffix.
        assert_eq!(
            default_wrapper_name(&inst("work", Harness::Claude))
                .unwrap()
                .as_deref(),
            Some("claude@work")
        );
        assert!(
            default_wrapper_name(&inst("claude", Harness::Claude))
                .unwrap()
                .is_none()
        );
        assert!(
            default_wrapper_name(&inst("claudeAgent", Harness::Claude))
                .unwrap()
                .is_none()
        );
        assert!(
            default_wrapper_name(&inst("codex", Harness::Codex))
                .unwrap()
                .is_none()
        );

        let script = String::from_utf8(
            script_contents(
                &inst("claude_second", Harness::Claude),
                &PathBuf::from("/h/me dir"),
            )
            .unwrap(),
        )
        .unwrap();
        assert!(
            script.starts_with("#!/bin/sh\n# managed by agentdeck: claude_second\n"),
            "{script}"
        );
        assert!(
            script.contains("export CLAUDE_CONFIG_DIR='/h/me dir'\n"),
            "{script}"
        );
        assert!(script.ends_with("exec claude \"$@\"\n"), "{script}");

        let codex = inst("codex_second", Harness::Codex);
        let script =
            String::from_utf8(script_contents(&codex, &PathBuf::from("/h/x")).unwrap()).unwrap();
        assert!(script.contains("export CODEX_HOME='/h/x'\n"), "{script}");

        let mut custom = inst("claude_second", Harness::Claude);
        custom.binary = Some("/opt/claude/bin/claude".to_string());
        let script =
            String::from_utf8(script_contents(&custom, &PathBuf::from("/h")).unwrap()).unwrap();
        assert!(
            script.contains("exec /opt/claude/bin/claude \"$@\"\n"),
            "{script}"
        );

        assert!(script_contents(&inst("o1", Harness::Opencode), &PathBuf::from("/h")).is_err());
    }

    #[test]
    fn install_idempotent_executable_and_desktop() {
        let fx = fixture();
        let claude = inst("claude_second", Harness::Claude);
        let changes = plan_install(&claude, &fx.home, &fx.bin, &fx.apps, None, false).unwrap();
        assert_eq!(changes.len(), 2);
        // Dry-run purity: planning writes nothing.
        assert!(!fx.bin.exists() && !fx.apps.exists());
        apply_changes(&changes).unwrap();

        let wrapper = fx.bin.join("claude@second");
        let mode = fs::metadata(&wrapper).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o755);
        let desktop = fs::read_to_string(fx.apps.join("claude@second.desktop")).unwrap();
        assert!(
            desktop.contains("Name=claude_second display\n"),
            "{desktop}"
        );
        assert!(
            desktop.contains(&format!("Exec={}\n", wrapper.display())),
            "{desktop}"
        );
        assert!(
            desktop.contains("Terminal=true\nType=Application\n"),
            "{desktop}"
        );
        assert!(
            desktop.contains("X-Agentdeck-Instance=claude_second\n"),
            "{desktop}"
        );

        assert!(
            plan_install(&claude, &fx.home, &fx.bin, &fx.apps, None, false)
                .unwrap()
                .is_empty()
        );
        // Primary instances refuse install.
        assert!(
            plan_install(
                &inst("claude", Harness::Claude),
                &fx.home,
                &fx.bin,
                &fx.apps,
                None,
                false
            )
            .is_err()
        );
    }

    #[test]
    fn refuse_foreign_files() {
        let fx = fixture();
        let claude = inst("claude_second", Harness::Claude);
        fs::create_dir_all(&fx.bin).unwrap();
        fs::write(fx.bin.join("claude@second"), "#!/bin/sh\necho foreign\n").unwrap();
        assert!(plan_install(&claude, &fx.home, &fx.bin, &fx.apps, None, false).is_err());

        fs::remove_file(fx.bin.join("claude@second")).unwrap();
        fs::create_dir_all(&fx.apps).unwrap();
        fs::write(
            fx.apps.join("claude@second.desktop"),
            "[Desktop Entry]\nName=x\n",
        )
        .unwrap();
        assert!(plan_install(&claude, &fx.home, &fx.bin, &fx.apps, None, false).is_err());
    }

    #[test]
    fn remove_only_ours() {
        let fx = fixture();
        let claude = inst("claude_second", Harness::Claude);
        let changes = plan_install(&claude, &fx.home, &fx.bin, &fx.apps, None, false).unwrap();
        apply_changes(&changes).unwrap();
        // Foreign files that must survive a remove.
        fs::write(fx.bin.join("other"), "foreign\n").unwrap();
        fs::write(fx.apps.join("other.desktop"), "[Desktop Entry]\n").unwrap();

        let changes = plan_remove("claude_second", &fx.bin, &fx.apps).unwrap();
        assert_eq!(changes.len(), 2);
        apply_changes(&changes).unwrap();
        assert!(!fx.bin.join("claude@second").exists());
        assert!(!fx.apps.join("claude@second.desktop").exists());
        assert!(fx.bin.join("other").exists());
        assert!(fx.apps.join("other.desktop").exists());

        assert!(plan_remove("claude_second", &fx.bin, &fx.apps).is_err());
        assert!(plan_remove("ghost", &fx.bin, &fx.apps).is_err());
    }

    #[test]
    fn name_override_and_no_desktop() {
        let fx = fixture();
        let claude = inst("claude_second", Harness::Claude);
        let changes =
            plan_install(&claude, &fx.home, &fx.bin, &fx.apps, Some("mywrap"), true).unwrap();
        assert_eq!(changes.len(), 1);
        apply_changes(&changes).unwrap();
        assert!(fx.bin.join("mywrap").exists());
        assert!(!fx.apps.exists());
        // Remove finds the renamed wrapper by marker scan.
        let changes = plan_remove("claude_second", &fx.bin, &fx.apps).unwrap();
        assert_eq!(changes.len(), 1);
        assert!(resolve_wrapper_name(&claude, Some("a/b")).is_err());
    }
}

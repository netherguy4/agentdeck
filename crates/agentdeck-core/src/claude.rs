//! Claude adapter: materialize and drift-check a secondary Claude Code account
//! from the shared primary profile (`~/.claude`).
//!
//! The primary's `settings.json`, `skills/`, `agents/`, `plugins/` and
//! `CLAUDE.md` live in `~/.claude/`, but its `.claude.json` lives one level
//! up at `$HOME/.claude.json`; callers pass that path explicitly as
//! `primary_json` (secondary instances keep `<home>/.claude.json`).
//!
//! `skills/`, `agents/`, `plugins/` and `CLAUDE.md` in a secondary home are
//! symlinks into the primary. `settings.json` is a real per-instance file:
//! the primary's keys are merged in, secondary-only keys are kept. In
//! `.claude.json` only the top-level `mcpServers` key is managed; every other
//! key (`oauthAccount`, `userID`, `projects`, ...) is left untouched and never
//! printed (account secrets).

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::{Map, Value};

use crate::changes::{Change, backup_dest, plan_write_file};

/// Entries in a secondary home that must be symlinks into the primary.
const LINKS: [&str; 4] = ["skills", "agents", "plugins", "CLAUDE.md"];

#[derive(Debug, Clone, Serialize)]
pub enum Drift {
    Symlink {
        link: PathBuf,
        expected: PathBuf,
        actual: Option<PathBuf>,
    },
    SettingsKey {
        key: String,
        primary: Option<Value>,
        secondary: Option<Value>,
    },
    /// Values are stored redacted (`env`/`headers` entries hidden).
    McpServers {
        primary: Option<Value>,
        secondary: Option<Value>,
    },
    /// The primary `$HOME/.claude.json` is missing, so `mcpServers` drift
    /// cannot be evaluated (a sync error; a check problem, never a skip).
    PrimaryConfigMissing { path: PathBuf },
}

impl Drift {
    pub fn describe(&self) -> String {
        match self {
            Drift::Symlink {
                link,
                expected,
                actual,
            } => match actual {
                Some(a) => format!(
                    "symlink {} points to {}, expected {}",
                    link.display(),
                    a.display(),
                    expected.display()
                ),
                None => format!(
                    "symlink {} missing or not a symlink, expected {}",
                    link.display(),
                    expected.display()
                ),
            },
            Drift::SettingsKey {
                key,
                primary,
                secondary,
            } => format!(
                "settings.json {key}: secondary={} primary={}",
                opt_json(secondary),
                opt_json(primary)
            ),
            Drift::McpServers { primary, secondary } => format!(
                "{}\n{}",
                "mcpServers differs:",
                diff_mcp_servers(
                    &secondary.clone().unwrap_or(Value::Null),
                    &primary.clone().unwrap_or(Value::Null)
                )
            ),
            Drift::PrimaryConfigMissing { path } => {
                format!("primary {} missing, mcpServers not managed", path.display())
            }
        }
    }
}

/// Plan the changes that make `home` a secondary account of `primary`.
/// `primary_json` is the primary's `.claude.json` (`$HOME/.claude.json`);
/// a missing one is an error.
/// Idempotent: a second run over applied state returns `[]`.
pub fn plan_materialize(
    primary: &Path,
    home: &Path,
    state_dir: &Path,
    primary_json: &Path,
) -> anyhow::Result<Vec<Change>> {
    let mut changes = Vec::new();
    if fs::symlink_metadata(home).is_err() {
        changes.push(Change::Mkdir {
            path: home.to_path_buf(),
        });
    }
    for name in LINKS {
        let link = home.join(name);
        let target = primary.join(name);
        match fs::symlink_metadata(&link) {
            Ok(m) if m.file_type().is_symlink() => {
                if fs::read_link(&link).ok() != Some(target.clone()) {
                    changes.push(Change::Symlink { link, target });
                }
            }
            // A real file/dir where a symlink belongs is backed up, never dropped.
            Ok(_) => {
                changes.push(Change::BackupRemove {
                    path: link.clone(),
                    backup: backup_dest(state_dir, name),
                });
                changes.push(Change::Symlink { link, target });
            }
            Err(e) if e.kind() == ErrorKind::NotFound => {
                changes.push(Change::Symlink { link, target });
            }
            Err(e) => return Err(e.into()),
        }
    }
    if let Some(c) = plan_settings_merge(primary, home, state_dir)? {
        changes.push(c);
    }
    if let Some(c) = plan_mcp_merge(primary_json, home, state_dir)? {
        changes.push(c);
    }
    Ok(changes)
}

/// List differences between `primary` and `home` without writing anything.
/// `primary_json` is the primary's `.claude.json`; a missing one is reported
/// as drift instead of silently skipping the `mcpServers` comparison.
pub fn check(primary: &Path, home: &Path, primary_json: &Path) -> anyhow::Result<Vec<Drift>> {
    let mut out = Vec::new();
    for name in LINKS {
        let link = home.join(name);
        let expected = primary.join(name);
        let actual = match fs::symlink_metadata(&link) {
            Ok(m) if m.file_type().is_symlink() => fs::read_link(&link).ok(),
            _ => None,
        };
        if actual != Some(expected.clone()) {
            out.push(Drift::Symlink {
                link,
                expected,
                actual,
            });
        }
    }
    let (_, pri_settings) = read_json_file(&primary.join("settings.json"))?;
    let (_, sec_settings) = read_json_file(&home.join("settings.json"))?;
    let pri = pri_settings.as_object();
    let sec = sec_settings.as_object();
    let mut keys: Vec<&String> = pri
        .into_iter()
        .flat_map(|m| m.keys())
        .chain(sec.into_iter().flat_map(|m| m.keys()))
        .collect();
    keys.sort();
    keys.dedup();
    for key in keys {
        let p = pri.and_then(|m| m.get(key)).cloned();
        let s = sec.and_then(|m| m.get(key)).cloned();
        if p != s {
            out.push(Drift::SettingsKey {
                key: key.clone(),
                primary: p,
                secondary: s,
            });
        }
    }
    if fs::symlink_metadata(primary_json).is_err() {
        out.push(Drift::PrimaryConfigMissing {
            path: primary_json.to_path_buf(),
        });
    } else {
        let (_, pri_file) = read_json_file(primary_json)?;
        let (_, sec_file) = read_json_file(&home.join(".claude.json"))?;
        let p = pri_file.get("mcpServers").cloned();
        let s = sec_file.get("mcpServers").cloned();
        if p != s {
            out.push(Drift::McpServers {
                primary: p.map(|v| redact_mcp_servers(&v)),
                secondary: s.map(|v| redact_mcp_servers(&v)),
            });
        }
    }
    Ok(out)
}

/// Dry-run printer: like [`Change::describe`], but a `.claude.json` write
/// shows only the redacted `mcpServers` diff; other keys hold account secrets.
pub fn describe_change(change: &Change) -> String {
    if let Change::WriteFile {
        path,
        contents,
        old,
        backup,
    } = change
        && path.file_name().is_some_and(|n| n == ".claude.json")
    {
        let new_mcp = mcp_of(contents);
        let old_mcp = old.as_deref().map(mcp_of).unwrap_or(Value::Null);
        let base = format!(
            "write {} (mcpServers only):\n{}",
            path.display(),
            diff_mcp_servers(&old_mcp, &new_mcp)
        );
        return match backup {
            Some(dest) => format!("{base} (backup {})", dest.display()),
            None => base,
        };
    }
    change.describe()
}

fn plan_settings_merge(
    primary: &Path,
    home: &Path,
    state_dir: &Path,
) -> anyhow::Result<Option<Change>> {
    let sec_path = home.join("settings.json");
    // A symlink here (older layouts) must become a real file, even when the
    // merged bytes would be identical.
    let is_link = fs::symlink_metadata(&sec_path).is_ok_and(|m| m.file_type().is_symlink());
    let (sec_bytes, sec) = read_json_file(&sec_path)?;
    let (_, pri) = read_json_file(&primary.join("settings.json"))?;
    let Some(pri) = pri.as_object() else {
        anyhow::bail!("primary settings.json is not a JSON object");
    };
    let Some(sec) = sec.as_object() else {
        anyhow::bail!("secondary settings.json is not a JSON object");
    };
    let mut merged = sec.clone();
    for (k, v) in pri {
        merged.insert(k.clone(), v.clone());
    }
    let new = to_pretty(&merged)?;
    match plan_write_file(sec_path.clone(), sec_bytes.as_deref(), &new) {
        Some(c) => Ok(Some(with_backup(state_dir, c))),
        None if is_link => Ok(Some(tracked_write(state_dir, sec_path, sec_bytes, new))),
        None => Ok(None),
    }
}

/// Merge the primary's `mcpServers` into the secondary `.claude.json`
/// (backup, every other key kept).
pub fn plan_mcp_merge(
    primary_json: &Path,
    home: &Path,
    state_dir: &Path,
) -> anyhow::Result<Option<Change>> {
    if fs::symlink_metadata(primary_json).is_err() {
        anyhow::bail!(
            "primary {} missing, cannot sync mcpServers",
            primary_json.display()
        );
    }
    let (_, pri) = read_json_file(primary_json)?;
    let Some(pri) = pri.as_object() else {
        anyhow::bail!("primary .claude.json is not a JSON object");
    };
    let (sec_bytes, sec) = read_json_file(&home.join(".claude.json"))?;
    let Some(sec) = sec.as_object() else {
        anyhow::bail!("secondary .claude.json is not a JSON object");
    };
    let mut merged = sec.clone();
    match pri.get("mcpServers") {
        Some(v) => {
            merged.insert("mcpServers".to_string(), v.clone());
        }
        None => {
            merged.remove("mcpServers");
        }
    }
    if merged == *sec {
        return Ok(None);
    }
    Ok(plan_write_file(
        home.join(".claude.json"),
        sec_bytes.as_deref(),
        &to_pretty(&merged)?,
    )
    .map(|c| with_backup(state_dir, c)))
}

/// Attach a state-dir backup destination to a `WriteFile` change.
fn with_backup(state_dir: &Path, change: Change) -> Change {
    match change {
        Change::WriteFile {
            path,
            contents,
            old,
            ..
        } => tracked_write(state_dir, path, old, contents),
        other => other,
    }
}

/// A `WriteFile` that backs up the existing file under `<state>/backups/`.
fn tracked_write(
    state_dir: &Path,
    path: PathBuf,
    old: Option<Vec<u8>>,
    contents: Vec<u8>,
) -> Change {
    let backup = path
        .file_name()
        .map(|n| backup_dest(state_dir, &n.to_string_lossy()));
    Change::WriteFile {
        path,
        contents,
        old,
        backup,
    }
}

/// Read a JSON file; a missing file is an empty object (nothing managed yet).
fn read_json_file(path: &Path) -> anyhow::Result<(Option<Vec<u8>>, Value)> {
    match fs::read(path) {
        Err(e) if e.kind() == ErrorKind::NotFound => Ok((None, Value::Object(Map::new()))),
        Err(e) => Err(e.into()),
        Ok(bytes) => {
            let value: Value = serde_json::from_slice(&bytes)
                .map_err(|e| anyhow::anyhow!("cannot parse {}: {e}", path.display()))?;
            Ok((Some(bytes), value))
        }
    }
}

/// Two-space pretty JSON with a trailing newline, matching Claude's own writer.
fn to_pretty(map: &Map<String, Value>) -> anyhow::Result<Vec<u8>> {
    let mut text = serde_json::to_string_pretty(&Value::Object(map.clone()))?;
    text.push('\n');
    Ok(text.into_bytes())
}

fn mcp_of(bytes: &[u8]) -> Value {
    serde_json::from_slice::<Value>(bytes)
        .ok()
        .and_then(|v| v.get("mcpServers").cloned())
        .unwrap_or(Value::Null)
}

/// Clone an `mcpServers` value with `env`/`headers` entries replaced by key
/// names only, so diffs never leak tokens.
pub fn redact_mcp_servers(value: &Value) -> Value {
    let Value::Object(servers) = value else {
        return value.clone();
    };
    let mut out = Map::new();
    for (name, server) in servers {
        if let Value::Object(fields) = server {
            let mut kept = fields.clone();
            for key in ["env", "headers"] {
                if let Some(Value::Object(entries)) = fields.get(key) {
                    kept.insert(
                        key.to_string(),
                        Value::Object(
                            entries
                                .keys()
                                .map(|k| (k.clone(), Value::String("<redacted>".into())))
                                .collect(),
                        ),
                    );
                }
            }
            out.insert(name.clone(), Value::Object(kept));
        } else {
            out.insert(name.clone(), server.clone());
        }
    }
    Value::Object(out)
}

/// Line-set diff of two redacted `mcpServers` values (same shortcut style as
/// `Change::describe`; files this describes stay small).
fn diff_mcp_servers(old: &Value, new: &Value) -> String {
    let old_text = serde_json::to_string_pretty(&redact_mcp_servers(old)).unwrap_or_default();
    let new_text = serde_json::to_string_pretty(&redact_mcp_servers(new)).unwrap_or_default();
    let old_lines: Vec<&str> = old_text.lines().collect();
    let new_lines: Vec<&str> = new_text.lines().collect();
    let mut out = vec!["mcpServers (env/headers values redacted):".to_string()];
    for line in &old_lines {
        if !new_lines.contains(line) {
            out.push(format!("-{line}"));
        }
    }
    for line in &new_lines {
        if !old_lines.contains(line) {
            out.push(format!("+{line}"));
        }
    }
    if out.len() == 1 && old != new {
        out.push("(only redacted values differ)".to_string());
    }
    out.join("\n")
}

fn opt_json(v: &Option<Value>) -> String {
    v.as_ref()
        .map(|v| serde_json::to_string(v).unwrap_or_else(|_| "?".into()))
        .unwrap_or_else(|| "(absent)".into())
}

/// Copy one `settings.json` key from the secondary home into the primary
/// profile (backup, other keys and their order kept); a later `sync`
/// propagates it to every secondary. Errors when the secondary lacks the
/// key. `Ok(None)` means the primary already holds that value.
pub fn import_settings_key(
    primary: &Path,
    home: &Path,
    state_dir: &Path,
    key: &str,
) -> anyhow::Result<Option<Change>> {
    if key.is_empty() {
        anyhow::bail!("settings key must not be empty");
    }
    let (pri_bytes, pri) = read_json_file(&primary.join("settings.json"))?;
    let (_, sec) = read_json_file(&home.join("settings.json"))?;
    let Some(pri_obj) = pri.as_object() else {
        anyhow::bail!("primary settings.json is not a JSON object");
    };
    let Some(sec_obj) = sec.as_object() else {
        anyhow::bail!("secondary settings.json is not a JSON object");
    };
    let Some(value) = sec_obj.get(key).cloned() else {
        anyhow::bail!("secondary settings.json has no key {key:?}");
    };
    // Clone-then-insert keeps every existing key and its order (serde_json
    // keeps insertion order with preserve_order); new keys append at the end.
    let mut merged = pri_obj.clone();
    if merged.get(key) == Some(&value) {
        return Ok(None);
    }
    merged.insert(key.to_string(), value);
    Ok(plan_write_file(
        primary.join("settings.json"),
        pri_bytes.as_deref(),
        &to_pretty(&merged)?,
    )
    .map(|c| with_backup(state_dir, c)))
}

/// Copy one `mcpServers` entry from the secondary `.claude.json` into the
/// primary one (backup, other servers kept); a later `sync` propagates it.
/// `Ok(None)` means the primary already holds that entry.
pub fn import_mcp_server(
    primary_json: &Path,
    home: &Path,
    state_dir: &Path,
    server: &str,
) -> anyhow::Result<Option<Change>> {
    if server.is_empty() {
        anyhow::bail!("mcp server name must not be empty");
    }
    if fs::symlink_metadata(primary_json).is_err() {
        anyhow::bail!(
            "primary {} missing, cannot import an mcpServer into it",
            primary_json.display()
        );
    }
    let (pri_bytes, pri) = read_json_file(primary_json)?;
    let (_, sec) = read_json_file(&home.join(".claude.json"))?;
    let Some(pri_obj) = pri.as_object() else {
        anyhow::bail!("primary .claude.json is not a JSON object");
    };
    let value = sec
        .get("mcpServers")
        .and_then(|v| v.as_object())
        .and_then(|m| m.get(server))
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("secondary .claude.json has no mcpServer {server:?}"))?;
    let mut merged = pri_obj.clone();
    let entry = merged
        .entry("mcpServers")
        .or_insert_with(|| Value::Object(Map::new()));
    let Some(servers) = entry.as_object_mut() else {
        anyhow::bail!("primary .claude.json mcpServers is not an object");
    };
    if servers.get(server) == Some(&value) {
        return Ok(None);
    }
    servers.insert(server.to_string(), value);
    Ok(plan_write_file(
        primary_json.to_path_buf(),
        pri_bytes.as_deref(),
        &to_pretty(&merged)?,
    )
    .map(|c| with_backup(state_dir, c)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::changes::apply_changes;
    use std::sync::atomic::{AtomicU64, Ordering};

    static N: AtomicU64 = AtomicU64::new(0);

    struct Guard(PathBuf);
    impl Drop for Guard {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    struct Fx {
        _g: Guard,
        primary: PathBuf,
        primary_json: PathBuf,
        home: PathBuf,
        state: PathBuf,
    }

    fn fixture() -> Fx {
        let dir = std::env::temp_dir().join(format!(
            "agentdeck-claude-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = fs::remove_dir_all(&dir);
        let primary = dir.join("primary");
        // The primary .claude.json lives at $HOME level, next to ~/.claude.
        let primary_json = dir.join(".claude.json");
        let home = dir.join("second");
        let state = dir.join("state");
        for name in ["skills", "agents", "plugins"] {
            fs::create_dir_all(primary.join(name)).unwrap();
        }
        fs::write(primary.join("skills/s.txt"), "s").unwrap();
        fs::write(primary.join("CLAUDE.md"), "me").unwrap();
        fs::write(primary.join("settings.json"), "{\"theme\":\"dark\"}\n").unwrap();
        fs::write(
            primary_json.clone(),
            "{\"oauthAccount\":{\"accountUuid\":\"P-UUID\"},\"userID\":\"U1\",\
             \"mcpServers\":{\"srv\":{\"command\":\"x\",\"env\":{\"TOKEN\":\"s3cr3t\"}}},\
             \"projects\":{\"p\":true}}",
        )
        .unwrap();
        Fx {
            _g: Guard(dir),
            primary,
            primary_json,
            home,
            state,
        }
    }

    fn json_of(path: &Path) -> Value {
        serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
    }

    #[test]
    fn fresh_materialize_then_idempotent() {
        let fx = fixture();
        let changes = plan_materialize(&fx.primary, &fx.home, &fx.state, &fx.primary_json).unwrap();
        assert!(!changes.is_empty());
        apply_changes(&changes).unwrap();
        for name in LINKS {
            assert_eq!(
                fs::read_link(fx.home.join(name)).unwrap(),
                fx.primary.join(name)
            );
        }
        assert_eq!(json_of(&fx.home.join("settings.json"))["theme"], "dark");
        let sec = json_of(&fx.home.join(".claude.json"));
        assert_eq!(sec["mcpServers"], json_of(&fx.primary_json)["mcpServers"]);
        // Minimal file: only the managed key.
        assert_eq!(
            sec.as_object().unwrap().keys().collect::<Vec<_>>(),
            vec!["mcpServers"]
        );
        assert!(
            plan_materialize(&fx.primary, &fx.home, &fx.state, &fx.primary_json)
                .unwrap()
                .is_empty()
        );
        assert!(
            check(&fx.primary, &fx.home, &fx.primary_json)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn settings_drift_both_ways() {
        let fx = fixture();
        apply_changes(
            &plan_materialize(&fx.primary, &fx.home, &fx.state, &fx.primary_json).unwrap(),
        )
        .unwrap();
        fs::write(
            fx.home.join("settings.json"),
            "{\"theme\":\"light\",\"mine\":1}\n",
        )
        .unwrap();
        let drift = check(&fx.primary, &fx.home, &fx.primary_json).unwrap();
        let keys: Vec<String> = drift
            .iter()
            .filter_map(|d| match d {
                Drift::SettingsKey { key, .. } => Some(key.clone()),
                _ => None,
            })
            .collect();
        assert!(keys.contains(&"theme".to_string()), "{keys:?}");
        assert!(keys.contains(&"mine".to_string()), "{keys:?}");
        apply_changes(
            &plan_materialize(&fx.primary, &fx.home, &fx.state, &fx.primary_json).unwrap(),
        )
        .unwrap();
        let merged = json_of(&fx.home.join("settings.json"));
        assert_eq!(merged["theme"], "dark");
        assert_eq!(merged["mine"], 1);
        let drift = check(&fx.primary, &fx.home, &fx.primary_json).unwrap();
        assert!(
            drift.iter().any(|d| matches!(
                d,
                Drift::SettingsKey { key, .. } if key == "mine"
            )),
            "secondary-only key is kept and still reported"
        );
    }

    #[test]
    fn mcp_merge_keeps_other_keys() {
        let fx = fixture();
        fs::create_dir_all(&fx.home).unwrap();
        fs::write(
            fx.home.join(".claude.json"),
            "{\"oauthAccount\":{\"accountUuid\":\"S-UUID\"},\"userID\":\"U9\",\
             \"mcpServers\":{\"old\":{}},\"projects\":{\"q\":1}}",
        )
        .unwrap();
        let before = json_of(&fx.home.join(".claude.json"));
        apply_changes(
            &plan_materialize(&fx.primary, &fx.home, &fx.state, &fx.primary_json).unwrap(),
        )
        .unwrap();
        let after = json_of(&fx.home.join(".claude.json"));
        for key in ["oauthAccount", "userID", "projects"] {
            assert_eq!(after[key], before[key], "key {key} must survive");
        }
        assert_eq!(after["mcpServers"], json_of(&fx.primary_json)["mcpServers"]);
    }

    #[test]
    fn dry_run_output_is_redacted() {
        let fx = fixture();
        apply_changes(
            &plan_materialize(&fx.primary, &fx.home, &fx.state, &fx.primary_json).unwrap(),
        )
        .unwrap();
        fs::write(
            fx.primary_json.clone(),
            "{\"oauthAccount\":{\"accountUuid\":\"P-NEW\"},\
             \"mcpServers\":{\"srv\":{\"command\":\"y\",\"env\":{\"TOKEN\":\"n3w\"}},\
             \"extra\":{\"env\":{\"K\":\"v\"}}}}",
        )
        .unwrap();
        let changes = plan_materialize(&fx.primary, &fx.home, &fx.state, &fx.primary_json).unwrap();
        let text = changes
            .iter()
            .map(describe_change)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("<redacted>"), "{text}");
        assert!(text.contains("extra"), "{text}");
        for secret in ["s3cr3t", "n3w", "P-UUID", "P-NEW"] {
            assert!(!text.contains(secret), "leak in {text}");
        }
    }

    #[test]
    fn real_dir_is_backed_up_then_linked() {
        let fx = fixture();
        fs::create_dir_all(fx.home.join("skills")).unwrap();
        fs::write(fx.home.join("skills/keep.txt"), "keep").unwrap();
        let changes = plan_materialize(&fx.primary, &fx.home, &fx.state, &fx.primary_json).unwrap();
        assert!(
            changes
                .iter()
                .any(|c| matches!(c, Change::BackupRemove { .. })),
            "{changes:?}"
        );
        apply_changes(&changes).unwrap();
        assert_eq!(
            fs::read_link(fx.home.join("skills")).unwrap(),
            fx.primary.join("skills")
        );
        let backups: Vec<PathBuf> = fs::read_dir(fx.state.join("backups"))
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        assert_eq!(backups.len(), 1);
        assert_eq!(fs::read(backups[0].join("keep.txt")).unwrap(), b"keep");
        assert!(
            plan_materialize(&fx.primary, &fx.home, &fx.state, &fx.primary_json)
                .unwrap()
                .is_empty()
        );
    }

    fn mode_of(path: &Path) -> u32 {
        use std::os::unix::fs::PermissionsExt;
        fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[test]
    fn primary_json_lives_beside_the_profile() {
        let fx = fixture();
        // A decoy inside the primary dir must be ignored.
        fs::write(
            fx.primary.join(".claude.json"),
            "{\"mcpServers\":{\"decoy\":{}}}",
        )
        .unwrap();
        let changes = plan_materialize(&fx.primary, &fx.home, &fx.state, &fx.primary_json).unwrap();
        apply_changes(&changes).unwrap();
        let sec = json_of(&fx.home.join(".claude.json"));
        assert_eq!(sec["mcpServers"], json_of(&fx.primary_json)["mcpServers"]);
        assert!(sec.get("decoy").is_none(), "{sec}");
        // A missing primary file is a sync error and a check problem.
        fs::remove_file(&fx.primary_json).unwrap();
        assert!(plan_materialize(&fx.primary, &fx.home, &fx.state, &fx.primary_json).is_err());
        let drift = check(&fx.primary, &fx.home, &fx.primary_json).unwrap();
        assert!(
            drift
                .iter()
                .any(|d| matches!(d, Drift::PrimaryConfigMissing { .. })),
            "{drift:?}"
        );
    }

    #[test]
    fn overwrite_makes_a_backup() {
        let fx = fixture();
        apply_changes(
            &plan_materialize(&fx.primary, &fx.home, &fx.state, &fx.primary_json).unwrap(),
        )
        .unwrap();
        let before = fs::read(fx.home.join("settings.json")).unwrap();
        fs::write(fx.primary.join("settings.json"), "{\"theme\":\"light\"}\n").unwrap();
        let changes = plan_materialize(&fx.primary, &fx.home, &fx.state, &fx.primary_json).unwrap();
        assert!(!changes.is_empty());
        let backup = changes
            .iter()
            .find_map(|c| match c {
                Change::WriteFile {
                    backup: Some(dest), ..
                } => Some(dest.clone()),
                _ => None,
            })
            .expect("claude writes carry a backup");
        let text = changes
            .iter()
            .map(describe_change)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains(&*backup.to_string_lossy()), "{text}");
        apply_changes(&changes).unwrap();
        assert_eq!(fs::read(&backup).unwrap(), before);
        assert_eq!(mode_of(&backup), 0o600);
    }

    #[test]
    fn existing_mode_is_preserved_except_claude_json() {
        use std::os::unix::fs::PermissionsExt;
        let fx = fixture();
        apply_changes(
            &plan_materialize(&fx.primary, &fx.home, &fx.state, &fx.primary_json).unwrap(),
        )
        .unwrap();
        assert_eq!(mode_of(&fx.home.join("settings.json")), 0o600);
        fs::set_permissions(
            fx.home.join("settings.json"),
            fs::Permissions::from_mode(0o644),
        )
        .unwrap();
        fs::set_permissions(
            fx.home.join(".claude.json"),
            fs::Permissions::from_mode(0o644),
        )
        .unwrap();
        fs::write(fx.primary.join("settings.json"), "{\"theme\":\"light\"}\n").unwrap();
        fs::write(&fx.primary_json, "{\"mcpServers\":{\"srv2\":{}}}").unwrap();
        apply_changes(
            &plan_materialize(&fx.primary, &fx.home, &fx.state, &fx.primary_json).unwrap(),
        )
        .unwrap();
        assert_eq!(mode_of(&fx.home.join("settings.json")), 0o644);
        assert_eq!(mode_of(&fx.home.join(".claude.json")), 0o600);
    }

    #[test]
    fn symlinked_settings_becomes_a_real_file() {
        let fx = fixture();
        // Pretty already, so only the symlink forces a write.
        fs::write(
            fx.primary.join("settings.json"),
            "{\n  \"theme\": \"dark\"\n}\n",
        )
        .unwrap();
        fs::create_dir_all(&fx.home).unwrap();
        std::os::unix::fs::symlink(
            fx.primary.join("settings.json"),
            fx.home.join("settings.json"),
        )
        .unwrap();
        let changes = plan_materialize(&fx.primary, &fx.home, &fx.state, &fx.primary_json).unwrap();
        assert!(
            changes.iter().any(|c| matches!(
                c,
                Change::WriteFile { path, .. } if path == &fx.home.join("settings.json")
            )),
            "{changes:?}"
        );
        apply_changes(&changes).unwrap();
        assert!(
            !fs::symlink_metadata(fx.home.join("settings.json"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(json_of(&fx.home.join("settings.json"))["theme"], "dark");
        assert!(
            plan_materialize(&fx.primary, &fx.home, &fx.state, &fx.primary_json)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn import_settings_key_into_primary_then_propagates() {
        let fx = fixture();
        // Secondary-only key plus a changed shared key.
        fs::create_dir_all(&fx.home).unwrap();
        fs::write(
            fx.home.join("settings.json"),
            "{\"theme\":\"light\",\"mine\":1}\n",
        )
        .unwrap();
        let change = import_settings_key(&fx.primary, &fx.home, &fx.state, "mine")
            .unwrap()
            .expect("secondary-only key imports");
        assert!(matches!(
            change,
            Change::WriteFile { ref path, ref backup, .. }
            if path == &fx.primary.join("settings.json") && backup.is_some()
        ));
        apply_changes(&[change]).unwrap();
        let raw = fs::read_to_string(fx.primary.join("settings.json")).unwrap();
        // Key order kept: existing keys stay first, the import appends.
        assert!(raw.find("\"theme\"").unwrap() < raw.find("\"mine\"").unwrap());
        assert_eq!(json_of(&fx.primary.join("settings.json"))["mine"], 1);
        // Idempotent, and a later sync no longer reports this key as drift.
        assert!(
            import_settings_key(&fx.primary, &fx.home, &fx.state, "mine")
                .unwrap()
                .is_none()
        );
        assert!(
            check(&fx.primary, &fx.home, &fx.primary_json)
                .unwrap()
                .iter()
                .all(|d| !matches!(d, Drift::SettingsKey { key, .. } if key == "mine"))
        );
        // Unknown keys and empty keys are errors, never writes.
        assert!(import_settings_key(&fx.primary, &fx.home, &fx.state, "nope").is_err());
        assert!(import_settings_key(&fx.primary, &fx.home, &fx.state, "").is_err());
    }

    #[test]
    fn import_mcp_server_keeps_other_servers() {
        let fx = fixture();
        apply_changes(
            &plan_materialize(&fx.primary, &fx.home, &fx.state, &fx.primary_json).unwrap(),
        )
        .unwrap();
        // Secondary gains a server the primary lacks.
        let mut sec: Value = json_of(&fx.home.join(".claude.json"));
        sec["mcpServers"]["extra"] = serde_json::json!({"command": "x"});
        fs::write(
            fx.home.join(".claude.json"),
            serde_json::to_vec_pretty(&sec).unwrap(),
        )
        .unwrap();
        let change = import_mcp_server(&fx.primary_json, &fx.home, &fx.state, "extra")
            .unwrap()
            .expect("new server imports");
        assert!(matches!(
            change,
            Change::WriteFile { ref backup, .. } if backup.is_some()
        ));
        apply_changes(&[change]).unwrap();
        let pri = json_of(&fx.primary_json);
        assert_eq!(
            pri["mcpServers"]["extra"],
            serde_json::json!({"command": "x"})
        );
        assert!(pri["mcpServers"].get("srv").is_some(), "old server kept");
        assert!(
            import_mcp_server(&fx.primary_json, &fx.home, &fx.state, "extra")
                .unwrap()
                .is_none()
        );
        assert!(import_mcp_server(&fx.primary_json, &fx.home, &fx.state, "nope").is_err());
        assert!(import_mcp_server(&fx.primary_json, &fx.home, &fx.state, "").is_err());
    }
}

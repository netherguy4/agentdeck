//! Harness detect/update/install via each harness's own updater.
//!
//! Detection is read-only (`<bin> --version` + install-method facts from the
//! owner environment: Codex standalone installer, OpenCode brew
//! `anomalyco/tap/opencode-v2`, Claude native installer, agy standalone
//! script). `update`/`install` call only the official updater/installer
//! command, always print it first, and take an injected command runner so
//! tests never run a real update or install.

use std::path::PathBuf;

use serde::Serialize;

/// Runs one command, capturing stdout. Injected so tests never touch the
/// real updaters; production passes the `std::process::Command` runner.
pub type Runner<'a> = &'a dyn Fn(&str, &[String]) -> anyhow::Result<String>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HarnessName {
    Claude,
    Codex,
    Opencode,
    Agy,
}

impl HarnessName {
    pub fn parse(s: &str) -> anyhow::Result<Self> {
        match s {
            "claude" => Ok(Self::Claude),
            "codex" => Ok(Self::Codex),
            "opencode" => Ok(Self::Opencode),
            "agy" => Ok(Self::Agy),
            other => {
                anyhow::bail!("bad harness {other:?}: expected claude, codex, opencode or agy")
            }
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Opencode => "opencode",
            Self::Agy => "agy",
        }
    }

    /// Install method fact (owner environment, AGENTS.md).
    pub fn install_method(self) -> &'static str {
        match self {
            Self::Claude => "native installer (`claude update`)",
            Self::Codex => "standalone installer (`codex update`)",
            Self::Opencode => "brew anomalyco/tap/opencode-v2 (`opencode upgrade`)",
            Self::Agy => "standalone script (`agy update`)",
        }
    }

    /// The official updater command (program + args).
    pub fn update_argv(self, binary: &str) -> Vec<String> {
        let sub = match self {
            // `opencode update` is an alias, but `upgrade` is canonical.
            Self::Opencode => "upgrade",
            Self::Claude | Self::Codex | Self::Agy => "update",
        };
        vec![binary.to_string(), sub.to_string()]
    }

    /// The official fresh-install command, run through `sh -c`.
    /// Pipe-to-shell installers are the vendors' documented method.
    pub fn install_argv(self) -> Vec<String> {
        let script = match self {
            Self::Claude => "curl -fsSL https://claude.ai/install.sh | bash",
            Self::Codex => "curl -fsSL https://chatgpt.com/codex/install.sh | sh",
            Self::Opencode => "brew install anomalyco/tap/opencode-v2",
            Self::Agy => "curl -fsSL https://antigravity.google/cli/install.sh | bash",
        };
        vec!["sh".to_string(), "-c".to_string(), script.to_string()]
    }
}

pub fn all() -> [HarnessName; 4] {
    use HarnessName::*;
    [Claude, Codex, Opencode, Agy]
}

#[derive(Debug, Clone, Serialize)]
pub struct Status {
    pub name: String,
    /// Resolved binary path, when found on PATH.
    pub binary: Option<PathBuf>,
    /// First line of `<binary> --version`, when it runs.
    pub version: Option<String>,
    pub install_method: String,
}

fn find_on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(name);
        if is_executable(&candidate) {
            return Some(candidate);
        }
    }
    None
}

#[cfg(unix)]
fn is_executable(path: &PathBuf) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(path: &PathBuf) -> bool {
    std::fs::metadata(path).is_ok_and(|m| m.is_file())
}

fn first_line(s: &str) -> Option<String> {
    let line = s.lines().next()?.trim();
    if line.is_empty() {
        None
    } else {
        Some(line.to_string())
    }
}

/// Read-only detection of every known harness. Never writes, never updates.
pub fn list(run: Runner<'_>) -> Vec<Status> {
    all()
        .iter()
        .map(|h| {
            let binary = find_on_path(h.as_str());
            let version = binary.as_ref().and_then(|bin| {
                let argv = vec!["--version".to_string()];
                run(&bin.display().to_string(), &argv)
                    .ok()
                    .and_then(|out| first_line(&out))
            });
            Status {
                name: h.as_str().to_string(),
                binary,
                version,
                install_method: h.install_method().to_string(),
            }
        })
        .collect()
}

fn shell_words(argv: &[String]) -> String {
    argv.join(" ")
}

/// Print the official command first, then run it (unless dry-run).
/// Never restarts or otherwise touches running instances.
pub fn update(
    name: HarnessName,
    binary: &str,
    dry_run: bool,
    run: Runner<'_>,
) -> anyhow::Result<()> {
    let argv = name.update_argv(binary);
    println!("+ {}", shell_words(&argv));
    if dry_run {
        println!("dry-run: {} not updated", name.as_str());
        return Ok(());
    }
    run(&argv[0], &argv[1..])?;
    println!("updated {}", name.as_str());
    Ok(())
}

/// Print the official installer first, then run it (unless dry-run).
pub fn install(name: HarnessName, dry_run: bool, run: Runner<'_>) -> anyhow::Result<()> {
    let argv = name.install_argv();
    println!("+ {}", shell_words(&argv));
    if dry_run {
        println!("dry-run: {} not installed", name.as_str());
        return Ok(());
    }
    run(&argv[0], &argv[1..])?;
    println!("installed {}", name.as_str());
    Ok(())
}

/// Production runner: real subprocess, stdout captured.
pub fn real_runner(program: &str, args: &[String]) -> anyhow::Result<String> {
    let out = std::process::Command::new(program)
        .args(args)
        .output()
        .map_err(|e| anyhow::anyhow!("cannot run {program}: {e}"))?;
    if !out.status.success() {
        anyhow::bail!(
            "{program} failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[test]
    fn argv_table() {
        assert_eq!(
            HarnessName::Claude.update_argv("claude"),
            vec!["claude", "update"]
        );
        assert_eq!(
            HarnessName::Codex.update_argv("codex"),
            vec!["codex", "update"]
        );
        assert_eq!(
            HarnessName::Opencode.update_argv("opencode"),
            vec!["opencode", "upgrade"]
        );
        assert_eq!(HarnessName::Agy.update_argv("agy"), vec!["agy", "update"]);
        assert!(HarnessName::parse("bogus").is_err());
        for h in all() {
            // Installers run through sh -c and name their source.
            let argv = h.install_argv();
            assert_eq!(&argv[..2], &["sh", "-c"]);
            assert!(!argv[2].is_empty());
        }
    }

    #[test]
    fn update_prints_first_and_injects_runner() {
        let seen: RefCell<Vec<Vec<String>>> = RefCell::new(Vec::new());
        let run = |prog: &str, args: &[String]| -> anyhow::Result<String> {
            let mut full = vec![prog.to_string()];
            full.extend(args.iter().cloned());
            seen.borrow_mut().push(full);
            Ok(String::new())
        };
        update(HarnessName::Opencode, "opencode", true, &run).unwrap();
        assert!(seen.borrow().is_empty(), "dry-run must not run");
        update(HarnessName::Opencode, "/x/opencode", false, &run).unwrap();
        assert_eq!(seen.borrow().as_slice(), &[vec!["/x/opencode", "upgrade"]]);
    }

    #[test]
    fn list_uses_runner_and_path() {
        let run = |_prog: &str, _args: &[String]| -> anyhow::Result<String> {
            Ok("opencode v9.9.9\n".to_string())
        };
        let rows = list(&run);
        assert_eq!(rows.len(), 4);
        // PATH lookup is real: at least our own test env finds sh, but these
        // harnesses may or may not be installed; only shape is asserted.
        for row in &rows {
            assert!(!row.install_method.is_empty());
            if let Some(v) = &row.version {
                assert!(!v.is_empty() && !v.contains('\n'));
                assert!(row.binary.is_some());
            }
        }
    }
}

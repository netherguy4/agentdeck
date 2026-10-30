//! Agy adapter: a single Google account per OS user (M7 keyring experiment).
//!
//! Temp-HOME `agy mcp list` shows no servers (config is per-HOME `~/.gemini`)
//! while temp-HOME `agy models` still lists the full catalog; blocking the
//! session bus flips it to "Please sign in". The only keyring entry is
//! `service=gemini username=antigravity`. So credentials live in the shared
//! OS keyring, NOT per-HOME: a second login dir never yields a second
//! account. Treat agy as single-account; login dirs separate profile only.
//! There is no login CLI: `agentdeck login` prints the manual IDE step.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;

/// Printed by `agentdeck login` for agy instances (no CLI login exists).
pub const LOGIN_HINT: &str =
    "agy has no login CLI: sign in with your Google account in the Antigravity IDE";

/// The login dir holding `~/.gemini` for a registry home.
pub fn gemini_home(home: &Path) -> PathBuf {
    home.join(".gemini")
}

#[derive(Debug, Clone, Serialize)]
pub enum Drift {
    NoHome { home: PathBuf },
    NoGeminiDir { dir: PathBuf },
    NoMcpConfig { path: PathBuf },
}

impl Drift {
    pub fn describe(&self) -> String {
        match self {
            Drift::NoHome { home } => {
                format!("login dir {} missing", home.display())
            }
            Drift::NoGeminiDir { dir } => {
                format!("{} missing (never signed in here)", dir.display())
            }
            Drift::NoMcpConfig { path } => {
                format!("{} missing (IDE MCP config absent)", path.display())
            }
        }
    }
}

/// Read-only check of one agy login dir. Never touches the keyring.
pub fn check(home: &Path) -> anyhow::Result<Vec<Drift>> {
    if fs::symlink_metadata(home).is_err() {
        return Ok(vec![Drift::NoHome {
            home: home.to_path_buf(),
        }]);
    }
    let gemini = gemini_home(home);
    if fs::symlink_metadata(&gemini).is_err() {
        return Ok(vec![Drift::NoGeminiDir { dir: gemini }]);
    }
    let mcp = gemini.join("config/mcp_config.json");
    if fs::symlink_metadata(&mcp).is_err() {
        return Ok(vec![Drift::NoMcpConfig { path: mcp }]);
    }
    Ok(vec![])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static N: AtomicU64 = AtomicU64::new(0);

    fn tmp() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "agentdeck-agy-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn check_reports_layout_problems() {
        let base = tmp();
        let home = base.join("home");
        assert!(matches!(
            check(&home).unwrap().as_slice(),
            [Drift::NoHome { .. }]
        ));
        fs::create_dir_all(&home).unwrap();
        assert!(matches!(
            check(&home).unwrap().as_slice(),
            [Drift::NoGeminiDir { .. }]
        ));
        fs::create_dir_all(home.join(".gemini")).unwrap();
        assert!(matches!(
            check(&home).unwrap().as_slice(),
            [Drift::NoMcpConfig { .. }]
        ));
        fs::create_dir_all(home.join(".gemini/config")).unwrap();
        fs::write(home.join(".gemini/config/mcp_config.json"), "{}").unwrap();
        assert!(check(&home).unwrap().is_empty());
        let _ = fs::remove_dir_all(&base);
    }
}

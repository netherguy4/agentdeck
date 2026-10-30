//! Config/state directory resolution. All paths derive from explicit roots
//! (env vars or caller-provided dirs) so tests run on temp dirs.

use std::env;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Paths {
    pub config_dir: PathBuf,
    pub state_dir: PathBuf,
}

impl Paths {
    pub fn new(config_dir: PathBuf, state_dir: PathBuf) -> Self {
        Self {
            config_dir,
            state_dir,
        }
    }

    /// Production resolution: `$AGENTDECK_CONFIG_DIR` / `$XDG_CONFIG_HOME/agentdeck` /
    /// `~/.config/agentdeck`, and likewise `$AGENTDECK_STATE_DIR` / `$XDG_STATE_HOME/agentdeck` /
    /// `~/.local/state/agentdeck`.
    pub fn from_env() -> anyhow::Result<Self> {
        let home = env::var_os("HOME").map(PathBuf::from);
        let config_dir = env::var_os("AGENTDECK_CONFIG_DIR")
            .map(PathBuf::from)
            .or_else(|| env::var_os("XDG_CONFIG_HOME").map(|d| PathBuf::from(d).join("agentdeck")))
            .or_else(|| home.clone().map(|h| h.join(".config/agentdeck")))
            .ok_or_else(|| {
                anyhow::anyhow!("cannot resolve config dir: set HOME or AGENTDECK_CONFIG_DIR")
            })?;
        let state_dir = env::var_os("AGENTDECK_STATE_DIR")
            .map(PathBuf::from)
            .or_else(|| env::var_os("XDG_STATE_HOME").map(|d| PathBuf::from(d).join("agentdeck")))
            .or_else(|| home.map(|h| h.join(".local/state/agentdeck")))
            .ok_or_else(|| {
                anyhow::anyhow!("cannot resolve state dir: set HOME or AGENTDECK_STATE_DIR")
            })?;
        Ok(Self {
            config_dir,
            state_dir,
        })
    }

    pub fn registry_file(&self) -> PathBuf {
        self.config_dir.join("instances.toml")
    }

    pub fn lock_file(&self) -> PathBuf {
        self.config_dir.join("instances.toml.lock")
    }

    pub fn backup_dir(&self) -> PathBuf {
        self.state_dir.join("backups")
    }
}

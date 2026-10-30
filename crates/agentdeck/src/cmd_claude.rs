//! `agentdeck claude sync|check`: materialize and drift-check secondary Claude
//! accounts from the primary profile. `instance add` stays registry-only.

use std::path::{Path, PathBuf};

use agentdeck_core::{Harness, Paths, Registry, claude};
use clap::Subcommand;

#[derive(Subcommand)]
pub enum ClaudeCmd {
    /// Materialize/sync a secondary Claude account from the primary profile.
    Sync {
        /// Registry instance id whose home becomes the secondary account.
        id: String,
        #[arg(long)]
        dry_run: bool,
    },
    /// Show drift between the primary profile and secondary account(s).
    Check {
        /// Registry instance id; omit to check all Claude instances.
        id: Option<String>,
        #[arg(long)]
        json: bool,
    },
}

pub fn run(cmd: &ClaudeCmd) -> anyhow::Result<()> {
    let paths = Paths::from_env()?;
    let reg = Registry::load(&paths)?;
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| anyhow::anyhow!("cannot resolve primary profile: HOME is not set"))?;
    let primary = home.join(".claude");
    let primary_json = home.join(".claude.json");
    match cmd {
        ClaudeCmd::Sync { id, dry_run } => {
            let inst = reg
                .get(id)
                .ok_or_else(|| anyhow::anyhow!("unknown instance id {id:?}"))?;
            if inst.harness != Harness::Claude {
                anyhow::bail!("instance {id:?} is not a Claude instance");
            }
            let dir = expand_tilde(&inst.home, &home);
            let changes =
                claude::plan_materialize(&primary, &dir, &paths.state_dir, &primary_json)?;
            for c in &changes {
                println!("{}", claude::describe_change(c));
            }
            if *dry_run {
                println!("dry-run: {id} unchanged");
            } else {
                agentdeck_core::apply_changes(&changes)?;
                println!("synced {id}");
            }
        }
        ClaudeCmd::Check { id, json } => {
            let targets: Vec<_> = reg
                .instances
                .iter()
                .filter(|i| i.harness == Harness::Claude && id.as_deref().is_none_or(|w| w == i.id))
                .collect();
            if id.is_some() && targets.is_empty() {
                anyhow::bail!("unknown instance id {:?}", id.as_deref().unwrap_or(""));
            }
            if *json {
                let mut out = Vec::new();
                for inst in &targets {
                    let drift =
                        claude::check(&primary, &expand_tilde(&inst.home, &home), &primary_json)?;
                    out.push(serde_json::json!({"id": inst.id, "drift": drift}));
                }
                println!("{}", serde_json::to_string_pretty(&out)?);
            } else {
                for inst in &targets {
                    let drift =
                        claude::check(&primary, &expand_tilde(&inst.home, &home), &primary_json)?;
                    if drift.is_empty() {
                        println!("{}: no drift", inst.id);
                    } else {
                        for d in &drift {
                            println!("{}: {}", inst.id, d.describe());
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

fn expand_tilde(value: &str, home: &Path) -> PathBuf {
    if value == "~" {
        home.to_path_buf()
    } else if let Some(rest) = value.strip_prefix("~/") {
        home.join(rest)
    } else {
        PathBuf::from(value)
    }
}

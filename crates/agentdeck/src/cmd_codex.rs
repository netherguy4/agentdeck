//! `agentdeck codex sync|check|fix`: materialize and drift-check Codex shadow
//! homes. `instance add` stays registry-only; the primary instance is the
//! Codex instance whose home is the main home (`~/.codex`).

use std::path::{Path, PathBuf};

use agentdeck_core::{Harness, Paths, Registry, codex};
use clap::Subcommand;

#[derive(Subcommand)]
pub enum CodexCmd {
    /// Materialize/sync a shadow Codex home from the main home.
    Sync {
        /// Registry instance id whose home becomes the shadow home.
        id: String,
        #[arg(long)]
        dry_run: bool,
    },
    /// Show drift between the main home and shadow home(s).
    Check {
        /// Registry instance id; omit to check all Codex shadow instances.
        id: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Merge, back up and relink real entries where links belong.
    Fix {
        /// Registry instance id whose home is the shadow home.
        id: String,
        #[arg(long)]
        dry_run: bool,
        /// Back up real entries with unknown content instead of deferring.
        #[arg(long)]
        force_backup: bool,
    },
}

pub fn run(cmd: &CodexCmd) -> anyhow::Result<()> {
    let paths = Paths::from_env()?;
    let reg = Registry::load(&paths)?;
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| anyhow::anyhow!("cannot resolve main home: HOME is not set"))?;
    let main_default = home.join(".codex");
    // Primary: the Codex instance living in the main home, if registered.
    let primary_home = reg
        .instances
        .iter()
        .find(|i| i.harness == Harness::Codex && expand_tilde(&i.home, &home) == main_default)
        .map(|i| expand_tilde(&i.home, &home))
        .unwrap_or_else(|| main_default.clone());
    match cmd {
        CodexCmd::Sync { id, dry_run } => {
            let shadow = shadow_of(&reg, id, &home)?;
            if shadow == primary_home {
                println!("{id}: primary home, nothing to sync");
                return Ok(());
            }
            let changes = codex::plan_materialize(&primary_home, &shadow, &paths.state_dir)?;
            for c in &changes {
                println!("{c}", c = c.describe());
            }
            if *dry_run {
                println!("dry-run: {id} unchanged");
            } else {
                agentdeck_core::apply_changes(&changes)?;
                println!("synced {id}");
            }
        }
        CodexCmd::Check { id, json } => {
            if let Some(want) = id.as_deref() {
                let inst = reg
                    .get(want)
                    .filter(|i| i.harness == Harness::Codex)
                    .ok_or_else(|| anyhow::anyhow!("unknown Codex instance id {want:?}"))?;
                if expand_tilde(&inst.home, &home) == primary_home {
                    println!("{want}: primary home (no shadow to check)");
                    return Ok(());
                }
            }
            let targets: Vec<(&str, PathBuf)> = reg
                .instances
                .iter()
                .filter(|i| {
                    i.harness == Harness::Codex
                        && id.as_deref().is_none_or(|w| w == i.id)
                        && expand_tilde(&i.home, &home) != primary_home
                })
                .map(|i| (i.id.as_str(), expand_tilde(&i.home, &home)))
                .collect();
            if *json {
                let mut out = Vec::new();
                for (id, shadow) in &targets {
                    let drift = codex::check(&primary_home, shadow)?;
                    out.push(serde_json::json!({"id": id, "drift": drift}));
                }
                println!("{}", serde_json::to_string_pretty(&out)?);
            } else {
                for (id, shadow) in &targets {
                    let drift = codex::check(&primary_home, shadow)?;
                    if drift.is_empty() {
                        println!("{id}: no drift");
                    } else {
                        for d in &drift {
                            println!("{id}: {}", d.describe());
                        }
                    }
                }
            }
            if targets.iter().any(|(_, s)| {
                !codex::check(&primary_home, s)
                    .unwrap_or_default()
                    .is_empty()
            }) {
                std::process::exit(1);
            }
        }
        CodexCmd::Fix {
            id,
            dry_run,
            force_backup,
        } => {
            let shadow = shadow_of(&reg, id, &home)?;
            if shadow == primary_home {
                println!("{id}: primary home, nothing to fix");
                return Ok(());
            }
            let plan = codex::fix(&primary_home, &shadow, &paths.state_dir, *force_backup)?;
            for c in &plan.changes {
                println!("{c}", c = c.describe());
            }
            for d in &plan.deferred {
                println!("deferred: {d}");
            }
            if *dry_run {
                println!("dry-run: {id} unchanged");
            } else if plan.changes.is_empty() && plan.deferred.is_empty() {
                println!("{id}: no drift");
            } else {
                agentdeck_core::apply_changes(&plan.changes)?;
                println!("fixed {id}");
            }
        }
    }
    Ok(())
}

fn shadow_of(reg: &Registry, id: &str, home: &Path) -> anyhow::Result<PathBuf> {
    let inst = reg
        .get(id)
        .ok_or_else(|| anyhow::anyhow!("unknown instance id {id:?}"))?;
    if inst.harness != Harness::Codex {
        anyhow::bail!("instance {id:?} is not a Codex instance");
    }
    Ok(expand_tilde(&inst.home, home))
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

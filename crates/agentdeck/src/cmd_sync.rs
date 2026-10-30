//! `agentdeck sync status|run|auto-apply`: report and background-apply drift.
//! Additive items auto-apply only when `[sync] auto_apply = true` (OFF by
//! default); decision items are only reported, never written.

use std::path::PathBuf;

use agentdeck_core::{Paths, sync};
use clap::Subcommand;

#[derive(Subcommand)]
pub enum SyncCmd {
    /// Show drift (additive vs decision) and the auto-apply flag. Writes nothing.
    Status {
        #[arg(long)]
        json: bool,
    },
    /// Run one sync pass: report drift, auto-apply additive items when enabled.
    Run {
        #[arg(long)]
        dry_run: bool,
    },
    /// Turn background auto-apply off (default) or on.
    AutoApply {
        /// `on` or `off`.
        value: String,
        #[arg(long)]
        dry_run: bool,
    },
}

fn home_dir() -> anyhow::Result<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| anyhow::anyhow!("cannot resolve home: HOME is not set"))
}

pub fn run(cmd: &SyncCmd) -> anyhow::Result<()> {
    let paths = Paths::from_env()?;
    let home = home_dir()?;
    let t3_root = agentdeck_core::t3::t3_root().ok();
    match cmd {
        SyncCmd::Status { json } => {
            let items = sync::gather(&paths, &home, t3_root.as_deref())?;
            let auto = sync::auto_apply_enabled(&paths);
            if *json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "auto_apply": auto,
                        "items": items,
                    }))?
                );
            } else {
                println!("auto-apply: {}", if auto { "on" } else { "off" });
                if items.is_empty() {
                    println!("no drift");
                } else {
                    for i in &items {
                        let kind = match i.kind {
                            sync::ItemKind::Additive => "additive",
                            sync::ItemKind::Decision => "decision",
                        };
                        println!("{} [{}] {kind}: {}", i.instance, i.harness, i.detail);
                    }
                }
            }
        }
        SyncCmd::Run { dry_run } => {
            let items = sync::gather(&paths, &home, t3_root.as_deref())?;
            let auto = sync::auto_apply_enabled(&paths);
            let additive = items
                .iter()
                .filter(|i| i.kind == sync::ItemKind::Additive)
                .count();
            let decision = items.len() - additive;
            if *dry_run || !auto {
                for i in &items {
                    let kind = match i.kind {
                        sync::ItemKind::Additive => "additive",
                        sync::ItemKind::Decision => "decision",
                    };
                    println!("{} [{}] {kind}: {}", i.instance, i.harness, i.detail);
                }
                if *dry_run {
                    println!("dry-run: nothing written");
                } else {
                    println!(
                        "auto-apply is off: {additive} additive item(s) reported, nothing written"
                    );
                }
            } else {
                let applied = sync::apply_additive(&paths, &home, t3_root.as_deref())?;
                for c in &applied {
                    println!("{}", redact_change(c));
                }
                println!(
                    "applied {} change(s), {decision} decision item(s) left",
                    applied.len()
                );
            }
        }
        SyncCmd::AutoApply { value, dry_run } => {
            let on = match value.as_str() {
                "on" => true,
                "off" => false,
                other => anyhow::bail!("expected on or off, got {other:?}"),
            };
            let changes = sync::set_auto_apply(&paths, on, *dry_run)?;
            if *dry_run {
                println!("dry-run: instances.toml unchanged");
            } else if changes.is_empty() {
                println!("auto-apply already {}", if on { "on" } else { "off" });
            } else {
                println!("auto-apply {}", if on { "on" } else { "off" });
            }
        }
    }
    Ok(())
}

/// Change description without secret values (`.claude.json` writes show only
/// the redacted mcpServers diff).
fn redact_change(change: &agentdeck_core::Change) -> String {
    agentdeck_core::claude::describe_change(change)
}

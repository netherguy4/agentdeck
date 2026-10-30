//! `agentdeck t3` commands: read-only status, validated sync, registry import.

use agentdeck_core::{Paths, t3};
use clap::Subcommand;

#[derive(Subcommand)]
pub enum T3Cmd {
    /// Read-only per-instance status: registry link, driver, cache health, T3 thread count.
    Status {
        #[arg(long)]
        json: bool,
    },
    /// Merge registry instances into T3 `providerInstances` (validated, atomic, verified).
    Sync {
        #[arg(long)]
        dry_run: bool,
    },
    /// Add T3 claudeAgent/codex instances missing from the registry into it.
    Import {
        #[arg(long)]
        dry_run: bool,
    },
}

pub fn run_t3(paths: &Paths, cmd: T3Cmd) -> anyhow::Result<()> {
    let root = t3::t3_root()?;
    match cmd {
        T3Cmd::Status { json } => {
            let rows = t3::t3_status(paths, &root)?;
            if json {
                let out: Vec<_> = rows
                    .iter()
                    .map(|r| {
                        serde_json::json!({
                            "id": r.id,
                            "driver": r.driver,
                            "in_registry": r.in_registry,
                            "enabled": r.enabled,
                            "cache": r.cache.as_ref().map(|c| serde_json::json!({
                                "status": c.status, "checkedAt": c.checked_at,
                            })),
                            "threads": r.threads,
                        })
                    })
                    .collect();
                println!("{}", serde_json::to_string_pretty(&out)?);
            } else {
                for r in &rows {
                    let cache = r.cache.as_ref().map_or("n/a".to_string(), |c| {
                        format!("{} @ {}", c.status, c.checked_at.as_deref().unwrap_or("?"))
                    });
                    println!(
                        "{} driver={} registry={} enabled={} cache=[{}] threads={}",
                        r.id,
                        r.driver,
                        if r.in_registry { "yes" } else { "no" },
                        r.enabled,
                        cache,
                        r.threads.map_or("n/a".to_string(), |n| n.to_string())
                    );
                }
            }
        }
        T3Cmd::Sync { dry_run } => {
            let entries = t3::sync_t3(paths, &root, dry_run)?;
            if dry_run {
                for e in &entries {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&serde_json::json!({
                            "id": e.id,
                            "action": e.action,
                            "before": e.before.as_ref().map(t3::redact),
                            "after": t3::redact(&e.after),
                        }))?
                    );
                }
                println!("dry-run: settings.json unchanged");
            } else {
                for e in &entries {
                    println!("{}: {}", e.id, e.action);
                }
            }
        }
        T3Cmd::Import { dry_run } => {
            let found = t3::import_t3(paths, &root, dry_run)?;
            for f in &found {
                println!("{} {} {} t3=true", f.id, f.harness.as_str(), f.home);
            }
            if found.is_empty() {
                println!("nothing to import");
            } else if dry_run {
                println!("dry-run: registry unchanged");
            }
        }
    }
    Ok(())
}

//! `agentdeck harness list|update|install`: detect harness binaries and run
//! their official updaters/installers. `list` is read-only; `update`/`install`
//! print the official command first and never run it under `--dry-run`.

use agentdeck_core::harness;
use clap::Subcommand;

#[derive(Subcommand)]
pub enum HarnessCmd {
    /// Show detected binary, version and install method per harness.
    List {
        #[arg(long)]
        json: bool,
    },
    /// Run the official updater (`codex update`, `opencode upgrade`, ...).
    Update {
        /// Harness name: claude, codex, opencode or agy.
        name: String,
        #[arg(long)]
        dry_run: bool,
    },
    /// Run the official fresh installer for a harness.
    Install {
        /// Harness name: claude, codex, opencode or agy.
        name: String,
        #[arg(long)]
        dry_run: bool,
    },
}

pub fn run(cmd: &HarnessCmd) -> anyhow::Result<()> {
    match cmd {
        HarnessCmd::List { json } => {
            let rows = harness::list(&harness::real_runner);
            if *json {
                println!("{}", serde_json::to_string_pretty(&rows)?);
            } else {
                for r in &rows {
                    println!(
                        "{}: binary={} version={} ({})",
                        r.name,
                        r.binary
                            .as_deref()
                            .map(|p| p.display().to_string())
                            .as_deref()
                            .unwrap_or("(not found)"),
                        r.version.as_deref().unwrap_or("unknown"),
                        r.install_method
                    );
                }
            }
        }
        HarnessCmd::Update { name, dry_run } => {
            let h = harness::HarnessName::parse(name)?;
            let binary = h.as_str().to_string();
            harness::update(h, &binary, *dry_run, &harness::real_runner)?;
        }
        HarnessCmd::Install { name, dry_run } => {
            let h = harness::HarnessName::parse(name)?;
            harness::install(h, *dry_run, &harness::real_runner)?;
        }
    }
    Ok(())
}

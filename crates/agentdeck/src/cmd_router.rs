//! `agentdeck router json`: refresh the extracted router copy and the
//! generated `router.json`.

use agentdeck_core::Paths;
use agentdeck_core::router;
use clap::Subcommand;

#[derive(Subcommand)]
pub enum RouterCmd {
    /// Refresh `<state_dir>/router/` (embedded files + generated router.json).
    Json {
        #[arg(long)]
        dry_run: bool,
    },
}

pub fn run(paths: &Paths, cmd: RouterCmd) -> anyhow::Result<()> {
    match cmd {
        RouterCmd::Json { dry_run } => {
            let reg = agentdeck_core::Registry::load(paths)?;
            let changes = router::plan_router_files(paths, &reg)?;
            if changes.is_empty() {
                println!("router json: up to date");
                return Ok(());
            }
            for c in &changes {
                println!("{c}", c = c.describe());
            }
            if dry_run {
                println!("dry-run: nothing changed");
                return Ok(());
            }
            agentdeck_core::apply_changes(&changes)?;
            println!("wrote {}", router::router_json_path(paths).display());
        }
    }
    Ok(())
}

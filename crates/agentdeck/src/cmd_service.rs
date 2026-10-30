//! `agentdeck service install/uninstall/status` for the systemd user unit.
//! Thin CLI over [`agentdeck_core::service`]; install never enables/starts.

use agentdeck_core::{Paths, service};
use clap::Subcommand;

#[derive(Subcommand)]
pub enum ServiceCmd {
    /// Render agentdeck-daemon.service and daemon-reload (never enable/start).
    Install {
        #[arg(long)]
        dry_run: bool,
    },
    /// Remove the unit file (refuses while the unit is active).
    Uninstall {
        #[arg(long)]
        dry_run: bool,
    },
    /// Show unit path, installed/active state. Read-only.
    Status,
}

pub fn run(paths: &Paths, cmd: ServiceCmd) -> anyhow::Result<()> {
    match cmd {
        ServiceCmd::Install { dry_run } => {
            let Some(change) = service::plan_install(paths)? else {
                println!("{}: already up to date", service::unit_path()?.display());
                return Ok(());
            };
            println!("{}", change.describe());
            if dry_run {
                println!("dry-run: unit unchanged");
                return Ok(());
            }
            service::apply_install(std::slice::from_ref(&change))?;
            println!("installed {}", service::unit_path()?.display());
            Ok(())
        }
        ServiceCmd::Uninstall { dry_run } => {
            let Some(change) = service::plan_uninstall()? else {
                println!("{}: not installed", service::unit_path()?.display());
                return Ok(());
            };
            println!("{}", change.describe());
            if dry_run {
                println!("dry-run: unit unchanged");
                return Ok(());
            }
            service::apply_uninstall(std::slice::from_ref(&change))?;
            println!("removed {}", service::unit_path()?.display());
            Ok(())
        }
        ServiceCmd::Status => {
            let st = service::status(paths)?;
            println!("unit: {}", st.path.display());
            println!("installed: {}", if st.installed { "yes" } else { "no" });
            if st.installed {
                println!(
                    "up to date: {}",
                    if st.up_to_date {
                        "yes"
                    } else {
                        "no (run `agentdeck service install`)"
                    }
                );
            }
            println!("active: {}", if st.active { "yes" } else { "no" });
            Ok(())
        }
    }
}

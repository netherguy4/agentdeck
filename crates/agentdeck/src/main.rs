use agentdeck_core::{AddSpec, Harness, Paths};
use clap::{Parser, Subcommand, ValueEnum};

mod cmd_pool;

#[derive(Parser)]
#[command(name = "agentdeck", version, about = "Manage harness instances")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Manage registry instances in instances.toml.
    Instance {
        #[command(subcommand)]
        cmd: InstanceCmd,
    },
    /// Manage the [[pool]] model pool in instances.toml.
    Pool {
        #[command(subcommand)]
        cmd: cmd_pool::PoolCmd,
    },
}

#[derive(Subcommand)]
enum InstanceCmd {
    /// List instances.
    List {
        #[arg(long)]
        json: bool,
    },
    /// Add an instance.
    Add {
        #[arg(long)]
        id: String,
        #[arg(long, value_enum)]
        harness: HarnessArg,
        #[arg(long)]
        home: String,
        #[arg(long)]
        display_name: Option<String>,
        #[arg(long)]
        binary: Option<String>,
        #[arg(long)]
        no_t3: bool,
        #[arg(long)]
        dry_run: bool,
    },
    /// Remove an instance from the registry (the home dir is kept).
    Remove {
        id: String,
        #[arg(long)]
        dry_run: bool,
    },
    /// Change instance flags (ids are immutable; there is no rename).
    Set {
        id: String,
        #[arg(long, action = clap::ArgAction::Set)]
        enabled: bool,
        #[arg(long)]
        dry_run: bool,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum HarnessArg {
    Claude,
    Codex,
    Opencode,
    Agy,
}

impl HarnessArg {
    fn as_core(self) -> Harness {
        match self {
            Self::Claude => Harness::Claude,
            Self::Codex => Harness::Codex,
            Self::Opencode => Harness::Opencode,
            Self::Agy => Harness::Agy,
        }
    }
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let paths = Paths::from_env()?;
    match cli.cmd {
        Cmd::Pool { cmd } => cmd_pool::run(&paths, cmd)?,
        Cmd::Instance { cmd } => match cmd {
            InstanceCmd::List { json } => {
                if json {
                    println!("{}", agentdeck_core::registry::instances_json(&paths)?);
                } else {
                    for i in agentdeck_core::registry::list_instances(&paths)? {
                        println!(
                            "{} {} {} enabled={} t3={}",
                            i.id,
                            i.harness.as_str(),
                            i.home,
                            i.enabled,
                            i.t3
                        );
                    }
                }
            }
            InstanceCmd::Add {
                id,
                harness,
                home,
                display_name,
                binary,
                no_t3,
                dry_run,
            } => {
                let changes = agentdeck_core::registry::add_instance(
                    &paths,
                    AddSpec {
                        id: id.clone(),
                        harness: harness.as_core(),
                        display_name,
                        home,
                        binary,
                        no_t3,
                    },
                    dry_run,
                )?;
                for c in &changes {
                    println!("{c}", c = c.describe());
                }
                if dry_run {
                    println!("dry-run: registry unchanged");
                } else {
                    println!("added {id}");
                }
            }
            InstanceCmd::Remove { id, dry_run } => {
                let (changes, home) =
                    agentdeck_core::registry::remove_instance(&paths, &id, dry_run)?;
                for c in &changes {
                    println!("{c}", c = c.describe());
                }
                if dry_run {
                    println!("dry-run: registry unchanged");
                } else {
                    println!("removed {id} (home dir kept: {home})");
                }
            }
            InstanceCmd::Set {
                id,
                enabled,
                dry_run,
            } => {
                let changes = agentdeck_core::registry::set_enabled(&paths, &id, enabled, dry_run)?;
                for c in &changes {
                    println!("{c}", c = c.describe());
                }
                if dry_run {
                    println!("dry-run: registry unchanged");
                } else if changes.is_empty() {
                    println!("{id}: already enabled={enabled}");
                } else {
                    println!("set {id} enabled={enabled}");
                }
            }
        },
    }
    Ok(())
}

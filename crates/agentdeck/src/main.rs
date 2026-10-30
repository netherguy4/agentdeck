use agentdeck_core::{AddSpec, Harness, Paths, t3};
use clap::{Parser, Subcommand, ValueEnum};

mod cmd_claude;
mod cmd_codex;
mod cmd_daemon;
mod cmd_login;
mod cmd_pool;
mod cmd_t3;
mod cmd_usage;
mod cmd_wrapper;

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
    /// Manage [[usage]] quota sources in instances.toml.
    Usage {
        #[command(subcommand)]
        cmd: cmd_usage::UsageCmd,
    },
    /// Run the usage polling daemon (single writer).
    Daemon(cmd_daemon::DaemonArgs),
    /// Sync and drift-check secondary Claude accounts.
    Claude {
        #[command(subcommand)]
        cmd: cmd_claude::ClaudeCmd,
    },
    /// Sync, check and fix Codex shadow homes.
    Codex {
        #[command(subcommand)]
        cmd: cmd_codex::CodexCmd,
    },
    /// T3 integration: status, sync and import of providerInstances.
    T3 {
        #[command(subcommand)]
        cmd: cmd_t3::T3Cmd,
    },
    /// Per-instance terminal wrappers (claude@second) and .desktop launchers.
    Wrapper {
        #[command(subcommand)]
        cmd: cmd_wrapper::WrapperCmd,
    },
    /// Run the official login CLI for an instance inside a PTY.
    Login {
        /// Registry instance id to log in.
        id: String,
        /// Use device-code authorization (codex only).
        #[arg(long)]
        device_auth: bool,
        /// Open the login URL with xdg-open (otherwise only print it).
        #[arg(long)]
        open: bool,
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
    // Installed as a symlink named `agent-process`, the binary is that tool (identical CLI).
    let mut args = std::env::args_os();
    let argv0 = args.next().unwrap_or_default();
    if std::path::Path::new(&argv0).file_name() == Some("agent-process".as_ref()) {
        std::process::exit(agent_process::main(args.collect()));
    }
    let cli = Cli::parse();
    let paths = Paths::from_env()?;
    agentdeck_usage::daemon::load_agentdeck_secrets(&paths.config_dir)?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    match runtime.block_on(async_main(&paths, cli.cmd)) {
        Ok(()) => Ok(()),
        Err(error) => {
            // Single-writer lock and old-service refusal need stable exit codes.
            if error
                .downcast_ref::<agentdeck_usage::daemon::LockHeld>()
                .is_some()
            {
                eprintln!("agentdeck: {error:#}");
                std::process::exit(3);
            }
            if error
                .downcast_ref::<agentdeck_usage::daemon::OldServiceActive>()
                .is_some()
            {
                eprintln!("agentdeck: {error:#}");
                std::process::exit(4);
            }
            Err(error)
        }
    }
}

async fn async_main(paths: &Paths, cmd: Cmd) -> anyhow::Result<()> {
    match cmd {
        Cmd::Pool { cmd } => cmd_pool::run(paths, cmd)?,
        Cmd::Usage { cmd } => cmd_usage::run(paths, cmd).await?,
        Cmd::Daemon(args) => cmd_daemon::run(paths, args).await?,
        Cmd::Wrapper { cmd } => cmd_wrapper::run(paths, cmd)?,
        Cmd::Login {
            id,
            device_auth,
            open,
        } => cmd_login::run(paths, &id, device_auth, open)?,
        Cmd::Claude { cmd } => cmd_claude::run(&cmd)?,
        Cmd::Codex { cmd } => cmd_codex::run(&cmd)?,
        Cmd::T3 { cmd } => cmd_t3::run_t3(paths, cmd)?,
        Cmd::Instance { cmd } => match cmd {
            InstanceCmd::List { json } => {
                if json {
                    println!("{}", agentdeck_core::registry::instances_json(paths)?);
                } else {
                    for i in agentdeck_core::registry::list_instances(paths)? {
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
                    paths,
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
                // Warn when T3 still holds thread history for the id (kept, but orphaned).
                let threads = t3::t3_root()
                    .ok()
                    .and_then(|root| t3::thread_count(&root, &id).unwrap_or(None));
                if let Some(n) = threads.filter(|&n| n > 0) {
                    eprintln!(
                        "warning: T3 keeps {n} thread(s) for {id:?}; history stays but cannot launch"
                    );
                }
                let (changes, home) =
                    agentdeck_core::registry::remove_instance(paths, &id, dry_run)?;
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
                let changes = agentdeck_core::registry::set_enabled(paths, &id, enabled, dry_run)?;
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

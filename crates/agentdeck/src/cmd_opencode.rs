//! `agentdeck opencode check|switch`: account inventory and active-account
//! switch inside one data dir. `instance add` stays registry-only.

use std::path::{Path, PathBuf};

use agentdeck_core::{Harness, Paths, Registry, opencode};
use clap::Subcommand;

#[derive(Subcommand)]
pub enum OpencodeCmd {
    /// Show saved accounts and the active one in the instance data dir.
    Check {
        /// Registry instance id (an account inside the data dir).
        id: String,
    },
    /// Switch the active account (`opencode auth switch`).
    Switch {
        /// Registry instance id whose credential becomes active.
        id: String,
        #[arg(long)]
        dry_run: bool,
    },
}

fn data_dir_of(reg: &Registry, id: &str, home: &Path) -> anyhow::Result<(PathBuf, String)> {
    let inst = reg
        .get(id)
        .ok_or_else(|| anyhow::anyhow!("unknown instance id {id:?}"))?;
    if inst.harness != Harness::Opencode {
        anyhow::bail!("instance {id:?} is not an OpenCode instance");
    }
    let data_dir = expand_tilde(&inst.home, home);
    let binary = inst
        .binary
        .clone()
        .unwrap_or_else(|| "opencode".to_string());
    Ok((data_dir, binary))
}

/// Credential label this instance refers to, from the registry `extra` map.
fn want_credential(reg: &Registry, id: &str) -> Option<String> {
    reg.get(id)?
        .extra
        .get("credential")?
        .as_str()
        .map(str::to_string)
}

pub fn run(cmd: &OpencodeCmd) -> anyhow::Result<()> {
    let paths = Paths::from_env()?;
    let reg = Registry::load(&paths)?;
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| anyhow::anyhow!("cannot resolve home: HOME is not set"))?;
    match cmd {
        OpencodeCmd::Check { id } => {
            let (data_dir, _) = data_dir_of(&reg, id, &home)?;
            let accounts = match opencode::check(&data_dir, want_credential(&reg, id).as_deref()) {
                Ok(drift) if drift.is_empty() => opencode::read_accounts(&data_dir)?,
                Ok(drift) => {
                    for d in &drift {
                        println!("{id}: {}", d.describe());
                    }
                    std::process::exit(1);
                }
                Err(e) => {
                    println!("{id}: cannot read data dir: {e:#}");
                    std::process::exit(1);
                }
            };
            for a in &accounts {
                println!(
                    "{id}: {} [{}] method={}{}",
                    a.label,
                    a.integration,
                    a.method,
                    if a.active { " (active)" } else { "" }
                );
            }
        }
        OpencodeCmd::Switch { id, dry_run } => {
            let (data_dir, binary) = data_dir_of(&reg, id, &home)?;
            let Some(credential) = want_credential(&reg, id) else {
                anyhow::bail!(
                    "instance {id:?} has no credential recorded (registry extra `credential`); \
                     add the account label under [instance.{id}] extra"
                );
            };
            // Isolated data dirs need their own service port (R3); the shared
            // dir talks to the running user service.
            let default_dir = home.join(".local/share/opencode");
            if data_dir != default_dir
                && let Some(port) = reg
                    .get(id)
                    .and_then(|i| i.extra.get("port"))
                    .and_then(|v| v.as_str())
            {
                println!("{id}: isolated data dir, service port {port} (recorded)");
            }
            let argv = opencode::switch_argv(&binary, None, &credential);
            let env = opencode::command_env(&data_dir)?;
            let show_env: Vec<String> = env.iter().map(|(k, v)| format!("{k}={v}")).collect();
            println!("+ {} {}", show_env.join(" "), argv.join(" "));
            if *dry_run {
                println!("dry-run: {id} unchanged");
                return Ok(());
            }
            let mut child = std::process::Command::new(&argv[0]);
            for (k, v) in &env {
                child.env(k, v);
            }
            let status = child.args(&argv[1..]).status()?;
            if !status.success() {
                std::process::exit(status.code().unwrap_or(1));
            }
            println!("switched {id} to {credential:?}");
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

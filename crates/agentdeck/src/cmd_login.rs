//! `agentdeck login <id>`: run the official harness login CLI in a PTY.

use std::io::BufRead as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use agentdeck_core::login::{self, LoginEvent};
use agentdeck_core::registry::Harness;
use agentdeck_core::{Paths, Registry};

pub fn run(paths: &Paths, id: &str, device_auth: bool, open: bool) -> anyhow::Result<()> {
    let reg = Registry::load(paths)?;
    let inst = reg
        .get(id)
        .ok_or_else(|| anyhow::anyhow!("unknown instance id {id:?}"))?;
    if device_auth && inst.harness != Harness::Codex {
        anyhow::bail!("--device-auth is only supported for codex instances");
    }
    let home = expand_home(
        &inst.home,
        std::env::var_os("HOME").map(PathBuf::from).as_deref(),
    )?;
    let mut session = login::start(&login::LoginTarget {
        home,
        harness: inst.harness,
        binary: inst.binary.clone(),
        device_auth,
    })?;
    let mut opened = false;
    let stdin = std::io::stdin();
    let mut stdin = stdin.lock();
    loop {
        let event = session
            .events
            .recv()
            .map_err(|_| anyhow::anyhow!("login process ended without an exit status"))?;
        match event {
            LoginEvent::Url(url) => {
                println!("login url: {url}");
                if open && !opened {
                    opened = true;
                    if let Err(e) = Command::new("xdg-open").arg(&url).spawn() {
                        eprintln!("warning: xdg-open failed: {e:#}");
                    }
                }
            }
            LoginEvent::CodePrompt => {
                // The prompt text itself arrived via Output; read one pasted line.
                let mut line = String::new();
                if stdin.read_line(&mut line)? == 0 {
                    session.cancel();
                    anyhow::bail!("no code pasted (EOF on stdin)");
                }
                let code = line.trim_end_matches(['\r', '\n']).to_string();
                session.send_line(&code)?; // never logged
            }
            LoginEvent::DeviceCode(code) => {
                println!("device code: {code} (enter it in the browser; expires in ~15 minutes)");
            }
            LoginEvent::Output(text) => {
                print!("{text}");
                let _ = <std::io::Stdout as std::io::Write>::flush(&mut std::io::stdout());
            }
            LoginEvent::Exited(code) => {
                if code == 0 {
                    println!("login process exited 0");
                    return Ok(());
                }
                std::process::exit(code as i32);
            }
        }
    }
}

fn expand_home(value: &str, home: Option<&Path>) -> anyhow::Result<PathBuf> {
    if value == "~" || value.starts_with("~/") {
        let Some(home) = home else {
            anyhow::bail!("cannot resolve home {value:?}: HOME is not set");
        };
        return Ok(if value == "~" {
            home.to_path_buf()
        } else {
            home.join(&value[2..])
        });
    }
    Ok(PathBuf::from(value))
}

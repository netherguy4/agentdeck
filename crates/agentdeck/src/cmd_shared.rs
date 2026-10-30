//! `agentdeck skills|instructions|mcp`: shared skills, global instruction
//! files and MCP servers across harnesses. Every write goes through the
//! `Change` list with backups and supports --dry-run; status/list are
//! read-only with secrets redacted.

use std::path::{Path, PathBuf};

use agentdeck_core::{Harness, Paths, Registry, claude, shared};
use clap::Subcommand;

#[derive(Subcommand)]
pub enum SkillsCmd {
    /// Show per-harness skill state. Read-only.
    Status,
    /// Sync shared skills into every harness dir. Read-only with --dry-run.
    Sync {
        #[arg(long)]
        dry_run: bool,
    },
}

#[derive(Subcommand)]
pub enum InstructionsCmd {
    /// Show which global instruction files exist and whether each links to
    /// the shared source. Read-only.
    Status,
    /// Link every global instruction file to the shared ~/AGENTS.md.
    Link {
        #[arg(long)]
        dry_run: bool,
        /// Back up and replace a real file even when contents differ.
        #[arg(long)]
        replace: bool,
    },
}

#[derive(Subcommand)]
pub enum McpCmd {
    /// Show the canonical MCP list (redacted) plus per-target state.
    /// Read-only.
    List,
    /// Sync the canonical list into secondary Claude accounts and the Codex
    /// config. Read-only with --dry-run.
    Sync {
        #[arg(long)]
        dry_run: bool,
        /// Remove Codex servers missing from the canonical list.
        #[arg(long)]
        prune: bool,
    },
}

fn home_dir() -> anyhow::Result<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| anyhow::anyhow!("cannot resolve home: set HOME"))
}

pub fn run_skills(paths: &Paths, cmd: SkillsCmd) -> anyhow::Result<()> {
    let home = home_dir()?;
    match cmd {
        SkillsCmd::Status => {
            for st in shared::skills_status(&home, paths) {
                if st.skill.is_empty() {
                    println!("{}: {}", st.target.display(), st.state.describe());
                } else {
                    println!(
                        "{}/{}: {}",
                        st.target.display(),
                        st.skill,
                        st.state.describe()
                    );
                }
            }
            Ok(())
        }
        SkillsCmd::Sync { dry_run } => {
            let changes = shared::plan_skills_sync(&home, paths)?;
            if changes.is_empty() {
                println!("skills: already in sync");
                return Ok(());
            }
            for c in &changes {
                println!("{}", c.describe());
            }
            if dry_run {
                println!("dry-run: nothing changed");
            } else {
                agentdeck_core::apply_changes(&changes)?;
                println!("synced skills");
            }
            Ok(())
        }
    }
}

pub fn run_instructions(paths: &Paths, cmd: InstructionsCmd) -> anyhow::Result<()> {
    let home = home_dir()?;
    match cmd {
        InstructionsCmd::Status => {
            for st in shared::instructions_status(&home) {
                println!("{}: {}", st.path.display(), st.state.describe());
            }
            Ok(())
        }
        InstructionsCmd::Link { dry_run, replace } => {
            let changes = shared::plan_instructions_link(&home, &paths.state_dir, replace)?;
            if changes.is_empty() {
                println!("instructions: already linked");
                return Ok(());
            }
            for c in &changes {
                println!("{}", c.describe());
            }
            if dry_run {
                println!("dry-run: nothing changed");
            } else {
                agentdeck_core::apply_changes(&changes)?;
                println!("linked instructions");
            }
            Ok(())
        }
    }
}

fn claude_secondaries(paths: &Paths, home: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let reg = Registry::load(paths)?;
    let primary = home.join(".claude");
    let mut out = Vec::new();
    for inst in &reg.instances {
        if inst.harness != Harness::Claude {
            continue;
        }
        let dir = crate::cmd_claude::expand_tilde(&inst.home, home);
        if dir != primary {
            out.push(dir);
        }
    }
    Ok(out)
}

pub fn run_mcp(paths: &Paths, cmd: McpCmd) -> anyhow::Result<()> {
    let home = home_dir()?;
    match cmd {
        McpCmd::List => {
            let secondaries = claude_secondaries(paths, &home)?;
            let (redacted, targets) = shared::mcp_status(&home, paths, &secondaries);
            println!(
                "canonical ({} servers, values redacted):",
                redacted.as_object().map(|m| m.len()).unwrap_or(0)
            );
            if let Some(servers) = redacted.as_object() {
                let mut names: Vec<&String> = servers.keys().collect();
                names.sort();
                for name in names {
                    println!("  {name}");
                }
            }
            for t in &targets {
                println!("{}: {}", t.name, t.detail);
            }
            Ok(())
        }
        McpCmd::Sync { dry_run, prune } => {
            let secondaries = claude_secondaries(paths, &home)?;
            let (changes, skipped) =
                shared::plan_mcp_sync(&home, &paths.state_dir, &secondaries, prune)?;
            for name in &skipped {
                println!("skip {name}: unknown entry shape, left alone");
            }
            if changes.is_empty() {
                println!("mcp: already in sync");
                return Ok(());
            }
            for c in &changes {
                println!("{}", describe_mcp_change(c));
            }
            if dry_run {
                println!("dry-run: nothing changed");
            } else {
                agentdeck_core::apply_changes(&changes)?;
                println!("synced mcp");
            }
            Ok(())
        }
    }
}

fn describe_mcp_change(change: &agentdeck_core::Change) -> String {
    let agentdeck_core::Change::WriteFile {
        path,
        contents,
        old,
        backup,
    } = change
    else {
        return change.describe();
    };
    if path.file_name().is_some_and(|n| n == ".claude.json") {
        return claude::describe_change(change);
    }
    if path.file_name().is_some_and(|n| n == "config.toml") {
        let suffix = match backup {
            Some(dest) => format!(" (backup {})", dest.display()),
            None => String::new(),
        };
        return format!(
            "{}{suffix}",
            shared::describe_codex_change(path, old.as_deref(), contents)
        );
    }
    change.describe()
}

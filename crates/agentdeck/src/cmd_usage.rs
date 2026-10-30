use agentdeck_core::Paths;
use agentdeck_core::usage::{self, UsageEntry, UsageKind};
use clap::{Subcommand, ValueEnum};

#[derive(Subcommand)]
pub enum UsageCmd {
    /// List quota accounts derived from [[usage]] entries (secret names/paths only, never values).
    Accounts {
        #[arg(long)]
        json: bool,
    },
    /// Add a [[usage]] source.
    Add {
        #[arg(long)]
        id: String,
        #[arg(long, value_enum)]
        kind: UsageKindArg,
        #[arg(long)]
        instance: Option<String>,
        #[arg(long)]
        display: Option<String>,
        #[arg(long)]
        config_dir: Option<String>,
        #[arg(long)]
        codex_home: Option<String>,
        #[arg(long)]
        token_file: Option<String>,
        #[arg(long)]
        api_key_env: Option<String>,
        #[arg(long)]
        api_key_file: Option<String>,
        #[arg(long)]
        db_file: Option<String>,
        #[arg(long)]
        dry_run: bool,
    },
    /// Remove a [[usage]] source.
    Remove {
        id: String,
        #[arg(long)]
        dry_run: bool,
    },
}

#[derive(Clone, Copy, ValueEnum)]
pub enum UsageKindArg {
    Claude,
    Codex,
    Antigravity,
    Deepseek,
    Openrouter,
    OpencodeGo,
}

impl UsageKindArg {
    fn as_core(self) -> UsageKind {
        match self {
            Self::Claude => UsageKind::Claude,
            Self::Codex => UsageKind::Codex,
            Self::Antigravity => UsageKind::Antigravity,
            Self::Deepseek => UsageKind::Deepseek,
            Self::Openrouter => UsageKind::Openrouter,
            Self::OpencodeGo => UsageKind::OpencodeGo,
        }
    }
}

fn opt(s: Option<String>) -> Option<String> {
    s.filter(|v| !v.is_empty())
}

pub fn run(paths: &Paths, cmd: UsageCmd) -> anyhow::Result<()> {
    match cmd {
        UsageCmd::Accounts { json } => {
            let reg = agentdeck_core::Registry::load(paths)?;
            let accounts = agentdeck_usage::config::accounts_from_registry(&reg.usage)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&accounts)?);
            } else {
                for a in &accounts {
                    println!("{} {} {}", a.id(), a.provider(), a.name());
                }
            }
        }
        UsageCmd::Add {
            id,
            kind,
            instance,
            display,
            config_dir,
            codex_home,
            token_file,
            api_key_env,
            api_key_file,
            db_file,
            dry_run,
        } => {
            let entry = UsageEntry {
                id: id.clone(),
                kind: kind.as_core(),
                instance: opt(instance),
                display: display.unwrap_or_default(),
                config_dir: opt(config_dir),
                codex_home: opt(codex_home),
                token_file: opt(token_file),
                api_key_env: opt(api_key_env),
                api_key_file: opt(api_key_file),
                db_file: opt(db_file),
                extra: Default::default(),
            };
            let changes = usage::add_usage_entry(paths, entry, dry_run)?;
            for c in &changes {
                println!("{c}", c = c.describe());
            }
            if dry_run {
                println!("dry-run: registry unchanged");
            } else {
                println!("added usage {id}");
            }
        }
        UsageCmd::Remove { id, dry_run } => {
            let changes = usage::remove_usage_entry(paths, &id, dry_run)?;
            for c in &changes {
                println!("{c}", c = c.describe());
            }
            if dry_run {
                println!("dry-run: registry unchanged");
            } else {
                println!("removed usage {id}");
            }
        }
    }
    Ok(())
}

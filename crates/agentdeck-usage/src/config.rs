use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

/// Ниже этого значения refresh становится бессмысленно частым: каждый Codex
/// аккаунт поднимает отдельный subprocess.
pub const MIN_REFRESH_SECONDS: u64 = 15;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "default_refresh_seconds")]
    pub refresh_seconds: u64,
    #[serde(default = "default_stale_seconds")]
    pub stale_seconds: i64,
    #[serde(default)]
    pub accounts: Vec<AccountConfig>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            refresh_seconds: default_refresh_seconds(),
            stale_seconds: default_stale_seconds(),
            accounts: Vec::new(),
        }
    }
}

impl Config {
    pub fn refresh_interval(&self) -> u64 {
        self.refresh_seconds.max(MIN_REFRESH_SECONDS)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "provider", rename_all = "lowercase")]
pub enum AccountConfig {
    Claude {
        id: String,
        name: String,
        config_dir: PathBuf,
        plan: Option<String>,
    },
    Codex {
        id: String,
        name: String,
        codex_home: PathBuf,
        #[serde(default = "default_codex_command")]
        command: String,
        #[serde(default = "default_codex_limit_id")]
        limit_id: String,
    },
    Deepseek {
        id: String,
        name: String,
        api_key_env: String,
        #[serde(default = "default_deepseek_base_url")]
        base_url: String,
    },
    Openrouter {
        id: String,
        name: String,
        api_key_env: String,
        #[serde(default = "default_openrouter_base_url")]
        base_url: String,
    },
    Antigravity {
        id: String,
        name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        token_file: Option<PathBuf>,
    },
    /// Учётные данные читаются из базы OpenCode (`/connect`).
    #[serde(rename = "opencode-go")]
    OpencodeGo {
        id: String,
        name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        db_file: Option<PathBuf>,
    },
}

impl AccountConfig {
    pub fn id(&self) -> &str {
        match self {
            Self::Claude { id, .. }
            | Self::Codex { id, .. }
            | Self::Deepseek { id, .. }
            | Self::Openrouter { id, .. }
            | Self::Antigravity { id, .. }
            | Self::OpencodeGo { id, .. } => id,
        }
    }

    pub fn name(&self) -> &str {
        match self {
            Self::Claude { name, .. }
            | Self::Codex { name, .. }
            | Self::Deepseek { name, .. }
            | Self::Openrouter { name, .. }
            | Self::Antigravity { name, .. }
            | Self::OpencodeGo { name, .. } => name,
        }
    }

    pub fn set_name(&mut self, value: String) {
        match self {
            Self::Claude { name, .. }
            | Self::Codex { name, .. }
            | Self::Deepseek { name, .. }
            | Self::Openrouter { name, .. }
            | Self::Antigravity { name, .. }
            | Self::OpencodeGo { name, .. } => *name = value,
        }
    }

    pub fn provider(&self) -> &'static str {
        match self {
            Self::Claude { .. } => "claude",
            Self::Codex { .. } => "codex",
            Self::Deepseek { .. } => "deepseek",
            Self::Openrouter { .. } => "openrouter",
            Self::Antigravity { .. } => "antigravity",
            Self::OpencodeGo { .. } => "opencode-go",
        }
    }

    /// Каталог, которым аккаунт владеет монопольно. Два аккаунта одного
    /// провайдера не могут его делить: для Claude это привело бы к драке за
    /// один `statusLine`, для Codex — к перепутанным лимитам.
    pub fn exclusive_dir(&self) -> Option<&Path> {
        match self {
            Self::Claude { config_dir, .. } => Some(config_dir),
            Self::Codex { codex_home, .. } => Some(codex_home),
            Self::Deepseek { .. }
            | Self::Openrouter { .. }
            | Self::Antigravity { .. }
            | Self::OpencodeGo { .. } => None,
        }
    }
}

fn default_refresh_seconds() -> u64 {
    120
}

fn default_stale_seconds() -> i64 {
    crate::providers::claude::DEFAULT_STALE_SECONDS
}

fn default_codex_command() -> String {
    "codex".to_owned()
}

fn default_codex_limit_id() -> String {
    "codex".to_owned()
}

fn default_deepseek_base_url() -> String {
    "https://api.deepseek.com".to_owned()
}

fn default_openrouter_base_url() -> String {
    "https://openrouter.ai/api/v1".to_owned()
}

pub fn config_path() -> Result<PathBuf> {
    let base = dirs::config_dir().context("Не удалось определить каталог конфигурации")?;
    Ok(base.join("ai-usage").join("config.toml"))
}

pub fn load() -> Result<Config> {
    let path = config_path()?;
    if !path.exists() {
        return Ok(Config::default());
    }

    let raw = fs::read_to_string(&path)
        .with_context(|| format!("Не удалось прочитать {}", path.display()))?;
    let config: Config =
        toml::from_str(&raw).with_context(|| format!("Некорректный TOML в {}", path.display()))?;
    validate(&config).with_context(|| format!("Некорректная конфигурация в {}", path.display()))?;
    Ok(config)
}

pub fn save(config: &Config) -> Result<PathBuf> {
    validate(config)?;
    let path = config_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let raw = toml::to_string_pretty(config)?;
    crate::util::atomic_write(&path, raw.as_bytes())?;
    Ok(path)
}

/// Проверяет инварианты, которые нельзя выразить типами: уникальность ID и
/// монопольность каталогов провайдеров.
pub fn validate(config: &Config) -> Result<()> {
    let mut seen_ids: HashMap<&str, ()> = HashMap::new();
    let mut seen_dirs: HashMap<(&'static str, PathBuf), &str> = HashMap::new();

    for account in &config.accounts {
        crate::util::validate_id(account.id())
            .with_context(|| format!("Некорректный ID аккаунта '{}'", account.id()))?;

        if seen_ids.insert(account.id(), ()).is_some() {
            bail!("Дублирующийся ID аккаунта: '{}'", account.id());
        }

        if let Some(dir) = account.exclusive_dir() {
            // Сравниваем развёрнутые пути: '~/.codex' и '/home/user/.codex'
            // указывают на один каталог.
            let key = (account.provider(), crate::util::expand_home(dir));
            if let Some(previous) = seen_dirs.insert(key, account.id()) {
                bail!(
                    "Аккаунты '{}' и '{}' используют один каталог {}; для {} нужны отдельные каталоги",
                    previous,
                    account.id(),
                    dir.display(),
                    account.provider()
                );
            }
        }
    }

    if config.stale_seconds <= 0 {
        bail!("stale_seconds должен быть положительным");
    }

    Ok(())
}

pub fn upsert_account(config: &mut Config, account: AccountConfig) {
    if let Some(index) = config
        .accounts
        .iter()
        .position(|item| item.id() == account.id())
    {
        config.accounts[index] = account;
    } else {
        config.accounts.push(account);
    }
}

/// Удаляет аккаунт и возвращает его, если он был.
pub fn remove_account(config: &mut Config, id: &str) -> Option<AccountConfig> {
    let index = config.accounts.iter().position(|item| item.id() == id)?;
    Some(config.accounts.remove(index))
}

pub fn find_account<'a>(config: &'a Config, id: &str) -> Option<&'a AccountConfig> {
    config.accounts.iter().find(|item| item.id() == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claude(id: &str, dir: &str) -> AccountConfig {
        AccountConfig::Claude {
            id: id.to_owned(),
            name: id.to_owned(),
            config_dir: PathBuf::from(dir),
            plan: None,
        }
    }

    fn codex(id: &str, home: &str) -> AccountConfig {
        AccountConfig::Codex {
            id: id.to_owned(),
            name: id.to_owned(),
            codex_home: PathBuf::from(home),
            command: "codex".to_owned(),
            limit_id: "codex".to_owned(),
        }
    }

    fn antigravity(id: &str) -> AccountConfig {
        AccountConfig::Antigravity {
            id: id.to_owned(),
            name: id.to_owned(),
            token_file: None,
        }
    }

    #[test]
    fn round_trips_every_provider_through_toml() {
        let config = Config {
            refresh_seconds: 60,
            stale_seconds: 3_600,
            accounts: vec![
                claude("claude-main", "~/.claude"),
                codex("codex-main", "~/.codex"),
                AccountConfig::Deepseek {
                    id: "deepseek-main".to_owned(),
                    name: "DeepSeek".to_owned(),
                    api_key_env: "AI_USAGE_DEEPSEEK_DEEPSEEK_MAIN".to_owned(),
                    base_url: "https://api.deepseek.com".to_owned(),
                },
                AccountConfig::Openrouter {
                    id: "openrouter-main".to_owned(),
                    name: "OpenRouter".to_owned(),
                    api_key_env: "AI_USAGE_OPENROUTER_OPENROUTER_MAIN".to_owned(),
                    base_url: "https://openrouter.ai/api/v1".to_owned(),
                },
                antigravity("agy-main"),
            ],
        };

        let raw = toml::to_string_pretty(&config).unwrap();
        let parsed: Config = toml::from_str(&raw).unwrap();

        assert_eq!(parsed.accounts.len(), 5);
        assert_eq!(parsed.refresh_seconds, 60);
        assert_eq!(parsed.stale_seconds, 3_600);
        assert_eq!(
            parsed
                .accounts
                .iter()
                .map(AccountConfig::provider)
                .collect::<Vec<_>>(),
            ["claude", "codex", "deepseek", "openrouter", "antigravity"]
        );
        validate(&parsed).unwrap();
    }

    #[test]
    fn example_config_is_valid() {
        // Пример из репозитория должен и разбираться, и проходить валидацию,
        // иначе документация ведёт пользователя в нерабочую конфигурацию.
        let raw = include_str!("../../../config/config.example.toml");
        let config: Config = toml::from_str(raw).unwrap();
        validate(&config).unwrap();
        assert_eq!(config.accounts.len(), 6);
    }

    #[test]
    fn saves_and_loads_round_trip() {
        let unique = format!(
            "ai-usage-test-{}-{:?}.toml",
            std::process::id(),
            std::thread::current().id()
        );
        let temp = std::env::temp_dir().join(unique);

        let config = Config {
            refresh_seconds: 30,
            stale_seconds: 1_800,
            accounts: vec![
                claude("primary", "~/.claude-one"),
                claude("secondary", "~/.claude-two"),
            ],
        };

        crate::util::atomic_write(&temp, toml::to_string_pretty(&config).unwrap().as_bytes())
            .unwrap();

        let raw = fs::read_to_string(&temp).unwrap();
        let loaded: Config = toml::from_str(&raw).unwrap();
        validate(&loaded).unwrap();

        assert_eq!(loaded.accounts.len(), 2);
        assert_eq!(loaded.accounts[0].id(), "primary");
        assert_eq!(loaded.accounts[1].id(), "secondary");

        let _ = fs::remove_file(temp);
    }

    #[test]
    fn rejects_duplicate_account_ids_across_providers() {
        let config = Config {
            accounts: vec![claude("same", "~/.claude"), codex("same", "~/.codex")],
            ..Config::default()
        };
        let error = validate(&config).unwrap_err().to_string();
        assert!(error.contains("Дублирующийся ID"), "{error}");
    }

    #[test]
    fn rejects_two_claude_accounts_sharing_a_config_dir() {
        // Это ломало бы восстановление statusLine при удалении.
        let config = Config {
            accounts: vec![claude("one", "~/.claude"), claude("two", "~/.claude")],
            ..Config::default()
        };
        let error = validate(&config).unwrap_err().to_string();
        assert!(error.contains("один каталог"), "{error}");
    }

    #[test]
    fn rejects_two_codex_accounts_sharing_codex_home() {
        let config = Config {
            accounts: vec![codex("one", "~/.codex"), codex("two", "~/.codex")],
            ..Config::default()
        };
        assert!(validate(&config).is_err());
    }

    #[test]
    fn claude_and_codex_may_share_a_path_value() {
        // Каталоги монопольны в пределах одного провайдера, а не глобально.
        let config = Config {
            accounts: vec![claude("c", "~/.shared"), codex("x", "~/.shared")],
            ..Config::default()
        };
        validate(&config).unwrap();
    }

    #[test]
    fn antigravity_accounts_need_only_unique_ids() {
        let config = Config {
            accounts: vec![antigravity("agy-main"), antigravity("agy-work")],
            ..Config::default()
        };
        validate(&config).unwrap();
    }

    #[test]
    fn antigravity_round_trips_token_file() {
        let config = Config {
            accounts: vec![AccountConfig::Antigravity {
                id: "agy-custom".to_owned(),
                name: "Custom ACP".to_owned(),
                token_file: Some(PathBuf::from("/etc/acp.json")),
            }],
            ..Config::default()
        };
        let raw = toml::to_string(&config).unwrap();
        assert!(raw.contains("token_file = \"/etc/acp.json\""));
        let parsed: Config = toml::from_str(&raw).unwrap();
        assert_eq!(
            parsed.accounts[0],
            AccountConfig::Antigravity {
                id: "agy-custom".to_owned(),
                name: "Custom ACP".to_owned(),
                token_file: Some(PathBuf::from("/etc/acp.json")),
            }
        );
    }

    #[test]
    fn rejects_ids_that_would_escape_the_cache_directory() {
        let config = Config {
            accounts: vec![claude("../../etc", "~/.claude")],
            ..Config::default()
        };
        assert!(validate(&config).is_err());
    }

    #[test]
    fn refresh_interval_has_a_floor() {
        let config = Config {
            refresh_seconds: 1,
            ..Config::default()
        };
        assert_eq!(config.refresh_interval(), MIN_REFRESH_SECONDS);
        let config = Config {
            refresh_seconds: 300,
            ..Config::default()
        };
        assert_eq!(config.refresh_interval(), 300);
    }

    #[test]
    fn rejects_non_positive_stale_seconds() {
        let config = Config {
            stale_seconds: 0,
            ..Config::default()
        };
        assert!(validate(&config).is_err());
    }
}

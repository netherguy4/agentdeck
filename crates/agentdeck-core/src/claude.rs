//! Claude adapter: materialize and drift-check a secondary Claude Code account
//! from the shared primary profile (`~/.claude`).
//!
//! `skills/`, `agents/`, `plugins/` and `CLAUDE.md` in a secondary home are
//! symlinks into the primary. `settings.json` is a real per-instance file:
//! the primary's keys are merged in, secondary-only keys are kept. In
//! `.claude.json` only the top-level `mcpServers` key is managed; every other
//! key (`oauthAccount`, `userID`, `projects`, ...) is left untouched and never
//! printed (account secrets).

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::{Map, Value};

use crate::changes::{Change, backup_dest, plan_write_file};

/// Entries in a secondary home that must be symlinks into the primary.
const LINKS: [&str; 4] = ["skills", "agents", "plugins", "CLAUDE.md"];

#[derive(Debug, Clone, Serialize)]
pub enum Drift {
    Symlink {
        link: PathBuf,
        expected: PathBuf,
        actual: Option<PathBuf>,
    },
    SettingsKey {
        key: String,
        primary: Option<Value>,
        secondary: Option<Value>,
    },
    /// Values are stored redacted (`env`/`headers` entries hidden).
    McpServers {
        primary: Option<Value>,
        secondary: Option<Value>,
    },
}

impl Drift {
    pub fn describe(&self) -> String {
        match self {
            Drift::Symlink {
                link,
                expected,
                actual,
            } => match actual {
                Some(a) => format!(
                    "symlink {} points to {}, expected {}",
                    link.display(),
                    a.display(),
                    expected.display()
                ),
                None => format!(
                    "symlink {} missing or not a symlink, expected {}",
                    link.display(),
                    expected.display()
                ),
            },
            Drift::SettingsKey {
                key,
                primary,
                secondary,
            } => format!(
                "settings.json {key}: secondary={} primary={}",
                opt_json(secondary),
                opt_json(primary)
            ),
            Drift::McpServers { primary, secondary } => format!(
                "{}\n{}",
                "mcpServers differs:",
                diff_mcp_servers(
                    &secondary.clone().unwrap_or(Value::Null),
                    &primary.clone().unwrap_or(Value::Null)
                )
            ),
        }
    }
}

/// Plan the changes that make `home` a secondary account of `primary`.
/// Idempotent: a second run over applied state returns `[]`.
pub fn plan_materialize(
    primary: &Path,
    home: &Path,
    state_dir: &Path,
) -> anyhow::Result<Vec<Change>> {
    let mut changes = Vec::new();
    if fs::symlink_metadata(home).is_err() {
        changes.push(Change::Mkdir {
            path: home.to_path_buf(),
        });
    }
    for name in LINKS {
        let link = home.join(name);
        let target = primary.join(name);
        match fs::symlink_metadata(&link) {
            Ok(m) if m.file_type().is_symlink() => {
                if fs::read_link(&link).ok() != Some(target.clone()) {
                    changes.push(Change::Symlink { link, target });
                }
            }
            // A real file/dir where a symlink belongs is backed up, never dropped.
            Ok(_) => {
                changes.push(Change::BackupRemove {
                    path: link.clone(),
                    backup: backup_dest(state_dir, name),
                });
                changes.push(Change::Symlink { link, target });
            }
            Err(e) if e.kind() == ErrorKind::NotFound => {
                changes.push(Change::Symlink { link, target });
            }
            Err(e) => return Err(e.into()),
        }
    }
    if let Some(c) = plan_settings_merge(primary, home)? {
        changes.push(c);
    }
    if let Some(c) = plan_mcp_merge(primary, home)? {
        changes.push(c);
    }
    Ok(changes)
}

/// List differences between `primary` and `home` without writing anything.
pub fn check(primary: &Path, home: &Path) -> anyhow::Result<Vec<Drift>> {
    let mut out = Vec::new();
    for name in LINKS {
        let link = home.join(name);
        let expected = primary.join(name);
        let actual = match fs::symlink_metadata(&link) {
            Ok(m) if m.file_type().is_symlink() => fs::read_link(&link).ok(),
            _ => None,
        };
        if actual != Some(expected.clone()) {
            out.push(Drift::Symlink {
                link,
                expected,
                actual,
            });
        }
    }
    let (_, pri_settings) = read_json_file(&primary.join("settings.json"))?;
    let (_, sec_settings) = read_json_file(&home.join("settings.json"))?;
    let pri = pri_settings.as_object();
    let sec = sec_settings.as_object();
    let mut keys: Vec<&String> = pri
        .into_iter()
        .flat_map(|m| m.keys())
        .chain(sec.into_iter().flat_map(|m| m.keys()))
        .collect();
    keys.sort();
    keys.dedup();
    for key in keys {
        let p = pri.and_then(|m| m.get(key)).cloned();
        let s = sec.and_then(|m| m.get(key)).cloned();
        if p != s {
            out.push(Drift::SettingsKey {
                key: key.clone(),
                primary: p,
                secondary: s,
            });
        }
    }
    if primary.join(".claude.json").exists() {
        let (_, pri_file) = read_json_file(&primary.join(".claude.json"))?;
        let (_, sec_file) = read_json_file(&home.join(".claude.json"))?;
        let p = pri_file.get("mcpServers").cloned();
        let s = sec_file.get("mcpServers").cloned();
        if p != s {
            out.push(Drift::McpServers {
                primary: p.map(|v| redact_mcp_servers(&v)),
                secondary: s.map(|v| redact_mcp_servers(&v)),
            });
        }
    }
    Ok(out)
}

/// Dry-run printer: like [`Change::describe`], but a `.claude.json` write
/// shows only the redacted `mcpServers` diff; other keys hold account secrets.
pub fn describe_change(change: &Change) -> String {
    if let Change::WriteFile {
        path,
        contents,
        old,
    } = change
        && path.file_name().is_some_and(|n| n == ".claude.json")
    {
        let new_mcp = mcp_of(contents);
        let old_mcp = old.as_deref().map(mcp_of).unwrap_or(Value::Null);
        return format!(
            "write {} (mcpServers only):\n{}",
            path.display(),
            diff_mcp_servers(&old_mcp, &new_mcp)
        );
    }
    change.describe()
}

fn plan_settings_merge(primary: &Path, home: &Path) -> anyhow::Result<Option<Change>> {
    let (sec_bytes, sec) = read_json_file(&home.join("settings.json"))?;
    let (_, pri) = read_json_file(&primary.join("settings.json"))?;
    let Some(pri) = pri.as_object() else {
        anyhow::bail!("primary settings.json is not a JSON object");
    };
    let Some(sec) = sec.as_object() else {
        anyhow::bail!("secondary settings.json is not a JSON object");
    };
    let mut merged = sec.clone();
    for (k, v) in pri {
        merged.insert(k.clone(), v.clone());
    }
    Ok(plan_write_file(
        home.join("settings.json"),
        sec_bytes.as_deref(),
        &to_pretty(&merged)?,
    ))
}

fn plan_mcp_merge(primary: &Path, home: &Path) -> anyhow::Result<Option<Change>> {
    if fs::symlink_metadata(primary.join(".claude.json")).is_err() {
        return Ok(None);
    }
    let (_, pri) = read_json_file(&primary.join(".claude.json"))?;
    let Some(pri) = pri.as_object() else {
        anyhow::bail!("primary .claude.json is not a JSON object");
    };
    let (sec_bytes, sec) = read_json_file(&home.join(".claude.json"))?;
    let Some(sec) = sec.as_object() else {
        anyhow::bail!("secondary .claude.json is not a JSON object");
    };
    let mut merged = sec.clone();
    match pri.get("mcpServers") {
        Some(v) => {
            merged.insert("mcpServers".to_string(), v.clone());
        }
        None => {
            merged.remove("mcpServers");
        }
    }
    if merged == *sec {
        return Ok(None);
    }
    Ok(plan_write_file(
        home.join(".claude.json"),
        sec_bytes.as_deref(),
        &to_pretty(&merged)?,
    ))
}

/// Read a JSON file; a missing file is an empty object (nothing managed yet).
fn read_json_file(path: &Path) -> anyhow::Result<(Option<Vec<u8>>, Value)> {
    match fs::read(path) {
        Err(e) if e.kind() == ErrorKind::NotFound => Ok((None, Value::Object(Map::new()))),
        Err(e) => Err(e.into()),
        Ok(bytes) => {
            let value: Value = serde_json::from_slice(&bytes)
                .map_err(|e| anyhow::anyhow!("cannot parse {}: {e}", path.display()))?;
            Ok((Some(bytes), value))
        }
    }
}

/// Two-space pretty JSON with a trailing newline, matching Claude's own writer.
fn to_pretty(map: &Map<String, Value>) -> anyhow::Result<Vec<u8>> {
    let mut text = serde_json::to_string_pretty(&Value::Object(map.clone()))?;
    text.push('\n');
    Ok(text.into_bytes())
}

fn mcp_of(bytes: &[u8]) -> Value {
    serde_json::from_slice::<Value>(bytes)
        .ok()
        .and_then(|v| v.get("mcpServers").cloned())
        .unwrap_or(Value::Null)
}

/// Clone an `mcpServers` value with `env`/`headers` entries replaced by key
/// names only, so diffs never leak tokens.
fn redact_mcp_servers(value: &Value) -> Value {
    let Value::Object(servers) = value else {
        return value.clone();
    };
    let mut out = Map::new();
    for (name, server) in servers {
        if let Value::Object(fields) = server {
            let mut kept = fields.clone();
            for key in ["env", "headers"] {
                if let Some(Value::Object(entries)) = fields.get(key) {
                    kept.insert(
                        key.to_string(),
                        Value::Object(
                            entries
                                .keys()
                                .map(|k| (k.clone(), Value::String("<redacted>".into())))
                                .collect(),
                        ),
                    );
                }
            }
            out.insert(name.clone(), Value::Object(kept));
        } else {
            out.insert(name.clone(), server.clone());
        }
    }
    Value::Object(out)
}

/// Line-set diff of two redacted `mcpServers` values (same shortcut style as
/// `Change::describe`; files this describes stay small).
fn diff_mcp_servers(old: &Value, new: &Value) -> String {
    let old_text = serde_json::to_string_pretty(&redact_mcp_servers(old)).unwrap_or_default();
    let new_text = serde_json::to_string_pretty(&redact_mcp_servers(new)).unwrap_or_default();
    let old_lines: Vec<&str> = old_text.lines().collect();
    let new_lines: Vec<&str> = new_text.lines().collect();
    let mut out = vec!["mcpServers (env/headers values redacted):".to_string()];
    for line in &old_lines {
        if !new_lines.contains(line) {
            out.push(format!("-{line}"));
        }
    }
    for line in &new_lines {
        if !old_lines.contains(line) {
            out.push(format!("+{line}"));
        }
    }
    if out.len() == 1 && old != new {
        out.push("(only redacted values differ)".to_string());
    }
    out.join("\n")
}

fn opt_json(v: &Option<Value>) -> String {
    v.as_ref()
        .map(|v| serde_json::to_string(v).unwrap_or_else(|_| "?".into()))
        .unwrap_or_else(|| "(absent)".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::changes::apply_changes;
    use std::sync::atomic::{AtomicU64, Ordering};

    static N: AtomicU64 = AtomicU64::new(0);

    struct Guard(PathBuf);
    impl Drop for Guard {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    struct Fx {
        _g: Guard,
        primary: PathBuf,
        home: PathBuf,
        state: PathBuf,
    }

    fn fixture() -> Fx {
        let dir = std::env::temp_dir().join(format!(
            "agentdeck-claude-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = fs::remove_dir_all(&dir);
        let primary = dir.join("primary");
        let home = dir.join("second");
        let state = dir.join("state");
        for name in ["skills", "agents", "plugins"] {
            fs::create_dir_all(primary.join(name)).unwrap();
        }
        fs::write(primary.join("skills/s.txt"), "s").unwrap();
        fs::write(primary.join("CLAUDE.md"), "me").unwrap();
        fs::write(primary.join("settings.json"), "{\"theme\":\"dark\"}\n").unwrap();
        fs::write(
            primary.join(".claude.json"),
            "{\"oauthAccount\":{\"accountUuid\":\"P-UUID\"},\"userID\":\"U1\",\
             \"mcpServers\":{\"srv\":{\"command\":\"x\",\"env\":{\"TOKEN\":\"s3cr3t\"}}},\
             \"projects\":{\"p\":true}}",
        )
        .unwrap();
        Fx {
            _g: Guard(dir),
            primary,
            home,
            state,
        }
    }

    fn json_of(path: &Path) -> Value {
        serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
    }

    #[test]
    fn fresh_materialize_then_idempotent() {
        let fx = fixture();
        let changes = plan_materialize(&fx.primary, &fx.home, &fx.state).unwrap();
        assert!(!changes.is_empty());
        apply_changes(&changes).unwrap();
        for name in LINKS {
            assert_eq!(
                fs::read_link(fx.home.join(name)).unwrap(),
                fx.primary.join(name)
            );
        }
        assert_eq!(json_of(&fx.home.join("settings.json"))["theme"], "dark");
        let sec = json_of(&fx.home.join(".claude.json"));
        assert_eq!(
            sec["mcpServers"],
            json_of(&fx.primary.join(".claude.json"))["mcpServers"]
        );
        // Minimal file: only the managed key.
        assert_eq!(
            sec.as_object().unwrap().keys().collect::<Vec<_>>(),
            vec!["mcpServers"]
        );
        assert!(
            plan_materialize(&fx.primary, &fx.home, &fx.state)
                .unwrap()
                .is_empty()
        );
        assert!(check(&fx.primary, &fx.home).unwrap().is_empty());
    }

    #[test]
    fn settings_drift_both_ways() {
        let fx = fixture();
        apply_changes(&plan_materialize(&fx.primary, &fx.home, &fx.state).unwrap()).unwrap();
        fs::write(
            fx.home.join("settings.json"),
            "{\"theme\":\"light\",\"mine\":1}\n",
        )
        .unwrap();
        let drift = check(&fx.primary, &fx.home).unwrap();
        let keys: Vec<String> = drift
            .iter()
            .filter_map(|d| match d {
                Drift::SettingsKey { key, .. } => Some(key.clone()),
                _ => None,
            })
            .collect();
        assert!(keys.contains(&"theme".to_string()), "{keys:?}");
        assert!(keys.contains(&"mine".to_string()), "{keys:?}");
        apply_changes(&plan_materialize(&fx.primary, &fx.home, &fx.state).unwrap()).unwrap();
        let merged = json_of(&fx.home.join("settings.json"));
        assert_eq!(merged["theme"], "dark");
        assert_eq!(merged["mine"], 1);
        let drift = check(&fx.primary, &fx.home).unwrap();
        assert!(
            drift.iter().any(|d| matches!(
                d,
                Drift::SettingsKey { key, .. } if key == "mine"
            )),
            "secondary-only key is kept and still reported"
        );
    }

    #[test]
    fn mcp_merge_keeps_other_keys() {
        let fx = fixture();
        fs::create_dir_all(&fx.home).unwrap();
        fs::write(
            fx.home.join(".claude.json"),
            "{\"oauthAccount\":{\"accountUuid\":\"S-UUID\"},\"userID\":\"U9\",\
             \"mcpServers\":{\"old\":{}},\"projects\":{\"q\":1}}",
        )
        .unwrap();
        let before = json_of(&fx.home.join(".claude.json"));
        apply_changes(&plan_materialize(&fx.primary, &fx.home, &fx.state).unwrap()).unwrap();
        let after = json_of(&fx.home.join(".claude.json"));
        for key in ["oauthAccount", "userID", "projects"] {
            assert_eq!(after[key], before[key], "key {key} must survive");
        }
        assert_eq!(
            after["mcpServers"],
            json_of(&fx.primary.join(".claude.json"))["mcpServers"]
        );
    }

    #[test]
    fn dry_run_output_is_redacted() {
        let fx = fixture();
        apply_changes(&plan_materialize(&fx.primary, &fx.home, &fx.state).unwrap()).unwrap();
        fs::write(
            fx.primary.join(".claude.json"),
            "{\"oauthAccount\":{\"accountUuid\":\"P-NEW\"},\
             \"mcpServers\":{\"srv\":{\"command\":\"y\",\"env\":{\"TOKEN\":\"n3w\"}},\
             \"extra\":{\"env\":{\"K\":\"v\"}}}}",
        )
        .unwrap();
        let changes = plan_materialize(&fx.primary, &fx.home, &fx.state).unwrap();
        let text = changes
            .iter()
            .map(describe_change)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("<redacted>"), "{text}");
        assert!(text.contains("extra"), "{text}");
        for secret in ["s3cr3t", "n3w", "P-UUID", "P-NEW"] {
            assert!(!text.contains(secret), "leak in {text}");
        }
    }

    #[test]
    fn real_dir_is_backed_up_then_linked() {
        let fx = fixture();
        fs::create_dir_all(fx.home.join("skills")).unwrap();
        fs::write(fx.home.join("skills/keep.txt"), "keep").unwrap();
        let changes = plan_materialize(&fx.primary, &fx.home, &fx.state).unwrap();
        assert!(
            changes
                .iter()
                .any(|c| matches!(c, Change::BackupRemove { .. })),
            "{changes:?}"
        );
        apply_changes(&changes).unwrap();
        assert_eq!(
            fs::read_link(fx.home.join("skills")).unwrap(),
            fx.primary.join("skills")
        );
        let backups: Vec<PathBuf> = fs::read_dir(fx.state.join("backups"))
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        assert_eq!(backups.len(), 1);
        assert_eq!(fs::read(backups[0].join("keep.txt")).unwrap(), b"keep");
        assert!(
            plan_materialize(&fx.primary, &fx.home, &fx.state)
                .unwrap()
                .is_empty()
        );
    }
}

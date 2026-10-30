//! `[[pool]]` model pool: `(instance, model, options)` entries with an
//! offline warning when the model is absent from the T3 instance cache.

use std::collections::BTreeMap;
use std::env;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::changes::{Change, plan_write_file};
use crate::paths::Paths;
use crate::registry::Registry;

fn default_true() -> bool {
    true
}

/// Pool option value: only string|bool are accepted.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PoolValue {
    Bool(bool),
    String(String),
}

impl std::fmt::Display for PoolValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Bool(b) => write!(f, "{b}"),
            Self::String(s) => write!(f, "{s}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PoolEntry {
    pub instance: String,
    pub model: String,
    #[serde(default)]
    pub options: BTreeMap<String, PoolValue>,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

impl PoolEntry {
    pub fn key_eq(&self, other: &Self) -> bool {
        self.instance == other.instance
            && self.model == other.model
            && self.options == other.options
    }

    pub fn describe(&self) -> String {
        let mut s = format!("{} {}", self.instance, self.model);
        for (k, v) in &self.options {
            s.push_str(&format!(" {k}={v}"));
        }
        s.push_str(if self.enabled {
            " enabled=true"
        } else {
            " enabled=false"
        });
        s
    }
}

/// Parse one `--option k=v` arg; "true"/"false" become bool, else string.
pub fn parse_option(arg: &str) -> anyhow::Result<(String, PoolValue)> {
    let (k, v) = arg
        .split_once('=')
        .ok_or_else(|| anyhow::anyhow!("bad option {arg:?}: expected k=v"))?;
    if k.is_empty() {
        anyhow::bail!("bad option {arg:?}: empty key");
    }
    let value = match v {
        "true" => PoolValue::Bool(true),
        "false" => PoolValue::Bool(false),
        _ => PoolValue::String(v.to_string()),
    };
    Ok((k.to_string(), value))
}

pub fn parse_options(args: &[String]) -> anyhow::Result<BTreeMap<String, PoolValue>> {
    let mut map = BTreeMap::new();
    for a in args {
        let (k, v) = parse_option(a)?;
        map.insert(k, v);
    }
    Ok(map)
}

/// T3 home for cache reads only: `$AGENTDECK_T3_DIR` or `~/.t3`.
pub fn t3_dir() -> Option<PathBuf> {
    if let Some(d) = env::var_os("AGENTDECK_T3_DIR") {
        return Some(PathBuf::from(d));
    }
    env::var_os("HOME").map(|h| PathBuf::from(h).join(".t3"))
}

/// Warn (not fail) when `<t3dir>/caches/<instance>.json` exists and lists no
/// such model slug. Missing/unreadable files mean "no data": no warning.
pub fn check_model(t3dir: Option<&Path>, instance: &str, model: &str) -> Option<String> {
    match t3dir {
        Some(d) => check_model_in(d, instance, model),
        None => check_model_in(&t3_owned()?, instance, model),
    }
}

fn t3_owned() -> Option<PathBuf> {
    t3_dir()
}

fn check_model_in(t3dir: &Path, instance: &str, model: &str) -> Option<String> {
    let raw = std::fs::read(t3dir.join("caches").join(format!("{instance}.json"))).ok()?;
    let v: serde_json::Value = serde_json::from_slice(&raw).ok()?;
    let models = v.get("models")?.as_array()?;
    let slugs: Vec<&str> = models
        .iter()
        .filter_map(|m| m.get("slug")?.as_str())
        .collect();
    if slugs.contains(&model) {
        return None;
    }
    let checked = v
        .get("checkedAt")
        .and_then(|c| c.as_str())
        .unwrap_or("unknown time");
    Some(format!(
        "model {model:?} not in T3 cache for instance {instance:?} (checkedAt {checked})"
    ))
}

impl Registry {
    pub fn pool(&self) -> &[PoolEntry] {
        &self.pool
    }

    pub fn add_pool(&mut self, entry: PoolEntry) -> anyhow::Result<Option<String>> {
        if self.get(&entry.instance).is_none() {
            anyhow::bail!("unknown instance id {:?}", entry.instance);
        }
        if entry.model.is_empty() {
            anyhow::bail!("model must not be empty");
        }
        if self.pool.iter().any(|e| e.key_eq(&entry)) {
            anyhow::bail!("duplicate pool entry {:?}", entry.describe());
        }
        let warn = check_model(None, &entry.instance, &entry.model);
        self.pool.push(entry);
        self.validate()?;
        Ok(warn)
    }

    pub fn remove_pool(
        &mut self,
        instance: &str,
        model: &str,
        options: &BTreeMap<String, PoolValue>,
    ) -> anyhow::Result<PoolEntry> {
        let pos = self
            .pool
            .iter()
            .position(|e| e.instance == instance && e.model == model && &e.options == options)
            .ok_or_else(|| anyhow::anyhow!("unknown pool entry {instance:?} {model:?}"))?;
        Ok(self.pool.remove(pos))
    }

    pub fn set_pool_enabled(
        &mut self,
        instance: &str,
        model: &str,
        options: &BTreeMap<String, PoolValue>,
        enabled: bool,
    ) -> anyhow::Result<()> {
        let e = self
            .pool
            .iter_mut()
            .find(|e| e.instance == instance && e.model == model && &e.options == options)
            .ok_or_else(|| anyhow::anyhow!("unknown pool entry {instance:?} {model:?}"))?;
        e.enabled = enabled;
        Ok(())
    }
}

pub fn list_pool(paths: &Paths) -> anyhow::Result<Vec<PoolEntry>> {
    Ok(Registry::load(paths)?.pool)
}

pub fn pool_json(paths: &Paths) -> anyhow::Result<String> {
    Ok(serde_json::to_string_pretty(&list_pool(paths)?)?)
}

fn preview_pool(
    paths: &Paths,
    mutate: impl FnOnce(&mut Registry) -> anyhow::Result<Option<String>>,
) -> anyhow::Result<(Vec<Change>, Option<String>)> {
    let mut reg = Registry::load(paths)?;
    let warn = mutate(&mut reg)?;
    reg.validate()?;
    let new_text = reg.render()?;
    let old = std::fs::read(paths.registry_file()).ok();
    let changes = plan_write_file(paths.registry_file(), old.as_deref(), new_text.as_bytes())
        .map(|c| vec![c])
        .unwrap_or_default();
    Ok((changes, warn))
}

pub fn add_pool_entry(
    paths: &Paths,
    entry: PoolEntry,
    dry_run: bool,
) -> anyhow::Result<(Vec<Change>, Option<String>)> {
    let (changes, warn) = preview_pool(paths, |reg| reg.add_pool(entry.clone()))?;
    if changes.is_empty() {
        anyhow::bail!("pool entry {:?} already exists", entry.describe());
    }
    if !dry_run {
        Registry::update(paths, |reg| {
            reg.add_pool(entry)?;
            Ok(())
        })?;
    }
    Ok((changes, warn))
}

pub fn remove_pool_entry(
    paths: &Paths,
    instance: &str,
    model: &str,
    options: &BTreeMap<String, PoolValue>,
    dry_run: bool,
) -> anyhow::Result<Vec<Change>> {
    Registry::load(paths)?
        .pool
        .iter()
        .find(|e| e.instance == instance && e.model == model && &e.options == options)
        .ok_or_else(|| anyhow::anyhow!("unknown pool entry {instance:?} {model:?}"))?;
    let (changes, _) = preview_pool(paths, |reg| {
        reg.remove_pool(instance, model, options)?;
        Ok(None)
    })?;
    if !dry_run {
        Registry::update(paths, |reg| {
            reg.remove_pool(instance, model, options)?;
            Ok(())
        })?;
    }
    Ok(changes)
}

pub fn set_pool_enabled(
    paths: &Paths,
    instance: &str,
    model: &str,
    options: &BTreeMap<String, PoolValue>,
    enabled: bool,
    dry_run: bool,
) -> anyhow::Result<Vec<Change>> {
    let (changes, _) = preview_pool(paths, |reg| {
        reg.set_pool_enabled(instance, model, options, enabled)?;
        Ok(None)
    })?;
    if !dry_run {
        if changes.is_empty() {
            Registry::load(paths)?
                .pool
                .iter()
                .find(|e| e.instance == instance && e.model == model && &e.options == options)
                .ok_or_else(|| anyhow::anyhow!("unknown pool entry {instance:?} {model:?}"))?;
            return Ok(vec![]);
        }
        Registry::update(paths, |reg| {
            reg.set_pool_enabled(instance, model, options, enabled)?;
            Ok(())
        })?;
    }
    Ok(changes)
}

/// One favorite from T3 `settings.json`: provider id, exact model, options.
#[derive(Debug, Clone)]
pub struct Favorite {
    pub provider: String,
    pub model: String,
    pub options: BTreeMap<String, PoolValue>,
    pub enabled: bool,
}

fn favorite_enabled(
    instances: Option<&serde_json::Map<String, serde_json::Value>>,
    legacy: Option<&serde_json::Map<String, serde_json::Value>>,
    provider: &str,
) -> bool {
    instances
        .and_then(|m| m.get(provider))
        .and_then(|p| p.get("enabled"))
        .and_then(|e| e.as_bool())
        .or_else(|| {
            legacy
                .and_then(|m| m.get(provider))
                .and_then(|p| p.get("enabled"))
                .and_then(|e| e.as_bool())
        })
        .unwrap_or(true)
}

/// Read T3 favorites (read-only; never edited). `t3_root` is injectable so
/// tests use a temp dir; production passes `t3::t3_root()?`.
pub fn read_favorites(t3_root: &Path) -> anyhow::Result<Vec<Favorite>> {
    let file = t3_root.join("userdata/settings.json");
    let raw =
        std::fs::read(&file).map_err(|e| anyhow::anyhow!("cannot read {}: {e}", file.display()))?;
    let v: serde_json::Value = serde_json::from_slice(&raw)
        .map_err(|e| anyhow::anyhow!("T3 settings.json is not valid JSON: {e}"))?;
    let favs = v
        .get("favorites")
        .and_then(|f| f.as_array())
        .ok_or_else(|| anyhow::anyhow!("T3 settings.json has no favorites array"))?;
    let instances = v.get("providerInstances").and_then(|p| p.as_object());
    let legacy = v.get("providers").and_then(|p| p.as_object());
    let mut out = Vec::new();
    for fav in favs {
        let (Some(provider), Some(model)) = (
            fav.get("provider").and_then(|p| p.as_str()),
            fav.get("model").and_then(|m| m.as_str()),
        ) else {
            continue;
        };
        if provider.is_empty() || model.is_empty() {
            continue;
        }
        let mut options = BTreeMap::new();
        if let Some(map) = fav.get("options").and_then(|o| o.as_object()) {
            for (k, val) in map {
                match val {
                    serde_json::Value::Bool(b) => {
                        options.insert(k.clone(), PoolValue::Bool(*b));
                    }
                    serde_json::Value::String(s) => {
                        options.insert(k.clone(), PoolValue::String(s.clone()));
                    }
                    _ => {}
                }
            }
        }
        out.push(Favorite {
            provider: provider.to_string(),
            model: model.to_string(),
            options,
            enabled: favorite_enabled(instances, legacy, provider),
        });
    }
    Ok(out)
}

/// Read-only T3 favorites import into `[[pool]]`: exact instance ids, models
/// and options (`enabled` follows `providerInstances`); the T3 favorites are
/// never edited. Idempotent: a second run adds nothing. Returns the registry
/// change plus the added count and skip notes.
pub fn import_favorites(
    paths: &Paths,
    t3_root: &Path,
    dry_run: bool,
) -> anyhow::Result<(Vec<Change>, usize, Vec<String>)> {
    let settings = t3_root.join("userdata/settings.json");
    let raw = std::fs::read(&settings)
        .map_err(|e| anyhow::anyhow!("cannot read {}: {e}", settings.display()))?;
    let v: serde_json::Value = serde_json::from_slice(&raw)
        .map_err(|e| anyhow::anyhow!("T3 settings.json is not valid JSON: {e}"))?;
    let favs = v
        .get("favorites")
        .and_then(|f| f.as_array())
        .ok_or_else(|| anyhow::anyhow!("T3 settings.json has no favorites array"))?;
    let instances = v
        .get("providerInstances")
        .and_then(|p| p.as_object())
        .ok_or_else(|| anyhow::anyhow!("T3 settings.json has no providerInstances object"))?;
    let legacy = v.get("providers").and_then(|p| p.as_object());
    let mut reg = Registry::load(paths)?;
    let mut added = 0;
    let mut skipped = Vec::new();
    for fav in favs {
        let (Some(provider), Some(model)) = (
            fav.get("provider").and_then(|p| p.as_str()),
            fav.get("model").and_then(|m| m.as_str()),
        ) else {
            skipped.push("favorite without provider/model".to_string());
            continue;
        };
        if provider.is_empty() || model.is_empty() {
            skipped.push(format!("{provider}/{model}: empty provider or model"));
            continue;
        }
        let enabled = instances
            .get(provider)
            .and_then(|p| p.get("enabled"))
            .and_then(|e| e.as_bool())
            .or_else(|| {
                legacy
                    .and_then(|l| l.get(provider))
                    .and_then(|p| p.get("enabled"))
                    .and_then(|e| e.as_bool())
            })
            .unwrap_or(true);
        let mut options = BTreeMap::new();
        if let Some(map) = fav.get("options").and_then(|o| o.as_object()) {
            for (k, val) in map {
                match val {
                    serde_json::Value::Bool(b) => {
                        options.insert(k.clone(), PoolValue::Bool(*b));
                    }
                    serde_json::Value::String(s) => {
                        options.insert(k.clone(), PoolValue::String(s.clone()));
                    }
                    _ => {}
                }
            }
        }
        let entry = PoolEntry {
            instance: provider.to_string(),
            model: model.to_string(),
            options,
            enabled,
        };
        if reg.pool.iter().any(|e| e.key_eq(&entry)) {
            continue;
        }
        if reg.get(&entry.instance).is_none() {
            skipped.push(format!(
                "{} {}: unknown instance (add it with `agentdeck t3 import` first)",
                entry.instance, entry.model
            ));
            continue;
        }
        reg.pool.push(entry);
        added += 1;
    }
    reg.validate()?;
    let new_text = reg.render()?;
    let old = std::fs::read(paths.registry_file()).ok();
    let changes = plan_write_file(paths.registry_file(), old.as_deref(), new_text.as_bytes())
        .map(|c| vec![c])
        .unwrap_or_default();
    if !dry_run {
        Registry::update(paths, |reg| {
            let raw = std::fs::read(&settings)
                .map_err(|e| anyhow::anyhow!("cannot read {}: {e}", settings.display()))?;
            let v: serde_json::Value = serde_json::from_slice(&raw)
                .map_err(|e| anyhow::anyhow!("T3 settings.json is not valid JSON: {e}"))?;
            let favs = v
                .get("favorites")
                .and_then(|f| f.as_array())
                .ok_or_else(|| anyhow::anyhow!("T3 settings.json has no favorites array"))?;
            for fav in favs {
                let (Some(provider), Some(model)) = (
                    fav.get("provider").and_then(|p| p.as_str()),
                    fav.get("model").and_then(|m| m.as_str()),
                ) else {
                    continue;
                };
                let enabled = v
                    .get("providerInstances")
                    .and_then(|p| p.get(provider))
                    .and_then(|p| p.get("enabled"))
                    .and_then(|e| e.as_bool())
                    .unwrap_or(true);
                let entry = PoolEntry {
                    instance: provider.to_string(),
                    model: model.to_string(),
                    options: BTreeMap::new(),
                    enabled,
                };
                if reg.pool.iter().any(|e| e.key_eq(&entry)) {
                    continue;
                }
                if reg.get(&entry.instance).is_none() || entry.model.is_empty() {
                    continue;
                }
                reg.pool.push(entry);
            }
            reg.validate()?;
            Ok(())
        })?;
    }
    Ok((changes, added, skipped))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{AddSpec, Harness};
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    static N: AtomicU64 = AtomicU64::new(100);

    struct Guard(PathBuf);
    impl Drop for Guard {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn test_paths(tag: &str) -> (Paths, Guard) {
        let dir = std::env::temp_dir().join(format!(
            "agentdeck-pool-{tag}-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = fs::remove_dir_all(&dir);
        (
            Paths::new(dir.join("config"), dir.join("state")),
            Guard(dir),
        )
    }

    fn spec(id: &str) -> AddSpec {
        AddSpec {
            id: id.to_string(),
            harness: Harness::Claude,
            display_name: None,
            home: format!("/fake/{id}"),
            binary: None,
            no_t3: false,
        }
    }

    fn entry(instance: &str, model: &str) -> PoolEntry {
        PoolEntry {
            instance: instance.into(),
            model: model.into(),
            options: BTreeMap::new(),
            enabled: true,
        }
    }

    #[test]
    fn round_trip_without_pool() {
        let (paths, _g) = test_paths("nopol");
        Registry::update(&paths, |r| r.add(spec("a"))).unwrap();
        let raw = fs::read_to_string(paths.registry_file()).unwrap();
        assert!(!raw.contains("[[pool]]"), "{raw}");
        let reg = Registry::load(&paths).unwrap();
        assert!(reg.pool.is_empty());
    }

    #[test]
    fn round_trip_with_pool() {
        let (paths, _g) = test_paths("roundtrip");
        Registry::update(&paths, |r| {
            r.add(spec("a"))?;
            let mut e = entry("a", "owner/model");
            e.options
                .insert("effort".into(), PoolValue::String("high".into()));
            e.enabled = false;
            r.pool.push(e);
            Ok(())
        })
        .unwrap();
        let raw = fs::read_to_string(paths.registry_file()).unwrap();
        assert!(raw.contains("[[pool]]"), "{raw}");
        let reg = Registry::load(&paths).unwrap();
        assert_eq!(reg.pool.len(), 1);
        assert_eq!(
            reg.pool[0].options["effort"],
            PoolValue::String("high".into())
        );
        assert!(!reg.pool[0].enabled);
        // enabled defaults to true when omitted
        let reg2 = Registry::from_toml(
            "version = 1\n[[instance]]\nid = \"a\"\nharness = \"claude\"\nhome = \"h\"\n\
             [[pool]]\ninstance = \"a\"\nmodel = \"m\"\n",
        )
        .unwrap();
        assert!(reg2.pool[0].enabled);
    }

    #[test]
    fn uniqueness_and_unknown_instance() {
        let (paths, _g) = test_paths("uniq");
        Registry::update(&paths, |r| r.add(spec("a"))).unwrap();
        let mut reg = Registry::load(&paths).unwrap();
        reg.add_pool(entry("a", "m")).unwrap();
        assert!(reg.add_pool(entry("a", "m")).is_err());
        assert!(reg.add_pool(entry("nope", "m")).is_err());
        // same instance+model but different options is fine
        let mut e2 = entry("a", "m");
        e2.options
            .insert("effort".into(), PoolValue::String("low".into()));
        reg.add_pool(e2).unwrap();
        assert!(
            Registry::from_toml("version = 1\n[[pool]]\ninstance = \"a\"\nmodel = \"m\"\n")
                .is_err()
        );
    }

    #[test]
    fn instance_removal_refused_with_pool() {
        let (paths, _g) = test_paths("refuse");
        Registry::update(&paths, |r| {
            r.add(spec("a"))?;
            r.add_pool(entry("a", "m"))?;
            Ok(())
        })
        .unwrap();
        let err = Registry::update(&paths, |r| {
            r.remove("a")?;
            Ok(())
        })
        .unwrap_err()
        .to_string();
        assert!(err.contains("pool"), "{err}");
        assert!(Registry::load(&paths).unwrap().get("a").is_some());
    }

    #[test]
    fn cache_warning() {
        let dir = std::env::temp_dir().join(format!(
            "agentdeck-t3-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("caches")).unwrap();
        fs::write(
            dir.join("caches/a.json"),
            r#"{"models":[{"slug":"yes-model"}],"checkedAt":"2026-01-01"}"#,
        )
        .unwrap();
        assert!(check_model_in(&dir, "a", "yes-model").is_none());
        let w = check_model_in(&dir, "a", "no-model").unwrap();
        assert!(w.contains("no-model"), "{w}");
        assert!(check_model_in(&dir, "missing", "m").is_none());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn dry_run_writes_nothing() {
        let (paths, _g) = test_paths("dry");
        Registry::update(&paths, |r| r.add(spec("a"))).unwrap();
        let before = fs::read(paths.registry_file()).unwrap();
        let (changes, _) = add_pool_entry(&paths, entry("a", "m"), true).unwrap();
        assert_eq!(changes.len(), 1);
        assert_eq!(fs::read(paths.registry_file()).unwrap(), before);
        let (changes, _) = add_pool_entry(&paths, entry("a", "m"), false).unwrap();
        assert_eq!(changes.len(), 1);
        let changes = remove_pool_entry(&paths, "a", "m", &BTreeMap::new(), true).unwrap();
        assert_eq!(changes.len(), 1);
        assert!(
            Registry::load(&paths)
                .unwrap()
                .pool
                .iter()
                .any(|e| e.model == "m")
        );
    }

    #[test]
    fn import_favorites_adds_once_and_skips_unknown() {
        let (paths, _g) = test_paths("import");
        let t3 = _g.0.join("t3");
        std::fs::create_dir_all(t3.join("userdata")).unwrap();
        std::fs::write(
            t3.join("userdata/settings.json"),
            serde_json::json!({
                "favorites": [
                    {"provider": "codex", "model": "gpt-x"},
                    {"provider": "ghost", "model": "m"},
                ],
                "providerInstances": {"codex": {"enabled": true}},
            })
            .to_string(),
        )
        .unwrap();
        Registry::update(&paths, |r| r.add(spec("codex"))).unwrap();
        let (changes, added, skipped) = import_favorites(&paths, &t3, true).unwrap();
        assert_eq!(added, 1);
        assert_eq!(changes.len(), 1);
        assert_eq!(skipped.len(), 1);
        // Dry run writes nothing.
        assert!(Registry::load(&paths).unwrap().pool.is_empty());
        let (_, added, _) = import_favorites(&paths, &t3, false).unwrap();
        assert_eq!(added, 1);
        assert_eq!(Registry::load(&paths).unwrap().pool.len(), 1);
        // A second run adds nothing and plans no changes.
        let (changes2, added2, _) = import_favorites(&paths, &t3, false).unwrap();
        assert_eq!(added2, 0);
        assert!(changes2.is_empty());
    }

    #[test]
    fn option_parsing() {
        assert_eq!(
            parse_option("effort=high").unwrap().1,
            PoolValue::String("high".into())
        );
        assert_eq!(parse_option("x=true").unwrap().1, PoolValue::Bool(true));
        assert_eq!(
            parse_option("x=False").unwrap().1,
            PoolValue::String("False".into())
        );
        assert!(parse_option("noeq").is_err());
    }
}

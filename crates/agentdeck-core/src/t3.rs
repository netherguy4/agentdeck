//! T3 integration: validated atomic merge-write of `providerInstances` in
//! `~/.t3/userdata/settings.json`, plus read-only health and thread counts.
//!
//! T3 itself is not managed: this module only edits `providerInstances` entries
//! for registry instances with `t3 = true` (validated, atomic, verified) and
//! reads `caches/<id>.json` and `statev2.sqlite` read-only. Registry instance
//! ids are never renamed (T3 thread history is bound to them). T3 root is
//! injectable: every function takes an explicit `t3_root` (production value is
//! [`t3_root()`); tests use temp dirs.

use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags};
use serde_json::{Map, Value};

use crate::Paths;
use crate::registry::{Harness, Instance, Registry};

/// Production T3 home: `$AGENTDECK_T3_DIR`, else `~/.t3`.
pub fn t3_root() -> anyhow::Result<PathBuf> {
    if let Some(dir) = std::env::var_os("AGENTDECK_T3_DIR") {
        return Ok(PathBuf::from(dir));
    }
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| anyhow::anyhow!("cannot resolve T3 dir: set HOME or AGENTDECK_T3_DIR"))?;
    Ok(home.join(".t3"))
}

pub fn settings_file(t3_root: &Path) -> PathBuf {
    t3_root.join("userdata/settings.json")
}

fn caches_dir(t3_root: &Path) -> PathBuf {
    t3_root.join("caches")
}

fn state_db(t3_root: &Path) -> PathBuf {
    t3_root.join("userdata/statev2.sqlite")
}

/// Config field names agentdeck may write, per driver (R4 tables). `None` =
/// a driver agentdeck never writes (its instances are left untouched).
fn allowed_config_fields(driver: &str) -> Option<&'static [&'static str]> {
    match driver {
        "claudeAgent" => Some(&[
            "enabled",
            "binaryPath",
            "homePath",
            "customModels",
            "launchArgs",
            "autoCompactWindow",
            "idleCompaction",
        ]),
        "codex" => Some(&[
            "enabled",
            "binaryPath",
            "homePath",
            "shadowHomePath",
            "launchArgs",
            "setupMode",
            "customModels",
        ]),
        _ => None,
    }
}

/// Reject config keys agentdeck must not write (unknown per the R4 driver
/// table, or secret-carrying: secrets go through T3's own login/UI flows).
pub fn validate_config_fields(driver: &str, config: &Map<String, Value>) -> anyhow::Result<()> {
    let allowed = allowed_config_fields(driver)
        .ok_or_else(|| anyhow::anyhow!("no T3 mapping for driver {driver:?}: refusing to write"))?;
    for key in config.keys() {
        if !allowed.contains(&key.as_str()) {
            anyhow::bail!(
                "refusing to write unknown T3 config field {key:?} for driver {driver:?}"
            );
        }
        if is_secret_key(key) {
            anyhow::bail!("refusing to write secret field {key:?} for driver {driver:?}");
        }
    }
    Ok(())
}

fn is_secret_key(key: &str) -> bool {
    let lower = key.to_lowercase();
    ["secret", "password", "apikey", "api_key", "token"]
        .iter()
        .any(|s| lower.contains(s))
}

fn check_t3_slug(id: &str) -> anyhow::Result<()> {
    if id.is_empty()
        || id.len() > 64
        || !id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        anyhow::bail!("bad T3 instance id {id:?}: must match [A-Za-z0-9_-]+");
    }
    Ok(())
}

/// Desired T3 envelope for one registry instance. `main_codex_home` is the
/// home of the registry instance id `codex` when present.
fn desired_envelope(inst: &Instance, main_codex_home: Option<&str>) -> anyhow::Result<Value> {
    check_t3_slug(&inst.id)?;
    let driver = match inst.harness {
        Harness::Claude => "claudeAgent",
        Harness::Codex => "codex",
        other => anyhow::bail!(
            "instance {:?} uses harness {}: no T3 mapping, left untouched",
            inst.id,
            other.as_str()
        ),
    };
    let mut config = Map::new();
    match inst.harness {
        Harness::Claude => {
            config.insert("homePath".into(), Value::String(inst.home.clone()));
            if let Some(bin) = &inst.binary {
                config.insert("binaryPath".into(), Value::String(bin.clone()));
            }
        }
        Harness::Codex => {
            let main = main_codex_home.unwrap_or(&inst.home);
            config.insert("homePath".into(), Value::String(main.to_string()));
            if inst.home != main {
                config.insert("shadowHomePath".into(), Value::String(inst.home.clone()));
            }
            if let Some(bin) = &inst.binary {
                config.insert("binaryPath".into(), Value::String(bin.clone()));
            }
        }
        _ => unreachable!("bailed above"),
    }
    validate_config_fields(driver, &config)?;
    let display_name = if inst.display_name.is_empty() {
        inst.id.clone()
    } else {
        inst.display_name.clone()
    };
    let mut env = Map::new();
    env.insert("driver".into(), Value::String(driver.into()));
    env.insert("displayName".into(), Value::String(display_name));
    env.insert("enabled".into(), Value::Bool(inst.enabled));
    env.insert("config".into(), Value::Object(config));
    Ok(Value::Object(env))
}

/// Desired envelopes for every registry instance agentdeck manages in T3
/// (harness claude/codex with `t3 = true`).
fn desired_map(reg: &Registry) -> anyhow::Result<BTreeMap<String, Value>> {
    let main_codex_home = reg
        .get("codex")
        .filter(|i| i.harness == Harness::Codex && i.t3)
        .map(|i| i.home.clone());
    let mut out = BTreeMap::new();
    for inst in &reg.instances {
        if !inst.t3 {
            continue;
        }
        match desired_envelope(inst, main_codex_home.as_deref()) {
            Ok(env) => {
                out.insert(inst.id.clone(), env);
            }
            Err(e) if matches!(inst.harness, Harness::Opencode | Harness::Agy) => {
                let _ = e; // no mapping yet (M7 owns those harnesses): leave untouched
            }
            Err(e) => return Err(e),
        }
    }
    Ok(out)
}

/// Merge desired envelopes into parsed settings: only `providerInstances`
/// entries for managed ids change, and within an existing envelope only
/// `driver`/`displayName`/`enabled` plus our own `config` keys. Everything
/// else (accentColor, customModels, launchArgs, environment, unknown keys,
/// foreign instances, all other top-level keys) is preserved verbatim.
pub fn merge_settings(old: &Value, desired: &BTreeMap<String, Value>) -> anyhow::Result<Value> {
    let mut new = old.clone();
    let pi = new
        .get_mut("providerInstances")
        .and_then(|v| v.as_object_mut())
        .ok_or_else(|| anyhow::anyhow!("settings.json has no providerInstances object"))?;
    for (id, env) in desired {
        check_t3_slug(id)?;
        let want = env.as_object().expect("desired_envelope builds objects");
        match pi.get_mut(id) {
            Some(existing) => {
                let obj = existing
                    .as_object_mut()
                    .ok_or_else(|| anyhow::anyhow!("providerInstances[{id:?}] is not an object"))?;
                for key in ["driver", "displayName", "enabled"] {
                    obj.insert(key.into(), want[key].clone());
                }
                let cfg = obj
                    .entry("config")
                    .or_insert_with(|| Value::Object(Map::new()));
                let cfg_obj = cfg.as_object_mut().ok_or_else(|| {
                    anyhow::anyhow!("providerInstances[{id:?}].config is not an object")
                })?;
                let want_cfg = want["config"].as_object().expect("envelope has config");
                validate_config_fields(want["driver"].as_str().unwrap_or_default(), want_cfg)?;
                for (k, v) in want_cfg {
                    cfg_obj.insert(k.clone(), v.clone());
                }
            }
            None => {
                pi.insert(id.clone(), env.clone());
            }
        }
    }
    Ok(new)
}

/// Leading-indent unit of a JSON file (`"  "`, `"    "`, …), from the first
/// indented line; defaults to two spaces (what T3 itself writes).
fn detect_indent(raw: &str) -> String {
    for line in raw.lines() {
        let n = line.chars().take_while(|c| *c == ' ').count();
        if n > 0 && line[n..].starts_with('"') {
            return " ".repeat(n.min(8));
        }
    }
    "  ".to_string()
}

fn apply_indent(pretty: &str, indent: &str) -> String {
    if indent == "  " {
        return pretty.to_string();
    }
    pretty
        .lines()
        .map(|line| {
            let n = line.chars().take_while(|c| *c == ' ').count();
            let level = n / 2;
            format!("{}{}", indent.repeat(level), &line[n..])
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Backup, atomically write (tmp + fsync + rename, original mode kept),
/// re-read and verify the result equals what was intended.
fn write_settings_verified(
    t3_root: &Path,
    new: &Value,
    indent: &str,
    backup_dir: &Path,
) -> anyhow::Result<()> {
    let path = settings_file(t3_root);
    let old_bytes = fs::read(&path)?;
    fs::create_dir_all(backup_dir)?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    fs::write(
        backup_dir.join(format!(
            "settings.json.{}.{}",
            stamp.as_secs(),
            stamp.subsec_nanos()
        )),
        &old_bytes,
    )?;
    let mut text = apply_indent(&serde_json::to_string_pretty(new)?, indent);
    text.push('\n');
    let mode = fs::metadata(&path)?.permissions().mode() & 0o777;
    let tmp = path.with_extension("json.tmp");
    {
        let mut f = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&tmp)?;
        use std::io::Write as _;
        f.write_all(text.as_bytes())?;
        f.sync_all()?;
    }
    fs::set_permissions(&tmp, fs::Permissions::from_mode(mode))?;
    fs::rename(&tmp, &path)?;
    // Verify: re-read must parse and equal what was intended.
    let back: Value = serde_json::from_slice(&fs::read(&path)?)
        .map_err(|e| anyhow::anyhow!("wrote settings.json but cannot re-parse it: {e}"))?;
    if back != *new {
        anyhow::bail!("wrote settings.json but re-read differs from intent");
    }
    Ok(())
}

fn parse_settings_strict(raw: &[u8]) -> anyhow::Result<Value> {
    // Strict parse: any decode problem aborts (T3 itself discards the whole
    // file to defaults on error, so agentdeck must never write then).
    serde_json::from_slice(raw).map_err(|e| anyhow::anyhow!("cannot parse settings.json: {e}"))
}

/// Redact secret values for dry-run output: secret-named keys, and
/// `environment[]` entries with `sensitive: true`.
pub fn redact(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut out = Map::with_capacity(map.len());
            for (k, v) in map {
                if k == "environment"
                    && let Value::Array(items) = v
                {
                    out.insert(
                        k.clone(),
                        Value::Array(
                            items
                                .iter()
                                .map(|e| {
                                    let sensitive = e
                                        .get("sensitive")
                                        .and_then(Value::as_bool)
                                        .unwrap_or(false);
                                    if sensitive && let Value::Object(mut em) = e.clone() {
                                        if em.contains_key("value") {
                                            em.insert("value".into(), Value::String("***".into()));
                                        }
                                        return Value::Object(em);
                                    }
                                    redact(e)
                                })
                                .collect(),
                        ),
                    );
                } else if is_secret_key(k) {
                    out.insert(k.clone(), Value::String("***".into()));
                } else {
                    out.insert(k.clone(), redact(v));
                }
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(items.iter().map(redact).collect()),
        _ => value.clone(),
    }
}

#[derive(Debug, Clone)]
pub struct SyncEntry {
    pub id: String,
    pub action: &'static str, // "created" | "updated" | "unchanged"
    pub before: Option<Value>,
    pub after: Value,
}

/// Re-read settings.json, merge managed `providerInstances`, and (unless
/// dry-run) backup + atomic-write + verify. Never deletes T3 instances that
/// are not in the registry. Returns per-instance entries; dry-run writes
/// nothing.
pub fn sync_t3(paths: &Paths, t3_root: &Path, dry_run: bool) -> anyhow::Result<Vec<SyncEntry>> {
    let raw = fs::read(settings_file(t3_root))
        .map_err(|e| anyhow::anyhow!("cannot read settings.json: {e}"))?;
    let old = parse_settings_strict(&raw)?;
    if old
        .get("providerInstances")
        .and_then(|v| v.as_object())
        .is_none()
    {
        anyhow::bail!("settings.json has no providerInstances object");
    }
    let reg = Registry::load(paths)?;
    let desired = desired_map(&reg)?;
    let new = merge_settings(&old, &desired)?;
    let old_pi = &old["providerInstances"];
    let mut entries = Vec::with_capacity(desired.len());
    for (id, env) in &desired {
        let before = old_pi.get(id).cloned();
        let action = match &before {
            None => "created",
            Some(b) if *b == *env => "unchanged",
            _ => "updated",
        };
        // Note: "updated" compares against the desired envelope, which only
        // carries our own keys; the merge keeps the instance's other fields.
        entries.push(SyncEntry {
            id: id.clone(),
            action,
            before,
            after: env.clone(),
        });
    }
    if new != old && !dry_run {
        write_settings_verified(
            t3_root,
            &new,
            &detect_indent(&String::from_utf8_lossy(&raw)),
            &paths.backup_dir(),
        )?;
    }
    Ok(entries)
}

#[derive(Debug, Clone)]
pub struct CacheHealth {
    pub status: String,
    pub checked_at: Option<String>,
}

#[derive(Debug, Clone)]
pub struct T3StatusRow {
    pub id: String,
    pub driver: String,
    pub in_registry: bool,
    pub enabled: bool,
    pub cache: Option<CacheHealth>,
    pub threads: Option<u64>,
}

/// Read-only health for one instance: `caches/<id>.json` (`status` +
/// `checkedAt`; missing file means no health).
pub fn cache_health(t3_root: &Path, id: &str) -> Option<CacheHealth> {
    let raw = fs::read(caches_dir(t3_root).join(format!("{id}.json"))).ok()?;
    let v: Value = serde_json::from_slice(&raw).ok()?;
    Some(CacheHealth {
        status: v.get("status")?.as_str()?.to_string(),
        checked_at: v
            .get("checkedAt")
            .and_then(Value::as_str)
            .map(str::to_string),
    })
}

/// Read-only thread count from `statev2.sqlite` (read-only connection, never
/// touches WAL). `Ok(None)` = db missing (or table absent): "n/a".
pub fn thread_count(t3_root: &Path, id: &str) -> anyhow::Result<Option<u64>> {
    let db = state_db(t3_root);
    if !db.exists() {
        return Ok(None);
    }
    let conn = Connection::open_with_flags(&db, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| anyhow::anyhow!("cannot open statev2.sqlite read-only: {e}"))?;
    match conn.query_row(
        "SELECT COUNT(*) FROM orchestration_v2_projection_provider_threads WHERE provider_instance_id = ?1",
        [id],
        |row| row.get::<_, u64>(0),
    ) {
        Ok(n) => Ok(Some(n)),
        Err(e) if e.to_string().contains("no such table") => Ok(None),
        Err(e) => Err(anyhow::anyhow!("thread count query failed: {e}")),
    }
}

/// One read-only row per T3 instance.
pub fn t3_status(paths: &Paths, t3_root: &Path) -> anyhow::Result<Vec<T3StatusRow>> {
    let raw = fs::read(settings_file(t3_root))
        .map_err(|e| anyhow::anyhow!("cannot read settings.json: {e}"))?;
    let settings = parse_settings_strict(&raw)?;
    let pi = settings
        .get("providerInstances")
        .and_then(|v| v.as_object())
        .ok_or_else(|| anyhow::anyhow!("settings.json has no providerInstances object"))?;
    let reg = Registry::load(paths).unwrap_or_default();
    let mut rows = Vec::with_capacity(pi.len());
    for (id, env) in pi {
        let threads = thread_count(t3_root, id)?;
        rows.push(T3StatusRow {
            id: id.clone(),
            driver: env
                .get("driver")
                .and_then(Value::as_str)
                .unwrap_or("?")
                .to_string(),
            in_registry: reg.get(id).is_some(),
            enabled: env.get("enabled").and_then(Value::as_bool).unwrap_or(false),
            cache: cache_health(t3_root, id),
            threads,
        });
    }
    rows.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(rows)
}

#[derive(Debug, Clone)]
pub struct Imported {
    pub id: String,
    pub harness: Harness,
    pub display_name: String,
    pub home: String,
}

/// Add T3 claudeAgent/codex instances missing from the registry into it
/// (id, harness, display_name, home from config; t3 = true).
pub fn import_t3(paths: &Paths, t3_root: &Path, dry_run: bool) -> anyhow::Result<Vec<Imported>> {
    let raw = fs::read(settings_file(t3_root))
        .map_err(|e| anyhow::anyhow!("cannot read settings.json: {e}"))?;
    let settings = parse_settings_strict(&raw)?;
    let pi = settings
        .get("providerInstances")
        .and_then(|v| v.as_object())
        .ok_or_else(|| anyhow::anyhow!("settings.json has no providerInstances object"))?;
    let reg = Registry::load(paths)?;
    let mut found = Vec::new();
    for (id, env) in pi {
        if reg.get(id).is_some() {
            continue;
        }
        let driver = env.get("driver").and_then(Value::as_str).unwrap_or("");
        let harness = match driver {
            "claudeAgent" => Harness::Claude,
            "codex" => Harness::Codex,
            _ => continue,
        };
        check_t3_slug(id)?;
        let cfg = env.get("config").and_then(Value::as_object);
        let str_field = |k: &str| {
            cfg.and_then(|c| c.get(k))
                .and_then(Value::as_str)
                .unwrap_or("")
        };
        let home = match harness {
            Harness::Codex => {
                let shadow = str_field("shadowHomePath");
                if !shadow.is_empty() {
                    shadow.to_string()
                } else {
                    let main = str_field("homePath");
                    if main.is_empty() {
                        "~/.codex".to_string()
                    } else {
                        main.to_string()
                    }
                }
            }
            _ => {
                let h = str_field("homePath");
                if h.is_empty() {
                    "~/.claude".to_string()
                } else {
                    h.to_string()
                }
            }
        };
        let display_name = env
            .get("displayName")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .unwrap_or(id)
            .to_string();
        let enabled = env.get("enabled").and_then(Value::as_bool).unwrap_or(true);
        found.push(Imported {
            id: id.clone(),
            harness,
            display_name: display_name.clone(),
            home: home.clone(),
        });
        if !dry_run {
            let (id_c, harness_c, name_c, home_c) =
                (id.clone(), harness, display_name.clone(), home.clone());
            Registry::update(paths, |r| {
                use crate::registry::AddSpec;
                r.add(AddSpec {
                    id: id_c.clone(),
                    harness: harness_c,
                    display_name: Some(name_c.clone()),
                    home: home_c.clone(),
                    binary: None,
                    no_t3: false,
                })?;
                r.set_enabled(&id_c, enabled)?;
                Ok(())
            })?;
        }
    }
    found.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::AddSpec;
    use std::sync::atomic::{AtomicU64, Ordering};

    static N: AtomicU64 = AtomicU64::new(0);

    struct Guard(PathBuf);
    impl Drop for Guard {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn setup(tag: &str, settings: &str) -> (Paths, PathBuf, Guard) {
        let dir = std::env::temp_dir().join(format!(
            "agentdeck-t3-{tag}-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = fs::remove_dir_all(&dir);
        let paths = Paths::new(dir.join("config"), dir.join("state"));
        let t3 = dir.join("t3");
        fs::create_dir_all(t3.join("userdata")).unwrap();
        fs::create_dir_all(t3.join("caches")).unwrap();
        fs::write(t3.join("userdata/settings.json"), settings).unwrap();
        (paths, t3, Guard(dir))
    }

    fn reg(paths: &Paths, specs: &[(&str, Harness, &str)]) {
        Registry::update(paths, |r| {
            for (id, h, home) in specs {
                r.add(AddSpec {
                    id: id.to_string(),
                    harness: *h,
                    display_name: None,
                    home: home.to_string(),
                    binary: None,
                    no_t3: false,
                })?;
            }
            Ok(())
        })
        .unwrap();
    }

    const SETTINGS: &str = r##"{
  "favorites": [],
  "providerHealthRefreshInterval": 300000,
  "providerInstances": {
    "codex": {
      "driver": "codex",
      "enabled": true,
      "config": {
        "binaryPath": "codex",
        "homePath": "",
        "shadowHomePath": "",
        "launchArgs": "",
        "customModels": []
      }
    },
    "codex_second": {
      "driver": "codex",
      "displayName": "Codex 2",
      "enabled": true,
      "accentColor": "#123456",
      "config": {
        "setupMode": "existing",
        "binaryPath": "codex",
        "homePath": "/main/.codex",
        "shadowHomePath": "/second/.codex",
        "launchArgs": "--foo"
      },
      "environment": [{"name": "X", "value": "1", "sensitive": false}]
    },
    "foreign": {
      "driver": "grok",
      "enabled": false,
      "config": {"binaryPath": "grok"}
    }
  }
}
"##;

    #[test]
    fn merge_preserves_unknown_fields_order_and_foreign() {
        let (paths, t3, _g) = setup("merge", SETTINGS);
        reg(
            &paths,
            &[("codex_second", Harness::Codex, "/second/.codex")],
        );
        // Registry's codex main home is unknown here, so homePath stays as-is
        // for codex_second? No: no "codex" in registry -> main = own home.
        let entries = sync_t3(&paths, &t3, false).unwrap();
        assert_eq!(entries.len(), 1);
        let raw = fs::read_to_string(settings_file(&t3)).unwrap();
        let v: Value = serde_json::from_str(&raw).unwrap();
        let cs = &v["providerInstances"]["codex_second"];
        // Unknown fields preserved.
        assert_eq!(cs["accentColor"], Value::String("#123456".into()));
        assert_eq!(cs["displayName"], Value::String("codex_second".into()));
        assert_eq!(cs["config"]["setupMode"], Value::String("existing".into()));
        assert_eq!(cs["config"]["launchArgs"], Value::String("--foo".into()));
        assert!(cs.get("environment").is_some());
        // Foreign instance untouched, main codex untouched.
        assert!(v["providerInstances"].get("foreign").is_some());
        assert_eq!(
            v["providerInstances"]["codex"]["config"]["homePath"],
            Value::String("".into())
        );
        // Top-level order preserved: favorites still first, providerInstances last.
        let top: Vec<_> = v.as_object().unwrap().keys().cloned().collect();
        assert_eq!(top.first().map(String::as_str), Some("favorites"));
        assert_eq!(top.last().map(String::as_str), Some("providerInstances"));
        // Formatting: 2-space indent + trailing newline.
        assert!(raw.starts_with("{\n  \"favorites\""));
        assert!(raw.ends_with("}\n"));
    }

    #[test]
    fn dry_run_writes_nothing() {
        let (paths, t3, _g) = setup("dryrun", SETTINGS);
        reg(
            &paths,
            &[("codex_second", Harness::Codex, "/second/.codex")],
        );
        let before = fs::read(settings_file(&t3)).unwrap();
        let entries = sync_t3(&paths, &t3, true).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].action, "updated");
        assert_eq!(fs::read(settings_file(&t3)).unwrap(), before);
        assert!(!paths.backup_dir().exists());
    }

    #[test]
    fn validation_rejects_unknown_field() {
        let mut cfg = Map::new();
        cfg.insert("homePath".into(), Value::String("/h".into()));
        cfg.insert("evilField".into(), Value::String("x".into()));
        assert!(validate_config_fields("codex", &cfg).is_err());
        assert!(validate_config_fields("nope", &cfg).is_err());
        cfg.remove("evilField");
        assert!(validate_config_fields("codex", &cfg).is_ok());
        cfg.insert("apiKey".into(), Value::String("s3cret".into()));
        assert!(validate_config_fields("antigravity", &cfg).is_err());
    }

    #[test]
    fn import_adds_missing_instances() {
        let (paths, t3, _g) = setup("import", SETTINGS);
        let found = import_t3(&paths, &t3, true).unwrap();
        let ids: Vec<_> = found.iter().map(|f| f.id.as_str()).collect();
        assert!(ids.contains(&"codex"), "{ids:?}");
        assert!(ids.contains(&"codex_second"), "{ids:?}");
        assert!(!ids.contains(&"foreign"), "{ids:?}");
        assert!(Registry::load(&paths).unwrap().instances.is_empty());
        let found = import_t3(&paths, &t3, false).unwrap();
        assert_eq!(found.len(), 2);
        let reg = Registry::load(&paths).unwrap();
        let cs = reg.get("codex_second").unwrap();
        assert!(matches!(cs.harness, Harness::Codex));
        assert_eq!(cs.home, "/second/.codex");
        assert_eq!(cs.display_name, "Codex 2");
        assert!(cs.t3);
        // Second import is a no-op.
        assert!(import_t3(&paths, &t3, false).unwrap().is_empty());
    }

    #[test]
    fn thread_count_from_fixture_db() {
        let (paths, t3, _g) = setup("threads", SETTINGS);
        assert_eq!(thread_count(&t3, "codex").unwrap(), None); // no db yet
        let db = state_db(&t3);
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch(
            "CREATE TABLE orchestration_v2_projection_provider_threads(provider_instance_id TEXT);
             INSERT INTO orchestration_v2_projection_provider_threads VALUES ('codex'), ('codex'), ('other');",
        )
        .unwrap();
        drop(conn);
        assert_eq!(thread_count(&t3, "codex").unwrap(), Some(2));
        assert_eq!(thread_count(&t3, "other").unwrap(), Some(1));
        assert_eq!(thread_count(&t3, "missing").unwrap(), Some(0));
        let _ = &paths;
    }

    #[test]
    fn status_rows_read_only() {
        let (paths, t3, _g) = setup("status", SETTINGS);
        reg(
            &paths,
            &[("codex_second", Harness::Codex, "/second/.codex")],
        );
        fs::write(
            t3.join("caches/codex_second.json"),
            r#"{"status": "ready", "checkedAt": "2026-10-24T00:00:00.000Z", "models": []}"#,
        )
        .unwrap();
        let rows = t3_status(&paths, &t3).unwrap();
        assert_eq!(rows.len(), 3);
        let cs = rows.iter().find(|r| r.id == "codex_second").unwrap();
        assert!(cs.in_registry);
        assert_eq!(cs.cache.as_ref().unwrap().status, "ready");
        assert!(cs.cache.as_ref().unwrap().checked_at.is_some());
        let foreign = rows.iter().find(|r| r.id == "foreign").unwrap();
        assert!(!foreign.in_registry);
        assert!(foreign.cache.is_none());
        assert_eq!(foreign.threads, None);
        // Status never writes.
        let raw = fs::read_to_string(settings_file(&t3)).unwrap();
        assert_eq!(raw, SETTINGS);
    }

    #[test]
    fn whole_file_rejected_on_error() {
        let (paths, t3, _g) = setup("broken", "{ not json");
        reg(&paths, &[("codex", Harness::Codex, "/h")]);
        assert!(sync_t3(&paths, &t3, false).is_err());
        assert!(t3_status(&paths, &t3).is_err());
        assert!(import_t3(&paths, &t3, false).is_err());
    }
}

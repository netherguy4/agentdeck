//! OpenCode adapter: one instance = one account inside one data dir.
//!
//! The default model shares the single data dir (`~/.local/share/opencode`):
//! each `opencode auth login` adds a saved credential and the newest one
//! becomes active; `opencode auth switch [<target>] [<credential>]` flips the
//! active account (`credential.active` in `opencode.db`). A separate data dir
//! is only for a fully isolated instance (separate history/sessions); it then
//! needs its own service port, recorded in the registry instance `extra` map
//! as `port` (`auth list` against a foreign dir otherwise collides with the
//! user's running service port).
//!
//! Instance `home` is the data dir itself (the dir holding `opencode.db`);
//! commands run with `XDG_DATA_HOME` pointed at its parent. The config dir
//! stays shared by design (`OPENCODE_CONFIG_DIR` is never set by us).
//! Secret values (`credential.value`, `auth.json` keys) are never read.

use std::fs;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags};
use serde::Serialize;

/// One saved credential: non-secret columns only, never the value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Account {
    pub integration: String,
    pub label: String,
    pub method: String,
    pub active: bool,
}

/// The data dir holding `opencode.db` for a login `home` root.
pub fn data_dir(home: &Path) -> PathBuf {
    home.join(".local/share/opencode")
}

/// Env override so the official CLI uses `data_dir` instead of the default.
/// Pure: returns `XDG_DATA_HOME=<parent>`; the caller applies it.
pub fn command_env(data_dir: &Path) -> anyhow::Result<Vec<(String, String)>> {
    let Some(parent) = data_dir.parent() else {
        anyhow::bail!(
            "opencode data dir {} has no parent for XDG_DATA_HOME",
            data_dir.display()
        );
    };
    Ok(vec![(
        "XDG_DATA_HOME".to_string(),
        parent.display().to_string(),
    )])
}

pub fn auth_list_argv(binary: &str) -> Vec<String> {
    vec![
        binary.to_string(),
        "auth".to_string(),
        "list".to_string(),
        "--format".to_string(),
        "json".to_string(),
    ]
}

pub fn switch_argv(binary: &str, target: Option<&str>, credential: &str) -> Vec<String> {
    let mut argv = vec![binary.to_string(), "auth".to_string(), "switch".to_string()];
    if let Some(t) = target.filter(|t| !t.is_empty()) {
        argv.push(t.to_string());
    }
    argv.push(credential.to_string());
    argv
}

pub fn login_argv(binary: &str) -> Vec<String> {
    vec![binary.to_string(), "auth".to_string(), "login".to_string()]
}

#[derive(Debug, Clone, Serialize)]
pub enum Drift {
    NoDataDir { dir: PathBuf },
    NoDb { db: PathBuf },
    NotLoggedIn,
    UnknownCredential { want: String, have: Vec<String> },
    InactiveCredential { want: String, active: String },
}

impl Drift {
    pub fn describe(&self) -> String {
        match self {
            Drift::NoDataDir { dir } => {
                format!("data dir {} missing (never logged in here)", dir.display())
            }
            Drift::NoDb { db } => {
                format!("{} missing (never logged in here)", db.display())
            }
            Drift::NotLoggedIn => "no saved credentials in this data dir".to_string(),
            Drift::UnknownCredential { want, have } => {
                format!(
                    "credential {want:?} not in this data dir (have: {})",
                    have.join(", ")
                )
            }
            Drift::InactiveCredential { want, active } => {
                format!(
                    "credential {want:?} is not active (active is {active:?}; run `agentdeck opencode switch`)"
                )
            }
        }
    }
}

/// Read-only inventory of one data dir: non-secret `credential` columns
/// (`integration_id`, `label`, `method_id`, `active`) via a read-only
/// connection. Never reads `value`; never touches a running service.
pub fn read_accounts(data_dir: &Path) -> anyhow::Result<Vec<Account>> {
    let db = data_dir.join("opencode.db");
    let conn = Connection::open_with_flags(&db, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| anyhow::anyhow!("cannot open {} read-only: {e}", db.display()))?;
    let mut stmt = conn
        .prepare("SELECT integration_id, label, method_id, active FROM credential ORDER BY label")
        .map_err(|e| anyhow::anyhow!("cannot read credential table (unexpected schema): {e}"))?;
    let rows = stmt
        .query_map([], |row| {
            Ok(Account {
                integration: row.get(0)?,
                label: row.get(1)?,
                method: row.get(2)?,
                active: row.get::<_, i64>(3)? != 0,
            })
        })
        .map_err(|e| anyhow::anyhow!("cannot read credential rows: {e}"))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| anyhow::anyhow!("cannot read credential rows: {e}"))
}

/// Offline drift of one data dir against the wanted credential label
/// (`None` = any logged-in account is fine). Pure fs + read-only db.
pub fn check(data_dir: &Path, want: Option<&str>) -> anyhow::Result<Vec<Drift>> {
    if fs::symlink_metadata(data_dir).is_err() {
        return Ok(vec![Drift::NoDataDir {
            dir: data_dir.to_path_buf(),
        }]);
    }
    if fs::symlink_metadata(data_dir.join("opencode.db")).is_err() {
        return Ok(vec![Drift::NoDb {
            db: data_dir.join("opencode.db"),
        }]);
    }
    let accounts = read_accounts(data_dir)?;
    if accounts.is_empty() {
        return Ok(vec![Drift::NotLoggedIn]);
    }
    let Some(want) = want.filter(|w| !w.is_empty()) else {
        return Ok(vec![]);
    };
    let Some(found) = accounts
        .iter()
        .find(|a| a.label == want || a.integration == want)
    else {
        return Ok(vec![Drift::UnknownCredential {
            want: want.to_string(),
            have: accounts.iter().map(|a| a.label.clone()).collect(),
        }]);
    };
    if found.active {
        return Ok(vec![]);
    }
    let active = accounts
        .iter()
        .find(|a| a.active)
        .map(|a| a.label.clone())
        .unwrap_or_else(|| "(none)".to_string());
    Ok(vec![Drift::InactiveCredential {
        want: want.to_string(),
        active,
    }])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_db(dir: &Path, rows: &[(&str, &str, &str, i64)]) -> PathBuf {
        let db = dir.join("opencode.db");
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch(
            "CREATE TABLE credential(id TEXT, integration_id TEXT, label TEXT,
             value TEXT, connector_id TEXT, method_id TEXT, active INTEGER,
             time_created INTEGER, time_updated INTEGER);",
        )
        .unwrap();
        for (id, integration, label, active) in rows {
            conn.execute(
                "INSERT INTO credential VALUES (?1, ?2, ?3, 's3cret', '', 'oauth', ?4, 0, 0)",
                rusqlite::params![id, integration, label, active],
            )
            .unwrap();
        }
        drop(conn);
        db
    }

    fn tmp(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("agentdeck-opencode-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn check_reports_missing_dir_db_and_empty() {
        let base = tmp("missing");
        let dir = base.join("data");
        assert!(matches!(
            check(&dir, None).unwrap().as_slice(),
            [Drift::NoDataDir { .. }]
        ));
        fs::create_dir_all(&dir).unwrap();
        assert!(matches!(
            check(&dir, None).unwrap().as_slice(),
            [Drift::NoDb { .. }]
        ));
        fixture_db(&dir, &[]);
        assert!(matches!(
            check(&dir, None).unwrap().as_slice(),
            [Drift::NotLoggedIn]
        ));
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn check_matches_wanted_credential() {
        let base = tmp("match");
        let dir = base.join("data");
        fs::create_dir_all(&dir).unwrap();
        fixture_db(
            &dir,
            &[
                ("c1", "opencode", "Personal", 0),
                ("c2", "opencode", "Default", 1),
            ],
        );
        assert!(check(&dir, None).unwrap().is_empty());
        assert!(check(&dir, Some("Default")).unwrap().is_empty());
        assert!(matches!(
            check(&dir, Some("Personal")).unwrap().as_slice(),
            [Drift::InactiveCredential { .. }]
        ));
        let drift = check(&dir, Some("Nobody")).unwrap();
        assert!(
            matches!(drift.as_slice(), [Drift::UnknownCredential { have, .. }] if have.len() == 2),
            "{drift:?}"
        );
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn argv_builders() {
        assert_eq!(
            switch_argv("opencode", Some("opencode"), "Personal"),
            vec!["opencode", "auth", "switch", "opencode", "Personal"]
        );
        assert_eq!(
            switch_argv("opencode", None, "Default"),
            vec!["opencode", "auth", "switch", "Default"]
        );
        assert_eq!(
            login_argv("/x/opencode"),
            vec!["/x/opencode", "auth", "login"]
        );
        let env = command_env(Path::new("/h/.local/share/opencode")).unwrap();
        assert_eq!(
            env,
            vec![("XDG_DATA_HOME".to_string(), "/h/.local/share".to_string())]
        );
    }
}

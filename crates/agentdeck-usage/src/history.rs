use std::fs;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

use anyhow::Result;
use serde_json::json;

use crate::model::AppState;

/// Предел файла журнала: при превышении он уезжает в `history.jsonl.1`, и
/// вместе с ним хранится примерно месяц активной работы.
const MAX_BYTES: u64 = 8 * 1024 * 1024;

/// Дописывает снимок лимитов в JSONL-журнал, чтобы внешние инструменты могли
/// сопоставить расход процентов с сессиями (сам `state.json` хранит только
/// последнее состояние). Пишутся только свежие `ok`-аккаунты и только когда
/// цифры изменились: last-known-good после сбоя провайдера выдал бы старые
/// данные за новые. В журнал не попадает ничего, кроме id, тарифа и чисел.
pub fn append(path: &Path, state: &AppState, last: &mut Option<String>) -> Result<()> {
    let accounts: Vec<_> = state
        .accounts
        .iter()
        .filter(|a| a.status == "ok" && a.error.is_none())
        .map(|a| {
            json!({
                "id": a.id,
                "provider": a.provider,
                "plan": a.plan,
                "windows": a.windows.iter().map(|w| json!({
                    "key": w.key,
                    "used_percent": w.used_percent,
                    "duration_mins": w.duration_mins,
                    "resets_at": w.resets_at,
                    "scope": w.scope,
                })).collect::<Vec<_>>(),
                "balances": a.balances.iter().map(|b| json!({
                    "currency": b.currency,
                    "total": b.total,
                })).collect::<Vec<_>>(),
            })
        })
        .collect();
    if accounts.is_empty() {
        return Ok(());
    }
    let snapshot = serde_json::to_string(&accounts)?;
    if last.as_deref() == Some(snapshot.as_str()) {
        return Ok(());
    }

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    if fs::metadata(path)
        .map(|m| m.len() > MAX_BYTES)
        .unwrap_or(false)
    {
        fs::rename(path, path.with_extension("jsonl.1"))?;
    }
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(path)?;
    writeln!(
        file,
        "{{\"t\":{},\"accounts\":{}}}",
        state.updated_at, snapshot
    )?;
    *last = Some(snapshot);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{self, QuotaWindow};

    fn state(used: f64, status: &str) -> AppState {
        let raw = serde_json::json!({
            "schema_version": model::SCHEMA_VERSION,
            "updated_at": 100,
            "accounts": [{
                "id": "claude-main", "name": "Claude", "provider": "claude",
                "status": status, "plan": "Max 5x", "email": "secret@example.com",
                "windows": [QuotaWindow::new("five_hour", "5 часов", used, Some(300), Some(1))],
                "balances": [], "error": null, "updated_at": 100
            }]
        });
        serde_json::from_value(raw).unwrap()
    }

    #[test]
    fn appends_only_changes_of_fresh_accounts_without_personal_data() {
        let dir =
            std::env::temp_dir().join(format!("ai-usage-history-test-{}", std::process::id()));
        let path = dir.join("history.jsonl");
        let _ = fs::remove_dir_all(&dir);
        let mut last = None;

        append(&path, &state(10.0, "ok"), &mut last).unwrap();
        append(&path, &state(10.0, "ok"), &mut last).unwrap();
        append(&path, &state(20.0, "stale"), &mut last).unwrap();
        append(&path, &state(30.0, "ok"), &mut last).unwrap();

        let raw = fs::read_to_string(&path).unwrap();
        let lines: Vec<_> = raw.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("\"used_percent\":10.0"));
        assert!(lines[1].contains("\"used_percent\":30.0"));
        assert!(
            lines
                .iter()
                .all(|l| l.contains("\"plan\":\"Max 5x\"") && !l.contains("secret@"))
        );
        fs::remove_dir_all(&dir).unwrap();
    }
}

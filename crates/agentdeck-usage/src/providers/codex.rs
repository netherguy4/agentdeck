use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStderr, Command};
use tokio::task::JoinHandle;
use tokio::time::timeout;

use crate::model::{AccountState, QuotaWindow};

const RESPONSE_TIMEOUT: Duration = Duration::from_secs(8);
/// Сколько байт stderr сохраняем для диагностики. Держим хвост, а не начало:
/// причина падения обычно в последних строках.
const STDERR_CAPTURE_BYTES: usize = 4096;

pub async fn fetch(
    id: &str,
    name: &str,
    codex_home: &Path,
    command: &str,
    limit_id: &str,
) -> Result<AccountState> {
    // Two attempts leave room for response timeouts and diagnostics within
    // the shared 30-second provider deadline.
    const MAX_ATTEMPTS: usize = 2;
    let mut last_error = None;

    for attempt in 1..=MAX_ATTEMPTS {
        match fetch_once(id, name, codex_home, command, limit_id).await {
            Ok(state) => return Ok(state),
            Err(error) => {
                let err_msg = error.to_string();
                let retriable = is_retriable_error(&err_msg);
                last_error = Some(error);

                if !retriable || attempt == MAX_ATTEMPTS {
                    break;
                }

                let delay = Duration::from_millis(1000 * attempt as u64);
                tokio::time::sleep(delay).await;
            }
        }
    }

    Err(last_error.unwrap_or_else(|| anyhow!("Codex fetch failed")))
}

async fn fetch_once(
    id: &str,
    name: &str,
    codex_home: &Path,
    command: &str,
    limit_id: &str,
) -> Result<AccountState> {
    let codex_home = crate::util::expand_home(codex_home);
    // Демон стартует до того, как сессия добавит ~/.local/bin в PATH, поэтому
    // короткое имя команды резолвим сами, а не полагаемся на окружение.
    let resolved = crate::util::resolve_command(command).ok_or_else(|| {
        anyhow!(
            "Команда '{command}' не найдена ни в PATH, ни в стандартных \
             пользовательских каталогах. Укажи абсолютный путь: \
             ai-usage account add codex --id {id} --codex-home {} --command /путь/к/codex",
            codex_home.display()
        )
    })?;

    let mut builder = Command::new(&resolved);
    if let Some(dir) = interpreter_dir(Path::new(&resolved)) {
        builder.env("PATH", prepend_path(&dir));
    }
    let mut child = builder
        .arg("app-server")
        .env("CODEX_HOME", &codex_home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .with_context(|| format!("Не удалось запустить '{resolved} app-server'"))?;

    let mut stdin = child
        .stdin
        .take()
        .context("Codex app-server не открыл stdin")?;
    let stdout = child
        .stdout
        .take()
        .context("Codex app-server не открыл stdout")?;
    let stderr = child
        .stderr
        .take()
        .context("Codex app-server не открыл stderr")?;
    let stderr_task = tokio::spawn(capture_stderr(stderr));

    // Все четыре сообщения отправляются одним батчем: живой app-server 0.145
    // принимает такую последовательность и отвечает на initialize первым.
    // Ответ на id 0 всё равно разбирается — чтобы отличить отказ handshake
    // от молчания сервера.
    for message in [
        json!({
            "method": "initialize",
            "id": 0,
            "params": {
                "clientInfo": {
                    "name": "ai_usage_gnome",
                    "title": "AI Usage GNOME",
                    "version": env!("CARGO_PKG_VERSION")
                }
            }
        }),
        json!({"method": "initialized", "params": {}}),
        json!({"method": "account/read", "id": 1, "params": {"refreshToken": false}}),
        json!({"method": "account/rateLimits/read", "id": 2}),
    ] {
        if let Err(error) = write_message(&mut stdin, &message).await {
            let details = collect_failure(&mut child, stderr_task).await;
            return Err(anyhow!("Codex app-server закрыл stdin: {error}{details}"));
        }
    }
    stdin.flush().await.ok();

    let exchange = timeout(RESPONSE_TIMEOUT, read_responses(stdout)).await;

    let responses = match exchange {
        Ok(Ok(responses)) => responses,
        Ok(Err(error)) => {
            let details = collect_failure(&mut child, stderr_task).await;
            return Err(anyhow!("Ошибка чтения ответа Codex: {error}{details}"));
        }
        Err(_) => {
            let details = collect_failure(&mut child, stderr_task).await;
            return Err(anyhow!(
                "Codex app-server не ответил за {} секунд{details}",
                RESPONSE_TIMEOUT.as_secs()
            ));
        }
    };

    // Ответы могут прийти в любом порядке: на неавторизованном профиле ошибка
    // на id 2 приходит раньше ответа на id 1.
    let Responses {
        handshake,
        account: account_response,
        limits: limits_response,
    } = responses;

    let account_message = match account_response {
        Some(message) => message,
        None => {
            let details = collect_failure(&mut child, stderr_task).await;
            let handshake_error = handshake
                .as_ref()
                .and_then(rpc_error_text)
                .map(|text| format!(" (initialize: {text})"))
                .unwrap_or_default();
            return Err(anyhow!(
                "Codex не вернул account/read{handshake_error}{details}"
            ));
        }
    };

    let limits_response = match limits_response {
        Some(message) => message,
        None => {
            let details = collect_failure(&mut child, stderr_task).await;
            return Err(anyhow!("Codex не вернул account/rateLimits/read{details}"));
        }
    };

    let _ = child.kill().await;
    stderr_task.abort();

    parse_account_state(
        id,
        name,
        command,
        &codex_home,
        limit_id,
        &account_message,
        &limits_response,
    )
}

/// Codex — это node-скрипт с шебангом `#!/usr/bin/env node`. Демон стартует без
/// mise-окружения, поэтому node в PATH может не быть вовсе. Ищем интерпретатор
/// рядом с самим codex, проходя цепочку симлинков: и у mise, и у npm-global
/// исполняемый файл лежит в том же bin-каталоге, что и node.
fn interpreter_dir(command: &Path) -> Option<PathBuf> {
    let mut current = command.to_path_buf();
    // Ограничение на длину цепочки — защита от кольца симлинков.
    for _ in 0..16 {
        let dir = current.parent()?;
        if dir.join("node").is_file() {
            return Some(dir.to_path_buf());
        }
        let target = std::fs::read_link(&current).ok()?;
        current = if target.is_absolute() {
            target
        } else {
            dir.join(target)
        };
    }
    None
}

fn prepend_path(dir: &Path) -> std::ffi::OsString {
    let rest = std::env::var_os("PATH").unwrap_or_default();
    let dirs = std::iter::once(dir.to_path_buf()).chain(std::env::split_paths(&rest));
    std::env::join_paths(dirs).unwrap_or(rest)
}

struct Responses {
    handshake: Option<Value>,
    account: Option<Value>,
    limits: Option<Value>,
}

async fn write_message(
    stdin: &mut tokio::process::ChildStdin,
    message: &Value,
) -> std::io::Result<()> {
    stdin
        .write_all(serde_json::to_string(message)?.as_bytes())
        .await?;
    stdin.write_all(b"\n").await
}

async fn read_responses(stdout: tokio::process::ChildStdout) -> Result<Responses> {
    let mut lines = BufReader::new(stdout).lines();
    let mut responses = Responses {
        handshake: None,
        account: None,
        limits: None,
    };

    while let Some(line) = lines.next_line().await? {
        if responses.absorb(&line) {
            break;
        }
    }

    Ok(responses)
}

impl Responses {
    /// Разбирает одну строку JSONL и сообщает, собраны ли оба нужных ответа.
    /// Нераспознанные строки и уведомления игнорируются: app-server шлёт их
    /// вперемешку с ответами.
    fn absorb(&mut self, line: &str) -> bool {
        if let Ok(message) = serde_json::from_str::<Value>(line) {
            match message.get("id").and_then(Value::as_i64) {
                Some(0) => self.handshake = Some(message),
                Some(1) => self.account = Some(message),
                Some(2) => self.limits = Some(message),
                _ => {}
            }
        }
        self.account.is_some() && self.limits.is_some()
    }
}

/// Читает stderr дочернего процесса, удерживая в памяти только хвост.
async fn capture_stderr(stderr: ChildStderr) -> Vec<u8> {
    let mut reader = BufReader::new(stderr);
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 1024];
    loop {
        match reader.read(&mut chunk).await {
            Ok(0) | Err(_) => break,
            Ok(read) => {
                buffer.extend_from_slice(&chunk[..read]);
                if buffer.len() > STDERR_CAPTURE_BYTES * 2 {
                    buffer.drain(..buffer.len() - STDERR_CAPTURE_BYTES);
                }
            }
        }
    }
    if buffer.len() > STDERR_CAPTURE_BYTES {
        buffer.drain(..buffer.len() - STDERR_CAPTURE_BYTES);
    }
    buffer
}

/// Завершает процесс и собирает безопасную для показа диагностику.
async fn collect_failure(child: &mut Child, stderr_task: JoinHandle<Vec<u8>>) -> String {
    let status = match timeout(Duration::from_secs(2), child.wait()).await {
        Ok(Ok(status)) => Some(status.to_string()),
        _ => {
            let _ = child.kill().await;
            None
        }
    };

    let captured = timeout(Duration::from_secs(2), stderr_task)
        .await
        .ok()
        .and_then(Result::ok)
        .unwrap_or_default();
    let stderr = redact(String::from_utf8_lossy(&captured).trim());

    let mut parts = Vec::new();
    if let Some(status) = status {
        parts.push(status);
    }
    if !stderr.is_empty() {
        parts.push(format!("stderr: {stderr}"));
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!(" [{}]", parts.join("; "))
    }
}

/// Вырезает из текста то, что похоже на токены и ключи, чтобы диагностика
/// Codex не утекала в state.json, логи и UI.
fn redact(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut token = String::new();
    let mut previous = String::new();

    let flush = |token: &mut String, previous: &mut String, out: &mut String| {
        if token.is_empty() {
            return;
        }
        if is_secret(token, previous) {
            out.push_str("<redacted>");
        } else {
            out.push_str(token);
        }
        previous.clear();
        previous.push_str(token);
        token.clear();
    };

    for ch in text.chars() {
        // '=' намеренно не входит в состав токена: иначе 'key=sk-...' стало бы
        // одним токеном и не попало бы ни под одно правило.
        if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | '+' | '/') {
            token.push(ch);
        } else {
            flush(&mut token, &mut previous, &mut out);
            out.push(ch);
        }
    }
    flush(&mut token, &mut previous, &mut out);
    out
}

fn is_secret(token: &str, previous: &str) -> bool {
    if previous.eq_ignore_ascii_case("bearer") || previous.eq_ignore_ascii_case("token") {
        return true;
    }
    if token.starts_with("sk-") || token.starts_with("sk_") {
        return true;
    }
    if token.starts_with("eyJ") && token.len() > 20 {
        return true;
    }
    // Длинные непрерывные base64-подобные строки. Пути и версии исключены
    // требованием отсутствия '/' и '.', чтобы диагностика оставалась читаемой.
    token.len() >= 40
        && !token.contains('/')
        && !token.contains('.')
        && token.chars().any(|ch| ch.is_ascii_digit())
        && token.chars().any(|ch| ch.is_ascii_alphabetic())
}

/// Чистая часть провайдера: превращает пару JSON-RPC ответов в `AccountState`.
/// Вынесена отдельно, чтобы тестироваться на фикстурах без subprocess.
pub fn parse_account_state(
    id: &str,
    name: &str,
    command: &str,
    codex_home: &Path,
    limit_id: &str,
    account_message: &Value,
    limits_message: &Value,
) -> Result<AccountState> {
    reject_rpc_error(account_message)?;
    let account = account_message
        .pointer("/result/account")
        .cloned()
        .unwrap_or(Value::Null);

    if account.is_null() {
        return Ok(AccountState {
            id: id.to_owned(),
            name: name.to_owned(),
            provider: "codex".to_owned(),
            status: "unauthenticated".to_owned(),
            plan: None,
            email: None,
            model: None,
            windows: Vec::new(),
            balances: Vec::new(),
            error: Some(format!(
                "Выполни: CODEX_HOME={} {} login",
                codex_home.display(),
                command
            )),
            updated_at: crate::util::unix_now(),
        });
    }

    let email = account
        .get("email")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let account_plan = account
        .get("planType")
        .and_then(Value::as_str)
        .map(plan_label);

    reject_rpc_error(limits_message)?;
    let result = limits_message
        .get("result")
        .context("В ответе Codex нет result")?;
    let bucket = result
        .get("rateLimitsByLimitId")
        .and_then(|buckets| buckets.get(limit_id))
        .or_else(|| result.get("rateLimits"))
        .context("Codex не вернул лимиты для выбранного bucket")?;

    let mut windows = Vec::new();
    for key in ["primary", "secondary"] {
        if let Some(window) = bucket.get(key).filter(|value| !value.is_null())
            && let Some(parsed) = parse_window(key, window)
        {
            windows.push(parsed);
        }
    }

    if windows.is_empty() {
        bail!("Codex вернул пустой набор quota windows");
    }

    // Codex не называет окна, а сообщает длительность: короткое выше длинного.
    windows.sort_by_key(|window| window.duration_mins.unwrap_or(u64::MAX));

    let bucket_plan = bucket
        .get("planType")
        .and_then(Value::as_str)
        .map(plan_label);

    Ok(AccountState {
        id: id.to_owned(),
        name: name.to_owned(),
        provider: "codex".to_owned(),
        status: "ok".to_owned(),
        plan: account_plan.or(bucket_plan),
        email,
        model: None,
        windows,
        balances: Vec::new(),
        error: None,
        updated_at: crate::util::unix_now(),
    })
}

/// Имена тарифов OpenAI в протоколе и в интерфейсе — разные слова: `team`
/// показывается как Business, а `business` — как Enterprise. Общий
/// `humanize_plan` назвал бы тариф чужим именем, поэтому здесь таблица,
/// снятая с `plan_type_display_name` из openai/codex
/// (codex-rs/tui/src/status/helpers.rs). Неизвестное значение не теряем:
/// протокол знает ещё и вариант `unknown`, а список тарифов растёт.
fn plan_label(raw: &str) -> String {
    match raw {
        // Апстрим зовёт эти тарифы «Pro» и «Pro Lite»; у нас они подписаны
        // множителем лимита — так видно, чем они отличаются, и подпись совпадает
        // по смыслу с Max 5x/20x у Claude.
        "pro" => "Pro 20x",
        "prolite" => "Pro 5x",
        "team" | "self_serve_business_usage_based" => "Business",
        "self_serve_business_prolite" => "Business Premium",
        "business" | "ent26" | "enterprise_cbp_usage_based" => "Enterprise",
        "enterprise_cbp_automation" => "Enterprise (Automation)",
        "edu_plus" => "Edu Plus",
        "edu_pro" => "Edu Pro",
        other => return crate::util::humanize_plan(other),
    }
    .to_owned()
}

fn rpc_error_text(message: &Value) -> Option<String> {
    message.get("error").map(|error| {
        error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("неизвестная JSON-RPC ошибка")
            .to_owned()
    })
}

fn reject_rpc_error(message: &Value) -> Result<()> {
    if let Some(text) = rpc_error_text(message) {
        bail!("Codex app-server: {text}");
    }
    Ok(())
}

pub fn is_retriable_error(text: &str) -> bool {
    let lower = text.to_lowercase();
    if lower.contains("authentication required") || lower.contains("unauthenticated") {
        return false;
    }
    lower.contains("failed to fetch codex rate limits")
        || lower.contains("error sending request")
        || lower.contains("connection")
        || lower.contains("connect")
        || lower.contains("timeout")
        || lower.contains("timed out")
        || lower.contains("codex app-server не ответил за ")
        || lower.contains("network")
        || lower.contains("сеть")
        || lower.contains("dns")
        || lower.contains("resolv")
        || lower.contains("temporary")
        || lower.contains("reset")
        || lower.contains("broken pipe")
        || lower.contains("http2 error")
        || lower.contains("502")
        || lower.contains("503")
        || lower.contains("504")
}

fn parse_window(key: &str, value: &Value) -> Option<QuotaWindow> {
    let used = value.get("usedPercent")?.as_f64()?;
    let duration = value.get("windowDurationMins").and_then(Value::as_u64);
    let resets_at = value.get("resetsAt").and_then(Value::as_i64);
    // Codex именует окна позиционно (primary/secondary), поэтому подпись
    // берём из длительности — она осмысленна для пользователя.
    Some(QuotaWindow::new(
        key,
        crate::model::label_for_duration(duration),
        used,
        duration,
        resets_at,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ACCOUNT_OK: &str = include_str!("../../tests/fixtures/codex_account_authenticated.json");
    const ACCOUNT_ANON: &str =
        include_str!("../../tests/fixtures/codex_account_unauthenticated.json");
    const LIMITS_PLUS: &str = include_str!("../../tests/fixtures/codex_rate_limits_plus.json");
    const LIMITS_ERROR: &str =
        include_str!("../../tests/fixtures/codex_rate_limits_unauthenticated_error.json");
    const LIMITS_TWO: &str =
        include_str!("../../tests/fixtures/codex_rate_limits_two_windows.json");

    fn json(raw: &str) -> Value {
        serde_json::from_str(raw).expect("фикстура должна разбираться")
    }

    fn state(account: &str, limits: &str) -> Result<AccountState> {
        parse_account_state(
            "codex-main",
            "Codex",
            "codex",
            Path::new("/home/user/.codex"),
            "codex",
            &json(account),
            &json(limits),
        )
    }

    #[test]
    fn parses_live_plus_account_with_a_single_weekly_window() {
        // На плане plus сервер возвращает только primary (10080 минут),
        // secondary равен null — окна на 5 часов у аккаунта нет вовсе.
        let state = state(ACCOUNT_OK, LIMITS_PLUS).unwrap();

        assert_eq!(state.status, "ok");
        assert_eq!(state.plan.as_deref(), Some("Plus"));
        assert_eq!(state.email.as_deref(), Some("user@example.com"));
        assert!(state.error.is_none());

        assert_eq!(state.windows.len(), 1, "у plus только недельное окно");
        let weekly = &state.windows[0];
        assert_eq!(weekly.used_percent, 54.0);
        assert_eq!(weekly.remaining_percent, 46.0);
        assert_eq!(weekly.duration_mins, Some(10_080));
        assert_eq!(weekly.resets_at, Some(1_785_631_187));
        assert_eq!(weekly.label, "Неделя");
    }

    /// Значения — полный `PlanType` из app-server-protocol, ожидания — из
    /// теста `plan_type_display_name_remaps_display_labels` того же апстрима.
    #[test]
    fn plan_labels_follow_the_names_codex_itself_shows() {
        for (raw, expected) in [
            ("free", "Free"),
            ("go", "Go"),
            ("plus", "Plus"),
            ("pro", "Pro 20x"),
            ("prolite", "Pro 5x"),
            ("team", "Business"),
            ("self_serve_business_prolite", "Business Premium"),
            ("self_serve_business_usage_based", "Business"),
            ("business", "Enterprise"),
            ("ent26", "Enterprise"),
            ("enterprise_cbp_automation", "Enterprise (Automation)"),
            ("enterprise_cbp_usage_based", "Enterprise"),
            ("enterprise", "Enterprise"),
            ("edu", "Edu"),
            ("edu_plus", "Edu Plus"),
            ("edu_pro", "Edu Pro"),
            ("unknown", "Unknown"),
            ("some_future_plan", "Some Future Plan"),
        ] {
            assert_eq!(plan_label(raw), expected, "тариф {raw}");
        }
    }

    #[test]
    fn parses_both_windows_when_the_plan_has_two() {
        let state = state(ACCOUNT_OK, LIMITS_TWO).unwrap();
        assert_eq!(
            state
                .windows
                .iter()
                .map(|w| w.duration_mins)
                .collect::<Vec<_>>(),
            [Some(300), Some(10_080)]
        );
    }

    #[test]
    fn unauthenticated_account_reports_the_login_command() {
        // Ошибка лимитов при этом игнорируется: до логина она ожидаема.
        let state = state(ACCOUNT_ANON, LIMITS_ERROR).unwrap();
        assert_eq!(state.status, "unauthenticated");
        let error = state.error.expect("должна быть подсказка");
        assert!(error.contains("/home/user/.codex"), "{error}");
        assert!(error.contains("login"), "{error}");
    }

    #[test]
    fn rpc_error_on_account_read_becomes_an_error() {
        let message = json(r#"{"id":1,"error":{"code":-32600,"message":"boom"}}"#);
        let error = parse_account_state(
            "codex-main",
            "Codex",
            "codex",
            Path::new("/home/user/.codex"),
            "codex",
            &message,
            &json(LIMITS_PLUS),
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("boom"), "{error}");
    }

    #[test]
    fn retriable_error_recognizes_transient_network_failures() {
        assert!(is_retriable_error(
            "failed to fetch codex rate limits: error sending request for url (https://chatgpt.com/backend-api/wham/usage)"
        ));
        assert!(is_retriable_error("connection refused"));
        assert!(is_retriable_error("operation timed out"));
        assert!(is_retriable_error(
            "Codex app-server не ответил за 8 секунд"
        ));
        assert!(is_retriable_error("Сеть недоступна"));
        assert!(is_retriable_error("error trying to connect: dns error"));

        assert!(!is_retriable_error(
            "codex account authentication required to read rate limits"
        ));
        assert!(!is_retriable_error("unknown bucket id"));
    }

    #[tokio::test]
    async fn retries_a_slow_app_server_and_keeps_timeout_failures_within_the_deadline() {
        use std::os::unix::fs::PermissionsExt;

        let root = std::env::temp_dir().join(format!(
            "ai-usage-codex-timeout-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let command = root.join("codex");
        std::fs::write(
            &command,
            r#"#!/bin/sh
printf 'attempt\n' >> "$CODEX_HOME/attempts"
for i in 1 2 3 4; do IFS= read -r message || exit 1; done
if [ ! -e "$CODEX_HOME/first-attempt" ] || [ -e "$CODEX_HOME/always-slow" ]; then
    : > "$CODEX_HOME/first-attempt"
    exec /usr/bin/sleep 60
fi
exec /usr/bin/cat "$CODEX_HOME/responses.jsonl"
"#,
        )
        .unwrap();
        std::fs::set_permissions(&command, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::write(
            root.join("responses.jsonl"),
            format!("{}\n{}\n", line(ACCOUNT_OK), line(LIMITS_PLUS)),
        )
        .unwrap();

        let recovered = timeout(
            crate::PROVIDER_DEADLINE,
            fetch(
                "mock-codex",
                "Codex",
                &root,
                command.to_str().unwrap(),
                "codex",
            ),
        )
        .await;
        let recovered_attempts = std::fs::read_to_string(root.join("attempts")).unwrap();

        std::fs::remove_file(root.join("attempts")).unwrap();
        std::fs::write(root.join("always-slow"), "").unwrap();
        let failed = timeout(
            crate::PROVIDER_DEADLINE,
            fetch(
                "mock-codex",
                "Codex",
                &root,
                command.to_str().unwrap(),
                "codex",
            ),
        )
        .await;
        let failed_attempts = std::fs::read_to_string(root.join("attempts")).unwrap();
        std::fs::remove_dir_all(&root).unwrap();

        assert_eq!(recovered.unwrap().unwrap().status, "ok");
        assert_eq!(recovered_attempts.lines().count(), 2);
        assert_eq!(failed_attempts.lines().count(), 2);
        let error = failed.expect("provider deadline must not erase the timeout error");
        assert!(
            error
                .unwrap_err()
                .to_string()
                .contains("не ответил за 8 секунд")
        );
    }

    #[test]
    fn authenticated_account_surfaces_a_rate_limit_error() {
        let error = state(ACCOUNT_OK, LIMITS_ERROR).unwrap_err().to_string();
        assert!(error.contains("authentication required"), "{error}");
    }

    #[test]
    fn unknown_bucket_id_is_reported() {
        let error = parse_account_state(
            "codex-main",
            "Codex",
            "codex",
            Path::new("/home/user/.codex"),
            "no-such-bucket",
            &json(ACCOUNT_OK),
            // Без общего rateLimits fallback bucket действительно не найти.
            &json(r#"{"id":2,"result":{"rateLimitsByLimitId":{"codex":{"primary":null}}}}"#),
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("bucket"), "{error}");
    }

    #[test]
    fn falls_back_to_the_flat_rate_limits_object() {
        // Старые сборки app-server отдавали только result.rateLimits.
        let limits = r#"{"id":2,"result":{"rateLimits":{"primary":{"usedPercent":10,"windowDurationMins":300,"resetsAt":7}}}}"#;
        let state = state(ACCOUNT_OK, limits).unwrap();
        assert_eq!(state.windows[0].remaining_percent, 90.0);
    }

    #[test]
    fn empty_window_set_is_an_error_not_a_silent_zero() {
        let limits = r#"{"id":2,"result":{"rateLimitsByLimitId":{"codex":{"primary":null,"secondary":null}}}}"#;
        assert!(state(ACCOUNT_OK, limits).is_err());
    }

    #[test]
    fn window_missing_used_percent_is_skipped() {
        let limits = r#"{"id":2,"result":{"rateLimitsByLimitId":{"codex":{"primary":{"windowDurationMins":300},"secondary":{"usedPercent":20,"windowDurationMins":10080}}}}}"#;
        let state = state(ACCOUNT_OK, limits).unwrap();
        assert_eq!(state.windows.len(), 1, "окно без usedPercent пропускается");
        assert_eq!(state.windows[0].used_percent, 20.0);
    }

    /// Прогоняет строки через ту же `absorb`, которую использует чтение stdout.
    fn absorb_lines(lines: &[&str]) -> Responses {
        let mut responses = Responses {
            handshake: None,
            account: None,
            limits: None,
        };
        for line in lines {
            if responses.absorb(line) {
                break;
            }
        }
        responses
    }

    fn line(raw: &str) -> String {
        serde_json::to_string(&json(raw)).unwrap()
    }

    #[test]
    fn handles_responses_in_reverse_order() {
        // Живой app-server на неавторизованном профиле присылает ошибку id 2
        // раньше ответа id 1. Чтение не должно зависеть от порядка.
        let limits = line(LIMITS_ERROR);
        let account = line(ACCOUNT_ANON);
        let responses = absorb_lines(&[&limits, &account]);

        assert!(responses.account.is_some(), "ответ id 1 потерян");
        assert!(responses.limits.is_some(), "ответ id 2 потерян");
    }

    #[test]
    fn ignores_notifications_and_unparsable_lines() {
        let account = line(ACCOUNT_OK);
        let limits = line(LIMITS_PLUS);
        let responses = absorb_lines(&[
            "not json at all",
            r#"{"method":"remoteControl/status/changed","params":{}}"#,
            &account,
            &limits,
        ]);

        assert!(responses.account.is_some());
        assert!(responses.limits.is_some());
    }

    #[test]
    fn keeps_the_initialize_response_for_diagnostics() {
        let handshake = r#"{"id":0,"error":{"code":-1,"message":"unsupported client"}}"#;
        let responses = absorb_lines(&[handshake]);
        assert_eq!(
            responses.handshake.as_ref().and_then(rpc_error_text),
            Some("unsupported client".to_owned())
        );
    }

    #[test]
    fn labels_windows_by_their_duration() {
        let state = state(ACCOUNT_OK, LIMITS_TWO).unwrap();
        let labels: Vec<_> = state.windows.iter().map(|w| w.label.as_str()).collect();
        assert_eq!(labels, ["5 часов", "Неделя"]);
    }

    #[test]
    fn shorter_windows_come_first_regardless_of_response_order() {
        // Недельное окно приходит в primary, пятичасовое — в secondary.
        let limits = r#"{"id":2,"result":{"rateLimitsByLimitId":{"codex":{
            "primary":{"usedPercent":50,"windowDurationMins":10080},
            "secondary":{"usedPercent":10,"windowDurationMins":300}}}}}"#;
        let state = state(ACCOUNT_OK, limits).unwrap();
        assert_eq!(
            state
                .windows
                .iter()
                .map(|w| w.duration_mins)
                .collect::<Vec<_>>(),
            [Some(300), Some(10_080)]
        );
    }

    #[test]
    fn an_unusual_window_duration_still_gets_a_label() {
        let limits = r#"{"id":2,"result":{"rateLimitsByLimitId":{"codex":{
            "primary":{"usedPercent":50,"windowDurationMins":1440}}}}}"#;
        let state = state(ACCOUNT_OK, limits).unwrap();
        assert_eq!(state.windows[0].label, "1 д");
    }

    #[test]
    fn redacts_bearer_tokens_and_api_keys() {
        let text = "auth failed: Authorization: Bearer abc123def456ghi789 key=sk-proj-1234567890";
        let redacted = redact(text);
        assert!(!redacted.contains("abc123def456ghi789"), "{redacted}");
        assert!(!redacted.contains("sk-proj-1234567890"), "{redacted}");
        assert!(redacted.contains("<redacted>"), "{redacted}");
    }

    #[test]
    fn redacts_jwt_like_strings() {
        let redacted = redact("token eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.payload");
        assert!(!redacted.contains("eyJhbGciOiJIUzI1NiIs"), "{redacted}");
    }

    #[test]
    fn redacts_long_opaque_blobs() {
        let blob = "a1b2c3d4e5f6g7h8i9j0k1l2m3n4o5p6q7r8s9t0";
        assert_eq!(blob.len(), 40);
        assert!(redact(blob).contains("<redacted>"));
    }

    #[test]
    fn keeps_diagnostics_readable() {
        // Ради редакции нельзя терять то, что делает сообщение полезным.
        let text = "Error: CODEX_HOME points to \"/var/home/user/.codex-work\", but that path does not exist";
        let redacted = redact(text);
        assert_eq!(redacted, text, "путь и текст ошибки должны сохраниться");
    }

    #[test]
    fn finds_node_beside_the_symlink_target() {
        // ~/.local/bin/codex — симлинк в mise-каталог; node лежит рядом с
        // целью ссылки, а не рядом с самой ссылкой.
        let root = std::env::temp_dir().join(format!("ai-usage-codex-{}", std::process::id()));
        let mise = root.join("mise/bin");
        let local = root.join("local/bin");
        std::fs::create_dir_all(&mise).unwrap();
        std::fs::create_dir_all(&local).unwrap();
        std::fs::write(mise.join("node"), "").unwrap();
        std::fs::write(mise.join("codex"), "").unwrap();
        let link = local.join("codex");
        std::os::unix::fs::symlink("../../mise/bin/codex", &link).unwrap();

        assert_eq!(
            interpreter_dir(&link).map(|dir| dir.canonicalize().unwrap()),
            Some(mise.canonicalize().unwrap())
        );
        // Обычный бинарник без node по соседству — искать нечего.
        assert!(interpreter_dir(&root.join("local/bin/other")).is_none());

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn keeps_version_numbers_and_paths() {
        let text = "codex-cli 0.145.0 at /usr/local/lib/node_modules/codex/bin/codex.js";
        assert_eq!(redact(text), text);
    }
}

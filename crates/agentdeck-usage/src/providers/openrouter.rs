use std::env;
use std::sync::OnceLock;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use reqwest::header::{ACCEPT, AUTHORIZATION};
use serde::Deserialize;

use crate::model::{AccountState, BalanceInfo, QuotaWindow};

const MAX_ERROR_BODY: usize = 200;

fn client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .connect_timeout(Duration::from_secs(5))
            .user_agent(concat!("ai-usage/", env!("CARGO_PKG_VERSION")))
            .build()
            .expect("не удалось создать HTTP-клиент")
    })
}

#[derive(Debug, Deserialize)]
struct CreditsEnvelope {
    data: CreditsData,
}

#[derive(Debug, Deserialize)]
struct CreditsData {
    total_credits: f64,
    total_usage: f64,
}

#[derive(Debug, Deserialize)]
struct KeyEnvelope {
    data: KeyData,
}

#[derive(Debug, Deserialize)]
struct KeyData {
    #[serde(default)]
    label: Option<String>,
    #[serde(default)]
    limit: Option<f64>,
    #[serde(default)]
    limit_remaining: Option<f64>,
    #[serde(default)]
    usage: f64,
}

pub async fn fetch(
    id: &str,
    name: &str,
    api_key_env: &str,
    base_url: &str,
) -> Result<AccountState> {
    let api_key =
        env::var(api_key_env).with_context(|| format!("Переменная {api_key_env} не задана"))?;
    if api_key.trim().is_empty() {
        bail!("Переменная {api_key_env} пуста");
    }
    fetch_with_key(id, name, &api_key, base_url).await
}

pub async fn fetch_with_key(
    id: &str,
    name: &str,
    api_key: &str,
    base_url: &str,
) -> Result<AccountState> {
    let base = base_url.trim_end_matches('/');

    // 1. Сначала пробуем получить баланс через /credits (для Management ключей)
    let credits_url = format!("{base}/credits");
    let credits_res = client()
        .get(credits_url)
        .header(ACCEPT, "application/json")
        .header(AUTHORIZATION, format!("Bearer {api_key}"))
        .send()
        .await
        .context("Не удалось подключиться к OpenRouter")?;

    if credits_res.status() == reqwest::StatusCode::UNAUTHORIZED {
        bail!("OpenRouter вернул HTTP 401 Unauthorized: неверный API-ключ");
    }

    if credits_res.status().is_success() {
        let payload: CreditsEnvelope = credits_res
            .json()
            .await
            .context("OpenRouter вернул некорректный JSON в /credits")?;

        let balance = payload.data.total_credits - payload.data.total_usage;
        let is_available = balance > 0.0;

        return Ok(AccountState {
            id: id.to_owned(),
            name: name.to_owned(),
            provider: "openrouter".to_owned(),
            status: if is_available { "ok" } else { "depleted" }.to_owned(),
            plan: Some("Credits".to_owned()),
            email: None,
            model: None,
            windows: Vec::new(),
            balances: vec![BalanceInfo {
                currency: "USD".to_owned(),
                total: format!("{:.2}", balance),
                granted: None,
                topped_up: Some(format!("{:.2}", payload.data.total_credits)),
            }],
            // The $0.00 balance row already says it.
            error: None,
            updated_at: crate::util::unix_now(),
        });
    }

    // 2. Если /credits недоступен (обычный ключ инференса), опрашиваем /key
    let key_url = format!("{base}/key");
    let key_res = client()
        .get(key_url)
        .header(ACCEPT, "application/json")
        .header(AUTHORIZATION, format!("Bearer {api_key}"))
        .send()
        .await
        .context("Не удалось подключиться к OpenRouter /key")?;

    let status = key_res.status();
    if !status.is_success() {
        let body = key_res.text().await.unwrap_or_default();
        let body = body.trim();
        let body: String = body.chars().take(MAX_ERROR_BODY).collect();
        bail!("OpenRouter вернул HTTP {status}: {body}");
    }

    let payload: KeyEnvelope = key_res
        .json()
        .await
        .context("OpenRouter вернул некорректный JSON в /key")?;

    let data = payload.data;
    let mut windows = Vec::new();
    let balances;
    let is_available;
    let status_str;

    if let Some(rem) = data.limit_remaining {
        is_available = rem > 0.0;
        status_str = if is_available { "ok" } else { "depleted" };
        balances = vec![BalanceInfo {
            currency: "USD".to_owned(),
            total: format!("{:.2}", rem),
            granted: None,
            topped_up: data.limit.map(|l| format!("{:.2}", l)),
        }];

        if let Some(lim) = data.limit
            && lim > 0.0
        {
            let used_percent = ((lim - rem) / lim * 100.0).clamp(0.0, 100.0);
            windows.push(QuotaWindow::new(
                "key_limit",
                "Лимит ключа",
                used_percent,
                None,
                None,
            ));
        }
    } else {
        // Лимит на ключе не установлен (unlimited) — отображаем накопленный расход ключа
        status_str = "ok";
        balances = vec![BalanceInfo {
            currency: "USD".to_owned(),
            total: format!("{:.2}", data.usage),
            granted: None,
            topped_up: None,
        }];
    }

    Ok(AccountState {
        id: id.to_owned(),
        name: name.to_owned(),
        provider: "openrouter".to_owned(),
        status: status_str.to_owned(),
        plan: data.label.or_else(|| Some("API Key".to_owned())),
        email: None,
        model: None,
        windows,
        balances,
        error: None,
        updated_at: crate::util::unix_now(),
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::Mutex;

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    use super::*;

    struct MockServer {
        base_url: String,
        #[allow(dead_code)]
        requests: Arc<Mutex<Vec<String>>>,
        handle: tokio::task::JoinHandle<()>,
    }

    impl MockServer {
        async fn start_responder<F>(respond: F) -> Self
        where
            F: Fn(&str) -> (u16, &'static str, String) + Send + Sync + 'static,
        {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let requests = Arc::new(Mutex::new(Vec::new()));
            let captured = Arc::clone(&requests);
            let respond = Arc::new(respond);

            let handle = tokio::spawn(async move {
                while let Ok((mut socket, _)) = listener.accept().await {
                    let mut buffer = vec![0_u8; 4096];
                    let read = socket.read(&mut buffer).await.unwrap_or(0);
                    if read == 0 {
                        continue;
                    }
                    let raw = String::from_utf8_lossy(&buffer[..read]).into_owned();
                    captured.lock().unwrap().push(raw.clone());
                    let (code, status_text, body) = respond(&raw);
                    let response = format!(
                        "HTTP/1.1 {code} {status_text}\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = socket.write_all(response.as_bytes()).await;
                    let _ = socket.flush().await;
                }
            });

            Self {
                base_url: format!("http://{address}"),
                requests,
                handle,
            }
        }
    }

    impl Drop for MockServer {
        fn drop(&mut self) {
            self.handle.abort();
        }
    }

    #[tokio::test]
    async fn parses_management_credits_successfully() {
        let server = MockServer::start_responder(|req| {
            if req.contains("GET /credits") {
                (
                    200,
                    "OK",
                    r#"{"data":{"total_credits":50.0,"total_usage":10.5}}"#.to_owned(),
                )
            } else {
                (404, "Not Found", "{}".to_owned())
            }
        })
        .await;

        let state = fetch_with_key("or", "OpenRouter", "test-key", &server.base_url)
            .await
            .unwrap();

        assert_eq!(state.status, "ok");
        assert_eq!(state.balances.len(), 1);
        assert_eq!(state.balances[0].total, "39.50");
        assert_eq!(state.balances[0].currency, "USD");
    }

    #[tokio::test]
    async fn falls_back_to_key_when_credits_forbidden() {
        let server = MockServer::start_responder(|req| {
            if req.contains("GET /credits") {
                (403, "Forbidden", r#"{"error":{"message":"Only management keys"}}"#.to_owned())
            } else if req.contains("GET /key") {
                (200, "OK", r#"{"data":{"label":"work-key","limit":20.0,"limit_remaining":15.0,"usage":5.0}}"#.to_owned())
            } else {
                (404, "Not Found", "{}".to_owned())
            }
        }).await;

        let state = fetch_with_key("or", "OpenRouter", "test-key", &server.base_url)
            .await
            .unwrap();

        assert_eq!(state.status, "ok");
        assert_eq!(state.balances.len(), 1);
        assert_eq!(state.balances[0].total, "15.00");
        assert_eq!(state.windows.len(), 1);
        assert_eq!(state.windows[0].used_percent, 25.0);
        assert_eq!(state.plan.as_deref(), Some("work-key"));
    }

    #[tokio::test]
    async fn handles_unlimited_inference_key() {
        let server = MockServer::start_responder(|req| {
            if req.contains("GET /credits") {
                (403, "Forbidden", r#"{"error":{}}"#.to_owned())
            } else if req.contains("GET /key") {
                (200, "OK", r#"{"data":{"label":"unlimited","limit":null,"limit_remaining":null,"usage":12.34}}"#.to_owned())
            } else {
                (404, "Not Found", "{}".to_owned())
            }
        }).await;

        let state = fetch_with_key("or", "OpenRouter", "test-key", &server.base_url)
            .await
            .unwrap();

        assert_eq!(state.status, "ok");
        assert_eq!(state.balances[0].total, "12.34");
        assert!(state.windows.is_empty());
    }

    #[tokio::test]
    async fn unauthorized_is_an_error() {
        let server = MockServer::start_responder(|_| {
            (
                401,
                "Unauthorized",
                r#"{"error":{"message":"Invalid key"}}"#.to_owned(),
            )
        })
        .await;

        let err = fetch_with_key("or", "OpenRouter", "bad-key", &server.base_url)
            .await
            .unwrap_err()
            .to_string();

        assert!(err.contains("401"), "{err}");
    }
}

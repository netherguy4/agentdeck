//! Official-harness login inside a PTY.
//!
//! Spawns `claude auth login` / `codex login [--device-auth]` in a
//! portable-pty, parses URL / code-prompt / device-code events out of the
//! incremental child output, and forwards a pasted code back to the child.
//! Cancellation is SIGTERM to the child's process group only (never the
//! caller's own processes), then SIGKILL after a grace period. Pasted codes
//! are written to the child and never logged.

use std::collections::HashSet;
use std::io::Read;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use portable_pty::{CommandBuilder, PtySize, native_pty_system};

use crate::registry::Harness;

/// Localhost callback port the default `codex login` flow listens on.
pub const CODEX_CALLBACK_PORT: u16 = 1455;

const CODE_PROMPT_MARKER: &str = "Paste code here if prompted";
const DEVICE_CODE_MARKER: &str = "one-time code";

/// Incremental login-UI event parsed from child output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoginEvent {
    /// Complete login URL, from an OSC 8 target or plain `https://` text.
    Url(String),
    /// The child asks for a pasted code (Claude's prompt).
    CodePrompt,
    /// Codex `--device-auth` one-time code, for entry in the browser.
    DeviceCode(String),
    /// Sanitized (ANSI-stripped) fresh output text, for relay.
    Output(String),
    /// The child exited; the numeric wait status.
    Exited(u32),
}

/// What to log in to.
#[derive(Debug, Clone)]
pub struct LoginTarget {
    pub home: PathBuf,
    pub harness: Harness,
    pub binary: Option<String>,
    pub device_auth: bool,
}

/// A live login session. Dropping it cancels the child.
pub struct LoginSession {
    pub events: mpsc::Receiver<LoginEvent>,
    writer: Box<dyn std::io::Write + Send>,
    #[allow(dead_code)]
    master: Box<dyn portable_pty::MasterPty + Send>,
    pid: Option<u32>,
    cancelled: bool,
}

impl LoginSession {
    /// Write one pasted-code line to the child. The value is never logged.
    pub fn send_line(&mut self, line: &str) -> anyhow::Result<()> {
        use std::io::Write as _;
        writeln!(self.writer, "{line}")?;
        self.writer.flush()?;
        Ok(())
    }

    /// SIGTERM the child process group only, then SIGKILL after a grace
    /// period. Never touches processes outside the child's group.
    pub fn cancel(&mut self) {
        if self.cancelled {
            return;
        }
        self.cancelled = true;
        let Some(pid) = self.pid else { return };
        let pgid = pid as libc::pid_t;
        // Child runs setsid'd in the pty, so its pgid == pid: only its group.
        unsafe {
            libc::killpg(pgid, libc::SIGTERM);
        }
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            if !process_group_alive(pgid) {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        if process_group_alive(pgid) {
            unsafe {
                libc::killpg(pgid, libc::SIGKILL);
            }
        }
    }
}

impl Drop for LoginSession {
    fn drop(&mut self) {
        self.cancel();
    }
}

fn process_group_alive(pgid: libc::pid_t) -> bool {
    unsafe {
        if libc::killpg(pgid, 0) == 0 {
            return true;
        }
        std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
    }
}

/// Fail unless the Codex localhost callback port is free. Never kills
/// whatever holds it; the caller should suggest `--device-auth` instead.
pub fn ensure_codex_callback_port_free() -> anyhow::Result<()> {
    match std::net::TcpListener::bind(("127.0.0.1", CODEX_CALLBACK_PORT)) {
        Ok(listener) => {
            drop(listener);
            Ok(())
        }
        Err(e) => anyhow::bail!(
            "127.0.0.1:{CODEX_CALLBACK_PORT} is in use ({e}); the default codex login needs \
             that callback port — stop whatever holds it, or use \
             `agentdeck login <id> --device-auth`"
        ),
    }
}

fn login_command(target: &LoginTarget) -> anyhow::Result<(String, Vec<String>)> {
    match target.harness {
        Harness::Claude => {
            if target.device_auth {
                anyhow::bail!("--device-auth is only supported for codex instances");
            }
            Ok((
                target.binary.clone().unwrap_or_else(|| "claude".into()),
                vec!["auth".into(), "login".into()],
            ))
        }
        Harness::Codex => {
            // File store keeps each account's credentials in its own CODEX_HOME (R2), never the shared keyring.
            let mut args = vec![
                "-c".to_string(),
                r#"cli_auth_credentials_store="file""#.to_string(),
                "login".to_string(),
            ];
            if target.device_auth {
                args.push("--device-auth".into());
            }
            Ok((
                target.binary.clone().unwrap_or_else(|| "codex".into()),
                args,
            ))
        }
        Harness::Opencode => {
            if target.device_auth {
                anyhow::bail!("--device-auth is only supported for codex instances");
            }
            Ok((
                target.binary.clone().unwrap_or_else(|| "opencode".into()),
                vec!["auth".into(), "login".into()],
            ))
        }
        other => anyhow::bail!("login is not implemented for {} instances", other.as_str()),
    }
}

fn is_primary_claude_home(home: &PathBuf) -> bool {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .is_some_and(|h| h.join(".claude") == *home)
}

/// Spawn the official login CLI for `target` in a PTY.
pub fn start(target: &LoginTarget) -> anyhow::Result<LoginSession> {
    let (program, args) = login_command(target)?;
    if matches!(target.harness, Harness::Codex) && !target.device_auth {
        ensure_codex_callback_port_free()?;
    }
    let mut extra = Vec::new();
    match target.harness {
        Harness::Claude => {
            // The primary profile must keep its default location.
            if !is_primary_claude_home(&target.home) {
                extra.push((
                    "CLAUDE_CONFIG_DIR".to_string(),
                    target.home.display().to_string(),
                ));
            }
        }
        Harness::Codex => extra.push(("CODEX_HOME".to_string(), target.home.display().to_string())),
        Harness::Opencode => {
            // Instance home is the data dir; point XDG_DATA_HOME at its parent.
            let Some(parent) = target.home.parent() else {
                anyhow::bail!(
                    "opencode instance {:?} has no parent dir for XDG_DATA_HOME",
                    target.home
                );
            };
            extra.push(("XDG_DATA_HOME".to_string(), parent.display().to_string()));
        }
        other => anyhow::bail!("login is not implemented for {} instances", other.as_str()),
    }
    start_with_command(&program, &args, &extra)
}

/// Spawn an arbitrary login-like command in a PTY (used by tests with a
/// fake script). `extra_env` overrides the inherited environment.
pub fn start_with_command(
    program: &str,
    args: &[String],
    extra_env: &[(String, String)],
) -> anyhow::Result<LoginSession> {
    let pair = native_pty_system().openpty(PtySize {
        rows: 30,
        cols: 100,
        pixel_width: 0,
        pixel_height: 0,
    })?;
    let mut cmd = CommandBuilder::new(program);
    for arg in args {
        cmd.arg(arg);
    }
    for (key, value) in extra_env {
        cmd.env(key, value);
    }
    // Never let the child open windows on the user's display.
    cmd.env_remove("DISPLAY");
    cmd.env_remove("WAYLAND_DISPLAY");
    if cmd.get_env("TERM").is_none() {
        cmd.env("TERM", "xterm-256color");
    }
    // BROWSER is forwarded as configured (inherited) so the child can be
    // observed with a recorder script; nothing is opened by agentdeck itself.
    let portable_pty::PtyPair { slave, master } = pair;
    let mut child = slave.spawn_command(cmd)?;
    drop(slave);
    let pid = child.process_id();
    let reader = master.try_clone_reader()?;
    let writer = master.take_writer()?;
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut parser = OutputParser::default();
        let mut chunk = [0u8; 4096];
        read_loop(
            &mut (reader as Box<dyn Read + Send>),
            &mut chunk,
            &mut parser,
            &tx,
        );
        for event in parser.flush() {
            let _ = tx.send(event);
        }
        let code = child.wait().map(|s| s.exit_code()).unwrap_or(1);
        let _ = tx.send(LoginEvent::Exited(code));
    });
    Ok(LoginSession {
        events: rx,
        writer,
        master,
        pid,
        cancelled: false,
    })
}

fn read_loop(
    reader: &mut Box<dyn Read + Send>,
    chunk: &mut [u8],
    parser: &mut OutputParser,
    tx: &mpsc::Sender<LoginEvent>,
) {
    loop {
        match reader.read(chunk) {
            Ok(0) => break, // EOF: slave side closed
            Ok(n) => {
                for event in parser.feed(&chunk[..n]) {
                    if tx.send(event).is_err() {
                        return;
                    }
                }
            }
            // EIO arrives on Linux once the slave side is gone; also EOF.
            Err(_) => break,
        }
    }
}

/// Incremental parser over arbitrary PTY read splits. Decoded text
/// accumulates, so split UTF-8/CSI/OSC sequences still resolve; a trailing
/// unterminated `https://` fragment is held back (never rebuilt from
/// wrapped lines) until a delimiter arrives or `flush` runs at EOF.
#[derive(Debug, Default)]
struct OutputParser {
    raw: Vec<u8>,
    text: String,
    seen_urls: HashSet<String>,
    code_prompted: bool,
    seen_device_codes: HashSet<String>,
}

impl OutputParser {
    fn feed(&mut self, bytes: &[u8]) -> Vec<LoginEvent> {
        let mut events = Vec::new();
        let decoded = decode_append(&mut self.raw, bytes);
        if decoded.is_empty() {
            return events;
        }
        self.text.push_str(&decoded);
        for url in osc8_targets(&self.text) {
            if self.seen_urls.insert(url.clone()) {
                events.push(LoginEvent::Url(url));
            }
        }
        for (url, complete) in plain_urls(&self.text) {
            if complete && self.seen_urls.insert(url.clone()) {
                events.push(LoginEvent::Url(url));
            }
        }
        if !self.code_prompted && self.text.contains(CODE_PROMPT_MARKER) {
            self.code_prompted = true;
            events.push(LoginEvent::CodePrompt);
        }
        if self.text.contains(DEVICE_CODE_MARKER) {
            for code in device_codes(&self.text) {
                if self.seen_device_codes.insert(code.clone()) {
                    events.push(LoginEvent::DeviceCode(code));
                }
            }
        }
        let sanitized = sanitize(&decoded);
        if !sanitized.is_empty() {
            events.push(LoginEvent::Output(sanitized));
        }
        events
    }

    fn flush(&mut self) -> Vec<LoginEvent> {
        let mut events = Vec::new();
        if !self.raw.is_empty() {
            let rest = String::from_utf8_lossy(&self.raw).into_owned();
            self.raw.clear();
            self.text.push_str(&rest);
        }
        // Emit any trailing URL fragment held back during streaming.
        for (url, _) in plain_urls(&format!("{} ", self.text)) {
            if self.seen_urls.insert(url.clone()) {
                events.push(LoginEvent::Url(url));
            }
        }
        if !self.code_prompted && self.text.contains(CODE_PROMPT_MARKER) {
            self.code_prompted = true;
            events.push(LoginEvent::CodePrompt);
        }
        events
    }
}

/// Move complete UTF-8 out of `pending`, keeping an incomplete tail.
fn decode_append(pending: &mut Vec<u8>, bytes: &[u8]) -> String {
    pending.extend_from_slice(bytes);
    let mut out = String::new();
    loop {
        match std::str::from_utf8(pending) {
            Ok(valid) => {
                out.push_str(valid);
                pending.clear();
                return out;
            }
            Err(e) => {
                let valid = e.valid_up_to();
                if valid > 0 {
                    // valid_up_to is always a char boundary.
                    out.push_str(std::str::from_utf8(&pending[..valid]).unwrap_or(""));
                    pending.drain(..valid);
                }
                match e.error_len() {
                    Some(len) => {
                        out.push('\u{FFFD}');
                        pending.drain(..len);
                    }
                    // Incomplete multi-byte sequence at the end: wait for more.
                    None => return out,
                }
            }
        }
    }
}

/// Complete OSC 8 hyperlink targets (`ESC ] 8 ; params ; URI (BEL | ESC \)`).
fn osc8_targets(text: &str) -> Vec<String> {
    let bytes = text.as_bytes();
    let mut urls = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let Some(open) = find_sub(bytes, i, b"\x1b]8;") else {
            break;
        };
        let rest = &bytes[open + 4..];
        let Some(end) = osc_end_len(rest) else {
            break; // unterminated: held back for the next read
        };
        let inner = &rest[..rest.len() - end];
        if let Some(semi) = inner.iter().position(|&b| b == b';')
            && let Ok(uri) = std::str::from_utf8(&inner[semi + 1..])
            && !uri.is_empty()
        {
            urls.push(uri.to_string());
        }
        i = open + 4 + rest.len();
    }
    urls
}

/// Length of the trailing ST (`BEL` or `ESC \`) if `rest` ends one, else None.
/// Only complete sequences are returned, so split OSC 8s are held back.
fn osc_end_len(rest: &[u8]) -> Option<usize> {
    // A complete sequence must contain its terminator somewhere.
    if let Some(bel) = rest.iter().position(|&b| b == 0x07) {
        return Some(rest.len() - bel);
    }
    find_sub(rest, 0, b"\x1b\\").map(|pos| rest.len() - pos)
}

fn find_sub(haystack: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    haystack[from..]
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|p| p + from)
}

/// Plain `https://` URLs. Each item flags whether it ended at a delimiter
/// (`false` = runs to the end of text, possibly split mid-read: hold back).
fn plain_urls(text: &str) -> Vec<(String, bool)> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(start) = find_sub(bytes, i, b"https://") {
        let mut end = start + 8;
        while end < bytes.len() && is_url_byte(bytes[end]) {
            end += 1;
        }
        let mut url = text[start..end].to_string();
        // Strip prose punctuation the terminal text may append, not the URL.
        while url.ends_with(['.', ',', ';', ':', '!', '?', '\'', '"', ')']) {
            url.pop();
        }
        if url.len() > 8 {
            out.push((url, end < bytes.len()));
        }
        i = if end == start + 8 { start + 8 } else { end };
    }
    out
}

fn is_url_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric()
        || matches!(
            b,
            b'-' | b'.'
                | b'_'
                | b'~'
                | b':'
                | b'/'
                | b'?'
                | b'#'
                | b'['
                | b']'
                | b'@'
                | b'!'
                | b'$'
                | b'&'
                | b'\''
                | b'('
                | b')'
                | b'*'
                | b'+'
                | b','
                | b';'
                | b'='
                | b'%'
                | b'|'
        )
}

/// Codex `--device-auth` one-time code, only inside the device-auth banner so
/// stray `XXXX-XXXXX` shapes elsewhere are not mistaken for it.
fn device_codes(text: &str) -> Vec<String> {
    let marker = text.find(DEVICE_CODE_MARKER).unwrap_or(0);
    let after = &text[marker..];
    let bytes = after.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i + 10 < bytes.len() {
        if bytes[i].is_ascii_uppercase_or_digit()
            && bytes[i + 1].is_ascii_uppercase_or_digit()
            && bytes[i + 2].is_ascii_uppercase_or_digit()
            && bytes[i + 3].is_ascii_uppercase_or_digit()
            && bytes[i + 4] == b'-'
        {
            let mut j = i + 5;
            while j < bytes.len() && bytes[j].is_ascii_uppercase_or_digit() {
                j += 1;
            }
            // E1: 4 chars, hyphen, 5 chars; accept 4-6 to tolerate versions.
            if (4..=6).contains(&(j - (i + 5)))
                && (i == 0 || !bytes[i - 1].is_ascii_uppercase_or_digit())
                && (j >= bytes.len() || !bytes[j].is_ascii_uppercase_or_digit())
            {
                out.push(after[i..j].to_string());
                i = j;
                continue;
            }
        }
        i += 1;
    }
    out
}

trait UpperDigit {
    fn is_ascii_uppercase_or_digit(&self) -> bool;
}

impl UpperDigit for u8 {
    fn is_ascii_uppercase_or_digit(&self) -> bool {
        self.is_ascii_uppercase() || self.is_ascii_digit()
    }
}

/// Strip ANSI CSI/OSC sequences and stray control bytes; keep `\n`/`\t`.
/// Visible link text inside OSC 8 is kept; only the framing is removed.
pub fn sanitize(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            0x1b => {
                if bytes.get(i + 1) == Some(&b'[') {
                    // CSI: ESC [ params intermediates final(@-~).
                    i += 2;
                    while i < bytes.len() && !(0x40..=0x7e).contains(&bytes[i]) {
                        i += 1;
                    }
                    i += 1;
                } else if bytes.get(i + 1) == Some(&b']') {
                    // OSC: skip to BEL or ESC \.
                    i += 2;
                    while i < bytes.len() {
                        if bytes[i] == 0x07 {
                            i += 1;
                            break;
                        }
                        if bytes[i] == 0x1b && bytes.get(i + 1) == Some(&b'\\') {
                            i += 2;
                            break;
                        }
                        i += 1;
                    }
                } else {
                    // Lone ESC or ESC + single char: drop both.
                    i += 2;
                }
            }
            b'\r' => i += 1,
            0x00..=0x08 | 0x0b..=0x1f | 0x7f => i += 1,
            _ => {
                let ch_len = text[i..].chars().next().map(|c| c.len_utf8()).unwrap_or(1);
                out.push_str(&text[i..i + ch_len]);
                i += ch_len;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    const CLAUDE_URL: &str = "https://claude.com/cai/oauth/authorize?code=true&client_id=abc&response_type=code&redirect_uri=https%3A%2F%2Fplatform.claude.com%2Foauth%2Fcode%2Fcallback&scope=openid&code_challenge=xyz&code_challenge_method=S256&state=st1";

    fn claude_bytes() -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(
            b"Opening browser to sign in...\r\nIf the browser didn't open, visit: ",
        );
        out.extend_from_slice(
            format!("\x1b]8;;{CLAUDE_URL}\x07{CLAUDE_URL}\x1b]8;;\x07\r\n").as_bytes(),
        );
        out.extend_from_slice(b"Paste code here if prompted > ");
        out
    }

    fn urls_and_prompts(events: &[LoginEvent]) -> (Vec<String>, usize) {
        let urls = events
            .iter()
            .filter_map(|e| match e {
                LoginEvent::Url(u) => Some(u.clone()),
                _ => None,
            })
            .collect();
        let prompts = events
            .iter()
            .filter(|e| matches!(e, LoginEvent::CodePrompt))
            .count();
        (urls, prompts)
    }

    #[test]
    fn osc8_and_plain_url_deduplicated_with_prompt() {
        let mut parser = OutputParser::default();
        let events = parser.feed(&claude_bytes());
        let (urls, prompts) = urls_and_prompts(&events);
        assert_eq!(urls, vec![CLAUDE_URL.to_string()]);
        assert_eq!(prompts, 1);
        assert!(events.iter().any(|e| matches!(e, LoginEvent::Output(_))));
    }

    #[test]
    fn split_reads_still_parse_once() {
        let bytes = claude_bytes();
        for width in [1usize, 2, 3, 5, 7, 13] {
            let mut parser = OutputParser::default();
            let mut events = Vec::new();
            for chunk in bytes.chunks(width) {
                events.extend(parser.feed(chunk));
            }
            events.extend(parser.flush());
            let (urls, prompts) = urls_and_prompts(&events);
            assert_eq!(urls, vec![CLAUDE_URL.to_string()], "width {width}");
            assert_eq!(prompts, 1, "width {width}");
        }
    }

    #[test]
    fn split_utf8_and_split_osc8_hold_until_complete() {
        // Bytes split inside the 2-byte `é`, then a split OSC 8.
        let full = "caf\u{00e9} \x1b]8;;https://example.com/a".to_string();
        let bytes = full.as_bytes();
        let split = "caf".len() + 1; // mid-`é`
        let mut parser = OutputParser::default();
        let first: Vec<_> = parser.feed(&bytes[..split]).into_iter().collect();
        assert!(first.iter().all(|e| !matches!(e, LoginEvent::Url(_))));
        let second = parser.feed(&bytes[split..]);
        assert!(second.iter().all(|e| !matches!(e, LoginEvent::Url(_))));
        let third = parser.feed(b"\x07visible\x1b]8;;\x07\n");
        let (urls, _) = urls_and_prompts(&third);
        assert_eq!(urls, vec!["https://example.com/a".to_string()]);
        let outputs: String = first
            .iter()
            .chain(second.iter())
            .chain(third.iter())
            .filter_map(|e| match e {
                LoginEvent::Output(t) => Some(t.clone()),
                _ => None,
            })
            .collect();
        assert!(outputs.contains("caf\u{00e9}"), "{outputs:?}");
    }

    #[test]
    fn urls_never_rebuilt_from_wrapped_lines() {
        let mut parser = OutputParser::default();
        let events = parser.feed(b"visit https://example.com/foo\r\nbar-baz now\n");
        let (urls, _) = urls_and_prompts(&events);
        assert_eq!(urls, vec!["https://example.com/foo".to_string()]);
    }

    #[test]
    fn ansi_noise_stripped_device_code_found() {
        let mut parser = OutputParser::default();
        let banner = "\x1b[34mFollow these steps to sign in with ChatGPT using device code authorization:\x1b[0m\r\n\r\n1. Open this link in your browser and sign in to your account\r\n   https://auth.openai.com/codex/device\r\n\r\n2. Enter this one-time code (expires in 15 minutes)\r\n   \x1b[1mAB12-CD345\x1b[0m\r\n";
        let events = parser.feed(banner.as_bytes());
        let codes: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                LoginEvent::DeviceCode(c) => Some(c.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(codes, vec!["AB12-CD345".to_string()]);
        let (urls, _) = urls_and_prompts(&events);
        assert_eq!(
            urls,
            vec!["https://auth.openai.com/codex/device".to_string()]
        );
        for e in &events {
            if let LoginEvent::Output(t) = e {
                assert!(!t.contains('\x1b'), "ANSI leaked: {t:?}");
            }
        }
    }

    static N: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn fake_script_through_real_pty() {
        let dir = std::env::temp_dir().join(format!(
            "agentdeck-login-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("fake-login.sh");
        std::fs::write(
            &script,
            "#!/bin/bash\n\
             printf '\\033]8;;https://example.com/login?x=1\\007https://example.com/login?x=1\\033]8;;\\007\\r\\n'\n\
             printf 'Paste code here if prompted > '\n\
             IFS= read -r line\n\
             printf 'done\\r\\n'\n\
             exit 0\n",
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let mut session =
            start_with_command(&script.to_string_lossy(), &[], &[]).expect("spawn fake login");
        let mut saw_url = false;
        let mut saw_prompt = false;
        let deadline = Instant::now() + Duration::from_secs(15);
        let exit = loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match session.events.recv_timeout(left) {
                Ok(LoginEvent::Url(url)) => {
                    assert_eq!(url, "https://example.com/login?x=1");
                    saw_url = true;
                }
                Ok(LoginEvent::CodePrompt) => {
                    saw_prompt = true;
                    session.send_line("test-code-123").unwrap();
                }
                Ok(LoginEvent::Exited(code)) => break code,
                Ok(_) => {}
                Err(_) => panic!("timed out waiting for fake login exit"),
            }
        };
        assert!(saw_url && saw_prompt);
        assert_eq!(exit, 0);
        std::fs::remove_dir_all(&dir).ok();
    }
}

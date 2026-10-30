//! Rust port of the `agent-process` bash helper (`~/.general/scripts/agent-process`).
//!
//! Identical CLI (`start`/`list`/`status`/`stop`), exit codes, registry layout
//! (`$XDG_RUNTIME_DIR/ai-agent-processes/<id>.json`) and child env tags
//! (`AI_AGENT_MANAGED=1`, `AI_AGENT_PROCESS_ID`, `AI_AGENT_PROCESS_OWNER`).
//! Deviations that could not be matched 1:1 are listed in README.md.

use std::ffi::{OsStr, OsString};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const USAGE: &str = "Usage:\n  agent-process start --name NAME [--owner SESSION] [--cwd DIR] [--log FILE] -- COMMAND [ARG...]\n  agent-process list\n  agent-process status ID\n  agent-process stop [--owner SESSION] [--force] [--grace-seconds 1..300] [--no-escalate] ID\n\nStarts long-running agent-owned processes in a separate process group and records ownership.\nstop refuses processes owned by another agent session unless --force is explicitly used.\nstop waits 5 seconds by default, then sends SIGKILL; --no-escalate returns status 4 instead.\nA graceful timeout leaves the live process and its ownership registration intact.\n";

/// Entry point for argv[0] dispatch: `args` without argv[0]. Returns the exit code.
pub fn main(args: Vec<OsString>) -> i32 {
    let mut iter = args.into_iter();
    let command = iter.next();
    let rest: Vec<OsString> = iter.collect();
    match command.as_deref() {
        None => {
            print!("{USAGE}");
            0
        }
        Some(cmd) if cmd == OsStr::new("start") => cmd_start(&rest),
        Some(cmd) if cmd == OsStr::new("list") => cmd_list(),
        Some(cmd) if cmd == OsStr::new("status") => cmd_status(&rest),
        Some(cmd) if cmd == OsStr::new("stop") => cmd_stop(&rest),
        Some(cmd)
            if cmd == OsStr::new("-h")
                || cmd == OsStr::new("--help")
                || cmd == OsStr::new("help") =>
        {
            print!("{USAGE}");
            0
        }
        Some(cmd) => {
            eprintln!("Unknown command: {}", cmd.to_string_lossy());
            eprint!("{USAGE}");
            2
        }
    }
}

fn registry_dir() -> Option<PathBuf> {
    let runtime_parent = std::env::var_os("XDG_RUNTIME_DIR")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    let dir = if runtime_parent == Path::new("/tmp") {
        let uid = command_output("id", &["-u"])?;
        PathBuf::from(format!("/tmp/ai-agent-processes-{uid}"))
    } else {
        runtime_parent.join("ai-agent-processes")
    };
    if std::fs::create_dir_all(&dir).is_err() {
        return None;
    }
    let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
    Some(dir)
}

fn default_owner() -> Option<String> {
    for key in [
        "AI_AGENT_SESSION_ID",
        "CODEX_THREAD_ID",
        "CODEX_SESSION_ID",
        "CLAUDE_SESSION_ID",
        "GEMINI_SESSION_ID",
        "OPENCODE_SESSION_ID",
    ] {
        if let Some(value) = std::env::var_os(key)
            && !value.is_empty()
        {
            return Some(value.to_string_lossy().into_owned());
        }
    }
    None
}

/// Field 22 of /proc/<pid>/stat, read exactly like the script's `awk '{print $22}'`.
fn start_ticks(pid: u32) -> Option<String> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    stat.split_whitespace().nth(21).map(str::to_owned)
}

/// Process group of a pid, parsed after the comm field (equals `ps -o pgid=`).
fn pgrp_of(pid: u32) -> Option<u32> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let after_comm = stat.rsplit_once(')')?.1;
    // Fields after comm: state ppid pgrp session ...
    after_comm.split_whitespace().nth(2)?.parse().ok()
}

fn environ_has_tag(pid: u32, process_id: &str) -> bool {
    let env = std::fs::read(format!("/proc/{pid}/environ")).unwrap_or_default();
    let want = format!("AI_AGENT_PROCESS_ID={process_id}");
    env.split(|b| *b == 0).any(|entry| entry == want.as_bytes())
}

/// Pids in `pgid` whose environment carries the process tag (sorted).
fn tagged_group_pids(process_id: &str, pgid: u32) -> Vec<u32> {
    let mut found = Vec::new();
    let Ok(proc) = std::fs::read_dir("/proc") else {
        return found;
    };
    for entry in proc.flatten() {
        let name = entry.file_name();
        let Ok(pid): Result<u32, _> = name.to_string_lossy().parse() else {
            continue;
        };
        if pgrp_of(pid) != Some(pgid) {
            continue;
        }
        if environ_has_tag(pid, process_id) {
            found.push(pid);
        }
    }
    found.sort_unstable();
    found
}

struct Metadata {
    value: serde_json::Value,
    id: String,
    pid: u32,
    pgid: u32,
    ticks: String,
}

fn read_metadata(file: &Path) -> Option<Metadata> {
    let text = std::fs::read_to_string(file).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    Some(Metadata {
        id: value.get("id")?.as_str()?.to_owned(),
        pid: value.get("pid")?.as_u64()? as u32,
        pgid: value.get("pgid")?.as_u64()? as u32,
        ticks: value.get("start_ticks")?.as_str()?.to_owned(),
        value,
    })
}

fn metadata_is_live(meta: &Metadata) -> bool {
    if let Some(current) = start_ticks(meta.pid)
        && current == meta.ticks
    {
        return true;
    }
    !tagged_group_pids(&meta.id, meta.pgid).is_empty()
}

/// Literal port of the script's centisecond arithmetic on /proc/uptime.
fn monotonic_centiseconds() -> Option<i64> {
    let uptime = std::fs::read_to_string("/proc/uptime").ok()?;
    let first = uptime.split_whitespace().next()?;
    let (secs, frac) = match first.split_once('.') {
        Some((s, f)) => (s, f),
        None => (first, first),
    };
    Some(secs.parse::<i64>().ok()? * 100 + frac.parse::<i64>().ok()?)
}

fn utc_parts() -> (i64, u32, u32, u32, u32, u32) {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let days = secs.div_euclid(86_400);
    let sod = secs.rem_euclid(86_400);
    // Days to civil date (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    (
        if m <= 2 { y + 1 } else { y },
        m,
        d,
        (sod / 3600) as u32,
        ((sod % 3600) / 60) as u32,
        (sod % 60) as u32,
    )
}

fn opt_value(args: &[OsString], index: &mut usize, flag: &str) -> Result<OsString, i32> {
    *index += 1;
    args.get(*index).cloned().ok_or_else(|| {
        eprintln!("Missing value for {flag}");
        1
    })
}

#[allow(clippy::too_many_lines)]
fn cmd_start(args: &[OsString]) -> i32 {
    let mut name = OsString::new();
    let mut owner = OsString::new();
    let mut cwd: Option<OsString> = None;
    let mut log: Option<OsString> = None;
    let mut i = 0;
    let mut command: &[OsString] = &[];
    while i < args.len() {
        let arg = args[i].as_os_str();
        if arg == OsStr::new("--name") {
            match opt_value(args, &mut i, "--name") {
                Ok(v) => name = v,
                Err(code) => return code,
            }
        } else if arg == OsStr::new("--owner") {
            match opt_value(args, &mut i, "--owner") {
                Ok(v) => owner = v,
                Err(code) => return code,
            }
        } else if arg == OsStr::new("--cwd") {
            match opt_value(args, &mut i, "--cwd") {
                Ok(v) => cwd = Some(v),
                Err(code) => return code,
            }
        } else if arg == OsStr::new("--log") {
            match opt_value(args, &mut i, "--log") {
                Ok(v) => log = Some(v),
                Err(code) => return code,
            }
        } else if arg == OsStr::new("--") {
            command = &args[i + 1..];
            break;
        } else {
            eprintln!("Unknown start option: {}", args[i].to_string_lossy());
            eprint!("{USAGE}");
            return 2;
        }
        i += 1;
    }

    if name.is_empty() {
        eprintln!("--name is required.");
        return 2;
    }
    if command.is_empty() {
        eprintln!("A command is required after --.");
        return 2;
    }
    let cwd = cwd
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."));
    if !cwd.is_dir() {
        eprintln!("Working directory does not exist: {}", cwd.display());
        return 2;
    }
    let cwd = std::fs::canonicalize(&cwd).unwrap_or(cwd);
    let owner = if owner.is_empty() {
        match default_owner() {
            Some(o) => o,
            None => {
                eprintln!("No stable agent session ID found. Pass --owner SESSION.");
                return 2;
            }
        }
    } else {
        owner.to_string_lossy().into_owned()
    };

    let Some(dir) = registry_dir() else {
        eprintln!("Cannot create registry directory.");
        return 1;
    };
    let uuid = std::fs::read_to_string("/proc/sys/kernel/random/uuid")
        .unwrap_or_default()
        .trim()
        .replace('-', "");
    let (y, mo, d, h, mi, s) = utc_parts();
    let process_id = format!("{y:04}{mo:02}{d:02}T{h:02}{mi:02}{s:02}Z-{uuid}");
    let meta_file = dir.join(format!("{process_id}.json"));
    let log_file = log
        .map(PathBuf::from)
        .unwrap_or_else(|| dir.join(format!("{process_id}.log")));
    if let Some(parent) = log_file.parent()
        && !parent.as_os_str().is_empty()
        && std::fs::create_dir_all(parent).is_err()
    {
        eprintln!("Cannot create log directory: {}", parent.display());
        return 1;
    }

    let Some(mut child) = spawn_detached(&cwd, &log_file, &process_id, &owner, command) else {
        eprintln!(
            "Process exited before it could be registered. See {}",
            log_file.display()
        );
        return 1;
    };
    let pid = child.id();
    let mut registered = false;
    for _ in 0..100 {
        if start_ticks(pid).is_some()
            && pgrp_of(pid) == Some(pid)
            && environ_has_tag(pid, &process_id)
        {
            registered = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    if !registered {
        let _ = child.wait();
        eprintln!(
            "Process exited before it could be registered. See {}",
            log_file.display()
        );
        return 1;
    }
    let pgid = pgrp_of(pid).unwrap_or(pid);
    let ticks = start_ticks(pid).unwrap_or_default();
    let (y, mo, d, h, mi, s) = utc_parts();
    let started_at = format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z");

    let mut meta = serde_json::Map::with_capacity(10);
    meta.insert("id".to_owned(), process_id.into());
    meta.insert(
        "name".to_owned(),
        name.to_string_lossy().into_owned().into(),
    );
    meta.insert("owner".to_owned(), owner.into());
    meta.insert("pid".to_owned(), pid.into());
    meta.insert("pgid".to_owned(), pgid.into());
    meta.insert("start_ticks".to_owned(), ticks.into());
    meta.insert("cwd".to_owned(), cwd.to_string_lossy().into_owned().into());
    meta.insert(
        "command".to_owned(),
        command
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .into(),
    );
    meta.insert(
        "log".to_owned(),
        log_file.to_string_lossy().into_owned().into(),
    );
    meta.insert("started_at".to_owned(), started_at.into());
    if write_metadata(&meta_file, &meta).is_err() {
        eprintln!("Cannot write registry entry.");
        return 1;
    }
    let mut out = serde_json::Value::Object(meta);
    out["live"] = true.into();
    println!("{}", serde_json::to_string_pretty(&out).unwrap_or_default());
    0
}

/// Run a helper command, return its trimmed stdout on success.
fn command_output(prog: &str, args: &[&str]) -> Option<String> {
    let out = std::process::Command::new(prog).args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

/// Spawn `(cd cwd; exec setsid env <tags> -- cmd)` exactly like the script:
/// stdin is /dev/null, stdout/stderr append to the log file.
fn spawn_detached(
    cwd: &Path,
    log: &Path,
    id: &str,
    owner: &str,
    cmd: &[OsString],
) -> Option<std::process::Child> {
    let log_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log)
        .ok()?;
    let log_err = log_file.try_clone().ok()?;
    let mut argv: Vec<OsString> = vec![
        OsString::from("env"),
        OsString::from("AI_AGENT_MANAGED=1"),
        OsString::from(format!("AI_AGENT_PROCESS_ID={id}")),
        OsString::from(format!("AI_AGENT_PROCESS_OWNER={owner}")),
    ];
    argv.extend(cmd.iter().cloned());
    std::process::Command::new("setsid")
        .args(&argv)
        .current_dir(cwd)
        .stdin(std::process::Stdio::null())
        .stdout(log_file)
        .stderr(log_err)
        .spawn()
        .ok()
}

fn write_metadata(
    file: &Path,
    meta: &serde_json::Map<String, serde_json::Value>,
) -> std::io::Result<()> {
    let tmp = file.with_extension(format!("json.tmp.{}", std::process::id()));
    let pretty = serde_json::to_string_pretty(&serde_json::Value::Object(meta.clone()))
        .map(|s| s + "\n")
        .unwrap_or_default();
    std::fs::write(&tmp, pretty)?;
    std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
    std::fs::rename(&tmp, file)?;
    Ok(())
}

fn cmd_list() -> i32 {
    let Some(dir) = registry_dir() else {
        eprintln!("Cannot create registry directory.");
        return 1;
    };
    println!(
        "{:<49} {:<5} {:<8} {:<24} OWNER",
        "ID", "LIVE", "PID", "NAME"
    );
    let mut entries: Vec<PathBuf> = std::fs::read_dir(&dir)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|e| e == "json"))
                .collect()
        })
        .unwrap_or_default();
    entries.sort();
    if entries.is_empty() {
        println!("(no registered processes)");
        return 0;
    }
    for file in &entries {
        let Some(meta) = read_metadata(file) else {
            continue;
        };
        let live = if metadata_is_live(&meta) { "yes" } else { "no" };
        println!(
            "{:<49} {:<5} {:<8} {:<24} {}",
            meta.id,
            live,
            meta.pid.to_string(),
            meta.value
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("null"),
            meta.value
                .get("owner")
                .and_then(|v| v.as_str())
                .unwrap_or("null"),
        );
    }
    0
}

fn cmd_status(args: &[OsString]) -> i32 {
    let Some(id) = args.first() else {
        eprintln!("Process ID is required");
        return 1;
    };
    let Some(dir) = registry_dir() else {
        eprintln!("Cannot create registry directory.");
        return 1;
    };
    let file = dir.join(format!("{}.json", id.to_string_lossy()));
    let Some(meta) = read_metadata(&file) else {
        eprintln!("Unknown process ID: {}", id.to_string_lossy());
        return 1;
    };
    let live = metadata_is_live(&meta);
    let mut out = meta.value;
    out["live"] = live.into();
    println!("{}", serde_json::to_string_pretty(&out).unwrap_or_default());
    0
}

fn valid_grace_seconds(value: &OsStr) -> Option<i64> {
    let s = value.to_str()?;
    if !(1..=3).contains(&s.len()) || !s.bytes().all(|b| b.is_ascii_digit()) || s.starts_with('0') {
        return None;
    }
    let n: i64 = s.parse().ok()?;
    (n <= 300).then_some(n)
}

#[allow(clippy::too_many_lines)]
fn cmd_stop(args: &[OsString]) -> i32 {
    let mut owner = OsString::new();
    let mut force = false;
    let mut grace_seconds: i64 = 5;
    let mut no_escalate = false;
    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_os_str();
        if arg == OsStr::new("--owner") {
            match opt_value(args, &mut i, "--owner") {
                Ok(v) => owner = v,
                Err(code) => return code,
            }
        } else if arg == OsStr::new("--force") {
            force = true;
        } else if arg == OsStr::new("--grace-seconds") {
            i += 1;
            match args.get(i).and_then(|v| valid_grace_seconds(v)) {
                Some(n) => grace_seconds = n,
                None => {
                    eprintln!("--grace-seconds requires an integer from 1 to 300.");
                    return 2;
                }
            }
        } else if arg == OsStr::new("--no-escalate") {
            no_escalate = true;
        } else if arg.as_bytes().starts_with(b"--") {
            eprintln!("Unknown stop option: {}", args[i].to_string_lossy());
            return 2;
        } else {
            break;
        }
        i += 1;
    }
    let Some(id) = args.get(i) else {
        eprintln!("Process ID is required");
        return 1;
    };

    let Some(dir) = registry_dir() else {
        eprintln!("Cannot create registry directory.");
        return 1;
    };
    let file = dir.join(format!("{}.json", id.to_string_lossy()));
    let Some(meta) = read_metadata(&file) else {
        eprintln!("Unknown process ID: {}", id.to_string_lossy());
        return 1;
    };
    let recorded_owner = meta
        .value
        .get("owner")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let owner = if owner.is_empty() {
        if force {
            "explicit-user-approval".to_owned()
        } else {
            match default_owner() {
                Some(o) => o,
                None => {
                    eprintln!("No stable agent session ID found. Pass --owner SESSION.");
                    return 2;
                }
            }
        }
    } else {
        owner.to_string_lossy().into_owned()
    };
    if !force && owner != recorded_owner {
        eprintln!(
            "Refusing: {} belongs to agent session {recorded_owner}, not {owner}.",
            id.to_string_lossy()
        );
        return 3;
    }

    if !metadata_is_live(&meta) {
        println!(
            "Process {} is no longer running; no signal sent.",
            id.to_string_lossy()
        );
        return 0;
    }

    // Like the script's `kill -TERM -- "-$pgid"` under `set -e`: a failed signal aborts with its status.
    let term = std::process::Command::new("kill")
        .args(["-TERM", "--", &format!("-{}", meta.pgid)])
        .status();
    if term.map(|s| !s.success()).unwrap_or(true) {
        return 1;
    }
    let Some(now) = monotonic_centiseconds() else {
        eprintln!("Cannot read process clock.");
        return 1;
    };
    let deadline = now + grace_seconds * 100;
    loop {
        if read_metadata(&file).is_none_or(|m| !metadata_is_live(&m)) {
            println!(
                "Stopped {} (process group {}).",
                id.to_string_lossy(),
                meta.pgid
            );
            return 0;
        }
        if monotonic_centiseconds().is_none_or(|t| t >= deadline) {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    if no_escalate {
        eprintln!(
            "Process {} is still running after {grace_seconds} seconds; no SIGKILL sent, registration retained.",
            id.to_string_lossy()
        );
        return 4;
    }
    // Like the script's `kill -KILL -- "-$pgid" || true`: failure is ignored.
    let _ = std::process::Command::new("kill")
        .args(["-KILL", "--", &format!("-{}", meta.pgid)])
        .status();
    println!(
        "Stopped {} with SIGKILL after the graceful timeout.",
        id.to_string_lossy()
    );
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grace_validation_matches_bash_regex() {
        for good in ["1", "2", "5", "120", "300"] {
            assert_eq!(
                valid_grace_seconds(OsStr::new(good)),
                Some(good.parse().unwrap())
            );
        }
        for bad in [
            "0",
            "301",
            "-1",
            "1.5",
            "nope",
            "01",
            "999999999999999999999",
            "",
            " 5",
            "５",
        ] {
            assert_eq!(valid_grace_seconds(OsStr::new(bad)), None, "{bad:?}");
        }
    }

    #[test]
    fn utc_parts_formats_like_date_u() {
        let (y, mo, d, h, mi, s) = utc_parts();
        assert!((2025..=2030).contains(&y));
        assert!((1..=12).contains(&mo));
        assert!((1..=31).contains(&d));
        assert!(h < 24 && mi < 60 && s < 61);
    }
}

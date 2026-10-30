//! CLI smoke test: runs the built binary with temp config/state dirs.

use std::path::PathBuf;
use std::process::Command;

fn run(dir: &std::path::Path, args: &[&str]) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_agentdeck"))
        .args(args)
        .env("AGENTDECK_CONFIG_DIR", dir.join("config"))
        .env("AGENTDECK_STATE_DIR", dir.join("state"))
        .output()
        .unwrap();
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    (out.status.success(), text)
}

#[test]
fn instance_lifecycle() {
    let dir: PathBuf = std::env::temp_dir().join(format!(
        "agentdeck-cli-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    // Dry-run first: nothing lands on disk.
    let (ok, out) = run(
        &dir,
        &[
            "instance",
            "add",
            "--id",
            "x",
            "--harness",
            "claude",
            "--home",
            "/fake/x",
            "--dry-run",
        ],
    );
    assert!(ok, "{out}");
    assert!(!dir.join("config/instances.toml").exists());

    let (ok, _) = run(
        &dir,
        &[
            "instance",
            "add",
            "--id",
            "x",
            "--harness",
            "claude",
            "--home",
            "/fake/x",
        ],
    );
    assert!(ok);
    let (ok, out) = run(&dir, &["instance", "list", "--json"]);
    assert!(ok, "{out}");
    assert!(out.contains("\"id\": \"x\""), "{out}");

    let (ok, _) = run(&dir, &["instance", "set", "x", "--enabled", "false"]);
    assert!(ok);
    let (ok, out) = run(&dir, &["instance", "list"]);
    assert!(ok, "{out}");
    assert!(out.contains("enabled=false"), "{out}");

    let (ok, out) = run(&dir, &["instance", "remove", "x"]);
    assert!(ok, "{out}");
    assert!(out.contains("home dir kept"), "{out}");
    let (ok, out) = run(&dir, &["instance", "list", "--json"]);
    assert!(ok, "{out}");
    assert!(!out.contains("\"id\""), "{out}");

    // Bad id is rejected.
    let (ok, out) = run(
        &dir,
        &[
            "instance",
            "add",
            "--id",
            "Bad!",
            "--harness",
            "claude",
            "--home",
            "h",
        ],
    );
    assert!(!ok, "{out}");

    std::fs::remove_dir_all(&dir).ok();
}

//! Vendored router Node suite: runs every *.test.mjs in assets/router
//! offline per its README. Skips with a printed note when node is absent.

use std::path::PathBuf;
use std::process::Command;

fn router_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/router")
}

#[test]
fn router_node_suite() {
    if Command::new("node").arg("--version").output().is_err() {
        println!("note: node is absent, skipping router Node suite (run *.test.mjs by hand)");
        return;
    }
    // Isolated HOME/XDG/TMPDIR: every suite builds its own fixtures under temp dirs.
    // Short scratch name: the socket suite binds a unix socket under TMPDIR,
    // whose path must stay within the ~108-byte sun_path limit.
    let scratch: PathBuf = std::env::temp_dir().join(format!(
        "artr-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&scratch).unwrap();
    let mut entries: Vec<String> = std::fs::read_dir(router_dir())
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .filter(|n| n.ends_with(".test.mjs"))
        .collect();
    entries.sort();
    assert!(!entries.is_empty(), "no router *.test.mjs found");
    let mut failed = Vec::new();
    for t in entries {
        let out = Command::new("node")
            .arg(router_dir().join(&t))
            .env("HOME", &scratch)
            .env("XDG_STATE_HOME", scratch.join("state"))
            .env("XDG_RUNTIME_DIR", scratch.join("run"))
            .env("XDG_CONFIG_HOME", scratch.join("config"))
            .env("TMPDIR", &scratch)
            .env_remove("MODEL_ROUTE_ROUTER_JSON")
            .output()
            .unwrap();
        if !out.status.success() {
            failed.push(t.clone());
            eprintln!(
                "--- {t} FAILED (stdout) ---\n{}\n--- {t} FAILED (stderr) ---\n{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            );
        }
    }
    let _ = std::fs::remove_dir_all(&scratch);
    assert!(failed.is_empty(), "router Node suites failed: {failed:?}");
}

//! Runs the black-box shell suite against this crate's binary.
#[test]
fn shell_suite_against_port() {
    let bin = env!("CARGO_BIN_EXE_agent-process");
    let script = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/agent-process.test.sh");
    let output = std::process::Command::new("bash")
        .arg(script)
        .env("AGENT_PROCESS_BIN", bin)
        // The suite manages its own isolated XDG_RUNTIME_DIR; make sure no
        // ambient owner leaks in except the suite's own explicit one.
        .env_remove("CODEX_THREAD_ID")
        .env_remove("CODEX_SESSION_ID")
        .env_remove("CLAUDE_SESSION_ID")
        .env_remove("GEMINI_SESSION_ID")
        .env_remove("OPENCODE_SESSION_ID")
        .output()
        .expect("run shell suite");
    println!("{}", String::from_utf8_lossy(&output.stdout));
    eprintln!("{}", String::from_utf8_lossy(&output.stderr));
    assert!(output.status.success(), "shell suite failed");
}

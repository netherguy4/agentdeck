//! Headless GUI tests (never a window): fixture registry + usage state files,
//! EN/RU snapshot PNGs via iced_test, plus sidebar navigation.
//!
//! Snapshots live in tests/snapshots/ and are committed only while stable and
//! small; `matches_image` creates them on first run and compares afterwards.

use std::path::{Path, PathBuf};

use agentdeck_core::{Paths, Registry};
use agentdeck_gui::{app, data};

const SNAPSHOTS: &str = "tests/snapshots";

fn fixture_paths(tag: &str) -> (Paths, PathBuf, tempfile_guard::Guard) {
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "agentdeck-gui-{tag}-{}-{}",
        std::process::id(),
        N.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
    ));
    // Isolate the legacy fallback (`$XDG_STATE_HOME/ai-usage/state.json`) from
    // the real user tree; callers hold ENV_LOCK while this env is set.
    unsafe {
        std::env::set_var("XDG_STATE_HOME", dir.join("xdg"));
    }
    let _ = std::fs::remove_dir_all(&dir);
    let paths = Paths::new(dir.join("config"), dir.join("state"));
    (paths, dir.clone(), tempfile_guard::Guard(dir))
}

// Minimal RAII dir guard (tempfile is not a dependency; keep it that way).
mod tempfile_guard {
    use std::path::PathBuf;
    pub struct Guard(pub PathBuf);
    impl Drop for Guard {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}

fn write_registry(paths: &Paths) {
    Registry::update(paths, |r| {
        r.add(agentdeck_core::registry::AddSpec {
            id: "claude-main".into(),
            harness: agentdeck_core::Harness::Claude,
            display_name: Some("Claude main".into()),
            home: "/fake/claude".into(),
            binary: None,
            no_t3: false,
        })?;
        r.add(agentdeck_core::registry::AddSpec {
            id: "codex-second".into(),
            harness: agentdeck_core::Harness::Codex,
            display_name: None,
            home: "/fake/codex-second".into(),
            binary: None,
            no_t3: false,
        })?;
        r.add_usage(agentdeck_core::UsageEntry {
            id: "claude-main".into(),
            kind: agentdeck_core::UsageKind::Claude,
            instance: Some("claude-main".into()),
            display: "Claude main".into(),
            config_dir: Some("~/.claude".into()),
            codex_home: None,
            token_file: None,
            api_key_env: None,
            db_file: None,
            extra: Default::default(),
        })?;
        r.add_usage(agentdeck_core::UsageEntry {
            id: "openrouter-main".into(),
            kind: agentdeck_core::UsageKind::Openrouter,
            instance: None,
            display: "OpenRouter".into(),
            config_dir: None,
            codex_home: None,
            token_file: None,
            api_key_env: Some("AGENTDECK_TEST_KEY".into()),
            db_file: None,
            extra: Default::default(),
        })?;
        r.set_enabled("codex-second", false)?;
        Ok(())
    })
    .unwrap();
}

fn write_state(paths: &Paths) {
    let dir = paths.state_dir.join("usage");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("state.json"),
        serde_json::json!({
            "schema_version": 2,
            "updated_at": 1_769_000_000,
            "accounts": [
                {
                    "id": "claude-main", "name": "Claude main", "provider": "claude",
                    "status": "ok", "plan": "Max 5x", "email": null, "model": null,
                    "windows": [
                        {"key": "five_hour", "label": "5 hours",
                         "used_percent": 35.0, "remaining_percent": 65.0,
                         "duration_mins": 300, "resets_at": null, "scope": null},
                        {"key": "seven_day", "label": "Week",
                         "used_percent": 72.5, "remaining_percent": 27.5,
                         "duration_mins": 10080, "resets_at": null, "scope": null}
                    ],
                    "balances": [], "error": null, "updated_at": 1_769_000_000
                },
                {
                    "id": "openrouter-main", "name": "OpenRouter", "provider": "openrouter",
                    "status": "error", "plan": null, "email": null, "model": null,
                    "windows": [], "balances": [],
                    "error": "no key configured", "updated_at": 1_769_000_000
                }
            ]
        })
        .to_string(),
    )
    .unwrap();
}

fn write_t3(dir: &Path) {
    let t3 = dir.join("t3");
    std::fs::create_dir_all(t3.join("userdata")).unwrap();
    std::fs::create_dir_all(t3.join("caches")).unwrap();
    std::fs::write(
        t3.join("userdata/settings.json"),
        r#"{"providerInstances": {
            "claude-main": {"driver": "claudeAgent", "enabled": true,
                "config": {"homePath": "/fake/claude"}},
            "codex-second": {"driver": "codex", "enabled": false,
                "config": {"homePath": "/fake/codex-second"}}
        }}"#,
    )
    .unwrap();
    std::fs::write(
        t3.join("caches/claude-main.json"),
        r#"{"status": "ready", "checkedAt": "2026-10-24T00:00:00.000Z", "models": []}"#,
    )
    .unwrap();
    // Point the T3 root at the fixture (restored by the caller).
    unsafe {
        std::env::set_var("AGENTDECK_T3_DIR", &t3);
    }
}

fn load_with_fixtures(
    tag: &str,
    _env: &std::sync::MutexGuard<'_, ()>,
) -> (data::Data, Paths, tempfile_guard::Guard) {
    let (paths, dir, guard) = fixture_paths(tag);
    write_registry(&paths);
    write_state(&paths);
    write_t3(&dir);
    let data = data::load(&paths);
    (data, paths, guard)
}

/// `AGENTDECK_T3_DIR` is process-global: tests that point it at fixtures
/// hold this lock for their whole body so parallel tests cannot race.
static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn lock_env() -> std::sync::MutexGuard<'static, ()> {
    // A poisoned lock means a sibling test panicked: keep going so every
    // failure prints instead of cascading into PoisonError noise.
    let guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    unsafe {
        std::env::remove_var("AGENTDECK_T3_DIR");
    }
    guard
}

#[test]
fn instances_data_links_registry_t3_and_state() {
    let env = lock_env();
    let (data, _paths, _guard) = load_with_fixtures("data", &env);
    assert_eq!(data.instances.len(), 2);
    let claude = &data.instances[0];
    assert_eq!(claude.display_name, "Claude main");
    assert!(claude.enabled);
    assert_eq!(claude.t3.as_ref().unwrap().status.as_deref(), Some("ready"));
    assert_eq!(claude.quotas.len(), 1);
    assert_eq!(claude.quotas[0].bars.len(), 2);
    assert!((claude.quotas[0].bars[0].used_percent - 35.0).abs() < f64::EPSILON);
    let codex = &data.instances[1];
    assert!(!codex.enabled);
    assert!(codex.t3.as_ref().unwrap().status.is_none());
    assert!(codex.quotas.is_empty());
    assert_eq!(data.standalone.len(), 1);
    assert_eq!(
        data.standalone[0].error.as_deref(),
        Some("no key configured")
    );
}

#[test]
fn missing_state_file_is_no_data_not_an_error() {
    let _env = lock_env();
    let (paths, _dir, _guard) = fixture_paths("nodata");
    // Fixture T3 dir exists but is empty: no settings.json means "no T3".
    unsafe {
        std::env::set_var("AGENTDECK_T3_DIR", _dir.join("t3"));
    }
    write_registry(&paths);
    let data = data::load(&paths);
    assert!(data.error.is_none());
    assert!(data.instances[0].quotas[0].bars.is_empty());
    assert!(data.instances[0].t3.is_none());
}

#[test]
fn snapshots_en_ru_and_sidebar_navigation() {
    let _env = lock_env();
    let (data, paths, _guard) = load_with_fixtures("shots", &_env);
    std::fs::create_dir_all(SNAPSHOTS).unwrap();

    for (lang, stem) in [("en", "instances_en"), ("ru", "instances_ru")] {
        agentdeck_gui::i18n::set_language(lang);
        let state = app::State::for_tests(paths.clone(), data.clone());
        let mut ui = iced_test::simulator(app::view(&state));
        let snapshot = ui.snapshot(&iced::Theme::Dark).expect("headless snapshot");
        // Golden path gains a `-{renderer}` suffix inside matches_image;
        // exactly one file per stem must exist afterwards.
        assert!(
            snapshot
                .matches_image(format!("{SNAPSHOTS}/{stem}.png"))
                .expect("snapshot compare/write"),
            "snapshot {stem} changed"
        );
        let goldens: Vec<_> = std::fs::read_dir(SNAPSHOTS)
            .expect("snapshots dir")
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| {
                p.file_stem()
                    .and_then(|s| s.to_str())
                    .is_some_and(|s| s.starts_with(stem))
                    && p.extension().and_then(|e| e.to_str()) == Some("png")
            })
            .collect();
        assert_eq!(goldens.len(), 1, "one golden per stem: {goldens:?}");
        let bytes = std::fs::metadata(&goldens[0]).expect("golden png").len();
        assert!(bytes > 1_024, "golden {} looks empty", goldens[0].display());
    }

    // Sidebar navigation: click Drift, the placeholder replaces Instances.
    agentdeck_gui::i18n::set_language("en");
    let mut state = app::State::for_tests(paths, data);
    let mut ui = iced_test::simulator(app::view(&state));
    ui.click("Drift").expect("drift nav button");
    for message in ui.into_messages() {
        let _ = app::update(&mut state, message);
    }
    assert!(matches!(state.page(), agentdeck_gui::Page::Drift));
    let mut ui = iced_test::simulator(app::view(&state));
    ui.find("Coming in a later unit.")
        .expect("drift placeholder body");
}

fn fixture_proc(pid: u32, cmd: &str) -> agentdeck_core::processes::ProcInfo {
    agentdeck_core::processes::ProcInfo {
        pid,
        ppid: 1,
        pgrp: pid,
        comm: cmd.to_string(),
        argv: vec![cmd.to_string(), "60".to_string()],
        cpu_percent: 0.5,
        rss_bytes: 2_000_000,
        read_bytes: Some(4096),
        write_bytes: Some(0),
        start_ticks: 1000,
        scope: None,
        owner_tag: Some("ses1".to_string()),
    }
}

fn fixture_groups() -> Vec<agentdeck_core::processes::SessionGroup> {
    use agentdeck_core::processes::{GroupKind, SessionGroup};
    vec![
        SessionGroup {
            kind: GroupKind::Agent {
                owner: "ses1".to_string(),
                id: "abc".to_string(),
            },
            title: "ses1 (abc)".to_string(),
            pid: 100,
            start_ticks: 1000,
            pgrp: 100,
            can_stop: true,
            processes: vec![fixture_proc(100, "sleep"), fixture_proc(101, "sh")],
            cpu_percent: 1.0,
            rss_bytes: 4_000_000,
            read_bytes: Some(8192),
            write_bytes: Some(0),
            scope_mem_bytes: None,
        },
        SessionGroup {
            kind: GroupKind::T3 {
                scope: "t3code-abc123.scope".to_string(),
            },
            title: "t3code-abc123.scope".to_string(),
            pid: 200,
            start_ticks: 2000,
            pgrp: 200,
            can_stop: false,
            processes: vec![fixture_proc(200, "node")],
            cpu_percent: 2.5,
            rss_bytes: 90_000_000,
            read_bytes: None,
            write_bytes: None,
            scope_mem_bytes: None,
        },
    ]
}

#[test]
fn snapshots_processes_en_ru_and_stop_dialog() {
    let _env = lock_env();
    let (paths, _dir, _guard) = fixture_paths("procs");
    std::fs::create_dir_all(SNAPSHOTS).unwrap();

    for (lang, stem) in [("en", "processes_en"), ("ru", "processes_ru")] {
        agentdeck_gui::i18n::set_language(lang);
        let data = data::Data {
            processes: fixture_groups(),
            ..data::Data::default()
        };
        let mut state = app::State::for_tests(paths.clone(), data);
        let _ = app::update(
            &mut state,
            app::Message::SelectPage(agentdeck_gui::Page::Processes),
        );
        let mut ui = iced_test::simulator(app::view(&state));
        let snapshot = ui.snapshot(&iced::Theme::Dark).expect("headless snapshot");
        assert!(
            snapshot
                .matches_image(format!("{SNAPSHOTS}/{stem}.png"))
                .expect("snapshot compare/write"),
            "snapshot {stem} changed"
        );
        let goldens: Vec<_> = std::fs::read_dir(SNAPSHOTS)
            .expect("snapshots dir")
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| {
                p.file_stem()
                    .and_then(|s| s.to_str())
                    .is_some_and(|s| s.starts_with(stem))
                    && p.extension().and_then(|e| e.to_str()) == Some("png")
            })
            .collect();
        assert_eq!(goldens.len(), 1, "one golden per stem: {goldens:?}");
        let bytes = std::fs::metadata(&goldens[0]).expect("golden png").len();
        assert!(bytes > 1_024, "golden {} looks empty", goldens[0].display());
        assert!(
            bytes < 100 * 1024,
            "golden {} over 100 KB",
            goldens[0].display()
        );
    }

    // Stop opens a confirmation listing every process; Cancel closes it.
    // Fixture pids are fake, so the fresh member lookup falls back to the
    // snapshot rows and nothing is ever signaled.
    agentdeck_gui::i18n::set_language("en");
    let data = data::Data {
        processes: fixture_groups(),
        ..data::Data::default()
    };
    let mut state = app::State::for_tests(paths, data);
    let _ = app::update(
        &mut state,
        app::Message::SelectPage(agentdeck_gui::Page::Processes),
    );
    let mut ui = iced_test::simulator(app::view(&state));
    ui.click("Stop").expect("stop button");
    for message in ui.into_messages() {
        let _ = app::update(&mut state, message);
    }
    assert!(state.proc_ui().pending.is_some());
    let _ = app::update(&mut state, app::Message::ProcForce(true));
    assert!(state.proc_ui().force);
    let mut ui = iced_test::simulator(app::view(&state));
    ui.find("Stop now").expect("confirm button");
    ui.find("100  sleep 60").expect("listed member pid");
    ui.click("Cancel").expect("cancel button");
    for message in ui.into_messages() {
        let _ = app::update(&mut state, message);
    }
    assert!(state.proc_ui().pending.is_none());
}

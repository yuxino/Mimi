//! App command behavior and capability regressions.

use super::*;

#[test]
fn frontend_readiness_markers_are_test_only_and_window_scoped() {
    let directory = std::env::temp_dir().join(format!("mimi-ui-ready-{}", uuid::Uuid::new_v4()));
    let destination = Some(directory.as_os_str());
    write_ui_test_frontend_ready(false, "settings", destination).unwrap();
    write_ui_test_frontend_ready(true, "../outside", destination).unwrap();
    assert!(!directory.exists());

    write_ui_test_frontend_ready(true, "settings", destination).unwrap();
    assert_eq!(
        std::fs::read(directory.join("settings")).unwrap(),
        b"ready\n"
    );
    assert!(!directory.join("overlay").exists());
    write_ui_test_frontend_ready(true, "overlay", destination).unwrap();
    assert_eq!(
        std::fs::read(directory.join("overlay")).unwrap(),
        b"ready\n"
    );
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn settings_navigation_target_accepts_only_known_sections() {
    assert_eq!(
        serde_json::from_str::<SettingsNavigationTarget>(r#""voice""#).unwrap(),
        SettingsNavigationTarget::Voice
    );
    assert_eq!(
        serde_json::from_str::<SettingsNavigationTarget>(r#""service""#).unwrap(),
        SettingsNavigationTarget::Service
    );
    assert_eq!(
        serde_json::from_str::<SettingsNavigationTarget>(r#""export""#).unwrap(),
        SettingsNavigationTarget::Export
    );
    assert_eq!(
        serde_json::from_str::<SettingsNavigationTarget>(r#""appleSpeechResources""#).unwrap(),
        SettingsNavigationTarget::AppleSpeechResources
    );
    assert_eq!(
        serde_json::from_str::<SettingsNavigationTarget>(r#""activeProfile""#).unwrap(),
        SettingsNavigationTarget::ActiveProfile
    );
    assert!(serde_json::from_str::<SettingsNavigationTarget>(r#""general""#).is_err());
}

#[test]
fn normal_quit_exits_only_after_successful_finalization_and_sanitizes_failures() {
    let exited = std::cell::Cell::new(false);
    let failure = std::io::Error::other("synthetic private failure detail");
    assert_eq!(
        finish_quit(Err(failure), || exited.set(true)),
        Err("Could not save session history before quitting.".to_string())
    );
    assert!(!exited.get());
    assert_eq!(finish_quit(Ok(()), || exited.set(true)), Ok(()));
    assert!(exited.get());
}

#[test]
fn simultaneous_quit_requests_share_one_save_and_failed_save_can_retry() {
    let gate = QuitRequestGate(AtomicBool::new(false));
    let exited = std::cell::Cell::new(false);
    let failed = (|| {
        let request = gate.begin().expect("first request accepted");
        assert!(gate.begin().is_none());
        finish_quit(Err(std::io::Error::other("synthetic failure")), || {
            exited.set(true)
        })?;
        request.exit_requested();
        Ok::<(), String>(())
    })();
    assert!(failed.is_err());
    assert!(!exited.get());
    let retry = gate.begin().expect("save failure permits a retry");
    finish_quit(Ok(()), || exited.set(true)).unwrap();
    retry.exit_requested();
    assert!(exited.get());
    assert!(
        gate.begin().is_none(),
        "exit already queued; do not save again"
    );
}

#[tokio::test]
async fn cancelled_quit_future_releases_the_request_gate() {
    let gate = Arc::new(QuitRequestGate(AtomicBool::new(false)));
    let task_gate = Arc::clone(&gate);
    let (started, received) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move {
        let _request = task_gate.begin().unwrap();
        started.send(()).unwrap();
        std::future::pending::<()>().await;
    });
    received.await.unwrap();
    assert!(gate.begin().is_none());
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert!(gate.begin().is_some());
}

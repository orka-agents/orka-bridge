use bridge_engine::send_summary::{
    AttemptSummary, AttemptVerdict, SummaryError, SummaryStore, TargetUids,
};
use bridge_runtime_protocol::{SnapshotId, Uuid};

fn summary(id: Uuid, verdict: AttemptVerdict) -> AttemptSummary {
    AttemptSummary {
        id: id.to_string(),
        source: SnapshotId::of(b"agent", b"tests"),
        target_display: "team-dev · agents".into(),
        target_uids: TargetUids {
            service_namespace: "control".into(),
            service_uid: "service-1".into(),
            deployment_namespace: "control".into(),
            deployment_uid: "deployment-1".into(),
            watched_namespace: "agents".into(),
        },
        provider: "team-model".into(),
        planned_agent_name: "bridge-run-before-post".into(),
        agent_uid: None,
        verdict,
        time: 1_762_876_800,
    }
}

#[test]
fn planned_agent_saved_before_post() {
    let dir = tempfile::tempdir().unwrap();
    let store = SummaryStore::new(Some(dir.path().into()));
    let attempt = summary(Uuid::new_v4(), AttemptVerdict::Running);
    store.begin(attempt.clone()).unwrap();
    let history = store.list().unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].planned_agent_name, attempt.planned_agent_name);
    assert_eq!(history[0].verdict, AttemptVerdict::Running);
    assert_eq!(history[0].agent_uid, None);
}

#[test]
fn completed_stays_completed() {
    let dir = tempfile::tempdir().unwrap();
    let store = SummaryStore::new(Some(dir.path().into()));
    let id = Uuid::new_v4();
    store.begin(summary(id, AttemptVerdict::Running)).unwrap();
    let mut done = summary(id, AttemptVerdict::Passed);
    done.agent_uid = Some("uid-agent".into());
    store.finish(done.clone()).unwrap();
    let reopened = SummaryStore::new(Some(dir.path().into()));
    assert_eq!(reopened.list().unwrap(), vec![done]);
}

#[test]
fn no_task_handle_or_source_or_token() {
    let dir = tempfile::tempdir().unwrap();
    let store = SummaryStore::new(Some(dir.path().into()));
    let mut attempt = summary(Uuid::new_v4(), AttemptVerdict::Unknown);
    attempt.provider = "provider-display".into();
    attempt.agent_uid = Some("uid-agent".into());
    store.begin(attempt).unwrap();
    let bytes = std::fs::read(dir.path().join("send-history-v1/history.json")).unwrap();
    let text = String::from_utf8_lossy(&bytes);
    assert!(!text.contains("bridge-task"));
    assert!(!text.contains("token"));
    assert!(!text.contains("Bearer"));
    assert!(!text.contains("systemPrompt"));
}

#[test]
fn missing_native_root_is_visible_not_silently_disabled() {
    let store = SummaryStore::new(None);
    assert_eq!(store.list(), Err(SummaryError::Unavailable));
    assert!(store
        .begin(summary(Uuid::new_v4(), AttemptVerdict::Running))
        .is_err());
}

#[test]
fn reading_absent_history_does_not_create_files() {
    let dir = tempfile::tempdir().unwrap();
    let store = SummaryStore::new(Some(dir.path().join("not-created")));
    assert!(store.list().unwrap().is_empty());
    assert!(!dir.path().join("not-created").exists());
}

#[cfg(unix)]
#[test]
fn summary_directory_is_private_and_symlinks_are_not_followed() {
    use std::os::unix::fs::{symlink, PermissionsExt};
    let dir = tempfile::tempdir().unwrap();
    let store = SummaryStore::new(Some(dir.path().into()));
    store
        .begin(summary(Uuid::new_v4(), AttemptVerdict::Running))
        .unwrap();
    let root = dir.path().join("send-history-v1");
    assert_eq!(
        std::fs::metadata(&root).unwrap().permissions().mode() & 0o077,
        0
    );
    let outside = tempfile::tempdir().unwrap();
    std::fs::remove_file(root.join("history.json")).unwrap();
    std::fs::remove_dir(&root).unwrap();
    symlink(outside.path(), &root).unwrap();
    assert!(store
        .begin(summary(Uuid::new_v4(), AttemptVerdict::Running))
        .is_err());
    assert_eq!(std::fs::read_dir(outside.path()).unwrap().count(), 0);
}

#[test]
fn byte_bound_keeps_recent_sends_without_permanently_blocking_new_attempts() {
    let dir = tempfile::tempdir().unwrap();
    let store = SummaryStore::new(Some(dir.path().into()));
    let mut latest = String::new();
    for _ in 0..260 {
        let mut attempt = summary(Uuid::new_v4(), AttemptVerdict::Unknown);
        attempt.target_display = "c".repeat(700);
        latest = attempt.id.clone();
        store.begin(attempt).unwrap();
    }
    let small = summary(Uuid::new_v4(), AttemptVerdict::Running);
    store.begin(small.clone()).unwrap();
    let saved = store.list().unwrap();
    assert!(saved.len() < 256);
    assert!(saved.iter().any(|a| a.id == latest));
    assert_eq!(saved.last().unwrap().id, small.id);
    assert!(
        std::fs::metadata(dir.path().join("send-history-v1/history.json"))
            .unwrap()
            .len()
            <= 256 * 1024
    );
}

#[test]
fn corrupt_summary_not_pass() {
    let dir = tempfile::tempdir().unwrap();
    let store = SummaryStore::new(Some(dir.path().into()));
    store
        .begin(summary(Uuid::new_v4(), AttemptVerdict::Running))
        .unwrap();
    let root = dir.path().join("send-history-v1");
    std::fs::write(
        root.join("history.json"),
        b"{\"version\":1,\"attempts\":[{\"id\":\"bad\"}]}",
    )
    .unwrap();
    assert_eq!(store.list(), Err(SummaryError::Corrupt));
}

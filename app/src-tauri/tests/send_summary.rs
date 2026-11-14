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

fn details() -> bridge_engine::send_summary::AttemptDetails {
    serde_json::from_value(serde_json::json!({
        "documentId":Uuid::new_v4().to_string(), "model":"model", "capturedAt":1,
        "cases":[{"id":"one","input":"question","assertions":[{"id":"a","type":"contains","value":"answer"}]}],
        "resources":[], "report":null, "detailState":"saved", "availability":null
    })).unwrap()
}

#[test]
fn details_restart_remove_and_legacy_keep_exact_summary_shape() {
    let dir = tempfile::tempdir().unwrap();
    let store = SummaryStore::new(Some(dir.path().into()));
    let old = summary(Uuid::new_v4(), AttemptVerdict::Unknown);
    store.begin(old.clone()).unwrap();
    assert_eq!(store.details(&old.id).unwrap().detail_state, "legacy");
    let attempt = summary(Uuid::new_v4(), AttemptVerdict::Passed);
    store.checkpoint(attempt.clone(), details()).unwrap();
    let reopened = SummaryStore::new(Some(dir.path().into()));
    assert_eq!(
        reopened.details(&attempt.id).unwrap().model.as_deref(),
        Some("model")
    );
    assert_eq!(reopened.list().unwrap(), vec![old, attempt.clone()]);
    let removed = reopened.remove_details(&attempt.id).unwrap();
    assert_eq!(removed.detail_state, "removed");
    assert!(removed.model.is_none());
    assert_eq!(reopened.list().unwrap().last().unwrap(), &attempt);
}

#[test]
fn overflow_retains_control_and_saturation_never_evicts_unresolved_receipts() {
    let dir = tempfile::tempdir().unwrap();
    let store = SummaryStore::new(Some(dir.path().into()));
    let mut evidence = details();
    evidence.resources.push(
        serde_json::from_value(serde_json::json!({
            "kind":"Task","name":"bridge-task-pending","namespace":"agents", "uid":null,
            "generation":0,"specDigest":"a".repeat(64),"terminal":false
        }))
        .unwrap(),
    );
    let mut first = String::new();
    for i in 0..256 {
        let attempt = summary(Uuid::new_v4(), AttemptVerdict::Unknown);
        if i == 0 {
            first = attempt.id.clone();
        }
        store.checkpoint(attempt, evidence.clone()).unwrap();
    }
    assert!(store
        .checkpoint(summary(Uuid::new_v4(), AttemptVerdict::Running), details())
        .is_err());
    assert!(store.remove_details(&first).is_err());
    assert_eq!(store.list().unwrap().len(), 256);
    evidence.report = Some(
        serde_json::from_value(serde_json::json!({"cases":[{
        "id":"one","question":"question","phase":"Succeeded","answer":"x".repeat(4*1024*1024),
        "completedTools":[],"assertions":[],"verdict":"unknown"
    }],"verdict":"unknown"}))
        .unwrap(),
    );
    let attempt = store.list().unwrap()[0].clone();
    store.checkpoint(attempt, evidence).unwrap();
    let view = store.details(&first).unwrap();
    assert_eq!(view.detail_state, "omitted");
    assert!(view.interrupted);
    let report = view.report.unwrap();
    assert_eq!(report.cases[0].phase, "Succeeded");
    assert!(report.cases[0].answer.is_none());
    assert_eq!(report.verdict, bridge_engine::engine::run::Verdict::Unknown);
    assert!(
        std::fs::metadata(dir.path().join(format!("send-history-v1/{first}.json")))
            .unwrap()
            .len()
            <= 4 * 1024 * 1024
    );
}

#[test]
fn interrupted_detail_publication_never_exposes_a_saved_pass_after_next_send() {
    let dir = tempfile::tempdir().unwrap();
    let store = SummaryStore::new(Some(dir.path().into()));
    let attempt = summary(Uuid::new_v4(), AttemptVerdict::Passed);
    let mut detail = details();
    detail.report =
        Some(serde_json::from_value(serde_json::json!({"cases":[], "verdict":"passed"})).unwrap());
    store.checkpoint(attempt.clone(), detail).unwrap();
    std::fs::write(dir.path().join("send-history-v1/saving"), &attempt.id).unwrap();
    assert!(store.details(&attempt.id).unwrap().report.is_none());
    store
        .begin(summary(Uuid::new_v4(), AttemptVerdict::Running))
        .unwrap();
    let quarantined = store.details(&attempt.id).unwrap();
    assert!(quarantined.report.is_none());
    assert_eq!(quarantined.detail_state, "omitted");
    assert_eq!(store.list().unwrap()[0].verdict, AttemptVerdict::Unknown);
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

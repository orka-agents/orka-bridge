//! B3 exercises the actual native port-forward, conditional HTTP and durable owner.
use super::*;

fn edit(s: &Setup) {
    std::fs::write(
        s.agent_path(),
        AGENT.replace("Fetch only.", "Changed instructions."),
    )
    .unwrap();
}
pub(super) async fn separate(s: &Setup) -> Value {
    let folder = Arc::new(AgentFolder::new(Some(s.home.path().into())));
    serde_json::to_value(
        s.owner
            .start_document_separate(folder, &s.target, &s.provider, s.source(), true)
            .await
            .view
            .unwrap(),
    )
    .unwrap()
}
fn puts(s: &Setup) -> Vec<(String, Value)> {
    s.api
        .wire
        .calls
        .lock()
        .unwrap()
        .iter()
        .filter(|c| c.0.starts_with("PUT"))
        .cloned()
        .collect()
}
#[tokio::test]
async fn repeated_send_reuses_uid_updates_conditionally_and_preserves_archived_answers() {
    let s = setup().await;
    assert_eq!(
        reply(s.owner.preflight(&s.target, &s.provider).await)["action"],
        "create"
    );
    let first = s.finish(&id(&s.start().await)).await;
    assert_eq!(first["agentAction"], "create");
    let uid = first["acknowledged"][0]["uid"].clone();
    edit(&s);
    let check = reply(s.owner.preflight(&s.target, &s.provider).await);
    assert_eq!(check["action"], "update");
    assert_eq!(check["canSeparate"], true);
    let updated = s.finish(&id(&s.start().await)).await;
    assert_eq!(updated["state"], "complete", "{updated}");
    assert_eq!(updated["agentAction"], "update");
    assert_eq!(updated["acknowledged"][0]["uid"], uid);
    let updates = puts(&s);
    assert_eq!(updates.len(), 1);
    assert!(updates[0].0.contains("/spec?namespace=agents"));
    assert_eq!(updates[0].1["expectedUID"], uid);
    assert_eq!(updates[0].1["expectedGeneration"], 1);
    assert_eq!(
        updates[0].1["spec"]["systemPrompt"]["inline"],
        "Changed instructions.\n"
    );
    let old = s.owner.history_check(&id(&first)).await.unwrap();
    assert_eq!(old.report.unwrap().verdict, Verdict::Passed);
    let folder = Arc::new(AgentFolder::new(Some(s.home.path().into())));
    let fresh = SendOwner::with_history_root(
        folder,
        s.session.clone(),
        Some(s.home.path().join("app-local")),
    );
    s.owner.history_remove(&id(&updated)).await.unwrap();
    assert_eq!(
        reply(fresh.preflight(&s.target, &s.provider).await)["action"],
        "unchanged"
    );
    let next = reply(fresh.start(&s.target, &s.provider, s.source()).await);
    until(|| fresh.status(Some(&id(&next))).unwrap().state != SendState::Running).await;
    let next = reply(fresh.status(Some(&id(&next))).unwrap());
    assert_eq!(next["agentAction"], "unchanged");
    assert_eq!(next["acknowledged"][0]["uid"], uid);
    assert_ne!(
        next["acknowledged"][1]["uid"],
        first["acknowledged"][1]["uid"]
    );
    assert_eq!(s.agent_posts().len(), 1);
    assert_eq!(puts(&s).len(), 1);
}
#[tokio::test]
async fn definite_create_rejections_do_not_leave_uncertain_bindings_or_tasks() {
    for mode in [
        "reject-agent400",
        "reject-agent403",
        "reject-agent409",
        "reject-task403",
    ] {
        let s = setup().await;
        let reused = mode.starts_with("reject-task");
        if reused {
            s.finish(&id(&s.start().await)).await;
        }
        *s.api.wire.mode.lock().unwrap() = mode;
        let rejected = s.finish(&id(&s.start().await)).await;
        assert_eq!(rejected["state"], "unknown");
        assert_eq!(rejected["error"], "submitRejected");
        *s.api.wire.mode.lock().unwrap() = "";
        let check = reply(s.owner.preflight(&s.target, &s.provider).await);
        assert_eq!(
            check["action"],
            if reused { "unchanged" } else { "create" },
            "{mode}: {check}"
        );
        assert_eq!(check["canSend"], true);
        assert!(!s.owner.history_details(&id(&rejected)).unwrap().interrupted);
        let retry = s.finish(&id(&s.start().await)).await;
        assert_eq!(retry["state"], "complete");
    }
}
#[tokio::test]
async fn conditional_rejections_never_create_tasks_or_fallback_agents() {
    for (mode, stage) in [
        ("update409", "updateConflict"),
        ("update404", "updateUnsupported"),
        ("update405", "updateUnsupported"),
        ("update400", "updateInvalid"),
        ("update403", "updateDenied"),
        ("update-missing", "updateMissing"),
    ] {
        let s = setup().await;
        s.finish(&id(&s.start().await)).await;
        edit(&s);
        let before = s.api.wire.posts();
        *s.api.wire.mode.lock().unwrap() = mode;
        let failed = s.finish(&id(&s.start().await)).await;
        assert_eq!(failed["error"], stage, "{mode}: {failed}");
        assert_eq!(s.api.wire.posts(), before);
        assert_eq!(s.agent_posts().len(), 1);
        assert_eq!(puts(&s).len(), 1);
        let history = s.owner.history_details(&id(&failed)).unwrap();
        assert!(!history.interrupted);
        *s.api.wire.mode.lock().unwrap() = "";
        let separated = s.finish(&id(&separate(&s).await)).await;
        assert_eq!(separated["state"], "complete");
        assert_eq!(s.agent_posts().len(), 2);
    }
}
#[tokio::test]
async fn outside_edits_and_replacements_block_before_put() {
    for mode in ["agent-generation", "agent-replaced"] {
        let s = setup().await;
        s.finish(&id(&s.start().await)).await;
        edit(&s);
        *s.api.wire.mode.lock().unwrap() = mode;
        let failed = s.finish(&id(&s.start().await)).await;
        assert_eq!(failed["error"], "updateConflict");
        assert!(puts(&s).is_empty());
        assert_eq!(s.api.wire.posts(), 2);
    }
}
#[tokio::test]
async fn lost_agent_responses_reconcile_read_only_but_old_or_missing_never_authorize_replay() {
    for mode in ["lose-agent", "update-lost", "update-lost-old", "broken"] {
        let s = setup().await;
        if mode.starts_with("update") {
            s.finish(&id(&s.start().await)).await;
            edit(&s);
        }
        *s.api.wire.mode.lock().unwrap() = mode;
        let failed = s.finish(&id(&s.start().await)).await;
        assert_eq!(failed["state"], "unknown");
        let writes = (s.api.wire.posts(), puts(&s).len());
        *s.api.wire.mode.lock().unwrap() = "";
        let check = reply(s.owner.preflight(&s.target, &s.provider).await);
        assert_eq!((s.api.wire.posts(), puts(&s).len()), writes);
        if mode == "lose-agent" || mode == "update-lost" {
            assert_eq!(check["action"], "unchanged", "{mode}: {check}");
            let next = s.finish(&id(&s.start().await)).await;
            assert_eq!(next["state"], "complete");
            assert_eq!(s.agent_posts().len(), 1);
            assert_eq!(puts(&s).len(), writes.1);
        } else {
            assert_eq!(check["action"], "blocked");
            assert_eq!(check["canSeparate"], true);
            let retry = s.finish(&id(&s.start().await)).await;
            assert_eq!(retry["error"], "updateUnconfirmed");
            assert_eq!((s.api.wire.posts(), puts(&s).len()), writes);
            let old = s.owner.history_details(&id(&failed)).unwrap();
            assert!(old.interrupted);
            let separated = s.finish(&id(&separate(&s).await)).await;
            assert_eq!(separated["state"], "complete");
            assert!(s.owner.history_details(&id(&failed)).unwrap().interrupted);
        }
    }
}
#[tokio::test]
async fn named_patch_review_is_bearer_only_and_never_gates_unchanged_reruns() {
    let s = setup().await;
    let first = s.finish(&id(&s.start().await)).await;
    *s.api.wire.mode.lock().unwrap() = "token-review";
    std::fs::write(
        s.agent_path().with_file_name("web-reader.agent.test.md"),
        TESTS.replace("expected answer", "changed expectation"),
    )
    .unwrap();
    s.api.calls.lock().unwrap().clear();
    let again = s.finish(&id(&s.start().await)).await;
    assert_eq!(again["state"], "complete");
    assert!(!s
        .api
        .calls
        .lock()
        .unwrap()
        .iter()
        .any(|c| c.1.ends_with("/selfsubjectaccessreviews")));
    edit(&s);
    *s.api.deny.lock().unwrap() = true;
    let denied = s.finish(&id(&s.start().await)).await;
    assert_eq!(denied["error"], "updateDenied");
    assert!(puts(&s).is_empty());
    let calls = s.api.calls.lock().unwrap();
    let review = calls
        .iter()
        .find(|c| c.1.ends_with("/selfsubjectaccessreviews"))
        .unwrap();
    assert_eq!(review.2, "Bearer creator-fixture");
    assert_eq!(
        review.3["spec"]["resourceAttributes"],
        json!({"group":"core.orka.ai","resource":"agents","verb":"patch","name":first["agentName"],"namespace":"agents"})
    );
}
#[tokio::test]
async fn reused_agent_lost_task_response_recovers_after_restart_without_posting_again() {
    let s = setup().await;
    let first = s.finish(&id(&s.start().await)).await;
    s.api.wire.lose_task_reply.store(1, Ordering::SeqCst);
    let lost = s.finish(&id(&s.start().await)).await;
    assert_eq!(lost["state"], "unknown");
    assert_eq!(
        lost["acknowledged"][0]["uid"],
        first["acknowledged"][0]["uid"]
    );
    s.api.wire.lose_task_reply.store(0, Ordering::SeqCst);
    let fresh = SendOwner::with_history_root(
        Arc::new(AgentFolder::new(Some(s.home.path().into()))),
        s.session.clone(),
        Some(s.home.path().join("app-local")),
    );
    let before = s.api.wire.posts();
    let checked = fresh.history_check(&id(&lost)).await.unwrap();
    assert!(!checked.interrupted);
    assert_eq!(checked.report.unwrap().verdict, Verdict::Passed);
    assert_eq!(s.api.wire.posts(), before);
    assert_eq!(s.agent_posts().len(), 1);
    assert_eq!(
        reply(fresh.preflight(&s.target, &s.provider).await)["action"],
        "unchanged"
    );
}
#[tokio::test]
async fn valid_update_receipt_can_keep_the_generation_after_server_normalization() {
    let s = setup().await;
    let first = s.finish(&id(&s.start().await)).await;
    edit(&s);
    *s.api.wire.mode.lock().unwrap() = "update-noop";
    let updated = s.finish(&id(&s.start().await)).await;
    assert_eq!(updated["state"], "complete", "{updated}");
    assert_eq!(
        updated["acknowledged"][0]["uid"],
        first["acknowledged"][0]["uid"]
    );
    assert_eq!(puts(&s).len(), 1);
}
#[tokio::test]
async fn unfinished_bound_tasks_block_normal_send_but_separate_preserves_control_receipts() {
    let s = setup().await;
    *s.api.wire.mode.lock().unwrap() = "running";
    let first = s.start().await;
    until(|| s.view(None)["stage"] == "observe").await;
    s.owner.stop(&id(&first)).await;
    let old = s.owner.history_details(&id(&first)).unwrap();
    assert!(old.interrupted);
    *s.api.wire.mode.lock().unwrap() = "";
    edit(&s);
    let blocked = s.finish(&id(&s.start().await)).await;
    assert_eq!(blocked["error"], "earlierWork");
    assert!(puts(&s).is_empty());
    let separate = s.finish(&id(&separate(&s).await)).await;
    assert_eq!(separate["state"], "complete");
    assert!(s.owner.history_details(&id(&first)).unwrap().can_cancel);
    let same = s.finish(&id(&s.start().await)).await;
    assert_eq!(same["state"], "complete");
    assert_eq!(s.agent_posts().len(), 2);
}

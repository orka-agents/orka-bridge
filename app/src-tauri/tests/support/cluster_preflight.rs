//! Reuses the real source owner, discovery client and HTTP-over-pod-forward fixtures.
use super::*;

fn catalog(s: &Setup, pages: Vec<(u16, Value)>) {
    *s.api.wire.catalog.lock().unwrap() = Some(pages);
}
fn page(items: Value, next: bool) -> (u16, Value) {
    (
        200,
        json!({"items":items,"metadata":if next {json!({"continue":"next+/ =?"})} else {json!({})}}),
    )
}
async fn check(s: &Setup) -> Value {
    serde_json::to_value(s.owner.preflight(&s.target, &s.provider).await).unwrap()
}
fn row<'a>(check: &'a Value, label: &str) -> &'a Value {
    check["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["label"] == label)
        .unwrap()
}

#[tokio::test]
async fn preflight_zero_agent_task_posts_and_literal_model() {
    let s = setup().await;
    let c = check(&s).await;
    assert_eq!(c["canSend"], true, "{c}");
    assert_eq!(c["model"], "model:exact");
    assert_eq!(
        c["source"],
        serde_json::to_value(SnapshotId::of(AGENT.as_bytes(), TESTS.as_bytes())).unwrap()
    );
    assert_eq!(c["targetId"], s.target);
    assert_eq!(c["providerId"], s.provider);
    assert_eq!(row(&c, "Model")["status"], "unverified");
    assert_eq!(row(&c, "web_fetch")["status"], "allowed");
    assert!(s.owner.status(None).is_none());
    assert_eq!(s.api.wire.posts(), 0);
    // Not even TokenRequest or access-review POSTs are part of preflight.
    let before = s.api.calls.lock().unwrap().len();
    check(&s).await;
    assert!(s.api.calls.lock().unwrap()[before..]
        .iter()
        .all(|c| c.0 == "GET"));
    for secret in ["creator-fixture", "http://", "Bearer", "systemPrompt"] {
        assert!(!c.to_string().contains(secret));
    }
    s.idle_forwards().await;
}

#[tokio::test]
async fn resource_presence_requires_all_pages_and_same_mapping_on_start() {
    let s = setup_with(
        &AGENT
            .replace("kind: builtin", "kind: tool")
            .replace("name: web_fetch", "name: lookup"),
    )
    .await;
    catalog(
        &s,
        vec![
            page(json!([{"name":"web_fetch","builtin":true}]), true),
            page(
                json!([{"name":"lookup","namespace":"agents","builtin":false}]),
                false,
            ),
        ],
    );
    let c = check(&s).await;
    assert_eq!(c["canSend"], true, "{c}");
    assert_eq!(row(&c, "lookup")["status"], "allowed");
    assert_eq!(s.api.wire.posts(), 0);
    let done = s.finish(&id(&s.start().await)).await;
    assert_eq!(done["state"], "complete", "{done}");
    assert_eq!(
        s.agent_posts()[0]["spec"]["tools"],
        json!([{"name":"lookup"}])
    );
    assert_eq!(s.agent_posts()[0]["spec"]["model"]["name"], "model:exact");
}

#[tokio::test]
async fn missing_create_it_there_and_unreadable_cannot_check_block() {
    let s = setup_with(
        &AGENT
            .replace("kind: builtin", "kind: tool")
            .replace("name: web_fetch", "name: lookup"),
    )
    .await;
    let c = check(&s).await;
    assert_eq!(c["canSend"], false);
    assert_eq!(row(&c, "lookup")["status"], "blocked");
    assert!(row(&c, "lookup")["message"]
        .as_str()
        .unwrap()
        .contains("create it on selected"));
    catalog(
        &s,
        vec![
            page(json!([{"name":"web_fetch","builtin":true}]), true),
            (403, json!({"error":"creator-fixture"})),
        ],
    );
    let c = check(&s).await;
    assert_eq!(c["canSend"], false);
    assert_eq!(row(&c, "lookup")["status"], "cannotCheck");
    assert!(row(&c, "lookup")["message"]
        .as_str()
        .unwrap()
        .contains("cannot check lookup"));
    assert!(!c.to_string().contains("creator-fixture"));
    let done = s.finish(&id(&s.start().await)).await;
    assert_eq!(done["error"], "preflight");
    assert_eq!(s.api.wire.posts(), 0);
}

#[tokio::test]
async fn collision_unverified_not_refused_and_wrong_namespace_not_present() {
    let s = setup_with(&AGENT.replace("kind: builtin", "kind: tool")).await;
    catalog(
        &s,
        vec![
            page(json!([{"name":"web_fetch","builtin":true}]), true),
            page(
                json!([{"name":"web_fetch","namespace":"agents","builtin":false}]),
                false,
            ),
        ],
    );
    let c = check(&s).await;
    assert_eq!(c["canSend"], true, "{c}");
    assert_eq!(row(&c, "web_fetch")["status"], "unverified");
    let done = s.finish(&id(&s.start().await)).await;
    assert_eq!(done["state"], "complete");
    catalog(
        &s,
        vec![page(
            json!([{"name":"web_fetch","namespace":"other","builtin":false}]),
            false,
        )],
    );
    let c = check(&s).await;
    assert_eq!(c["canSend"], false);
    assert_eq!(row(&c, "web_fetch")["status"], "cannotCheck");
}

#[tokio::test]
async fn builtin_positive_evidence_does_not_read_unneeded_later_pages() {
    let s = setup().await;
    catalog(
        &s,
        vec![
            page(json!([{"name":"web_fetch","builtin":true}]), true),
            (403, json!({"error":"unrelated"})),
        ],
    );
    let c = check(&s).await;
    assert_eq!(c["canSend"], true, "{c}");
    assert!(s
        .api
        .wire
        .calls
        .lock()
        .unwrap()
        .iter()
        .all(|c| !c.0.contains("continue=")));
}

#[tokio::test]
async fn partial_catalog_cannot_prove_absence_or_skip_collision_scan() {
    let s = setup_with(&AGENT.replace("kind: builtin", "kind: tool")).await;
    for pages in [
        vec![
            page(
                json!([{"name":"web_fetch","namespace":"agents","builtin":false}]),
                true,
            ),
            (403, json!({})),
        ],
        vec![page(json!([]), true), page(json!([]), true)],
        vec![page(
            json!([{"name":"web_fetch","namespace":"agents"}]),
            false,
        )],
    ] {
        catalog(&s, pages);
        let c = check(&s).await;
        assert_eq!(c["canSend"], false, "{c}");
        assert_eq!(row(&c, "web_fetch")["status"], "cannotCheck");
    }
    assert_eq!(s.api.wire.posts(), 0);
}

#[tokio::test]
async fn provider_ready_not_model_catalogue_and_start_rechecks() {
    let s = setup().await;
    assert_eq!(check(&s).await["canSend"], true);
    *s.api.wire.mode.lock().unwrap() = "unready";
    let c = check(&s).await;
    assert_eq!(c["canSend"], false);
    assert_eq!(row(&c, "Provider")["status"], "blocked");
    let done = s.finish(&id(&s.start().await)).await;
    assert_eq!(done["error"], "preflight");
    *s.api.wire.mode.lock().unwrap() = "paged";
    let c = check(&s).await;
    assert_eq!(row(&c, "Provider")["status"], "blocked");
    assert_eq!(s.api.wire.posts(), 0);
}

#[tokio::test]
async fn edited_pair_and_replaced_binding_recheck_after_allowed_preview() {
    let s = setup().await;
    let initial = check(&s).await;
    assert_eq!(initial["canSend"], true);
    std::fs::write(
        s.agent_path(),
        AGENT
            .replace("kind: builtin", "kind: tool")
            .replace("name: web_fetch", "name: missing"),
    )
    .unwrap();
    let changed = check(&s).await;
    assert_ne!(initial["source"], changed["source"]);
    assert_eq!(changed["canSend"], false);
    assert_eq!(s.finish(&id(&s.start().await)).await["error"], "preflight");
    std::fs::write(s.agent_path(), AGENT).unwrap();
    *s.api.replacement.lock().unwrap() = true;
    let c = check(&s).await;
    assert_eq!(c["canSend"], false);
    assert_eq!(row(&c, "Installation")["status"], "cannotCheck");
    assert_eq!(s.finish(&id(&s.start().await)).await["error"], "preflight");
    assert_eq!(s.api.wire.posts(), 0);
}

#[tokio::test]
async fn required_send_schemas_are_rechecked_without_image_equality() {
    let s = setup().await; // Fixture image v99 deliberately differs from the local pin.
    assert_eq!(check(&s).await["canSend"], true);
    for resource in ["agents", "tasks", "providers", "tools"] {
        let suffix = format!("{resource}.core.orka.ai");
        let path = format!("/apis/apiextensions.k8s.io/v1/customresourcedefinitions/{suffix}");
        let mut crd = s.api.response(&path).1;
        crd["spec"]["versions"][0]["schema"] = json!({"openAPIV3Schema":{"type":"object"}});
        *s.api.override_response.lock().unwrap() = Some((suffix, 200, crd));
        let c = check(&s).await;
        assert_eq!(
            c["canSend"], false,
            "missing {resource} schema was accepted: {c}"
        );
        assert_eq!(row(&c, "Installation")["status"], "cannotCheck");
        assert_eq!(s.finish(&id(&s.start().await)).await["error"], "preflight");
        *s.api.override_response.lock().unwrap() = None;
    }
    assert_eq!(s.api.wire.posts(), 0);
}

#[tokio::test]
async fn capture_and_invalid_ids_are_finite_rows_not_raw_errors() {
    let s = setup().await;
    let c = serde_json::to_value(
        s.owner
            .preflight("https://secret.invalid", "Bearer bad")
            .await,
    )
    .unwrap();
    assert_eq!(c["canSend"], false);
    assert!(!c.to_string().contains("secret.invalid"));
    assert!(!c.to_string().contains("Bearer"));
    std::fs::write(s.agent_path(), "invalid").unwrap();
    let c = check(&s).await;
    assert_eq!(c["source"], Value::Null);
    assert_eq!(c["model"], Value::Null);
    assert_eq!(row(&c, "Source")["status"], "blocked");
    assert_eq!(s.api.wire.posts(), 0);
}

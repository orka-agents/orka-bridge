//! Native Send owner fixtures. A child of the forward fixture so it can count actual wire calls.
use super::*;
use bridge_engine::{
    agent_folder::AgentFolder,
    clusters::commands::ClusterSession,
    send::{SendOwner, SendState},
};
use bridge_runtime_protocol::SnapshotId;

const AGENT: &str = "---\nname: reader\ndescription: inert\nmodel: 'model:exact'\nkmx:\n  schemaVersion: 1\n  runtime: orka\n  tools:\n    - kind: builtin\n      name: web_fetch\n---\nFetch only.\n";
const TESTS: &str = "---\nschemaVersion: 1\n---\n```yaml agent-test\nid: fetch\ninput: go\nassertions:\n  - id: answer\n    type: contains\n    value: expected answer\n  - id: fetched\n    type: toolCalled\n    tool: web_fetch\n```\n";

type Hook = Arc<Mutex<Option<Box<dyn FnOnce() + Send>>>>;
struct Setup {
    api: Arc<ApiFixture>,
    session: ClusterSession,
    owner: Arc<SendOwner>,
    home: tempfile::TempDir,
    fresh: Arc<Mutex<Kubeconfig>>,
    on_read: Hook,
    target: String,
    provider: String,
}
impl Setup {
    fn agent_path(&self) -> std::path::PathBuf {
        self.home
            .path()
            .join("Orka/agents/web-reader/web-reader.agent.md")
    }
    fn view(&self, id: Option<&str>) -> Value {
        serde_json::to_value(self.owner.status(id)).unwrap()
    }
    async fn start(&self) -> Value {
        serde_json::to_value(self.owner.start(&self.target, &self.provider).await).unwrap()
    }
    async fn finish(&self, id: &str) -> Value {
        until(|| self.view(Some(id))["state"] != "running").await;
        self.view(Some(id))
    }
    async fn idle_forwards(&self) {
        until(|| self.api.wire.active.load(Ordering::SeqCst) == 0).await;
    }
    fn agent_posts(&self) -> Vec<Value> {
        self.api
            .wire
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|c| c.0.starts_with("POST /api/v1/agents"))
            .map(|c| c.1.clone())
            .collect()
    }
}
async fn until(done: impl Fn() -> bool) {
    tokio::time::timeout(Duration::from_secs(20), async {
        while !done() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("condition was not reached");
}
fn id(value: &Value) -> String {
    value["id"].as_str().unwrap().to_string()
}
async fn setup_with(agent: &str) -> Setup {
    let api = Arc::new(ApiFixture::default());
    let cfg = config(&api.start(), json!({"token":"creator-fixture"}));
    let fresh = Arc::new(Mutex::new(cfg.clone()));
    let on_read: Hook = Arc::default();
    let reader = {
        let (fresh, on_read) = (fresh.clone(), on_read.clone());
        Arc::new(move || {
            if let Some(hook) = on_read.lock().unwrap().take() {
                hook();
            }
            Ok(fresh.lock().unwrap().clone())
        })
    };
    let session =
        ClusterSession::with_reader(ClusterClient::new(cfg, Arc::new(PodOrkaRead)), reader);
    let contexts = serde_json::to_value(session.contexts().await.unwrap()).unwrap();
    let found = serde_json::to_value(session.discover(&id(&contexts[0])).await.unwrap()).unwrap();
    let target = id(&found[0]);
    let access = serde_json::to_value(session.use_target(&target, None).await).unwrap();
    assert_eq!(access["status"], "connected");
    let provider = id(&access["providers"][0]);
    let home = tempfile::tempdir().unwrap();
    let folder = Arc::new(AgentFolder::new(Some(home.path().into())));
    folder.save(agent.into()).unwrap();
    std::fs::write(
        home.path()
            .join("Orka/agents/web-reader/web-reader.agent.test.md"),
        TESTS,
    )
    .unwrap();
    Setup {
        owner: SendOwner::new(folder, session.clone()),
        session,
        api,
        home,
        fresh,
        on_read,
        target,
        provider,
    }
}
async fn setup() -> Setup {
    setup_with(AGENT).await
}

#[tokio::test]
async fn send_click_runs_captured_file() {
    let s = setup().await;
    assert!(s.owner.status(None).is_none());
    let started = s.start().await;
    assert_eq!(started["state"], "running");
    assert_eq!(started["targetId"], s.target);
    assert_eq!(started["targetName"], Value::Null);
    assert_eq!(started["providerId"], s.provider);
    let view = s.finish(&id(&started)).await;
    assert_eq!(view["state"], "complete", "{view}");
    assert_eq!(view["targetName"], "selected");
    assert_eq!(view["stage"], "done");
    assert_eq!(view["error"], Value::Null);
    assert_eq!(view["report"]["verdict"], "passed");
    assert_eq!(view["model"], "model:exact");
    assert_eq!(
        view["source"],
        serde_json::to_value(SnapshotId::of(AGENT.as_bytes(), TESTS.as_bytes())).unwrap()
    );
    let receipts = view["acknowledged"].as_array().unwrap();
    assert_eq!(receipts.len(), 2);
    assert_eq!(receipts[0]["kind"], "Agent");
    assert_eq!(receipts[0]["name"], view["agentName"]);
    assert_eq!(view["pending"], Value::Null);
    assert_eq!(s.api.wire.posts(), 2);
    assert_eq!(
        s.agent_posts()[0]["spec"]["systemPrompt"]["inline"],
        "Fetch only.\n"
    );
    assert_eq!(s.view(None), view);
    let text = view.to_string();
    for secret in ["creator-fixture", "http://", "Bearer"] {
        assert!(!text.contains(secret), "{secret}");
    }
    s.idle_forwards().await;
}

#[tokio::test]
async fn edit_after_capture_does_not_change_bytes() {
    let s = setup().await;
    let path = s.agent_path();
    // The fresh kubeconfig is read after capture, before any network wait or write.
    *s.on_read.lock().unwrap() = Some(Box::new(move || {
        std::fs::write(path, AGENT.replace("Fetch only.", "Edited later.")).unwrap();
    }));
    let view = s.finish(&id(&s.start().await)).await;
    assert_eq!(view["state"], "complete", "{view}");
    assert!(std::fs::read_to_string(s.agent_path())
        .unwrap()
        .contains("Edited later."));
    assert_eq!(
        s.agent_posts()[0]["spec"]["systemPrompt"]["inline"],
        "Fetch only.\n"
    );
    assert_eq!(
        view["source"],
        serde_json::to_value(SnapshotId::of(AGENT.as_bytes(), TESTS.as_bytes())).unwrap()
    );
}

#[tokio::test]
async fn duplicate_active_start_observes() {
    let s = setup().await;
    *s.api.wire.mode.lock().unwrap() = "hang";
    let first = s.start().await;
    until(|| s.api.wire.posts() == 1).await;
    let forwards = s.api.wire.forwards.load(Ordering::SeqCst);
    let second = s.start().await;
    assert_eq!(second["id"], first["id"]);
    assert_eq!(second["state"], "running");
    assert_eq!(second["stage"], "submit");
    assert_eq!(s.api.wire.posts(), 1);
    assert_eq!(s.api.wire.forwards.load(Ordering::SeqCst), forwards);
    // In-flight Stop keeps the possibly-created Agent name and awaits forward cleanup.
    let stopped = serde_json::to_value(s.owner.stop(&id(&first)).await).unwrap();
    assert_eq!(stopped["state"], "unknown");
    assert_eq!(stopped["stage"], "stopped");
    assert_eq!(stopped["error"], "stopped");
    assert_eq!(stopped["pending"]["kind"], "Agent");
    assert_eq!(stopped["pending"]["name"], stopped["agentName"]);
    // The client side has joined its drivers; the fixture thread notices the close shortly after.
    s.idle_forwards().await;
    assert_eq!(s.api.wire.posts(), 1);
}

#[tokio::test]
async fn stop_returns_receipts() {
    let s = setup().await;
    *s.api.wire.mode.lock().unwrap() = "running";
    let started = s.start().await;
    until(|| s.view(None)["stage"] == "observe").await;
    let stopped = serde_json::to_value(s.owner.stop(&id(&started)).await).unwrap();
    let calls = s.api.wire.calls.lock().unwrap().len();
    assert_eq!(stopped["state"], "unknown");
    assert_eq!(stopped["stage"], "stopped");
    assert_eq!(stopped["acknowledged"].as_array().unwrap().len(), 2);
    assert_eq!(stopped["pending"], Value::Null);
    assert_eq!(stopped["report"], Value::Null);
    // The client side has joined its drivers; the fixture thread notices the close shortly after.
    s.idle_forwards().await;
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(s.api.wire.calls.lock().unwrap().len(), calls);
    assert_eq!(s.api.wire.posts(), 2);
    // A finished attempt's Stop is observation only.
    assert_eq!(
        serde_json::to_value(s.owner.stop(&id(&started)).await).unwrap(),
        stopped
    );
}

#[tokio::test]
async fn stop_during_preflight_closes_forward_before_release() {
    let s = setup().await;
    s.idle_forwards().await;
    *s.api.wire.mode.lock().unwrap() = "hang-tools";
    let started = s.start().await;
    let tools = || {
        let calls = s.api.wire.calls.lock().unwrap();
        calls.iter().any(|c| c.0.starts_with("GET /api/v1/tools?"))
    };
    until(tools).await;
    let (owner, api, send_id) = (s.owner.clone(), s.api.clone(), id(&started));
    // Runs on the one test-runtime thread. Driver cleanup that was only
    // scheduled when admission was released is queued behind this task.
    let probe = tokio::spawn(async move {
        let mut stop = Box::pin(owner.stop(&send_id));
        std::future::poll_fn(|cx| {
            assert!(stop.as_mut().poll(cx).is_pending());
            std::task::Poll::Ready(())
        })
        .await;
        drop(stop);
        std::future::poll_fn(|cx| {
            if owner.status(Some(&send_id)).unwrap().state == SendState::Running {
                cx.waker().wake_by_ref();
                return std::task::Poll::Pending;
            }
            std::task::Poll::Ready(())
        })
        .await;
        // Hold the thread: only a forward already closed by the job can reach the server now.
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while api.wire.active.load(Ordering::SeqCst) != 0 && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        api.wire.active.load(Ordering::SeqCst)
    });
    assert_eq!(
        probe.await.unwrap(),
        0,
        "admission released before forward close"
    );
    let view = s.view(Some(&id(&started)));
    assert_eq!(view["stage"], "stopped");
    assert_eq!(s.api.wire.posts(), 0);
}

#[tokio::test]
async fn ipc_waiter_loss_keeps_native_owner() {
    let s = setup().await;
    *s.api.wire.mode.lock().unwrap() = "running";
    let started = s.start().await;
    until(|| s.view(None)["stage"] == "observe").await;
    let send_id = id(&started);
    let mut waiter = Box::pin(s.owner.stop(&send_id));
    std::future::poll_fn(|cx| {
        assert!(waiter.as_mut().poll(cx).is_pending());
        std::task::Poll::Ready(())
    })
    .await;
    drop(waiter);
    let view = s.finish(&id(&started)).await;
    assert_eq!(view["stage"], "stopped");
    assert_eq!(view["acknowledged"].as_array().unwrap().len(), 2);
    s.idle_forwards().await;
}

#[tokio::test]
async fn unknown_explicit_resend_fresh_agent() {
    let s = setup().await;
    *s.api.wire.mode.lock().unwrap() = "broken";
    let first = s.finish(&id(&s.start().await)).await;
    assert_eq!(first["state"], "unknown");
    assert_eq!(first["stage"], "submit");
    assert_eq!(first["error"], "submit");
    assert_eq!(first["pending"]["name"], first["agentName"]);
    assert!(first["acknowledged"].as_array().unwrap().is_empty());
    *s.api.wire.mode.lock().unwrap() = "";
    let second = s.finish(&id(&s.start().await)).await;
    assert_ne!(second["id"], first["id"]);
    assert_eq!(second["state"], "complete");
    assert_ne!(second["agentName"], first["agentName"]);
    let posts = s.agent_posts();
    assert_eq!(posts.len(), 2);
    assert_ne!(posts[0]["name"], posts[1]["name"]);
    // The old unknown attempt keeps its possibly-created identity; nothing is adopted or replayed.
    assert_eq!(s.view(Some(&id(&first))), first);
    assert_eq!(s.api.wire.posts(), 3);
}

#[tokio::test]
async fn replaced_installation_or_kubeconfig_blocks_before_post() {
    let s = setup().await;
    *s.api.replacement.lock().unwrap() = true;
    let view = s.finish(&id(&s.start().await)).await;
    assert_eq!(view["state"], "unknown");
    assert_eq!(view["error"], "preflight");
    *s.api.replacement.lock().unwrap() = false;
    let server = {
        let mut fresh = s.fresh.lock().unwrap();
        let cluster = fresh.clusters[0].cluster.as_mut().unwrap();
        let server = cluster.server.clone().unwrap();
        cluster.server = Some(format!("{server}/different-api-root"));
        server
    };
    let view = s.finish(&id(&s.start().await)).await;
    assert_eq!(view["error"], "preflight", "{view}");
    assert!(!view.to_string().contains(&server));
    assert_eq!(s.api.wire.posts(), 0);
}

#[tokio::test]
async fn provider_name_recheck_not_readiness_gate() {
    let s = setup().await;
    // Configured but not ready: readiness is display data, execution establishes availability.
    *s.api.wire.mode.lock().unwrap() = "unready";
    let view = s.finish(&id(&s.start().await)).await;
    assert_eq!(view["state"], "complete", "{view}");
    // The selected configured Provider was removed/renamed after Use.
    *s.api.wire.mode.lock().unwrap() = "paged";
    let view = s.finish(&id(&s.start().await)).await;
    assert_eq!(view["error"], "preflight");
    assert_eq!(s.api.wire.posts(), 2);
}

#[tokio::test]
async fn missing_builtin_blocks_before_post() {
    let s = setup_with(&AGENT.replace("name: web_fetch", "name: absent_tool")).await;
    let view = s.finish(&id(&s.start().await)).await;
    assert_eq!(view["state"], "unknown");
    assert_eq!(view["stage"], "preflight");
    assert_eq!(s.api.wire.posts(), 0);
}

#[tokio::test]
async fn unknown_ids_and_missing_access_never_write() {
    let s = setup().await;
    let view = serde_json::to_value(s.owner.start(&s.target, "not-a-provider").await).unwrap();
    let view = s.finish(&id(&view)).await;
    assert_eq!(view["error"], "access");
    assert_eq!(view["providerId"], "");
    let view =
        serde_json::to_value(s.owner.start("https://example.com", &s.provider).await).unwrap();
    let view = s.finish(&id(&view)).await;
    assert_eq!(view["error"], "access");
    assert_eq!(view["targetName"], Value::Null);
    assert!(s.owner.status(Some("unknown")).is_none());
    assert!(s.owner.stop("unknown").await.is_none());
    assert_eq!(s.api.wire.posts(), 0);
}

#[tokio::test]
async fn shutdown_stops_and_refuses_new_admission() {
    let s = setup().await;
    *s.api.wire.mode.lock().unwrap() = "running";
    let started = s.start().await;
    until(|| s.view(None)["stage"] == "observe").await;
    s.owner.shutdown().await;
    assert_eq!(s.view(Some(&id(&started)))["stage"], "stopped");
    // The client side has joined its drivers; the fixture thread notices the close shortly after.
    s.idle_forwards().await;
    let refused = s.start().await;
    assert_ne!(refused["state"], "running");
    assert_eq!(s.api.wire.posts(), 2);
}

#[tokio::test]
async fn send_start_future_is_send_for_desktop_dispatch() {
    let s = setup().await;
    let owner = s.owner.clone();
    let (target, provider) = (s.target.clone(), s.provider.clone());
    let view = tokio::spawn(async move { owner.start(&target, &provider).await })
        .await
        .unwrap();
    s.finish(&id(&serde_json::to_value(view).unwrap())).await;
}

#[tokio::test]
async fn admission_does_not_wait_for_discovery_lock() {
    let s = setup().await;
    let (release, gate) = std::sync::mpsc::channel();
    *s.api.gate.lock().unwrap() = Some(gate);
    let contexts = serde_json::to_value(s.session.contexts().await.unwrap()).unwrap();
    let services = || {
        let calls = s.api.calls.lock().unwrap();
        calls.iter().filter(|c| c.1.ends_with("/services")).count()
    };
    let before = services();
    let session = s.session.clone();
    let discovery = tokio::spawn(async move { session.discover(&id(&contexts[0])).await.is_ok() });
    until(|| services() > before).await;
    // Discovery now holds the session lock across a Kubernetes read.
    let quick = Duration::from_secs(1);
    let first = tokio::time::timeout(quick, s.start()).await.unwrap();
    assert_eq!(first["state"], "running");
    let second = tokio::time::timeout(quick, s.start()).await.unwrap();
    assert_eq!(second["id"], first["id"]);
    assert_eq!(s.view(None)["state"], "running");
    release.send(()).unwrap();
    assert!(discovery.await.unwrap());
    // Rediscovery cleared the earlier Use, so this attempt never reaches a write.
    let view = s.finish(&id(&first)).await;
    assert_eq!(view["error"], "access");
    assert_eq!(s.api.wire.posts(), 0);
}

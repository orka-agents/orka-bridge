use bridge_engine::engine::run::*;
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};
const AGENT: &str = "---\nname: reader\ndescription: inert\nmodel: 'model:exact'\nkmx:\n  schemaVersion: 1\n  runtime: orka\n  tools:\n    - kind: builtin\n      name: web_fetch\n---\n\nFetch only.\n";
const TESTS: &str = "---\nschemaVersion: 1\n---\n# Cases\n```yaml agent-test\nid: checksum\ninput: |\n  Fetch https://example.test/checksum\nassertions:\n  - id: answer\n    type: contains\n    value: secret-expected\n  - id: fetched\n    type: toolCalled\n    tool: web_fetch\n```\n";

#[test]
fn real_yaml_preserves_prompt_input_model_and_exact_byte_provenance() {
    // Catches trimming/rebuilding source, a handwritten YAML subset, or lost hashes.
    let pair = ParsedPair::parse(AGENT.as_bytes(), TESTS.as_bytes()).unwrap();
    assert_eq!(pair.prompt(), "\nFetch only.\n");
    assert_eq!(pair.model(), "model:exact");
    assert_eq!(pair.tools(), &["web_fetch"]);
    assert_eq!(
        pair.cases()[0].input(),
        "Fetch https://example.test/checksum\n"
    );
    assert_eq!(
        pair.snapshot(),
        &bridge_runtime_protocol::SnapshotId::of(AGENT.as_bytes(), TESTS.as_bytes())
    );
    let changed = ParsedPair::parse(format!("{AGENT}\n").as_bytes(), TESTS.as_bytes()).unwrap();
    assert_ne!(pair.snapshot(), changed.snapshot());
}
#[test]
fn closed_parser_refuses_unsupported_or_empty_semantics() {
    // Removing any closed-schema check would accept one of these invalid sources.
    for agent in [
        AGENT.replace("runtime: orka", "runtime: other"),
        AGENT.replace("kind: builtin", "kind: mcp"),
        AGENT.replace("  tools:", "  repeats: 2\n  tools:"),
        AGENT.replace("model: 'model:exact'", "model: ''"),
        AGENT.replace("name: reader", "name: reader\nunknown: true"),
    ] {
        assert!(ParsedPair::parse(agent.as_bytes(), TESTS.as_bytes()).is_err());
    }
    for tests in [TESTS.replace("type: contains", "type: judge"), TESTS.replace("    value: secret-expected", "    value: ''"), TESTS.replace("id: checksum", "id: checksum\nrepeat: 2"), TESTS.replace("assertions:", "mocks: {}\nassertions:"), TESTS.replace("schemaVersion: 1", "schemaVersion: 2"), "---\nschemaVersion: 1\n---\n".into(), "---\nschemaVersion: 1\n---\n```yaml agent-test\nid: empty\ninput: go\nassertions: []\n```\n".into(), TESTS.replace("```\n", "")] {
        assert!(ParsedPair::parse(AGENT.as_bytes(), tests.as_bytes()).is_err());
    }
}

#[derive(Clone, Copy)]
struct Behavior {
    phase: &'static str,
    answer: Option<&'static str>,
    events: &'static str,
    fault: &'static str,
}
impl Default for Behavior {
    fn default() -> Self {
        Self {
            phase: "Succeeded",
            answer: Some("secret-expected"),
            events: "complete",
            fault: "",
        }
    }
}
struct Fixture {
    url: String,
    requests: Arc<Mutex<Vec<(String, Value)>>>,
    worker: tokio::task::JoinHandle<()>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.worker.abort();
    }
}
impl Fixture {
    async fn start(b: Behavior) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured = requests.clone();
        let worker = tokio::spawn(async move {
            let mut objects: HashMap<String, Value> = HashMap::new();
            let mut event_reads = 0;
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut bytes = Vec::new();
                let header_end = loop {
                    let mut buf = [0; 4096];
                    let n = socket.read(&mut buf).await.unwrap();
                    if n == 0 {
                        break 0;
                    }
                    bytes.extend_from_slice(&buf[..n]);
                    if let Some(i) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                        break i + 4;
                    }
                };
                if header_end == 0 {
                    continue;
                }
                let headers = String::from_utf8_lossy(&bytes[..header_end]).into_owned();
                let length: usize = headers
                    .lines()
                    .find_map(|l| {
                        l.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .map(|v| v.trim().parse().unwrap())
                    })
                    .unwrap_or(0);
                while bytes.len() < header_end + length {
                    let mut buf = [0; 4096];
                    let n = socket.read(&mut buf).await.unwrap();
                    if n == 0 {
                        break;
                    }
                    bytes.extend_from_slice(&buf[..n]);
                }
                let mut first = headers.lines().next().unwrap().split_whitespace();
                let method = first.next().unwrap();
                let path = first.next().unwrap();
                let body: Value =
                    serde_json::from_slice(&bytes[header_end..]).unwrap_or(Value::Null);
                captured
                    .lock()
                    .unwrap()
                    .push((format!("{method} {path}"), body.clone()));
                let mut status = 200;
                let mut response;
                if method == "POST" {
                    let path = path.split('?').next().unwrap();
                    if b.fault == "lost-agent"
                        || (b.fault == "lost-task" && path.ends_with("tasks"))
                    {
                        continue;
                    }
                    status = 201;
                    response = json!({"apiVersion":"core.orka.ai/v1alpha1", "kind":if path.ends_with("agents") {"Agent"} else {"Task"}, "metadata":{"name":body["name"],"namespace":body["namespace"],"uid":format!("uid-{}",body["name"].as_str().unwrap()),"generation":1},"spec":body["spec"],"status":{"phase":b.phase}});
                    if path.ends_with("agents") {
                        response["spec"]["defaulted"] = json!(true);
                    } else {
                        // Real API canonicalizes Duration and omits zero MaxRetries.
                        response["spec"]["timeout"] = json!("2m0s");
                        response["spec"]["retryPolicy"]
                            .as_object_mut()
                            .unwrap()
                            .remove("maxRetries");
                        if b.fault == "retry-enabled" {
                            response["spec"]["retryPolicy"]["maxRetries"] = json!(1);
                        }
                    }
                    if b.fault == "wrong-submit" {
                        response["spec"]["model"]["name"] = json!("substitute");
                    }
                    objects.insert(
                        format!("{path}/{}", body["name"].as_str().unwrap()),
                        response.clone(),
                    );
                    // Orka's create handlers return metadata/spec without TypeMeta;
                    // later GETs return the complete typed Kubernetes object.
                    response.as_object_mut().unwrap().remove("apiVersion");
                    response.as_object_mut().unwrap().remove("kind");
                    if b.fault == "wrong-create-type" {
                        response["kind"] = json!("Provider");
                    }
                } else {
                    let route = path.split('?').next().unwrap();
                    if route.ends_with("/result") {
                        response = json!({"result":b.answer});
                        if b.answer.is_none() {
                            status = 404;
                        }
                    } else if route.ends_with("/events") {
                        event_reads += 1;
                        let task = route
                            .trim_end_matches("/events")
                            .rsplit('/')
                            .next()
                            .unwrap();
                        let agent = objects.values().find(|o| o["kind"] == "Agent").unwrap()
                            ["metadata"]["name"]
                            .as_str()
                            .unwrap();
                        let after: u64 = path
                            .split("after=")
                            .nth(1)
                            .unwrap()
                            .split('&')
                            .next()
                            .unwrap()
                            .parse()
                            .unwrap();
                        let seq = after + 1;
                        let typ = match b.events {
                            "started" => "ToolCallStarted",
                            "failed" => "ToolCallFailed",
                            "paged" if after == 0 => "ToolCallStarted",
                            _ => "ToolCallCompleted",
                        };
                        let mut event = json!({"namespace":"sandbox","streamType":"task","streamID":task,"seq":seq,"type":typ,"taskName":task,"agentName":agent,"toolName":"web_fetch","toolCallID":"call-1"});
                        match b.events {
                            "wrong-task" => event["taskName"] = json!("unrelated"),
                            "wrong-agent" => event["agentName"] = json!("unrelated"),
                            "wrong-event-stream" => event["streamID"] = json!("unrelated"),
                            "no-call-id" => event["toolCallID"] = json!(""),
                            "gap" => event["seq"] = json!(seq + 1),
                            "repeated-seq" => event["seq"] = json!(after),
                            _ => {}
                        }
                        let empty =
                            b.events == "missing" || (b.events == "late" && event_reads < 3);
                        response = json!({"namespace":"sandbox","streamType":"task","streamID":task,"afterSeq":after,"latestSeq":if empty {after} else if b.events == "paged" {2} else {seq},"events":if empty {vec![]} else {vec![event]}});
                        if b.events == "wrong-stream" {
                            response["streamID"] = json!("unrelated");
                        }
                        if b.events == "wrong-namespace" {
                            response["namespace"] = json!("elsewhere");
                        }
                        if b.events == "wrong-cursor" {
                            response["afterSeq"] = json!(99);
                        }
                        if b.events == "disabled" {
                            status = 501;
                        }
                    } else {
                        response = objects.get(route).unwrap().clone();
                        if b.fault == "task-type" && response["kind"] == "Task" {
                            response.as_object_mut().unwrap().remove("kind");
                        }
                        if b.fault == "task-before" && response["kind"] == "Task"
                            || b.fault == "task-after"
                                && event_reads > 0
                                && response["kind"] == "Task"
                        {
                            response["metadata"]["uid"] = json!("replacement");
                        }
                        if event_reads > 0 && response["kind"] == "Agent" {
                            match b.fault {
                                "agent-after" => response["metadata"]["uid"] = json!("replacement"),
                                "agent-generation" => response["metadata"]["generation"] = json!(2),
                                "agent-spec" => {
                                    response["spec"]["model"]["name"] = json!("replacement")
                                }
                                _ => {}
                            }
                        }
                    }
                }
                if b.fault == "slow" {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
                let payload = if b.fault == "oversize" {
                    "x".repeat(1024 * 1024 + 1)
                } else if b.fault == "malformed" {
                    "secret-server-body".into()
                } else {
                    response.to_string()
                };
                if b.fault == "redirect" {
                    status = 307;
                }
                let message = format!("HTTP/1.1 {status} fixture\r\nContent-Type: application/json\r\nLocation: /credential-leak\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}", payload.len());
                let _ = socket.write_all(message.as_bytes()).await;
            }
        });
        Self {
            url,
            requests,
            worker,
        }
    }
    fn runner(&self) -> NativeRunner<'static> {
        NativeRunner::new(
            NativeRunTarget::new(&self.url, "sandbox", "provider", Some("sensitive-token"))
                .unwrap(),
            RunLimits {
                request_timeout: Duration::from_millis(50),
                operation_timeout: Duration::from_secs(2),
                poll_interval: Duration::from_millis(1),
            },
        )
        .unwrap()
    }
}
fn pair() -> ParsedPair {
    ParsedPair::parse(AGENT.as_bytes(), TESTS.as_bytes()).unwrap()
}
#[tokio::test]
async fn native_runner_accepts_a_borrowed_transport_without_a_second_http_target() {
    use bridge_engine::engine::run::{RunFuture, RunTransport};
    struct Borrowed<'a> {
        requests: &'a Mutex<Vec<String>>,
    }
    impl RunTransport for Borrowed<'_> {
        fn namespace(&self) -> &str {
            "orka-system"
        }
        fn provider(&self) -> &str {
            "copilot"
        }
        fn limits(&self) -> RunLimits {
            RunLimits::default()
        }
        fn request<'a>(
            &'a self,
            path: &'a str,
            _after: Option<u64>,
            body: Option<&'a Value>,
            _deadline: tokio::time::Instant,
        ) -> RunFuture<'a> {
            Box::pin(async move {
                self.requests.lock().unwrap().push(path.into());
                let body = body.unwrap();
                assert_eq!(body["namespace"], "orka-system");
                assert_eq!(body["spec"]["providerRef"]["name"], "copilot");
                Err(bridge_engine::engine::run::ErrorKind::Transport)
            })
        }
    }
    let requests = Mutex::new(Vec::new());
    let runner = NativeRunner::with_transport(Borrowed {
        requests: &requests,
    });
    let error = runner.submit(pair()).await.unwrap_err();
    assert_eq!(requests.lock().unwrap().as_slice(), ["agents"]);
    assert_eq!(error.pending.unwrap().kind, "Agent");
    assert!(error.handle.agent().is_none());
}
#[tokio::test]
async fn submits_exact_mapping_and_observes_only_acknowledged_unique_resources() {
    // Catches prompt contamination, model fallback, retries, namespace omission and accidental writes in observe.
    let fixture = Fixture::start(Behavior {
        events: "paged",
        ..Behavior::default()
    })
    .await;
    let runner = fixture.runner();
    let handle = runner.submit(pair()).await.unwrap();
    assert_eq!(handle.snapshot(), pair().snapshot());
    assert_eq!(handle.tasks().len(), 1);
    assert!(!handle.agent().unwrap().uid.is_empty());
    let before = fixture.requests.lock().unwrap().len();
    let report = runner.observe(&handle).await.unwrap();
    assert_eq!(report.verdict, Verdict::Passed);
    assert_eq!(report.cases[0].answer.as_deref(), Some("secret-expected"));
    let second = runner.submit(pair()).await.unwrap();
    assert_ne!(handle.agent().unwrap().name, second.agent().unwrap().name);
    assert_ne!(handle.tasks()[0].name, second.tasks()[0].name);
    let requests = fixture.requests.lock().unwrap();
    let creates: Vec<_> = requests
        .iter()
        .filter(|(p, _)| p.starts_with("POST"))
        .collect();
    assert_eq!(creates.len(), 4);
    assert_eq!(creates[0].1["spec"]["model"], json!({"name":"model:exact"}));
    assert_eq!(
        creates[0].1["spec"]["providerRef"],
        json!({"name":"provider"})
    );
    assert_eq!(
        creates[0].1["spec"]["systemPrompt"],
        json!({"inline":"\nFetch only.\n"})
    );
    assert_eq!(creates[0].1["spec"]["tools"], json!([{"name":"web_fetch"}]));
    assert_eq!(
        creates[1].1["spec"],
        json!({"type":"ai","agentRef":{"name":handle.agent().unwrap().name},"prompt":"Fetch https://example.test/checksum\n","timeout":"120s","retryPolicy":{"maxRetries":0}})
    );
    for (_, body) in creates {
        assert!(!body.to_string().contains("secret-expected"));
        assert_eq!(body["namespace"], "sandbox");
    }
    for (path, _) in &requests[before..requests.len() - 2] {
        assert!(path.starts_with("GET ") && path.contains("namespace=sandbox"));
    }
    assert!(requests.iter().any(|(path, _)| path.contains("after=1")));
}
#[tokio::test]
async fn creates_one_task_per_case_without_reusing_inputs_or_expected_values() {
    let tests = format!("{TESTS}\n```yaml agent-test\nid: second\ninput: distinct input\nassertions:\n - id: other\n   type: contains\n   value: different-expected\n```\n");
    let fixture = Fixture::start(Behavior::default()).await;
    let h = fixture
        .runner()
        .submit(ParsedPair::parse(AGENT.as_bytes(), tests.as_bytes()).unwrap())
        .await
        .unwrap();
    assert_eq!(h.tasks().len(), 2);
    let requests = fixture.requests.lock().unwrap();
    assert_eq!(requests.len(), 3);
    assert_eq!(requests[2].1["spec"]["prompt"], "distinct input");
    assert!(!requests[2].1.to_string().contains("different-expected"));
}
#[tokio::test]
async fn verdicts_depend_on_answer_completion_and_positive_attributable_tool_evidence() {
    // Each row attacks a different proof source; missing tools are never a negative completeness claim.
    for (behavior, contains, tool, overall) in [
        (
            Behavior {
                answer: Some("wrong"),
                ..Behavior::default()
            },
            Verdict::Failed,
            Verdict::Passed,
            Verdict::Failed,
        ),
        (
            Behavior {
                answer: Some(""),
                ..Behavior::default()
            },
            Verdict::Failed,
            Verdict::Passed,
            Verdict::Failed,
        ),
        (
            Behavior {
                answer: None,
                ..Behavior::default()
            },
            Verdict::Unknown,
            Verdict::Passed,
            Verdict::Unknown,
        ),
        (
            Behavior {
                phase: "Failed",
                events: "missing",
                ..Behavior::default()
            },
            Verdict::Passed,
            Verdict::Unknown,
            Verdict::Failed,
        ),
        (
            Behavior {
                phase: "Cancelled",
                ..Behavior::default()
            },
            Verdict::Passed,
            Verdict::Passed,
            Verdict::Failed,
        ),
        (
            Behavior {
                events: "late",
                ..Behavior::default()
            },
            Verdict::Passed,
            Verdict::Passed,
            Verdict::Passed,
        ),
    ] {
        let fixture = Fixture::start(behavior).await;
        let runner = fixture.runner();
        let h = runner.submit(pair()).await.unwrap();
        let report = runner.observe(&h).await.unwrap();
        assert_eq!(report.cases[0].assertions[0].verdict, contains);
        assert_eq!(report.cases[0].assertions[1].verdict, tool);
        if tool == Verdict::Passed {
            assert!(report.cases[0]
                .completed_tools
                .iter()
                .any(|name| name == "web_fetch"));
        }
        assert_eq!(report.verdict, overall);
    }
    for events in [
        "missing",
        "disabled",
        "started",
        "failed",
        "wrong-task",
        "wrong-agent",
        "wrong-stream",
        "wrong-event-stream",
        "wrong-namespace",
        "wrong-cursor",
        "no-call-id",
        "gap",
        "repeated-seq",
    ] {
        let fixture = Fixture::start(Behavior {
            events,
            ..Behavior::default()
        })
        .await;
        let runner = fixture.runner();
        let h = runner.submit(pair()).await.unwrap();
        let report = runner.observe(&h).await.unwrap();
        assert_eq!(
            report.cases[0].assertions[0].verdict,
            Verdict::Passed,
            "{events}"
        );
        assert_eq!(
            report.cases[0].assertions[1].verdict,
            Verdict::Unknown,
            "{events}"
        );
        assert!(report.cases[0].completed_tools.is_empty(), "{events}");
        assert_eq!(report.verdict, Verdict::Unknown);
    }
}
#[tokio::test]
async fn observation_rejects_replaced_tasks_and_changed_agent_versions() {
    for fault in [
        "task-before",
        "task-after",
        "task-type",
        "agent-after",
        "agent-generation",
        "agent-spec",
    ] {
        let fixture = Fixture::start(Behavior {
            fault,
            ..Behavior::default()
        })
        .await;
        let runner = fixture.runner();
        let h = runner.submit(pair()).await.unwrap();
        let error = runner.observe(&h).await.unwrap_err();
        assert_eq!(error.kind, ErrorKind::Identity, "{fault}");
        assert_eq!(error.handle.tasks()[0].uid, h.tasks()[0].uid);
    }
}
#[tokio::test]
async fn lost_mutation_response_keeps_acknowledgements_and_never_replays_post() {
    for fault in [
        "lost-agent",
        "lost-task",
        "wrong-submit",
        "wrong-create-type",
        "retry-enabled",
    ] {
        let fixture = Fixture::start(Behavior {
            fault,
            ..Behavior::default()
        })
        .await;
        let error = fixture.runner().submit(pair()).await.unwrap_err();
        let requests = fixture.requests.lock().unwrap();
        let task_attempted = matches!(fault, "lost-task" | "retry-enabled");
        assert_eq!(requests.len(), if task_attempted { 2 } else { 1 });
        assert_eq!(error.handle.agent().is_some(), task_attempted);
        assert!(error.handle.tasks().is_empty());
        assert!(error.pending.is_some());
        assert!(!error.pending.as_ref().unwrap().name.is_empty());
        let safe = format!("{error:?} {error}");
        assert!(
            !safe.contains("sensitive-token")
                && !safe.contains("secret-server-body")
                && !safe.contains(&fixture.url)
        );
    }
}
#[tokio::test]
async fn bounded_http_rejects_redirects_malformed_and_oversize_responses_without_retry() {
    for fault in ["redirect", "malformed", "oversize", "slow"] {
        let fixture = Fixture::start(Behavior {
            fault,
            ..Behavior::default()
        })
        .await;
        let error = fixture.runner().submit(pair()).await.unwrap_err();
        assert!(matches!(
            error.kind,
            ErrorKind::Response | ErrorKind::Deadline
        ));
        assert_eq!(fixture.requests.lock().unwrap().len(), 1, "{fault}");
    }
}
#[tokio::test]
async fn deadline_retains_handle_and_does_not_claim_remote_cancellation() {
    let fixture = Fixture::start(Behavior {
        phase: "Running",
        ..Behavior::default()
    })
    .await;
    let target = NativeRunTarget::new(&fixture.url, "sandbox", "provider", None).unwrap();
    let runner = NativeRunner::new(
        target,
        RunLimits {
            request_timeout: Duration::from_millis(30),
            operation_timeout: Duration::from_millis(50),
            poll_interval: Duration::from_millis(1),
        },
    )
    .unwrap();
    let h = runner.submit(pair()).await.unwrap();
    let error = runner.observe(&h).await.unwrap_err();
    assert_eq!(error.kind, ErrorKind::Deadline);
    assert_eq!(error.handle.tasks()[0].uid, h.tasks()[0].uid);
    assert!(fixture
        .requests
        .lock()
        .unwrap()
        .iter()
        .all(|(p, _)| !p.starts_with("DELETE")));
}
#[test]
fn target_rejects_ambient_or_credential_forwarding_authority() {
    for url in [
        "http://example.test",
        "http://localhost",
        "https://user:password@example.test",
        "https://example.test/?token=secret",
        "https://example.test/#secret",
        "file:///tmp/secret",
        "https://example.test/path",
    ] {
        assert!(NativeRunTarget::new(url, "sandbox", "provider", None).is_err());
    }
    assert!(NativeRunTarget::new("https://example.test", "sandbox", "provider", None).is_ok());
    assert!(NativeRunTarget::new("http://[::1]:8080", "sandbox", "provider", None).is_ok());
    assert!(
        NativeRunTarget::new("https://example.test", "../namespace", "provider", None).is_err()
    );
    assert!(NativeRunTarget::new(
        "https://example.test",
        "sandbox",
        "provider",
        Some("bad\nheader")
    )
    .is_err());
}

// Included in orka's test module to reuse its synthetic kubectl and HTTP fixtures.
const OVERVIEW_SESSION: &str = "bridge-chat-0123456789abcdef0123456789abcdef";
fn sessions_kube(port: u16) -> (tempfile::TempDir, Orka) {
    let (dir, run) = kube(port);
    let mut sessions = Orka::new_sessions(installation());
    sessions.kubectl = run.kubectl.clone();
    (dir, sessions)
}
fn ok(body: &Value) -> Option<Vec<u8>> {
    http("200 OK", &body.to_string())
}
fn session_with(transcript: Value) -> Value {
    json!({"name":OVERVIEW_SESSION,"namespace":"orka-system","sessionType":"chat",
        "messageCount":3,"transcript":transcript,"inputTokens":9})
}
fn lines(values: &[Value]) -> Value {
    json!(values
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n"))
}
fn projected(result: EngineResult) -> Vec<Value> {
    let body = reply(result).body;
    body["transcript"]
        .as_str()
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

#[test]
fn sessions_use_a_separate_exact_read_only_identity() {
    let s = server(vec![ok(&json!({"items":[],"metadata":{}}))]);
    let (dir, mut sessions) = sessions_kube(s.port);
    assert_eq!(reply(sessions.session_list(None).unwrap()).status, 200);
    let created: Vec<Value> = fs::read_to_string(dir.path().join("created"))
        .unwrap()
        .lines()
        .filter(|l| !l.is_empty())
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    let labels = json!({"app.kubernetes.io/managed-by":"bridge-guest"});
    let meta = json!({"name":"bridge-native-sessions","namespace":"orka-system","labels":labels});
    assert_eq!(
        created,
        [
            json!({"apiVersion":"v1","kind":"ServiceAccount","metadata":meta,"automountServiceAccountToken":false}),
            json!({"apiVersion":"rbac.authorization.k8s.io/v1","kind":"Role","metadata":meta,"rules":[
                {"apiGroups":["core.orka.ai"],"resources":["sessions"],"verbs":["get","list"]}]}),
            json!({"apiVersion":"rbac.authorization.k8s.io/v1","kind":"RoleBinding","metadata":meta,
                "roleRef":{"apiGroup":"rbac.authorization.k8s.io","kind":"Role","name":"bridge-native-sessions"},
                "subjects":[{"kind":"ServiceAccount","name":"bridge-native-sessions","namespace":"orka-system"}]}),
        ]
    );
    let commands = log(dir.path());
    let p = |rest: &str| format!("{CTX} --namespace=orka-system {rest}");
    assert_eq!(
        commands,
        [
            p("get serviceaccount bridge-native-sessions --ignore-not-found -o json"),
            p("create -f - -o json"),
            p("get role.rbac.authorization.k8s.io bridge-native-sessions --ignore-not-found -o json"),
            p("create -f - -o json"),
            p("get rolebinding.rbac.authorization.k8s.io bridge-native-sessions --ignore-not-found -o json"),
            p("create -f - -o json"),
            p("create token bridge-native-sessions --duration=10m"),
            p("port-forward --address=127.0.0.1 svc/orka-api 0:api"),
        ]
    );
    // Chat keeps its own unchanged Role; neither identity is widened for the other.
    let chat = manifests(CHAT_IDENTITY);
    assert_eq!(
        chat[1].1["rules"],
        json!([
            {"apiGroups":["core.orka.ai"],"resources":["agents"],"verbs":["create","get"]},
            {"apiGroups":["core.orka.ai"],"resources":["tasks"],"verbs":["create","get","delete"]},
            {"apiGroups":["core.orka.ai"],"resources":["sessions"],"verbs":["get"]},
            {"apiGroups":["core.orka.ai"],"resources":["tools","providers"],"verbs":["get"]}])
    );
    assert_ne!(chat[0].1["metadata"], manifests(SESSIONS_IDENTITY)[0].1["metadata"]);
}

#[test]
fn sessions_refuse_an_existing_mismatched_role_without_overwriting_it() {
    let s = server(vec![]);
    let (dir, mut sessions) = sessions_kube(s.port);
    for (i, (kind, value)) in manifests(SESSIONS_IDENTITY).iter().enumerate() {
        let mut value = value.clone();
        if i == 1 {
            value["rules"][0]["verbs"] = json!(["get", "list", "delete"]);
        }
        fs::write(dir.path().join(format!("{kind}.json")), value.to_string()).unwrap();
    }
    assert!(sessions.session_list(None).is_err());
    assert!(!dir.path().join("created").exists());
    assert!(s.seen.lock().unwrap().is_empty());
}

#[test]
fn session_list_reads_fixed_pages_and_keeps_only_summary_fields() {
    let item = json!({"id":"x","name":"s1","namespace":"orka-system","sessionType":"chat",
        "messageCount":4,"inputTokens":10,"activeTask":"run-1","createdAt":"2026-01-01T00:00:00Z",
        "updatedAt":"2026-01-02T00:00:00Z","transcript":"{\"role\":\"user\",\"content\":\"private\"}",
        "unknown":{"nested":"private"}});
    let sparse = json!({"name":"s2","namespace":"orka-system","messageCount":0,
        "activeTask":{"nested":"private"},"sessionType":"x".repeat(600)});
    let s = server(vec![
        ok(&json!({"items":[item, sparse],"metadata":{"continue":"a+/="}})),
        ok(&json!({"items":[],"metadata":{"continue":""}})),
        ok(&json!({"items":[]})),
    ]);
    let (_dir, mut sessions) = sessions_kube(s.port);
    let first = reply(sessions.session_list(None).unwrap());
    assert_eq!(
        first,
        OrkaReply {
            status: 200,
            body: json!({"items":[
                {"name":"s1","namespace":"orka-system","sessionType":"chat","messageCount":4,
                    "createdAt":"2026-01-01T00:00:00Z","updatedAt":"2026-01-02T00:00:00Z","activeTask":"run-1"},
                {"name":"s2","namespace":"orka-system","messageCount":0}],
                "metadata":{"continue":"a+/="}})
        }
    );
    let last = reply(sessions.session_list(Some("a+/=")).unwrap());
    assert_eq!(last.body, json!({"items":[],"metadata":{"continue":null}}));
    let none = reply(sessions.session_list(Some("b")).unwrap());
    assert_eq!(none.body["metadata"]["continue"], Value::Null);
    let seen = s.seen.lock().unwrap().clone();
    assert!(seen[0].starts_with("GET /api/v1/sessions?namespace=orka-system&limit=20 HTTP/1.1\r\n"));
    assert!(seen[1].starts_with(
        "GET /api/v1/sessions?namespace=orka-system&limit=20&continue=a%2B%2F%3D HTTP/1.1\r\n"
    ));
    assert!(seen[2].starts_with(
        "GET /api/v1/sessions?namespace=orka-system&limit=20&continue=b HTTP/1.1\r\n"
    ));
    for request in &seen {
        assert!(request.contains("authorization: Bearer synthetic-bearer-1\r\n"));
        assert!(!request.contains("content-type"));
    }
}

#[test]
fn session_list_refuses_bad_cursors_and_reports_only_status_on_refusal() {
    let (dir, mut sessions) = sessions_kube(1);
    let long = "a".repeat(MAX_SESSION_CURSOR + 1);
    for after in ["", "a b", "a&limit=1", long.as_str()] {
        assert!(sessions.session_list(Some(after)).is_err(), "{after}");
    }
    assert!(log(dir.path()).is_empty(), "invalid input runs no command");
    let many = vec![json!({"name":"s"}); SESSION_PAGE + 1];
    let s = server(vec![
        ok(&json!({"items":[],"metadata":{"continue":"a b"}})),
        ok(&json!({"items":[],"metadata":{"continue":7}})),
        ok(&json!({"items":[],"metadata":{"continue":"a".repeat(MAX_SESSION_CURSOR + 1)}})),
        ok(&json!({"items":{}})),
        ok(&json!({"items":["s"]})),
        ok(&json!({ "items": many })),
        http("200 OK", "not json"),
        http("403 Forbidden", r#"{"message":"synthetic-private"}"#),
        http("401 Unauthorized", "denied"),
        http("500 Internal Server Error", r#"{"items":[]}"#),
    ]);
    let (_dir, mut sessions) = sessions_kube(s.port);
    for _ in 0..7 {
        assert!(sessions.session_list(None).is_err());
    }
    for status in [403, 401, 500] {
        let result = sessions.session_list(None).unwrap();
        assert!(!String::from_utf8_lossy(&result.stdout).contains("private"));
        assert_eq!(
            reply(result),
            OrkaReply {
                status,
                body: Value::Null
            }
        );
    }
    assert_eq!(s.seen.lock().unwrap().len(), 10, "no retries");
}

#[test]
fn session_raw_replies_above_the_general_cap_project_but_above_the_session_cap_fail() {
    let big = json!({"items":[{"name":"s","transcript":"x".repeat(2 * MAX_ORKA_REPLY)}],"metadata":{}});
    let over = format!("\"{}\"", "x".repeat(MAX_SESSION_READ));
    let s = server(vec![ok(&big)]);
    let (_dir, mut sessions) = sessions_kube(s.port);
    let result = sessions.session_list(None).unwrap();
    assert!(result.stdout.len() < 200);
    assert_eq!(reply(result).body["items"], json!([{"name":"s"}]));
    // One oversize reply per server: the fixture stops when the client stops reading.
    for status in ["200 OK", "404 Not Found"] {
        let s = server(vec![http(status, &over)]);
        let (_dir, mut sessions) = sessions_kube(s.port);
        assert!(sessions.session_read(OVERVIEW_SESSION).is_err(), "{status}");
        assert_eq!(s.seen.lock().unwrap().len(), 1);
    }
}

#[test]
fn session_bearer_echo_is_refused_raw_or_escaped() {
    let s = server(vec![
        http("200 OK", r#"{"items":[{"name":"{AUTH}"}],"metadata":{}}"#),
        http("200 OK", r#"{"items":[{"name":"synthetic-bearer-1"}],"metadata":{}}"#),
        http("403 Forbidden", r#"{"message":"{AUTH}"}"#),
        // The line escapes the bearer, and the Session escapes the line again.
        ok(&session_with(json!(
            r#"{"role":"user","content":"\u0073ynthetic-bearer-1"}"#
        ))),
    ]);
    let (_dir, mut sessions) = sessions_kube(s.port);
    assert!(sessions.session_list(None).is_err());
    assert!(sessions.session_list(None).is_err());
    assert!(sessions.session_list(None).is_err(), "an echo in a refusal");
    assert!(sessions.session_read(OVERVIEW_SESSION).is_err(), "escaped twice, in a transcript line");
    assert_eq!(s.seen.lock().unwrap().len(), 4);
}

#[test]
fn session_read_keeps_latest_bounded_messages_and_tool_names_only() {
    let mut messages: Vec<Value> = (0..SESSION_MESSAGES + 50)
        .map(|i| json!({"role":"user","content":format!("m{i}"),"ts":"2026-01-01T00:00:00Z","extra":"private"}))
        .collect();
    let long = "é".repeat(SESSION_MESSAGE_TEXT); // two bytes each
    messages.push(json!({"role":"assistant","content":long,"toolCalls":[
        {"name":"web_fetch","arguments":{"url":"https://private.example"}},
        {"id":"c2","function":{"name":"shell","arguments":"rm private"}},
        {"id":"c3"}],"ts":"2026-01-01T00:00:01Z"}));
    let mut text = lines(&messages).as_str().unwrap().to_owned();
    text.push_str("\n\n   \n");
    let s = server(vec![
        ok(&session_with(json!(text))),
        ok(&session_with(Value::Null)),
        ok(&session_with(json!(""))),
    ]);
    let (_dir, mut sessions) = sessions_kube(s.port);
    let result = sessions.session_read(OVERVIEW_SESSION).unwrap();
    assert!(!String::from_utf8_lossy(&result.stdout).contains("private"));
    let body = reply(result.clone()).body;
    assert_eq!(
        body.as_object().unwrap().keys().collect::<Vec<_>>(),
        ["name", "namespace", "transcript", "transcriptCut"]
    );
    assert_eq!(body["name"], OVERVIEW_SESSION);
    assert_eq!(body["namespace"], "orka-system");
    assert_eq!(body["transcriptCut"], true);
    let kept = projected(result);
    assert_eq!(kept.len(), SESSION_MESSAGES);
    assert_eq!(
        kept[0],
        json!({"role":"user","content":"m51","ts":"2026-01-01T00:00:00Z"})
    );
    let last = kept.last().unwrap();
    let content = last["content"].as_str().unwrap();
    assert_eq!(content.len(), SESSION_MESSAGE_TEXT);
    assert!(content.chars().all(|c| c == 'é'));
    assert_eq!(last["cut"], true);
    assert_eq!(
        last["toolCalls"],
        json!([{"name":"web_fetch"},{"name":"shell"}])
    );
    let odd = "é".repeat(SESSION_MESSAGE_TEXT / 2 - 1) + "x" + &"é".repeat(4);
    let s2 = server(vec![ok(&session_with(lines(&[json!({"role":"user","content":odd})])))]);
    let (_d, mut other) = sessions_kube(s2.port);
    let cut = &projected(other.session_read(OVERVIEW_SESSION).unwrap())[0];
    assert_eq!(cut["content"].as_str().unwrap().len(), SESSION_MESSAGE_TEXT - 1);
    for _ in 0..2 {
        assert_eq!(
            reply(sessions.session_read(OVERVIEW_SESSION).unwrap()).body,
            json!({"name":OVERVIEW_SESSION,"namespace":"orka-system","transcript":"","transcriptCut":false})
        );
    }
    let seen = s.seen.lock().unwrap().clone();
    assert!(seen[0].starts_with(&format!(
        "GET /api/v1/sessions/{OVERVIEW_SESSION}?namespace=orka-system HTTP/1.1\r\n"
    )));
}

#[test]
fn session_read_leaves_out_unparseable_lines_and_refuses_another_session() {
    let text = [
        json!({"role":"user","content":"hi"}).to_string(),
        "{not json".into(),
        "\"just a string\"".into(),
        json!({"role":"assistant","content":"hello"}).to_string(),
    ]
    .join("\n");
    let mut other = session_with(Value::Null);
    other["name"] = json!("another");
    let mut array = session_with(Value::Null);
    array["transcript"] = json!(["x"]);
    let s = server(vec![
        ok(&session_with(json!(text))),
        ok(&other),
        ok(&array),
        http("404 Not Found", r#"{"error":"missing"}"#),
    ]);
    let (_dir, mut sessions) = sessions_kube(s.port);
    let result = sessions.session_read(OVERVIEW_SESSION).unwrap();
    assert_eq!(reply(result.clone()).body["transcriptCut"], true);
    assert_eq!(
        projected(result),
        [
            json!({"role":"user","content":"hi"}),
            json!({"role":"assistant","content":"hello"})
        ]
    );
    assert!(sessions.session_read(OVERVIEW_SESSION).is_err());
    assert!(sessions.session_read(OVERVIEW_SESSION).is_err());
    assert_eq!(
        reply(sessions.session_read(OVERVIEW_SESSION).unwrap()),
        OrkaReply {
            status: 404,
            body: Value::Null
        }
    );
    let (dir, mut sessions) = sessions_kube(1);
    for name in ["", "../x", "a/b", "a?b", "-a"] {
        assert!(sessions.session_read(name).is_err());
    }
    assert!(log(dir.path()).is_empty());
}

#[test]
fn session_read_drops_oldest_lines_until_the_reply_fits_one_result() {
    let messages: Vec<Value> = (0..SESSION_MESSAGES)
        .map(|i| {
            // Quotes grow when escaped; the fit counts escaped bytes.
            let text = format!("{i:04}{}{}", "\"".repeat(4096), "x".repeat(SESSION_MESSAGE_TEXT));
            json!({"role":"user","content":text})
        })
        .collect();
    let s = server(vec![ok(&session_with(lines(&messages)))]);
    let (_dir, mut sessions) = sessions_kube(s.port);
    let result = sessions.session_read(OVERVIEW_SESSION).unwrap();
    assert!(result.stdout.len() <= MAX_FILE);
    let body = reply(result.clone()).body;
    assert_eq!(body["transcriptCut"], true);
    let kept = projected(result.clone());
    assert!(!kept.is_empty() && kept.len() < SESSION_MESSAGES);
    let newest = SESSION_MESSAGES - 1;
    assert!(kept.last().unwrap()["content"]
        .as_str()
        .unwrap()
        .starts_with(&format!("{newest:04}")));
    let first = SESSION_MESSAGES - kept.len();
    assert!(kept[0]["content"].as_str().unwrap().starts_with(&format!("{first:04}")));
    // One more line would not have fit.
    assert!(result.stdout.len() + result.stdout.len() / kept.len() > MAX_FILE - 4096);
    // And the engine accepts it as an operation result.
    let root = tempfile::tempdir().unwrap();
    let staging = crate::staging::Staging::new(root.path().join("snapshots")).unwrap();
    let s = server(vec![ok(&session_with(lines(&messages)))]);
    let (_dir, sessions) = sessions_kube(s.port);
    let runner = crate::engine::GuestRunner {
        kmx: crate::engine::Kmx,
        orka: Orka::new(installation()),
        chat: Orka::new_chat(installation()),
        sessions,
        catalog_provider: None,
    };
    let mut ops = crate::engine::Operations::new(root.path().join("ops"), runner).unwrap();
    let id = Uuid::new_v4();
    let action = EngineActionWire::SessionRead {
        name: OVERVIEW_SESSION.into(),
    };
    assert_eq!(
        ops.execute(id, action, None, &staging).unwrap(),
        OperationState::Completed
    );
    assert_eq!(ops.info(id).unwrap().bytes, result.stdout.len());
}

#[test]
fn sessions_identity_serves_only_sessions_and_others_never_serve_sessions() {
    let (dir, mut sessions) = sessions_kube(1);
    assert!(sessions.execute(&get(&format!("agents/{RUN}"))).is_err());
    assert!(sessions
        .execute_chat(&ChatRequest {
            method: ChatMethod::Get,
            path: format!("sessions/{OVERVIEW_SESSION}"),
            after: None,
            body: None,
        })
        .is_err());
    assert!(sessions.run_worker(TASK, "uid").is_err());
    assert!(sessions.cleanup_run_agent("uid").is_err());
    let catalog = sessions.catalog("copilot", None);
    assert_eq!(catalog.models, CatalogSection::Unavailable {});
    assert_eq!(catalog.tools, CatalogSection::Unavailable {});
    assert!(log(dir.path()).is_empty());
    let (dir, mut run) = kube(1);
    let mut chat = Orka::new_chat(installation());
    chat.kubectl = run.kubectl.clone();
    for orka in [&mut run, &mut chat] {
        assert!(orka.session_list(None).is_err());
        assert!(orka.session_read(OVERVIEW_SESSION).is_err());
    }
    assert!(log(dir.path()).is_empty());
}

#[test]
fn runner_dispatches_sessions_to_their_identity_and_never_persists_a_bearer() {
    let s = server(vec![
        ok(&json!({"items":[{"name":"s1","transcript":"private"}],"metadata":{}})),
        http("200 OK", r#"{"items":[{"name":"{AUTH}"}],"metadata":{}}"#),
    ]);
    let (dir, sessions) = sessions_kube(s.port);
    let root = tempfile::tempdir().unwrap();
    let staging = crate::staging::Staging::new(root.path().join("snapshots")).unwrap();
    let runner = crate::engine::GuestRunner {
        kmx: crate::engine::Kmx,
        orka: Orka::new(installation()),
        chat: Orka::new_chat(installation()),
        sessions,
        catalog_provider: None,
    };
    let mut ops = crate::engine::Operations::new(root.path().join("ops"), runner).unwrap();
    let action = EngineActionWire::SessionList { after: None };
    assert!(crate::engine::argv(&action).is_err());
    assert!(crate::engine::argv(&EngineActionWire::SessionRead {
        name: OVERVIEW_SESSION.into()
    })
    .is_err());
    let id = Uuid::new_v4();
    assert_eq!(
        ops.execute(id, action.clone(), None, &staging).unwrap(),
        OperationState::Completed
    );
    let got: OrkaReply = serde_json::from_slice(&ops.read(id, 0).unwrap()).unwrap();
    assert_eq!(got.body["items"], json!([{"name":"s1"}]));
    assert_eq!(
        ops.execute(Uuid::new_v4(), action, None, &staging).unwrap(),
        OperationState::Unknown
    );
    assert!(log(dir.path())
        .iter()
        .any(|l| l.contains("create token bridge-native-sessions")));
    for path in walk(root.path()) {
        let bytes = fs::read(path).unwrap();
        let text = String::from_utf8_lossy(&bytes);
        assert!(!text.contains("synthetic-bearer") && !text.contains("private"));
    }
}

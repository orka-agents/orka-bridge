use bridge_runtime_protocol::*;
use serde_json::{json, Value};

const AGENT: &str = "bridge-chat-agent-0123456789abcdef0123456789abcdef";
const TASK: &str = "bridge-chat-task-0123456789abcdef0123456789abcdef";
const SESSION: &str = "bridge-chat-0123456789abcdef0123456789abcdef";
fn action(method: &str, path: &str, after: Value, body: Value) -> Value {
    json!({"type":"orkaChat","request":{"method":method,"path":path,"after":after,"body":body}})
}
fn valid(v: Value) -> bool {
    serde_json::from_value::<EngineActionWire>(v).is_ok_and(|a| validate_action(&a).is_ok())
}
fn create(name: &str, spec: Value) -> Value {
    json!({"name":name,"namespace":"orka-system","spec":spec})
}
fn task() -> Value {
    create(
        TASK,
        json!({"type":"ai","agentRef":{"name":AGENT},"prompt":"hello","sessionRef":{"name":SESSION,"create":true,"append":true,"maxMessages":50}}),
    )
}
#[test]
fn chat_wire_accepts_only_generated_chat_resources() {
    let mut cases = vec![
        action("post", "agents", json!(null), create(AGENT, json!({}))),
        action("post", "tasks", json!(null), task()),
        action("delete", &format!("tasks/{TASK}"), json!(null), json!(null)),
        action(
            "get",
            &format!("tasks/{TASK}/events"),
            json!(0),
            json!(null),
        ),
    ];
    for path in [
        format!("agents/{AGENT}"),
        format!("tasks/{TASK}"),
        format!("tasks/{TASK}/result"),
        format!("sessions/{SESSION}"),
    ] {
        cases.push(action("get", &path, json!(null), json!(null)));
    }
    for value in cases {
        assert!(valid(value.clone()), "{value}");
        let a: EngineActionWire = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(serde_json::to_value(&a).unwrap(), value);
        let frame = Frame::Request(WireRequest {
            version: VERSION,
            boot_nonce: Uuid::from_u128(1),
            request_id: 1,
            body: RequestBody::Execute {
                operation: Uuid::from_u128(2),
                action: a,
                snapshot: None,
            },
        });
        assert!(frame.validate().is_ok());
        assert!(encode(&frame).is_ok());
    }
}
#[test]
fn chat_methods_paths_and_bodies_cannot_cross_run_authority() {
    for path in [
        "tasks".to_owned(),
        "sessions".into(),
        "secrets".into(),
        format!("agents/{AGENT}"),
        format!("sessions/{SESSION}"),
        "tasks/bridge-task-0123456789abcdef0123456789abcdef".into(),
        format!("tasks/{TASK}/result"),
        format!("tasks/{TASK}?namespace=other"),
        format!("tasks/{TASK}/../x"),
        format!("tasks/{TASK}%2fresult"),
        format!("tasks/{TASK}#x"),
        format!("tasks/{}", TASK.to_uppercase()),
        "http://127.0.0.1/tasks/x".into(),
    ] {
        assert!(
            !valid(action("delete", &path, json!(null), json!(null))),
            "{path}"
        );
    }
    for method in ["get", "delete", "put", "patch", "DELETE"] {
        assert!(
            !valid(action(method, "tasks", json!(null), task())),
            "{method}"
        );
    }
    assert!(!valid(action(
        "post",
        &format!("tasks/{TASK}"),
        json!(null),
        task()
    )));
    assert!(!valid(action(
        "get",
        &format!("tasks/{TASK}/events"),
        json!(null),
        json!(null)
    )));
    assert!(!valid(action(
        "delete",
        &format!("tasks/{TASK}"),
        json!(0),
        json!(null)
    )));
    assert!(!valid(action(
        "get",
        &format!("tasks/{TASK}"),
        json!(null),
        json!({})
    )));
    assert!(!valid(action("get", "tools", json!(null), json!(null))));
    let mut run = action(
        "delete",
        "tasks/bridge-task-0123456789abcdef0123456789abcdef",
        json!(null),
        json!(null),
    );
    run["type"] = json!("orka");
    assert!(!valid(run)); // Run's existing schema must never gain a method field.
}
#[test]
fn chat_creates_bind_generated_native_agent_and_session() {
    for (pointer, value) in [
        ("/namespace", json!("other")),
        ("/name", json!(AGENT)),
        ("/spec/type", json!("agent")),
        ("/spec/agentRef/name", json!("foreign")),
        ("/spec/agentRef/namespace", json!("other")),
        ("/spec/sessionRef/name", json!("foreign")),
    ] {
        let mut body = task();
        let parts: Vec<_> = pointer.trim_start_matches('/').split('/').collect();
        let mut current = &mut body;
        for p in &parts[..parts.len() - 1] {
            current = &mut current[*p];
        }
        current[parts[parts.len() - 1]] = value;
        assert!(
            !valid(action("post", "tasks", json!(null), body)),
            "{pointer}"
        );
    }
    let mut same = task();
    same["spec"]["agentRef"]["namespace"] = json!("orka-system");
    assert!(valid(action("post", "tasks", json!(null), same)));
    let mut extra = create(AGENT, json!({}));
    extra["url"] = json!("x");
    assert!(!valid(action("post", "agents", json!(null), extra)));
    assert!(!valid(action(
        "post",
        "agents",
        json!(null),
        create(TASK, json!({}))
    )));
}
#[test]
fn chat_bounds_and_unknown_fields_fail_closed() {
    let fill = |n: usize| create(AGENT, json!({"systemPrompt":{"inline":"x".repeat(n)}}));
    let base = serde_json::to_vec(&fill(0)).unwrap().len();
    let value = action("post", "agents", json!(null), fill(MAX_ORKA_BODY - base));
    assert!(valid(value.clone()));
    let a = serde_json::from_value(value).unwrap();
    let frame = Frame::Request(WireRequest {
        version: VERSION,
        boot_nonce: Uuid::from_u128(1),
        request_id: 1,
        body: RequestBody::Execute {
            operation: Uuid::from_u128(2),
            action: a,
            snapshot: None,
        },
    });
    assert!(encode(&frame).unwrap().len() < MAX_FRAME);
    assert!(!valid(action(
        "post",
        "agents",
        json!(null),
        fill(MAX_ORKA_BODY - base + 1)
    )));
    for field in ["url", "headers", "namespace"] {
        let mut a = action("get", &format!("tasks/{TASK}"), json!(null), json!(null));
        a["request"][field] = json!("x");
        assert!(!valid(a));
    }
}
const NAV_AGENT: &str = "bridge-nav-agent-0123456789abcdef0123456789abcdef";
const NAV_TASK: &str = "bridge-nav-task-0123456789abcdef0123456789abcdef";
const NAV_SESSION: &str = "bridge-nav-0123456789abcdef0123456789abcdef";
fn nav_task(agent: &str, session: &str) -> Value {
    create(
        NAV_TASK,
        json!({"type":"ai","agentRef":{"name":agent},"prompt":"hello","sessionRef":{"name":session,"create":true,"append":true,"maxMessages":50}}),
    )
}
#[test]
fn navigator_family_has_the_same_shape_as_chat() {
    assert_eq!(NAVIGATOR_FAMILY.agent, "bridge-nav-agent-");
    assert_eq!(NAVIGATOR_FAMILY.task, "bridge-nav-task-");
    assert_eq!(NAVIGATOR_FAMILY.session, "bridge-nav-");
    assert_eq!(CHAT_FAMILY.agent, "bridge-chat-agent-");
    let mut cases = vec![
        action("post", "agents", json!(null), create(NAV_AGENT, json!({}))),
        action(
            "post",
            "tasks",
            json!(null),
            nav_task(NAV_AGENT, NAV_SESSION),
        ),
        action(
            "delete",
            &format!("tasks/{NAV_TASK}"),
            json!(null),
            json!(null),
        ),
        action(
            "get",
            &format!("tasks/{NAV_TASK}/events"),
            json!(0),
            json!(null),
        ),
    ];
    for path in [
        format!("agents/{NAV_AGENT}"),
        format!("tasks/{NAV_TASK}"),
        format!("tasks/{NAV_TASK}/result"),
        format!("sessions/{NAV_SESSION}"),
    ] {
        cases.push(action("get", &path, json!(null), json!(null)));
    }
    for value in cases {
        assert!(valid(value.clone()), "{value}");
    }
}
#[test]
fn navigator_and_chat_resources_never_reference_each_other() {
    // A Task binds only the Agent and Session of its own family, in both directions.
    for body in [
        nav_task(AGENT, NAV_SESSION),
        nav_task(NAV_AGENT, SESSION),
        nav_task(AGENT, SESSION),
    ] {
        assert!(
            !valid(action("post", "tasks", json!(null), body.clone())),
            "{body}"
        );
    }
    let mut chat = task();
    chat["spec"]["agentRef"]["name"] = json!(NAV_AGENT);
    assert!(!valid(action("post", "tasks", json!(null), chat)));
    let mut chat = task();
    chat["spec"]["sessionRef"]["name"] = json!(NAV_SESSION);
    assert!(!valid(action("post", "tasks", json!(null), chat)));
    // Names must be complete: a family prefix alone or another kind's prefix is not a name.
    for path in [
        format!("sessions/{NAV_AGENT}"),
        format!("agents/{NAV_SESSION}"),
        format!("agents/{NAV_TASK}"),
        format!("tasks/{NAV_AGENT}"),
        "sessions/bridge-nav-".to_owned(),
        format!("sessions/{}", NAV_SESSION.to_uppercase()),
        "agents/bridge-nav-agent-0123".to_owned(),
    ] {
        assert!(
            !valid(action("get", &path, json!(null), json!(null))),
            "{path}"
        );
    }
    assert!(!valid(action(
        "post",
        "agents",
        json!(null),
        create(NAV_TASK, json!({}))
    )));
    assert!(!valid(action(
        "post",
        "tasks",
        json!(null),
        create(NAV_AGENT, json!({}))
    )));
    // Only generated Tasks can be deleted, never Agents or Sessions of either family.
    for path in [
        format!("agents/{NAV_AGENT}"),
        format!("sessions/{NAV_SESSION}"),
    ] {
        assert!(
            !valid(action("delete", &path, json!(null), json!(null))),
            "{path}"
        );
    }
}

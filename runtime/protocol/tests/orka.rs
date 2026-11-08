use bridge_runtime_protocol::*;
use serde_json::{json, Value};

const RUN: &str = "bridge-run-0123456789abcdef0123456789abcdef";
const TASK: &str = "bridge-task-0123456789abcdef0123456789abcdef";
fn get(path: &str, after: Option<u64>) -> OrkaRequest {
    OrkaRequest {
        path: path.into(),
        after,
        body: None,
    }
}
fn post(path: &str, body: Value) -> OrkaRequest {
    OrkaRequest {
        path: path.into(),
        after: None,
        body: Some(body),
    }
}
fn agent() -> Value {
    json!({"name":RUN,"namespace":"orka-system","spec":{"model":{"name":"m"}}})
}
fn task() -> Value {
    json!({"name":TASK,"namespace":"orka-system","spec":{"type":"ai"}})
}
fn execute(request: OrkaRequest) -> Frame {
    Frame::Request(WireRequest {
        version: VERSION,
        boot_nonce: Uuid::from_u128(9),
        request_id: 3,
        body: RequestBody::Execute {
            operation: Uuid::from_u128(4),
            action: EngineActionWire::Orka { request },
            snapshot: None,
        },
    })
}

#[test]
fn only_run_create_and_generated_reads_are_allowed() {
    assert_eq!(post("agents", agent()).validate(), Ok(true));
    assert_eq!(post("tasks", task()).validate(), Ok(true));
    for path in [
        format!("agents/{RUN}"),
        format!("tasks/{TASK}"),
        format!("tasks/{TASK}/result"),
    ] {
        assert_eq!(get(&path, None).validate(), Ok(false), "{path}");
    }
    assert_eq!(
        get(&format!("tasks/{TASK}/events"), Some(7)).validate(),
        Ok(false)
    );
    let wire = serde_json::to_value(EngineActionWire::Orka {
        request: get(&format!("tasks/{TASK}/events"), Some(7)),
    })
    .unwrap();
    assert_eq!(
        wire,
        json!({"type":"orka","request":{"path":format!("tasks/{TASK}/events"),"after":7,"body":null}})
    );
}

#[test]
fn other_endpoints_traversal_queries_and_foreign_names_are_refused() {
    let refused = [
        "".to_owned(),
        "agents/".into(),
        "/agents".into(),
        "providers".into(),
        "tools".into(),
        "secrets".into(),
        format!("agents/{TASK}"),
        format!("tasks/{RUN}"),
        format!("agents/{RUN}/result"),
        format!("tasks/{TASK}/logs"),
        format!("tasks/{TASK}/../../secrets"),
        format!("tasks/{TASK}/result?namespace=kube-system"),
        format!("tasks/{TASK}/result#x"),
        format!("tasks/{TASK}%2Fresult"),
        format!("agents/{}", RUN.to_uppercase()),
        format!("agents/{RUN}0"),
        "agents/bridge-run-".into(),
        "agents/other".into(),
        "http://127.0.0.1/api/v1/agents".into(),
        format!("tasks//{TASK}"),
        format!("tasks/{TASK}/events/"),
    ];
    for path in refused {
        assert!(get(&path, None).validate().is_err(), "{path}");
        assert!(post(&path, agent()).validate().is_err(), "{path}");
    }
    // `after` belongs only to events, and events require it.
    assert!(get(&format!("tasks/{TASK}/events"), None)
        .validate()
        .is_err());
    assert!(get(&format!("tasks/{TASK}/result"), Some(1))
        .validate()
        .is_err());
    assert!(post("tasks", task()).validate().is_ok());
    let mut with_after = post("tasks", task());
    with_after.after = Some(0);
    assert!(with_after.validate().is_err());
}

#[test]
fn reads_never_carry_bodies_and_creates_need_fixed_namespace_generated_names() {
    assert!(post(&format!("agents/{RUN}"), agent()).validate().is_err());
    assert!(get("agents", None).validate().is_err());
    assert!(post("agents", task()).validate().is_err());
    assert!(post("tasks", agent()).validate().is_err());
    for body in [
        json!({"name":RUN,"namespace":"default","spec":{}}),
        json!({"name":RUN,"spec":{}}),
        json!({"name":"bridge-run-x","namespace":"orka-system","spec":{}}),
        json!({"name":RUN,"namespace":"orka-system","spec":{},"headers":{"x":"y"}}),
        json!({"name":RUN,"namespace":"orka-system","spec":"text"}),
        json!({"name":RUN,"namespace":"orka-system"}),
        json!([agent()]),
        json!(null),
    ] {
        assert!(post("agents", body.clone()).validate().is_err(), "{body}");
    }
}

#[test]
fn create_body_is_capped_and_any_valid_request_fits_one_frame() {
    let fill = |n: usize| json!({"name":RUN,"namespace":"orka-system","spec":{"systemPrompt":{"inline":"x".repeat(n)}}});
    let size = |v: &Value| serde_json::to_vec(v).unwrap().len();
    let base = size(&fill(0));
    let largest = fill(MAX_ORKA_BODY - base);
    assert_eq!(size(&largest), MAX_ORKA_BODY);
    assert_eq!(post("agents", largest.clone()).validate(), Ok(true));
    assert!(encode(&execute(post("agents", largest))).unwrap().len() < MAX_FRAME);
    assert_eq!(
        post("agents", fill(MAX_ORKA_BODY - base + 1)).validate(),
        Err(ProtocolError::Oversize)
    );
    assert_eq!(
        encode(&execute(post("agents", fill(MAX_ORKA_BODY)))),
        Err(ProtocolError::Oversize)
    );
}

#[test]
fn wire_frames_validate_orka_actions_and_reject_unknown_fields() {
    assert!(execute(post("agents", agent())).validate().is_ok());
    assert_eq!(
        execute(get("secrets", None)).validate(),
        Err(ProtocolError::Invalid)
    );
    assert!(validate_action(&EngineActionWire::Orka {
        request: get("tools", None)
    })
    .is_err());
    for text in [
        json!({"type":"orka","request":{"path":"tasks","after":null,"body":null,"url":"x"}}),
        json!({"type":"orka","request":{"path":"tasks","after":null,"body":null,"headers":{}}}),
        json!({"type":"orka","request":{"path":"tasks","after":null,"body":null},"namespace":"x"}),
    ] {
        assert!(serde_json::from_value::<EngineActionWire>(text).is_err());
    }
    let reply = OrkaReply {
        status: 404,
        body: json!({"error":"missing"}),
    };
    let text = serde_json::to_value(&reply).unwrap();
    assert_eq!(text, json!({"status":404,"body":{"error":"missing"}}));
    assert_eq!(serde_json::from_value::<OrkaReply>(text).unwrap(), reply);
    assert!(serde_json::from_value::<OrkaReply>(
        json!({"status":200,"body":{},"diagnostics":"raw"})
    )
    .is_err());
}

use bridge_runtime_protocol::*;
use serde_json::{json, Value};

fn valid(v: Value) -> bool {
    serde_json::from_value::<EngineActionWire>(v).is_ok_and(|a| validate_action(&a).is_ok())
}
fn execute(action: EngineActionWire) -> Frame {
    Frame::Request(WireRequest {
        version: VERSION,
        boot_nonce: Uuid::from_u128(9),
        request_id: 3,
        body: RequestBody::Execute {
            operation: Uuid::from_u128(4),
            action,
            snapshot: None,
        },
    })
}

#[test]
fn session_actions_round_trip_with_fixed_tags() {
    for (action, text) in [
        (
            EngineActionWire::SessionList { after: None },
            json!({"type":"sessionList","after":null}),
        ),
        (
            EngineActionWire::SessionList {
                after: Some("eyJ2IjoxfQ==".into()),
            },
            json!({"type":"sessionList","after":"eyJ2IjoxfQ=="}),
        ),
        (
            EngineActionWire::SessionRead {
                name: "bridge-chat-0123".into(),
            },
            json!({"type":"sessionRead","name":"bridge-chat-0123"}),
        ),
    ] {
        assert_eq!(serde_json::to_value(&action).unwrap(), text);
        assert_eq!(
            serde_json::from_value::<EngineActionWire>(text).unwrap(),
            action
        );
        assert!(execute(action).validate().is_ok());
    }
    assert_eq!(
        serde_json::from_value::<EngineActionWire>(json!({"type":"sessionList"})).unwrap(),
        EngineActionWire::SessionList { after: None }
    );
    for text in [
        json!({"type":"sessionList","after":null,"limit":100}),
        json!({"type":"sessionRead","name":"s","namespace":"other"}),
        json!({"type":"sessionRead"}),
        json!({"type":"sessionRead","name":null}),
        json!({"type":"sessionList","after":7}),
    ] {
        assert!(
            serde_json::from_value::<EngineActionWire>(text.clone()).is_err(),
            "{text}"
        );
    }
}

#[test]
fn session_names_are_object_like_and_cursors_bounded_url_safe_text() {
    let longest = "a".repeat(253);
    for name in [
        "s",
        "0",
        "bridge-chat-0123456789abcdef0123456789abcdef",
        "a.b_c-d",
        longest.as_str(),
    ] {
        assert!(valid(json!({"type":"sessionRead","name":name})), "{name}");
    }
    let long = "a".repeat(254);
    for name in [
        "",
        "-s",
        ".s",
        "_s",
        "a/b",
        "../x",
        "a b",
        "a?b",
        "a%2Fb",
        "é",
        long.as_str(),
    ] {
        assert!(!valid(json!({"type":"sessionRead","name":name})), "{name}");
    }
    let cursor = "a".repeat(MAX_SESSION_CURSOR);
    for after in [
        json!(null),
        json!("eyJ2IjoxfQ=="),
        json!("a+/=:._-Z9"),
        json!(cursor),
    ] {
        assert!(
            valid(json!({"type":"sessionList","after":after})),
            "{after}"
        );
    }
    let over = "a".repeat(MAX_SESSION_CURSOR + 1);
    for after in [
        "",
        "a b",
        "a&limit=1",
        "a?b",
        "a#b",
        "a%b",
        "é",
        over.as_str(),
    ] {
        assert!(
            !valid(json!({"type":"sessionList","after":after})),
            "{after}"
        );
        assert_eq!(
            execute(EngineActionWire::SessionList {
                after: Some(after.into())
            })
            .validate(),
            Err(ProtocolError::Invalid)
        );
    }
}

#[test]
fn session_bounds_fit_the_operation_store() {
    assert_eq!(SESSION_PAGE, 20);
    assert_eq!(SESSION_MESSAGES, 200);
    const { assert!(SESSION_MESSAGE_TEXT < MAX_FILE && MAX_SESSION_READ > MAX_ORKA_REPLY) };
    // Any valid action fits one frame.
    let frame = encode(&execute(EngineActionWire::SessionList {
        after: Some("a".repeat(MAX_SESSION_CURSOR)),
    }))
    .unwrap();
    assert!(frame.len() < MAX_FRAME);
}

#[test]
fn only_a_serving_hello_offers_sessions_and_an_older_hello_omits_it() {
    let nonce = Uuid::from_u128(9);
    let hello = |stage, sessions| StageHello {
        version: VERSION,
        stage,
        guest_nonce: Uuid::from_u128(5),
        accepted_boot_nonce: (stage == Stage::Serve).then_some(nonce),
        run_cleanup: false,
        sessions,
    };
    assert!(validate_hello(&hello(Stage::Serve, true), Stage::Serve, nonce).is_ok());
    assert_eq!(
        validate_hello(&hello(Stage::Early, true), Stage::Early, nonce),
        Err(ProtocolError::Stage)
    );
    // An older guest's hello has no `sessions` key and reads as not offering them.
    let mut old = serde_json::to_value(hello(Stage::Serve, false)).unwrap();
    assert!(old.get("sessions").is_none());
    old.as_object_mut().unwrap().remove("runCleanup");
    assert!(!serde_json::from_value::<StageHello>(old).unwrap().sessions);
}

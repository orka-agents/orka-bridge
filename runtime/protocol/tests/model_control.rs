use bridge_runtime_protocol::*;
#[test]
fn native_model_selection_rejects_kmx_preflight_errors_before_delivery() {
    for provider in ["", "UPPER", "with.dot", "-start", "end-", &"x".repeat(46)] {
        assert!(!ModelSelection {
            provider: provider.into(),
            model: "gpt-4.1".into()
        }
        .valid());
    }
    for model in [
        "",
        "auto",
        "AUTO",
        "white space",
        "line\n",
        &"x".repeat(257),
    ] {
        assert!(!ModelSelection {
            provider: "copilot".into(),
            model: model.into()
        }
        .valid());
    }
    assert!(ModelSelection {
        provider: "copilot-1".into(),
        model: "gpt-4.1".into()
    }
    .valid());
}
#[test]
fn model_support_can_be_checked_without_loading_or_sending_a_secret() {
    let j = serde_json::json!({"type":"request","body":{"version":VERSION,"bootNonce":Uuid::new_v4(),"requestId":3,"body":{"op":"modelStatus","body":{}}}});
    assert!(
        serde_json::from_value::<Frame>(j).is_ok(),
        "model capability must precede secret access"
    );
}

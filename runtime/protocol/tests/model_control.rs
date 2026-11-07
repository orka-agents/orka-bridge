use bridge_runtime_protocol::*;
#[test]
fn model_support_can_be_checked_without_loading_or_sending_a_secret() {
    let j = serde_json::json!({"type":"request","body":{"version":VERSION,"bootNonce":Uuid::new_v4(),"requestId":3,"body":{"op":"modelStatus","body":{}}}});
    assert!(
        serde_json::from_value::<Frame>(j).is_ok(),
        "model capability must precede secret access"
    );
}

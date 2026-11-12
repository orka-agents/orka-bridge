use super::*;
use crate::clusters::{ContextId, TargetBinding};
use crate::send_summary::{AttemptSummary, AttemptVerdict};
use bridge_runtime_protocol::{SnapshotId, Uuid};

fn binding() -> TargetBinding {
    TargetBinding {
        context_id: ContextId(Uuid::new_v4()),
        context_name: "team".into(),
        server_address: "https://cluster.invalid".into(),
        service_name: "orka".into(),
        service_namespace: "control".into(),
        service_uid: "service".into(),
        deployment_name: "controller".into(),
        deployment_namespace: "control".into(),
        deployment_uid: "deployment".into(),
        watched_namespace: "agents".into(),
        image: "image".into(),
        api_port: 8080,
    }
}
fn record() -> AttemptSummary {
    serde_json::from_value(serde_json::json!({"id":Uuid::new_v4().to_string(),"source":SnapshotId::of(b"agent",b"tests"),"targetDisplay":"team · agents","targetUids":{"serviceNamespace":"control","serviceUid":"service","deploymentNamespace":"control","deploymentUid":"deployment","watchedNamespace":"agents"},"provider":"model","plannedAgentName":"same-name","agentUid":"one","verdict":"passed","time":1})).unwrap()
}
fn place() -> Place {
    let mut p = Place::new(binding());
    p.stale = false;
    p.observed_at = Some(1);
    p.agents = vec![
        agent(
            &serde_json::json!({"metadata":{"name":"same-name","namespace":"agents","uid":"one"}}),
            "agents",
        )
        .unwrap(),
        agent(
            &serde_json::json!({"metadata":{"name":"same-name","namespace":"agents","uid":"two"}}),
            "agents",
        )
        .unwrap(),
    ];
    p
}
#[test]
fn recorded_target_and_uid_only_never_name_or_unacknowledged() {
    let summary = record();
    let mut unknown = record();
    unknown.agent_uid = None;
    unknown.verdict = AttemptVerdict::Unknown;
    let result = project(vec![place()], Some(vec![unknown.clone(), summary.clone()]));
    assert!(result.places[0].agents[0].record.is_some());
    assert!(result.places[0].agents[1].record.is_none());
    assert_eq!(result.unobserved.len(), 1);
    assert!(result.unobserved[0].agent_uid.is_none());
    let mut other = place();
    other.binding.deployment_uid = "replacement".into();
    assert!(
        project(vec![other], Some(vec![summary.clone()])).places[0].agents[0]
            .record
            .is_none()
    );
    let mut other = place();
    other.binding.context_name = "other".into();
    assert!(
        project(vec![other], Some(vec![summary])).places[0].agents[0]
            .record
            .is_none()
    );
}
#[test]
fn every_version_joins_independently_and_history_error_never_means_not_tested() {
    let first = record();
    let mut second = record();
    second.agent_uid = Some("two".into());
    second.verdict = AttemptVerdict::Unknown;
    let result = project(vec![place()], Some(vec![second, first]));
    assert_eq!(
        result.places[0].agents[0].record.as_ref().unwrap().verdict,
        AttemptVerdict::Passed
    );
    assert_eq!(
        result.places[0].agents[1].record.as_ref().unwrap().verdict,
        AttemptVerdict::Unknown
    );
    assert_eq!(project(vec![place()], None).history_state, "unavailable");
}
#[test]
fn optional_runtime_facts_are_not_inferred_and_required_scope_is_checked() {
    let value = serde_json::json!({"metadata":{"name":"agent","namespace":"agents","uid":"uid"},"spec":{"type":"runtime","model":{"name":"model"},"tools":[{"name":"web_fetch"}]}});
    let a = agent(&value, "agents").unwrap();
    assert_eq!(a.agent_type.as_deref(), Some("runtime"));
    assert_eq!(a.model.as_deref(), Some("model"));
    assert_eq!(a.tools, Some(vec!["web_fetch".into()]));
    assert!(agent(&value, "other").is_err());
    let mut invalid = value.clone();
    invalid["spec"]["model"] = serde_json::json!(42);
    invalid["spec"]["tools"] = serde_json::json!([{"url":"not-a-tool-name"}]);
    let a = agent(&invalid, "agents").unwrap();
    assert!(a.model.is_none());
    assert!(a.tools.is_none());
    invalid["metadata"]["uid"] = serde_json::Value::Null;
    assert!(agent(&invalid, "agents").is_err());
}
#[test]
fn place_key_ignores_ephemeral_handles_and_image_but_not_installation_or_namespace() {
    let a = binding();
    let mut b = a.clone();
    b.context_id = ContextId(Uuid::new_v4());
    b.image = "upgraded".into();
    assert_eq!(Place::new(a.clone()).key, Place::new(b.clone()).key);
    b.watched_namespace = "different".into();
    assert_ne!(Place::new(a).key, Place::new(b).key);
}

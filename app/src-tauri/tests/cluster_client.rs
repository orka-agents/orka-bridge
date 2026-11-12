#[path = "support/exec_credential_fixture.rs"]
mod credential_fixture;
#[path = "support/cluster_tls_fixture.rs"]
mod tls_fixture;
use bridge_engine::clusters::*;
use kube::config::Kubeconfig;
use serde_json::{json, Value};
use std::{
    future::Future,
    io::{Read, Write},
    net::TcpListener,
    pin::Pin,
    sync::{Arc, Mutex},
};

#[path = "support/cluster_forward.rs"]
mod forward;

#[tokio::test]
async fn command_use_cluster_no_agent_task_post() {
    use bridge_engine::clusters::commands::ClusterSession;
    let api = Arc::new(ApiFixture::default());
    let client = ClusterClient::new(
        config(&api.start(), json!({"token":"creator-fixture"})),
        Arc::new(PodOrkaRead),
    );
    let session = ClusterSession::new(client);
    let contexts = session.contexts().await.unwrap();
    assert!(api.calls.lock().unwrap().is_empty());
    let id = serde_json::to_value(&contexts[0]).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let found = session.discover(&id).await.unwrap();
    assert!(api.calls.lock().unwrap().iter().all(|c| c.0 == "GET"));
    let target = serde_json::to_value(&found[0]).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let reply = session.use_target(&target, None).await;
    let value = serde_json::to_value(reply).unwrap();
    assert_eq!(value["status"], "connected");
    assert_eq!(value["providers"][0]["name"], "model");
    assert!(!value.to_string().contains("creator-fixture"));
    assert!(!value.to_string().contains("http://"));
    assert!(!api
        .calls
        .lock()
        .unwrap()
        .iter()
        .any(|c| c.1.ends_with("/token")));
    assert_eq!(api.wire.posts(), 0);
}

#[tokio::test]
async fn command_no_tokenrequest_before_use_and_conditional_account() {
    use bridge_engine::clusters::commands::ClusterSession;
    let api = Arc::new(ApiFixture::default());
    let orka = Arc::new(OrkaFixture::default());
    *orka.identity.lock().unwrap() =
        json!({"authenticated":true,"authType":"tokenReview","namespace":""});
    let session = ClusterSession::new(client(&api, &orka, json!({"token":"static-fixture"})));
    let contexts = session.contexts().await.unwrap();
    let id = serde_json::to_value(&contexts[0]).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let found = session.discover(&id).await.unwrap();
    assert!(!api
        .calls
        .lock()
        .unwrap()
        .iter()
        .any(|c| c.1.ends_with("/token")));
    let target = serde_json::to_value(&found[0]).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(
        serde_json::to_value(session.use_target(&target, None).await).unwrap()["status"],
        "needServiceAccount"
    );
    assert!(!api
        .calls
        .lock()
        .unwrap()
        .iter()
        .any(|c| c.1.ends_with("/token")));
    assert_eq!(
        serde_json::to_value(session.use_target(&target, Some("existing".into())).await).unwrap()
            ["status"],
        "connected"
    );
    assert_eq!(
        api.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|c| c.1.ends_with("/token"))
            .count(),
        1
    );
}

#[tokio::test]
async fn command_unknown_ids_never_reach_cluster() {
    use bridge_engine::clusters::commands::ClusterSession;
    let api = Arc::new(ApiFixture::default());
    let session = ClusterSession::new(client(
        &api,
        &Arc::new(OrkaFixture::default()),
        json!({"token":"static-fixture"}),
    ));
    for id in [
        "https://example.com",
        "../kubeconfig",
        "00000000-0000-4000-8000-000000000001",
    ] {
        assert!(session.discover(id).await.is_err());
        assert_eq!(
            serde_json::to_value(session.use_target(id, Some("existing".into())).await).unwrap()
                ["status"],
            "unavailable"
        );
    }
    assert!(api.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn command_unsupported_discovery_remains_distinct() {
    use bridge_engine::clusters::commands::ClusterSession;
    let api = Arc::new(ApiFixture::default());
    let session = ClusterSession::new(client(
        &api,
        &Arc::new(OrkaFixture::default()),
        json!({"token":"static-fixture"}),
    ));
    let contexts = session.contexts().await.unwrap();
    let id = serde_json::to_value(&contexts[1]).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let error = session.discover(&id).await.unwrap_err();
    assert_eq!(
        serde_json::to_value(error).unwrap(),
        json!({"check":"credentials","result":"unsupportedCredential"})
    );
    assert!(api.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn command_use_future_is_send_for_desktop_dispatch() {
    use bridge_engine::clusters::commands::ClusterSession;
    let api = Arc::new(ApiFixture::default());
    let session = ClusterSession::new(client(
        &api,
        &Arc::new(OrkaFixture::default()),
        json!({"token":"static-fixture"}),
    ));
    tokio::spawn(async move { session.use_target("unknown", None).await })
        .await
        .unwrap();
}

// Only the mandatory fields Bridge sends/reads; no equal-image/version requirement.
fn send_schema(resource: &str) -> Value {
    match resource {
        "agents" => json!({"type":"object","properties":{
            "spec":{"type":"object","properties":{
                "model":{"type":"object","properties":{"name":{"type":"string"}}},
                "providerRef":{"type":"object","properties":{"name":{"type":"string"}}},
                "systemPrompt":{"type":"object","properties":{"inline":{"type":"string"}}},
                "tools":{"type":"array","items":{"type":"object","properties":{"name":{"type":"string"}}}}
            }}
        }}),
        "tasks" => json!({"type":"object","properties":{
            "spec":{"type":"object","properties":{
                "type":{"type":"string","enum":["container","ai","agent"]},
                "agentRef":{"type":"object","properties":{"name":{"type":"string"}}},
                "prompt":{"type":"string"},"timeout":{"type":"string"},
                "retryPolicy":{"type":"object","properties":{"maxRetries":{"type":"integer"}}}
            }}
        }}),
        "providers" => {
            json!({"type":"object","properties":{"status":{"type":"object","properties":{"ready":{"type":"boolean"}}}}})
        }
        "tools" => json!({"type":"object","properties":{"metadata":{"type":"object"}}}),
        _ => panic!("unexpected resource"),
    }
}

type ImpersonatedRequest = (String, Vec<(String, String)>);
type GateStep = (String, std::sync::mpsc::Sender<()>);
#[derive(Default)]
struct ApiFixture {
    wire: forward::Wire,
    calls: Mutex<Vec<(String, String, String, Value)>>,
    impersonation: Mutex<Vec<ImpersonatedRequest>>,
    deny: Mutex<bool>,
    replacement: Mutex<bool>,
    no_watch: Mutex<bool>,
    unrelated_api: Mutex<bool>,
    override_response: Mutex<Option<(String, u16, Value)>>,
    // Hold an individual check open to exercise lock queuing and deadline labels.
    gate: Mutex<Option<std::sync::mpsc::Receiver<()>>>,
    step_gate: Mutex<Option<std::sync::mpsc::Sender<GateStep>>>,
}
impl ApiFixture {
    fn start(self: &Arc<Self>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        let fixture = self.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let fixture = fixture.clone();
                std::thread::spawn(move || {
                    let mut stream = stream.unwrap();
                    stream
                        .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                        .unwrap();
                    let mut bytes = Vec::new();
                    let mut buf = [0; 4096];
                    let header_end = loop {
                        let n = stream.read(&mut buf).unwrap();
                        if n == 0 {
                            return;
                        }
                        bytes.extend_from_slice(&buf[..n]);
                        if let Some(i) = bytes.windows(4).position(|s| s == b"\r\n\r\n") {
                            break i + 4;
                        }
                    };
                    let headers = String::from_utf8(bytes[..header_end].to_vec()).unwrap();
                    let mut first = headers.lines().next().unwrap().split_whitespace();
                    let method = first.next().unwrap().to_string();
                    let path = first.next().unwrap().to_string();
                    let header = |name: &str| {
                        headers.lines().find_map(|line| {
                            let (key, value) = line.split_once(':')?;
                            key.eq_ignore_ascii_case(name)
                                .then(|| value.trim().to_string())
                        })
                    };
                    fixture.impersonation.lock().unwrap().push((
                        path.split('?').next().unwrap().to_owned(),
                        headers
                            .lines()
                            .filter_map(|line| {
                                let (key, value) = line.split_once(':')?;
                                key.to_ascii_lowercase()
                                    .starts_with("impersonate-")
                                    .then(|| (key.to_ascii_lowercase(), value.trim().to_owned()))
                            })
                            .collect(),
                    ));
                    if path.contains("/portforward") {
                        fixture.wire.serve(stream, &headers);
                        return;
                    }
                    let len = header("content-length")
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(0);
                    while bytes.len() < header_end + len {
                        let n = stream.read(&mut buf).unwrap();
                        bytes.extend_from_slice(&buf[..n]);
                    }
                    let body = serde_json::from_slice(&bytes[header_end..]).unwrap_or(Value::Null);
                    fixture.calls.lock().unwrap().push((
                        method,
                        path.split('?').next().unwrap().into(),
                        header("authorization").unwrap_or_default(),
                        body,
                    ));
                    let (status, response) = fixture.response(path.split('?').next().unwrap());
                    let body = response.to_string();
                    let reply = write!(stream, "HTTP/1.1 {status} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                    // A deadline test deliberately drops the gated request before its reply.
                    assert!(
                        reply.is_ok()
                            || reply.err().is_some_and(|e| matches!(
                                e.kind(),
                                std::io::ErrorKind::BrokenPipe
                                    | std::io::ErrorKind::ConnectionReset
                            ))
                    );
                });
            }
        });
        address
    }
    fn response(&self, path: &str) -> (u16, Value) {
        if let Some(gate) = self.step_gate.lock().unwrap().as_ref() {
            let (release, wait) = std::sync::mpsc::channel();
            if gate.send((path.into(), release)).is_ok() {
                let _ = wait.recv();
            }
        }
        if let Some((suffix, status, body)) = self.override_response.lock().unwrap().as_ref() {
            if path.ends_with(suffix) {
                return (*status, body.clone());
            }
        }
        if path.ends_with("/token") {
            return (
                200,
                json!({"apiVersion":"authentication.k8s.io/v1","kind":"TokenRequest", "spec":{"audiences":[]}, "status":{"token":"issued-fixture","expirationTimestamp":"2099-01-01T00:00:00Z"}}),
            );
        }
        if path.ends_with("/selfsubjectaccessreviews") {
            return (
                200,
                json!({"apiVersion":"authorization.k8s.io/v1","kind":"SelfSubjectAccessReview","spec":{},"status":{"allowed": !*self.deny.lock().unwrap()}}),
            );
        }
        if path.contains("/customresourcedefinitions/") {
            let name = path.rsplit('/').next().unwrap();
            let plural = name.split('.').next().unwrap();
            return (
                200,
                json!({"apiVersion":"apiextensions.k8s.io/v1","kind":"CustomResourceDefinition","metadata":{"name":name},"spec":{"group":"core.orka.ai","scope":"Namespaced","names":{"kind":"Fixture","plural":plural},"versions":[{"name":"v1alpha1","served":true,"storage":true,"schema":{"openAPIV3Schema":send_schema(plural)}}]}}),
            );
        }
        if path.ends_with("/pods") {
            return (
                200,
                json!({"apiVersion":"v1","kind":"PodList","metadata":{},"items":[
                    {"metadata":{"name":"not-ready","namespace":"control","uid":"bad","labels":{"app":"orka"}},"spec":{"containers":[]},"status":{"phase":"Running","conditions":[{"type":"Ready","status":"False"}]}},
                    {"metadata":{"name":"ready","namespace":"control","uid":"pod-uid","labels":{"app":"orka"}},"spec":{"containers":[{"name":"controller","ports":[{"name":"api","containerPort":8080}]}]},"status":{"phase":"Running","conditions":[{"type":"Ready","status":"True"}]}}
                ]}),
            );
        }
        if path.ends_with("/services") {
            if let Some(gate) = self.gate.lock().unwrap().take() {
                let _ = gate.recv();
            }
            let uid = if *self.replacement.lock().unwrap() {
                "replacement"
            } else {
                "service-uid"
            };
            let mut list = json!({"apiVersion":"v1","kind":"ServiceList","metadata":{},"items":[{"metadata":{"name":"orka","namespace":"control","uid":uid},"spec":{"selector":{"app":"orka"},"ports":[{"name":"api","port":80,"targetPort":"api"}]}}]});
            if *self.unrelated_api.lock().unwrap() {
                list["items"].as_array_mut().unwrap().push(json!({"metadata":{"name":"other","namespace":"control","uid":"other-service"},"spec":{"selector":{"app":"other"},"ports":[{"name":"api","port":80,"targetPort":9000}]}}));
            }
            return (200, list);
        }
        if path.ends_with("/deployments") {
            let args = if *self.no_watch.lock().unwrap() {
                json!(["--api-port=8080"])
            } else {
                json!(["--api-port=8080", "--watch-namespace=agents"])
            };
            let mut list = json!({"apiVersion":"apps/v1","kind":"DeploymentList","metadata":{},"items":[{"metadata":{"name":"controller","namespace":"control","uid":"deployment-uid"},"spec":{"selector":{"matchLabels":{"app":"orka"}},"template":{"metadata":{"labels":{"app":"orka"}},"spec":{"containers":[{"name":"controller","image":"ghcr.io/orka-agents/orka:v99","args":args,"ports":[{"name":"api","containerPort":8080}]}]}}}}]});
            if *self.unrelated_api.lock().unwrap() {
                list["items"][0]["spec"]["template"]["metadata"]["labels"]
                    ["app.kubernetes.io/name"] = json!("orka");
                list["items"][0]["spec"]["template"]["spec"]["containers"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!({"name":"sidecar","image":"example/sidecar:v1"}));
                list["items"].as_array_mut().unwrap().push(json!({"metadata":{"name":"other","namespace":"control","uid":"other-deployment"},"spec":{"selector":{"matchLabels":{"app":"other"}},"template":{"metadata":{"labels":{"app":"other"}},"spec":{"containers":[{"name":"server","image":"example/api:v1","args":["--api-port=9000","--watch-namespace=agents"]}]}}}}));
            }
            return (200, list);
        }
        panic!("unexpected Kubernetes path {path}");
    }
}

type OrkaCall = (OrkaResource, String, String, Option<String>);
struct OrkaFixture {
    calls: Mutex<Vec<OrkaCall>>,
    providers: Mutex<Option<Value>>,
    agents: Mutex<Option<Value>>,
    identity: Mutex<Value>,
    fail: Mutex<Option<AccessOutcome>>,
    fail_at: Mutex<Option<(OrkaResource, AccessOutcome)>>,
    // Endless Provider pages, each slower than a third of the operation budget.
    slow_pages: Mutex<bool>,
    echo: Mutex<bool>,
    paged: Mutex<bool>,
}
impl Default for OrkaFixture {
    fn default() -> Self {
        Self {
            calls: Mutex::default(),
            providers: Mutex::new(None),
            agents: Mutex::new(None),
            identity: Mutex::new(
                json!({"authenticated":true,"authType":"tokenReview","namespace":"agents"}),
            ),
            fail: Mutex::new(None),
            fail_at: Mutex::new(None),
            slow_pages: Mutex::new(false),
            echo: Mutex::new(false),
            paged: Mutex::new(false),
        }
    }
}
impl OrkaRead for OrkaFixture {
    fn get<'a>(
        &'a self,
        _: &'a kube::Client,
        _: &'a TargetBinding,
        bearer: &'a str,
        resource: OrkaResource,
        namespace: &'a str,
        after: Option<&'a str>,
    ) -> Pin<Box<dyn Future<Output = Result<Value, AccessOutcome>> + Send + 'a>> {
        Box::pin(async move {
            self.calls.lock().unwrap().push((
                resource,
                bearer.into(),
                namespace.into(),
                after.map(str::to_string),
            ));
            if let Some(error) = *self.fail.lock().unwrap() {
                return Err(error);
            }
            if let Some((at, error)) = *self.fail_at.lock().unwrap() {
                if at == resource {
                    return Err(error);
                }
            }
            if resource == OrkaResource::Providers && *self.slow_pages.lock().unwrap() {
                tokio::time::sleep(std::time::Duration::from_secs(9)).await;
                let next = format!("page-{}", self.calls.lock().unwrap().len());
                return Ok(json!({"items":[],"metadata":{"continue":next}}));
            }
            if *self.echo.lock().unwrap() {
                return Ok(json!({"echo":bearer}));
            }
            Ok(match resource {
                OrkaResource::WhoAmI if bearer == "issued-fixture" => json!({"authenticated":true,"authType":"tokenReview","namespace":"agents"}),
                OrkaResource::WhoAmI => self.identity.lock().unwrap().clone(),
                OrkaResource::Providers => self.providers.lock().unwrap().clone().unwrap_or_else(|| {
                    json!({"items":[{"apiVersion":"core.orka.ai/v1alpha1","kind":"Provider","metadata":{"name":"model","namespace":"agents"},"spec":{"type":"openai","defaultModel":"fixture-model"},"status":{"ready":true}}],"metadata":{}})
                }),
                OrkaResource::Tools if *self.paged.lock().unwrap() && after.is_none() => {
                    json!({"items":[{"name":"custom","builtin":false}],"metadata":{"continue":"next"}})
                }
                OrkaResource::Tools => {
                    json!({"items":[{"name":"web_fetch","builtin":true}],"metadata":{}})
                }
                OrkaResource::Agents => self.agents.lock().unwrap().clone().unwrap_or_else(|| json!({"items":[],"metadata":{}})),
            })
        })
    }
}
fn failure(check: Check, result: CheckResult) -> ClusterFailure {
    ClusterFailure { check, result }
}
fn failed(check: Check, result: CheckResult) -> AccessOutcome {
    AccessOutcome::Failed(failure(check, result))
}
fn config(server: &str, auth: Value) -> Kubeconfig {
    serde_json::from_value(json!({"apiVersion":"v1","kind":"Config","current-context":"wrong","clusters":[{"name":"fixture","cluster":{"server":server}}],"users":[{"name":"user","user":auth}],"contexts":[{"name":"selected","context":{"cluster":"fixture","user":"user"}},{"name":"exec","context":{"cluster":"fixture","user":"helper"}}]})).unwrap()
}
fn client(api: &Arc<ApiFixture>, orka: &Arc<OrkaFixture>, auth: Value) -> ClusterClient {
    ClusterClient::new(config(&api.start(), auth), orka.clone())
}
async fn target(client: &mut ClusterClient) -> TargetId {
    let context = client.contexts().unwrap()[0].id;
    client.discover(context).await.unwrap()[0].id
}

#[tokio::test]
async fn creator_bearer_echo_after_serviceaccount_fallback_is_not_exported() {
    let api = Arc::new(ApiFixture::default());
    let orka = Arc::new(OrkaFixture::default());
    *orka.identity.lock().unwrap() =
        json!({"authenticated":true,"authType":"tokenReview","namespace":""});
    *orka.providers.lock().unwrap() = Some(
        json!({"items":[{"metadata":{"name":"creator-fixture","namespace":"agents"},"status":{"ready":true}}],"metadata":{"continue":null}}),
    );
    let mut client = client(&api, &orka, json!({"token":"creator-fixture"}));
    let id = target(&mut client).await;
    assert_eq!(
        client.connect(id, Some("existing".into())).await.err(),
        Some(failed(Check::Providers, CheckResult::Unknown))
    );
    let calls = api.calls.lock().unwrap();
    assert_eq!(calls.iter().filter(|c| c.1.ends_with("/token")).count(), 1);
    assert!(calls.iter().all(|c| c.0 == "GET"
        || c.1.ends_with("/token")
        || c.1.ends_with("/selfsubjectaccessreviews")));
}

#[tokio::test]
async fn omitted_provider_readiness_is_false_beside_a_ready_provider() {
    let api = Arc::new(ApiFixture::default());
    let orka = Arc::new(OrkaFixture::default());
    *orka.providers.lock().unwrap() = Some(json!({"items":[
        {"metadata":{"name":"ready","namespace":"agents"},"status":{"ready":true}},
        {"metadata":{"name":"pending","namespace":"agents"},"status":{}}
    ],"metadata":{"continue":null}}));
    let mut client = client(&api, &orka, json!({"token":"static-fixture"}));
    let id = target(&mut client).await;
    let access = client.connect(id, None).await.unwrap();
    assert_eq!(
        access
            .providers()
            .iter()
            .map(|p| (p.name.as_str(), p.ready))
            .collect::<Vec<_>>(),
        [("ready", true), ("pending", false)]
    );
    *orka.providers.lock().unwrap() = Some(
        json!({"items":[{"metadata":{"name":"invalid","namespace":"agents"},"status":{"ready":"false"}}],"metadata":{"continue":null}}),
    );
    assert_eq!(
        client.connect(id, None).await.err(),
        Some(failed(Check::Providers, CheckResult::Unknown))
    );
}

#[tokio::test]
async fn contexts_are_offline_and_explicit_discovery_only_gets() {
    let api = Arc::new(ApiFixture::default());
    let orka = Arc::new(OrkaFixture::default());
    let mut client = client(&api, &orka, json!({"token":"static-fixture"}));
    let views = client.contexts().unwrap();
    assert_eq!(
        views.iter().map(|v| v.name.as_str()).collect::<Vec<_>>(),
        ["selected", "exec"]
    );
    assert!(api.calls.lock().unwrap().is_empty());
    let found = client.discover(views[0].id).await.unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].watched_namespace, "agents");
    assert_eq!(found[0].service_namespace, "control");
    assert_eq!(found[0].image, "ghcr.io/orka-agents/orka:v99");
    assert!(api.calls.lock().unwrap().iter().all(|c| c.0 == "GET"));
    assert!(orka.calls.lock().unwrap().is_empty());
    let serialized = serde_json::to_string(&found).unwrap();
    assert!(!serialized.contains("static-fixture"));
    assert!(!serialized.contains("http://"));
}
#[tokio::test]
async fn unrelated_api_workload_does_not_hide_orka_or_break_revalidation() {
    let api = Arc::new(ApiFixture::default());
    *api.unrelated_api.lock().unwrap() = true;
    let orka = Arc::new(OrkaFixture::default());
    let mut client = client(&api, &orka, json!({"token":"static-fixture"}));
    let views = client
        .discover(client.contexts().unwrap()[0].id)
        .await
        .unwrap();
    assert_eq!(views.len(), 1);
    assert_eq!(views[0].service_name, "orka");
    let access = client.connect(views[0].id, None).await.unwrap();
    client.revalidate(&access).await.unwrap();
    assert!(api
        .calls
        .lock()
        .unwrap()
        .iter()
        .all(|c| c.0 == "GET" || c.1.ends_with("/selfsubjectaccessreviews")));
}

#[tokio::test]
async fn missing_watch_namespace_is_not_guessed() {
    let api = Arc::new(ApiFixture::default());
    *api.no_watch.lock().unwrap() = true;
    let mut client = client(
        &api,
        &Arc::new(OrkaFixture::default()),
        json!({"token":"static-fixture"}),
    );
    let id = client.contexts().unwrap()[0].id;
    assert_eq!(
        client.discover(id).await.err(),
        Some(failure(Check::Compatibility, CheckResult::Incompatible))
    );
}
#[tokio::test]
async fn static_namespace_bearer_no_tokenrequest_and_same_bearer_permission_check() {
    let api = Arc::new(ApiFixture::default());
    let orka = Arc::new(OrkaFixture::default());
    let mut client = client(
        &api,
        &orka,
        json!({"token":"static-fixture","exec":{"command":"must-not-execute"}}),
    );
    let id = target(&mut client).await;
    let access = client.connect(id, None).await.unwrap();
    let providers = access.providers();
    assert_eq!(providers.len(), 1);
    assert_eq!(providers[0].name, "model");
    assert!(providers[0].ready);
    access
        .require_builtins(&["web_fetch".into()])
        .await
        .unwrap();
    let calls = api.calls.lock().unwrap();
    assert!(!calls.iter().any(|c| c.1.ends_with("/token")));
    let reviews: Vec<_> = calls
        .iter()
        .filter(|c| c.1.ends_with("/selfsubjectaccessreviews"))
        .collect();
    assert_eq!(reviews.len(), 10);
    assert!(reviews.iter().all(|c| c.2 == "Bearer static-fixture"));
    assert!(reviews.iter().any(
        |c| c.3["spec"]["resourceAttributes"]["resource"] == "agents"
            && c.3["spec"]["resourceAttributes"]["verb"] == "create"
    ));
}
#[tokio::test]
async fn oidc_no_sar_and_denial_does_not_substitute_identity() {
    let api = Arc::new(ApiFixture::default());
    let orka = Arc::new(OrkaFixture::default());
    *orka.identity.lock().unwrap() =
        json!({"authenticated":true,"authType":"oidc","namespace":"agents"});
    let mut client = client(&api, &orka, json!({"token":"static-fixture"}));
    let id = target(&mut client).await;
    client.connect(id, None).await.unwrap();
    assert!(api.calls.lock().unwrap().iter().all(|c| c.0 == "GET"));
    let rejected = failed(Check::OrkaIdentity, CheckResult::AuthenticationRejected);
    *orka.fail.lock().unwrap() = Some(rejected);
    assert_eq!(
        client.connect(id, Some("existing".into())).await.err(),
        Some(rejected)
    );
    assert!(!api
        .calls
        .lock()
        .unwrap()
        .iter()
        .any(|c| c.1.ends_with("/token")));
}
#[tokio::test]
async fn namespace_less_tokenreview_requests_named_existing_sa_only_on_connect() {
    let api = Arc::new(ApiFixture::default());
    let orka = Arc::new(OrkaFixture::default());
    *orka.identity.lock().unwrap() =
        json!({"authenticated":true,"authType":"tokenReview","namespace":""});
    let mut client = client(&api, &orka, json!({"token":"static-fixture"}));
    let id = target(&mut client).await;
    assert_eq!(
        client.connect(id, None).await.err(),
        Some(AccessOutcome::NeedServiceAccount)
    );
    assert!(!api
        .calls
        .lock()
        .unwrap()
        .iter()
        .any(|c| c.1.ends_with("/token")));
    *orka.identity.lock().unwrap() =
        json!({"authenticated":true,"authType":"tokenReview","namespace":"agents"});
    // A namespaced creator no longer needs substitution, even if the caller supplied a name.
    client.connect(id, Some("existing".into())).await.unwrap();
    assert!(!api
        .calls
        .lock()
        .unwrap()
        .iter()
        .any(|c| c.1.ends_with("/token")));
}
#[tokio::test]
async fn sa_name_not_uri_no_request() {
    let api = Arc::new(ApiFixture::default());
    let orka = Arc::new(OrkaFixture::default());
    let mut client = client(&api, &orka, json!({"token":"static-fixture"}));
    let id = target(&mut client).await;
    let before = api.calls.lock().unwrap().len();
    for bad in [
        "https://host/token",
        "agents/name",
        " name",
        "name:other",
        "",
        "a..b",
        "Bad",
        "-name",
    ] {
        assert_eq!(
            client.connect(id, Some(bad.into())).await.err(),
            Some(failed(Check::ServiceAccount, CheckResult::Unknown))
        );
    }
    assert_eq!(api.calls.lock().unwrap().len(), before);
    assert!(orka.calls.lock().unwrap().is_empty());
}
#[tokio::test]
async fn denied_create_reports_denied_no_product_post() {
    let api = Arc::new(ApiFixture::default());
    *api.deny.lock().unwrap() = true;
    let mut client = client(
        &api,
        &Arc::new(OrkaFixture::default()),
        json!({"token":"static-fixture"}),
    );
    let id = target(&mut client).await;
    assert_eq!(
        client.connect(id, None).await.err(),
        Some(failed(Check::Permissions, CheckResult::PermissionDenied))
    );
    assert!(api
        .calls
        .lock()
        .unwrap()
        .iter()
        .all(|c| c.0 == "GET" || c.1.ends_with("/selfsubjectaccessreviews")));
}
#[tokio::test]
async fn effective_exec_visible_but_unavailable_and_credential_echo_not_exported() {
    let api = Arc::new(ApiFixture::default());
    let orka = Arc::new(OrkaFixture::default());
    let mut client = client(&api, &orka, json!({"exec":{"command":"must-not-execute"}}));
    let id = client.contexts().unwrap()[0].id;
    assert_eq!(
        client.discover(id).await.err(),
        Some(failure(
            Check::Credentials,
            CheckResult::UnsupportedCredential
        ))
    );
    assert!(api.calls.lock().unwrap().is_empty());
    let mut client = ClusterClient::new(
        config(&api.start(), json!({"token":"static-fixture"})),
        orka.clone(),
    );
    let id = target(&mut client).await;
    *orka.echo.lock().unwrap() = true;
    let result = client.connect(id, None).await;
    assert_eq!(
        result.err(),
        Some(failed(Check::OrkaIdentity, CheckResult::Unknown))
    );
}
#[tokio::test]
async fn exec_creator_preserves_impersonation_but_ssar_is_token_only() {
    let api = Arc::new(ApiFixture::default());
    let version = "client.authentication.k8s.io/v1";
    let mut auth = credential_fixture::exec(version, credential_fixture::response(version));
    auth["as"] = json!("restricted-user");
    auth["as-uid"] = json!("restricted-uid");
    auth["as-groups"] = json!(["restricted-group"]);
    auth["as-user-extra"] = json!({"purpose":["fixture"]});
    let mut client = client(&api, &Arc::new(OrkaFixture::default()), auth);
    let target = target(&mut client).await;
    client.connect(target, None).await.unwrap();
    let headers = api.impersonation.lock().unwrap();
    assert!(headers
        .iter()
        .any(|(path, _)| path.ends_with("/selfsubjectaccessreviews")));
    for (path, values) in headers.iter() {
        if path.ends_with("/selfsubjectaccessreviews") {
            assert!(
                values.is_empty(),
                "bearer-only SSAR carried creator impersonation"
            );
        } else {
            let mut values = values.clone();
            values.sort();
            assert_eq!(
                values,
                vec![
                    ("impersonate-extra-purpose".into(), "fixture".into()),
                    ("impersonate-group".into(), "restricted-group".into()),
                    ("impersonate-uid".into(), "restricted-uid".into()),
                    ("impersonate-user".into(), "restricted-user".into())
                ]
            );
        }
    }
}

#[tokio::test]
async fn exec_certificate_creator_uses_existing_account_without_export_or_tls_relaxation() {
    use base64::Engine;
    let api = Arc::new(ApiFixture::default());
    let tls = tls_fixture::TlsFixture::start(&api.start()).await;
    let version = "client.authentication.k8s.io/v1";
    let decode = |key: &str| {
        String::from_utf8(
            base64::engine::general_purpose::STANDARD
                .decode(tls.auth[key].as_str().unwrap())
                .unwrap(),
        )
        .unwrap()
    };
    let result = json!({"kind":"ExecCredential","apiVersion":version,"status":{"clientCertificateData":decode("client-certificate-data"),"clientKeyData":decode("client-key-data")}});
    let mut cfg = config(&tls.server, credential_fixture::exec(version, result));
    cfg.clusters[0]
        .cluster
        .as_mut()
        .unwrap()
        .certificate_authority_data = Some(tls.ca.clone());
    cfg.clusters[0].cluster.as_mut().unwrap().tls_server_name = Some("fixture-server".into());
    let mut client = ClusterClient::new(cfg, Arc::new(OrkaFixture::default()));
    let target = target(&mut client).await;
    assert_eq!(
        client.connect(target, None).await.err(),
        Some(AccessOutcome::NeedServiceAccount)
    );
    assert!(!api
        .calls
        .lock()
        .unwrap()
        .iter()
        .any(|c| c.1.ends_with("/token")));
    let access = client
        .connect(target, Some("existing".into()))
        .await
        .unwrap();
    assert!(access.expiry.is_some());
    let calls = api.calls.lock().unwrap();
    let peers = tls.peers.lock().unwrap();
    for (i, call) in calls.iter().enumerate() {
        if call.1.ends_with("/selfsubjectaccessreviews") {
            assert!(call.2 == "Bearer issued-fixture");
            assert!(!peers[i], "SSAR inherited the plugin certificate");
        }
    }
}

#[tokio::test]
async fn exec_mixed_credentials_preserve_creator_tls_but_ssar_uses_only_bearer() {
    use base64::Engine;
    for token in ["", "mixed-fixture"] {
        let api = Arc::new(ApiFixture::default());
        let tls = tls_fixture::TlsFixture::start(&api.start()).await;
        let version = "client.authentication.k8s.io/v1";
        let decode = |key: &str| {
            String::from_utf8(
                base64::engine::general_purpose::STANDARD
                    .decode(tls.auth[key].as_str().unwrap())
                    .unwrap(),
            )
            .unwrap()
        };
        let output = json!({"kind":"ExecCredential","apiVersion":version,"status":{"token":token,"clientCertificateData":decode("client-certificate-data"),"clientKeyData":decode("client-key-data")}});
        let mut cfg = config(&tls.server, credential_fixture::exec(version, output));
        cfg.clusters[0]
            .cluster
            .as_mut()
            .unwrap()
            .certificate_authority_data = Some(tls.ca.clone());
        cfg.clusters[0].cluster.as_mut().unwrap().tls_server_name = Some("fixture-server".into());
        let mut client = ClusterClient::new(cfg, Arc::new(OrkaFixture::default()));
        let target = target(&mut client).await;
        assert!(tls.peers.lock().unwrap().iter().all(|peer| *peer));
        if token.is_empty() {
            assert_eq!(
                client.connect(target, None).await.err(),
                Some(AccessOutcome::NeedServiceAccount)
            );
        } else {
            client.connect(target, None).await.unwrap();
            let calls = api.calls.lock().unwrap();
            assert!(!calls.iter().any(|c| c.1.ends_with("/token")));
            let peers = tls.peers.lock().unwrap();
            for (i, call) in calls.iter().enumerate() {
                assert!(call.2 == "Bearer mixed-fixture");
                assert_eq!(peers[i], !call.1.ends_with("/selfsubjectaccessreviews"));
            }
        }
    }
}

#[tokio::test]
async fn certificate_creator_existing_sa_and_issued_ssar_has_no_creator_tls_certificate() {
    let api = Arc::new(ApiFixture::default());
    let tls = tls_fixture::TlsFixture::start(&api.start()).await;
    let orka = Arc::new(OrkaFixture::default());
    let mut cfg = config(&tls.server, tls.auth.clone());
    let cluster = cfg.clusters[0].cluster.as_mut().unwrap();
    cluster.certificate_authority_data = Some(tls.ca.clone());
    cluster.tls_server_name = Some("fixture-server".into());
    let mut exec_cfg = cfg.clone();
    exec_cfg.auth_infos[0].auth_info.as_mut().unwrap().exec = Some(kube::config::ExecConfig {
        command: Some("must-not-execute".into()),
        ..Default::default()
    });
    let mut exec_client = ClusterClient::new(exec_cfg, orka.clone());
    let exec_id = exec_client.contexts().unwrap()[0].id;
    assert_eq!(
        exec_client.discover(exec_id).await.err(),
        Some(failure(
            Check::Credentials,
            CheckResult::UnsupportedCredential
        ))
    );
    assert!(api.calls.lock().unwrap().is_empty());
    let mut client = ClusterClient::new(cfg, orka.clone());
    let id = target(&mut client).await;
    assert!(tls.peers.lock().unwrap().iter().all(|peer| *peer));
    assert_eq!(
        client.connect(id, None).await.err(),
        Some(AccessOutcome::NeedServiceAccount)
    );
    assert!(!api
        .calls
        .lock()
        .unwrap()
        .iter()
        .any(|c| c.1.ends_with("/token")));
    let access = client
        .connect(id, Some("existing.account".into()))
        .await
        .unwrap();
    assert!(access.expiry.is_some());
    let calls = api.calls.lock().unwrap();
    let requests: Vec<_> = calls.iter().filter(|c| c.1.ends_with("/token")).collect();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].1,
        "/api/v1/namespaces/agents/serviceaccounts/existing.account/token"
    );
    assert_eq!(requests[0].3["spec"]["expirationSeconds"], 600);
    assert_eq!(requests[0].3["spec"]["audiences"], json!([]));
    assert!(requests[0].2.is_empty());
    let reviews: Vec<_> = calls
        .iter()
        .enumerate()
        .filter(|(_, c)| c.1.ends_with("/selfsubjectaccessreviews"))
        .collect();
    assert_eq!(reviews.len(), 10);
    for (i, call) in reviews {
        assert_eq!(call.2, "Bearer issued-fixture");
        assert!(
            !tls.peers.lock().unwrap()[i],
            "SSAR TLS request still used creator certificate"
        );
    }
    assert!(orka
        .calls
        .lock()
        .unwrap()
        .iter()
        .all(|c| c.1 == "issued-fixture"));
}
#[tokio::test]
async fn stronger_credentials_do_not_run_exec_or_auth_provider_helpers() {
    let api = Arc::new(ApiFixture::default());
    for auth in [
        json!({"client-certificate-data":"invalid","client-key-data":"invalid","exec":{"command":"must-not-execute"}}),
        json!({"token":"static-fixture","auth-provider":{"name":"gcp","config":{"cmd-path":"must-not-execute"}}}),
        json!({"token":"static-fixture","username":"basic","password":"fixture"}),
    ] {
        let mut client = client(&api, &Arc::new(OrkaFixture::default()), auth);
        let id = client.contexts().unwrap()[0].id;
        assert_eq!(
            client.discover(id).await.err(),
            Some(failure(
                Check::Credentials,
                CheckResult::UnsupportedCredential
            ))
        );
    }
    assert!(api.calls.lock().unwrap().is_empty());
}
#[tokio::test]
async fn namespace_less_bearer_issues_only_after_confirmed_name() {
    let api = Arc::new(ApiFixture::default());
    let orka = Arc::new(OrkaFixture::default());
    *orka.identity.lock().unwrap() =
        json!({"authenticated":true,"authType":"tokenReview","namespace":""});
    let mut client = client(&api, &orka, json!({"token":"static-fixture"}));
    let id = target(&mut client).await;
    assert_eq!(
        client.connect(id, None).await.err(),
        Some(AccessOutcome::NeedServiceAccount)
    );
    assert!(!api
        .calls
        .lock()
        .unwrap()
        .iter()
        .any(|c| c.1.ends_with("/token")));
    client.connect(id, Some("existing".into())).await.unwrap();
    let calls = api.calls.lock().unwrap();
    assert_eq!(calls.iter().filter(|c| c.1.ends_with("/token")).count(), 1);
    assert!(calls
        .iter()
        .filter(|c| c.1.ends_with("/selfsubjectaccessreviews"))
        .all(|c| c.2 == "Bearer issued-fixture"));
}
#[tokio::test]
async fn token_file_capture_is_frozen_and_unused_exec_never_runs() {
    use std::io::Write as _;
    let mut file = tempfile::NamedTempFile::new().unwrap();
    file.write_all(b"file-fixture").unwrap();
    let api = Arc::new(ApiFixture::default());
    let orka = Arc::new(OrkaFixture::default());
    let mut client = client(
        &api,
        &orka,
        json!({"tokenFile":file.path(),"exec":{"command":"must-not-execute"}}),
    );
    let id = target(&mut client).await;
    let access = client.connect(id, None).await.unwrap();
    std::fs::write(file.path(), b"replacement-fixture").unwrap();
    // kube5 refreshes token files after 50 seconds (60-second cache minus its 10-second margin).
    tokio::time::sleep(std::time::Duration::from_secs(51)).await;
    client.revalidate(&access).await.unwrap();
    assert!(api
        .calls
        .lock()
        .unwrap()
        .iter()
        .all(|c| c.2 == "Bearer file-fixture"));
    access
        .require_builtins(&["web_fetch".into()])
        .await
        .unwrap();
    assert!(orka
        .calls
        .lock()
        .unwrap()
        .iter()
        .all(|c| c.1 == "file-fixture"));
}
#[tokio::test]
async fn full_api_address_replacement_cannot_revalidate_existing_access() {
    let api = Arc::new(ApiFixture::default());
    let orka = Arc::new(OrkaFixture::default());
    let server = api.start();
    let cfg = config(&server, json!({"token":"static-fixture"}));
    let mut client = ClusterClient::new(cfg.clone(), orka.clone());
    let id = target(&mut client).await;
    let access = client.connect(id, None).await.unwrap();
    client.revalidate(&access).await.unwrap();
    let mut replacement = cfg;
    replacement.clusters[0].cluster.as_mut().unwrap().server =
        Some(format!("{server}/different-api-root"));
    let changed = ClusterClient::new(replacement, orka);
    let before = api.calls.lock().unwrap().len();
    assert_eq!(
        changed.revalidate(&access).await.err(),
        Some(ClusterError::Unavailable)
    );
    assert_eq!(api.calls.lock().unwrap().len(), before);
}
#[tokio::test]
async fn ssar_http_403_is_denied_and_tokenrequest_503_is_not_retried() {
    let api = Arc::new(ApiFixture::default());
    let orka = Arc::new(OrkaFixture::default());
    let mut client = client(&api, &orka, json!({"token":"static-fixture"}));
    let id = target(&mut client).await;
    *api.override_response.lock().unwrap() = Some((
        "/selfsubjectaccessreviews".into(),
        403,
        json!({"kind":"Status","apiVersion":"v1","status":"Failure","message":"static-fixture","reason":"Forbidden","code":403}),
    ));
    assert_eq!(
        client.connect(id, None).await.err(),
        Some(failed(Check::Permissions, CheckResult::PermissionDenied))
    );
    *orka.identity.lock().unwrap() =
        json!({"authenticated":true,"authType":"tokenReview","namespace":""});
    *api.override_response.lock().unwrap() = Some((
        "/token".into(),
        503,
        json!({"kind":"Status","apiVersion":"v1","status":"Failure","message":"issued-fixture","reason":"Unavailable","code":503}),
    ));
    assert_eq!(
        client.connect(id, Some("existing".into())).await.err(),
        Some(failed(Check::ServiceAccount, CheckResult::Unknown))
    );
    assert_eq!(
        api.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|c| c.1.ends_with("/token"))
            .count(),
        1
    );
}
#[tokio::test]
async fn binding_changed_before_access_and_missing_builtin_cannot_pass() {
    let api = Arc::new(ApiFixture::default());
    let orka = Arc::new(OrkaFixture::default());
    let mut client = client(&api, &orka, json!({"token":"static-fixture"}));
    let id = target(&mut client).await;
    let access = client.connect(id, None).await.unwrap();
    *orka.paged.lock().unwrap() = true;
    access
        .require_builtins(&["web_fetch".into()])
        .await
        .unwrap();
    assert!(orka
        .calls
        .lock()
        .unwrap()
        .iter()
        .any(|c| c.0 == OrkaResource::Tools && c.3.as_deref() == Some("next")));
    assert_eq!(
        access.require_builtins(&["missing".into()]).await.err(),
        Some(ClusterError::MissingTool)
    );
    *api.replacement.lock().unwrap() = true;
    assert_eq!(
        client.connect(id, None).await.err(),
        Some(failed(Check::ClusterApi, CheckResult::Unknown))
    );
}

type Fresh = Arc<Mutex<Option<Kubeconfig>>>;
fn reading_session(
    cfg: Kubeconfig,
    orka: &Arc<OrkaFixture>,
) -> (bridge_engine::clusters::commands::ClusterSession, Fresh) {
    let fresh: Fresh = Arc::new(Mutex::new(Some(cfg.clone())));
    let reader = {
        let fresh = fresh.clone();
        Arc::new(move || {
            fresh
                .lock()
                .unwrap()
                .clone()
                .ok_or(ClusterError::Unavailable)
        })
    };
    let session = bridge_engine::clusters::commands::ClusterSession::with_reader(
        ClusterClient::new(cfg, orka.clone()),
        reader,
    );
    (session, fresh)
}
fn id_of(value: impl serde::Serialize) -> String {
    serde_json::to_value(value).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string()
}
fn reply(value: impl serde::Serialize) -> Value {
    serde_json::to_value(value).unwrap()
}
async fn until(done: impl Fn() -> bool) {
    tokio::time::timeout(std::time::Duration::from_secs(20), async {
        while !done() {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("condition was not reached");
}
fn stale() -> Value {
    json!({"check":"configuration","result":"unknown"})
}

#[tokio::test]
async fn reload_rereads_corrected_config_and_invalidates_old_ids_offline() {
    let api = Arc::new(ApiFixture::default());
    let orka = Arc::new(OrkaFixture::default());
    let cfg = config(&api.start(), json!({"token":"static-fixture"}));
    let (session, fresh) = reading_session(cfg.clone(), &orka);
    let old = session.contexts().await.unwrap();
    let old_ids: Vec<_> = old.iter().map(id_of).collect();
    let old_target = id_of(&session.discover(&old_ids[0]).await.unwrap()[0]);
    let used = reply(session.use_target(&old_target, None).await);
    assert_eq!(used["status"], "connected");
    assert_eq!(used["failure"], Value::Null);
    let mut corrected = cfg;
    corrected.contexts[0].name = "corrected".into();
    *fresh.lock().unwrap() = Some(corrected);
    let calls = (
        api.calls.lock().unwrap().len(),
        orka.calls.lock().unwrap().len(),
    );
    let reloaded = session.reload().await.unwrap();
    assert_eq!(
        reloaded.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
        ["corrected", "exec"]
    );
    assert!(reloaded.iter().all(|c| !old_ids.contains(&id_of(c))));
    // Old IDs and confirmed access are gone; reload reached neither the cluster nor Orka.
    assert_eq!(
        reply(session.discover(&old_ids[0]).await.unwrap_err()),
        stale()
    );
    let old_use = reply(session.use_target(&old_target, None).await);
    assert_eq!(old_use["status"], "unavailable");
    assert_eq!(old_use["failure"], stale());
    assert_eq!(
        (
            api.calls.lock().unwrap().len(),
            orka.calls.lock().unwrap().len()
        ),
        calls
    );
    // An explicit new selection and Use are required.
    let target = id_of(&session.discover(&id_of(&reloaded[0])).await.unwrap()[0]);
    assert_ne!(target, old_target);
    assert_eq!(
        reply(session.use_target(&target, None).await)["status"],
        "connected"
    );
}

#[tokio::test]
async fn failed_reload_invalidates_and_keeps_the_injected_orka_reader() {
    let api = Arc::new(ApiFixture::default());
    let orka = Arc::new(OrkaFixture::default());
    let cfg = config(&api.start(), json!({"token":"static-fixture"}));
    let (session, fresh) = reading_session(cfg.clone(), &orka);
    let old_context = id_of(&session.contexts().await.unwrap()[0]);
    let old_target = id_of(&session.discover(&old_context).await.unwrap()[0]);
    assert_eq!(
        reply(session.use_target(&old_target, None).await)["status"],
        "connected"
    );
    *fresh.lock().unwrap() = None;
    let calls = api.calls.lock().unwrap().len();
    assert_eq!(reply(session.reload().await.unwrap_err()), stale());
    assert_eq!(
        reply(session.discover(&old_context).await.unwrap_err()),
        stale()
    );
    assert_eq!(
        reply(session.use_target(&old_target, None).await)["failure"],
        stale()
    );
    assert_eq!(reply(session.contexts().await.unwrap_err()), stale());
    assert_eq!(api.calls.lock().unwrap().len(), calls);
    *fresh.lock().unwrap() = Some(cfg);
    let context = id_of(&session.contexts().await.unwrap()[0]);
    assert_ne!(context, old_context);
    let target = id_of(&session.discover(&context).await.unwrap()[0]);
    let orka_calls = orka.calls.lock().unwrap().len();
    assert_eq!(
        reply(session.use_target(&target, None).await)["status"],
        "connected"
    );
    assert!(orka.calls.lock().unwrap().len() > orka_calls);
}

#[tokio::test]
async fn reload_waits_for_discovery_and_use_and_late_replies_are_stale() {
    let api = Arc::new(ApiFixture::default());
    let orka = Arc::new(OrkaFixture::default());
    let (session, _fresh) = reading_session(
        config(&api.start(), json!({"token":"static-fixture"})),
        &orka,
    );
    let services = || {
        let calls = api.calls.lock().unwrap();
        calls.iter().filter(|c| c.1.ends_with("/services")).count()
    };
    let context = id_of(&session.contexts().await.unwrap()[0]);
    let (release, gate) = std::sync::mpsc::channel();
    *api.gate.lock().unwrap() = Some(gate);
    let discovery = tokio::spawn({
        let session = session.clone();
        async move { session.discover(&context).await }
    });
    until(|| services() == 1).await;
    let reload = tokio::spawn({
        let session = session.clone();
        async move { session.reload().await }
    });
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    assert!(!reload.is_finished(), "reload must queue behind discovery");
    release.send(()).unwrap();
    let late = id_of(&discovery.await.unwrap().unwrap()[0]);
    let contexts = reload.await.unwrap().unwrap();
    assert_eq!(
        reply(session.use_target(&late, None).await)["failure"],
        stale()
    );

    let target = id_of(&session.discover(&id_of(&contexts[0])).await.unwrap()[0]);
    let (release, gate) = std::sync::mpsc::channel();
    *api.gate.lock().unwrap() = Some(gate);
    let before = services();
    let using = tokio::spawn({
        let session = session.clone();
        let target = target.clone();
        async move { reply(session.use_target(&target, None).await) }
    });
    until(|| services() > before).await;
    let reload = tokio::spawn({
        let session = session.clone();
        async move { session.reload().await }
    });
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    assert!(!reload.is_finished(), "reload must queue behind Use");
    release.send(()).unwrap();
    assert_eq!(using.await.unwrap()["status"], "connected");
    reload.await.unwrap().unwrap();
    // The access confirmed just before reload is no longer selectable.
    assert_eq!(
        reply(session.use_target(&target, None).await)["failure"],
        stale()
    );
}

#[tokio::test]
async fn reload_never_runs_the_plugin_and_keeps_its_shutdown_owner() {
    let api = Arc::new(ApiFixture::default());
    let dir = tempfile::tempdir().unwrap();
    let count = dir.path().join("calls");
    let version = "client.authentication.k8s.io/v1";
    let mut response = credential_fixture::response(version);
    response["status"]["token"] = json!("creator-fixture");
    let mut auth = credential_fixture::exec(version, response);
    auth["exec"]["env"]
        .as_array_mut()
        .unwrap()
        .push(json!({"name":"RECORD_FILE","value":count}));
    let (session, _fresh) = reading_session(
        config(&api.start(), auth),
        &Arc::new(OrkaFixture::default()),
    );
    let context = id_of(&session.contexts().await.unwrap()[0]);
    session.discover(&context).await.unwrap();
    assert_eq!(std::fs::read(&count).unwrap().len(), 1);
    let contexts = session.reload().await.unwrap();
    assert_eq!(std::fs::read(&count).unwrap().len(), 1);
    // The retained owner still refuses plugin work after app shutdown.
    session.shutdown().await;
    assert_eq!(
        reply(session.discover(&id_of(&contexts[0])).await.unwrap_err()),
        json!({"check":"credentials","result":"unknown"})
    );
    assert_eq!(std::fs::read(&count).unwrap().len(), 1);
}

#[tokio::test]
async fn discovery_reports_its_first_failed_check() {
    let orka = Arc::new(OrkaFixture::default());
    for (suffix, code, check, result) in [
        (
            "/services",
            401,
            Check::ClusterApi,
            CheckResult::AuthenticationRejected,
        ),
        (
            "/services",
            403,
            Check::ClusterApi,
            CheckResult::PermissionDenied,
        ),
        ("/deployments", 500, Check::ClusterApi, CheckResult::Unknown),
        (
            "agents.core.orka.ai",
            404,
            Check::Compatibility,
            CheckResult::Incompatible,
        ),
        (
            "tools.core.orka.ai",
            403,
            Check::Compatibility,
            CheckResult::PermissionDenied,
        ),
    ] {
        let api = Arc::new(ApiFixture::default());
        *api.override_response.lock().unwrap() = Some((
            suffix.into(),
            code,
            json!({"kind":"Status","apiVersion":"v1","status":"Failure","message":"static-fixture","reason":"Fixture","code":code}),
        ));
        let mut client = client(&api, &orka, json!({"token":"static-fixture"}));
        let id = client.contexts().unwrap()[0].id;
        assert_eq!(
            client.discover(id).await.err(),
            Some(failure(check, result)),
            "{suffix} {code}"
        );
    }
    // A refused connection is established as such; it is not a permission problem.
    let closed = TcpListener::bind("127.0.0.1:0").unwrap();
    let server = format!("http://{}", closed.local_addr().unwrap());
    drop(closed);
    let mut client = ClusterClient::new(config(&server, json!({"token":"static-fixture"})), orka);
    let id = client.contexts().unwrap()[0].id;
    assert_eq!(
        client.discover(id).await.err(),
        Some(failure(Check::ClusterApi, CheckResult::ConnectionFailed))
    );
}

#[tokio::test]
async fn discovery_deadline_keeps_the_compatibility_check_in_progress() {
    let api = Arc::new(ApiFixture::default());
    let orka = Arc::new(OrkaFixture::default());
    let mut client = client(&api, &orka, json!({"token":"static-fixture"}));
    let id = client.contexts().unwrap()[0].id;
    let (steps, received) = std::sync::mpsc::channel();
    *api.step_gate.lock().unwrap() = Some(steps);
    tokio::time::pause();
    let task = tokio::spawn(async move { client.discover(id).await });
    let mut last_release = None;
    for stage in 0..4 {
        let (path, release) = loop {
            if let Ok(step) = received.try_recv() {
                break step;
            }
            tokio::task::yield_now().await;
        };
        if stage == 3 {
            assert!(path.contains("/customresourcedefinitions/"));
        }
        tokio::time::advance(std::time::Duration::from_secs(if stage == 3 {
            4
        } else {
            9
        }))
        .await;
        if stage == 3 {
            last_release = Some(release);
        } else {
            let _ = release.send(());
        }
    }
    let result = task.await.unwrap();
    let _ = last_release.unwrap().send(());
    assert_eq!(
        result.err(),
        Some(failure(Check::Compatibility, CheckResult::Unknown))
    );
}

#[tokio::test]
async fn use_deadline_during_pod_opening_keeps_the_connection_stage() {
    let api = Arc::new(ApiFixture::default());
    let mut client = ClusterClient::new(
        config(&api.start(), json!({"token":"static-fixture"})),
        Arc::new(PodOrkaRead),
    );
    let target = target(&mut client).await;
    let (steps, received) = std::sync::mpsc::channel();
    *api.step_gate.lock().unwrap() = Some(steps);
    tokio::time::pause();
    let task = tokio::spawn(async move { client.connect(target, None).await.err() });
    let last_release = loop {
        let (path, release) = loop {
            if let Ok(step) = received.try_recv() {
                break step;
            }
            tokio::task::yield_now().await;
        };
        if path.ends_with("/pods") {
            tokio::time::advance(std::time::Duration::from_millis(5600)).await;
            break release;
        }
        tokio::time::advance(std::time::Duration::from_millis(3500)).await;
        let _ = release.send(());
    };
    let result = task.await.unwrap();
    let _ = last_release.send(());
    assert_eq!(
        result,
        Some(failed(Check::OrkaConnection, CheckResult::Unknown))
    );
}

type Arrange = fn(&ApiFixture, &OrkaFixture);
#[tokio::test]
async fn use_keeps_kubernetes_permission_apart_from_orka_identity() {
    let cases: [(Arrange, Check, CheckResult); 9] = [
        (
            |_, orka| {
                *orka.identity.lock().unwrap() = json!({"authType":"oidc","namespace":"agents"})
            },
            Check::OrkaIdentity,
            CheckResult::Unknown,
        ),
        (
            |_, orka| {
                *orka.identity.lock().unwrap() =
                    json!({"authenticated":"true","authType":"oidc","namespace":"agents"})
            },
            Check::OrkaIdentity,
            CheckResult::Unknown,
        ),
        (
            |_, orka| *orka.agents.lock().unwrap() = Some(json!({"items":"not-an-array"})),
            Check::Agents,
            CheckResult::Unknown,
        ),
        (
            |api, _| {
                *api.override_response.lock().unwrap() = Some((
                    "/selfsubjectaccessreviews".into(),
                    200,
                    json!({"apiVersion":"authorization.k8s.io/v1","kind":"SelfSubjectAccessReview","spec":{},"status":{"allowed":false,"evaluationError":"static-fixture"}}),
                ))
            },
            Check::Permissions,
            CheckResult::Unknown,
        ),
        (
            |_, orka| *orka.identity.lock().unwrap() = json!({"authenticated":false}),
            Check::OrkaIdentity,
            CheckResult::IdentityRejected,
        ),
        (
            |_, orka| {
                *orka.identity.lock().unwrap() =
                    json!({"authenticated":true,"authType":"tokenReview","namespace":"other"})
            },
            Check::OrkaIdentity,
            CheckResult::IdentityRejected,
        ),
        (
            |_, orka| {
                *orka.identity.lock().unwrap() =
                    json!({"authenticated":true,"authType":7,"namespace":"agents"})
            },
            Check::OrkaIdentity,
            CheckResult::Unknown,
        ),
        (
            |_, orka| {
                *orka.identity.lock().unwrap() =
                    json!({"authenticated":true,"authType":"tokenReview"})
            },
            Check::OrkaIdentity,
            CheckResult::Unknown,
        ),
        (
            |_, orka| *orka.providers.lock().unwrap() = Some(json!({"items":[],"metadata":{}})),
            Check::Providers,
            CheckResult::NoProviders,
        ),
    ];
    for (i, (arrange, check, result)) in cases.into_iter().enumerate() {
        let api = Arc::new(ApiFixture::default());
        let orka = Arc::new(OrkaFixture::default());
        let (session, _fresh) = reading_session(
            config(&api.start(), json!({"token":"static-fixture"})),
            &orka,
        );
        let context = id_of(&session.contexts().await.unwrap()[0]);
        let target = id_of(&session.discover(&context).await.unwrap()[0]);
        arrange(&api, &orka);
        let view = reply(session.use_target(&target, None).await);
        assert_eq!(view["failure"], reply(failure(check, result)), "case {i}");
        let status = match result {
            CheckResult::IdentityRejected => "denied",
            _ => "unavailable",
        };
        assert_eq!(view["status"], status, "case {i}");
        assert_eq!(api.wire.posts(), 0);
    }
}

#[tokio::test]
async fn need_service_account_has_no_failure() {
    let api = Arc::new(ApiFixture::default());
    let orka = Arc::new(OrkaFixture::default());
    *orka.identity.lock().unwrap() =
        json!({"authenticated":true,"authType":"tokenReview","namespace":""});
    let (session, _fresh) = reading_session(
        config(&api.start(), json!({"token":"static-fixture"})),
        &orka,
    );
    let context = id_of(&session.contexts().await.unwrap()[0]);
    let target = id_of(&session.discover(&context).await.unwrap()[0]);
    let view = reply(session.use_target(&target, None).await);
    assert_eq!(view["status"], "needServiceAccount");
    assert_eq!(view["failure"], Value::Null);
}

#[tokio::test]
async fn use_deadline_is_unknown_at_the_provider_page_in_progress() {
    let api = Arc::new(ApiFixture::default());
    let orka = Arc::new(OrkaFixture::default());
    let mut client = client(&api, &orka, json!({"token":"static-fixture"}));
    let id = target(&mut client).await;
    *orka.slow_pages.lock().unwrap() = true;
    let start = std::time::Instant::now();
    assert_eq!(
        client.connect(id, None).await.err(),
        Some(failed(Check::Providers, CheckResult::Unknown))
    );
    let elapsed = start.elapsed().as_secs();
    assert!((29..33).contains(&elapsed), "{elapsed}s");
}

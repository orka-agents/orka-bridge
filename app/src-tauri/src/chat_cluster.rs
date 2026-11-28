//! A conversation's cluster transport. The same state machine as the local workspace, sent over
//! the selected cluster's confirmed Orka connection instead of the VM channel. Everything here is
//! independent of Kubernetes: `clusters::chat` opens the port-forward and supplies a [`ClusterWire`].
//!
//! Every request is checked against [`ChatRequest::validate_cluster`] in the place's watched
//! namespace before it is shaped, and every reply is size-bounded and read as data only.
use crate::chat::ChatError;
use bridge_runtime_protocol::{ChatMethod, ChatRequest, OrkaReply, MAX_ORKA_REPLY};
use serde::Serialize;
use serde_json::Value;
use std::{future::Future, pin::Pin, sync::Arc};

pub type WireFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// One origin-form HTTP request to the Orka API: a fixed method, `/api/v1/...` path and query.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WireRequest {
    pub method: ChatMethod,
    pub uri: String,
    pub body: Option<Vec<u8>>,
}
/// An opened, destination-checked connection to one Orka installation.
pub trait ClusterWire: Send + Sync {
    /// The status and the whole (bounded) body. Transport failures are `Connection`.
    fn exchange(&self, request: WireRequest) -> WireFuture<'_, Result<(u16, Vec<u8>), ChatError>>;
    fn close(&self) -> WireFuture<'_, ()>;
}
/// Opens a fresh connection for one conversation step, revalidating the confirmed access.
pub trait ClusterOpen: Send + Sync {
    fn open(&self) -> WireFuture<'_, Result<Arc<dyn ClusterWire>, ChatError>>;
}
/// Where a cluster conversation runs. Built natively from the confirmed access for the
/// renderer's opaque target and AI connection ids; never from renderer names or paths.
pub struct ClusterChannel {
    /// The remembered place's key: a changed destination is a different conversation.
    pub key: String,
    /// The installation's watched namespace, where every resource is created.
    pub namespace: String,
    /// The Orka Provider of the chosen AI connection.
    pub provider: String,
    pub open: Arc<dyn ClusterOpen>,
}
impl ClusterChannel {
    pub fn new(
        key: String,
        namespace: String,
        provider: String,
        open: Arc<dyn ClusterOpen>,
    ) -> Result<Self, ChatError> {
        if key.is_empty()
            || !bridge_runtime_protocol::dns_label(&namespace)
            || !bridge_runtime_protocol::dns_label(&provider)
        {
            return Err(ChatError::ClusterUnavailable);
        }
        Ok(Self {
            key,
            namespace,
            provider,
            open,
        })
    }
}

/// The Orka permission a refused request needed, named in the renderer's message.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ChatPermission {
    CreateAgents,
    GetAgents,
    DeleteAgents,
    CreateTasks,
    GetTasks,
    DeleteTasks,
    GetSessions,
    DeleteSessions,
    GetProviders,
}
pub fn permission(request: &ChatRequest) -> ChatPermission {
    let collection = request.path.split('/').next().unwrap_or("");
    match (request.method, collection) {
        (ChatMethod::Post, "agents") => ChatPermission::CreateAgents,
        (ChatMethod::Delete, "agents") => ChatPermission::DeleteAgents,
        (_, "agents") => ChatPermission::GetAgents,
        (ChatMethod::Post, _) => ChatPermission::CreateTasks,
        (ChatMethod::Delete, "tasks") => ChatPermission::DeleteTasks,
        (_, "tasks") => ChatPermission::GetTasks,
        (ChatMethod::Delete, _) => ChatPermission::DeleteSessions,
        (_, "sessions") => ChatPermission::GetSessions,
        _ => ChatPermission::GetProviders,
    }
}

/// The exact request line for a checked conversation request, as the guest builds it locally.
pub fn shape(
    namespace: &str,
    provider: &str,
    request: &ChatRequest,
) -> Result<WireRequest, ChatError> {
    request
        .validate_cluster(namespace, provider)
        .map_err(|_| ChatError::Response)?;
    // Validated paths and namespaces are lowercase DNS labels, digits and '/': nothing to encode.
    let mut uri = format!("/api/v1/{}?namespace={namespace}", request.path);
    if let Some(after) = request.after {
        uri.push_str(&format!("&after={after}&limit=1000"));
    }
    let body = request
        .body
        .as_ref()
        .map(serde_json::to_vec)
        .transpose()
        .map_err(|_| ChatError::Response)?;
    Ok(WireRequest {
        method: request.method,
        uri,
        body,
    })
}

/// Why a cluster request produced no reply to act on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The transport or the request failed; the error the state machine already knows.
    Failed(ChatError),
    /// Orka's authorization refused the request (HTTP 403).
    Denied(ChatPermission),
}
/// One checked round trip. A refused credential (401) means the access needs reconnecting; 403
/// names the permission; any other status is returned for the state machine to judge, as locally.
pub async fn send(
    wire: &dyn ClusterWire,
    namespace: &str,
    provider: &str,
    request: &ChatRequest,
) -> Result<OrkaReply, Refusal> {
    let shaped = shape(namespace, provider, request).map_err(Refusal::Failed)?;
    let (status, bytes) = wire.exchange(shaped).await.map_err(Refusal::Failed)?;
    if bytes.len() > MAX_ORKA_REPLY {
        return Err(Refusal::Failed(ChatError::Response));
    }
    match status {
        401 => return Err(Refusal::Failed(ChatError::AccessLost)),
        403 => return Err(Refusal::Denied(permission(request))),
        _ => {}
    }
    Ok(OrkaReply {
        status,
        body: serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    })
}

/// The selected AI connection's default model, from its Provider in the watched namespace.
pub fn default_model(
    reply: &OrkaReply,
    namespace: &str,
    provider: &str,
) -> Result<String, ChatError> {
    if reply.status != 200
        || reply.body["metadata"]["name"] != provider
        || reply.body["metadata"]["namespace"] != namespace
    {
        return Err(ChatError::Model);
    }
    reply.body["spec"]["defaultModel"]
        .as_str()
        .filter(|model| !model.trim().is_empty() && model.len() <= 256)
        .map(str::to_owned)
        .ok_or(ChatError::Model)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn get(path: &str) -> ChatRequest {
        ChatRequest {
            method: ChatMethod::Get,
            path: path.into(),
            after: None,
            body: None,
        }
    }
    const HEX: &str = "0123456789abcdef0123456789abcdef";

    #[test]
    fn shapes_only_checked_paths_in_the_watched_namespace() {
        let events = ChatRequest {
            after: Some(3),
            ..get(&format!("tasks/bridge-chat-task-{HEX}/events"))
        };
        assert_eq!(
            shape("team-a", "fixture", &events).unwrap().uri,
            format!(
                "/api/v1/tasks/bridge-chat-task-{HEX}/events?namespace=team-a&after=3&limit=1000"
            )
        );
        assert!(shape("team-a", "fixture", &get("providers/fixture")).is_ok());
        assert!(shape("team-a", "fixture", &get("providers/other")).is_err());
        assert!(shape(
            "Team_A",
            "fixture",
            &get(&format!("agents/bridge-chat-agent-{HEX}"))
        )
        .is_err());
        assert!(shape("team-a", "fixture", &get("agents/bridge-run-x")).is_err());
        let create = ChatRequest {
            method: ChatMethod::Post,
            path: "agents".into(),
            after: None,
            body: Some(
                json!({"name":format!("bridge-chat-agent-{HEX}"),"namespace":"orka-system","spec":{}}),
            ),
        };
        assert!(
            shape("team-a", "fixture", &create).is_err(),
            "another namespace"
        );
        let deletes = [
            format!("agents/bridge-nav-agent-{HEX}"),
            format!("sessions/bridge-nav-{HEX}"),
        ];
        for path in deletes {
            let delete = ChatRequest {
                method: ChatMethod::Delete,
                ..get(&path)
            };
            assert!(shape("team-a", "fixture", &delete).is_ok());
            assert!(
                delete.validate().is_err(),
                "the local channel never deletes {path}"
            );
        }
    }

    #[test]
    fn names_the_permission_each_request_needs() {
        let delete = |path: &str| ChatRequest {
            method: ChatMethod::Delete,
            ..get(path)
        };
        assert_eq!(permission(&get("sessions/x")), ChatPermission::GetSessions);
        assert_eq!(
            permission(&delete("sessions/x")),
            ChatPermission::DeleteSessions
        );
        assert_eq!(permission(&get("tasks/x/events")), ChatPermission::GetTasks);
        assert_eq!(permission(&delete("tasks/x")), ChatPermission::DeleteTasks);
        assert_eq!(
            permission(&delete("agents/x")),
            ChatPermission::DeleteAgents
        );
        assert_eq!(
            permission(&get("providers/x")),
            ChatPermission::GetProviders
        );
        let post = |path: &str| ChatRequest {
            method: ChatMethod::Post,
            ..get(path)
        };
        assert_eq!(permission(&post("agents")), ChatPermission::CreateAgents);
        assert_eq!(permission(&post("tasks")), ChatPermission::CreateTasks);
    }

    #[test]
    fn default_model_comes_only_from_the_chosen_provider() {
        let reply = |body| OrkaReply { status: 200, body };
        let good = json!({"metadata":{"name":"fixture","namespace":"team-a"},"spec":{"defaultModel":"m-1"}});
        assert_eq!(
            default_model(&reply(good.clone()), "team-a", "fixture").unwrap(),
            "m-1"
        );
        assert!(default_model(&reply(good.clone()), "team-b", "fixture").is_err());
        assert!(default_model(
            &reply(json!({"metadata":{"name":"fixture","namespace":"team-a"},"spec":{}})),
            "team-a",
            "fixture"
        )
        .is_err());
    }
}

pub mod catalog;
mod client;
mod credentials;
#[cfg(test)]
mod tests;
pub use credentials::{Credential, ResolvedCreator};
pub mod commands;
mod destination;
mod forward;
pub(crate) use destination::valid_identity;
pub mod preflight;
use bridge_runtime_protocol::Uuid;
pub use client::{ClusterClient, RemoteAccess};
pub use forward::{OrkaStreamTransport, PodOrkaRead};
use serde::Serialize;
use serde_json::Value;
use std::{future::Future, pin::Pin};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize)]
pub struct ContextId(pub(crate) Uuid);
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize)]
pub struct TargetId(pub(crate) Uuid);
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize)]
pub struct ProviderId(pub(crate) Uuid);
#[derive(Clone, Debug, Serialize)]
pub struct ContextView {
    pub id: ContextId,
    pub name: String,
    pub current: bool,
}
#[derive(Clone, Debug, Serialize)]
pub struct TargetView {
    pub id: TargetId,
    pub context_id: ContextId,
    pub context_name: String,
    pub service_name: String,
    pub service_namespace: String,
    pub watched_namespace: String,
    pub image: String,
}
#[derive(Clone, Debug, Serialize)]
pub struct ServiceAccountsView {
    pub names: Vec<String>,
    pub truncated: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PermissionStatus {
    Allowed,
    Denied,
    Unknown,
    NotChecked,
}
#[derive(Clone, Debug, Serialize)]
pub struct PermissionCheck {
    pub resource: &'static str,
    pub verb: &'static str,
    pub status: PermissionStatus,
}
#[derive(Clone, Debug, Serialize)]
pub struct ServiceAccountPreview {
    pub status: PermissionStatus,
    pub checks: Vec<PermissionCheck>,
}
#[derive(Clone, Debug, Serialize)]
pub struct ProviderView {
    pub id: ProviderId,
    pub name: String,
    pub ready: bool,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TargetBinding {
    pub cluster_identity: String,
    pub context_id: ContextId,
    pub context_name: String,
    pub server_address: String,
    pub service_name: String,
    pub service_namespace: String,
    pub service_uid: String,
    pub deployment_name: String,
    pub deployment_namespace: String,
    pub deployment_uid: String,
    pub watched_namespace: String,
    pub image: String,
    pub api_port: u16,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ForwardPeer {
    pub pod_name: String,
    pub pod_namespace: String,
    pub pod_uid: String,
    pub port: u16,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClusterError {
    Unavailable,
    DestinationMismatch,
    Denied,
    UnsupportedCredential,
    MissingTool,
}
/// The first cluster check that failed. Finite labels only; never a native or server message.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Check {
    Configuration,
    Credentials,
    ClusterApi,
    Compatibility,
    ServiceAccount,
    Permissions,
    OrkaIdentity,
    OrkaConnection,
    Agents,
    Providers,
}
/// Unknown covers timeouts and malformed, oversized or unclassifiable replies.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CheckResult {
    Unknown,
    ConnectionFailed,
    AuthenticationRejected,
    PermissionDenied,
    UnsupportedCredential,
    Incompatible,
    IdentityRejected,
    NoProviders,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct ClusterFailure {
    pub check: Check,
    pub result: CheckResult,
}
// Use stays in one task. Share its active check through nested read/operation
// timeouts so cancellation cannot erase whether a stream had opened yet.
tokio::task_local! { static ACTIVE_CHECK: std::cell::Cell<Check>; }
impl Check {
    pub(crate) fn begin(self) {
        let _ = ACTIVE_CHECK.try_with(|active| active.set(self));
    }
    pub(crate) fn current(self) -> Self {
        ACTIVE_CHECK.try_with(std::cell::Cell::get).unwrap_or(self)
    }
    pub(crate) fn fail(self, result: CheckResult) -> AccessOutcome {
        AccessOutcome::Failed(ClusterFailure {
            check: self,
            result,
        })
    }
    pub(crate) fn unknown(self) -> AccessOutcome {
        self.fail(CheckResult::Unknown)
    }
}
impl ClusterFailure {
    /// The broad categories Send and preflight already act on.
    pub fn legacy(self) -> ClusterError {
        match self.result {
            CheckResult::AuthenticationRejected
            | CheckResult::PermissionDenied
            | CheckResult::IdentityRejected => ClusterError::Denied,
            CheckResult::UnsupportedCredential => ClusterError::UnsupportedCredential,
            _ => ClusterError::Unavailable,
        }
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AccessOutcome {
    NeedServiceAccount,
    Denied,
    Unavailable,
    UnsupportedCredential,
    Failed(ClusterFailure),
}
impl AccessOutcome {
    /// Keeps a known origin. A broad failure without one is unknown at `check`, never a guessed cause.
    pub(crate) fn at(self, check: Check) -> Self {
        match self {
            Self::UnsupportedCredential => check.fail(CheckResult::UnsupportedCredential),
            Self::Denied | Self::Unavailable => check.unknown(),
            other => other,
        }
    }
    pub(crate) fn failure(self, check: Check) -> ClusterFailure {
        match self.at(check) {
            Self::Failed(failure) => failure,
            _ => ClusterFailure {
                check,
                result: CheckResult::Unknown,
            },
        }
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OrkaResource {
    WhoAmI,
    Agents,
    AgentInventory,
    Providers,
    Tools,
    /// Overview Activity: one bounded page of the Task list, in Orka's name order.
    Tasks,
    /// Overview Sessions: one bounded page of the Session list, newest first.
    Sessions,
}
/// One named object read by Overview Sessions.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OrkaItem {
    /// A Session with its transcript.
    Session,
    /// A Task, read only to name the Agent of a Session's running Task.
    Task,
}
impl OrkaResource {
    pub(crate) fn check(self) -> Check {
        match self {
            Self::WhoAmI => Check::OrkaIdentity,
            // Tools are read only by Send, whose callers keep the broad category. Activity
            // and Sessions report only whether a place could be read, so they share the label.
            Self::Agents | Self::AgentInventory | Self::Tools | Self::Tasks | Self::Sessions => {
                Check::Agents
            }
            Self::Providers => Check::Providers,
        }
    }
}
/// Native GET-only seam. The implementation owns the selected pod stream, not an arbitrary URL.
pub trait OrkaRead: Send + Sync {
    fn get<'a>(
        &'a self,
        creator: &'a kube::Client,
        binding: &'a TargetBinding,
        bearer: &'a str,
        resource: OrkaResource,
        namespace: &'a str,
        after: Option<&'a str>,
    ) -> Pin<Box<dyn Future<Output = Result<Value, AccessOutcome>> + Send + 'a>>;
    /// One page of an execution event stream after sequence `after`. Readers without an
    /// event route report it unavailable.
    fn events<'a>(
        &'a self,
        _creator: &'a kube::Client,
        _binding: &'a TargetBinding,
        _bearer: &'a str,
        _namespace: &'a str,
        _stream: EventStream<'a>,
        _after: u64,
    ) -> Pin<Box<dyn Future<Output = Result<Value, AccessOutcome>> + Send + 'a>> {
        Box::pin(async { Err(AccessOutcome::Unavailable) })
    }
    /// One named Session or Task, for Overview Sessions. Readers without an item route
    /// report it unavailable.
    #[allow(clippy::too_many_arguments)]
    fn item<'a>(
        &'a self,
        _creator: &'a kube::Client,
        _binding: &'a TargetBinding,
        _bearer: &'a str,
        _item: OrkaItem,
        _namespace: &'a str,
        _name: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Value, AccessOutcome>> + Send + 'a>> {
        Box::pin(async { Err(AccessOutcome::Unavailable) })
    }
}
/// An Orka execution event stream that `OrkaRead::events` pages through. It is built only
/// from a valid Kubernetes object name, and its kind fixes the request path and event filter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EventStream<'a> {
    kind: EventKind,
    name: &'a str,
}
/// Which execution stream, and which of its events, `EventStream` reads.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EventKind {
    /// A Task's completed and failed tool calls.
    TaskToolCalls,
}
impl<'a> EventStream<'a> {
    /// A Task's tool-call events, or `None` when `task` is not a Kubernetes object name.
    pub fn task_tool_calls(task: &'a str) -> Option<Self> {
        crate::overview_activity::valid_task(task).then_some(Self {
            kind: EventKind::TaskToolCalls,
            name: task,
        })
    }
    pub fn kind(self) -> EventKind {
        self.kind
    }
    /// The Task the stream belongs to.
    pub fn name(self) -> &'a str {
        self.name
    }
    /// The event types the read asks Orka for.
    pub fn types(self) -> &'static [&'static str] {
        match self.kind {
            EventKind::TaskToolCalls => &["ToolCallCompleted", "ToolCallFailed"],
        }
    }
    pub(crate) fn path(self) -> String {
        match self.kind {
            EventKind::TaskToolCalls => format!("tasks/{}/events", self.name),
        }
    }
}

//! Read-only recent activity for confirmed cluster places. Orka lists Tasks in name order,
//! with no newest-first or cross-Task event feed, so a read takes a bounded number of Task
//! pages and sorts here. Only `RemoteAccess::tasks` and `RemoteAccess::tool_calls` read Orka,
//! so a later sorted or namespace-wide API replaces them without touching the view.
#[cfg(test)]
#[path = "activity_tests.rs"]
mod tests;
use crate::clusters::{commands::ClusterSession, ClusterError, RemoteAccess};
use crate::overview_places::Lost;
use serde::Serialize;
use serde_json::Value;
use std::{future::Future, sync::Arc};

/// Entries shown per place, newest first.
pub const SHOWN: usize = 50;
/// Tasks read per place before the list is reported as partial.
pub const MAX_TASKS: usize = 200;
/// Tool calls kept per Task before the list is reported as partial.
pub const MAX_TOOL_CALLS: usize = 200;
const MESSAGE: usize = 512;
/// Bridge names the Tasks its own Send tests create with this prefix.
const BRIDGE_TASK: &str = "bridge-task-";

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Phase {
    Pending,
    Scheduled,
    Running,
    Finalizing,
    Succeeded,
    Failed,
    Cancelled,
    Unknown,
}
impl Phase {
    fn parse(value: &Value) -> Self {
        match value.as_str() {
            Some("Pending") | None => Self::Pending,
            Some("Scheduled") => Self::Scheduled,
            Some("Running") => Self::Running,
            Some("Finalizing") => Self::Finalizing,
            Some("Succeeded") => Self::Succeeded,
            Some("Failed") => Self::Failed,
            Some("Cancelled") => Self::Cancelled,
            Some(_) => Self::Unknown,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub task: String,
    pub uid: String,
    /// `spec.agentRef.name`; absent for Tasks that name no Agent.
    pub agent: Option<String>,
    /// The Agent UID Orka bound at execution, only when it agrees with `agent` and the namespace.
    pub agent_uid: Option<String>,
    pub phase: Phase,
    pub created_at: u64,
    pub started_at: Option<u64>,
    pub completed_at: Option<u64>,
    /// Orka's status message, bounded, kept only for failed or cancelled runs.
    pub message: Option<String>,
    /// Created by Bridge's own Send test.
    pub test: bool,
}
impl Entry {
    fn latest(&self) -> u64 {
        self.completed_at
            .into_iter()
            .chain(self.started_at)
            .fold(self.created_at, u64::max)
    }
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Tasks {
    pub entries: Vec<Entry>,
    /// False when more Tasks exist than one read takes, so newer runs may be missing.
    pub complete: bool,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCall {
    pub tool: String,
    pub failed: bool,
    pub at: u64,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ToolCalls {
    pub calls: Vec<ToolCall>,
    pub complete: bool,
}
fn text(value: &Value, max: usize) -> Option<String> {
    value
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= max)
        .map(str::to_owned)
}
fn millis(value: &Value) -> Option<u64> {
    let stamp: k8s_openapi::jiff::Timestamp = value.as_str()?.parse().ok()?;
    u64::try_from(stamp.as_millisecond()).ok()
}
fn bounded(value: &Value) -> Option<String> {
    let message = value.as_str()?.trim();
    if message.is_empty() {
        return None;
    }
    let mut end = message.len().min(MESSAGE);
    while !message.is_char_boundary(end) {
        end -= 1;
    }
    Some(message[..end].to_owned())
}
/// One listed Task. Identity fields are required; status fields degrade to absent.
pub(crate) fn task(value: &Value, namespace: &str) -> Result<Entry, ClusterError> {
    let bad = ClusterError::Unavailable;
    let metadata = &value["metadata"];
    if metadata["namespace"].as_str() != Some(namespace) {
        return Err(bad);
    }
    let task = text(&metadata["name"], 253).ok_or(bad)?;
    let uid = text(&metadata["uid"], 128).ok_or(bad)?;
    let created_at = millis(&metadata["creationTimestamp"]).ok_or(bad)?;
    let status = &value["status"];
    let agent = text(&value["spec"]["agentRef"]["name"], 253);
    let bound = &status["agentExecutionBinding"]["agent"];
    let agent_uid = agent
        .as_deref()
        .filter(|name| {
            bound["name"].as_str() == Some(name) && bound["namespace"].as_str() == Some(namespace)
        })
        .and_then(|_| text(&bound["uid"], 128));
    let phase = Phase::parse(&status["phase"]);
    Ok(Entry {
        test: task.starts_with(BRIDGE_TASK),
        message: matches!(phase, Phase::Failed | Phase::Cancelled)
            .then(|| bounded(&status["message"]))
            .flatten(),
        started_at: millis(&status["startTime"]),
        completed_at: millis(&status["completionTime"]),
        task,
        uid,
        agent,
        agent_uid,
        phase,
        created_at,
    })
}
/// Newest first by the latest recorded time, then by name for a stable order.
pub(crate) fn newest(mut entries: Vec<Entry>, complete: bool) -> Tasks {
    entries.sort_by(|a, b| b.latest().cmp(&a.latest()).then(a.task.cmp(&b.task)));
    entries.truncate(SHOWN);
    Tasks { entries, complete }
}
/// One filtered page of tool-call events.
pub(crate) struct ToolPage {
    pub calls: Vec<ToolCall>,
    /// The cursor for the next read, or None when the stream has no further events.
    pub next: Option<u64>,
    /// An event without a tool name or time was left out, so the list is not complete.
    pub dropped: bool,
}
/// Tool-call events from one filtered page.
pub(crate) fn tool_page(
    page: &Value,
    namespace: &str,
    task: &str,
    after: u64,
    page_size: usize,
) -> Result<ToolPage, ClusterError> {
    let bad = ClusterError::Unavailable;
    let latest = page["latestSeq"].as_u64().ok_or(bad)?;
    if page["namespace"].as_str() != Some(namespace)
        || page["streamType"].as_str() != Some("task")
        || page["streamID"].as_str() != Some(task)
        || page["afterSeq"].as_u64() != Some(after)
        || latest < after
    {
        return Err(bad);
    }
    let events = page["events"].as_array().ok_or(bad)?;
    if events.len() > page_size {
        return Err(bad);
    }
    let mut calls = Vec::new();
    let mut dropped = false;
    let mut last = after;
    for event in events {
        let seq = event["seq"].as_u64().ok_or(bad)?;
        if seq <= last
            || seq > latest
            || event["namespace"].as_str() != Some(namespace)
            || event["streamID"].as_str() != Some(task)
            || event["taskName"]
                .as_str()
                .is_some_and(|s| !s.is_empty() && s != task)
        {
            return Err(bad);
        }
        last = seq;
        let failed = match event["type"].as_str() {
            Some("ToolCallCompleted") => false,
            Some("ToolCallFailed") => true,
            // An Orka that ignores the type filter sends other events; they are not tool calls.
            _ => continue,
        };
        match (text(&event["toolName"], 256), millis(&event["createdAt"])) {
            (Some(tool), Some(at)) => calls.push(ToolCall { tool, failed, at }),
            _ => dropped = true,
        }
    }
    // The type filter skips sequence numbers and Orka may cap the page size, so only an
    // empty page or reaching the stream's latest sequence ends the list.
    let more = !events.is_empty() && last < latest;
    Ok(ToolPage {
        calls,
        next: more.then_some(last),
        dropped,
    })
}
/// A Kubernetes object name; anything else never reaches a request path.
pub(crate) fn valid_task(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 253
        && name.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && label
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
                && label.as_bytes()[0].is_ascii_alphanumeric()
                && label.as_bytes()[label.len() - 1].is_ascii_alphanumeric()
        })
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaceActivity {
    pub key: String,
    /// `complete`, `partial` (more Tasks than one read takes), `denied`, `unavailable`,
    /// `revoked` (Reload or a new connection removed this session's access), or `disconnected`
    /// (no confirmed access this session, or the last read failed).
    pub state: &'static str,
    pub observed_at: Option<u64>,
    pub entries: Vec<Entry>,
    /// The place's connection revision; earlier runs are only shown again under the same one.
    pub connection: Option<u64>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Activity {
    pub places: Vec<PlaceActivity>,
}
fn failure(error: ClusterError) -> &'static str {
    match error {
        ClusterError::Denied => "denied",
        _ => "unavailable",
    }
}
fn lost(lost: Lost) -> &'static str {
    match lost {
        Lost::Denied => "denied",
        Lost::Revoked => "revoked",
        Lost::Disconnected => "disconnected",
    }
}
/// Why a result read on connection `read_on` no longer stands, if it doesn't: the place has a
/// new connection, or its access was refused or revoked since.
fn superseded(session: &ClusterSession, key: &str, read_on: Option<u64>) -> Option<&'static str> {
    match session.activity_connection(key) {
        (now, _) if now != read_on => Some("revoked"),
        (_, Err(why @ (Lost::Denied | Lost::Revoked))) => Some(lost(why)),
        _ => None,
    }
}
/// `with_access`, also returning the connection revision the read used.
async fn read_on<'a, T, F>(
    session: &'a ClusterSession,
    key: &str,
    read: impl FnOnce(Arc<RemoteAccess>) -> F,
) -> (Option<u64>, Result<T, &'static str>)
where
    F: Future<Output = Result<T, ClusterError>> + 'a,
{
    let (connection, access) = session.activity_connection(key);
    let access = match access {
        Ok(access) => access,
        Err(why) => return (connection, Err(lost(why))),
    };
    let result = read(access).await.map_err(failure);
    match superseded(session, key, connection) {
        Some(state) => (connection, Err(state)),
        None => (connection, result),
    }
}
/// Runs `read` with the place's current access. The result stands only if the place still has
/// the same connection afterwards and was not refused or revoked meanwhile; otherwise the
/// error says which (`revoked` for a new connection).
pub(crate) async fn with_access<'a, T, F>(
    session: &'a ClusterSession,
    key: &str,
    read: impl FnOnce(Arc<RemoteAccess>) -> F,
) -> Result<T, &'static str>
where
    F: Future<Output = Result<T, ClusterError>> + 'a,
{
    read_on(session, key, read).await.1
}
pub async fn read(session: &ClusterSession) -> Activity {
    let mut places = Vec::new();
    for place in session.places().places {
        let mut view = PlaceActivity {
            key: place.key.clone(),
            state: "disconnected",
            observed_at: None,
            entries: Vec::new(),
            connection: None,
        };
        let (connection, read) =
            read_on(
                session,
                &place.key,
                |access| async move { access.tasks().await },
            )
            .await;
        view.connection = connection;
        match read {
            Ok(tasks) => {
                view.state = if tasks.complete {
                    "complete"
                } else {
                    "partial"
                };
                view.observed_at = Some(crate::overview::now());
                view.entries = tasks.entries;
            }
            Err(state) => view.state = state,
        }
        places.push(view);
    }
    // Places are read one after another, so recheck each before answering: one reloaded,
    // reconnected or refused while a later place was read must not report its old runs.
    for view in &mut places {
        if let Some(state) = superseded(session, &view.key, view.connection) {
            view.state = state;
            view.observed_at = None;
            view.entries.clear();
            view.connection = session.activity_connection(&view.key).0;
        }
    }
    Activity { places }
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskToolCalls {
    /// `complete`, `partial`, `denied`, `unavailable`, `revoked` or `disconnected`.
    pub state: &'static str,
    pub calls: Vec<ToolCall>,
}
pub async fn tool_calls(session: &ClusterSession, key: &str, task: &str) -> TaskToolCalls {
    let mut view = TaskToolCalls {
        state: "disconnected",
        calls: Vec::new(),
    };
    if !valid_task(task) {
        view.state = "unavailable";
        return view;
    }
    let task = task.to_owned();
    match with_access(session, key, |access| async move {
        access.tool_calls(&task).await
    })
    .await
    {
        Ok(calls) => {
            view.state = if calls.complete {
                "complete"
            } else {
                "partial"
            };
            view.calls = calls.calls;
        }
        Err(state) => view.state = state,
    }
    view
}

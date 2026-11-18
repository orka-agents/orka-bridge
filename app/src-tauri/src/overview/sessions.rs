//! Read-only Sessions for confirmed cluster places. Orka lists Sessions newest created first and
//! hides gateway (channel) Sessions (orka-agents/orka#1183). A listed Session names no Agent
//! or initiator (orka-agents/orka#1182), so the Agent is shown only while a Task runs in it,
//! from that Task's `agentRef`.
#[cfg(test)]
#[path = "sessions_tests.rs"]
mod tests;
use crate::clusters::{commands::ClusterSession, ClusterError, OrkaItem, RemoteAccess};
use crate::overview_activity::{read_on, superseded, valid_task, with_access};
use serde::Serialize;
use serde_json::Value;
use std::sync::Arc;

/// Sessions shown per place, newest first.
pub const SHOWN: usize = 50;
/// Sessions read per place before the list is reported as partial.
pub const MAX_SESSIONS: usize = 100;
/// Running Tasks read per place to name their Agents.
pub const MAX_AGENT_READS: usize = 10;
/// Time all of one place's Agent reads may take together.
const AGENT_READS_LIMIT: std::time::Duration = std::time::Duration::from_secs(10);
/// Transcript messages kept, the latest ones, before the transcript is reported as partial.
pub const MAX_MESSAGES: usize = 200;
/// Characters of each message's text kept.
pub const MESSAGE_TEXT: usize = 16 * 1024;
const GATEWAY: &str = "gateway";

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    pub name: String,
    /// Orka's `sessionType`, such as `chat`; absent when Orka records none.
    pub session_type: Option<String>,
    pub message_count: u64,
    pub created_at: u64,
    pub updated_at: u64,
    /// The Task running in this Session now, if any.
    pub active_task: Option<String>,
    /// The running Task's Agent; absent when nothing runs or it could not be read.
    pub agent: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SessionList {
    pub entries: Vec<Session>,
    /// False when more Sessions exist than one read takes, so older ones are missing.
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
/// A Session name Bridge will put in a request path: Orka's names are object-like.
pub fn valid_session(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 253
        && name.as_bytes()[0].is_ascii_alphanumeric()
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}
/// One listed Session, or None for a gateway Session. Identity and times are required.
pub(crate) fn session(value: &Value, namespace: &str) -> Result<Option<Session>, ClusterError> {
    let bad = ClusterError::Unavailable;
    // Skip gateway Sessions first: their channel-derived names need not be object-like.
    if value["sessionType"].as_str() == Some(GATEWAY) {
        return Ok(None);
    }
    if value["namespace"].as_str() != Some(namespace) {
        return Err(bad);
    }
    let name = text(&value["name"], 253)
        .filter(|n| valid_session(n))
        .ok_or(bad)?;
    let session_type = text(&value["sessionType"], 32)
        .filter(|t| t.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'));
    Ok(Some(Session {
        message_count: value["messageCount"].as_u64().ok_or(bad)?,
        created_at: millis(&value["createdAt"]).ok_or(bad)?,
        updated_at: millis(&value["updatedAt"]).ok_or(bad)?,
        active_task: text(&value["activeTask"], 253).filter(|t| valid_task(t)),
        agent: None,
        session_type,
        name,
    }))
}
/// Newest first by last update, then by name for a stable order.
pub(crate) fn newest(mut entries: Vec<Session>, complete: bool) -> SessionList {
    entries.sort_by(|a, b| b.updated_at.cmp(&a.updated_at).then(a.name.cmp(&b.name)));
    let complete = complete && entries.len() <= SHOWN;
    entries.truncate(SHOWN);
    SessionList { entries, complete }
}
/// What a read of a Session's active Task shows, when the reply is that Task in the watched
/// namespace.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum ActiveTask {
    /// The Task has finished; the Session's lock is stale, so nothing runs in it.
    Ended,
    /// The Task has not finished; its Agent, if it names one.
    Running(Option<String>),
}
pub(crate) fn active_task(value: &Value, namespace: &str, task: &str) -> Option<ActiveTask> {
    let metadata = &value["metadata"];
    if metadata["name"].as_str() != Some(task) || metadata["namespace"].as_str() != Some(namespace)
    {
        return None;
    }
    if matches!(
        value["status"]["phase"].as_str(),
        Some("Succeeded" | "Failed" | "Cancelled")
    ) {
        return Some(ActiveTask::Ended);
    }
    Some(ActiveTask::Running(text(
        &value["spec"]["agentRef"]["name"],
        253,
    )))
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    /// Orka's role, such as `user`, `assistant`, `tool` or `system`.
    pub role: String,
    pub text: String,
    /// The text was longer than Bridge shows and was cut.
    pub cut: bool,
    /// Names of the tools an assistant message called.
    pub tools: Vec<String>,
    pub at: Option<u64>,
}
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ParsedTranscript {
    pub messages: Vec<Message>,
    pub complete: bool,
}
fn bounded(text: &str) -> (String, bool) {
    if text.len() <= MESSAGE_TEXT {
        return (text.to_owned(), false);
    }
    let mut end = MESSAGE_TEXT;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    (text[..end].to_owned(), true)
}
fn tool_names(calls: &Value) -> Vec<String> {
    calls
        .as_array()
        .map(|calls| {
            calls
                .iter()
                .filter_map(|c| text(&c["name"], 256).or_else(|| text(&c["function"]["name"], 256)))
                .collect()
        })
        .unwrap_or_default()
}
/// A Session reply's transcript. The reply must be the requested Session in the namespace.
/// Lines that do not parse are left out and make the transcript partial.
pub(crate) fn transcript(
    value: &Value,
    namespace: &str,
    name: &str,
) -> Result<ParsedTranscript, ClusterError> {
    let bad = ClusterError::Unavailable;
    if value["name"].as_str() != Some(name) || value["namespace"].as_str() != Some(namespace) {
        return Err(bad);
    }
    let raw = match &value["transcript"] {
        Value::Null => "",
        Value::String(s) => s.as_str(),
        _ => return Err(bad),
    };
    let mut messages = Vec::new();
    let mut complete = true;
    for line in raw.lines().filter(|l| !l.trim().is_empty()) {
        let Ok(message) = serde_json::from_str::<Value>(line) else {
            complete = false;
            continue;
        };
        let Some(role) = text(&message["role"], 32) else {
            complete = false;
            continue;
        };
        let (text, cut) = bounded(message["content"].as_str().unwrap_or(""));
        messages.push(Message {
            role,
            text,
            cut,
            tools: tool_names(&message["toolCalls"]),
            at: millis(&message["ts"]),
        });
    }
    if messages.len() > MAX_MESSAGES {
        messages.drain(..messages.len() - MAX_MESSAGES);
        complete = false;
    }
    Ok(ParsedTranscript { messages, complete })
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaceSessions {
    pub key: String,
    /// `complete`, `partial` (more Sessions than one read takes), `denied`, `unavailable`,
    /// `revoked` (Reload or a new connection removed this session's access), or `disconnected`
    /// (no confirmed access this session, or the last read failed).
    pub state: &'static str,
    pub observed_at: Option<u64>,
    pub entries: Vec<Session>,
    /// The place's connection revision; earlier Sessions are only shown again under the same one.
    pub connection: Option<u64>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Sessions {
    pub places: Vec<PlaceSessions>,
}
/// The place's Sessions, with the Agent of each running Task, from one access.
async fn list(access: Arc<RemoteAccess>) -> Result<SessionList, ClusterError> {
    let mut list = access.sessions().await?;
    let namespace = access.binding().watched_namespace.clone();
    // Name the Agent only for running Tasks, within one shared budget; a failed or late read
    // leaves the Task shown without an Agent.
    let _ = tokio::time::timeout(AGENT_READS_LIMIT, async {
        for entry in list
            .entries
            .iter_mut()
            .filter(|e| e.active_task.is_some())
            .take(MAX_AGENT_READS)
        {
            let task = entry.active_task.clone().unwrap();
            let Ok(value) = access.item(OrkaItem::Task, &task).await else {
                continue;
            };
            match active_task(&value, &namespace, &task) {
                Some(ActiveTask::Ended) => entry.active_task = None,
                Some(ActiveTask::Running(agent)) => entry.agent = agent,
                None => {}
            }
        }
    })
    .await;
    Ok(list)
}
pub async fn read(session: &ClusterSession) -> Sessions {
    let mut places = Vec::new();
    for place in session.places().places {
        let mut view = PlaceSessions {
            key: place.key.clone(),
            state: "disconnected",
            observed_at: None,
            entries: Vec::new(),
            connection: None,
        };
        let (connection, read) = read_on(session, &place.key, list).await;
        view.connection = connection;
        match read {
            Ok(list) => {
                view.state = if list.complete { "complete" } else { "partial" };
                view.observed_at = Some(crate::overview::now());
                view.entries = list.entries;
            }
            Err(state) => view.state = state,
        }
        places.push(view);
    }
    // Places are read one after another, so recheck each before answering: one reloaded,
    // reconnected or refused while a later place was read must not report its old Sessions.
    for view in &mut places {
        if let Some(state) = superseded(session, &view.key, view.connection) {
            view.state = state;
            view.observed_at = None;
            view.entries.clear();
            view.connection = session.activity_connection(&view.key).0;
        }
    }
    Sessions { places }
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Transcript {
    /// `complete`, `partial`, `denied`, `unavailable`, `revoked` or `disconnected`.
    pub state: &'static str,
    pub messages: Vec<Message>,
}
pub async fn read_transcript(session: &ClusterSession, key: &str, name: &str) -> Transcript {
    let mut view = Transcript {
        state: "disconnected",
        messages: Vec::new(),
    };
    if !valid_session(name) {
        view.state = "unavailable";
        return view;
    }
    let name = name.to_owned();
    match with_access(session, key, |access| async move {
        let namespace = access.binding().watched_namespace.clone();
        let value = access.item(OrkaItem::Session, &name).await?;
        transcript(&value, &namespace, &name)
    })
    .await
    {
        Ok(read) => {
            view.state = if read.complete { "complete" } else { "partial" };
            view.messages = read.messages;
        }
        Err(state) => view.state = state,
    }
    view
}

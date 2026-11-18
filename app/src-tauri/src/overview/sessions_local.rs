//! Read-only Sessions on this computer's VM. The guest reads them with its own read-only
//! identity and returns bounded projections (see `bridge_runtime_protocol::sessions`). A VM
//! image older than these actions refuses them, and the place says it needs a newer image.
#[cfg(test)]
#[path = "sessions_local_tests.rs"]
mod tests;
use super::{newest, session, transcript, PlaceSessions, SessionList, Transcript, MAX_SESSIONS};
use crate::runtime::types::RuntimeError;
use bridge_runtime_protocol::{
    valid_session_cursor, valid_session_name, EngineActionWire, OrkaReply, Uuid, ORKA_NAMESPACE,
    SESSION_PAGE,
};
use serde_json::Value;
use std::{collections::HashSet, future::Future, io, time::Duration};

/// The place key for this computer's VM.
pub const LOCAL: &str = "local";
/// One whole local read, every page included.
const LOCAL_LIMIT: Duration = Duration::from_secs(30);
/// Tries when another VM request holds the session, and the wait between them.
const BUSY_TRIES: usize = 3;
const BUSY_WAIT: Duration = Duration::from_millis(250);

/// The VM seam: one read-only Sessions action, with the boot that answered it.
pub trait LocalSessions: Send + Sync {
    fn sessions_request(
        &self,
        action: EngineActionWire,
    ) -> impl Future<Output = Result<(Uuid, io::Result<OrkaReply>), RuntimeError>> + Send;
}
impl LocalSessions for crate::local_vm::LocalVm {
    fn sessions_request(
        &self,
        action: EngineActionWire,
    ) -> impl Future<Output = Result<(Uuid, io::Result<OrkaReply>), RuntimeError>> + Send {
        crate::local_vm::LocalVm::sessions_request(self, action)
    }
}

/// A boot as a connection: Overview keeps a place's last Sessions only on the same one.
fn connection(boot: Uuid) -> u64 {
    u64::from_be_bytes(boot.as_bytes()[..8].try_into().unwrap())
}
type Failure = (Option<u64>, &'static str);
/// One action's 200 body. A busy session is retried briefly; a stopped VM is disconnected.
async fn call(vm: &impl LocalSessions, action: EngineActionWire) -> Result<(u64, Value), Failure> {
    let mut tries = 0;
    let (boot, reply) = loop {
        tries += 1;
        match vm.sessions_request(action.clone()).await {
            Ok(answer) => break answer,
            Err(RuntimeError::Busy) if tries < BUSY_TRIES => tokio::time::sleep(BUSY_WAIT).await,
            Err(RuntimeError::Busy) => return Err((None, "unavailable")),
            Err(_) => return Err((None, "disconnected")),
        }
    };
    let boot = connection(boot);
    match reply {
        Ok(OrkaReply { status: 200, body }) => Ok((boot, body)),
        Ok(OrkaReply {
            status: 401 | 403, ..
        }) => Err((Some(boot), "denied")),
        Ok(_) => Err((Some(boot), "unavailable")),
        Err(e) if e.kind() == io::ErrorKind::Unsupported => Err((Some(boot), "outdated")),
        Err(_) => Err((Some(boot), "unavailable")),
    }
}
async fn list(vm: &impl LocalSessions) -> Result<(u64, SessionList), Failure> {
    let mut entries = Vec::new();
    let mut after: Option<String> = None;
    let mut cursors = HashSet::new();
    let mut names = HashSet::new();
    let mut boot = None;
    for _ in 0..MAX_SESSIONS.div_ceil(SESSION_PAGE) {
        let (answered, page) = call(
            vm,
            EngineActionWire::SessionList {
                after: after.clone(),
            },
        )
        .await?;
        // Pages from different boots are different lists.
        if *boot.get_or_insert(answered) != answered {
            return Err((Some(answered), "unavailable"));
        }
        let bad = (Some(answered), "unavailable");
        for item in page["items"].as_array().ok_or(bad)? {
            let Some(entry) = session(item, ORKA_NAMESPACE).map_err(|_| bad)? else {
                continue;
            };
            if !names.insert(entry.name.clone()) {
                return Err(bad);
            }
            entries.push(entry);
        }
        after = match &page["metadata"]["continue"] {
            Value::Null => None,
            Value::String(s) if s.is_empty() => None,
            Value::String(s) if valid_session_cursor(s) => Some(s.clone()),
            _ => return Err(bad),
        };
        let Some(next) = &after else {
            return Ok((answered, newest(entries, true)));
        };
        if entries.len() >= MAX_SESSIONS {
            return Ok((answered, newest(entries, false)));
        }
        if !cursors.insert(next.clone()) {
            return Err(bad);
        }
    }
    Ok((boot.unwrap_or_default(), newest(entries, false)))
}
/// This computer's Sessions. A Task holding a Session is named, but its Agent is not read.
pub async fn read(vm: &impl LocalSessions) -> PlaceSessions {
    let mut view = PlaceSessions {
        key: LOCAL.into(),
        state: "disconnected",
        observed_at: None,
        entries: Vec::new(),
        connection: None,
    };
    match tokio::time::timeout(LOCAL_LIMIT, list(vm))
        .await
        .unwrap_or(Err((None, "unavailable")))
    {
        Ok((boot, list)) => {
            view.state = if list.complete { "complete" } else { "partial" };
            view.observed_at = Some(crate::overview::now());
            view.entries = list.entries;
            view.connection = Some(boot);
        }
        Err((boot, state)) => {
            view.state = state;
            view.connection = boot;
        }
    }
    view
}
/// One Session's latest messages on this computer.
pub async fn read_transcript(vm: &impl LocalSessions, name: &str) -> Transcript {
    let mut view = Transcript {
        state: "unavailable",
        messages: Vec::new(),
    };
    if !valid_session_name(name) {
        return view;
    }
    let action = EngineActionWire::SessionRead { name: name.into() };
    match tokio::time::timeout(LOCAL_LIMIT, call(vm, action))
        .await
        .unwrap_or(Err((None, "unavailable")))
    {
        Ok((_, body)) => match transcript(&body, ORKA_NAMESPACE, name) {
            Ok(read) => {
                view.state = if read.complete { "complete" } else { "partial" };
                view.messages = read.messages;
            }
            Err(_) => view.state = "unavailable",
        },
        Err((_, state)) => view.state = state,
    }
    view
}

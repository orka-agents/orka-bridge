//! Read-only Sessions in this computer's workspace. The guest reads them with its own read-only
//! identity and returns bounded projections (see `bridge_runtime_protocol::sessions`). A guest
//! whose hello does not offer Sessions is never sent one, and the place says it needs a newer
//! image. Like Overview's live Agents, a read is skipped while the workspace is not ready and is
//! not retried while another request holds the workspace session.
#[cfg(test)]
#[path = "sessions_local_tests.rs"]
mod tests;
use super::{
    newest, session, transcript, Listed, PlaceSessions, SessionList, Transcript, MAX_SESSIONS,
};
use crate::runtime::types::RuntimeError;
use bridge_runtime_protocol::{
    valid_session_cursor, valid_session_name, EngineActionWire, OrkaReply, Uuid, ORKA_NAMESPACE,
    SESSION_PAGE,
};
use serde_json::Value;
use std::{collections::HashSet, future::Future, io, time::Duration};

/// The place key for this computer's workspace.
pub const LOCAL: &str = "local";
/// One whole local read, every page included.
const LOCAL_LIMIT: Duration = Duration::from_secs(30);

/// The workspace seam.
pub trait LocalSessions: Send + Sync {
    /// Whether the workspace is ready; nothing is requested otherwise.
    fn ready(&self) -> bool;
    /// The boot that answered the last Sessions request, if any.
    fn last_boot(&self) -> Option<Uuid>;
    /// One read-only Sessions action, with the boot that answered it.
    fn sessions_request(
        &self,
        action: EngineActionWire,
    ) -> impl Future<Output = Result<(Uuid, io::Result<OrkaReply>), RuntimeError>> + Send;
}
impl LocalSessions for crate::local_vm::LocalVm {
    fn ready(&self) -> bool {
        self.status() == crate::local_vm::LocalVmStatus::Ready
    }
    fn last_boot(&self) -> Option<Uuid> {
        self.sessions_boot()
    }
    fn sessions_request(
        &self,
        action: EngineActionWire,
    ) -> impl Future<Output = Result<(Uuid, io::Result<OrkaReply>), RuntimeError>> + Send {
        crate::local_vm::LocalVm::sessions_request(self, action)
    }
}

/// A boot as a connection: Overview keeps a place's last Sessions only on the same one. The
/// value fits a JavaScript safe integer.
pub(crate) fn connection(boot: Uuid) -> u64 {
    u64::from_be_bytes(boot.as_bytes()[..8].try_into().unwrap()) >> 11
}
type Failure = (Option<u64>, &'static str);
/// One action's 200 body. A stopped or busy workspace keeps the last boot as its connection,
/// so Overview can go on showing what it last read on that boot.
async fn call(vm: &impl LocalSessions, action: EngineActionWire) -> Result<(u64, Value), Failure> {
    let last = || vm.last_boot().map(connection);
    if !vm.ready() {
        return Err((last(), "disconnected"));
    }
    let (boot, reply) = match vm.sessions_request(action).await {
        Ok(answer) => answer,
        Err(RuntimeError::Busy) => return Err((last(), "unavailable")),
        Err(_) => return Err((last(), "disconnected")),
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
    let mut unsupported = 0;
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
            let entry = match session(item, ORKA_NAMESPACE).map_err(|_| bad)? {
                Listed::Shown(entry) => entry,
                Listed::Gateway => continue,
                Listed::Unsupported => {
                    unsupported += 1;
                    continue;
                }
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
            return Ok((answered, newest(entries, true, unsupported)));
        };
        if entries.len() >= MAX_SESSIONS {
            return Ok((answered, newest(entries, false, unsupported)));
        }
        if !cursors.insert(next.clone()) {
            return Err(bad);
        }
    }
    Ok((
        boot.unwrap_or_default(),
        newest(entries, false, unsupported),
    ))
}
/// This computer's Sessions. A Task holding a Session is named, but its Agent is not read.
pub async fn read(vm: &impl LocalSessions) -> PlaceSessions {
    let mut view = PlaceSessions {
        key: LOCAL.into(),
        state: "disconnected",
        observed_at: None,
        entries: Vec::new(),
        connection: None,
        unsupported: 0,
    };
    match tokio::time::timeout(LOCAL_LIMIT, list(vm))
        .await
        .unwrap_or_else(|_| Err((vm.last_boot().map(connection), "unavailable")))
    {
        Ok((boot, list)) => {
            view.state = if list.complete { "complete" } else { "partial" };
            view.observed_at = Some(crate::overview::now());
            view.entries = list.entries;
            view.unsupported = list.unsupported;
            view.connection = Some(boot);
        }
        Err((boot, state)) => {
            view.state = state;
            view.connection = boot;
        }
    }
    view
}
/// Drop a result read on a boot that has since been replaced. A stop keeps the boot, so it keeps
/// the rows; a new boot forgets it, so rows from the old one are never shown as current.
pub fn recheck(vm: &impl LocalSessions, view: &mut PlaceSessions) {
    let now = vm.last_boot().map(connection);
    if view.connection.is_some() && view.connection != now {
        view.state = "disconnected";
        view.observed_at = None;
        view.entries.clear();
        view.unsupported = 0;
        view.connection = now;
    }
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

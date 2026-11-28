//! Native, memory-only conversations. No renderer paths, specs or transport authority.
use crate::{
    agent_folder::{AgentFolder, DocumentId},
    chat_cluster::{ChatPermission, ClusterChannel, ClusterWire, Refusal},
    engine::run::{ParsedAgent, ResourceId},
    local_vm::{LocalVm, LocalVmStatus},
    runtime::types::RuntimeError,
};
use bridge_runtime_protocol::{
    ChatFamily, ChatMethod, ChatRequest, ModelConnection, ModelPhase, ModelSelection, OrkaReply,
    Uuid, CHAT_FAMILY, MAX_ORKA_BODY, NAVIGATOR_FAMILY, ORKA_NAMESPACE,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{
    sync::{watch, OwnedMutexGuard},
    time::Instant,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ChatState {
    Ready,
    Preparing,
    Working,
    WaitingForInput,
    WaitingForSession,
    ObservationLost,
    RequestingStop,
    StopRequested,
    StopUnconfirmed,
    ContextUnavailable,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ChatError {
    InvalidMessage,
    Busy,
    Source,
    UnsupportedTool,
    Model,
    Connection,
    Identity,
    Response,
    ContextUnavailable,
    WorkspaceChanged,
    Interrupted,
    /// The cluster refused a request; the snapshot's `denied` names the permission.
    Permission,
    /// The cluster's Orka does not keep Sessions, so it cannot hold a conversation.
    SessionsUnsupported,
    /// The selected cluster is not connected in this run, or its access could not be rechecked.
    ClusterUnavailable,
    /// This computer's workspace image predates the Navigator; nothing was sent to it.
    WorkspaceOutdated,
    /// This computer's workspace stopped serving its channel; it needs a restart.
    WorkspaceUnavailable,
    /// This place's conversation was started on another AI connection; start a new one.
    PlaceChanged,
    /// The selected cluster's access can no longer authenticate and needs reconnecting.
    AccessLost,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolObservation {
    pub name: String,
    pub call_id: String,
    pub phase: String,
}
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatTurn {
    pub message: String,
    pub answer: Option<String>,
    pub phase: String,
    pub tools: Vec<ToolObservation>,
    pub context_retained: bool,
    pub termination_unconfirmed: bool,
    pub stop_unconfirmed: bool,
    /// Unix milliseconds when the turn was admitted.
    pub started_at: u64,
    /// Unix milliseconds when a terminal Task phase was first observed.
    pub finished_at: Option<u64>,
}
#[derive(Clone, Debug, Default, Serialize)]
pub struct ChatVersion {
    pub turns: Vec<ChatTurn>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatSnapshot {
    pub document_id: DocumentId,
    pub state: ChatState,
    pub busy: bool,
    /// Local admission only; Send also checks the VM and saved source natively.
    pub can_send: bool,
    pub versions: Vec<ChatVersion>,
    pub error: Option<ChatError>,
    /// With `Permission`: what the cluster refused. Null otherwise.
    pub denied: Option<ChatPermission>,
    /// Latched once a cluster refused to delete a replaced conversation's resources.
    pub kept_on_cluster: bool,
    /// The conversation was started on another AI connection of this cluster: shown, not continued.
    pub read_only: bool,
}
struct Entry {
    view: ChatSnapshot,
    context: Option<Context>,
    lease: Option<Arc<OwnedMutexGuard<()>>>,
    stop: watch::Sender<bool>,
    /// The model this conversation last thought with, and whether on a cluster. Display only.
    thinks: Option<(ModelSelection, bool)>,
    used: u64,
}
impl Entry {
    fn new(id: DocumentId) -> Self {
        Self {
            view: ChatSnapshot {
                document_id: id,
                state: ChatState::Ready,
                busy: false,
                can_send: true,
                versions: vec![],
                error: None,
                denied: None,
                kept_on_cluster: false,
                read_only: false,
            },
            context: None,
            lease: None,
            stop: watch::channel(false).0,
            thinks: None,
            used: 0,
        }
    }
    /// The view for a selected AI connection: a cluster conversation started on another one is
    /// read-only until New.
    fn snapshot(&self, provider: Option<&str>) -> ChatSnapshot {
        let mut view = self.view.clone();
        view.read_only = !view.busy
            && matches!(
                (provider, self.context.as_ref().map(|c| &c.binding)),
                (Some(selected), Some(Binding::Cluster { provider, .. })) if provider != selected
            );
        if view.read_only {
            view.can_send = false;
        }
        view
    }
}
impl Entry {
    fn finish(&mut self, safe: bool, success: bool) {
        // Publish completion under the same lock as Stop admission.
        if success && safe {
            self.view.state = ChatState::Ready;
        }
        self.view.busy = false;
        self.view.can_send = safe && self.view.state == ChatState::Ready;
    }
}
/// What a conversation's Agent is built from. Both are read natively; the renderer
/// supplies only the message.
pub enum ChatSource {
    /// A saved agent file, captured when the message is sent.
    Document(Arc<AgentFolder>),
    /// The Navigator's fixed instructions with this summary of the saved agents.
    Navigator(String),
}
impl ChatSource {
    fn family(&self) -> ChatFamily {
        match self {
            Self::Document(_) => CHAT_FAMILY,
            Self::Navigator(_) => NAVIGATOR_FAMILY,
        }
    }
}
/// Where a conversation's requests go: the local workspace's VM channel, or the selected
/// cluster's confirmed Orka connection. One state machine drives both.
#[derive(Clone)]
pub enum Channel {
    Local(Arc<LocalVm>),
    Cluster(Arc<ClusterChannel>),
}
#[derive(Clone, PartialEq)]
enum Binding {
    Local {
        installation: Uuid,
        boot: Uuid,
        connection: ModelConnection,
    },
    /// A remembered place, its watched namespace and the chosen AI connection.
    Cluster {
        key: String,
        namespace: String,
        provider: String,
    },
}
impl Binding {
    fn provider(&self) -> &str {
        match self {
            Self::Local { connection, .. } => &connection.selection.provider,
            Self::Cluster { provider, .. } => provider,
        }
    }
}
struct Planned {
    body: Value,
    receipt: Option<ResourceId>,
    sent: bool,
}
impl Planned {
    fn new(prefix: &str, namespace: &str, spec: Value) -> Self {
        Self {
            body: json!({"name":format!("{prefix}{}",Uuid::new_v4().simple()),"namespace":namespace,"spec":spec}),
            receipt: None,
            sent: false,
        }
    }
    fn name(&self) -> &str {
        self.body["name"].as_str().unwrap()
    }
    fn request(&self, path: &str) -> ChatRequest {
        request(ChatMethod::Post, path, Some(self.body.clone()), None)
    }
}
struct Context {
    source: Vec<u8>,
    binding: Binding,
    agent: Planned,
    task: Planned,
    session: String,
    cursor: u64,
    deleted: bool,
    stopping: bool,
    /// A cluster conversation's channel, kept for Reconnect, Stop and cleanup.
    cluster: Option<Arc<ClusterChannel>>,
    /// This conversation's earlier acknowledged Tasks, for cleanup on a cluster.
    earlier: Vec<ResourceId>,
    /// A refusal ended it before it held a turn: the next message starts a fresh one.
    broken: bool,
}
/// Where a conversation lives. Each place keeps its own conversation per source, so switching
/// the selected place never shows, continues or confirms another place's turns.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Place {
    ThisComputer,
    /// A remembered cluster, by its place key.
    Cluster(String),
}
type Key = (DocumentId, Place);
/// Conversations kept in memory per owner. Beyond it, the least recently used one that is idle
/// and holds nothing to settle is dropped, its cluster resources following the rule for New.
pub const MAX_CONVERSATIONS: usize = 8;
impl Channel {
    pub fn place(&self) -> Place {
        match self {
            Self::Local(_) => Place::ThisComputer,
            Self::Cluster(cluster) => Place::Cluster(cluster.key.clone()),
        }
    }
    fn provider(&self) -> Option<&str> {
        match self {
            Self::Local(_) => None,
            Self::Cluster(cluster) => Some(&cluster.provider),
        }
    }
}
pub struct ChatOwner(
    Mutex<HashMap<Key, Entry>>,
    tokio::runtime::Handle,
    /// The local workspace's epoch; replacing the workspace retires only local conversations.
    AtomicU64,
    /// Recency for the in-memory cap.
    AtomicU64,
);
impl ChatOwner {
    pub fn new(runtime: tokio::runtime::Handle) -> Arc<Self> {
        Arc::new(Self(
            Mutex::default(),
            runtime,
            AtomicU64::new(0),
            AtomicU64::new(0),
        ))
    }
    /// The model this place's conversation last thought with, and whether on a cluster.
    pub fn thinks_with_on(&self, id: DocumentId, place: &Place) -> Option<(ModelSelection, bool)> {
        self.0
            .lock()
            .unwrap()
            .get(&(id, place.clone()))
            .and_then(|e| e.thinks.clone())
    }
    /// The model the local conversation last thought with (this computer).
    pub fn thinks_with(&self) -> Option<ModelSelection> {
        self.thinks_with_on(crate::navigator::CONVERSATION, &Place::ThisComputer)
            .map(|(selection, _)| selection)
    }
    /// Forget local conversations without sending cleanup to either workspace. Cluster
    /// conversations do not depend on the workspace and are kept.
    pub fn retire_for_workspace_replacement(&self) {
        let mut entries = self.0.lock().unwrap();
        for ((_, place), entry) in entries.iter() {
            if *place == Place::ThisComputer {
                entry.stop.send_replace(true);
            }
        }
        self.2.fetch_add(1, Ordering::Relaxed);
        entries.retain(|(_, place), _| *place != Place::ThisComputer);
    }
    /// An empty conversation for a place that can't be read now, saying why.
    pub fn unavailable(id: DocumentId, error: ChatError) -> ChatSnapshot {
        let mut view = Entry::new(id).view;
        view.can_send = false;
        view.error = Some(error);
        view
    }
    pub fn status(&self, id: DocumentId) -> ChatSnapshot {
        self.status_at(id, &Place::ThisComputer, None)
    }
    /// The conversation of `channel`'s place. A cluster conversation started on another AI
    /// connection is shown read-only until New.
    pub fn status_on(&self, id: DocumentId, channel: &Channel) -> ChatSnapshot {
        self.status_at(id, &channel.place(), channel.provider())
    }
    fn status_at(&self, id: DocumentId, place: &Place, provider: Option<&str>) -> ChatSnapshot {
        self.0
            .lock()
            .unwrap()
            .get(&(id, place.clone()))
            .map(|e| e.snapshot(provider))
            .unwrap_or_else(|| Entry::new(id).view)
    }
    pub fn send(
        self: &Arc<Self>,
        vm: Arc<LocalVm>,
        folder: Arc<AgentFolder>,
        message: String,
    ) -> Result<ChatSnapshot, ChatError> {
        self.send_on(Channel::Local(vm), folder, message)
    }
    /// One message on the selected place: the local workspace or a connected cluster.
    pub fn send_on(
        self: &Arc<Self>,
        channel: Channel,
        folder: Arc<AgentFolder>,
        message: String,
    ) -> Result<ChatSnapshot, ChatError> {
        let id = folder.document_id();
        self.admit(channel, id, ChatSource::Document(folder), message)
    }
    /// One message to the Navigator's single conversation, built from the bundled
    /// instructions and `saved_agents`, a native summary of the saved agents.
    pub fn send_navigator(
        self: &Arc<Self>,
        vm: Arc<LocalVm>,
        saved_agents: String,
        message: String,
    ) -> Result<ChatSnapshot, ChatError> {
        self.send_navigator_on(Channel::Local(vm), saved_agents, message)
    }
    pub fn send_navigator_on(
        self: &Arc<Self>,
        channel: Channel,
        saved_agents: String,
        message: String,
    ) -> Result<ChatSnapshot, ChatError> {
        self.admit(
            channel,
            crate::navigator::CONVERSATION,
            ChatSource::Navigator(saved_agents),
            message,
        )
    }
    fn admit(
        self: &Arc<Self>,
        channel: Channel,
        id: DocumentId,
        source: ChatSource,
        message: String,
    ) -> Result<ChatSnapshot, ChatError> {
        if message.trim().is_empty() || message.len() > MAX_ORKA_BODY {
            return Err(ChatError::InvalidMessage);
        }
        let mut entries = self.0.lock().unwrap();
        let key = (id, channel.place());
        if entries
            .get(&key)
            .is_some_and(|e| e.snapshot(channel.provider()).read_only)
        {
            return Err(ChatError::PlaceChanged);
        }
        let lease = match &channel {
            // Retirement shares this lock: capture the lease and launch epoch together,
            // and admit nothing in the gap before the replacement VM is ready.
            Channel::Local(vm) => {
                let lease = vm.try_authoring().ok_or(ChatError::Busy)?;
                if vm.status() != LocalVmStatus::Ready || !vm.authoring_current(&lease) {
                    return Err(ChatError::Busy);
                }
                lease
            }
            // A cluster conversation holds no workspace authority; its lease only marks a
            // retained context that Reconnect or Stop must settle.
            Channel::Cluster(_) => Arc::new(tokio::sync::Mutex::new(()))
                .try_lock_owned()
                .map_err(|_| ChatError::Busy)?,
        };
        if !entries.contains_key(&key) {
            self.make_room(&mut entries, &key);
        }
        let e = entries.entry(key.clone()).or_insert_with(|| Entry::new(id));
        if e.view.busy || e.lease.is_some() {
            return Err(ChatError::Busy);
        }
        if e.view.state == ChatState::ContextUnavailable {
            return Err(ChatError::ContextUnavailable);
        }
        e.used = self.3.fetch_add(1, Ordering::Relaxed);
        e.lease = Some(Arc::new(lease));
        e.view.busy = true;
        e.view.can_send = false;
        e.view.state = ChatState::Preparing;
        e.view.error = None;
        e.view.denied = None;
        e.stop = watch::channel(false).0;
        self.launch(channel, key, e, Some((source, message)));
        Ok(e.view.clone())
    }
    /// Drops the least recently used idle conversation with nothing to settle when the cap is
    /// reached. Its cluster resources are deleted or kept exactly as New would.
    fn make_room(self: &Arc<Self>, entries: &mut HashMap<Key, Entry>, incoming: &Key) {
        if entries.len() < MAX_CONVERSATIONS {
            return;
        }
        let oldest = entries
            .iter()
            .filter(|(key, e)| *key != incoming && !e.view.busy && e.lease.is_none())
            .min_by_key(|(_, e)| e.used)
            .map(|(key, _)| key.clone());
        if let Some(key) = oldest {
            if let Some(mut dropped) = entries.remove(&key) {
                if let Some(context) = dropped.context.take() {
                    self.clean_up(key, context);
                }
            }
        }
    }
    pub fn reconnect(
        self: &Arc<Self>,
        vm: Arc<LocalVm>,
        id: DocumentId,
    ) -> Result<ChatSnapshot, ChatError> {
        self.resume(Channel::Local(vm), id, false)
    }
    pub fn stop(
        self: &Arc<Self>,
        vm: Arc<LocalVm>,
        id: DocumentId,
    ) -> Result<ChatSnapshot, ChatError> {
        self.resume(Channel::Local(vm), id, true)
    }
    /// Reconnect or Stop the conversation of `channel`'s place, wherever its turn runs.
    pub fn reconnect_on(
        self: &Arc<Self>,
        channel: Channel,
        id: DocumentId,
    ) -> Result<ChatSnapshot, ChatError> {
        self.resume(channel, id, false)
    }
    pub fn stop_on(
        self: &Arc<Self>,
        channel: Channel,
        id: DocumentId,
    ) -> Result<ChatSnapshot, ChatError> {
        self.resume(channel, id, true)
    }
    fn resume(
        self: &Arc<Self>,
        channel: Channel,
        id: DocumentId,
        stop: bool,
    ) -> Result<ChatSnapshot, ChatError> {
        let mut entries = self.0.lock().unwrap();
        let key = (id, channel.place());
        let e = entries.get_mut(&key).ok_or(ChatError::ContextUnavailable)?;
        if e.view.busy {
            if stop {
                e.stop.send_replace(true);
                e.view.state = ChatState::RequestingStop;
            }
            return Ok(e.view.clone());
        }
        if e.lease.is_none() {
            return Err(ChatError::ContextUnavailable);
        }
        e.used = self.3.fetch_add(1, Ordering::Relaxed);
        e.stop = watch::channel(stop).0;
        e.view.busy = true;
        e.view.error = None;
        e.view.denied = None;
        if stop {
            e.view.state = ChatState::RequestingStop;
        }
        // A retained cluster conversation keeps the channel it was admitted on.
        let channel = match e.context.as_ref().and_then(|c| c.cluster.clone()) {
            Some(cluster) => Channel::Cluster(cluster),
            None => channel,
        };
        self.launch(channel, key, e, None);
        Ok(e.view.clone())
    }
    pub fn new_conversation(self: &Arc<Self>, id: DocumentId) -> Result<ChatSnapshot, ChatError> {
        self.new_at(id, Place::ThisComputer)
    }
    /// New on `channel`'s place only; other places' conversations are untouched.
    pub fn new_conversation_on(
        self: &Arc<Self>,
        channel: &Channel,
        id: DocumentId,
    ) -> Result<ChatSnapshot, ChatError> {
        self.new_at(id, channel.place())
    }
    fn new_at(self: &Arc<Self>, id: DocumentId, place: Place) -> Result<ChatSnapshot, ChatError> {
        let mut entries = self.0.lock().unwrap();
        let key = (id, place);
        if !entries.contains_key(&key) {
            self.make_room(&mut entries, &key);
        }
        let e = entries.entry(key.clone()).or_insert_with(|| Entry::new(id));
        if e.view.busy || e.lease.is_some() {
            return Err(ChatError::Busy);
        }
        e.used = self.3.fetch_add(1, Ordering::Relaxed);
        if let Some(previous) = e.context.take() {
            self.clean_up(key, previous);
        }
        e.thinks = None;
        e.view.state = ChatState::Ready;
        e.view.can_send = true;
        e.view.error = None;
        e.view.denied = None;
        Ok(e.view.clone())
    }
    /// Deletes what a replaced cluster conversation created, when the role allows. Each
    /// resource is identified by its generated name and, for the Task and Agent, the UID
    /// acknowledged at creation; a refusal leaves everything and is said once. Local
    /// conversations are only forgotten, as before.
    fn clean_up(self: &Arc<Self>, key: Key, context: Context) {
        let Some(cluster) = context.cluster.clone() else {
            return;
        };
        let owner = self.clone();
        let epoch = self.2.load(Ordering::Relaxed);
        self.1.spawn(async move {
            let kept = clean_up_on(&cluster, &context).await;
            if kept {
                owner.edit(&key, epoch, |s| s.kept_on_cluster = true);
            }
        });
    }
    /// Local edits are fenced by the workspace epoch; cluster conversations outlive it.
    fn edit(&self, key: &Key, epoch: u64, f: impl FnOnce(&mut ChatSnapshot)) {
        let mut entries = self.0.lock().unwrap();
        if key.1 != Place::ThisComputer || self.2.load(Ordering::Relaxed) == epoch {
            if let Some(entry) = entries.get_mut(key) {
                f(&mut entry.view);
            }
        }
    }
    fn set_thinks(&self, key: &Key, epoch: u64, thinks: (ModelSelection, bool)) {
        let mut entries = self.0.lock().unwrap();
        if key.1 != Place::ThisComputer || self.2.load(Ordering::Relaxed) == epoch {
            if let Some(entry) = entries.get_mut(key) {
                entry.thinks = Some(thinks);
            }
        }
    }
    fn current(&self, key: &Key, epoch: u64) -> bool {
        key.1 != Place::ThisComputer || self.2.load(Ordering::Relaxed) == epoch
    }
    fn launch(
        self: &Arc<Self>,
        channel: Channel,
        id: Key,
        entry: &mut Entry,
        send: Option<(ChatSource, String)>,
    ) {
        let owner = self.clone();
        let mut worker = Worker {
            owner: owner.clone(),
            channel,
            wire: None,
            id: id.clone(),
            epoch: self.2.load(Ordering::Relaxed),
            context: entry.context.take(),
            lease: entry.lease.take().unwrap(),
            stop: entry.stop.subscribe(),
            deadline: Instant::now() + Duration::from_secs(150),
            stopping: false,
            safe: send.is_some(),
            preflight: send.is_some(),
        };
        self.1.spawn(async move {
            let mut result = worker.connect().await;
            if result.is_ok() {
                result = worker.run(send).await;
            }
            if result == Err(ChatError::Interrupted) && !worker.preflight {
                result = worker.stop_task().await;
            }
            if let Some(wire) = worker.wire.take() {
                wire.close().await;
            }
            let mut entries = owner.0.lock().unwrap();
            // Retirement can remove and recreate the same document while this
            // detached worker is awaiting a response. Never restore its old lease.
            if !owner.current(&id, worker.epoch) {
                return;
            }
            let Some(e) = entries.get_mut(&id) else {
                return;
            };
            // A refusal that created nothing ends the turn plainly, never as unconfirmed; the
            // next message starts a fresh conversation instead of resuming this one.
            let refused = worker.safe
                && result.as_ref().err().is_some_and(|error| {
                    matches!(
                        error,
                        ChatError::Permission
                            | ChatError::SessionsUnsupported
                            | ChatError::WorkspaceOutdated
                            | ChatError::WorkspaceUnavailable
                            | ChatError::ClusterUnavailable
                            | ChatError::AccessLost
                    )
                });
            // Before preflight passes, the previous conversation is still the one held.
            if refused && !worker.preflight {
                if let Some(context) = worker.context.as_mut() {
                    context.broken = true;
                }
            }
            if let Err(error) = result {
                e.view.error = Some(error);
                e.view.state = if refused
                    || worker.preflight
                        && matches!(
                            error,
                            ChatError::InvalidMessage
                                | ChatError::Source
                                | ChatError::UnsupportedTool
                                | ChatError::Model
                                | ChatError::Connection
                                | ChatError::Interrupted
                                | ChatError::Permission
                                | ChatError::SessionsUnsupported
                                | ChatError::ClusterUnavailable
                                | ChatError::AccessLost
                                | ChatError::WorkspaceOutdated
                                | ChatError::WorkspaceUnavailable
                        ) {
                    ChatState::Ready
                } else if worker.safe || error == ChatError::WorkspaceChanged {
                    ChatState::ContextUnavailable
                } else if worker.stopping
                    || *worker.stop.borrow()
                    || worker.context.as_ref().is_some_and(|c| c.stopping)
                {
                    ChatState::StopUnconfirmed
                } else {
                    ChatState::ObservationLost
                };
            }
            e.context = worker.context;
            e.lease = if worker.safe || result == Err(ChatError::WorkspaceChanged) {
                drop(worker.lease);
                None
            } else {
                Some(worker.lease)
            };
            e.finish(worker.safe, result.is_ok() || refused);
        });
    }
}
struct Worker {
    owner: Arc<ChatOwner>,
    channel: Channel,
    /// A cluster conversation's connection for this one worker run.
    wire: Option<Arc<dyn ClusterWire>>,
    id: Key,
    epoch: u64,
    context: Option<Context>,
    lease: Arc<OwnedMutexGuard<()>>,
    stop: watch::Receiver<bool>,
    deadline: Instant,
    stopping: bool,
    safe: bool,
    preflight: bool,
}
impl Worker {
    fn state(&self, state: ChatState) {
        self.owner.edit(&self.id, self.epoch, |s| s.state = state);
    }
    fn turn(&self, f: impl FnOnce(&mut ChatTurn)) {
        self.owner.edit(&self.id, self.epoch, |s| {
            if let Some(t) = s.versions.last_mut().and_then(|v| v.turns.last_mut()) {
                f(t);
            }
        });
    }
    async fn pause(&self) -> Result<(), ChatError> {
        if Instant::now() >= self.deadline {
            return Err(ChatError::Connection);
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
        Ok(())
    }
    async fn borrow<T: Send + 'static>(
        &self,
        retained: bool,
        work: impl Fn(Uuid, &mut crate::runtime::client::VmClient) -> Result<T, ChatError>
            + Send
            + Sync
            + 'static,
    ) -> Result<T, ChatError> {
        let work = Arc::new(work);
        loop {
            if !self.stopping && *self.stop.borrow() {
                return Err(ChatError::Interrupted);
            }
            if Instant::now() >= self.deadline {
                return Err(ChatError::Connection);
            }
            let Channel::Local(vm) = &self.channel else {
                return Err(ChatError::WorkspaceChanged);
            };
            let (work, vm, lease) = (work.clone(), vm.clone(), self.lease.clone());
            let (owner, epoch) = (self.owner.clone(), self.epoch);
            let binding = self
                .context
                .as_ref()
                .filter(|_| retained)
                .map(|c| c.binding.clone());
            match vm
                .clone()
                .with_client(move |id, client| {
                    // Check admission *inside* the serialized borrow: startup resets its epoch.
                    if owner.2.load(Ordering::Relaxed) != epoch
                        || !vm.authoring_current(&lease)
                        || binding.is_some_and(|b| match b {
                            Binding::Local {
                                installation, boot, ..
                            } => id != installation || client.boot_nonce() != boot,
                            Binding::Cluster { .. } => true,
                        })
                    {
                        return Err(ChatError::WorkspaceChanged);
                    }
                    work(id, client)
                })
                .await
            {
                Err(RuntimeError::Busy) => self.pause().await?,
                Ok(result) => return result,
                Err(_) => return Err(ChatError::Connection),
            }
        }
    }
    /// Opens a cluster conversation's connection for this run. The local channel needs none.
    async fn connect(&mut self) -> Result<(), ChatError> {
        if let Channel::Cluster(cluster) = &self.channel {
            self.wire = Some(open(cluster).await?);
        }
        Ok(())
    }
    fn namespace(&self) -> &str {
        match &self.channel {
            Channel::Local(_) => ORKA_NAMESPACE,
            Channel::Cluster(cluster) => &cluster.namespace,
        }
    }
    fn cluster(&self) -> Option<&Arc<ClusterChannel>> {
        match &self.channel {
            Channel::Local(_) => None,
            Channel::Cluster(cluster) => Some(cluster),
        }
    }
    async fn binding(&self, retained: bool, navigator: bool) -> Result<Binding, ChatError> {
        let previous = self
            .context
            .as_ref()
            .filter(|_| retained)
            .map(|c| c.binding.clone());
        if let Some(cluster) = self.cluster() {
            let binding = Binding::Cluster {
                key: cluster.key.clone(),
                namespace: cluster.namespace.clone(),
                provider: cluster.provider.clone(),
            };
            // A retained context only ever resumes on the channel it was admitted on.
            return match previous {
                Some(previous) if previous != binding => Err(ChatError::WorkspaceChanged),
                _ => Ok(binding),
            };
        }
        self.borrow(retained, move |installation, client| {
            if let Some(binding) = &previous {
                return Ok(binding.clone());
            }
            // A guest that predates the Navigator's names stops serving its channel on the
            // first such request, so none is ever sent to it.
            if navigator && !client.navigator_chat() {
                return Err(ChatError::WorkspaceOutdated);
            }
            let model = client.model_status().map_err(|_| lost(client))?;
            if model.phase != ModelPhase::Ready {
                return Err(ChatError::Model);
            }
            Ok(Binding::Local {
                installation,
                boot: client.boot_nonce(),
                connection: model.connection.ok_or(ChatError::Model)?,
            })
        })
        .await
    }
    async fn request(&self, r: ChatRequest) -> Result<OrkaReply, ChatError> {
        let Some(cluster) = self.cluster() else {
            return self
                .borrow(true, move |_, client| {
                    if r.navigator() && !client.navigator_chat() {
                        return Err(ChatError::WorkspaceOutdated);
                    }
                    client.chat_request(r.clone()).map_err(|_| lost(client))
                })
                .await;
        };
        // The same admission checks the local borrow makes before every request.
        if !self.stopping && *self.stop.borrow() {
            return Err(ChatError::Interrupted);
        }
        if Instant::now() >= self.deadline {
            return Err(ChatError::Connection);
        }
        let wire = self.wire.as_ref().ok_or(ChatError::ClusterUnavailable)?;
        let sent = crate::chat_cluster::send(&**wire, &cluster.namespace, &cluster.provider, &r);
        match tokio::time::timeout_at(self.deadline, sent).await {
            Err(_) => Err(ChatError::Connection),
            Ok(Ok(reply)) => Ok(reply),
            Ok(Err(Refusal::Failed(error))) => Err(error),
            Ok(Err(Refusal::Denied(permission))) => {
                self.owner
                    .edit(&self.id, self.epoch, |s| s.denied = Some(permission));
                Err(ChatError::Permission)
            }
        }
    }
    /// The chosen AI connection's default model, which the Navigator thinks with on a cluster.
    async fn default_model(&self) -> Result<String, ChatError> {
        let cluster = self.cluster().ok_or(ChatError::Model)?;
        let reply = self.get(&format!("providers/{}", cluster.provider)).await?;
        crate::chat_cluster::default_model(&reply, &cluster.namespace, &cluster.provider)
    }
    async fn get(&self, path: &str) -> Result<OrkaReply, ChatError> {
        self.request(request(ChatMethod::Get, path, None, None))
            .await
    }
    async fn checked(&self, id: &ResourceId) -> Result<Option<Value>, ChatError> {
        let reply = self.get(&id.path()).await?;
        match reply.status {
            200 => {
                id.unchanged(&reply.body).map_err(|_| ChatError::Identity)?;
                Ok(Some(reply.body))
            }
            404 => Ok(None),
            _ => Err(ChatError::Response),
        }
    }
    async fn run(&mut self, send: Option<(ChatSource, String)>) -> Result<(), ChatError> {
        // Identify the boot before sending *any* retained name, including during reconnect.
        let navigator = matches!(send, Some((ChatSource::Navigator(_), _)));
        let binding = self.binding(send.is_none(), navigator).await?;
        if let Binding::Local { connection, .. } = &binding {
            self.owner
                .set_thinks(&self.id, self.epoch, (connection.selection.clone(), false));
        }
        if let Some((origin, message)) = send {
            let family = origin.family();
            let source = match origin {
                ChatSource::Document(folder) => {
                    tokio::task::spawn_blocking(move || folder.capture_agent())
                        .await
                        .map_err(|_| ChatError::Source)?
                        .map_err(|_| ChatError::Source)?
                }
                // A Navigator conversation keeps the instructions it started with, so a newly
                // saved agent does not cost it its Session. A new model starts a new one.
                ChatSource::Navigator(saved) => {
                    match self.context.as_ref().filter(|c| c.binding == binding) {
                        Some(c) => c.source.clone(),
                        None => match &binding {
                            Binding::Local { connection, .. } => {
                                crate::navigator::agent_source(&connection.selection.model, &saved)
                            }
                            Binding::Cluster { .. } => {
                                crate::navigator::agent_source(&self.default_model().await?, &saved)
                            }
                        },
                    }
                }
            };
            let parsed = ParsedAgent::parse(&source).map_err(|_| ChatError::Source)?;
            if parsed.has_unknown_local_builtin() {
                return Err(ChatError::Source);
            }
            if parsed.has_unsupported_local_tool() {
                return Err(ChatError::UnsupportedTool);
            }
            if let Binding::Cluster { provider, .. } = &binding {
                let selection = ModelSelection {
                    vendor: None,
                    provider: provider.clone(),
                    model: parsed.model().to_owned(),
                };
                self.owner
                    .set_thinks(&self.id, self.epoch, (selection, true));
            }
            let fresh = self
                .context
                .as_ref()
                .is_none_or(|c| c.broken || c.source != source || c.binding != binding);
            let session = if fresh {
                format!("{}{}", family.session, Uuid::new_v4().simple())
            } else {
                self.context.as_ref().unwrap().session.clone()
            };
            if !fresh && !self.session(self.context.as_ref().unwrap().deleted).await? {
                return Err(ChatError::ContextUnavailable);
            }
            let agent = if fresh {
                Planned::new(
                    family.agent,
                    self.namespace(),
                    parsed.spec(binding.provider()),
                )
            } else {
                let c = self.context.as_ref().unwrap();
                let receipt = c.agent.receipt.as_ref().ok_or(ChatError::Identity)?;
                self.checked(receipt).await?.ok_or(ChatError::Identity)?;
                Planned {
                    body: c.agent.body.clone(),
                    receipt: Some(receipt.clone()),
                    sent: true,
                }
            };
            let mut task_spec = json!({"type":"ai","agentRef":{"name":agent.name()},"prompt":message,"timeout":"120s","retryPolicy":{"maxRetries":0},"sessionRef":{"name":session,"append":true,"maxMessages":50}});
            // Go omits false Create; only the first turn may create a Session.
            if fresh {
                task_spec["sessionRef"]["create"] = json!(true);
            }
            let task = Planned::new(family.task, self.namespace(), task_spec);
            // Validate both *whole* create bodies before the first mutation.
            let check = |r: ChatRequest| match self.cluster() {
                None => r.validate(),
                Some(c) => r.validate_cluster(&c.namespace, &c.provider),
            };
            check(agent.request("agents")).map_err(|_| ChatError::Source)?;
            check(task.request("tasks")).map_err(|_| ChatError::InvalidMessage)?;
            // Preflight rejection must leave the preceding conversation usable.
            self.preflight = false;
            let mut earlier = Vec::new();
            if fresh {
                // A replaced cluster conversation's resources are deleted when the role allows.
                if let Some(previous) = self.context.take() {
                    self.owner.clean_up(self.id.clone(), previous);
                }
            } else if let Some(previous) = self.context.take() {
                earlier = previous.earlier;
                earlier.extend(previous.task.receipt);
            }
            self.context = Some(Context {
                source,
                binding,
                agent,
                task,
                session,
                cursor: 0,
                deleted: false,
                stopping: false,
                cluster: self.cluster().cloned(),
                earlier,
                broken: false,
            });
            self.owner.edit(&self.id, self.epoch, |s| {
                if fresh {
                    s.versions.push(ChatVersion::default());
                }
                s.versions.last_mut().unwrap().turns.push(ChatTurn {
                    message,
                    started_at: unix_millis(),
                    ..ChatTurn::default()
                });
            });
            self.adopt(true, true).await?;
            self.safe = true;
            self.adopt(false, true).await?;
        } else {
            if self.context.as_ref().is_some_and(|c| c.stopping) || *self.stop.borrow() {
                return self.stop_task().await;
            }
            self.adopt(true, false).await?;
            if !self.context.as_ref().unwrap().task.sent {
                self.safe = true;
                return Err(ChatError::ContextUnavailable);
            }
            self.adopt(false, false).await?;
        }
        self.observe().await
    }
    async fn adopt(&mut self, agent: bool, create: bool) -> Result<(), ChatError> {
        let c = self.context.as_ref().unwrap();
        let p = if agent { &c.agent } else { &c.task };
        if p.receipt.is_some() {
            return Ok(());
        }
        let collection = if agent { "agents" } else { "tasks" };
        let body = p.body.clone();
        let name = p.name().to_owned();
        let sent = p.sent;
        if create && !sent {
            let c = self.context.as_mut().unwrap();
            (if agent { &mut c.agent } else { &mut c.task }).sent = true;
            self.safe = agent; // An Agent create alone cannot be executing a turn.
            match self
                .request(request(
                    ChatMethod::Post,
                    collection,
                    Some(body.clone()),
                    None,
                ))
                .await
            {
                Ok(reply) if reply.status == 201 => {
                    // An Orka without Sessions drops the reference and runs a one-off Task.
                    if !agent
                        && self.cluster().is_some()
                        && reply.body["spec"].get("sessionRef").is_none()
                    {
                        return self.without_sessions(reply.body).await;
                    }
                    return self.ack(agent, reply.body);
                }
                // A refused create created nothing.
                Err(ChatError::Permission) => {
                    self.safe = true;
                    return Err(ChatError::Permission);
                }
                Ok(reply)
                    if (400..500).contains(&reply.status)
                        && reply.status != 408
                        && reply.status != 409
                        && reply.status != 429 =>
                {
                    self.safe = true;
                    return Err(ChatError::Response);
                }
                Err(error @ (ChatError::Interrupted | ChatError::WorkspaceChanged)) => {
                    return Err(error)
                }
                _ => {} // A dispatched POST is never retried, even when its reply is lost.
            }
        }
        let reply = self.get(&format!("{collection}/{name}")).await?;
        if reply.status == 404 {
            if !agent {
                let c = self.context.as_mut().unwrap();
                c.deleted = true;
                self.safe = false;
                if c.stopping {
                    self.stopped();
                }
                return Ok(()); // Task absence alone does not establish Session release.
            }
            self.safe = true;
            return Err(ChatError::ContextUnavailable);
        }
        if reply.status != 200 {
            return Err(ChatError::Response);
        }
        self.ack(agent, reply.body)
    }
    fn ack(&mut self, agent: bool, value: Value) -> Result<(), ChatError> {
        let namespace = self.namespace().to_owned();
        let c = self.context.as_mut().unwrap();
        let p = if agent { &mut c.agent } else { &mut c.task };
        p.receipt = Some(
            ResourceId::acknowledge(
                &value,
                &p.body["spec"],
                p.name(),
                &namespace,
                if agent { "Agent" } else { "Task" },
            )
            .map_err(|_| ChatError::Identity)?,
        );
        Ok(())
    }
    /// The Task this cluster created can never join the conversation: delete it when it is
    /// exactly ours, and end the conversation with the precise reason.
    async fn without_sessions(&mut self, value: Value) -> Result<(), ChatError> {
        let namespace = self.namespace().to_owned();
        let c = self.context.as_mut().unwrap();
        let mut spec = c.task.body["spec"].clone();
        if let Some(spec) = spec.as_object_mut() {
            spec.remove("sessionRef");
        }
        if let Ok(task) = ResourceId::acknowledge(&value, &spec, c.task.name(), &namespace, "Task")
        {
            let _ = self
                .request(request(ChatMethod::Delete, &task.path(), None, None))
                .await;
        }
        self.context.as_mut().unwrap().deleted = true;
        self.safe = true;
        Err(ChatError::SessionsUnsupported)
    }
    async fn stop_task(&mut self) -> Result<(), ChatError> {
        self.stopping = true;
        if self.safe {
            return Err(ChatError::ContextUnavailable);
        }
        let Some(c) = self.context.as_mut() else {
            self.safe = true;
            return Err(ChatError::ContextUnavailable);
        };
        c.stopping = true;
        if !c.task.sent {
            self.safe = true;
            return Err(ChatError::ContextUnavailable);
        }
        let explicit = *self.stop.borrow();
        if c.task.receipt.is_none() {
            match self.adopt(false, false).await {
                Err(ChatError::ContextUnavailable) => self.context.as_mut().unwrap().deleted = true,
                other => other?,
            }
        }
        self.safe = false;
        if explicit && !self.context.as_ref().unwrap().deleted {
            self.state(ChatState::RequestingStop);
            let task = self
                .context
                .as_ref()
                .unwrap()
                .task
                .receipt
                .as_ref()
                .unwrap()
                .clone();
            if self.checked(&task).await?.is_none() {
                self.context.as_mut().unwrap().deleted = true;
                self.stopped();
            } else {
                let reply = self
                    .request(request(ChatMethod::Delete, &task.path(), None, None))
                    .await;
                if reply
                    .as_ref()
                    .is_ok_and(|r| r.status == 204 || r.status == 404)
                {
                    self.stopped();
                } else {
                    self.turn(|t| t.stop_unconfirmed = true);
                    self.state(ChatState::StopUnconfirmed);
                }
            }
        }
        self.observe().await
    }
    fn stopped(&self) {
        self.turn(|t| {
            t.termination_unconfirmed = true;
            t.stop_unconfirmed = false;
        });
        self.state(ChatState::StopRequested);
    }
    async fn observe(&mut self) -> Result<(), ChatError> {
        loop {
            let c = self.context.as_ref().unwrap();
            let stopping = c.stopping;
            let task = c.task.receipt.clone();
            if !c.deleted {
                let task = task.as_ref().ok_or(ChatError::Identity)?;
                if let Some(value) = self.checked(task).await? {
                    if !stopping {
                        let agent = self
                            .context
                            .as_ref()
                            .unwrap()
                            .agent
                            .receipt
                            .as_ref()
                            .unwrap()
                            .clone();
                        self.checked(&agent).await?.ok_or(ChatError::Identity)?;
                        // A just-created Task has no phase until controller reconciliation.
                        let phase = match value["status"].get("phase") {
                            None => "",
                            Some(phase) => phase.as_str().ok_or(ChatError::Response)?,
                        }
                        .to_owned();
                        let terminal =
                            matches!(phase.as_str(), "Succeeded" | "Failed" | "Cancelled");
                        self.turn(|t| {
                            t.phase = phase.clone();
                            if terminal && t.finished_at.is_none() {
                                t.finished_at = Some(unix_millis());
                            }
                        });
                        self.state(if phase == "WaitingForInput" || phase == "InputRequired" {
                            ChatState::WaitingForInput
                        } else {
                            ChatState::Working
                        });
                        if terminal {
                            let reply = self.get(&format!("{}/result", task.path())).await?;
                            match reply.status {
                                200 => {
                                    let answer = reply.body["result"]
                                        .as_str()
                                        .ok_or(ChatError::Response)?
                                        .to_owned();
                                    self.turn(|t| t.answer = Some(answer));
                                }
                                404 => {}
                                _ => return Err(ChatError::Response),
                            }
                        }
                        self.events(task).await?;
                        if terminal {
                            self.checked(task).await?.ok_or(ChatError::Identity)?;
                            self.checked(&agent).await?.ok_or(ChatError::Identity)?;
                            self.state(ChatState::WaitingForSession);
                            if self.session(false).await? {
                                return Ok(());
                            }
                        }
                    }
                } else {
                    self.context.as_mut().unwrap().deleted = true;
                    if stopping {
                        self.stopped();
                    }
                }
            }
            if self.context.as_ref().unwrap().deleted && self.session(true).await? {
                return Ok(());
            }
            self.pause().await?;
        }
    }
    async fn events(&mut self, task: &ResourceId) -> Result<(), ChatError> {
        loop {
            let after = self.context.as_ref().unwrap().cursor;
            let reply = self
                .request(request(
                    ChatMethod::Get,
                    &format!("{}/events", task.path()),
                    None,
                    Some(after),
                ))
                .await?;
            let p = &reply.body;
            let latest = p["latestSeq"].as_u64().ok_or(ChatError::Response)?;
            if reply.status != 200
                || p["namespace"] != self.namespace()
                || p["streamType"] != "task"
                || p["streamID"] != task.name
                || p["afterSeq"] != after
                || latest < after
            {
                return Err(ChatError::Identity);
            }
            let events = p["events"]
                .as_array()
                .filter(|e| e.len() <= 1000)
                .ok_or(ChatError::Response)?;
            let namespace = self.namespace().to_owned();
            for event in events {
                let c = self.context.as_mut().unwrap();
                let next = c.cursor.checked_add(1).ok_or(ChatError::Response)?;
                if event["seq"] != next
                    || next > latest
                    || event["namespace"] != namespace
                    || event["streamType"] != "task"
                    || event["streamID"] != task.name
                    || event
                        .get("taskName")
                        .and_then(Value::as_str)
                        .is_some_and(|s| !s.is_empty() && s != task.name)
                    || event
                        .get("agentName")
                        .and_then(Value::as_str)
                        .is_some_and(|s| !s.is_empty() && s != c.agent.name())
                {
                    return Err(ChatError::Identity);
                }
                c.cursor = next;
                if let (Some(name), Some(call), Some(phase)) = (
                    event["toolName"].as_str(),
                    event["toolCallID"].as_str(),
                    event["type"].as_str(),
                ) {
                    if !name.trim().is_empty()
                        && !call.trim().is_empty()
                        && matches!(
                            phase,
                            "ToolCallStarted" | "ToolCallCompleted" | "ToolCallFailed"
                        )
                    {
                        self.turn(|t| {
                            t.tools.push(ToolObservation {
                                name: name.into(),
                                call_id: call.into(),
                                phase: phase.into(),
                            })
                        });
                    }
                }
            }
            if self.context.as_ref().unwrap().cursor == latest {
                return Ok(());
            }
            if self.context.as_ref().unwrap().cursor == after {
                return Err(ChatError::Response);
            }
        }
    }
    async fn session(&mut self, deleted: bool) -> Result<bool, ChatError> {
        let c = self.context.as_ref().unwrap();
        let reply = self.get(&format!("sessions/{}", c.session)).await?;
        if reply.status == 404 && deleted {
            self.safe = true;
            return Err(ChatError::ContextUnavailable);
        }
        if reply.status == 404 {
            return Ok(false);
        }
        if reply.status != 200 {
            return Err(ChatError::Response);
        }
        if reply.body["name"] != c.session || reply.body["namespace"] != self.namespace() {
            return Err(ChatError::Identity);
        }
        if !reply.body["activeTask"]
            .as_str()
            .ok_or(ChatError::Response)?
            .is_empty()
        {
            return Ok(false);
        }
        // Only observed Session release makes malformed context safe to abandon.
        self.safe = true;
        let text = reply.body["transcript"]
            .as_str()
            .ok_or(ChatError::ContextUnavailable)?;
        let uid = c
            .task
            .receipt
            .as_ref()
            .map(|t| t.uid.as_str())
            .unwrap_or("");
        let prefix = format!("task:{uid}:");
        let mut messages = HashMap::new();
        for line in text.lines() {
            let m: TranscriptMessage =
                serde_json::from_str(line).map_err(|_| ChatError::ContextUnavailable)?;
            if m.id.is_empty()
                || !matches!(m.role.as_str(), "user" | "assistant" | "system" | "tool")
                || m.id
                    .strip_prefix(&prefix)
                    .is_some_and(|role| role != m.role)
                || messages.insert(m.id.clone(), m).is_some()
            {
                return Err(ChatError::ContextUnavailable);
            }
        }
        let user = messages
            .get(&format!("{prefix}user"))
            .map(|m| m.content.as_str());
        let assistant = messages
            .get(&format!("{prefix}assistant"))
            .map(|m| m.content.as_str());
        let turn = {
            let entries = self.owner.0.lock().unwrap();
            if !self.owner.current(&self.id, self.epoch) {
                return Err(ChatError::WorkspaceChanged);
            }
            entries
                .get(&self.id)
                .and_then(|e| e.view.versions.last())
                .and_then(|v| v.turns.last())
                .cloned()
                .ok_or(ChatError::WorkspaceChanged)?
        };
        if user.is_some_and(|u| u != turn.message)
            || assistant.is_some_and(|a| turn.answer.as_ref().is_some_and(|answer| answer != a))
        {
            return Err(ChatError::ContextUnavailable);
        }
        let needs_answer = turn.phase == "Succeeded" && turn.answer.as_deref() != Some("")
            || turn.answer.as_ref().is_some_and(|a| !a.is_empty());
        if !deleted && (user.is_none() || needs_answer && assistant.is_none()) {
            self.safe = false;
            return Ok(false);
        }
        self.turn(|t| {
            t.context_retained = user.is_some();
            if t.answer.is_none() {
                t.answer = assistant.map(str::to_owned);
            }
        });
        if deleted && !c.stopping {
            return Err(ChatError::ContextUnavailable);
        }
        self.state(ChatState::Ready);
        Ok(true)
    }
}
/// Deletes a replaced cluster conversation's Task, Session and Agent, in that order, on a
/// fresh connection. Only generated names of this conversation are addressed, and the Task and
/// Agent only while they still carry the UID acknowledged at creation. Returns true when the
/// cluster refused a delete, so the resources stay.
/// Opening rereads kubeconfig through one shared slot, so a cleanup and the next message can
/// briefly contend for it. A refusal that persists is the cluster's answer.
async fn open(cluster: &ClusterChannel) -> Result<Arc<dyn ClusterWire>, ChatError> {
    for _ in 0..7 {
        if let Ok(wire) = cluster.open.open().await {
            return Ok(wire);
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    cluster.open.open().await
}
async fn clean_up_on(cluster: &ClusterChannel, context: &Context) -> bool {
    let Ok(wire) = open(cluster).await else {
        return false;
    };
    let call = |r: ChatRequest| {
        let wire = wire.clone();
        async move { crate::chat_cluster::send(&*wire, &cluster.namespace, &cluster.provider, &r).await }
    };
    let mut paths = Vec::new();
    let tasks = context.earlier.iter().chain(context.task.receipt.as_ref());
    for receipt in tasks.chain(context.agent.receipt.as_ref()) {
        if receipt.kind() == "Agent" && context.task.sent {
            // A conversation's Session exists once its first Task was sent.
            paths.push(format!("sessions/{}", context.session));
        }
        match call(request(ChatMethod::Get, &receipt.path(), None, None)).await {
            Ok(reply)
                if reply.status == 200 && reply.body["metadata"]["uid"] == receipt.uid.as_str() =>
            {
                paths.push(receipt.path())
            }
            Err(Refusal::Denied(_)) => return finish(&*wire, true).await,
            _ => {}
        }
    }
    let mut kept = false;
    for path in paths {
        if let Err(Refusal::Denied(_)) = call(request(ChatMethod::Delete, &path, None, None)).await
        {
            kept = true;
        }
    }
    finish(&*wire, kept).await
}
async fn finish(wire: &dyn ClusterWire, kept: bool) -> bool {
    wire.close().await;
    kept
}
/// A failed local request is a lost connection, or a workspace that stopped serving entirely.
fn lost(client: &crate::runtime::client::VmClient) -> ChatError {
    if client.closed() {
        ChatError::WorkspaceUnavailable
    } else {
        ChatError::Connection
    }
}
fn unix_millis() -> u64 {
    u64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
    )
    .unwrap_or(u64::MAX)
}
#[derive(Deserialize)]
struct TranscriptMessage {
    id: String,
    role: String,
    content: String,
}
fn request(method: ChatMethod, path: &str, body: Option<Value>, after: Option<u64>) -> ChatRequest {
    ChatRequest {
        method,
        path: path.into(),
        after,
        body,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn workspace_retirement_releases_retained_lease_and_signals_stop() {
        let owner = ChatOwner::new(tokio::runtime::Handle::current());
        let vm = LocalVm::unavailable();
        let id = DocumentId::SAMPLE;
        let mut entry = Entry::new(id);
        entry.lease = Some(Arc::new(vm.try_authoring().unwrap()));
        entry.view.versions.push(ChatVersion {
            turns: vec![ChatTurn {
                message: "old workspace".into(),
                ..ChatTurn::default()
            }],
        });
        let stop = entry.stop.subscribe();
        owner
            .0
            .lock()
            .unwrap()
            .insert((id, Place::ThisComputer), entry);
        assert!(vm.try_authoring().is_none());
        owner.retire_for_workspace_replacement();
        assert!(*stop.borrow());
        assert!(owner.0.lock().unwrap().is_empty());
        assert!(owner.status(id).versions.is_empty());
        assert!(vm.try_authoring().is_some());
    }

    #[tokio::test]
    async fn retired_worker_edits_cannot_touch_a_new_same_document_conversation() {
        let owner = ChatOwner::new(tokio::runtime::Handle::current());
        let vm = LocalVm::unavailable();
        let id = DocumentId::SAMPLE;
        let worker = Worker {
            owner: owner.clone(),
            channel: Channel::Local(vm.clone()),
            wire: None,
            id: (id, Place::ThisComputer),
            epoch: owner.2.load(std::sync::atomic::Ordering::Relaxed),
            context: None,
            lease: Arc::new(vm.try_authoring().unwrap()),
            stop: watch::channel(false).1,
            deadline: Instant::now() + Duration::from_secs(1),
            stopping: false,
            safe: true,
            preflight: true,
        };
        owner
            .0
            .lock()
            .unwrap()
            .insert((id, Place::ThisComputer), Entry::new(id));
        owner.retire_for_workspace_replacement();
        // Missing entries as well as a recreated document must reject old edits.
        worker.state(ChatState::Working);
        owner.new_conversation(id).unwrap();
        worker.state(ChatState::ObservationLost);
        worker.turn(|_| panic!("a stale turn edit must never run"));
        let fresh = owner.status(id);
        assert_eq!(fresh.state, ChatState::Ready);
        assert!(fresh.versions.is_empty());
        // Current-epoch edits retain their original semantics.
        owner.edit(
            &(id, Place::ThisComputer),
            owner.2.load(std::sync::atomic::Ordering::Relaxed),
            |s| s.state = ChatState::Working,
        );
        assert_eq!(owner.status(id).state, ChatState::Working);
    }

    #[tokio::test]
    async fn retired_worker_finalization_does_not_restore_old_state_or_lease() {
        let owner = ChatOwner::new(tokio::runtime::Handle::current());
        let vm = LocalVm::unavailable();
        let folder = Arc::new(AgentFolder::new(None));
        let id = folder.document_id();
        // Exercise detached finalization without requiring a running VM.
        {
            let mut entries = owner.0.lock().unwrap();
            let entry = entries
                .entry((id, Place::ThisComputer))
                .or_insert_with(|| Entry::new(id));
            entry.lease = Some(Arc::new(vm.try_authoring().unwrap()));
            owner.launch(
                Channel::Local(vm.clone()),
                (id, Place::ThisComputer),
                entry,
                Some((ChatSource::Document(folder), "old message".into())),
            );
        }
        owner.retire_for_workspace_replacement();
        owner.new_conversation(id).unwrap();
        // Drive the real detached task through its failed old-client completion.
        for _ in 0..100 {
            if vm.try_authoring().is_some() {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert!(
            vm.try_authoring().is_some(),
            "old worker must drop its lease"
        );
        let fresh = owner.status(id);
        assert_eq!(fresh.state, ChatState::Ready);
        assert!(fresh.error.is_none());
        assert!(fresh.can_send && !fresh.busy && fresh.versions.is_empty());
    }

    #[test]
    fn turn_times_serialize_as_unix_milliseconds_with_null_until_finished() {
        let turn = ChatTurn {
            started_at: 1_700_000_000_000,
            ..ChatTurn::default()
        };
        let value = serde_json::to_value(&turn).unwrap();
        assert_eq!(value["startedAt"], 1_700_000_000_000u64);
        assert!(value["finishedAt"].is_null());
        let done = ChatTurn {
            finished_at: Some(1_700_000_004_000),
            ..turn
        };
        assert_eq!(
            serde_json::to_value(&done).unwrap()["finishedAt"],
            1_700_000_004_000u64
        );
        assert!(unix_millis() > 1_700_000_000_000);
    }

    #[test]
    fn each_source_creates_only_its_own_family_of_names() {
        let folder = Arc::new(AgentFolder::new(None));
        assert_eq!(ChatSource::Document(folder).family(), CHAT_FAMILY);
        assert_eq!(
            ChatSource::Navigator(String::new()).family(),
            NAVIGATOR_FAMILY
        );
        let planned = Planned::new(NAVIGATOR_FAMILY.agent, ORKA_NAMESPACE, json!({}));
        assert!(planned.name().starts_with("bridge-nav-agent-"));
        assert!(planned.request("agents").validate().is_ok());
    }

    #[tokio::test]
    async fn navigator_rejects_empty_messages_before_admission_and_starts_unbound() {
        let owner = ChatOwner::new(tokio::runtime::Handle::current());
        let vm = LocalVm::unavailable();
        assert!(owner.thinks_with().is_none());
        assert_eq!(
            owner
                .send_navigator(vm.clone(), "- web-reader".into(), " \n ".into())
                .unwrap_err(),
            ChatError::InvalidMessage
        );
        assert!(owner.0.lock().unwrap().is_empty());
        // The Navigator's conversation is a fixed key that never names a saved file.
        assert_ne!(crate::navigator::CONVERSATION, DocumentId::SAMPLE);
        assert!(owner
            .status(crate::navigator::CONVERSATION)
            .versions
            .is_empty());
        assert!(vm.try_authoring().is_some());
    }

    #[test]
    fn completion_overrides_late_stop_without_erasing_interruption_notice() {
        let mut entry = Entry::new(DocumentId::SAMPLE);
        entry.view.busy = true;
        entry.view.state = ChatState::RequestingStop;
        entry.view.versions.push(ChatVersion {
            turns: vec![ChatTurn {
                termination_unconfirmed: true,
                ..ChatTurn::default()
            }],
        });
        entry.finish(true, true);
        assert_eq!(entry.view.state, ChatState::Ready);
        assert!(entry.view.can_send);
        assert!(!entry.view.busy);
        assert!(entry.view.versions[0].turns[0].termination_unconfirmed);
    }
}

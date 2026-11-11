//! Fixed example Run on the original VM client. Credentials remain in the guest.
use crate::{
    agent_folder::{AgentFolder, DocumentId, DocumentRef},
    engine::run::{
        ErrorKind, NativeRunner, ParsedPair, RunFuture, RunHandle, RunLimits, RunReport,
        RunTransport,
    },
    local_vm::{LocalVm, LocalVmStatus},
    pair_source::CapturedPair,
    runtime::client::VmClient,
};
use bridge_runtime_protocol::{ModelPhase, OrkaRequest, SnapshotId, MAX_ORKA_BODY};
use serde::Serialize;
use serde_json::Value;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use tokio::{sync::watch, time::Instant};

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum RunState {
    Idle,
    Running,
    Complete,
    Unknown,
}
#[derive(Clone, Serialize)]
pub struct Receipt {
    pub kind: &'static str,
    pub name: String,
    pub uid: String,
}
#[derive(Clone, Serialize)]
pub struct PendingRef {
    pub kind: &'static str,
    pub name: String,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunSnapshot {
    pub document_id: DocumentId,
    pub busy: Option<DocumentRef>,
    pub state: RunState,
    pub can_run: bool,
    pub model_connected: bool,
    pub source: Option<SnapshotId>,
    pub report: Option<RunReport>,
    pub acknowledged: Vec<Receipt>,
    pub pending: Option<PendingRef>,
    pub error: Option<&'static str>,
}
impl RunSnapshot {
    fn idle(document_id: DocumentId) -> Self {
        Self {
            document_id,
            busy: None,
            state: RunState::Idle,
            can_run: false,
            model_connected: false,
            source: None,
            report: None,
            acknowledged: Vec::new(),
            pending: None,
            error: None,
        }
    }
}
#[derive(Default)]
struct State {
    snapshots: HashMap<DocumentId, RunSnapshot>,
    active: Option<DocumentRef>,
}
pub struct ExampleRun(Mutex<State>, tokio::runtime::Handle, bool, Arc<AgentFolder>);
impl ExampleRun {
    /// The native shell supplies its runtime; a blocking IPC thread need not enter it.
    pub fn new(
        runtime: tokio::runtime::Handle,
        model_image_available: bool,
        folder: Arc<AgentFolder>,
    ) -> Arc<Self> {
        Arc::new(Self(
            Mutex::default(),
            runtime,
            model_image_available,
            folder,
        ))
    }
    /// Sample-only compatibility for native setup callers.
    pub fn status(&self, vm: &LocalVm) -> RunSnapshot {
        self.status_document(vm, DocumentId::SAMPLE)
    }
    pub fn status_document(&self, vm: &LocalVm, id: DocumentId) -> RunSnapshot {
        let state = self.0.lock().unwrap();
        let mut snapshot = state
            .snapshots
            .get(&id)
            .cloned()
            .unwrap_or_else(|| RunSnapshot::idle(id));
        snapshot.busy = state.active.clone();
        // Terminal means admission is released, not merely that the report arrived.
        if state
            .active
            .as_ref()
            .is_some_and(|job| job.document_id == id)
        {
            snapshot.state = RunState::Running;
        }
        snapshot.model_connected = vm.model_connected();
        snapshot.can_run = self.2 && state.active.is_none() && vm.status() == LocalVmStatus::Ready;
        snapshot
    }
    pub fn start(self: &Arc<Self>, vm: Arc<LocalVm>) -> RunSnapshot {
        self.start_document(vm, self.3.clone())
    }
    pub fn start_document(
        self: &Arc<Self>,
        vm: Arc<LocalVm>,
        folder: Arc<AgentFolder>,
    ) -> RunSnapshot {
        let id = folder.document_id();
        if !self.2 {
            return self.status_document(&vm, id);
        }
        {
            let mut state = self.0.lock().unwrap();
            if state.active.is_some() {
                drop(state);
                return self.status_document(&vm, id);
            }
            state.active = Some(folder.document());
            let snapshot = state
                .snapshots
                .entry(id)
                .or_insert_with(|| RunSnapshot::idle(id));
            snapshot.state = RunState::Running;
            snapshot.can_run = false;
            snapshot.error = None;
            snapshot.report = None;
            snapshot.source = None;
            // Keep this document's receipts through preflight failures.
        }
        let owner = self.clone();
        let target = vm.clone();
        self.1.spawn(async move {
            let pair = tokio::task::spawn_blocking(move || {
                let snapshot = folder.capture().map_err(|_| ())?;
                CapturedPair::from_snapshot(snapshot).map_err(|_| ())
            })
            .await;
            if let Ok(Ok(pair)) = pair {
                let pair = pair.into_pair();
                owner
                    .0
                    .lock()
                    .unwrap()
                    .snapshots
                    .get_mut(&id)
                    .unwrap()
                    .source = Some(pair.snapshot().clone());
                let runtime = tokio::runtime::Handle::current();
                let worker = owner.clone();
                let observing = target.clone();
                if target
                    .with_client(move |_, client| {
                        runtime.block_on(worker.execute(id, client, observing, pair))
                    })
                    .await
                    .is_err()
                {
                    owner.fail(id, "session");
                }
            } else {
                owner.fail(id, "source");
            }
            // One global admission, released only after the client work returns.
            owner.0.lock().unwrap().active = None;
        });
        self.status_document(&vm, id)
    }
    fn fail(&self, id: DocumentId, message: &'static str) {
        let mut state = self.0.lock().unwrap();
        let snapshot = state.snapshots.get_mut(&id).unwrap();
        snapshot.state = RunState::Unknown;
        snapshot.error = Some(message);
    }
    async fn execute(
        &self,
        id: DocumentId,
        client: &mut VmClient,
        vm: Arc<LocalVm>,
        pair: ParsedPair,
    ) {
        let model = match client.model_status() {
            Ok(model) if model.phase == ModelPhase::Ready => model,
            Ok(_) => {
                vm.clear_model_acknowledgment();
                self.fail(id, "model");
                return;
            }
            Err(_) => {
                self.fail(id, "session");
                return;
            }
        };
        let Some(connection) = model.connection else {
            self.fail(id, "model");
            return;
        };
        let runner = NativeRunner::with_transport(VmRunTransport {
            client: Mutex::new(client),
            provider: connection.selection.provider,
            vm,
        });
        let prepared = match runner.prepare(pair) {
            Ok(prepared) => prepared,
            Err(_) => {
                self.fail(id, "submit");
                return;
            }
        };
        let (_sender, stop) = watch::channel(false);
        let handle = match runner.submit_prepared(prepared, stop).await {
            Ok(handle) => handle,
            Err(error) => {
                self.record(
                    id,
                    &error.handle,
                    error.pending.map(|p| PendingRef {
                        kind: p.kind,
                        name: p.name,
                    }),
                    None,
                );
                self.fail(id, "submit");
                return;
            }
        };
        self.record(id, &handle, None, None);
        match runner.observe(&handle).await {
            Ok(report) => {
                self.record(id, &handle, None, Some(report));
                self.0.lock().unwrap().snapshots.get_mut(&id).unwrap().state = RunState::Complete;
            }
            Err(_) => self.fail(id, "observe"),
        }
    }
    fn record(
        &self,
        id: DocumentId,
        handle: &RunHandle,
        pending: Option<PendingRef>,
        report: Option<RunReport>,
    ) {
        let mut state = self.0.lock().unwrap();
        let snapshot = state.snapshots.get_mut(&id).unwrap();
        snapshot.acknowledged = receipts(handle);
        snapshot.pending = pending;
        snapshot.report = report;
    }
}
/// Acknowledged identities only, shared by local Run and remote Send.
pub fn receipts(handle: &RunHandle) -> Vec<Receipt> {
    handle
        .agent()
        .map(|r| Receipt {
            kind: "Agent",
            name: r.name.clone(),
            uid: r.uid.clone(),
        })
        .into_iter()
        .chain(handle.tasks().iter().map(|r| Receipt {
            kind: "Task",
            name: r.name.clone(),
            uid: r.uid.clone(),
        }))
        .collect()
}
struct VmRunTransport<'a> {
    client: Mutex<&'a mut VmClient>,
    provider: String,
    vm: Arc<LocalVm>,
}
impl RunTransport for VmRunTransport<'_> {
    fn namespace(&self) -> &str {
        "orka-system"
    }
    fn provider(&self) -> &str {
        &self.provider
    }
    fn limits(&self) -> RunLimits {
        RunLimits::default()
    }
    fn max_create_body_bytes(&self) -> Option<usize> {
        Some(MAX_ORKA_BODY)
    }
    fn request<'a>(
        &'a self,
        path: &'a str,
        after: Option<u64>,
        body: Option<&'a Value>,
        deadline: Instant,
    ) -> RunFuture<'a> {
        Box::pin(async move {
            if self.vm.status() != LocalVmStatus::Ready {
                return Err(ErrorKind::Transport);
            }
            if Instant::now() >= deadline {
                return Err(ErrorKind::Deadline);
            }
            let reply = self
                .client
                .lock()
                .map_err(|_| ErrorKind::Transport)?
                .orka_request(OrkaRequest {
                    path: path.into(),
                    after,
                    body: body.cloned(),
                })
                .map_err(|_| ErrorKind::Transport)?;
            if reply.status != if body.is_some() { 201 } else { 200 } {
                return Err(ErrorKind::Response);
            }
            Ok(reply.body)
        })
    }
}

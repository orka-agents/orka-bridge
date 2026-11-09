//! Fixed example Run on the original VM client. Credentials remain in the guest.
use crate::{
    engine::run::{
        ErrorKind, NativeRunner, ParsedPair, RunFuture, RunHandle, RunLimits, RunReport,
        RunTransport,
    },
    local_vm::{LocalVm, LocalVmStatus},
    runtime::client::VmClient,
};
use bridge_runtime_protocol::{ModelPhase, OrkaRequest};
use serde::Serialize;
use serde_json::Value;
use std::sync::{Arc, Mutex};
use tokio::time::Instant;

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
    pub state: RunState,
    pub can_run: bool,
    pub report: Option<RunReport>,
    pub acknowledged: Vec<Receipt>,
    pub pending: Option<PendingRef>,
    pub error: Option<&'static str>,
}
struct State {
    snapshot: RunSnapshot,
    handle: Option<RunHandle>,
}
pub struct ExampleRun(Mutex<State>, tokio::runtime::Handle, bool);
impl ExampleRun {
    /// The native shell supplies its runtime; a blocking IPC thread need not enter it.
    pub fn new(runtime: tokio::runtime::Handle, model_image_available: bool) -> Arc<Self> {
        Arc::new(Self(
            Mutex::new(State {
                snapshot: RunSnapshot {
                    state: RunState::Idle,
                    can_run: false,
                    report: None,
                    acknowledged: Vec::new(),
                    pending: None,
                    error: None,
                },
                handle: None,
            }),
            runtime,
            model_image_available,
        ))
    }
    /// Observation only, including after a renderer remount; never submits or loads credentials.
    pub fn status(&self, vm: &LocalVm) -> RunSnapshot {
        let mut snapshot = self.0.lock().unwrap().snapshot.clone();
        snapshot.can_run =
            self.2 && snapshot.state != RunState::Running && vm.status() == LocalVmStatus::Ready;
        snapshot
    }
    pub fn start(self: &Arc<Self>, vm: Arc<LocalVm>) -> RunSnapshot {
        if !self.2 {
            return self.status(&vm);
        }
        {
            let mut state = self.0.lock().unwrap();
            if state.snapshot.state == RunState::Running {
                return state.snapshot.clone();
            }
            state.snapshot.state = RunState::Running;
            state.snapshot.can_run = false;
            state.snapshot.error = None;
            state.snapshot.report = None;
            // Keep remote identities through session/model preflight failures.
            // record() replaces them when submission returns a new handle.
        }
        let owner = self.clone();
        let target = vm.clone();
        // The owned task and worker publish receipts without depending on an IPC waiter.
        self.1.spawn(async move {
            let runtime = tokio::runtime::Handle::current();
            let worker = owner.clone();
            let observing = target.clone();
            let result = target
                .with_client(move |_, client| runtime.block_on(worker.execute(client, observing)))
                .await;
            if result.is_err() {
                owner.fail("session");
            }
        });
        self.status(&vm)
    }
    fn fail(&self, message: &'static str) {
        let mut state = self.0.lock().unwrap();
        state.snapshot.state = RunState::Unknown;
        state.snapshot.error = Some(message);
    }
    async fn execute(&self, client: &mut VmClient, vm: Arc<LocalVm>) {
        let pair = match ParsedPair::parse(
            include_bytes!("../example-agent/web-reader.agent.md"),
            include_bytes!("../example-agent/web-reader.agent.test.md"),
        ) {
            Ok(pair) => pair,
            Err(_) => {
                self.fail("source");
                return;
            }
        };
        let model = match client.model_status() {
            Ok(model) if model.phase == ModelPhase::Ready => model,
            _ => {
                self.fail("model");
                return;
            }
        };
        let Some(connection) = model.connection else {
            self.fail("model");
            return;
        };
        let runner = NativeRunner::with_transport(VmRunTransport {
            client: Mutex::new(client),
            provider: connection.selection.provider,
            vm,
        });
        let handle = match runner.submit(pair).await {
            Ok(handle) => handle,
            Err(error) => {
                self.record(
                    &error.handle,
                    error.pending.map(|p| PendingRef {
                        kind: p.kind,
                        name: p.name,
                    }),
                    None,
                );
                self.fail("submit");
                return;
            }
        };
        self.record(&handle, None, None);
        match runner.observe(&handle).await {
            Ok(report) => {
                self.record(&handle, None, Some(report));
                self.0.lock().unwrap().snapshot.state = RunState::Complete;
            }
            Err(_) => self.fail("observe"),
        }
    }
    fn record(&self, handle: &RunHandle, pending: Option<PendingRef>, report: Option<RunReport>) {
        let mut state = self.0.lock().unwrap();
        state.handle = Some(handle.clone());
        state.snapshot.acknowledged = handle
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
            .collect();
        state.snapshot.pending = pending;
        state.snapshot.report = report;
    }
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

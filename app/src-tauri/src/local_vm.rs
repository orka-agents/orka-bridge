//! App-level custody of one delivered native session; not a second VM lifecycle.
use crate::{
    engine::{types::*, EngineTransport, TransportFuture},
    runtime::types::RuntimeError,
};
use serde::Serialize;
use std::{
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
};
use tokio::sync::{watch, Mutex as AsyncMutex, Notify};
#[path = "local_vm_native.rs"]
mod native;
pub use native::configured;
#[derive(Serialize)]
pub struct ExamplePair {
    pub agent: &'static str,
    pub tests: &'static str,
}
pub fn example() -> ExamplePair {
    ExamplePair {
        agent: include_str!("../example-agent/web-reader.agent.md"),
        tests: include_str!("../example-agent/web-reader.agent.test.md"),
    }
}

pub async fn setup_status(
    store: Arc<crate::setup::SetupStore>,
    vm: &LocalVm,
) -> Result<crate::setup::SetupSnapshot, crate::setup::SetupError> {
    Ok(vm.snapshot(store.status_async().await?))
}
pub async fn select_local(
    store: Arc<crate::setup::SetupStore>,
    vm: &Arc<LocalVm>,
) -> Result<crate::setup::SetupSnapshot, crate::setup::SetupError> {
    let selected = store.select_local_async().await?;
    vm.start();
    Ok(vm.snapshot(selected))
}
pub async fn resume_selected(
    store: Arc<crate::setup::SetupStore>,
    vm: &Arc<LocalVm>,
) -> Result<(), crate::setup::SetupError> {
    if store.status_async().await?.selected.is_some() {
        vm.start();
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum LocalVmStatus {
    NotIntegrated,
    Idle,
    Preparing,
    Installing,
    Starting,
    Ready,
    Closing,
    Closed,
    Failed,
}
impl LocalVmStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotIntegrated => "notIntegrated",
            Self::Idle => "idle",
            Self::Preparing => "preparing",
            Self::Installing => "installing",
            Self::Starting => "starting",
            Self::Ready => "ready",
            Self::Closing => "closing",
            Self::Closed => "closed",
            Self::Failed => "failed",
        }
    }
}
fn failure_reason(error: &RuntimeError) -> String {
    match error {
        RuntimeError::Io(error) => error.to_string(),
        _ => error.to_string(),
    }
}

pub type StartFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Box<dyn LocalSession>, RuntimeError>> + Send + 'a>>;
pub trait LocalFactory: Send + Sync {
    fn launch(
        &self,
        closed: watch::Receiver<bool>,
        progress: Arc<dyn Fn(LocalVmStatus) + Send + Sync>,
    ) -> StartFuture<'_>;
}
pub trait LocalSession: Send {
    fn execute(&mut self, request: EngineRequest) -> TransportFuture<'_>;
    fn stop(&mut self) -> Result<(), RuntimeError>;
}
struct State {
    phase: LocalVmStatus,
    failure: Option<String>,
    closing: bool,
    close_started: bool,
    stopped: Option<bool>,
    startup: Option<tokio::task::JoinHandle<bool>>,
}
struct Inner {
    state: Mutex<State>,
    session: Arc<AsyncMutex<Option<Box<dyn LocalSession>>>>,
    closed: watch::Sender<bool>,
    finished: Notify,
}
impl Inner {
    fn publish(&self, phase: LocalVmStatus) {
        let mut state = self.state.lock().unwrap();
        if !state.closing {
            state.phase = phase;
        }
    }
    fn fail(&self, reason: String) {
        let mut state = self.state.lock().unwrap();
        state.failure = Some(reason);
        if !state.closing {
            state.phase = LocalVmStatus::Failed;
        }
    }
    fn ready(&self) -> bool {
        let state = self.state.lock().unwrap();
        !state.closing && state.phase == LocalVmStatus::Ready
    }
}
pub struct LocalVm {
    inner: Arc<Inner>,
    factory: Option<Arc<dyn LocalFactory>>,
}
impl LocalVm {
    fn make(factory: Option<Arc<dyn LocalFactory>>) -> Arc<Self> {
        let (closed, _) = watch::channel(false);
        Arc::new(Self {
            inner: Arc::new(Inner {
                state: Mutex::new(State {
                    phase: if factory.is_some() {
                        LocalVmStatus::Idle
                    } else {
                        LocalVmStatus::NotIntegrated
                    },
                    failure: None,
                    closing: false,
                    close_started: false,
                    stopped: None,
                    startup: None,
                }),
                session: Arc::new(AsyncMutex::new(None)),
                closed,
                finished: Notify::new(),
            }),
            factory,
        })
    }
    pub fn new(factory: Arc<dyn LocalFactory>) -> Arc<Self> {
        Self::make(Some(factory))
    }
    pub fn unavailable() -> Arc<Self> {
        Self::make(None)
    }
    pub fn status(&self) -> LocalVmStatus {
        self.inner.state.lock().unwrap().phase
    }
    fn snapshot(&self, snapshot: crate::setup::SetupSnapshot) -> crate::setup::SetupSnapshot {
        let state = self.inner.state.lock().unwrap();
        snapshot.with_local_vm(state.phase.as_str(), state.failure.clone())
    }
    pub fn start(self: &Arc<Self>) {
        let mut state = self.inner.state.lock().unwrap();
        if state.closing || state.phase != LocalVmStatus::Idle {
            return;
        }
        let Some(factory) = self.factory.clone() else {
            return;
        };
        state.phase = LocalVmStatus::Preparing;
        let inner = self.inner.clone();
        // Stored before releasing the admission lock. Dropping an IPC waiter does
        // not drop this task or any installation/handshake job it awaits.
        state.startup = Some(tokio::spawn(async move {
            let progress = {
                let i = inner.clone();
                Arc::new(move |phase| i.publish(phase))
            };
            let result = factory.launch(inner.closed.subscribe(), progress).await;
            let successful = result.is_ok();
            let mut session = inner.session.lock().await;
            match result {
                Ok(live) => {
                    *session = Some(live);
                    inner.publish(LocalVmStatus::Ready);
                }
                Err(error) => inner.fail(format!(
                    "Starting the local VM failed: {}",
                    failure_reason(&error)
                )),
            }
            successful
        }));
    }
    pub fn request_close(&self) {
        let mut state = self.inner.state.lock().unwrap();
        if !state.closing {
            state.closing = true;
            state.phase = LocalVmStatus::Closing;
            self.inner.closed.send_replace(true);
        }
    }
    pub async fn shutdown(&self) -> Result<(), RuntimeError> {
        self.request_close();
        {
            let mut state = self.inner.state.lock().unwrap();
            if !state.close_started {
                state.close_started = true;
                let startup = state.startup.take();
                let inner = self.inner.clone();
                // The close job owns its custody independently of any waiter.
                tokio::spawn(async move {
                    let started = match startup {
                        Some(task) => matches!(task.await, Ok(true)),
                        None => true,
                    };
                    let mut session = inner.session.clone().lock_owned().await;
                    let stop_inner = inner.clone();
                    let stopped = tokio::task::spawn_blocking(move || {
                        match session.as_mut().map_or(Ok(()), |s| s.stop()) {
                            Ok(()) => {
                                session.take();
                                true
                            }
                            Err(error) => {
                                // On failure keep the original session/quarantined worker.
                                stop_inner.fail(format!(
                                    "Stopping the local VM failed: {}",
                                    failure_reason(&error)
                                ));
                                false
                            }
                        }
                    })
                    .await
                    .unwrap_or(false)
                        && started;
                    let mut state = inner.state.lock().unwrap();
                    state.phase = if stopped {
                        LocalVmStatus::Closed
                    } else {
                        LocalVmStatus::Failed
                    };
                    state.stopped = Some(stopped);
                    inner.finished.notify_waiters();
                });
            }
        }
        loop {
            let notified = self.inner.finished.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if let Some(ok) = self.inner.state.lock().unwrap().stopped {
                return if ok {
                    Ok(())
                } else {
                    Err(RuntimeError::Unavailable)
                };
            }
            notified.await;
        }
    }
}
impl EngineTransport for LocalVm {
    fn execute(&self, request: EngineRequest) -> TransportFuture<'_> {
        Box::pin(async move {
            if !self.inner.ready() {
                return Err(EngineError::new(ErrorKind::Unavailable));
            }
            let mut session = self
                .inner
                .session
                .clone()
                .try_lock_owned()
                .map_err(|_| EngineError::new(ErrorKind::Execution))?;
            let inner = self.inner.clone();
            let runtime = tokio::runtime::Handle::current();
            tokio::task::spawn_blocking(move || {
                if !inner.ready() {
                    return Err(EngineError::new(ErrorKind::Unavailable));
                }
                let session = session
                    .as_mut()
                    .ok_or_else(|| EngineError::new(ErrorKind::Unavailable))?;
                // Keep the owned session guard through Mac's existing inner job,
                // even if the renderer cancels its outer request.
                let result = runtime.block_on(session.execute(request));
                if inner.ready() {
                    result
                } else {
                    Err(EngineError::new(ErrorKind::Unavailable))
                }
            })
            .await
            .map_err(|_| EngineError::new(ErrorKind::Execution))?
        })
    }
}

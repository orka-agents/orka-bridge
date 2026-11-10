//! Native remote Send owner. One admitted attempt at a time; credentials, the
//! runner and its handles stay here beyond any IPC waiter. Stop ends admission
//! and observation, never remote Task cancellation.
use crate::{
    agent_folder::AgentFolder,
    clusters::{commands::ClusterSession, ProviderId},
    engine::run::{ErrorKind, NativeRunner, ParsedPair, RunReport},
    example_run::{receipts, PendingRef, Receipt},
    pair_source::PairCatalog,
};
use bridge_runtime_protocol::{SnapshotId, Uuid};
use serde::Serialize;
use std::{
    collections::HashMap,
    future::Future,
    sync::{Arc, Mutex},
};
use tokio::sync::watch;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
pub struct SendId(Uuid);
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SendState {
    Running,
    Complete,
    Unknown,
}
/// Also the finite error vocabulary: the stage that failed, or Stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Stage {
    Capture,
    Access,
    Preflight,
    Submit,
    Observe,
    Stopped,
    Done,
}
/// Renderer view: IDs, display names, hashes and receipts; never a credential or endpoint.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SendView {
    pub id: SendId,
    pub target_id: String,
    pub target_name: Option<String>,
    pub provider_id: String,
    pub source: Option<SnapshotId>,
    pub model: Option<String>,
    pub agent_name: Option<String>,
    pub state: SendState,
    pub stage: Stage,
    pub acknowledged: Vec<Receipt>,
    pub pending: Option<PendingRef>,
    pub report: Option<RunReport>,
    pub error: Option<Stage>,
}
struct Active {
    id: SendId,
    stop: watch::Sender<bool>,
    done: watch::Receiver<bool>,
}
impl Active {
    fn halt(&self) -> watch::Receiver<bool> {
        self.stop.send_replace(true);
        self.done.clone()
    }
}
#[derive(Default)]
struct Inner {
    views: HashMap<SendId, SendView>,
    current: Option<SendId>,
    active: Option<Active>,
    closed: bool,
}
pub struct SendOwner {
    folder: Arc<AgentFolder>,
    session: ClusterSession,
    inner: Mutex<Inner>,
}
fn parse(id: &str) -> Option<Uuid> {
    Uuid::parse_str(id).ok()
}
async fn finished(mut done: watch::Receiver<bool>) {
    // A dropped sender also means the job ended.
    let _ = done.wait_for(|d| *d).await;
}
async fn stopped(stop: &mut watch::Receiver<bool>) {
    if stop.wait_for(|s| *s).await.is_err() {
        std::future::pending::<()>().await;
    }
}
/// GET-only or local stages may be dropped on Stop; nothing has been written yet.
async fn until_stop<T, E>(
    stop: &mut watch::Receiver<bool>,
    stage: Stage,
    work: impl Future<Output = Result<T, E>>,
) -> Result<T, Stage> {
    tokio::select! {
        biased;
        _ = stopped(stop) => Err(Stage::Stopped),
        result = work => result.map_err(|_| stage),
    }
}
fn failed(kind: ErrorKind, stage: Stage) -> Stage {
    if kind == ErrorKind::Stopped {
        Stage::Stopped
    } else {
        stage
    }
}
fn capture(folder: &AgentFolder) -> Result<ParsedPair, ()> {
    let (root, document) = folder.source().map_err(drop)?;
    // A per-attempt catalog releases its held directory after this one capture.
    let catalog = PairCatalog::default();
    let id = catalog.bind_owned_agent(root, document).map_err(drop)?;
    Ok(catalog.capture(id).map_err(drop)?.into_pair())
}
impl SendOwner {
    pub fn new(folder: Arc<AgentFolder>, session: ClusterSession) -> Arc<Self> {
        Arc::new(Self {
            folder,
            session,
            inner: Mutex::default(),
        })
    }
    /// Admits one job, or observes the running one, without waiting on the
    /// cluster session; capture, access and all rechecks happen inside the job.
    pub async fn start(self: &Arc<Self>, target_id: &str, provider_id: &str) -> SendView {
        let target = parse(target_id).map(|u| u.to_string()).unwrap_or_default();
        let provider = parse(provider_id).map(ProviderId);
        let mut inner = self.inner.lock().unwrap();
        if let Some(active) = &inner.active {
            return inner.views[&active.id].clone();
        }
        let id = SendId(Uuid::new_v4());
        let mut view = SendView {
            id,
            target_id: target.clone(),
            target_name: None,
            provider_id: provider.map(|p| p.0.to_string()).unwrap_or_default(),
            source: None,
            model: None,
            agent_name: None,
            state: SendState::Running,
            stage: Stage::Capture,
            acknowledged: Vec::new(),
            pending: None,
            report: None,
            error: None,
        };
        inner.current = Some(id);
        if inner.closed {
            (view.state, view.stage, view.error) =
                (SendState::Unknown, Stage::Stopped, Some(Stage::Stopped));
            inner.views.insert(id, view.clone());
            return view;
        }
        let (stop, mut stop_rx) = watch::channel(false);
        let (done_tx, done) = watch::channel(false);
        inner.views.insert(id, view.clone());
        inner.active = Some(Active { id, stop, done });
        drop(inner);
        let owner = self.clone();
        tokio::spawn(async move {
            let result = owner.drive(id, &target, provider, &mut stop_rx).await;
            let mut inner = owner.inner.lock().unwrap();
            let view = inner.views.get_mut(&id).unwrap();
            match result {
                Ok(report) => {
                    (view.state, view.stage) = (SendState::Complete, Stage::Done);
                    view.report = Some(report);
                }
                Err(stage) => {
                    view.state = SendState::Unknown;
                    if stage == Stage::Stopped {
                        view.stage = stage;
                    }
                    view.error = Some(stage);
                }
            }
            // Release admission only after the runner and forward were closed.
            inner.active = None;
            drop(inner);
            let _ = done_tx.send(true);
        });
        view
    }
    pub fn status(&self, id: Option<&str>) -> Option<SendView> {
        let inner = self.inner.lock().unwrap();
        let id = match id {
            Some(id) => SendId(parse(id)?),
            None => inner.current?,
        };
        inner.views.get(&id).cloned()
    }
    /// Signals before the first await, so a lost IPC waiter still stops the job.
    pub async fn stop(&self, id: &str) -> Option<SendView> {
        let id = SendId(parse(id)?);
        let done = {
            let inner = self.inner.lock().unwrap();
            inner.views.get(&id)?;
            inner
                .active
                .as_ref()
                .filter(|a| a.id == id)
                .map(Active::halt)
        };
        if let Some(done) = done {
            finished(done).await;
        }
        self.inner.lock().unwrap().views.get(&id).cloned()
    }
    /// App exit: refuse new admissions, stop the current job and await its cleanup.
    pub async fn shutdown(&self) {
        let done = {
            let mut inner = self.inner.lock().unwrap();
            inner.closed = true;
            inner.active.as_ref().map(Active::halt)
        };
        if let Some(done) = done {
            finished(done).await;
        }
    }
    fn update(&self, id: SendId, change: impl FnOnce(&mut SendView)) {
        change(self.inner.lock().unwrap().views.get_mut(&id).unwrap());
    }
    async fn drive(
        &self,
        id: SendId,
        target: &str,
        provider: Option<ProviderId>,
        stop: &mut watch::Receiver<bool>,
    ) -> Result<RunReport, Stage> {
        // Freeze the pair once, before any network wait; later edits never reach this attempt.
        let folder = self.folder.clone();
        let pair = until_stop(stop, Stage::Capture, async move {
            tokio::task::spawn_blocking(move || capture(&folder))
                .await
                .map_err(drop)?
        })
        .await?;
        self.update(id, |v| {
            v.source = Some(pair.snapshot().clone());
            v.model = Some(pair.model().into());
            v.stage = Stage::Access;
        });
        // Shares the frozen access under a short session lock; never held for the run.
        let (access, reader) = until_stop(stop, Stage::Access, async {
            Ok::<_, ()>(self.session.send_access(target).await)
        })
        .await?;
        if let Some(access) = &access {
            self.update(id, |v| {
                v.target_name = Some(access.binding().context_name.clone())
            });
        }
        let (access, provider) = access
            .zip(provider)
            .filter(|(a, p)| a.providers().iter().any(|v| v.id == *p))
            .ok_or(Stage::Access)?;
        let kubeconfig = until_stop(stop, Stage::Access, async move {
            tokio::task::spawn_blocking(move || reader())
                .await
                .map_err(drop)?
                .map_err(drop)
        })
        .await?;
        let fresh = access.fresh_client(kubeconfig);
        self.update(id, |v| v.stage = Stage::Preflight);
        // Revalidates the fresh kubeconfig binding. Not raced with Stop, so the
        // one opened forward is always explicitly closed (and joined) below,
        // including when Stop drops a recheck GET on it.
        let transport = access
            .transport(&fresh, provider)
            .await
            .map_err(|_| Stage::Preflight)?;
        let checked = until_stop(
            stop,
            Stage::Preflight,
            access.recheck(&transport, provider, pair.tools()),
        )
        .await;
        let runner = NativeRunner::with_transport(transport);
        let result = match checked {
            Ok(()) => self.run(id, &runner, pair, stop).await,
            Err(stage) => Err(stage),
        };
        runner.close().await;
        result
    }
    async fn run(
        &self,
        id: SendId,
        runner: &NativeRunner<'_>,
        pair: ParsedPair,
        stop: &watch::Receiver<bool>,
    ) -> Result<RunReport, Stage> {
        if *stop.borrow() {
            return Err(Stage::Stopped);
        }
        let prepared = runner.prepare(pair).map_err(|_| Stage::Submit)?;
        self.update(id, |v| {
            v.agent_name = Some(prepared.agent_name().into());
            v.stage = Stage::Submit;
        });
        let handle = match runner.submit_prepared(prepared, stop.clone()).await {
            Ok(handle) => handle,
            Err(error) => {
                // Possibly-created identities stay visible; nothing is replayed.
                self.update(id, |v| {
                    v.acknowledged = receipts(&error.handle);
                    v.pending = error.pending.map(|p| PendingRef {
                        kind: p.kind,
                        name: p.name,
                    });
                });
                return Err(failed(error.kind, Stage::Submit));
            }
        };
        self.update(id, |v| {
            v.acknowledged = receipts(&handle);
            v.stage = Stage::Observe;
        });
        runner
            .observe_stoppable(&handle, stop.clone())
            .await
            .map_err(|e| failed(e.kind, Stage::Observe))
    }
}

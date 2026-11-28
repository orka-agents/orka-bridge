//! A running workspace that stops by itself is noticed and can be started again in place.
use super::*;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

#[derive(Default)]
struct Vm {
    /// The VM process exited, or its guest channel failed.
    gone: AtomicBool,
    /// Only a request sees the loss (the watcher's check reads `gone`).
    lost_on_request: AtomicBool,
    requests: AtomicUsize,
    stops: AtomicUsize,
    launches: AtomicUsize,
}
#[derive(Clone, Copy)]
enum Stop {
    Clean,
    Forced,
    Unconfirmed,
}
struct Session(Arc<Vm>, Stop);
impl LocalSession for Session {
    fn with_client(
        &mut self,
        _: &mut dyn FnMut(bridge_runtime_protocol::Uuid, &mut VmClient) -> Result<(), RuntimeError>,
    ) -> Result<(), RuntimeError> {
        self.0.requests.fetch_add(1, Ordering::SeqCst);
        if self.0.lost_on_request.load(Ordering::SeqCst) {
            self.0.gone.store(true, Ordering::SeqCst);
        }
        Err(RuntimeError::Io(std::io::ErrorKind::BrokenPipe.into()))
    }
    fn execute(&mut self, _: EngineRequest) -> TransportFuture<'_> {
        self.0.requests.fetch_add(1, Ordering::SeqCst);
        if self.0.lost_on_request.load(Ordering::SeqCst) {
            self.0.gone.store(true, Ordering::SeqCst);
        }
        Box::pin(async { Err(EngineError::new(ErrorKind::Execution)) })
    }
    fn stop(&mut self) -> Result<(), RuntimeError> {
        self.0.stops.fetch_add(1, Ordering::SeqCst);
        match self.1 {
            Stop::Clean => Ok(()),
            Stop::Forced => Err(RuntimeError::ShutdownForced),
            Stop::Unconfirmed => Err(RuntimeError::StopUnconfirmed("held".into())),
        }
    }
    fn lost(&mut self) -> bool {
        self.0.gone.load(Ordering::SeqCst)
    }
}
struct Factory(Arc<Vm>, Stop);
impl LocalFactory for Factory {
    fn launch(
        &self,
        _: watch::Receiver<bool>,
        _: Arc<dyn Fn(LocalProgress) + Send + Sync>,
    ) -> StartFuture<'_> {
        self.0.launches.fetch_add(1, Ordering::SeqCst);
        self.0.gone.store(false, Ordering::SeqCst);
        let session: Box<dyn LocalSession> = Box::new(Session(self.0.clone(), self.1));
        Box::pin(async move { Ok(session) })
    }
}
async fn until(vm: &LocalVm, phase: LocalVmStatus, within: std::time::Duration) -> bool {
    let end = std::time::Instant::now() + within;
    while std::time::Instant::now() < end {
        if vm.status() == phase {
            return true;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    vm.status() == phase
}
fn exited(vm: &LocalVm) -> bool {
    let state = vm.inner.state.lock().unwrap();
    state.phase == LocalVmStatus::Failed
        && !state.closing
        && state.workspace_failure
            == Some(WorkspaceFailure::new(
                WorkspaceOperation::Serve,
                crate::runtime::types::WorkspaceCause::Exited,
                [],
            ))
}

#[tokio::test]
async fn a_vm_that_exits_while_ready_is_seen_within_seconds_and_starts_again_in_place() {
    for stop in [Stop::Forced, Stop::Clean] {
        let shared = Arc::new(Vm::default());
        let vm = LocalVm::new(Arc::new(Factory(shared.clone(), stop)));
        vm.start();
        assert!(until(&vm, LocalVmStatus::Ready, Duration::from_secs(5)).await);
        vm.inner.state.lock().unwrap().model_connected = true;
        // Nothing asks the guest anything: the watcher reads the session's own state.
        shared.gone.store(true, Ordering::SeqCst);
        assert!(until(&vm, LocalVmStatus::Failed, Duration::from_secs(5)).await);
        assert!(exited(&vm));
        assert_eq!(shared.requests.load(Ordering::SeqCst), 0);
        assert_eq!(shared.stops.load(Ordering::SeqCst), 1);
        assert!(
            vm.inner.session.lock().await.is_none(),
            "the session is released"
        );
        assert!(!vm.model_connected());
        assert!(matches!(
            vm.with_client(|_, _| ()).await,
            Err(RuntimeError::Unavailable)
        ));
        // Restart needs no quit: the same owner starts a new boot.
        vm.start();
        assert!(until(&vm, LocalVmStatus::Ready, Duration::from_secs(5)).await);
        assert_eq!(shared.launches.load(Ordering::SeqCst), 2);
        assert!(vm.inner.state.lock().unwrap().workspace_failure.is_none());
        vm.shutdown().await.unwrap_or(());
    }
}

#[tokio::test]
async fn a_request_that_finds_the_workspace_gone_flips_the_state_at_once() {
    let shared = Arc::new(Vm::default());
    let vm = LocalVm::new(Arc::new(Factory(shared.clone(), Stop::Forced)));
    vm.start();
    assert!(until(&vm, LocalVmStatus::Ready, Duration::from_secs(5)).await);
    shared.lost_on_request.store(true, Ordering::SeqCst);
    assert!(vm.with_client(|_, _| ()).await.is_err());
    // Already stopping, before the watcher's next look.
    assert_ne!(vm.status(), LocalVmStatus::Ready);
    assert!(until(&vm, LocalVmStatus::Failed, Duration::from_millis(1500)).await);
    assert!(exited(&vm));
    // The same through an engine request.
    vm.start();
    assert!(until(&vm, LocalVmStatus::Ready, Duration::from_secs(5)).await);
    let request = EngineRequest {
        action: EngineAction::Version {},
    };
    assert!(EngineTransport::execute(&*vm, request).await.is_err());
    assert_ne!(vm.status(), LocalVmStatus::Ready);
    assert!(until(&vm, LocalVmStatus::Failed, Duration::from_millis(1500)).await);
    assert!(exited(&vm));
    vm.shutdown().await.unwrap_or(());
}

#[tokio::test]
async fn a_lost_workspace_whose_stop_is_not_confirmed_stays_quarantined() {
    let shared = Arc::new(Vm::default());
    let vm = LocalVm::new(Arc::new(Factory(shared.clone(), Stop::Unconfirmed)));
    vm.start();
    assert!(until(&vm, LocalVmStatus::Ready, Duration::from_secs(5)).await);
    shared.gone.store(true, Ordering::SeqCst);
    assert!(until(&vm, LocalVmStatus::Failed, Duration::from_secs(5)).await);
    {
        let state = vm.inner.state.lock().unwrap();
        assert!(state.closing, "an unconfirmed stop never admits a restart");
        assert_eq!(
            state.workspace_failure.as_ref().map(|f| f.cause),
            Some(crate::runtime::types::WorkspaceCause::StopUnconfirmed)
        );
    }
    assert!(
        vm.inner.session.lock().await.is_some(),
        "the session is kept"
    );
    vm.start();
    assert_eq!(shared.launches.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_healthy_workspace_is_left_alone_and_a_user_stop_is_no_loss() {
    let shared = Arc::new(Vm::default());
    let vm = LocalVm::new(Arc::new(Factory(shared.clone(), Stop::Clean)));
    vm.start();
    assert!(until(&vm, LocalVmStatus::Ready, Duration::from_secs(5)).await);
    // Healthy: several watcher ticks change nothing.
    tokio::time::sleep(Duration::from_millis(4500)).await;
    assert_eq!(vm.status(), LocalVmStatus::Ready);
    assert_eq!(shared.stops.load(Ordering::SeqCst), 0);
    // A user stop is a stop, not a loss.
    vm.cancel().await.unwrap();
    assert_eq!(vm.status(), LocalVmStatus::Cancelled);
    assert!(vm.inner.state.lock().unwrap().workspace_failure.is_none());
}

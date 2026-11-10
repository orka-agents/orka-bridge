use bridge_engine::{
    engine::{types::*, EngineState, EngineTransport, TransportFuture},
    local_vm::{LocalFactory, LocalSession, LocalVm, LocalVmStatus, StartFuture},
    runtime::types::RuntimeError,
};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use tokio::sync::{watch, Notify};

#[derive(Default)]
struct Probe {
    launch: Notify,
    stops: AtomicUsize,
    fail_stop: bool,
}
struct Factory(Arc<Probe>);
struct Session(Arc<Probe>);
impl LocalFactory for Factory {
    fn launch(
        &self,
        _: watch::Receiver<bool>,
        progress: Arc<dyn Fn(bridge_engine::local_vm::LocalProgress) + Send + Sync>,
    ) -> StartFuture<'_> {
        let p = self.0.clone();
        Box::pin(async move {
            progress(bridge_engine::local_vm::LocalProgress::Status(
                LocalVmStatus::Installing,
            ));
            p.launch.notified().await;
            Ok(Box::new(Session(p)) as Box<dyn LocalSession>)
        })
    }
}
impl LocalSession for Session {
    fn execute(&mut self, _: EngineRequest) -> TransportFuture<'_> {
        Box::pin(async { Err(EngineError::new(ErrorKind::Unavailable)) })
    }
    fn stop(&mut self) -> Result<(), RuntimeError> {
        self.0.stops.fetch_add(1, Ordering::SeqCst);
        if self.0.fail_stop {
            Err(RuntimeError::Unavailable)
        } else {
            Ok(())
        }
    }
}
fn owner(p: Arc<Probe>) -> Arc<LocalVm> {
    LocalVm::new(Arc::new(Factory(p)))
}
async fn phase(vm: &LocalVm, expected: LocalVmStatus) {
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while vm.status() != expected {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}
fn request() -> EngineRequest {
    EngineRequest {
        action: EngineAction::Version {},
    }
}

// The terminal coordinator must bar admission on the native thread, without a runtime.
#[test]
fn terminal_request_has_one_winner_without_an_async_runtime() {
    let shutdown = bridge_engine::shutdown::TerminalShutdown::default();
    let vm = LocalVm::unavailable();
    let engine = EngineState::with_transport(vm.clone());
    let winners = std::thread::scope(|scope| {
        let attempts: Vec<_> = (0..16)
            .map(|_| scope.spawn(|| shutdown.request(&vm, &engine)))
            .collect();
        attempts
            .into_iter()
            .map(|task| usize::from(task.join().unwrap()))
            .sum::<usize>()
    });
    assert_eq!(winners, 1);
    assert_eq!(vm.status(), LocalVmStatus::Closing);
    assert!(!shutdown.completed());
}

struct ResponsiveTransport;
impl EngineTransport for ResponsiveTransport {
    fn execute(&self, _: EngineRequest) -> TransportFuture<'_> {
        Box::pin(async {
            Ok(EngineOutput {
                exit: 0,
                stdout: b"kmx v0.9.0\n".to_vec(),
                stderr: Vec::new(),
            })
        })
    }
}

#[tokio::test]
async fn terminal_completion_waits_for_the_late_session() {
    use bridge_engine::{
        agent_folder::AgentFolder, clusters::commands::ClusterSession, send::SendOwner,
    };
    let p = Arc::new(Probe::default());
    let vm = owner(p.clone());
    vm.start();
    phase(&vm, LocalVmStatus::Installing).await;
    // Independent transport: closing the VM alone must not satisfy the engine barrier.
    let engine = EngineState::with_transport(Arc::new(ResponsiveTransport));
    assert!(engine.call(request()).await.is_ok());
    let shutdown = bridge_engine::shutdown::TerminalShutdown::default();
    let clusters = ClusterSession::default();
    let send = SendOwner::new(Arc::new(AgentFolder::new(None)), clusters.clone());
    assert!(shutdown.request(&vm, &engine));
    assert!(engine.call(request()).await.is_err());
    assert!(!shutdown.request(&vm, &engine));
    let cleanup = shutdown.finish(&clusters, &send, &vm);
    tokio::pin!(cleanup);
    // Poll cleanup before checking completion: merely spawning it would not prove it ran.
    let mut context = std::task::Context::from_waker(std::task::Waker::noop());
    assert!(std::future::Future::poll(cleanup.as_mut(), &mut context).is_pending());
    assert!(!shutdown.completed());
    p.launch.notify_one();
    assert!(cleanup.await);
    assert!(shutdown.completed());
    assert_eq!(vm.status(), LocalVmStatus::Closed);
    assert_eq!(p.stops.load(Ordering::SeqCst), 1);
    assert!(!shutdown.request(&vm, &engine));
}

#[tokio::test]
async fn terminal_failure_completes_without_claiming_a_clean_stop() {
    use bridge_engine::{
        agent_folder::AgentFolder, clusters::commands::ClusterSession, send::SendOwner,
    };
    let p = Arc::new(Probe {
        fail_stop: true,
        ..Default::default()
    });
    let vm = owner(p.clone());
    vm.start();
    p.launch.notify_one();
    phase(&vm, LocalVmStatus::Ready).await;
    let engine = EngineState::with_transport(vm.clone());
    let shutdown = bridge_engine::shutdown::TerminalShutdown::default();
    let clusters = ClusterSession::default();
    let send = SendOwner::new(Arc::new(AgentFolder::new(None)), clusters.clone());
    assert!(shutdown.request(&vm, &engine));
    assert!(!shutdown.finish(&clusters, &send, &vm).await);
    assert!(shutdown.completed());
    assert_eq!(vm.status(), LocalVmStatus::Failed);
    assert!(!shutdown.request(&vm, &engine));
    assert_eq!(p.stops.load(Ordering::SeqCst), 1);
}

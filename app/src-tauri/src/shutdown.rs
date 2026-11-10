use crate::{engine::EngineState, local_vm::LocalVm, send::SendOwner};
use std::sync::atomic::{AtomicBool, Ordering};

/// One terminal cleanup worker, shared by native exit requests and its completion.
#[derive(Default)]
pub struct TerminalShutdown {
    closing: AtomicBool,
    completed: AtomicBool,
}

impl TerminalShutdown {
    pub fn completed(&self) -> bool {
        self.completed.load(Ordering::SeqCst)
    }

    /// Run on the native event thread before scheduling the winning caller's worker.
    pub fn request(&self, vm: &LocalVm, engine: &EngineState) -> bool {
        if self.closing.swap(true, Ordering::SeqCst) {
            return false;
        }
        vm.request_close(); // Bar late Ready before scheduling async work.
        engine.shutdown();
        true
    }

    /// Only the request winner runs cleanup. The caller retains the worker until exit.
    pub async fn finish(&self, send: &SendOwner, vm: &LocalVm) -> bool {
        send.shutdown().await; // Stop observation and close the forward; remote Tasks continue.
        let clean = vm.shutdown().await.is_ok();
        if !clean {
            eprintln!("Local VM startup or shutdown did not finish successfully; cleanup was not reported as clean.");
        }
        self.completed.store(true, Ordering::SeqCst);
        clean
    }
}

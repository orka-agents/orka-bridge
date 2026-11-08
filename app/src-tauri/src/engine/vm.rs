use super::{types::*, EngineTransport, TransportFuture};
use crate::runtime::client::VmClient;
use std::sync::Arc;
use tokio::sync::Mutex;
/// A live client is supplied only by the owned runtime, never an environment override.
#[derive(Default)]
pub struct VmTransport(Option<Arc<Mutex<VmClient>>>);
impl VmTransport {
    pub fn with_client(client: VmClient) -> Self {
        Self(Some(Arc::new(Mutex::new(client))))
    }
    /// Lends the original client inside its native session's blocking worker.
    pub fn borrow_client<T>(
        &self,
        work: impl FnOnce(&mut VmClient) -> T,
    ) -> Result<T, EngineError> {
        let mut client = self
            .0
            .as_ref()
            .ok_or_else(|| EngineError::new(ErrorKind::Unavailable))?
            .try_lock()
            .map_err(|_| EngineError::new(ErrorKind::Execution))?;
        Ok(work(&mut client))
    }
}
impl EngineTransport for VmTransport {
    fn execute(&self, request: EngineRequest) -> TransportFuture<'_> {
        Box::pin(async move {
            let client = self
                .0
                .clone()
                .ok_or_else(|| EngineError::new(ErrorKind::Unavailable))?;
            // Refuse excess work before consuming a shared blocking-pool worker.
            let mut client = client
                .try_lock_owned()
                .map_err(|_| EngineError::new(ErrorKind::Execution))?;
            tokio::task::spawn_blocking(move || {
                let id = client
                    .execute(request.action, None)
                    .map_err(|_| EngineError::new(ErrorKind::Execution))?;
                let result = client
                    .result(id)
                    .map_err(|_| EngineError::new(ErrorKind::Execution))?;
                Ok(EngineOutput {
                    exit: result.exit,
                    stdout: result.stdout,
                    stderr: if result.has_diagnostics {
                        vec![1]
                    } else {
                        Vec::new()
                    },
                })
            })
            .await
            .map_err(|_| EngineError::new(ErrorKind::Execution))?
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{self, Read, Write},
        time::Duration,
    };
    struct Closed;
    impl Read for Closed {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Ok(0)
        }
    }
    impl Write for Closed {
        fn write(&mut self, b: &[u8]) -> io::Result<usize> {
            Ok(b.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    #[tokio::test]
    async fn native_borrow_lends_the_original_client_once_and_refuses_a_held_lock() {
        let id = bridge_runtime_protocol::Uuid::new_v4();
        let vm = VmTransport::with_client(VmClient::established(Box::new(Closed), id).unwrap());
        let guard = vm.0.as_ref().unwrap().try_lock().unwrap();
        assert!(vm
            .borrow_client(|_| panic!("busy must not call work"))
            .is_err());
        drop(guard);
        let mut count = 0;
        for _ in 0..2 {
            vm.borrow_client(|client| {
                count += 1;
                assert_eq!(client.boot_nonce(), id);
            })
            .unwrap();
        }
        assert_eq!(count, 2);
    }
    #[tokio::test]
    async fn concurrent_admission_never_parks_a_blocking_worker_on_the_client_lock() {
        let vm = VmTransport::with_client(
            VmClient::established(Box::new(Closed), bridge_runtime_protocol::Uuid::new_v4())
                .unwrap(),
        );
        let client = vm.0.clone().unwrap();
        let _held = client.try_lock().unwrap();
        let response = tokio::time::timeout(
            Duration::from_millis(100),
            vm.execute(EngineRequest {
                action: EngineAction::Version {},
            }),
        )
        .await;
        assert!(
            response.is_ok(),
            "a busy client must be refused before spawning blocking work"
        );
        assert_eq!(response.unwrap().unwrap_err().kind, ErrorKind::Execution);
    }
}

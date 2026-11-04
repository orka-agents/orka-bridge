use super::{types::*, EngineTransport, TransportFuture};
use crate::runtime::client::VmClient;
use std::sync::{Arc, Mutex};
/// A live client is supplied only by the owned runtime, never an environment override.
#[derive(Default)]
pub struct VmTransport(Option<Arc<Mutex<VmClient>>>);
impl VmTransport {
    pub fn with_client(client: VmClient) -> Self {
        Self(Some(Arc::new(Mutex::new(client))))
    }
}
impl EngineTransport for VmTransport {
    fn execute(&self, request: EngineRequest) -> TransportFuture<'_> {
        Box::pin(async move {
            let client = self
                .0
                .clone()
                .ok_or_else(|| EngineError::new(ErrorKind::Unavailable))?;
            tokio::task::spawn_blocking(move || {
                let mut client = client
                    .lock()
                    .map_err(|_| EngineError::new(ErrorKind::Execution))?;
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

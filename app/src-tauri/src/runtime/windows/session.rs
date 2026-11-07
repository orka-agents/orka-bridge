//! The native Windows owner; not setup activation or renderer configuration.
use super::{backend::Lease, job::OwnedQemuProcess};
use crate::runtime::{client::VmClient, handshake, types::RuntimeError, RuntimeWorker};
use std::sync::Arc;

pub struct WindowsSession {
    worker: RuntimeWorker,
    client: Option<VmClient>,
    owner: Arc<OwnedQemuProcess>,
    lease: Lease,
}
impl WindowsSession {
    pub fn start(mut worker: RuntimeWorker, host_utc: u64) -> Result<Self, RuntimeError> {
        worker.start()?;
        let lease = worker
            .lease()
            .ok_or(RuntimeError::Unavailable)?
            .handle
            .get::<Lease>()
            .ok_or(RuntimeError::Unavailable)?
            .clone();
        let owner = lease.owner()?;
        let channel = lease.guest()?;
        let client = handshake::establish(&mut worker, Box::new(channel), host_utc)?;
        Ok(Self {
            worker,
            client: Some(client),
            owner,
            lease,
        })
    }
    pub fn client_mut(&mut self) -> Result<&mut VmClient, RuntimeError> {
        self.client.as_mut().ok_or(RuntimeError::Closed)
    }
    pub fn shutdown(&mut self) -> Result<(), RuntimeError> {
        if self.client.is_none() {
            return self.worker.stop();
        }
        // Begin the same original budget before a guest can acknowledge and
        // exit. GuestStream clamps its native I/O to this existing deadline.
        self.lease.prepare_shutdown()?;
        let ack = self.client.as_mut().expect("live client").shutdown();
        let stopped = self.worker.stop();
        self.client = None;
        ack.map_err(RuntimeError::from)?;
        stopped
    }
    pub fn diagnostics(&self) -> Result<super::backend::Diagnostics, RuntimeError> {
        self.lease.diagnostics()
    }
}
impl Drop for WindowsSession {
    fn drop(&mut self) {
        if self.shutdown().is_err() {
            // RuntimeWorker's existing failed-stop quarantine retains the
            // journal/backend bundle; this attempt never reconstructs a lease.
            let _ = self.owner.abort();
        }
    }
}

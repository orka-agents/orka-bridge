//! Opens Try a task and Navigator conversations on the selected cluster. The renderer names
//! the place only by its opaque target and AI connection ids; the confirmed access, its
//! watched namespace and its Provider come from this native session.
use super::{
    commands::{ClusterSession, KubeconfigReader},
    *,
};
use crate::{
    chat::ChatError,
    chat_cluster::{ClusterChannel, ClusterOpen, ClusterWire, WireFuture, WireRequest},
    engine::run::{ErrorKind, RunTransport},
};
use std::sync::Arc;

/// The conversation channel for the selected cluster and AI connection, if both are confirmed
/// in this run.
pub fn channel(
    session: &ClusterSession,
    target: &str,
    provider: &str,
) -> Result<Arc<ClusterChannel>, ChatError> {
    let (access, reader) = session.send_access(target);
    let access = access.ok_or(ChatError::ClusterUnavailable)?;
    let provider = Uuid::parse_str(provider)
        .ok()
        .map(ProviderId)
        .ok_or(ChatError::Model)?;
    let name = access
        .providers()
        .iter()
        .find(|p| p.id == provider)
        .map(|p| p.name.clone())
        .ok_or(ChatError::Model)?;
    let key = crate::overview_places::Place::new(access.binding().clone()).key;
    let namespace = access.binding().watched_namespace.clone();
    let opener = Opener {
        session: session.clone(),
        access,
        reader,
        provider,
    };
    ClusterChannel::new(key, namespace, name, Arc::new(opener)).map(Arc::new)
}
struct Opener {
    session: ClusterSession,
    access: Arc<RemoteAccess>,
    reader: KubeconfigReader,
    provider: ProviderId,
}
impl ClusterOpen for Opener {
    /// A fresh kubeconfig read and destination revalidation for every worker run, as Send does.
    fn open(&self) -> WireFuture<'_, Result<Arc<dyn ClusterWire>, ChatError>> {
        Box::pin(async move {
            let kubeconfig = self
                .session
                .read_configuration(self.reader.clone())
                .await
                .map_err(|_| ChatError::ClusterUnavailable)?;
            let fresh = self.access.fresh_client(kubeconfig);
            let transport = self
                .access
                .transport(&fresh, self.provider)
                .await
                .map_err(|_| ChatError::ClusterUnavailable)?;
            Ok(Arc::new(Wire(transport)) as Arc<dyn ClusterWire>)
        })
    }
}
struct Wire(OrkaStreamTransport);
impl ClusterWire for Wire {
    fn exchange(&self, request: WireRequest) -> WireFuture<'_, Result<(u16, Vec<u8>), ChatError>> {
        Box::pin(async move {
            self.0
                .exchange(request.method, &request.uri, request.body.as_deref())
                .await
                .map_err(|error| match error {
                    // The destination recheck before a write failed: not an Orka refusal.
                    ErrorKind::Identity | ErrorKind::Permission => ChatError::ClusterUnavailable,
                    _ => ChatError::Connection,
                })
        })
    }
    fn close(&self) -> WireFuture<'_, ()> {
        RunTransport::close(&self.0)
    }
}

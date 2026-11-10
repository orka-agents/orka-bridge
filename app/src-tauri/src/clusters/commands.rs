//! ID-only desktop boundary. Credentials and confirmed access stay in this native owner.
use super::*;
use kube::config::Kubeconfig;
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AccessStatus {
    Connected,
    NeedServiceAccount,
    Denied,
    Unavailable,
    UnsupportedCredential,
}
#[derive(Serialize)]
pub struct AccessView {
    pub status: AccessStatus,
    pub providers: Vec<ProviderView>,
}
impl From<AccessOutcome> for AccessView {
    fn from(outcome: AccessOutcome) -> Self {
        Self {
            status: match outcome {
                AccessOutcome::NeedServiceAccount => AccessStatus::NeedServiceAccount,
                AccessOutcome::Denied => AccessStatus::Denied,
                AccessOutcome::Unavailable => AccessStatus::Unavailable,
                AccessOutcome::UnsupportedCredential => AccessStatus::UnsupportedCredential,
            },
            providers: Vec::new(),
        }
    }
}
#[derive(Default)]
struct Session {
    client: Option<ClusterClient>,
    // Send shares this frozen access; a later Use replaces it without switching a running attempt.
    access: Option<(TargetId, Arc<RemoteAccess>)>,
}
impl Session {
    fn client(&mut self) -> Result<&mut ClusterClient, &'static str> {
        if self.client.is_none() {
            self.client = Some(
                ClusterClient::read(Arc::new(PodOrkaRead))
                    .map_err(|_| "kubectl contexts are unavailable.")?,
            );
        }
        self.client
            .as_mut()
            .ok_or("kubectl contexts are unavailable.")
    }
}
/// Reads the current kubeconfig for each Send; tests inject fixtures here.
pub type KubeconfigReader = Arc<dyn Fn() -> Result<Kubeconfig, ClusterError> + Send + Sync>;
#[derive(Clone)]
pub struct ClusterSession(Arc<Mutex<Session>>, KubeconfigReader);
impl Default for ClusterSession {
    fn default() -> Self {
        Self(Arc::default(), Arc::new(read_kubeconfig))
    }
}
fn read_kubeconfig() -> Result<Kubeconfig, ClusterError> {
    Kubeconfig::read().map_err(|_| ClusterError::Unavailable)
}
impl ClusterSession {
    pub fn new(client: ClusterClient) -> Self {
        Self::with_reader(client, Arc::new(read_kubeconfig))
    }
    pub fn with_reader(client: ClusterClient, reader: KubeconfigReader) -> Self {
        Self(
            Arc::new(Mutex::new(Session {
                client: Some(client),
                access: None,
            })),
            reader,
        )
    }
    /// Memory-only lookup; the caller rechecks a fresh kubeconfig before writing.
    pub(crate) async fn send_access(
        &self,
        target: &str,
    ) -> (Option<Arc<RemoteAccess>>, KubeconfigReader) {
        let access = self
            .0
            .lock()
            .await
            .access
            .as_ref()
            .and_then(|(id, access)| (id.0.to_string() == target).then(|| access.clone()));
        (access, self.1.clone())
    }
    pub async fn contexts(&self) -> Result<Vec<ContextView>, &'static str> {
        self.0
            .lock()
            .await
            .client()?
            .contexts()
            .map_err(|_| "kubectl contexts are unavailable.")
    }
    pub async fn discover(&self, context_id: &str) -> Result<Vec<TargetView>, AccessStatus> {
        let mut session = self.0.lock().await;
        session.access = None;
        let client = session.client().map_err(|_| AccessStatus::Unavailable)?;
        let context = client
            .contexts()
            .map_err(|_| AccessStatus::Unavailable)?
            .into_iter()
            .find(|c| c.id.0.to_string() == context_id)
            .ok_or(AccessStatus::Unavailable)?;
        client
            .discover(context.id)
            .await
            .map_err(|error| match error {
                ClusterError::Denied => AccessStatus::Denied,
                ClusterError::UnsupportedCredential => AccessStatus::UnsupportedCredential,
                _ => AccessStatus::Unavailable,
            })
    }
    pub async fn use_target(&self, target_id: &str, sa_name: Option<String>) -> AccessView {
        let mut session = self.0.lock().await;
        session.access = None;
        let id = Uuid::parse_str(target_id).ok().map(TargetId);
        let result = async {
            let client = session.client().map_err(|_| AccessOutcome::Unavailable)?;
            client
                .connect(id.ok_or(AccessOutcome::Unavailable)?, sa_name)
                .await
        }
        .await;
        match result {
            Ok(access) => {
                let view = AccessView {
                    status: AccessStatus::Connected,
                    providers: access.providers().to_vec(),
                };
                session.access = id.map(|id| (id, Arc::new(access)));
                view
            }
            Err(outcome) => outcome.into(),
        }
    }
}

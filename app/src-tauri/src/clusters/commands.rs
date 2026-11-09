//! ID-only desktop boundary. Credentials and confirmed access stay in this native owner.
use super::*;
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
    // Task 4 consumes this access, rather than asking the renderer for a credential.
    access: Option<RemoteAccess>,
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
#[derive(Default)]
pub struct ClusterSession(Mutex<Session>);
impl ClusterSession {
    pub fn new(client: ClusterClient) -> Self {
        Self(Mutex::new(Session {
            client: Some(client),
            access: None,
        }))
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
        let result = async {
            let client = session.client().map_err(|_| AccessOutcome::Unavailable)?;
            let id = Uuid::parse_str(target_id).map_err(|_| AccessOutcome::Unavailable)?;
            client.connect(TargetId(id), sa_name).await
        }
        .await;
        match result {
            Ok(access) => {
                let view = AccessView {
                    status: AccessStatus::Connected,
                    providers: access.providers().to_vec(),
                };
                session.access = Some(access);
                view
            }
            Err(outcome) => outcome.into(),
        }
    }
}

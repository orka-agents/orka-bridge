//! ID-only desktop boundary. Credentials and confirmed access stay in this native owner.
use super::credentials::ExecOwner;
use super::*;
use kube::config::Kubeconfig;
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::sync::Mutex;

const READ_LIMIT: Duration = Duration::from_secs(10);
const UNKNOWN_CONFIGURATION: ClusterFailure = ClusterFailure {
    check: Check::Configuration,
    result: CheckResult::Unknown,
};

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
    /// Null exactly when connected or a ServiceAccount name is needed.
    pub failure: Option<ClusterFailure>,
}
#[derive(Serialize)]
pub struct SelectedCluster {
    pub target: TargetView,
    pub providers: Vec<ProviderView>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClusterPlaceSnapshot {
    pub places: Vec<crate::overview_places::Place>,
    pub selected_key: Option<String>,
    pub persistence_state: &'static str,
    /// The selected place's access can no longer authenticate: it needs reconnecting.
    pub access_lost: bool,
}
impl From<AccessOutcome> for AccessView {
    fn from(outcome: AccessOutcome) -> Self {
        // Connect tags every failure at its origin; the fallback check is never reached.
        let failure = (outcome != AccessOutcome::NeedServiceAccount)
            .then(|| outcome.failure(Check::ClusterApi));
        Self {
            status: match failure.map(ClusterFailure::legacy) {
                None => AccessStatus::NeedServiceAccount,
                Some(ClusterError::Denied) => AccessStatus::Denied,
                Some(ClusterError::UnsupportedCredential) => AccessStatus::UnsupportedCredential,
                Some(_) => AccessStatus::Unavailable,
            },
            providers: Vec::new(),
            failure,
        }
    }
}
#[derive(Default)]
struct Session {
    client: Option<ClusterClient>,
}
impl Session {
    /// Lazily reads configuration off the executor. A read that misses its budget is dropped,
    /// never published later; the plugin owner and Orka reader outlive every reload.
    async fn client(
        &mut self,
        owner: &ClusterSession,
    ) -> Result<&mut ClusterClient, ClusterFailure> {
        let client = match self.client.take() {
            Some(client) => client,
            None => {
                let kubeconfig = owner.read_configuration(owner.1.clone()).await?;
                ClusterClient::new(kubeconfig, owner.3.clone()).with_exec_owner(owner.2.clone())
            }
        };
        Ok(self.client.insert(client))
    }
}
/// Reads the current kubeconfig for each Send; tests inject fixtures here.
pub type KubeconfigReader = Arc<dyn Fn() -> Result<Kubeconfig, ClusterError> + Send + Sync>;
type ConfirmedAccess = Option<(TargetId, Arc<RemoteAccess>)>;
pub(crate) type SendAccess = (Option<Arc<RemoteAccess>>, KubeconfigReader);
#[derive(Clone)]
pub struct ClusterSession(
    Arc<Mutex<Session>>,
    KubeconfigReader,
    ExecOwner,
    Arc<dyn OrkaRead>,
    // Memory-only admission snapshot, independent of bounded discovery/Use waits.
    Arc<std::sync::Mutex<ConfirmedAccess>>,
    Arc<AtomicBool>,
    Arc<crate::overview_places::Inventory>,
);
impl Default for ClusterSession {
    fn default() -> Self {
        Self(
            Arc::default(),
            Arc::new(read_kubeconfig),
            ExecOwner::default(),
            Arc::new(PodOrkaRead),
            Arc::default(),
            Arc::default(),
            Arc::default(),
        )
    }
}
fn read_kubeconfig() -> Result<Kubeconfig, ClusterError> {
    Kubeconfig::read().map_err(|_| ClusterError::Unavailable)
}
impl ClusterSession {
    /// Desktop-only persistence. Other constructors deliberately remain memory-only.
    pub fn with_storage(app_local_data: Option<std::path::PathBuf>) -> Self {
        Self(
            Arc::default(),
            Arc::new(read_kubeconfig),
            ExecOwner::default(),
            Arc::new(PodOrkaRead),
            Arc::default(),
            Arc::default(),
            Arc::new(crate::overview_places::Inventory::with_storage(
                app_local_data,
            )),
        )
    }
    /// All configuration reads share one slot, including reads abandoned by Stop or timeout.
    pub(crate) async fn read_configuration(
        &self,
        reader: KubeconfigReader,
    ) -> Result<Kubeconfig, ClusterFailure> {
        if self.5.swap(true, Ordering::SeqCst) {
            return Err(UNKNOWN_CONFIGURATION);
        }
        let running = self.5.clone();
        let (reply, read) = tokio::sync::oneshot::channel();
        // A stuck file read cannot be cancelled. Admit at most one, outside
        // Tokio's blocking pool so it cannot hold up runtime shutdown.
        if std::thread::Builder::new()
            .name("bridge-kubeconfig".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| reader()))
                    .unwrap_or(Err(ClusterError::Unavailable));
                running.store(false, Ordering::SeqCst);
                let _ = reply.send(result);
            })
            .is_err()
        {
            self.5.store(false, Ordering::SeqCst);
            return Err(UNKNOWN_CONFIGURATION);
        }
        // A late completion only releases the slot; it cannot publish configuration.
        tokio::time::timeout(READ_LIMIT, read)
            .await
            .ok()
            .and_then(Result::ok)
            .and_then(Result::ok)
            .ok_or(UNKNOWN_CONFIGURATION)
    }
    pub fn new(client: ClusterClient) -> Self {
        Self::with_reader(client, Arc::new(read_kubeconfig))
    }
    pub fn with_reader(client: ClusterClient, reader: KubeconfigReader) -> Self {
        let (exec, orka) = (client.exec_owner(), client.orka());
        Self(
            Arc::new(Mutex::new(Session {
                client: Some(client),
            })),
            reader,
            exec,
            orka,
            Arc::default(),
            Arc::default(),
            Arc::default(),
        )
    }
    /// Independent of the discovery/Use lock, so close can interrupt a running plugin.
    pub async fn shutdown(&self) {
        self.6.invalidate(None);
        self.2.shutdown().await;
    }
    pub async fn overview(&self) -> Vec<crate::overview_places::Place> {
        self.6.refresh().await
    }
    pub async fn overview_one(&self, key: &str) -> Vec<crate::overview_places::Place> {
        self.6.refresh_one(key).await
    }
    /// A display-only snapshot: never reads kubeconfig, refreshes inventory or renews access.
    pub fn places(&self) -> ClusterPlaceSnapshot {
        // Do not hold the admission lock while reading the independent inventory.
        let selected = self
            .4
            .lock()
            .unwrap()
            .as_ref()
            .map(|(_, access)| (access.binding().clone(), access.lost()));
        ClusterPlaceSnapshot {
            places: self.6.snapshot(),
            persistence_state: self.6.persistence_state(),
            access_lost: selected.as_ref().is_some_and(|(_, lost)| *lost),
            selected_key: selected
                .map(|(binding, _)| crate::overview_places::Place::new(binding).key),
        }
    }
    /// Memory-only lookup; the caller rechecks a fresh kubeconfig before writing.
    pub(crate) fn send_access(&self, target: &str) -> SendAccess {
        let access = self
            .4
            .lock()
            .unwrap()
            .as_ref()
            .and_then(|(id, access)| (id.0.to_string() == target).then(|| access.clone()));
        (access, self.1.clone())
    }
    pub(crate) fn history_access(&self) -> SendAccess {
        (
            self.4
                .lock()
                .unwrap()
                .as_ref()
                .map(|(_, access)| access.clone()),
            self.1.clone(),
        )
    }
    pub async fn catalog(
        &self,
        target: &str,
        provider: &str,
    ) -> Result<catalog::CatalogReply, &'static str> {
        use crate::engine::run::RunTransport;
        let failure = "The selected cluster catalog could not be read.";
        tokio::time::timeout(READ_LIMIT, async {
            // Serialize with Use/Forget so a catalog never renews another target's access.
            let _session = self.0.lock().await;
            let access = self.send_access(target).0.ok_or(failure)?;
            let provider = Uuid::parse_str(provider)
                .ok()
                .map(ProviderId)
                .ok_or(failure)?;
            if !access.providers().iter().any(|p| p.id == provider) {
                return Err(failure);
            }
            let current = access.fresh_client(
                self.read_configuration(self.1.clone())
                    .await
                    .map_err(|_| failure)?,
            );
            let stream = access.read_transport(&current).await.map_err(|_| failure)?;
            let result = async {
                access.recheck_reads(&stream).await.map_err(|_| failure)?;
                access
                    .designer_catalog(&stream, provider)
                    .await
                    .map_err(|_| failure)
            }
            .await;
            stream.close().await;
            result
        })
        .await
        .map_err(|_| failure)?
    }
    pub async fn contexts(&self) -> Result<Vec<ContextView>, ClusterFailure> {
        let mut session = self.0.lock().await;
        session
            .client(self)
            .await?
            .contexts()
            .map_err(|_| UNKNOWN_CONFIGURATION)
    }
    /// Rereads configuration offline, queued behind any discovery or Use. A context whose own
    /// kubeconfig entry is unchanged keeps its IDs and confirmed access; every other ID and
    /// access is invalid, as all are if the read fails. A running Send keeps its frozen access.
    pub async fn reload(&self) -> Result<Vec<ContextView>, ClusterFailure> {
        let mut session = self.0.lock().await;
        self.reread(&mut session).await?;
        session
            .client
            .as_ref()
            .ok_or(UNKNOWN_CONFIGURATION)?
            .contexts()
            .map_err(|_| UNKNOWN_CONFIGURATION)
    }
    async fn reread(&self, session: &mut Session) -> Result<(), ClusterFailure> {
        let old = session.client.take();
        let read = session.client(self).await;
        let client = match read {
            Ok(client) => client,
            Err(failure) => {
                self.6.invalidate(None);
                *self.4.lock().unwrap() = None;
                return Err(failure);
            }
        };
        match &old {
            Some(old) => client.adopt_unchanged(old),
            None => {
                self.6.invalidate(None);
                *self.4.lock().unwrap() = None;
                return Ok(());
            }
        }
        self.6.retain_access(|binding| client.keeps(binding));
        let mut selected = self.4.lock().unwrap();
        if selected
            .as_ref()
            .is_some_and(|(_, access)| !client.keeps(access.binding()))
        {
            *selected = None;
        }
        Ok(())
    }
    /// Browsing another context never retires the access already in use.
    pub async fn discover(&self, context_id: &str) -> Result<Vec<TargetView>, ClusterFailure> {
        let mut session = self.0.lock().await;
        let client = session.client(self).await?;
        let context = client
            .contexts()
            .map_err(|_| UNKNOWN_CONFIGURATION)?
            .into_iter()
            .find(|c| c.id.0.to_string() == context_id)
            .ok_or(UNKNOWN_CONFIGURATION)?;
        client.discover(context.id).await
    }
    /// Select only current retained access and its exact, existing discovery handle.
    /// A failed selection leaves no previous destination admitted to the renderer.
    pub async fn select(&self, key: &str) -> Result<SelectedCluster, ClusterFailure> {
        let session = self.0.lock().await;
        *self.4.lock().unwrap() = None;
        let access = self.6.connected_access(key).ok_or(UNKNOWN_CONFIGURATION)?;
        let target = session
            .client
            .as_ref()
            .and_then(|client| client.existing_target(access.binding()))
            .ok_or(UNKNOWN_CONFIGURATION)?;
        let providers = access.providers().to_vec();
        *self.4.lock().unwrap() = Some((target.id, access));
        Ok(SelectedCluster { target, providers })
    }
    /// Resolve saved identity against freshly read configuration and fresh discovery only.
    pub async fn reconnect(&self, key: &str) -> Result<TargetView, ClusterFailure> {
        let mut session = self.0.lock().await;
        let saved = self.6.get(key).ok_or(UNKNOWN_CONFIGURATION)?;
        // Rechecking a remembered place rereads kubeconfig, keeping the place in use unless
        // its own context changed; only Use replaces it.
        self.reread(&mut session).await?;
        let client = session.client.as_mut().ok_or(UNKNOWN_CONFIGURATION)?;
        let contexts: Vec<_> = client
            .contexts()
            .map_err(|_| UNKNOWN_CONFIGURATION)?
            .into_iter()
            .filter(|c| c.name == saved.context)
            .collect();
        if contexts.len() != 1 {
            return Err(UNKNOWN_CONFIGURATION);
        }
        let targets = client.discover(contexts[0].id).await?;
        let mut matches = targets.into_iter().filter(|t| {
            client
                .target_binding(t.id)
                .is_some_and(|b| saved.matches(b))
        });
        let rejected = ClusterFailure {
            check: Check::Compatibility,
            result: CheckResult::IdentityRejected,
        };
        let target = matches.next().ok_or(rejected)?;
        if matches.next().is_some() {
            return Err(rejected);
        }
        Ok(target)
    }
    /// Serialize behind Use. Already-admitted Sends retain their independent frozen Arc.
    pub async fn forget(&self, key: &str) -> Result<(), &'static str> {
        let mut session = self.0.lock().await;
        self.6.forget(key)?;
        if let Some(client) = session.client.as_mut() {
            client.forget_targets(key);
        }
        let mut access = self.4.lock().unwrap();
        if access.as_ref().is_some_and(|(_, a)| {
            crate::overview_places::Place::new(a.binding().clone()).key == key
        }) {
            *access = None;
        }
        Ok(())
    }
    pub async fn service_accounts(
        &self,
        target_id: &str,
    ) -> Result<ServiceAccountsView, ClusterFailure> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        let unknown = Check::ServiceAccount
            .unknown()
            .failure(Check::ServiceAccount);
        tokio::time::timeout_at(deadline, async {
            let mut session = self.0.lock().await;
            let id = Uuid::parse_str(target_id)
                .ok()
                .map(TargetId)
                .ok_or(unknown)?;
            session
                .client(self)
                .await?
                .service_accounts(id, deadline)
                .await
        })
        .await
        .unwrap_or(Err(unknown))
    }
    pub async fn service_account_preview(
        &self,
        target_id: &str,
        sa_name: &str,
    ) -> Result<ServiceAccountPreview, ClusterFailure> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        let unknown = Check::ServiceAccount
            .unknown()
            .failure(Check::ServiceAccount);
        tokio::time::timeout_at(deadline, async {
            let mut session = self.0.lock().await;
            let id = Uuid::parse_str(target_id)
                .ok()
                .map(TargetId)
                .ok_or(unknown)?;
            session
                .client(self)
                .await?
                .service_account_preview(id, sa_name, deadline)
                .await
        })
        .await
        .unwrap_or(Err(unknown))
    }
    pub async fn use_target(&self, target_id: &str, sa_name: Option<String>) -> AccessView {
        let mut session = self.0.lock().await;
        *self.4.lock().unwrap() = None;
        let id = Uuid::parse_str(target_id).ok().map(TargetId);
        let result = async {
            let client = session.client(self).await.map_err(AccessOutcome::Failed)?;
            let id = id.ok_or(AccessOutcome::Failed(UNKNOWN_CONFIGURATION))?;
            if let Some(binding) = client.target_binding(id) {
                self.6.invalidate(Some(binding));
            }
            client.connect(id, sa_name).await
        }
        .await;
        match result {
            Ok(access) => {
                let view = AccessView {
                    status: AccessStatus::Connected,
                    providers: access.providers().to_vec(),
                    failure: None,
                };
                let access = Arc::new(access);
                bearer::keep_renewed(&access.bearer);
                self.6.remember(access.clone());
                *self.4.lock().unwrap() = id.map(|id| (id, access));
                view
            }
            Err(outcome) => outcome.into(),
        }
    }
}

#[cfg(test)]
mod browsing_tests {
    use super::*;

    fn kubeconfig(server_a: &str) -> Kubeconfig {
        Kubeconfig::from_yaml(&format!(
            "apiVersion: v1\nkind: Config\ncurrent-context: a\nclusters:\n- name: a\n  cluster:\n    server: {server_a}\n- name: b\n  cluster:\n    server: http://127.0.0.1:2\nusers:\n- name: u\n  user:\n    token: fixture\ncontexts:\n- name: a\n  context:\n    cluster: a\n    user: u\n- name: b\n  context:\n    cluster: b\n    user: u\n"
        ))
        .unwrap()
    }
    fn binding(context: &ContextView) -> TargetBinding {
        TargetBinding {
            cluster_identity: "a".repeat(64),
            context_id: context.id,
            context_name: context.name.clone(),
            server_address: "http://127.0.0.1:1/".into(),
            service_name: "orka".into(),
            service_namespace: "control".into(),
            service_uid: "service".into(),
            deployment_name: "controller".into(),
            deployment_namespace: "control".into(),
            deployment_uid: "deployment".into(),
            watched_namespace: "agents".into(),
            image: "orka:fixture".into(),
            api_port: 8080,
        }
    }

    #[tokio::test]
    async fn browsing_and_reloading_keep_the_place_in_use_until_its_context_changes() {
        let current = Arc::new(std::sync::Mutex::new(kubeconfig("http://127.0.0.1:1")));
        let reader: KubeconfigReader = {
            let current = current.clone();
            Arc::new(move || Ok(current.lock().unwrap().clone()))
        };
        let client = ClusterClient::new(kubeconfig("http://127.0.0.1:1"), Arc::new(PodOrkaRead));
        let owner = ClusterSession::with_reader(client, reader);
        let contexts = owner.contexts().await.unwrap();
        let a = contexts.iter().find(|c| c.name == "a").unwrap().clone();
        let b = contexts.iter().find(|c| c.name == "b").unwrap().clone();
        let target = TargetId(Uuid::new_v4());
        let access = Arc::new(RemoteAccess::fixture(binding(&a), Arc::new(PodOrkaRead)));
        owner
            .0
            .lock()
            .await
            .client
            .as_mut()
            .unwrap()
            .targets
            .insert(target, binding(&a));
        owner.6.remember(access.clone());
        *owner.4.lock().unwrap() = Some((target, access));
        let key = crate::overview_places::Place::new(binding(&a)).key;

        // Discovering another context (here unreachable) only browses.
        let _ = owner.discover(&b.id.0.to_string()).await;
        assert_eq!(owner.places().selected_key.as_deref(), Some(key.as_str()));
        // An unchanged reload keeps ids, discovered targets and the confirmed access.
        let reloaded = owner.reload().await.unwrap();
        assert!(reloaded.iter().any(|c| c.id == a.id && c.name == "a"));
        assert_eq!(owner.places().selected_key.as_deref(), Some(key.as_str()));
        assert!(owner.send_access(&target.0.to_string()).0.is_some());
        // Changing that context's own entry retires its access and ids.
        *current.lock().unwrap() = kubeconfig("http://127.0.0.1:3");
        let reloaded = owner.reload().await.unwrap();
        assert!(reloaded.iter().all(|c| c.id != a.id));
        assert!(owner.places().selected_key.is_none());
        assert!(owner.send_access(&target.0.to_string()).0.is_none());
    }
}

#[cfg(test)]
mod service_account_tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn service_account_lock_queue_expires_without_publishing_late_work() {
        let owner = ClusterSession::default();
        let lock = owner.0.lock().await;
        let list = tokio::spawn({
            let owner = owner.clone();
            async move {
                owner
                    .service_accounts("00000000-0000-0000-0000-000000000000")
                    .await
            }
        });
        let preview = tokio::spawn({
            let owner = owner.clone();
            async move {
                owner
                    .service_account_preview("00000000-0000-0000-0000-000000000000", "runner")
                    .await
            }
        });
        tokio::task::yield_now().await;
        assert!(!list.is_finished() && !preview.is_finished());
        tokio::time::advance(Duration::from_secs(31)).await;
        tokio::task::yield_now().await;
        assert!(
            list.is_finished() && preview.is_finished(),
            "queue time must consume the operation budget"
        );
        assert!(list.await.unwrap().is_err());
        assert!(preview.await.unwrap().is_err());
        drop(lock);
        tokio::task::yield_now().await;
        assert!(
            owner.0.lock().await.client.is_none(),
            "expired waiter must never initialize or publish a client later"
        );
    }
}

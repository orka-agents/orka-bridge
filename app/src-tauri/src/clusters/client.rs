use super::bearer::{Bearer, Mint, MintFuture, Minted, Refused};
use super::credentials::{self, selected_config, ExecOwner};
use super::*;
use k8s_openapi::{
    api::{
        apps::v1::Deployment,
        authentication::v1::{TokenRequest, TokenRequestSpec},
        authorization::v1::{
            ResourceAttributes, SelfSubjectAccessReview, SelfSubjectAccessReviewSpec,
        },
        core::v1::{Service, ServiceAccount},
    },
    apiextensions_apiserver::pkg::apis::apiextensions::v1::CustomResourceDefinition,
    apimachinery::pkg::util::intstr::IntOrString,
};
use kube::{
    api::{ListParams, PostParams},
    config::Kubeconfig,
    Api, Client, ResourceExt,
};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::Duration,
};
use tokio::time::{timeout_at, Instant};

#[path = "service_accounts.rs"]
mod service_accounts;

#[cfg(test)]
#[path = "inventory_tests.rs"]
mod inventory_tests;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
/// One discovery or Use, from taking the session lock, including every Provider page.
const OPERATION_LIMIT: Duration = Duration::from_secs(30);
#[derive(PartialEq)]
enum Identity {
    NeedAccount,
    TokenReview,
    Oidc,
}
#[derive(Clone)]
pub struct ClusterClient {
    kubeconfig: Kubeconfig,
    contexts: Vec<ContextView>,
    pub(super) targets: HashMap<TargetId, TargetBinding>,
    orka: Arc<dyn OrkaRead>,
    exec: ExecOwner,
}
#[cfg(test)]
#[path = "catalog_read_tests.rs"]
mod catalog_read_tests;

/// Native access only: no Serialize/Debug implementation can export credentials.
pub struct RemoteAccess {
    pub(crate) creator: Client,
    /// Shared by every consumer of this access, so a renewal reaches running Sends too.
    pub(crate) bearer: Arc<Bearer>,
    // Orka has seen this credential even after the confirmed ServiceAccount fallback.
    pub(super) creator_bearer: String,
    pub(super) binding: TargetBinding,
    orka: Arc<dyn OrkaRead>,
    providers: Vec<ProviderView>,
}
impl ClusterClient {
    pub fn new(kubeconfig: Kubeconfig, orka: Arc<dyn OrkaRead>) -> Self {
        let contexts = kubeconfig
            .contexts
            .iter()
            .map(|c| ContextView {
                id: ContextId(Uuid::new_v4()),
                name: c.name.clone(),
                current: kubeconfig.current_context.as_deref() == Some(c.name.as_str()),
            })
            .collect();
        Self {
            kubeconfig,
            contexts,
            targets: HashMap::new(),
            orka,
            exec: ExecOwner::default(),
        }
    }
    pub async fn shutdown(&self) {
        self.exec.shutdown().await;
    }
    pub(super) fn exec_owner(&self) -> ExecOwner {
        self.exec.clone()
    }
    pub(super) fn orka(&self) -> Arc<dyn OrkaRead> {
        self.orka.clone()
    }
    pub(super) fn with_exec_owner(mut self, exec: ExecOwner) -> Self {
        self.exec = exec;
        self
    }
    pub(crate) fn target_binding(&self, id: TargetId) -> Option<&TargetBinding> {
        self.targets.get(&id)
    }
    /// Resolve an already-discovered binding without minting IDs or refreshing access.
    pub(crate) fn existing_target(&self, binding: &TargetBinding) -> Option<TargetView> {
        let mut matches = self.targets.iter().filter(|(_, b)| *b == binding);
        let (id, binding) = matches.next()?;
        if matches.next().is_some() {
            return None;
        }
        Some(TargetView {
            id: *id,
            context_id: binding.context_id,
            context_name: binding.context_name.clone(),
            service_name: binding.service_name.clone(),
            service_namespace: binding.service_namespace.clone(),
            watched_namespace: binding.watched_namespace.clone(),
            image: binding.image.clone(),
        })
    }
    pub(crate) fn forget_targets(&mut self, key: &str) {
        self.targets
            .retain(|_, binding| crate::overview_places::Place::new(binding.clone()).key != key);
    }
    pub fn contexts(&self) -> Result<Vec<ContextView>, ClusterError> {
        Ok(self.contexts.clone())
    }
    /// A reread kubeconfig keeps the ids and discovered installations of every context whose
    /// own entry (context, cluster and user) is unchanged, so browsing never retires them.
    pub(super) fn adopt_unchanged(&mut self, old: &ClusterClient) {
        for context in &mut self.contexts {
            let Some(previous) = old.contexts.iter().find(|c| c.name == context.name) else {
                continue;
            };
            if !same_entry(&old.kubeconfig, &self.kubeconfig, &context.name) {
                continue;
            }
            context.id = previous.id;
            for (id, binding) in &old.targets {
                if binding.context_id == previous.id {
                    self.targets.insert(*id, binding.clone());
                }
            }
        }
    }
    /// Whether this configuration still holds the exact context a binding was discovered with.
    pub(super) fn keeps(&self, binding: &TargetBinding) -> bool {
        self.contexts
            .iter()
            .any(|c| c.id == binding.context_id && c.name == binding.context_name)
    }
    pub async fn resolve_creator(
        &self,
        context_id: ContextId,
        deadline: Instant,
    ) -> Result<ResolvedCreator, ClusterError> {
        self.resolve(context_id, deadline)
            .await
            .map_err(cluster_error)
    }
    async fn resolve(
        &self,
        context_id: ContextId,
        deadline: Instant,
    ) -> Result<ResolvedCreator, AccessOutcome> {
        let name = &self
            .contexts
            .iter()
            .find(|c| c.id == context_id)
            .ok_or(Check::Configuration.unknown())?
            .name;
        credentials::resolve(&self.kubeconfig, name, deadline, &self.exec).await
    }
    pub async fn discover(&mut self, id: ContextId) -> Result<Vec<TargetView>, ClusterFailure> {
        self.discover_targets(id)
            .await
            .map_err(|e| e.failure(Check::ClusterApi))
    }
    async fn discover_targets(&mut self, id: ContextId) -> Result<Vec<TargetView>, AccessOutcome> {
        let deadline = Instant::now() + OPERATION_LIMIT;
        let config = within(deadline, Check::Credentials, self.resolve(id, deadline))
            .await?
            .config;
        let client =
            Client::try_from(config.clone()).map_err(|_| Check::Configuration.unknown())?;
        let name = &self
            .contexts
            .iter()
            .find(|c| c.id == id)
            .ok_or(Check::Configuration.unknown())?
            .name;
        let server = config.cluster_url.to_string();
        let identity = super::destination::fingerprint(&config)?;
        let bindings = installations(client, id, name, &server, &identity, deadline).await?;
        let mut views = Vec::new();
        for binding in bindings {
            let target_id = self
                .targets
                .iter()
                .find(|(_, b)| **b == binding)
                .map(|(id, _)| *id)
                .unwrap_or(TargetId(Uuid::new_v4()));
            views.push(TargetView {
                id: target_id,
                context_id: id,
                context_name: name.clone(),
                service_name: binding.service_name.clone(),
                service_namespace: binding.service_namespace.clone(),
                watched_namespace: binding.watched_namespace.clone(),
                image: binding.image.clone(),
            });
            self.targets.insert(target_id, binding);
        }
        Ok(views)
    }
    /// A newly read kubeconfig can validate an existing access without switching its frozen identity.
    pub async fn revalidate(&self, access: &RemoteAccess) -> Result<(), ClusterError> {
        self.revalidate_binding(&access.binding, &access.creator)
            .await
    }
    pub(super) async fn revalidate_binding(
        &self,
        binding: &TargetBinding,
        creator: &Client,
    ) -> Result<(), ClusterError> {
        self.check_binding(binding, creator, Instant::now() + OPERATION_LIMIT)
            .await
            .map_err(cluster_error)
    }
    async fn check_binding(
        &self,
        binding: &TargetBinding,
        creator: &Client,
        deadline: Instant,
    ) -> Result<(), AccessOutcome> {
        let config =
            selected_config(&self.kubeconfig, &binding.context_name, "binding-only").await?;
        if config.cluster_url.to_string() != binding.server_address
            || super::destination::fingerprint(&config)? != binding.cluster_identity
        {
            return Err(Check::Configuration.unknown());
        }
        let bindings = installations(
            creator.clone(),
            binding.context_id,
            &binding.context_name,
            &config.cluster_url.to_string(),
            &binding.cluster_identity,
            deadline,
        )
        .await?;
        if bindings.contains(binding) {
            Ok(())
        } else {
            Err(Check::ClusterApi.unknown())
        }
    }
    /// Calling connect is the native Use confirmation; discovery never requests a token.
    pub async fn connect(
        &self,
        id: TargetId,
        sa_name: Option<String>,
    ) -> Result<RemoteAccess, AccessOutcome> {
        ACTIVE_CHECK
            .scope(
                std::cell::Cell::new(Check::Configuration),
                self.connect_checked(id, sa_name),
            )
            .await
    }
    async fn connect_checked(
        &self,
        id: TargetId,
        sa_name: Option<String>,
    ) -> Result<RemoteAccess, AccessOutcome> {
        use Check::{Agents, Configuration, Credentials, OrkaIdentity, Permissions, Providers};
        let deadline = Instant::now() + OPERATION_LIMIT;
        if sa_name.as_ref().is_some_and(|s| !dns_subdomain(s)) {
            return Err(Check::ServiceAccount.unknown());
        }
        let binding = self
            .targets
            .get(&id)
            .ok_or(Configuration.unknown())?
            .clone();
        let resolved = within(
            deadline,
            Credentials,
            self.resolve(binding.context_id, deadline),
        )
        .await?;
        if super::destination::fingerprint(&resolved.config)? != binding.cluster_identity {
            return Err(Configuration.unknown());
        }
        let (bearer, expiry) = match resolved.credential {
            Credential::Bearer {
                value,
                known_expiry,
            } => (value, known_expiry.as_ref().map(instant)),
            Credential::CertificateOnly => (String::new(), None),
        };
        let creator = Client::try_from(resolved.config).map_err(|_| Configuration.unknown())?;
        let mut access = RemoteAccess {
            creator,
            creator_bearer: bearer.clone(),
            bearer: Arc::new(Bearer::fixed(bearer, expiry)),
            binding,
            orka: self.orka.clone(),
            providers: Vec::new(),
        };
        self.check_binding(&access.binding, &access.creator, deadline)
            .await?;
        let mut identity = if access.bearer.peek().is_empty() {
            Identity::NeedAccount
        } else {
            within(deadline, OrkaIdentity, access.identity()).await?
        };
        if identity == Identity::NeedAccount {
            let name = sa_name.ok_or(AccessOutcome::NeedServiceAccount)?;
            let issue = issue_token(&access.creator, &access.binding.watched_namespace, &name);
            let minted = within(deadline, Check::ServiceAccount, issue).await?;
            let token = minted.token.clone();
            let whoami =
                within(deadline, OrkaIdentity, access.whoami(&*access.orka, &token)).await?;
            let subject;
            (identity, subject) = access.identity_of(&whoami)?;
            if identity == Identity::NeedAccount {
                return Err(OrkaIdentity.fail(CheckResult::IdentityRejected));
            }
            // Renewal mints again for this exact account, as this same sign-in, only while
            // Orka still names the same subject. Without one to compare, it simply expires.
            access.bearer = Arc::new(
                match subject.filter(|_| identity == Identity::TokenReview) {
                    Some(subject) => Bearer::minted(
                        minted,
                        Arc::new(AccountMint {
                            client: self.clone(),
                            creator: access.creator.clone(),
                            binding: access.binding.clone(),
                            account: name,
                            subject,
                            creator_bearer: access.creator_bearer.clone(),
                        }),
                    ),
                    None => Bearer::fixed(minted.token, Some(minted.expires)),
                },
            );
        }
        // Even a certificate creator must check permissions as the frozen Orka bearer alone.
        if identity == Identity::TokenReview {
            let token_config = selected_config(
                &self.kubeconfig,
                &access.binding.context_name,
                &access.bearer.peek(),
            )
            .await?;
            let token_client =
                Client::try_from(token_config).map_err(|_| Configuration.unknown())?;
            let review = permissions(token_client, &access.binding.watched_namespace);
            within(deadline, Permissions, review).await?;
        }
        let agents = within(deadline, Agents, access.read(OrkaResource::Agents, None)).await?;
        if !agents["items"].is_array() {
            return Err(Agents.unknown());
        }
        let providers = access.configured_providers(&*access.orka);
        for (name, ready) in within(deadline, Providers, providers).await? {
            access.providers.push(ProviderView {
                id: ProviderId(Uuid::new_v4()),
                name,
                ready,
            });
        }
        if access.providers.is_empty() {
            return Err(Providers.fail(CheckResult::NoProviders));
        }
        Ok(access)
    }
}
#[cfg(test)]
impl RemoteAccess {
    /// Confirmed access for native tests; it reaches no server until a test sends through it.
    pub(crate) fn fixture(binding: TargetBinding, orka: Arc<dyn OrkaRead>) -> Self {
        Self {
            creator: Client::try_from(kube::Config::new("http://127.0.0.1:1".parse().unwrap()))
                .unwrap(),
            bearer: Arc::new(Bearer::fixed("fixture-bearer".into(), None)),
            creator_bearer: "fixture-creator".into(),
            binding,
            orka,
            providers: vec![ProviderView {
                id: ProviderId(Uuid::new_v4()),
                name: "fixture".into(),
                ready: true,
            }],
        }
    }
}
impl RemoteAccess {
    /// A complete bounded GET-only inventory using already confirmed access.
    pub async fn inventory(
        &self,
    ) -> Result<Vec<crate::overview_places::RemoteAgent>, ClusterError> {
        if self.bearer.lost() {
            return Err(ClusterError::Denied);
        }
        tokio::time::timeout(OPERATION_LIMIT, async {
            // A retained Service may now route to a recreated controller. Reuse
            // discovery's list permission, scoped to the one recorded Deployment.
            let deployments: Api<Deployment> =
                Api::namespaced(self.creator.clone(), &self.binding.deployment_namespace);
            let params = ListParams::default()
                .fields(&format!("metadata.name={}", self.binding.deployment_name))
                .limit(1);
            let listed = tokio::time::timeout(REQUEST_TIMEOUT, deployments.list(&params))
                .await
                .map_err(|_| ClusterError::Unavailable)?
                .map_err(|error| cluster_error(kube_error(Check::ClusterApi, error)))?;
            if listed
                .metadata
                .continue_
                .as_ref()
                .is_some_and(|s| !s.is_empty())
                || listed.items.len() != 1
                || listed.items[0].metadata.uid.as_deref() != Some(&self.binding.deployment_uid)
                || listed.items[0].metadata.name.as_deref() != Some(&self.binding.deployment_name)
                || listed.items[0].metadata.namespace.as_deref()
                    != Some(&self.binding.deployment_namespace)
            {
                return Err(ClusterError::Unavailable);
            }
            if !deployment_watches(&listed.items[0], &self.binding.watched_namespace) {
                return Err(ClusterError::Unavailable);
            }
            let mut agents = Vec::new();
            let mut after = None;
            let mut cursors = HashSet::new();
            let mut uids = HashSet::new();
            let mut bytes = 0;
            loop {
                let page = self
                    .read(OrkaResource::AgentInventory, after.as_deref())
                    .await
                    .map_err(cluster_error)?;
                bytes += serde_json::to_vec(&page)
                    .map_err(|_| ClusterError::Unavailable)?
                    .len();
                if bytes > 16 * 1024 * 1024 {
                    return Err(ClusterError::Unavailable);
                }
                for item in items(&page).map_err(cluster_error)? {
                    let agent =
                        crate::overview_places::agent(item, &self.binding.watched_namespace)?;
                    if !uids.insert(agent.uid.clone()) || agents.len() >= 4096 {
                        return Err(ClusterError::Unavailable);
                    }
                    agents.push(agent);
                }
                after = continuation(&page).map_err(cluster_error)?;
                if after.is_none() {
                    return Ok(agents);
                }
                if !cursors.insert(after.clone()) {
                    return Err(ClusterError::Unavailable);
                }
            }
        })
        .await
        .unwrap_or(Err(ClusterError::Unavailable))
    }
    pub fn binding(&self) -> &TargetBinding {
        &self.binding
    }
    /// This access can no longer authenticate to Orka and needs reconnecting.
    pub fn lost(&self) -> bool {
        self.bearer.lost()
    }
    /// Bridge minted this access's token and renews it for the confirmed account.
    pub fn renewable(&self) -> bool {
        self.bearer.renewable()
    }
    pub fn providers(&self) -> &[ProviderView] {
        &self.providers
    }
    async fn read(
        &self,
        resource: OrkaResource,
        after: Option<&str>,
    ) -> Result<Value, AccessOutcome> {
        self.read_via(&*self.orka, resource, after).await
    }
    /// One read with the current bearer. A rejected bearer is renewed once and the read
    /// retried once; a renewal that is impossible reports the rejection.
    pub(super) async fn read_via(
        &self,
        orka: &dyn OrkaRead,
        resource: OrkaResource,
        after: Option<&str>,
    ) -> Result<Value, AccessOutcome> {
        let check = resource.check();
        let bearer = self
            .bearer
            .token()
            .await
            .map_err(|_| check.fail(CheckResult::AuthenticationRejected))?;
        match self.read_as(orka, resource, after, &bearer).await {
            // Only Orka's own answer rejects the bearer; a refused port-forward is the sign-in's.
            Err(AccessOutcome::Failed(failure))
                if failure.check == check
                    && failure.result == CheckResult::AuthenticationRejected =>
            {
                match self.bearer.rejected(&bearer).await {
                    Ok(renewed) => self.read_as(orka, resource, after, &renewed).await,
                    Err(_) => Err(AccessOutcome::Failed(failure)),
                }
            }
            other => other,
        }
    }
    async fn read_as(
        &self,
        orka: &dyn OrkaRead,
        resource: OrkaResource,
        after: Option<&str>,
        bearer: &str,
    ) -> Result<Value, AccessOutcome> {
        read_with(
            orka,
            &self.creator,
            &self.binding,
            resource,
            after,
            bearer,
            &self.creator_bearer,
        )
        .await
    }
    async fn whoami(&self, orka: &dyn OrkaRead, bearer: &str) -> Result<Value, AccessOutcome> {
        self.read_as(orka, OrkaResource::WhoAmI, None, bearer).await
    }
    async fn identity(&self) -> Result<Identity, AccessOutcome> {
        self.identity_via(&*self.orka).await
    }
    async fn identity_via(&self, orka: &dyn OrkaRead) -> Result<Identity, AccessOutcome> {
        let identity = self.read_via(orka, OrkaResource::WhoAmI, None).await?;
        Ok(self.identity_of(&identity)?.0)
    }
    fn identity_of(&self, identity: &Value) -> Result<(Identity, Option<Subject>), AccessOutcome> {
        identity_in(identity, &self.binding.watched_namespace)
    }
    pub(super) async fn configured_providers(
        &self,
        orka: &dyn OrkaRead,
    ) -> Result<Vec<(String, bool)>, AccessOutcome> {
        let mut providers = Vec::new();
        let mut after = None;
        let mut seen = HashSet::new();
        loop {
            let page = self
                .read_via(orka, OrkaResource::Providers, after.as_deref())
                .await?;
            for item in items(&page)? {
                let name = item["metadata"]["name"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .ok_or(AccessOutcome::Unavailable)?;
                if item["metadata"]["namespace"] != self.binding.watched_namespace {
                    return Err(AccessOutcome::Unavailable);
                }
                let ready = match item.get("status") {
                    None => false,
                    Some(Value::Object(status)) => match status.get("ready") {
                        None => false,
                        Some(Value::Bool(ready)) => *ready,
                        _ => return Err(AccessOutcome::Unavailable),
                    },
                    _ => return Err(AccessOutcome::Unavailable),
                };
                providers.push((name.to_owned(), ready));
            }
            after = continuation(&page)?;
            if after.is_none() {
                break;
            }
            if !seen.insert(after.clone()) {
                return Err(AccessOutcome::Unavailable);
            }
        }
        Ok(providers)
    }
    pub(super) async fn recheck_reads(
        &self,
        stream: &OrkaStreamTransport,
    ) -> Result<(), ClusterError> {
        if self.identity_via(stream).await.map_err(cluster_error)? == Identity::NeedAccount {
            return Err(ClusterError::Denied);
        }
        let agents = self
            .read_via(stream, OrkaResource::Agents, None)
            .await
            .map_err(cluster_error)?;
        items(&agents).map_err(cluster_error)?;
        Ok(())
    }
    /// A freshly read kubeconfig, using this access's Orka reader, for admission rechecks.
    pub fn fresh_client(&self, kubeconfig: Kubeconfig) -> ClusterClient {
        ClusterClient::new(kubeconfig, self.orka.clone())
    }
    pub(crate) async fn check_agent_patch(
        &self,
        current: &ClusterClient,
        stream: &OrkaStreamTransport,
        name: &str,
    ) -> Result<(), ClusterError> {
        // Orka's direct OIDC users are not Kubernetes TokenReview identities.
        // They retain the endpoint's authorization contract, never a creator SAR.
        match self.identity_via(stream).await.map_err(cluster_error)? {
            Identity::Oidc => return Ok(()),
            Identity::NeedAccount => return Err(ClusterError::Denied),
            Identity::TokenReview => (),
        }
        let check = async {
            let bearer = self
                .bearer
                .token()
                .await
                .map_err(|_| AccessOutcome::Denied)?;
            let config =
                selected_config(&current.kubeconfig, &self.binding.context_name, &bearer).await?;
            let client = Client::try_from(config).map_err(|_| AccessOutcome::Unavailable)?;
            let review = SelfSubjectAccessReview {
                spec: SelfSubjectAccessReviewSpec {
                    resource_attributes: Some(ResourceAttributes {
                        namespace: Some(self.binding.watched_namespace.clone()),
                        group: Some("core.orka.ai".into()),
                        resource: Some("agents".into()),
                        verb: Some("patch".into()),
                        name: Some(name.into()),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                ..Default::default()
            };
            let result = Api::<SelfSubjectAccessReview>::all(client)
                .create(&PostParams::default(), &review)
                .await
                .map_err(|e| kube_error(Check::Permissions, e))?;
            let status = result.status.ok_or(AccessOutcome::Unavailable)?;
            if status
                .evaluation_error
                .as_ref()
                .is_some_and(|s| !s.is_empty())
            {
                return Err(AccessOutcome::Unavailable);
            }
            if !status.allowed || status.denied == Some(true) {
                return Err(Check::Permissions.fail(CheckResult::PermissionDenied));
            }
            Ok(())
        };
        within(Instant::now() + REQUEST_TIMEOUT, Check::Permissions, check)
            .await
            .map_err(cluster_error)
    }
    pub async fn require_builtins(&self, required: &[String]) -> Result<(), ClusterError> {
        self.builtins(&*self.orka, required).await
    }
    async fn builtins(&self, orka: &dyn OrkaRead, required: &[String]) -> Result<(), ClusterError> {
        let mut missing: HashSet<&str> = required.iter().map(String::as_str).collect();
        let mut after = None;
        let mut seen = HashSet::new();
        while !missing.is_empty() {
            let page = self
                .read_via(orka, OrkaResource::Tools, after.as_deref())
                .await
                .map_err(cluster_error)?;
            for item in items(&page).map_err(cluster_error)? {
                if item["builtin"] == true {
                    let name = item["name"].as_str().ok_or(ClusterError::Unavailable)?;
                    missing.remove(name);
                }
            }
            if missing.is_empty() {
                return Ok(());
            }
            after = continuation(&page).map_err(cluster_error)?;
            if after.is_none() {
                return Err(ClusterError::MissingTool);
            }
            if !seen.insert(after.clone()) {
                return Err(ClusterError::Unavailable);
            }
        }
        Ok(())
    }
}
/// Who Orka says a bearer is: the account's username and UID from TokenReview.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Subject {
    username: String,
    uid: String,
}
fn identity_in(
    identity: &Value,
    namespace: &str,
) -> Result<(Identity, Option<Subject>), AccessOutcome> {
    let rejected = Check::OrkaIdentity.fail(CheckResult::IdentityRejected);
    match identity["authenticated"].as_bool() {
        Some(true) => {}
        Some(false) => return Err(rejected),
        None => return Err(Check::OrkaIdentity.unknown()),
    }
    // A malformed reply is unknown, never an identity rejection.
    let (Some(ns), Some(auth)) = (
        identity["namespace"].as_str(),
        identity["authType"].as_str(),
    ) else {
        return Err(Check::OrkaIdentity.unknown());
    };
    let kind = match auth {
        "tokenReview" if ns.is_empty() => Identity::NeedAccount,
        "tokenReview" if ns == namespace => Identity::TokenReview,
        "oidc" if ns == namespace => Identity::Oidc,
        _ => return Err(rejected),
    };
    let text = |key: &str| {
        identity[key]
            .as_str()
            .filter(|s| !s.is_empty() && s.len() <= 1024)
            .map(str::to_owned)
    };
    let subject = text("username")
        .zip(text("uid"))
        .map(|(username, uid)| Subject { username, uid });
    Ok((kind, subject))
}
/// One bounded GET whose reply may not echo either credential.
async fn read_with(
    orka: &dyn OrkaRead,
    creator: &Client,
    binding: &TargetBinding,
    resource: OrkaResource,
    after: Option<&str>,
    bearer: &str,
    creator_bearer: &str,
) -> Result<Value, AccessOutcome> {
    let check = resource.check();
    let value = tokio::time::timeout(
        REQUEST_TIMEOUT,
        orka.get(
            creator,
            binding,
            bearer,
            resource,
            &binding.watched_namespace,
            after,
        ),
    )
    .await
    .map_err(|_| check.current().unknown())?
    .map_err(|e| e.at(check))?;
    if serde_json::to_vec(&value).map_or(true, |v| v.len() > 1024 * 1024)
        || echoed(&value, bearer)
        || echoed(&value, creator_bearer)
    {
        return Err(check.unknown());
    }
    Ok(value)
}
/// Bridge asks for ten minutes; the server's own expiry is what counts.
const TOKEN_SECONDS: i64 = 600;
async fn issue_token(
    creator: &Client,
    namespace: &str,
    account: &str,
) -> Result<Minted, AccessOutcome> {
    let request = TokenRequest {
        spec: TokenRequestSpec {
            audiences: vec![],
            expiration_seconds: Some(TOKEN_SECONDS),
            ..Default::default()
        },
        ..Default::default()
    };
    let status = Api::<ServiceAccount>::namespaced(creator.clone(), namespace)
        .create_token_request(account, &PostParams::default(), &request)
        .await
        .map_err(|e| kube_error(Check::ServiceAccount, e))?
        .status
        .ok_or(Check::ServiceAccount.unknown())?;
    if status.token.is_empty()
        || status.expiration_timestamp.0 <= k8s_openapi::jiff::Timestamp::now()
    {
        return Err(Check::ServiceAccount.unknown());
    }
    Ok(Minted {
        token: status.token,
        expires: instant(&status.expiration_timestamp),
    })
}
/// A server timestamp on this process's monotonic clock, so renewal follows paused test time.
fn instant(time: &k8s_openapi::apimachinery::pkg::apis::meta::v1::Time) -> Instant {
    let left = time.0.as_millisecond() - k8s_openapi::jiff::Timestamp::now().as_millisecond();
    Instant::now() + Duration::from_millis(u64::try_from(left).unwrap_or(0))
}
/// Renews a minted ServiceAccount token exactly as Use first confirmed it: the same sign-in,
/// installation, namespace and account, and the same subject in Orka. Never another account.
struct AccountMint {
    client: ClusterClient,
    creator: Client,
    binding: TargetBinding,
    account: String,
    subject: Subject,
    creator_bearer: String,
}
impl AccountMint {
    async fn renew(&self) -> Result<Minted, AccessOutcome> {
        let deadline = Instant::now() + OPERATION_LIMIT;
        let namespace = &self.binding.watched_namespace;
        self.client
            .check_binding(&self.binding, &self.creator, deadline)
            .await?;
        let issue = issue_token(&self.creator, namespace, &self.account);
        let minted = within(deadline, Check::ServiceAccount, issue).await?;
        let whoami = read_with(
            &*self.client.orka,
            &self.creator,
            &self.binding,
            OrkaResource::WhoAmI,
            None,
            &minted.token,
            &self.creator_bearer,
        );
        let whoami = within(deadline, Check::OrkaIdentity, whoami).await?;
        match identity_in(&whoami, namespace)? {
            (Identity::TokenReview, Some(subject)) if subject == self.subject => {}
            _ => return Err(Check::OrkaIdentity.fail(CheckResult::IdentityRejected)),
        }
        let config = selected_config(
            &self.client.kubeconfig,
            &self.binding.context_name,
            &minted.token,
        )
        .await?;
        let client = Client::try_from(config).map_err(|_| Check::Configuration.unknown())?;
        within(deadline, Check::Permissions, permissions(client, namespace)).await?;
        Ok(minted)
    }
}
impl Mint for AccountMint {
    fn mint(&self) -> MintFuture<'_> {
        Box::pin(async move {
            self.renew().await.map_err(|outcome| match outcome {
                // Timeouts and dropped connections may pass; the current token is kept.
                AccessOutcome::Unavailable
                | AccessOutcome::Failed(ClusterFailure {
                    result: CheckResult::Unknown | CheckResult::ConnectionFailed,
                    ..
                }) => Refused::Unreachable,
                _ => Refused::Rejected,
            })
        })
    }
}
pub(super) fn deployment_watches(deployment: &Deployment, namespace: &str) -> bool {
    let Some(template) = deployment.spec.as_ref().map(|s| &s.template) else {
        return false;
    };
    let Some(spec) = template.spec.as_ref() else {
        return false;
    };
    let labels = template
        .metadata
        .as_ref()
        .and_then(|m| m.labels.as_ref())
        .cloned()
        .unwrap_or_default();
    let mut found = false;
    for container in &spec.containers {
        if is_orka(container, &labels) {
            found = true;
            if watched_namespace(container.args.as_deref().unwrap_or_default()).ok()
                != Some(namespace)
            {
                return false;
            }
        }
    }
    found
}
fn is_orka(
    container: &k8s_openapi::api::core::v1::Container,
    labels: &std::collections::BTreeMap<String, String>,
) -> bool {
    let args = container.args.as_deref().unwrap_or_default();
    // A generic API port is not Orka evidence; unrelated controllers use it too.
    let serves_api = args
        .iter()
        .any(|a| a.starts_with("--api-port=") || a == "--api-port")
        || container
            .ports
            .as_ref()
            .is_some_and(|ports| ports.iter().any(|p| p.name.as_deref() == Some("api")));
    let orka_label = serves_api
        && labels
            .get("app.kubernetes.io/name")
            .is_some_and(|n| n == "orka");
    let orka_image = container.image.as_deref().is_some_and(|image| {
        image
            .split('@')
            .next()
            .and_then(|i| i.rsplit('/').next())
            .and_then(|i| i.split(':').next())
            == Some("orka")
    });
    orka_label || orka_image
}
fn watched_namespace(args: &[String]) -> Result<&str, AccessOutcome> {
    let mut watched = args.iter().enumerate().filter_map(|(i, a)| {
        a.strip_prefix("--watch-namespace=").or_else(|| {
            (a == "--watch-namespace")
                .then(|| args.get(i + 1).map(String::as_str))
                .flatten()
        })
    });
    let incompatible = Check::Compatibility.fail(CheckResult::Incompatible);
    let namespace = watched
        .next()
        .filter(|s| dns_label(s))
        .ok_or(incompatible)?;
    if watched.next().is_some() {
        return Err(incompatible);
    }
    Ok(namespace)
}
async fn installations(
    client: Client,
    context_id: ContextId,
    context_name: &str,
    server: &str,
    cluster_identity: &str,
    deadline: Instant,
) -> Result<Vec<TargetBinding>, AccessOutcome> {
    let services = within(deadline, Check::ClusterApi, async {
        Api::<Service>::all(client.clone())
            .list(&ListParams::default())
            .await
            .map_err(|e| kube_error(Check::ClusterApi, e))
    })
    .await?;
    let deployments = within(deadline, Check::ClusterApi, async {
        Api::<Deployment>::all(client.clone())
            .list(&ListParams::default())
            .await
            .map_err(|e| kube_error(Check::ClusterApi, e))
    })
    .await?;
    let mut bindings = Vec::new();
    for service in services {
        let Some(spec) = &service.spec else {
            continue;
        };
        let Some(selector) = spec.selector.as_ref().filter(|s| !s.is_empty()) else {
            continue;
        };
        let Some(port) = spec
            .ports
            .as_ref()
            .and_then(|p| p.iter().find(|p| p.name.as_deref() == Some("api")))
        else {
            continue;
        };
        for deployment in &deployments.items {
            if deployment.namespace() != service.namespace() {
                continue;
            }
            let Some(template) = deployment.spec.as_ref().map(|s| &s.template) else {
                continue;
            };
            let Some(labels) = template.metadata.as_ref().and_then(|m| m.labels.as_ref()) else {
                continue;
            };
            if !selector.iter().all(|(k, v)| labels.get(k) == Some(v)) {
                continue;
            }
            let Some(containers) = template.spec.as_ref().map(|s| &s.containers) else {
                continue;
            };
            for container in containers {
                let args = container.args.as_deref().unwrap_or_default();
                if !is_orka(container, labels) {
                    continue;
                }
                let namespace = watched_namespace(args)?;
                let target_port = match &port.target_port {
                    Some(IntOrString::Int(n)) => *n,
                    Some(IntOrString::String(name)) => container
                        .ports
                        .as_ref()
                        .and_then(|p| p.iter().find(|p| p.name.as_ref() == Some(name)))
                        .map(|p| p.container_port)
                        .ok_or(AccessOutcome::Unavailable)?,
                    None => port.port,
                };
                bindings.push(TargetBinding {
                    cluster_identity: cluster_identity.into(),
                    context_id,
                    context_name: context_name.into(),
                    server_address: server.into(),
                    service_name: service.name_any(),
                    service_namespace: service.namespace().ok_or(AccessOutcome::Unavailable)?,
                    service_uid: service.uid().ok_or(AccessOutcome::Unavailable)?,
                    deployment_name: deployment.name_any(),
                    deployment_namespace: deployment
                        .namespace()
                        .ok_or(AccessOutcome::Unavailable)?,
                    deployment_uid: deployment.uid().ok_or(AccessOutcome::Unavailable)?,
                    watched_namespace: namespace.into(),
                    image: container
                        .image
                        .clone()
                        .filter(|s| !s.is_empty())
                        .ok_or(AccessOutcome::Unavailable)?,
                    api_port: u16::try_from(target_port)
                        .ok()
                        .filter(|p| *p > 0)
                        .ok_or(AccessOutcome::Unavailable)?,
                });
            }
        }
    }
    if !bindings.is_empty() {
        for resource in ["agents", "tasks", "providers", "tools"] {
            let crd = within(deadline, Check::Compatibility, async {
                Api::<CustomResourceDefinition>::all(client.clone())
                    .get(&format!("{resource}.core.orka.ai"))
                    .await
                    .map_err(|e| kube_error(Check::Compatibility, e))
            })
            .await?;
            if crd.spec.group != "core.orka.ai"
                || crd.spec.scope != "Namespaced"
                || crd.spec.names.plural != resource
                || !crd.spec.versions.iter().any(|v| {
                    v.name == "v1alpha1"
                        && v.served
                        && v.schema
                            .as_ref()
                            .and_then(|s| s.open_api_v3_schema.as_ref())
                            .and_then(|s| serde_json::to_value(s).ok())
                            .is_some_and(|s| super::preflight::supports_schema(resource, &s))
                })
            {
                return Err(Check::Compatibility.fail(CheckResult::Incompatible));
            }
        }
    }
    Ok(bindings)
}
const ORKA_PERMISSIONS: [(&str, &str); 10] = [
    ("agents", "create"),
    ("agents", "get"),
    ("agents", "list"),
    ("tasks", "create"),
    ("tasks", "get"),
    ("tasks", "list"),
    ("providers", "get"),
    ("providers", "list"),
    ("tools", "get"),
    ("tools", "list"),
];
async fn permissions(client: Client, namespace: &str) -> Result<(), AccessOutcome> {
    for (resource, verb) in ORKA_PERMISSIONS {
        let review = SelfSubjectAccessReview {
            spec: SelfSubjectAccessReviewSpec {
                resource_attributes: Some(ResourceAttributes {
                    namespace: Some(namespace.into()),
                    group: Some("core.orka.ai".into()),
                    resource: Some(resource.into()),
                    verb: Some(verb.into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            ..Default::default()
        };
        let result = Api::<SelfSubjectAccessReview>::all(client.clone())
            .create(&PostParams::default(), &review)
            .await
            .map_err(|e| kube_error(Check::Permissions, e))?;
        let status = result.status.ok_or(AccessOutcome::Unavailable)?;
        if status
            .evaluation_error
            .as_ref()
            .is_some_and(|s| !s.is_empty())
        {
            return Err(AccessOutcome::Unavailable);
        }
        if !status.allowed || status.denied == Some(true) {
            return Err(Check::Permissions.fail(CheckResult::PermissionDenied));
        }
    }
    Ok(())
}
/// One context's whole kubeconfig entry, compared in memory only and never exported.
fn context_entry(config: &Kubeconfig, name: &str) -> Option<Value> {
    fn one<T>(mut items: impl Iterator<Item = T>) -> Option<T> {
        let item = items.next()?;
        items.next().is_none().then_some(item)
    }
    let context = one(config.contexts.iter().filter(|c| c.name == name))?
        .context
        .as_ref()?;
    let cluster = one(config.clusters.iter().filter(|c| c.name == context.cluster))?;
    let user = match &context.user {
        Some(user) => Some(one(config.auth_infos.iter().filter(|a| &a.name == user))?),
        None => None,
    };
    serde_json::to_value((context, cluster, user)).ok()
}
fn same_entry(old: &Kubeconfig, new: &Kubeconfig, name: &str) -> bool {
    matches!((context_entry(old, name), context_entry(new, name)), (Some(a), Some(b)) if a == b)
}
fn dns_label(s: &str) -> bool {
    s.len() <= 63 && dns_part(s)
}
fn dns_part(s: &str) -> bool {
    !s.is_empty()
        && s.as_bytes().first().is_some_and(u8::is_ascii_alphanumeric)
        && s.as_bytes().last().is_some_and(u8::is_ascii_alphanumeric)
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}
fn dns_subdomain(s: &str) -> bool {
    s.len() <= 253 && s.split('.').all(dns_part)
}
pub(super) fn items(page: &Value) -> Result<&Vec<Value>, AccessOutcome> {
    page["items"].as_array().ok_or(AccessOutcome::Unavailable)
}
pub(super) fn continuation(page: &Value) -> Result<Option<String>, AccessOutcome> {
    let metadata = page
        .get("metadata")
        .and_then(Value::as_object)
        .ok_or(AccessOutcome::Unavailable)?;
    match metadata.get("continue") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok((!s.is_empty()).then(|| s.clone())),
        _ => Err(AccessOutcome::Unavailable),
    }
}
pub(super) fn echoed(value: &Value, bearer: &str) -> bool {
    if bearer.is_empty() {
        return false;
    }
    match value {
        Value::String(s) => s.contains(bearer),
        Value::Array(a) => a.iter().any(|v| echoed(v, bearer)),
        Value::Object(o) => o
            .iter()
            .any(|(k, v)| k.contains(bearer) || echoed(v, bearer)),
        _ => false,
    }
}
/// Bounds one check by the shared operation deadline; a timeout is unknown at that check.
async fn within<T>(
    deadline: Instant,
    check: Check,
    work: impl std::future::Future<Output = Result<T, AccessOutcome>>,
) -> Result<T, AccessOutcome> {
    check.begin();
    timeout_at(deadline, work)
        .await
        .unwrap_or_else(|_| Err(check.current().unknown()))
        .map_err(|e| e.at(check))
}
fn kube_error(check: Check, error: kube::Error) -> AccessOutcome {
    check.fail(match &error {
        kube::Error::Api(e) if e.code == 401 => CheckResult::AuthenticationRejected,
        kube::Error::Api(e) if e.code == 403 => CheckResult::PermissionDenied,
        kube::Error::Api(e) if e.code == 404 && check == Check::Compatibility => {
            CheckResult::Incompatible
        }
        _ if connection_failed(&error) => CheckResult::ConnectionFailed,
        _ => CheckResult::Unknown,
    })
}
/// Only an OS-reported connection error counts; DNS, TLS and timeouts stay unknown.
fn connection_failed(error: &(dyn std::error::Error + 'static)) -> bool {
    use std::io::ErrorKind::*;
    let mut source = Some(error);
    while let Some(error) = source {
        if error.downcast_ref::<std::io::Error>().is_some_and(|e| {
            matches!(
                e.kind(),
                ConnectionRefused
                    | ConnectionReset
                    | ConnectionAborted
                    | HostUnreachable
                    | NetworkUnreachable
            )
        }) {
            return true;
        }
        source = error.source();
    }
    false
}
fn cluster_error(error: AccessOutcome) -> ClusterError {
    match error {
        AccessOutcome::Denied => ClusterError::Denied,
        AccessOutcome::UnsupportedCredential => ClusterError::UnsupportedCredential,
        AccessOutcome::Failed(failure) => failure.legacy(),
        _ => ClusterError::Unavailable,
    }
}

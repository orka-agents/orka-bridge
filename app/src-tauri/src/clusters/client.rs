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
use tokio::time::Instant;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
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
    targets: HashMap<TargetId, TargetBinding>,
    orka: Arc<dyn OrkaRead>,
    exec: ExecOwner,
}
/// Native access only: no Serialize/Debug implementation can export credentials.
pub struct RemoteAccess {
    pub(crate) creator: Client,
    pub(crate) bearer: String,
    // Orka has seen this credential even after the confirmed ServiceAccount fallback.
    pub(super) creator_bearer: String,
    pub expiry: Option<k8s_openapi::apimachinery::pkg::apis::meta::v1::Time>,
    pub(super) binding: TargetBinding,
    orka: Arc<dyn OrkaRead>,
    providers: Vec<ProviderView>,
}
impl ClusterClient {
    pub fn read(orka: Arc<dyn OrkaRead>) -> Result<Self, ClusterError> {
        Ok(Self::new(
            Kubeconfig::read().map_err(|_| ClusterError::Unavailable)?,
            orka,
        ))
    }
    pub fn new(kubeconfig: Kubeconfig, orka: Arc<dyn OrkaRead>) -> Self {
        let contexts = kubeconfig
            .contexts
            .iter()
            .map(|c| ContextView {
                id: ContextId(Uuid::new_v4()),
                name: c.name.clone(),
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
    pub(super) fn with_exec_owner(mut self, exec: ExecOwner) -> Self {
        self.exec = exec;
        self
    }
    pub fn contexts(&self) -> Result<Vec<ContextView>, ClusterError> {
        Ok(self.contexts.clone())
    }
    pub async fn resolve_creator(
        &self,
        context_id: ContextId,
        deadline: Instant,
    ) -> Result<ResolvedCreator, ClusterError> {
        let name = &self
            .contexts
            .iter()
            .find(|c| c.id == context_id)
            .ok_or(ClusterError::Unavailable)?
            .name;
        credentials::resolve(&self.kubeconfig, name, deadline, &self.exec)
            .await
            .map_err(cluster_error)
    }
    pub async fn discover(&mut self, id: ContextId) -> Result<Vec<TargetView>, ClusterError> {
        let config = self
            .resolve_creator(id, Instant::now() + REQUEST_TIMEOUT)
            .await?
            .config;
        let client = Client::try_from(config.clone()).map_err(|_| ClusterError::Unavailable)?;
        let name = &self
            .contexts
            .iter()
            .find(|c| c.id == id)
            .ok_or(ClusterError::Unavailable)?
            .name;
        let bindings = installations(client, id, name, &config.cluster_url.to_string())
            .await
            .map_err(cluster_error)?;
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
        let config = selected_config(&self.kubeconfig, &binding.context_name, "binding-only")
            .await
            .map_err(cluster_error)?;
        if config.cluster_url.to_string() != binding.server_address {
            return Err(ClusterError::Unavailable);
        }
        let bindings = installations(
            creator.clone(),
            binding.context_id,
            &binding.context_name,
            &config.cluster_url.to_string(),
        )
        .await
        .map_err(cluster_error)?;
        if bindings.contains(binding) {
            Ok(())
        } else {
            Err(ClusterError::Unavailable)
        }
    }
    /// Calling connect is the native Use confirmation; discovery never requests a token.
    pub async fn connect(
        &self,
        id: TargetId,
        sa_name: Option<String>,
    ) -> Result<RemoteAccess, AccessOutcome> {
        if sa_name.as_ref().is_some_and(|s| !dns_subdomain(s)) {
            return Err(AccessOutcome::Unavailable);
        }
        let binding = self
            .targets
            .get(&id)
            .ok_or(AccessOutcome::Unavailable)?
            .clone();
        let resolved = self
            .resolve_creator(binding.context_id, Instant::now() + REQUEST_TIMEOUT)
            .await
            .map_err(access_error)?;
        let (bearer, expiry) = match resolved.credential {
            Credential::Bearer {
                value,
                known_expiry,
            } => (value, known_expiry),
            Credential::CertificateOnly => (String::new(), None),
        };
        let creator = Client::try_from(resolved.config).map_err(|_| AccessOutcome::Unavailable)?;
        let mut access = RemoteAccess {
            creator,
            creator_bearer: bearer.clone(),
            bearer,
            expiry,
            binding,
            orka: self.orka.clone(),
            providers: Vec::new(),
        };
        self.revalidate(&access).await.map_err(access_error)?;
        let mut identity = if access.bearer.is_empty() {
            Identity::NeedAccount
        } else {
            access.identity().await?
        };
        if identity == Identity::NeedAccount {
            let name = sa_name.ok_or(AccessOutcome::NeedServiceAccount)?;
            let request = TokenRequest {
                spec: TokenRequestSpec {
                    audiences: vec![],
                    expiration_seconds: Some(600),
                    ..Default::default()
                },
                ..Default::default()
            };
            let response = Api::<ServiceAccount>::namespaced(
                access.creator.clone(),
                &access.binding.watched_namespace,
            )
            .create_token_request(&name, &PostParams::default(), &request)
            .await
            .map_err(kube_error)?;
            let status = response.status.ok_or(AccessOutcome::Unavailable)?;
            if status.token.is_empty()
                || status.expiration_timestamp.0 <= k8s_openapi::jiff::Timestamp::now()
            {
                return Err(AccessOutcome::Unavailable);
            }
            access.bearer = status.token;
            access.expiry = Some(status.expiration_timestamp);
            identity = access.identity().await?;
            if identity == Identity::NeedAccount {
                return Err(AccessOutcome::Denied);
            }
        }
        // Even a certificate creator must check permissions as the frozen Orka bearer alone.
        if identity == Identity::TokenReview {
            let token_config = selected_config(
                &self.kubeconfig,
                &access.binding.context_name,
                &access.bearer,
            )
            .await?;
            let token_client =
                Client::try_from(token_config).map_err(|_| AccessOutcome::Unavailable)?;
            permissions(token_client, &access.binding.watched_namespace).await?;
        }
        access.read(OrkaResource::Agents, None).await?;
        for (name, ready) in access.configured_providers(&*access.orka).await? {
            access.providers.push(ProviderView {
                id: ProviderId(Uuid::new_v4()),
                name,
                ready,
            });
        }
        if access.providers.is_empty() {
            return Err(AccessOutcome::Unavailable);
        }
        Ok(access)
    }
}
impl RemoteAccess {
    pub fn binding(&self) -> &TargetBinding {
        &self.binding
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
    async fn read_via(
        &self,
        orka: &dyn OrkaRead,
        resource: OrkaResource,
        after: Option<&str>,
    ) -> Result<Value, AccessOutcome> {
        let value = tokio::time::timeout(
            REQUEST_TIMEOUT,
            orka.get(
                &self.creator,
                &self.binding,
                &self.bearer,
                resource,
                &self.binding.watched_namespace,
                after,
            ),
        )
        .await
        .map_err(|_| AccessOutcome::Unavailable)??;
        if serde_json::to_vec(&value)
            .map_err(|_| AccessOutcome::Unavailable)?
            .len()
            > 1024 * 1024
            || echoed(&value, &self.bearer)
            || echoed(&value, &self.creator_bearer)
        {
            return Err(AccessOutcome::Unavailable);
        }
        Ok(value)
    }
    async fn identity(&self) -> Result<Identity, AccessOutcome> {
        let identity = self.read(OrkaResource::WhoAmI, None).await?;
        if identity["authenticated"] != true {
            return Err(AccessOutcome::Denied);
        }
        let ns = identity["namespace"]
            .as_str()
            .ok_or(AccessOutcome::Unavailable)?;
        match identity["authType"].as_str() {
            Some("tokenReview") if ns.is_empty() => Ok(Identity::NeedAccount),
            Some("tokenReview") if ns == self.binding.watched_namespace => {
                Ok(Identity::TokenReview)
            }
            Some("oidc") if ns == self.binding.watched_namespace => Ok(Identity::Oidc),
            _ => Err(AccessOutcome::Denied),
        }
    }
    async fn configured_providers(
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
    /// Send's built-in and Provider rechecks over the attempt's own stream, so a
    /// Stop that drops a check leaves no forward for anyone but its owner to close.
    /// The Provider is rechecked by name/namespace, never readiness: tests establish availability.
    pub async fn recheck(
        &self,
        stream: &OrkaStreamTransport,
        id: ProviderId,
        tools: &[String],
    ) -> Result<(), ClusterError> {
        let selected = self.providers.iter().find(|p| p.id == id);
        let selected = selected.ok_or(ClusterError::Unavailable)?;
        self.builtins(stream, tools).await?;
        let current = self
            .configured_providers(stream)
            .await
            .map_err(cluster_error)?;
        if current.iter().any(|(name, _)| *name == selected.name) {
            Ok(())
        } else {
            Err(ClusterError::Unavailable)
        }
    }
    /// A freshly read kubeconfig, using this access's Orka reader, for admission rechecks.
    pub fn fresh_client(&self, kubeconfig: Kubeconfig) -> ClusterClient {
        ClusterClient::new(kubeconfig, self.orka.clone())
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
async fn installations(
    client: Client,
    context_id: ContextId,
    context_name: &str,
    server: &str,
) -> Result<Vec<TargetBinding>, AccessOutcome> {
    let services = Api::<Service>::all(client.clone())
        .list(&ListParams::default())
        .await
        .map_err(kube_error)?;
    let deployments = Api::<Deployment>::all(client.clone())
        .list(&ListParams::default())
        .await
        .map_err(kube_error)?;
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
                // A generic API port is not Orka evidence; unrelated controllers use it too.
                let serves_api = args
                    .iter()
                    .any(|a| a.starts_with("--api-port=") || a == "--api-port")
                    || container.ports.as_ref().is_some_and(|ports| {
                        ports.iter().any(|p| p.name.as_deref() == Some("api"))
                    });
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
                if !orka_label && !orka_image {
                    continue;
                }
                let mut watched = args.iter().enumerate().filter_map(|(i, a)| {
                    a.strip_prefix("--watch-namespace=").or_else(|| {
                        (a == "--watch-namespace")
                            .then(|| args.get(i + 1).map(String::as_str))
                            .flatten()
                    })
                });
                let namespace = watched
                    .next()
                    .filter(|s| dns_label(s))
                    .ok_or(AccessOutcome::Unavailable)?;
                if watched.next().is_some() {
                    return Err(AccessOutcome::Unavailable);
                }
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
            let crd = Api::<CustomResourceDefinition>::all(client.clone())
                .get(&format!("{resource}.core.orka.ai"))
                .await
                .map_err(kube_error)?;
            if crd.spec.group != "core.orka.ai"
                || crd.spec.scope != "Namespaced"
                || crd.spec.names.plural != resource
                || !crd
                    .spec
                    .versions
                    .iter()
                    .any(|v| v.name == "v1alpha1" && v.served)
            {
                return Err(AccessOutcome::Unavailable);
            }
        }
    }
    Ok(bindings)
}
async fn permissions(client: Client, namespace: &str) -> Result<(), AccessOutcome> {
    for (resource, verbs) in [
        ("agents", &["create", "get", "list"][..]),
        ("tasks", &["create", "get", "list"][..]),
        ("providers", &["get", "list"][..]),
        ("tools", &["get", "list"][..]),
    ] {
        for verb in verbs {
            let review = SelfSubjectAccessReview {
                spec: SelfSubjectAccessReviewSpec {
                    resource_attributes: Some(ResourceAttributes {
                        namespace: Some(namespace.into()),
                        group: Some("core.orka.ai".into()),
                        resource: Some(resource.into()),
                        verb: Some((*verb).into()),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                ..Default::default()
            };
            let result = Api::<SelfSubjectAccessReview>::all(client.clone())
                .create(&PostParams::default(), &review)
                .await
                .map_err(kube_error)?;
            let status = result.status.ok_or(AccessOutcome::Unavailable)?;
            if status
                .evaluation_error
                .as_ref()
                .is_some_and(|s| !s.is_empty())
            {
                return Err(AccessOutcome::Unavailable);
            }
            if !status.allowed || status.denied == Some(true) {
                return Err(AccessOutcome::Denied);
            }
        }
    }
    Ok(())
}
fn dns_label(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 63
        && s.as_bytes().first().is_some_and(u8::is_ascii_alphanumeric)
        && s.as_bytes().last().is_some_and(u8::is_ascii_alphanumeric)
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}
fn dns_subdomain(s: &str) -> bool {
    s.len() <= 253 && s.split('.').all(dns_label)
}
fn items(page: &Value) -> Result<&Vec<Value>, AccessOutcome> {
    page["items"].as_array().ok_or(AccessOutcome::Unavailable)
}
fn continuation(page: &Value) -> Result<Option<String>, AccessOutcome> {
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
fn kube_error(error: kube::Error) -> AccessOutcome {
    match error {
        kube::Error::Api(e) if e.code == 401 || e.code == 403 => AccessOutcome::Denied,
        _ => AccessOutcome::Unavailable,
    }
}
fn cluster_error(error: AccessOutcome) -> ClusterError {
    match error {
        AccessOutcome::Denied => ClusterError::Denied,
        AccessOutcome::UnsupportedCredential => ClusterError::UnsupportedCredential,
        _ => ClusterError::Unavailable,
    }
}
fn access_error(error: ClusterError) -> AccessOutcome {
    match error {
        ClusterError::Denied => AccessOutcome::Denied,
        ClusterError::UnsupportedCredential => AccessOutcome::UnsupportedCredential,
        _ => AccessOutcome::Unavailable,
    }
}

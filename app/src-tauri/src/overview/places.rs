//! Session-only inventories of explicitly confirmed places. No credentials in replies.
#[cfg(test)]
#[path = "places_tests.rs"]
mod tests;
use crate::{
    clusters::{commands::ClusterSession, ClusterError, RemoteAccess, TargetBinding},
    send_summary::AttemptSummary,
};
use serde::Serialize;
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashSet},
    sync::{Arc, Mutex},
};

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteAgent {
    pub name: String,
    pub namespace: String,
    pub uid: String,
    pub agent_type: Option<String>,
    pub model: Option<String>,
    pub tools: Option<Vec<String>>,
    pub record: Option<AttemptSummary>,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Place {
    pub key: String,
    pub context: String,
    pub service: String,
    pub namespace: String,
    pub stale: bool,
    pub observed_at: Option<u64>,
    pub agents: Vec<RemoteAgent>,
    #[serde(skip)]
    pub(crate) binding: TargetBinding,
}
impl Place {
    pub(crate) fn new(binding: TargetBinding) -> Self {
        // Opaque selection IDs and mutable image settings are not place identity.
        let key = serde_json::to_string(&(
            &binding.context_name,
            &binding.service_namespace,
            &binding.service_uid,
            &binding.deployment_namespace,
            &binding.deployment_uid,
            &binding.watched_namespace,
        ))
        .unwrap();
        Self {
            key,
            context: binding.context_name.clone(),
            service: format!("{}/{}", binding.service_namespace, binding.service_name),
            namespace: binding.watched_namespace.clone(),
            stale: true,
            observed_at: None,
            agents: Vec::new(),
            binding,
        }
    }
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Places {
    pub places: Vec<Place>,
    pub history_state: &'static str,
    pub unobserved: Vec<AttemptSummary>,
}
fn text(value: &Value) -> Option<String> {
    value
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 4096)
        .map(str::to_owned)
}
pub(crate) fn agent(value: &Value, namespace: &str) -> Result<RemoteAgent, ClusterError> {
    let bad = ClusterError::Unavailable;
    if value["metadata"]["namespace"].as_str() != Some(namespace) {
        return Err(bad);
    }
    Ok(RemoteAgent {
        name: text(&value["metadata"]["name"]).ok_or(bad)?,
        namespace: namespace.into(),
        uid: text(&value["metadata"]["uid"]).ok_or(bad)?,
        agent_type: text(&value["spec"]["type"]),
        model: text(&value["spec"]["model"]["name"]),
        tools: value["spec"]["tools"]
            .as_array()
            .and_then(|items| items.iter().map(|t| text(&t["name"])).collect()),
        record: None,
    })
}
struct Entry {
    view: Place,
    access: Option<Arc<RemoteAccess>>,
    generation: u64,
}
#[derive(Default)]
pub(crate) struct Inventory {
    entries: Mutex<BTreeMap<String, Entry>>,
    refreshing: tokio::sync::Mutex<()>,
}
impl Inventory {
    pub(crate) fn remember(&self, access: Arc<RemoteAccess>) {
        let view = Place::new(access.binding().clone());
        let mut entries = self.entries.lock().unwrap();
        let entry = entries.entry(view.key.clone()).or_insert(Entry {
            view,
            access: None,
            generation: 0,
        });
        entry.generation += 1;
        entry.view.stale = true;
        entry.access = Some(access);
    }
    pub(crate) fn invalidate(&self, binding: Option<&TargetBinding>) {
        let key = binding.map(|b| Place::new(b.clone()).key);
        for (id, entry) in self.entries.lock().unwrap().iter_mut() {
            if key.as_ref().is_none_or(|key| key == id) {
                entry.generation += 1;
                entry.access = None;
                entry.view.stale = true;
            }
        }
    }
    fn snapshot(&self) -> Vec<Place> {
        self.entries
            .lock()
            .unwrap()
            .values()
            .map(|e| e.view.clone())
            .collect()
    }
    pub(crate) async fn refresh(&self) -> Vec<Place> {
        let Ok(_refresh) = self.refreshing.try_lock() else {
            // Concurrent IPC requests share the in-progress read, not another round.
            let _refresh = self.refreshing.lock().await;
            return self.snapshot();
        };
        let reads: Vec<_> = self
            .entries
            .lock()
            .unwrap()
            .iter_mut()
            .filter_map(|(key, e)| {
                e.view.stale = true;
                e.access.clone().map(|a| (key.clone(), e.generation, a))
            })
            .collect();
        for (key, generation, access) in reads {
            let result = access.inventory().await;
            if let Some(entry) = self.entries.lock().unwrap().get_mut(&key) {
                // Reload/re-Use while a read was in flight revokes that response.
                if entry.generation == generation {
                    if let Ok(agents) = result {
                        entry.view.agents = agents;
                        entry.view.observed_at = Some(crate::overview::now());
                        entry.view.stale = false;
                    }
                }
            }
        }
        self.snapshot()
    }
}
pub(crate) fn project(mut places: Vec<Place>, history: Option<Vec<AttemptSummary>>) -> Places {
    let Some(history) = history else {
        return Places {
            places,
            history_state: "unavailable",
            unobserved: Vec::new(),
        };
    };
    let mut matched = HashSet::new();
    for place in &mut places {
        let b = &place.binding;
        for agent in &mut place.agents {
            // History is newest first. A name is never an association key.
            if let Some(record) = history.iter().find(|r| {
                r.agent_uid.as_deref() == Some(&agent.uid)
                    && r.target_display == format!("{} · {}", b.context_name, b.watched_namespace)
                    && r.target_uids.service_namespace == b.service_namespace
                    && r.target_uids.service_uid == b.service_uid
                    && r.target_uids.deployment_namespace == b.deployment_namespace
                    && r.target_uids.deployment_uid == b.deployment_uid
                    && r.target_uids.watched_namespace == b.watched_namespace
                    && agent.namespace == b.watched_namespace
            }) {
                matched.insert(record.id.clone());
                agent.record = Some(record.clone());
            }
        }
    }
    Places {
        places,
        history_state: "complete",
        unobserved: history
            .into_iter()
            .filter(|r| !matched.contains(&r.id))
            .collect(),
    }
}
pub async fn list(session: &ClusterSession, history: Option<Vec<AttemptSummary>>) -> Places {
    project(session.overview().await, history)
}

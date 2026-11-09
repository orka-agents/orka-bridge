//! Native-owned source bindings. IDs, never renderer paths, select captured pairs.
use crate::{
    engine::run::ParsedPair,
    runtime::snapshot::{AgentDocument, Snapshot},
};
use bridge_runtime_protocol::{FileSlot, Uuid};
use std::{
    collections::HashMap,
    path::{Component, PathBuf},
    sync::Mutex,
};

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct PairId(Uuid);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PairError {
    Binding,
    Capture,
    Source,
}

pub struct CapturedPair {
    snapshot: Snapshot,
    pair: ParsedPair,
}
impl CapturedPair {
    /// Reuse an owner's held-directory capture without rereading its files.
    pub fn from_snapshot(snapshot: Snapshot) -> Result<Self, PairError> {
        let pair = ParsedPair::parse(
            snapshot.file(FileSlot::Agent),
            snapshot.file(FileSlot::Tests),
        )
        .map_err(|_| PairError::Source)?;
        Ok(Self { snapshot, pair })
    }
    pub fn snapshot(&self) -> &Snapshot {
        &self.snapshot
    }
    pub fn pair(&self) -> &ParsedPair {
        &self.pair
    }
    pub fn into_pair(self) -> ParsedPair {
        self.pair
    }
}
struct Binding {
    root: PathBuf,
    document: AgentDocument,
}
#[derive(Default)]
pub struct PairCatalog(Mutex<HashMap<PairId, Binding>>);
impl PairCatalog {
    /// Only the native document owner may supply this trusted root/document.
    pub fn bind_owned_agent(
        &self,
        root: PathBuf,
        document: AgentDocument,
    ) -> Result<PairId, PairError> {
        if !root.is_absolute()
            || [&document.agent, &document.tests].iter().any(|path| {
                path.as_os_str().is_empty()
                    || path
                        .components()
                        .any(|part| !matches!(part, Component::Normal(_)))
            })
        {
            return Err(PairError::Binding);
        }
        let id = PairId(Uuid::new_v4());
        self.0
            .lock()
            .unwrap()
            .insert(id, Binding { root, document });
        Ok(id)
    }
    pub fn capture(&self, id: PairId) -> Result<CapturedPair, PairError> {
        let bindings = self.0.lock().unwrap();
        let binding = bindings.get(&id).ok_or(PairError::Binding)?;
        let snapshot =
            Snapshot::capture(&binding.root, &binding.document).map_err(|_| PairError::Capture)?;
        CapturedPair::from_snapshot(snapshot)
    }
}

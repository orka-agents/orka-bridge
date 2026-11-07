use crate::{CredentialState, Uuid};
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelSelection {
    pub provider: String,
    pub model: String,
}
impl ModelSelection {
    pub fn valid(&self) -> bool {
        [&self.provider, &self.model]
            .iter()
            .all(|s| !s.trim().is_empty() && s.len() <= 4096 && !s.contains('\0'))
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ModelPhase {
    Idle,
    Ready,
    Unknown,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelConnection {
    pub id: Uuid,
    pub selection: ModelSelection,
    pub generation: Uuid,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelState {
    pub phase: ModelPhase,
    pub connection: Option<ModelConnection>,
    pub credential: CredentialState,
}
impl ModelState {
    pub(crate) fn valid(&self) -> bool {
        self.credential.valid()
            && self.connection.as_ref().is_none_or(|c| {
                !c.id.is_nil()
                    && !c.generation.is_nil()
                    && c.selection.valid()
                    && self.credential.generation == Some(c.generation)
            })
            && match self.phase {
                ModelPhase::Idle => self.connection.is_none(),
                ModelPhase::Ready => self.connection.is_some() && self.credential.installed,
                ModelPhase::Unknown => true,
            }
    }
}

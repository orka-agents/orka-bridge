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
        let p = self.provider.as_bytes();
        !p.is_empty()
            && p.len() <= 45
            && p.first().is_some_and(u8::is_ascii_alphanumeric)
            && p.last().is_some_and(u8::is_ascii_alphanumeric)
            && p.iter()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-')
            && !self.model.is_empty()
            && self.model.len() <= 256
            && self.model.bytes().all(|b| (b'!'..=b'~').contains(&b))
            && !self.model.eq_ignore_ascii_case("auto")
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

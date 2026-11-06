//! Private transport payload, never a renderer or persistent operation value.
use crate::{ProtocolError, Uuid};
use serde::{Deserialize, Deserializer, Serialize};
pub const MAX_CREDENTIAL: usize = 16 * 1024;
pub const CREDENTIAL_PROFILE: u16 = 1;

#[derive(Clone, PartialEq, Serialize)]
pub struct CredentialPayload(Vec<u8>);
impl CredentialPayload {
    pub fn new(bytes: &[u8]) -> Result<Self, ProtocolError> {
        if bytes.is_empty()
            || bytes.len() > MAX_CREDENTIAL
            || !bytes.iter().all(|b| (33..=126).contains(b))
        {
            return Err(ProtocolError::Invalid);
        }
        Ok(Self(bytes.to_vec()))
    }
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}
impl std::fmt::Debug for CredentialPayload {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CredentialPayload([REDACTED])")
    }
}
impl<'de> Deserialize<'de> for CredentialPayload {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let bytes = Vec::<u8>::deserialize(d)?;
        Self::new(&bytes).map_err(|_| serde::de::Error::custom("invalid credential payload"))
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CredentialState {
    pub profile: u16,
    pub generation: Option<Uuid>,
    pub installed: bool,
}
impl CredentialState {
    pub(crate) fn valid(&self) -> bool {
        self.profile == CREDENTIAL_PROFILE
            && !self.generation.is_some_and(|id| id.is_nil())
            && (!self.installed || self.generation.is_some())
    }
}

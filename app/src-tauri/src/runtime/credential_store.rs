//! Native credential storage. No plaintext fallback.
use bridge_runtime_protocol::{CredentialPayload, Uuid, MAX_CREDENTIAL};

pub trait CredentialStore: Send + Sync {
    fn save(&self, slot: Uuid, secret: &[u8]) -> Result<(), StoreError>;
    fn load(&self, slot: Uuid) -> Result<CredentialPayload, StoreError>;
    fn delete(&self, slot: Uuid) -> Result<(), StoreError>;
}

pub struct OsCredentialStore;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StoreError {
    Unavailable,
    Invalid,
    TooLarge,
}
impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Unavailable => "credential store unavailable",
            Self::Invalid => "invalid credential",
            Self::TooLarge => "credential too large",
        })
    }
}
impl std::error::Error for StoreError {}

#[cfg(windows)]
const STORE_LIMIT: usize =
    windows_sys::Win32::Security::Credentials::CRED_MAX_CREDENTIAL_BLOB_SIZE as usize;
#[cfg(not(windows))]
const STORE_LIMIT: usize = MAX_CREDENTIAL;
fn payload(secret: &[u8]) -> Result<CredentialPayload, StoreError> {
    if secret.len() > STORE_LIMIT || secret.len() > MAX_CREDENTIAL {
        return Err(StoreError::TooLarge);
    }
    CredentialPayload::new(secret).map_err(|_| StoreError::Invalid)
}
fn valid_slot(slot: Uuid) -> Result<(), StoreError> {
    if slot.is_nil() {
        Err(StoreError::Invalid)
    } else {
        Ok(())
    }
}
impl CredentialStore for OsCredentialStore {
    fn save(&self, slot: Uuid, secret: &[u8]) -> Result<(), StoreError> {
        valid_slot(slot)?;
        native::save(slot, payload(secret)?.as_bytes())
    }
    fn load(&self, slot: Uuid) -> Result<CredentialPayload, StoreError> {
        valid_slot(slot)?;
        native::load(slot)
    }
    fn delete(&self, slot: Uuid) -> Result<(), StoreError> {
        valid_slot(slot)?;
        native::delete(slot)
    }
}

#[cfg(any(target_os = "macos", test))]
fn suppress_interaction<T>(
    get: impl FnOnce() -> Result<bool, StoreError>,
    mut set: impl FnMut(bool) -> Result<(), StoreError>,
    operation: impl FnOnce() -> Result<T, StoreError>,
) -> Result<T, StoreError> {
    let previous = get()?;
    if set(false).is_err() {
        let _ = set(previous);
        return Err(StoreError::Unavailable);
    }
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(operation));
    let restored = set(previous);
    match result {
        Ok(result) => {
            restored?;
            result
        }
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
mod native {
    use super::*;
    pub fn save(_: Uuid, _: &[u8]) -> Result<(), StoreError> {
        Err(StoreError::Unavailable)
    }
    pub fn load(_: Uuid) -> Result<CredentialPayload, StoreError> {
        Err(StoreError::Unavailable)
    }
    pub fn delete(_: Uuid) -> Result<(), StoreError> {
        Err(StoreError::Unavailable)
    }
}

#[cfg(target_os = "macos")]
mod native {
    use super::*;
    use core_foundation::{base::TCFType, string::CFString};
    use security_framework::{
        os::macos::keychain::SecKeychain,
        passwords::{self, PasswordOptions},
    };
    use security_framework_sys::{
        base::errSecItemNotFound, item::kSecUseAuthenticationUI,
        keychain::SecKeychainSetUserInteractionAllowed,
    };

    fn noninteractive<T>(
        operation: impl FnOnce() -> Result<T, StoreError>,
    ) -> Result<T, StoreError> {
        // Legacy Keychain ignores the per-query UI option. Preserve its process-local policy.
        static ACCESS: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _access = ACCESS.lock().map_err(|_| StoreError::Unavailable)?;
        suppress_interaction(
            || SecKeychain::user_interaction_allowed().map_err(|_| StoreError::Unavailable),
            |allowed| {
                // SAFETY: documented Boolean argument; no pointers, process-local setting only.
                if unsafe { SecKeychainSetUserInteractionAllowed(allowed as u8) } == 0 {
                    Ok(())
                } else {
                    Err(StoreError::Unavailable)
                }
            },
            operation,
        )
    }

    fn options(slot: Uuid) -> PasswordOptions {
        let mut options =
            PasswordOptions::new_generic_password("ai.orka.bridge.signin.v1", &slot.to_string());
        options.set_access_synchronized(Some(false));
        // sys 2.17 omits kSecUseAuthenticationUIFail; Apple's SecItemConstants.c defines its value.
        #[allow(deprecated)]
        options.query.push((
            // SAFETY: this framework constant is a process-lifetime CFString; get-rule retains it.
            unsafe { CFString::wrap_under_get_rule(kSecUseAuthenticationUI) },
            CFString::from("u_AuthUIF").into_CFType(),
        ));
        options
    }
    pub fn save(slot: Uuid, secret: &[u8]) -> Result<(), StoreError> {
        noninteractive(|| {
            passwords::set_generic_password_options(secret, options(slot))
                .map_err(|_| StoreError::Unavailable)
        })
    }
    pub fn load(slot: Uuid) -> Result<CredentialPayload, StoreError> {
        noninteractive(|| {
            let bytes =
                passwords::generic_password(options(slot)).map_err(|_| StoreError::Unavailable)?;
            payload(&bytes)
        })
    }
    pub fn delete(slot: Uuid) -> Result<(), StoreError> {
        noninteractive(
            || match passwords::delete_generic_password_options(options(slot)) {
                Ok(()) => Ok(()),
                Err(error) if error.code() == errSecItemNotFound => Ok(()),
                Err(_) => Err(StoreError::Unavailable),
            },
        )
    }
}

#[cfg(windows)]
mod native {
    use super::*;
    use windows_sys::Win32::{
        Foundation::{GetLastError, ERROR_NOT_FOUND},
        Security::Credentials::*,
    };

    fn target(slot: Uuid) -> Vec<u16> {
        format!("ai.orka.bridge.signin.v1/{slot}")
            .encode_utf16()
            .chain(Some(0))
            .collect()
    }
    pub fn save(slot: Uuid, secret: &[u8]) -> Result<(), StoreError> {
        let mut name = target(slot);
        let credential = CREDENTIALW {
            Type: CRED_TYPE_GENERIC,
            TargetName: name.as_mut_ptr(),
            CredentialBlobSize: secret.len() as u32,
            CredentialBlob: secret.as_ptr().cast_mut(),
            Persist: CRED_PERSIST_LOCAL_MACHINE,
            ..Default::default()
        };
        // SAFETY: pointers remain live for the call; validated secret is within the native blob limit.
        if unsafe { CredWriteW(&credential, 0) } == 0 {
            Err(StoreError::Unavailable)
        } else {
            Ok(())
        }
    }
    struct OwnedCredential(*mut CREDENTIALW);
    impl Drop for OwnedCredential {
        fn drop(&mut self) {
            // SAFETY: the successful CredReadW allocation is owned solely by this guard.
            unsafe { CredFree(self.0.cast()) };
        }
    }
    pub fn load(slot: Uuid) -> Result<CredentialPayload, StoreError> {
        let name = target(slot);
        let mut ptr = std::ptr::null_mut();
        // SAFETY: name is NUL terminated; ptr is a writable output pointer.
        if unsafe { CredReadW(name.as_ptr(), CRED_TYPE_GENERIC, 0, &mut ptr) } == 0 {
            return Err(StoreError::Unavailable);
        }
        if ptr.is_null() {
            return Err(StoreError::Invalid);
        }
        let owned = OwnedCredential(ptr);
        // SAFETY: a successful read returned an owned CREDENTIALW; guard outlives its borrowed data.
        let credential = unsafe { &*owned.0 };
        let len = credential.CredentialBlobSize as usize;
        if len > STORE_LIMIT {
            return Err(StoreError::TooLarge);
        }
        if len == 0 || credential.CredentialBlob.is_null() {
            return Err(StoreError::Invalid);
        }
        // SAFETY: non-null blob and bounded length are provided by the owned native credential.
        payload(unsafe { std::slice::from_raw_parts(credential.CredentialBlob, len) })
    }
    pub fn delete(slot: Uuid) -> Result<(), StoreError> {
        let name = target(slot);
        // SAFETY: name is a live NUL-terminated target, scoped to this exact slot.
        if unsafe { CredDeleteW(name.as_ptr(), CRED_TYPE_GENERIC, 0) } != 0
            || unsafe { GetLastError() } == ERROR_NOT_FOUND
        {
            Ok(())
        } else {
            Err(StoreError::Unavailable)
        }
    }
}

#[cfg(test)]
#[path = "credential_store_tests.rs"]
mod tests;

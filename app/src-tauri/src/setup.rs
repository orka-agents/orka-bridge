use serde::Serialize;
use std::{
    fs::{self, OpenOptions},
    io::ErrorKind,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SetupChoice {
    LocalVm,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SetupSnapshot {
    pub selected: Option<SetupChoice>,
    local_vm: &'static str,
    local_vm_error: Option<String>,
    model: &'static str,
}

impl SetupSnapshot {
    pub fn with_model(mut self, state: &'static str) -> Self {
        self.model = state;
        self
    }
    pub fn with_local_vm(mut self, state: &'static str, error: Option<String>) -> Self {
        self.local_vm = state;
        self.local_vm_error = error;
        self
    }
}

#[derive(Debug, Serialize)]
pub struct SetupError {
    kind: &'static str,
    message: &'static str,
}
impl SetupError {
    pub(crate) fn model(kind: &'static str) -> Self {
        let message = match kind {
            "modelCaptureUnavailable" => {
                "Install GitHub CLI and sign in to github.com before connecting."
            }
            "modelCaptureFailed" => {
                "The stored GitHub sign-in could not be read. Check your GitHub CLI sign-in."
            }
            "modelCaptureInvalid" => "The stored GitHub sign-in returned invalid credential data.",
            "modelCaptureTimedOut" => "Reading the stored GitHub sign-in timed out.",
            "modelStore" => "The native credential store could not save or load the sign-in.",
            "modelGuest" => "The guest model handler is unavailable or is not idle.",
            "modelDelivery" => {
                "Private credential delivery was not acknowledged. Completion is unknown."
            }
            "modelConnect" => {
                "Guest Connect did not acknowledge a ready model. Completion is unknown."
            }
            "modelBusy" => "Another operation is using the local VM. Wait for it to finish.",
            "modelSession" => "The local VM is not available for model connection.",
            _ => "The stored model connection is unavailable or its completion is unknown.",
        };
        Self { kind, message }
    }
}
fn storage_error() -> SetupError {
    SetupError {
        kind: "storage",
        message: "The setup choice could not be read or saved.",
    }
}

#[path = "setup_model.rs"]
mod model;
pub use model::ModelSetupState;

/// Owns preferences and an opaque sign-in association, never stored secret bytes.
pub struct SetupStore {
    root: Option<PathBuf>,
    access: Mutex<()>,
}
impl SetupStore {
    pub fn new(app_data: Option<PathBuf>) -> Self {
        Self {
            root: app_data.map(|path| path.join("setup-v1")),
            access: Mutex::new(()),
        }
    }
    pub async fn status_async(self: Arc<Self>) -> Result<SetupSnapshot, SetupError> {
        tokio::task::spawn_blocking(move || self.status())
            .await
            .map_err(|_| storage_error())?
    }
    pub async fn select_local_async(self: Arc<Self>) -> Result<SetupSnapshot, SetupError> {
        tokio::task::spawn_blocking(move || self.select_local())
            .await
            .map_err(|_| storage_error())?
    }
    pub fn status(&self) -> Result<SetupSnapshot, SetupError> {
        let _access = self.access.lock().map_err(|_| storage_error())?;
        snapshot(self.root.as_deref().ok_or_else(storage_error)?)
    }
    pub fn select_local(&self) -> Result<SetupSnapshot, SetupError> {
        let _access = self.access.lock().map_err(|_| storage_error())?;
        let root = self.root.as_deref().ok_or_else(storage_error)?;
        if !selected(root)? {
            fs::create_dir_all(root.parent().ok_or_else(storage_error)?)
                .map_err(|_| storage_error())?;
            match fs::create_dir(root) {
                Ok(()) => {}
                Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
                Err(_) => return Err(storage_error()),
            }
            // Revalidate after directory creation, including the concurrent creator case.
            selected(root)?;
            match OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(root.join("local-vm"))
            {
                Ok(file) => file.sync_all().map_err(|_| storage_error())?,
                Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
                Err(_) => return Err(storage_error()),
            }
        }
        snapshot(root)
    }
}

fn selected(root: &Path) -> Result<bool, SetupError> {
    // Windows can report NotFound for a child of a regular file. Validate the
    // native app-data parent separately instead of mistaking that for no choice.
    match fs::metadata(root.parent().ok_or_else(storage_error)?) {
        Ok(metadata) if metadata.is_dir() => {}
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(false),
        _ => return Err(storage_error()),
    }
    match fs::symlink_metadata(root) {
        Ok(metadata) if metadata.file_type().is_dir() => {}
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(false),
        _ => return Err(storage_error()),
    }
    match fs::symlink_metadata(root.join("local-vm")) {
        Ok(metadata) if metadata.file_type().is_file() && metadata.len() == 0 => Ok(true),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
        _ => Err(storage_error()),
    }
}
fn snapshot(root: &Path) -> Result<SetupSnapshot, SetupError> {
    Ok(SetupSnapshot {
        selected: selected(root)?.then_some(SetupChoice::LocalVm),
        // Selection is not runtime readiness. These gates have no renderer override.
        local_vm: "notIntegrated",
        local_vm_error: None,
        model: "locked",
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        sync::{mpsc, Arc, Barrier},
        time::Duration,
    };

    #[tokio::test(flavor = "current_thread")]
    async fn filesystem_waits_do_not_stall_the_async_caller() {
        for select in [false, true] {
            let tmp = tempfile::tempdir().unwrap();
            let store = Arc::new(SetupStore::new(Some(tmp.path().to_owned())));
            let barrier = Arc::new(Barrier::new(2));
            let (send, receive) = mpsc::channel();
            let blocked = Arc::clone(&store);
            let waiting = Arc::clone(&barrier);
            let holder = std::thread::spawn(move || {
                let _guard = blocked.access.lock().unwrap();
                waiting.wait();
                receive.recv_timeout(Duration::from_secs(2)).is_ok()
            });
            barrier.wait();
            tokio::join!(biased;
                async {
                    if select { store.select_local_async().await.unwrap(); }
                    else { store.status_async().await.unwrap(); }
                },
                async { tokio::task::yield_now().await; let _ = send.send(()); }
            );
            assert!(
                holder.join().unwrap(),
                "Preference I/O blocked the async caller"
            );
        }
    }
}

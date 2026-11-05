use serde::Serialize;
use std::{
    fs::{self, OpenOptions},
    io::ErrorKind,
    path::{Path, PathBuf},
    sync::Mutex,
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
    model: &'static str,
}

#[derive(Debug, Serialize)]
pub struct SetupError {
    kind: &'static str,
    message: &'static str,
}
fn storage_error() -> SetupError {
    SetupError {
        kind: "storage",
        message: "The setup choice could not be read or saved.",
    }
}

/// Owns only a preference. It cannot install a backend, connect a model or hold credentials.
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
        model: "locked",
    })
}

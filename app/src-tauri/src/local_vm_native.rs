use super::*;
#[cfg(test)]
use crate::runtime::types::BackendKind;
#[cfg(any(all(debug_assertions, windows), test))]
use crate::runtime::windows::inputs::{verify_windows_runtime, FilePin};
#[cfg(any(
    all(
        debug_assertions,
        any(windows, all(target_os = "macos", target_arch = "aarch64"))
    ),
    test
))]
use crate::runtime::{
    image_install::{open_installed_image, InstalledImageSet},
    journal::Journal,
};
#[cfg(any(all(debug_assertions, windows), test))]
use std::io::Read;
use std::path::PathBuf;
#[cfg(any(
    all(
        debug_assertions,
        any(windows, all(target_os = "macos", target_arch = "aarch64"))
    ),
    test
))]
use std::{fs, path::Path};

/// Release has no approved image catalog; native development uses the existing
/// verified source override or its own committed installation, never renderer paths.
pub fn configured(resources: Option<PathBuf>, data: Option<PathBuf>) -> Arc<LocalVm> {
    #[cfg(all(
        debug_assertions,
        any(windows, all(target_os = "macos", target_arch = "aarch64"))
    ))]
    if let (Some(resources), Some(data)) = (&resources, &data) {
        let root = data.join("local-vm-v1");
        if std::env::var_os("BRIDGE_DEV_IMAGE_DIR").is_some() || root.exists() {
            return LocalVm::new(Arc::new(NativeFactory {
                resources: resources.clone(),
                root,
            }));
        }
    }
    let _ = (resources, data);
    LocalVm::unavailable()
}
#[cfg(any(all(debug_assertions, windows), test))]
fn runtime_manifest_pin() -> FilePin {
    FilePin {
        name: "provenance/runtime-manifest.json".into(),
        bytes: 117615,
        sha256: "a1dead19c0d1dcef558008e556cd9bd4a8c5082daf4ecea6f531d03173748385".into(),
    }
}
#[cfg(any(all(debug_assertions, windows), test))]
fn read_runtime_manifest(root: &Path, pin: &FilePin) -> Result<Vec<FilePin>, RuntimeError> {
    use sha2::{Digest, Sha256};
    #[derive(serde::Deserialize)]
    struct Manifest {
        files: Vec<Entry>,
    }
    #[derive(serde::Deserialize)]
    struct Entry {
        path: String,
        bytes: u64,
        sha256: String,
    }
    let path = root.join(&pin.name);
    if !fs::symlink_metadata(&path)?.file_type().is_file() {
        return Err(RuntimeError::Unavailable);
    }
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options.open(path)?;
    if !file.metadata()?.is_file() || file.metadata()?.len() != pin.bytes {
        return Err(RuntimeError::Unavailable);
    }
    let mut bytes = Vec::new();
    file.take(pin.bytes + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 != pin.bytes || format!("{:x}", Sha256::digest(&bytes)) != pin.sha256 {
        return Err(RuntimeError::Unavailable);
    }
    // The compiled hash authenticates every producer row before projection.
    let manifest: Manifest =
        serde_json::from_slice(&bytes).map_err(|_| RuntimeError::Unavailable)?;
    let mut pins: Vec<_> = manifest
        .files
        .into_iter()
        .map(|e| FilePin {
            name: e.path,
            bytes: e.bytes,
            sha256: e.sha256,
        })
        .collect();
    pins.push(pin.clone()); // Nested provenance manifest is itself in the exact inventory.
    Ok(pins)
}
#[cfg(any(
    all(
        debug_assertions,
        any(windows, all(target_os = "macos", target_arch = "aarch64"))
    ),
    test
))]
fn reopen(
    root: &Path,
    compatibility: &str,
) -> Result<Option<(Journal, Arc<InstalledImageSet>)>, RuntimeError> {
    match fs::symlink_metadata(root) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
        Ok(_) => {
            let journal = Journal::open(root)?;
            let image = open_installed_image(&journal.image_target(compatibility)?)?;
            Ok(Some((journal, Arc::new(image))))
        }
    }
}
#[cfg(all(
    debug_assertions,
    any(windows, all(target_os = "macos", target_arch = "aarch64"))
))]
struct NativeFactory {
    resources: PathBuf,
    root: PathBuf,
}
#[cfg(all(
    debug_assertions,
    any(windows, all(target_os = "macos", target_arch = "aarch64"))
))]
impl LocalFactory for NativeFactory {
    fn launch(
        &self,
        closed: watch::Receiver<bool>,
        progress: Arc<dyn Fn(LocalVmStatus) + Send + Sync>,
    ) -> StartFuture<'_> {
        let (resources, root) = (self.resources.clone(), self.root.clone());
        Box::pin(async move {
            let runtime = tokio::runtime::Handle::current();
            tokio::task::spawn_blocking(move || {
                runtime.block_on(async move {
                    use crate::{
                        dev_image,
                        runtime::{self, types::BackendKind},
                    };
                    if *closed.borrow() {
                        return Err(RuntimeError::Closed);
                    }
                    #[cfg(windows)]
                    let inputs = {
                        runtime::windows::launch::control_path(
                            &root,
                            bridge_runtime_protocol::Uuid::nil(),
                        )?;
                        let path = resources.join("native/qemu");
                        Arc::new(verify_windows_runtime(
                            &read_runtime_manifest(&path, &runtime_manifest_pin())?,
                            path,
                        )?)
                    };
                    let installed = match reopen(&root, dev_image::STATE_COMPATIBILITY_ID)? {
                        Some(existing) => existing,
                        None => {
                            let image = dev_image::from_environment()
                                .map_err(|_| RuntimeError::Unavailable)?
                                .ok_or(RuntimeError::Unavailable)?;
                            let prepared = runtime::setup_adapter::prepare_development_image(
                                &image,
                                closed.clone(),
                            )
                            .await?;
                            if *closed.borrow() {
                                return Err(RuntimeError::Closed);
                            }
                            #[cfg(windows)]
                            let (backend, identity) = (BackendKind::Qemu, b"pc-q35-11.0".to_vec());
                            #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
                            let (backend, identity) = (
                                BackendKind::Apple,
                                runtime::apple_input::machine_identifier(&resources)?,
                            );
                            if *closed.borrow() {
                                return Err(RuntimeError::Closed);
                            }
                            fs::create_dir_all(root.parent().ok_or(RuntimeError::Unavailable)?)?;
                            let journal = Journal::create(&root, backend, identity)?;
                            progress(LocalVmStatus::Installing);
                            let image = prepared
                                .materialize(
                                    &journal.image_target(dev_image::STATE_COMPATIBILITY_ID)?,
                                    closed.clone(),
                                    |_| {},
                                )
                                .await?;
                            (journal, Arc::new(image))
                        }
                    };
                    if *closed.borrow() {
                        return Err(RuntimeError::Closed);
                    }
                    progress(LocalVmStatus::Starting);
                    let utc = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map_err(|_| RuntimeError::Unavailable)?
                        .as_secs();
                    let installation = installed.1.installation_id();
                    #[cfg(windows)]
                    let session = runtime::windows::session::WindowsSession::start(
                        runtime::RuntimeWorker::new(
                            Box::new(runtime::windows::backend::WindowsBackend::new(inputs)),
                            installed.0,
                            installed.1,
                        )?,
                        utc,
                    )?;
                    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
                    let session = runtime::apple_session::AppleVmSession::launch(
                        resources,
                        installed.0,
                        installed.1,
                        utc,
                    )?;
                    Ok(Box::new(NativeSession(session, installation)) as Box<dyn LocalSession>)
                })
            })
            .await
            .map_err(|_| RuntimeError::Unavailable)?
        })
    }
}
#[cfg(all(debug_assertions, windows))]
struct NativeSession(
    crate::runtime::windows::session::WindowsSession,
    bridge_runtime_protocol::Uuid,
);
#[cfg(all(debug_assertions, all(target_os = "macos", target_arch = "aarch64")))]
struct NativeSession(
    crate::runtime::apple_session::AppleVmSession,
    bridge_runtime_protocol::Uuid,
);
#[cfg(all(
    debug_assertions,
    any(windows, all(target_os = "macos", target_arch = "aarch64"))
))]
impl LocalSession for NativeSession {
    fn with_client(
        &mut self,
        work: &mut dyn FnMut(
            bridge_runtime_protocol::Uuid,
            &mut VmClient,
        ) -> Result<(), RuntimeError>,
    ) -> Result<(), RuntimeError> {
        #[cfg(windows)]
        {
            work(self.1, self.0.client_mut()?)
        }
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        {
            self.0
                .transport()
                .ok_or(RuntimeError::Unavailable)?
                .borrow_client(|client| work(self.1, client))
                .map_err(|_| RuntimeError::Unavailable)?
        }
    }
    fn execute(&mut self, request: EngineRequest) -> TransportFuture<'_> {
        #[cfg(windows)]
        {
            Box::pin(async move {
                let client = self
                    .0
                    .client_mut()
                    .map_err(|_| EngineError::new(ErrorKind::Unavailable))?;
                let id = client
                    .execute(request.action, None)
                    .map_err(|_| EngineError::new(ErrorKind::Execution))?;
                let result = client
                    .result(id)
                    .map_err(|_| EngineError::new(ErrorKind::Execution))?;
                Ok(EngineOutput {
                    exit: result.exit,
                    stdout: result.stdout,
                    stderr: if result.has_diagnostics {
                        vec![1]
                    } else {
                        Vec::new()
                    },
                })
            })
        }
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        Box::pin(async move {
            self.0
                .transport()
                .ok_or_else(|| EngineError::new(ErrorKind::Unavailable))?
                .execute(request)
                .await
        })
    }
    fn stop(&mut self) -> Result<(), RuntimeError> {
        #[cfg(windows)]
        {
            self.0.shutdown()
        }
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        self.0.stop()
    }
}
#[cfg(test)]
#[path = "local_vm_native_tests.rs"]
mod tests;

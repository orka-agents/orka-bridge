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

#[cfg(any(
    all(
        debug_assertions,
        any(windows, all(target_os = "macos", target_arch = "aarch64"))
    ),
    test
))]
async fn blocking_start(
    work: impl FnOnce(tokio::runtime::Handle) -> Result<Box<dyn LocalSession>, RuntimeError>
        + Send
        + 'static,
) -> Result<Box<dyn LocalSession>, RuntimeError> {
    let runtime = tokio::runtime::Handle::current();
    // Only async image operations may enter block_on. Synchronous backend
    // calls must run outside that context, including inheritable-handle guards.
    tokio::task::spawn_blocking(move || work(runtime))
        .await
        .map_err(|_| RuntimeError::Unavailable)?
}

#[cfg(any(all(debug_assertions, windows), test))]
fn retain_start_failure(
    root: &Path,
    stage: &str,
    error: RuntimeError,
    stderr: std::io::Result<Vec<u8>>,
) -> RuntimeError {
    use std::io::Write;
    let reason = failure_reason(&error);
    let (stderr, capture_error) = match stderr {
        Ok(bytes) => (bytes, None),
        Err(error) => (Vec::new(), Some(error.to_string())),
    };
    let directory = root.join(format!(
        "startup-failure-{}",
        bridge_runtime_protocol::Uuid::new_v4()
    ));
    let saved = (|| -> std::io::Result<()> {
        // Fresh names never overwrite evidence or follow an existing file.
        #[cfg(windows)]
        let _held = crate::runtime::windows::security::create_af_directory(&directory)?;
        #[cfg(not(windows))]
        {
            use std::os::unix::fs::DirBuilderExt;
            fs::DirBuilder::new().mode(0o700).create(&directory)?;
        }
        for (name, bytes) in [
            ("qemu-stderr.txt", stderr.clone()),
            (
                "failure.json",
                serde_json::to_vec_pretty(&serde_json::json!({
                    "stage": stage, "reason": reason, "stderrCaptureError": capture_error,
                }))?,
            ),
        ] {
            let mut options = fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(directory.join(name))?;
            file.write_all(&bytes)?;
            file.sync_all()?;
        }
        Ok(())
    })();
    let diagnostic: String = String::from_utf8_lossy(&stderr)
        .trim()
        .chars()
        .take(2048)
        .collect();
    let mut message = format!("{stage}: {reason}");
    if !diagnostic.is_empty() {
        message.push_str(&format!(". QEMU: {diagnostic}"));
    }
    match saved {
        Ok(()) => message.push_str(&format!(". Details: {}", directory.display())),
        Err(error) => message.push_str(&format!(". Could not save diagnostics: {error}")),
    }
    std::io::Error::other(message).into()
}

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
        Box::pin(blocking_start(move |runtime| {
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
                    let prepared = runtime.block_on(
                        runtime::setup_adapter::prepare_development_image(&image, closed.clone()),
                    )?;
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
                    let image = runtime.block_on(prepared.materialize(
                        &journal.image_target(dev_image::STATE_COMPATIBILITY_ID)?,
                        closed.clone(),
                        |_| {},
                    ))?;
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
            #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
            let installation = installed.1.installation_id();
            #[cfg(windows)]
            let native = start_windows_session(inputs, installed.0, installed.1, &root, utc)?;
            #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
            let session = runtime::apple_session::AppleVmSession::launch(
                resources,
                installed.0,
                installed.1,
                utc,
            )?;
            #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
            let native = NativeSession(session, installation);
            Ok(Box::new(native) as Box<dyn LocalSession>)
        }))
    }
}
#[cfg(all(debug_assertions, windows))]
fn start_windows_session(
    inputs: Arc<crate::runtime::windows::inputs::VerifiedWindowsInputs>,
    journal: Journal,
    image: Arc<InstalledImageSet>,
    root: &Path,
    utc: u64,
) -> Result<NativeSession, RuntimeError> {
    // Keep the verified private root non-replaceable even if failed startup or
    // shutdown releases its worker before diagnostics are written.
    let diagnostic_root = crate::runtime::windows::security::private_af_directory(root)?;
    let installation = image.installation_id();
    let backend = crate::runtime::windows::backend::WindowsBackend::new(inputs);
    let failure = backend.failure_diagnostics();
    let result = crate::runtime::RuntimeWorker::new(Box::new(backend), journal, image)
        .and_then(|worker| crate::runtime::windows::session::WindowsSession::start(worker, utc));
    match result {
        Ok(session) => Ok(NativeSession {
            session,
            installation,
            root: root.to_owned(),
            _diagnostic_root: diagnostic_root,
        }),
        Err(error) => Err(retain_start_failure(
            root,
            failure.stage(),
            error,
            failure.stderr(),
        )),
    }
}
#[cfg(all(debug_assertions, windows))]
struct NativeSession {
    session: crate::runtime::windows::session::WindowsSession,
    installation: bridge_runtime_protocol::Uuid,
    root: PathBuf,
    _diagnostic_root: fs::File,
}
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
            work(self.installation, self.session.client_mut()?)
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
                    .session
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
            self.session.shutdown().map_err(|error| {
                let stderr = self
                    .session
                    .diagnostics()
                    .map(|diagnostics| diagnostics.stderr)
                    .map_err(|error| std::io::Error::other(error.to_string()));
                retain_start_failure(&self.root, "Shut down guest", error, stderr)
            })
        }
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        self.0.stop()
    }
}
#[cfg(test)]
#[path = "local_vm_native_tests.rs"]
mod tests;

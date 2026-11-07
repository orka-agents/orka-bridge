#[cfg(any(windows, test))]
use std::{
    fs::File,
    io::{self, Seek},
};
#[cfg(any(windows, test))]
fn fill_state(scratch: &mut File, state: &mut File) -> io::Result<()> {
    if state.metadata()?.len() != 0 {
        return Err(io::Error::other("existing state is retained"));
    }
    scratch.rewind()?;
    state.rewind()?;
    io::copy(scratch, state)?;
    state.sync_all()
}
#[cfg(windows)]
pub struct Diagnostics {
    pub rx: Vec<u8>,
    pub tx: Vec<u8>,
    pub stderr: Vec<u8>,
}
#[cfg(windows)]
mod native {
    use super::*;
    use crate::runtime::{
        types::*,
        windows::{
            control::{ControlEndpoint, GuestStream},
            inputs::VerifiedWindowsInputs,
            job::OwnedQemuProcess,
            launch::{launch_argv, validate_qcow2_info},
            qmp::{Qmp, Transport},
            qmp_io::NativeTransport,
        },
    };
    use std::{
        ffi::OsString,
        fs::{self, OpenOptions},
        path::Path,
        sync::{Arc, Mutex},
        time::{Duration, Instant},
    };
    #[derive(Clone, Default)]
    pub struct FailureDiagnostics(
        Arc<Mutex<Option<crate::runtime::windows::qmp_io::StderrCapture>>>,
    );
    impl FailureDiagnostics {
        pub fn stderr(&self) -> std::io::Result<Vec<u8>> {
            self.0
                .lock()
                .map_err(|_| std::io::Error::other("diagnostics poisoned"))?
                .as_ref()
                .map_or(Ok(Vec::new()), |capture| capture.snapshot())
        }
    }
    pub struct WindowsBackend {
        inputs: Arc<VerifiedWindowsInputs>,
        failed: bool,
        diagnostics: FailureDiagnostics,
    }
    #[derive(Clone)]
    pub(crate) struct Lease(pub Arc<Mutex<Vm>>);
    pub(crate) struct Vm {
        pub owner: Arc<OwnedQemuProcess>,
        qmp: Qmp<NativeTransport>,
        guest: Option<GuestStream>,
    }
    impl WindowsBackend {
        pub fn new(inputs: Arc<VerifiedWindowsInputs>) -> Self {
            Self {
                inputs,
                failed: false,
                diagnostics: FailureDiagnostics::default(),
            }
        }
        pub fn failure_diagnostics(&self) -> FailureDiagnostics {
            self.diagnostics.clone()
        }
        fn tool(&self, spec: &LaunchSpec, args: &[OsString]) -> Result<Vec<u8>, RuntimeError> {
            let endpoint = ControlEndpoint::create(&spec.root)?;
            let mut child = OwnedQemuProcess::spawn_image_tool(
                self.inputs.clone(),
                spec.ownership.clone(),
                &spec.root,
                args,
                endpoint,
            )?;
            let stdio = child.take_stdio()?;
            let owner = Arc::new(child);
            let mut io = NativeTransport::new(owner.clone(), stdio)?;
            *self
                .diagnostics
                .0
                .lock()
                .map_err(|_| RuntimeError::Closed)? = Some(io.stderr_capture());
            let until = io.prepare_stop()?;
            owner.resume()?;
            let mut output = Vec::new();
            loop {
                match io.receive(until) {
                    Ok(frame) => {
                        if output.len() + frame.bytes.len() > 128 * 1024 {
                            return Err(
                                std::io::Error::from(std::io::ErrorKind::InvalidData).into()
                            );
                        }
                        output.extend(frame.bytes);
                        if let Some(error) = frame.failure {
                            if error.kind() == std::io::ErrorKind::UnexpectedEof {
                                break;
                            }
                            return Err(error.into());
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => break,
                    Err(error) => return Err(error.into()),
                }
            }
            io.wait_for_stop()?;
            if owner.exit_code()? != 0 {
                return Err(std::io::Error::other("owned qemu-img failed").into());
            }
            Ok(output)
        }
        fn inspect(&self, spec: &LaunchSpec, path: &Path, size: u64) -> Result<(), RuntimeError> {
            let output = self.tool(
                spec,
                &[
                    "info".into(),
                    "-f".into(),
                    "qcow2".into(),
                    "--output=json".into(),
                    path.as_os_str().into(),
                ],
            )?;
            let size = if size == 0 {
                serde_json::from_slice::<serde_json::Value>(&output)
                    .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidData))?
                    .get("virtual-size")
                    .and_then(serde_json::Value::as_u64)
                    .filter(|n| *n > 0)
                    .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::InvalidData))?
            } else {
                size
            };
            validate_qcow2_info(&output, size)
        }
        fn start_vm(&self, spec: &LaunchSpec) -> Result<BackendHandle, RuntimeError> {
            self.inspect(spec, &spec.root.join(spec.image.disk_path()), 0)?;
            self.inspect(spec, &spec.state_path, spec.state_bytes)?;
            let endpoint = ControlEndpoint::create(&spec.root)?;
            let args = launch_argv(
                &self.inputs,
                spec,
                endpoint
                    .path()
                    .to_str()
                    .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::InvalidInput))?,
            )?;
            let mut child = OwnedQemuProcess::spawn_suspended(
                self.inputs.clone(),
                spec.ownership.clone(),
                &spec.root,
                &args,
                endpoint,
            )?;
            let stdio = child.take_stdio()?;
            let owner = Arc::new(child);
            let transport = NativeTransport::new(owner.clone(), stdio)?;
            *self
                .diagnostics
                .0
                .lock()
                .map_err(|_| RuntimeError::Closed)? = Some(transport.stderr_capture());
            owner.resume()?;
            let until = Instant::now() + Duration::from_secs(30);
            let mut qmp = Qmp::negotiate(transport, until)?;
            let status = qmp.query_status(until)?;
            if status.running || status.status != "prelaunch" {
                return Err(std::io::Error::other("QEMU did not remain prelaunch").into());
            }
            let guest = owner.guest_stream()?;
            qmp.set_running(true, Instant::now() + Duration::from_secs(30))?;
            Ok(BackendHandle::new(Lease(Arc::new(Mutex::new(Vm {
                owner,
                qmp,
                guest: Some(guest),
            })))))
        }
    }
    impl Lease {
        pub(crate) fn guest(&self) -> Result<GuestStream, RuntimeError> {
            self.0
                .lock()
                .map_err(|_| RuntimeError::Closed)?
                .guest
                .take()
                .ok_or(RuntimeError::Closed)
        }
        pub(crate) fn diagnostics(&self) -> Result<Diagnostics, RuntimeError> {
            let vm = self.0.lock().map_err(|_| RuntimeError::Closed)?;
            Ok(Diagnostics {
                rx: vm.qmp.transcript().rx.clone(),
                tx: vm.qmp.transcript().tx.clone(),
                stderr: vm.qmp.transport().stderr()?,
            })
        }
        pub(crate) fn owner(&self) -> Result<Arc<OwnedQemuProcess>, RuntimeError> {
            Ok(self
                .0
                .lock()
                .map_err(|_| RuntimeError::Closed)?
                .owner
                .clone())
        }
    }
    fn vm(handle: &BackendHandle) -> Result<std::sync::MutexGuard<'_, Vm>, RuntimeError> {
        handle
            .get::<Lease>()
            .ok_or(RuntimeError::Unavailable)?
            .0
            .lock()
            .map_err(|_| RuntimeError::Closed)
    }
    impl VmBackend for WindowsBackend {
        fn create_state(&mut self, spec: &LaunchSpec) -> Result<(), RuntimeError> {
            if self.failed || spec.backend != BackendKind::Qemu {
                return Err(RuntimeError::Unavailable);
            }
            let mut state = OpenOptions::new()
                .read(true)
                .write(true)
                .open(&spec.state_path)?;
            if state.metadata()?.len() != 0 {
                return Err(RuntimeError::RetainedState);
            }
            let path = spec.root.join(format!(
                "state-{}.qcow2",
                bridge_runtime_protocol::Uuid::new_v4()
            ));
            let mut scratch = OpenOptions::new()
                .create_new(true)
                .read(true)
                .write(true)
                .open(&path)?;
            let result = (|| {
                self.tool(
                    spec,
                    &[
                        "create".into(),
                        "-f".into(),
                        "qcow2".into(),
                        path.as_os_str().into(),
                        spec.state_bytes.to_string().into(),
                    ],
                )?;
                self.inspect(spec, &path, spec.state_bytes)?;
                fill_state(&mut scratch, &mut state)?;
                Ok(())
            })();
            drop(scratch);
            if result.is_ok() {
                fs::remove_file(path)?;
            } else {
                self.failed = true;
            }
            result
        }
        fn start(&mut self, spec: &LaunchSpec) -> Result<BackendHandle, RuntimeError> {
            if self.failed {
                return Err(RuntimeError::Unavailable);
            }
            let result = self.start_vm(spec);
            if result.is_err() {
                self.failed = true;
            }
            result
        }
        fn stop(&mut self, handle: &BackendHandle) -> Result<(), RuntimeError> {
            let mut vm = vm(handle)?;
            if vm.owner.exited() {
                vm.owner.wait_for_stop()?;
            } else {
                let until = vm
                    .owner
                    .stopping_deadline()
                    .unwrap_or_else(|| Instant::now() + Duration::from_secs(45));
                vm.qmp.quit(until)?;
            }
            if vm.owner.exit_code()? != 0 {
                return Err(std::io::Error::other("QEMU exited unsuccessfully").into());
            }
            Ok(())
        }
        fn pause(&mut self, handle: &BackendHandle) -> Result<(), RuntimeError> {
            vm(handle)?
                .qmp
                .set_running(false, Instant::now() + Duration::from_secs(30))
                .map_err(Into::into)
        }
        fn resume(&mut self, handle: &BackendHandle) -> Result<(), RuntimeError> {
            vm(handle)?
                .qmp
                .set_running(true, Instant::now() + Duration::from_secs(30))
                .map_err(Into::into)
        }
    }
}
#[cfg(windows)]
pub(crate) use native::Lease;
#[cfg(windows)]
pub use native::WindowsBackend;
#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    #[test]
    fn prepared_state_is_filled_without_replacing_its_held_identity() {
        let t = tempfile::tempdir().unwrap();
        let path = t.path().join("state.disk");
        let mut state = std::fs::OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        let identity = crate::runtime::identity::file_identity(&state).unwrap();
        let mut scratch = tempfile::tempfile().unwrap();
        scratch.write_all(b"prepared-qcow").unwrap();
        fill_state(&mut scratch, &mut state).unwrap();
        assert_eq!(
            identity,
            crate::runtime::identity::file_identity(&state).unwrap()
        );
        assert_eq!(
            identity,
            crate::runtime::identity::file_identity(&std::fs::File::open(&path).unwrap()).unwrap()
        );
        let mut bytes = Vec::new();
        state.rewind().unwrap();
        state.read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, b"prepared-qcow");
        assert!(fill_state(&mut scratch, &mut state).is_err());
    }
}

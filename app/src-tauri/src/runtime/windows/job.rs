use std::{
    io,
    sync::{Mutex, MutexGuard},
};
/// Cooperating app-owned spawns must not overlap a Win32 inheritable-handle window.
pub fn spawn_guard() -> io::Result<MutexGuard<'static, ()>> {
    static SPAWNS: Mutex<()> = Mutex::new(());
    SPAWNS
        .lock()
        .map_err(|_| io::Error::other("process spawn coordinator poisoned"))
}
/// Release only after observed exit; otherwise quarantine all still-owned capabilities.
pub fn release_bundle<T>(bundle: T, observed_stop: bool) {
    if !observed_stop {
        std::mem::forget(bundle);
    }
}
pub fn owner_lock_error(e: io::Error) -> crate::runtime::types::RuntimeError {
    if matches!(e.raw_os_error(), Some(32 | 33)) {
        crate::runtime::types::RuntimeError::Busy
    } else {
        e.into()
    }
}

/// The two attribute lists are inseparable: only these handles, already in this Job.
pub struct SpawnPolicy {
    job: [usize; 1],
    handles: Vec<usize>,
}
impl SpawnPolicy {
    pub fn new(job: usize, handles: Vec<usize>) -> io::Result<Self> {
        let unique: std::collections::BTreeSet<_> = handles.iter().copied().collect();
        if job == 0
            || handles.len() != 4
            || unique.len() != 4
            || unique.contains(&0)
            || unique.contains(&job)
        {
            return Err(io::Error::from(io::ErrorKind::InvalidInput));
        }
        Ok(Self {
            job: [job],
            handles,
        })
    }
    pub fn jobs(&self) -> &[usize] {
        &self.job
    }
    pub fn handles(&self) -> &[usize] {
        &self.handles
    }
    pub fn flags(&self) -> u32 {
        0x08080404
    } // CREATE_NO_WINDOW | EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT | CREATE_SUSPENDED
}
pub fn create_verified<T>(
    policy: &SpawnPolicy,
    create: impl FnOnce(&SpawnPolicy) -> io::Result<T>,
    member: impl FnOnce(&T) -> io::Result<bool>,
    reap: impl FnOnce(&T) -> io::Result<()>,
) -> io::Result<T> {
    let child = create(policy)?;
    let failure = match member(&child) {
        Ok(true) => return Ok(child),
        Ok(false) => io::Error::new(
            io::ErrorKind::PermissionDenied,
            "child was not atomically assigned to its Job",
        ),
        Err(e) => e,
    };
    if let Err(cleanup) = reap(&child) {
        // Quarantine capabilities when observed stop failed, just like RuntimeWorker.
        std::mem::forget(child);
        return Err(io::Error::other(format!(
            "{failure}; owned child stop unconfirmed: {cleanup}"
        )));
    }
    Err(failure)
}

#[cfg(windows)]
pub use native::OwnedQemuProcess;
#[cfg(windows)]
mod native {
    use super::super::{inputs::VerifiedWindowsInputs, launch::quote_windows_arg, security::*};
    use super::*;
    use std::{
        ffi::OsString,
        fs::File,
        mem::{size_of, zeroed},
        os::windows::io::AsRawHandle,
        path::Path,
        sync::Arc,
        time::Duration,
    };
    use windows_sys::Win32::{
        Foundation::*,
        Security::SECURITY_ATTRIBUTES,
        System::{
            JobObjects::*, Pipes::CreatePipe, SystemInformation::GetWindowsDirectoryW, Threading::*,
        },
    };

    pub struct OwnedQemuProcess {
        resources: Option<Resources>,
    }
    struct Resources {
        process: Arc<Handle>,
        thread: Handle,
        job: Handle,
        stdio: Option<(File, File, File)>,
        _ownership: Arc<File>,
        _inputs: Arc<VerifiedWindowsInputs>,
        _cwd: File,
    }
    struct Attributes {
        ptr: LPPROC_THREAD_ATTRIBUTE_LIST,
        _storage: Vec<usize>,
    }
    impl Drop for Attributes {
        fn drop(&mut self) {
            unsafe { DeleteProcThreadAttributeList(self.ptr) }
        }
    }
    impl Attributes {
        unsafe fn new(policy: &SpawnPolicy) -> io::Result<Self> {
            let mut size = 0;
            InitializeProcThreadAttributeList(std::ptr::null_mut(), 2, 0, &mut size);
            if size == 0 {
                return Err(io::Error::last_os_error());
            }
            let mut storage = vec![0usize; size.div_ceil(size_of::<usize>())];
            let ptr = storage.as_mut_ptr().cast();
            check(InitializeProcThreadAttributeList(ptr, 2, 0, &mut size))?;
            let result = Self {
                ptr,
                _storage: storage,
            };
            check(UpdateProcThreadAttribute(
                ptr,
                0,
                PROC_THREAD_ATTRIBUTE_JOB_LIST as usize,
                policy.jobs().as_ptr().cast(),
                size_of::<HANDLE>(),
                std::ptr::null_mut(),
                std::ptr::null(),
            ))?;
            check(UpdateProcThreadAttribute(
                ptr,
                0,
                PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
                policy.handles().as_ptr().cast(),
                4 * size_of::<HANDLE>(),
                std::ptr::null_mut(),
                std::ptr::null(),
            ))?;
            Ok(result)
        }
    }
    unsafe fn anonymous(input: bool) -> io::Result<(File, Handle)> {
        let mut read = std::ptr::null_mut();
        let mut write = std::ptr::null_mut();
        let sa = SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: std::ptr::null_mut(),
            bInheritHandle: 1,
        };
        check(CreatePipe(&mut read, &mut write, &sa, 0))?;
        let (parent, child) = if input {
            (Handle(write), Handle(read))
        } else {
            (Handle(read), Handle(write))
        };
        check(SetHandleInformation(parent.0, HANDLE_FLAG_INHERIT, 0))?;
        Ok((parent.into_file(), child))
    }
    impl OwnedQemuProcess {
        fn resources(&self) -> &Resources {
            self.resources.as_ref().expect("live process capability")
        }
        /// Internal capability API. argv comes only from the fixed host launch builders.
        pub fn spawn_suspended(
            inputs: Arc<VerifiedWindowsInputs>,
            ownership: Arc<File>,
            cwd: &Path,
            argv: &[OsString],
        ) -> io::Result<Self> {
            inputs.recheck_runtime().map_err(io::Error::other)?;
            let cwd_file = private_directory(cwd)?;
            let exe = inputs
                .executable("qemu-system-x86_64.exe")
                .map_err(io::Error::other)?;
            let mut command = quote_windows_arg(
                exe.to_str()
                    .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidInput))?,
            )
            .map_err(io::Error::other)?;
            for arg in argv {
                command.push(' ');
                command.push_str(
                    &quote_windows_arg(
                        arg.to_str()
                            .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidInput))?,
                    )
                    .map_err(io::Error::other)?,
                );
            }
            let mut command = wide(&command)?;
            let exe = wide_path(&exe)?;
            let cwd_w = wide_path(cwd)?;
            unsafe {
                let token = restricted_token()?;
                let job = Handle::new(CreateJobObjectW(std::ptr::null(), std::ptr::null()))?;
                let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = zeroed();
                limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                check(SetInformationJobObject(
                    job.0,
                    JobObjectExtendedLimitInformation,
                    (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                    size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                ))?;
                // Declared first so it is released after every temporary inheritable handle.
                let _spawn_guard = spawn_guard()?;
                let (stdin, child_in) = anonymous(true)?;
                let (stdout, child_out) = anonymous(false)?;
                let (stderr, child_err) = anonymous(false)?;
                let mut guard = std::ptr::null_mut();
                check(DuplicateHandle(
                    GetCurrentProcess(),
                    ownership.as_raw_handle(),
                    GetCurrentProcess(),
                    &mut guard,
                    0,
                    1,
                    DUPLICATE_SAME_ACCESS,
                ))?;
                let guard = Handle::new(guard)?;
                let policy = SpawnPolicy::new(
                    job.0 as usize,
                    vec![
                        child_in.0 as usize,
                        child_out.0 as usize,
                        child_err.0 as usize,
                        guard.0 as usize,
                    ],
                )?;
                let attrs = Attributes::new(&policy)?;
                let mut startup: STARTUPINFOEXW = zeroed();
                startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
                startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
                startup.StartupInfo.hStdInput = child_in.0;
                startup.StartupInfo.hStdOutput = child_out.0;
                startup.StartupInfo.hStdError = child_err.0;
                startup.lpAttributeList = attrs.ptr;
                let mut windows = [0u16; 32768];
                let n = GetWindowsDirectoryW(windows.as_mut_ptr(), windows.len() as u32) as usize;
                if n == 0 || n >= windows.len() {
                    return Err(io::Error::last_os_error());
                }
                let windows = String::from_utf16(&windows[..n]).map_err(io::Error::other)?;
                let mut env = Vec::new();
                for row in [
                    format!(
                        "PATH={};{windows}\\System32",
                        inputs.paths.runtime.join("bin").display()
                    ),
                    format!("SYSTEMROOT={windows}"),
                    format!("TEMP={}", cwd.display()),
                    format!("TMP={}", cwd.display()),
                ] {
                    env.extend(wide(&row)?)
                }
                env.push(0);
                let mut info: PROCESS_INFORMATION = zeroed();
                // No AssignProcessToJobObject fallback exists: JOB_LIST is part of creation.
                check(CreateProcessAsUserW(
                    token.0,
                    exe.as_ptr(),
                    command.as_mut_ptr(),
                    std::ptr::null(),
                    std::ptr::null(),
                    1,
                    policy.flags(),
                    env.as_ptr().cast(),
                    cwd_w.as_ptr(),
                    &startup.StartupInfo,
                    &mut info,
                ))?;
                let process = Self {
                    resources: Some(Resources {
                        process: Arc::new(Handle::new(info.hProcess)?),
                        thread: Handle::new(info.hThread)?,
                        job,
                        stdio: Some((stdin, stdout, stderr)),
                        _ownership: ownership,
                        _inputs: inputs,
                        _cwd: cwd_file,
                    }),
                };
                // Exercise the same postcondition/cleanup gate as the injected tests.
                create_verified(
                    &policy,
                    |_| Ok(process),
                    |p| {
                        let mut inside = 0;
                        check(IsProcessInJob(
                            p.resources().process.0,
                            p.resources().job.0,
                            &mut inside,
                        ))?;
                        Ok(inside != 0)
                    },
                    |p| p.abort_suspended(),
                )
            }
        }
        fn abort_suspended(&self) -> io::Result<()> {
            // Before the membership postcondition, only the retained process is authority.
            // It has never resumed, so there cannot be descendants to stop here.
            if !self.exited() {
                unsafe {
                    check(TerminateProcess(self.resources().process.0, 1))?;
                }
            }
            self.wait_exit(Duration::from_secs(45))
        }
        pub fn resume(&self) -> io::Result<()> {
            unsafe {
                if ResumeThread(self.resources().thread.0) == u32::MAX {
                    Err(io::Error::last_os_error())
                } else {
                    Ok(())
                }
            }
        }
        pub fn take_stdio(&mut self) -> io::Result<(File, File, File)> {
            self.resources
                .as_mut()
                .expect("live process capability")
                .stdio
                .take()
                .ok_or_else(|| io::Error::from(io::ErrorKind::BrokenPipe))
        }
        pub fn process_handle(&self) -> Arc<Handle> {
            self.resources().process.clone()
        }
        pub fn exited(&self) -> bool {
            unsafe { WaitForSingleObject(self.resources().process.0, 0) == WAIT_OBJECT_0 }
        }
        pub fn wait_exit(&self, timeout: Duration) -> io::Result<()> {
            unsafe {
                match WaitForSingleObject(
                    self.resources().process.0,
                    timeout.as_millis().min(u32::MAX as u128 - 1) as u32,
                ) {
                    WAIT_OBJECT_0 => Ok(()),
                    WAIT_TIMEOUT => Err(io::Error::from(io::ErrorKind::TimedOut)),
                    _ => Err(io::Error::last_os_error()),
                }
            }
        }
        pub fn abort(&self) -> io::Result<()> {
            if self.exited() {
                return Ok(());
            }
            unsafe {
                check(TerminateJobObject(self.resources().job.0, 1))?;
            }
            self.wait_exit(Duration::from_secs(45))
        }
    }
    impl Drop for OwnedQemuProcess {
        fn drop(&mut self) {
            let observed_stop = self.abort().is_ok();
            if let Some(resources) = self.resources.take() {
                release_bundle(resources, observed_stop);
            }
        }
    }
}

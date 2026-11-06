//! Guard-only coordination and off-process, deadline-bounded metadata observation.
use super::*;
use crate::guard_probe::GuardProbe;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use windows_sys::Win32::Security::{Authorization::*, *};
use windows_sys::Win32::System::SystemInformation::GetTickCount64;

struct Local(*mut core::ffi::c_void);
impl Drop for Local {
    fn drop(&mut self) {
        unsafe {
            LocalFree(self.0);
        }
    }
}
pub(super) struct GuardEvent {
    pub name: String,
    handle: OwnedHandle,
}
impl GuardEvent {
    pub fn create(sid: &str) -> io::Result<Self> {
        let name = format!(
            "Local\\bridge-guard-{}",
            bridge_runtime_protocol::Uuid::new_v4().simple()
        );
        Self::create_named(sid, name)
    }
    fn create_named(sid: &str, name: String) -> io::Result<Self> {
        let sddl = wide(&format!(
            "O:{sid}D:P(A;;0x1f0003;;;{sid})(A;;0x1f0003;;;SY)"
        ));
        let mut descriptor = std::ptr::null_mut();
        unsafe {
            win(ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                1,
                &mut descriptor,
                std::ptr::null_mut(),
            ))?;
        }
        let descriptor = Local(descriptor);
        let attributes = SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor.0,
            bInheritHandle: 0,
        };
        let raw = unsafe {
            SetLastError(0);
            CreateEventW(&attributes, 1, 0, wide(&name).as_ptr())
        };
        if raw.is_null() {
            return Err(io::Error::last_os_error());
        }
        let existed = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
        let handle = unsafe { OwnedHandle::from_raw_handle(raw) };
        if existed {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "guard event collision",
            ));
        }
        check_event_security(raw, sid)?;
        Ok(Self { name, handle })
    }
    pub fn signal(&self) -> io::Result<()> {
        unsafe { win(SetEvent(self.handle.as_raw_handle())) }
    }
}
fn sid(s: &str) -> io::Result<Local> {
    let mut raw = std::ptr::null_mut();
    unsafe {
        win(ConvertStringSidToSidW(wide(s).as_ptr(), &mut raw))?;
    }
    Ok(Local(raw))
}
fn check_event_security(raw: HANDLE, user: &str) -> io::Result<()> {
    unsafe {
        let mut flags = HANDLE_FLAG_INHERIT;
        win(GetHandleInformation(raw, &mut flags))?;
        check(flags & HANDLE_FLAG_INHERIT == 0)?;
        let (mut owner, mut acl, mut descriptor) = (
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        );
        let error = GetSecurityInfo(
            raw,
            SE_KERNEL_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            &mut owner,
            std::ptr::null_mut(),
            &mut acl,
            std::ptr::null_mut(),
            &mut descriptor,
        );
        if error != 0 {
            return Err(io::Error::from_raw_os_error(error as i32));
        }
        let _descriptor = Local(descriptor);
        check(!owner.is_null() && !acl.is_null())?;
        let user = sid(user)?;
        let system = sid("S-1-5-18")?;
        check(EqualSid(owner, user.0) != 0)?;
        let (mut control, mut revision) = (0, 0);
        win(GetSecurityDescriptorControl(
            descriptor,
            &mut control,
            &mut revision,
        ))?;
        check(control & SE_DACL_PROTECTED != 0 && (*acl).AceCount == 2)?;
        let (mut users, mut systems) = (0, 0);
        for index in 0..2 {
            let mut raw_ace = std::ptr::null_mut();
            win(GetAce(acl, index, &mut raw_ace))?;
            check(!raw_ace.is_null())?;
            let header = &*raw_ace.cast::<ACE_HEADER>();
            check(header.AceType == 0 && header.AceFlags == 0 && header.AceSize >= 16)?;
            let ace = &*raw_ace.cast::<ACCESS_ALLOWED_ACE>();
            check(ace.Mask == EVENT_ALL_ACCESS)?;
            let member = raw_ace
                .cast::<u8>()
                .add(std::mem::offset_of!(ACCESS_ALLOWED_ACE, SidStart))
                .cast();
            check(
                IsValidSid(member) != 0
                    && GetLengthSid(member) as usize <= ace.Header.AceSize as usize - 8,
            )?;
            if EqualSid(member, user.0) != 0 {
                users += 1;
            } else if EqualSid(member, system.0) != 0 {
                systems += 1;
            } else {
                return Err(io::ErrorKind::PermissionDenied.into());
            }
        }
        check(users == 1 && systems == 1)
    }
}
pub(super) fn wait_for_close(probe: &GuardProbe, event: OwnedHandle) -> io::Result<()> {
    match unsafe { WaitForSingleObject(event.as_raw_handle(), probe.remaining(GetTickCount64())?) }
    {
        WAIT_OBJECT_0 => Ok(()),
        WAIT_TIMEOUT => Err(io::ErrorKind::TimedOut.into()),
        WAIT_FAILED => Err(io::Error::last_os_error()),
        _ => Err(io::ErrorKind::InvalidData.into()),
    }
}
pub(super) fn open(probe: &GuardProbe) -> io::Result<OwnedHandle> {
    let raw = unsafe { OpenEventW(0x00100000, 0, wide(&probe.event).as_ptr()) };
    if raw.is_null() {
        return Err(io::Error::last_os_error());
    }
    let handle = unsafe { OwnedHandle::from_raw_handle(raw) };
    let mut flags = HANDLE_FLAG_INHERIT;
    unsafe {
        win(GetHandleInformation(raw, &mut flags))?;
    }
    check(flags & HANDLE_FLAG_INHERIT == 0)?;
    Ok(handle)
}
pub(super) fn snapshot(
    worker: HANDLE,
    guard: &runtime::identity::FileIdentity,
    canary: &runtime::identity::FileIdentity,
    deadline: Instant,
) -> io::Result<Value> {
    let mut raw = std::ptr::null_mut();
    unsafe {
        win(DuplicateHandle(
            GetCurrentProcess(),
            worker,
            GetCurrentProcess(),
            &mut raw,
            0,
            0,
            DUPLICATE_SAME_ACCESS,
        ))?;
    }
    let target = unsafe { OwnedHandle::from_raw_handle(raw) };
    let (guard, canary) = (guard.clone(), canary.clone());
    let (send, receive) = mpsc::sync_channel(1);
    // The observer owns target, buffers and all duplicates, even if the caller's
    // deadline expires. On timeout the controller fails/exits, never joins it.
    std::thread::spawn(move || {
        let _ = send.send(handle_snapshot(target.as_raw_handle(), &guard, &canary));
    });
    let result = receive
        .recv_timeout(deadline.saturating_duration_since(Instant::now()))
        .map_err(|_| {
            io::Error::new(io::ErrorKind::TimedOut, "guard snapshot deadline exceeded")
        })??;
    check(Instant::now() < deadline)?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pending_pipe_reader_fixture() {
        if std::env::var_os("BRIDGE_GUARD_PIPE_FIXTURE").is_some() {
            println!("PIPE-READY");
            std::io::stdout().flush().unwrap();
            let mut byte = [0];
            let _ = std::io::stdin().read_exact(&mut byte);
        }
    }
    #[test]
    fn pending_pipe_read_cannot_extend_the_snapshot_result_deadline() {
        use std::{
            os::windows::process::CommandExt,
            process::{Command, Stdio},
        };
        let root = tempfile::tempdir().unwrap();
        let one = fs::File::create(root.path().join("guard")).unwrap();
        let two = fs::File::create(root.path().join("canary")).unwrap();
        let guard = runtime::identity::file_identity(&one).unwrap();
        let canary = runtime::identity::file_identity(&two).unwrap();
        let gate = runtime::spawn_guard::blocking_guard();
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "native::guard_case::tests::pending_pipe_reader_fixture",
                "--nocapture",
            ])
            .env("BRIDGE_GUARD_PIPE_FIXTURE", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .unwrap();
        drop(gate);
        let output = child.stdout.take().unwrap();
        let (ready_send, ready_receive) = mpsc::sync_channel(1);
        std::thread::spawn(move || {
            for line in BufReader::new(output).lines().take(16) {
                if line.is_ok_and(|s| s.contains("PIPE-READY")) {
                    let _ = ready_send.send(());
                    break;
                }
            }
        });
        let ready = ready_receive.recv_timeout(Duration::from_secs(5));
        let start = Instant::now();
        // Either complete metadata or a strict metadata error is acceptable here;
        // an observer blocked in a synchronous kernel query cannot delay failure.
        let _ = snapshot(
            child.as_raw_handle(),
            &guard,
            &canary,
            start + Duration::from_millis(100),
        );
        let elapsed = start.elapsed();
        if child.try_wait().unwrap().is_none() {
            child.kill().unwrap();
        }
        child.wait().unwrap();
        assert!(ready.is_ok(), "pending-read fixture reached its read");
        assert!(elapsed < Duration::from_secs(2));
    }
    #[test]
    fn guard_event_is_private_noninherited_and_signals_once() {
        let (_, user) = controller::caller().unwrap();
        let event = GuardEvent::create(&user).unwrap();
        assert!(GuardEvent::create_named(&user, event.name.clone()).is_err());
        let now = unsafe { GetTickCount64() };
        let probe = GuardProbe::parse(&event.name, &(now + 1000).to_string(), now).unwrap();
        let worker = open(&probe).unwrap();
        assert_eq!(
            unsafe { WaitForSingleObject(worker.as_raw_handle(), 0) },
            WAIT_TIMEOUT
        );
        event.signal().unwrap();
        wait_for_close(&probe, worker).unwrap();
    }
}

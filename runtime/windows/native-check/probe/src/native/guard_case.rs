//! Direct guard-close and sharing proof; incidental handle metadata is diagnostic only.
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
            CreateEventW(&attributes, 0, 0, wide(&name).as_ptr())
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
pub(super) fn wait_for_signal(probe: &GuardProbe, event: &OwnedHandle) -> io::Result<()> {
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

/// Best-effort description of a parent-owned handle; never supplies a proof predicate.
pub(super) fn describe_parent_handle(raw: HANDLE, guard: HANDLE) -> Value {
    use windows_sys::Wdk::Foundation::{NtQueryObject, ObjectTypeInformation};
    use windows_sys::Win32::System::{
        Console::*, WindowsProgramming::PUBLIC_OBJECT_TYPE_INFORMATION,
    };
    unsafe {
        let mut flags = 0;
        let info = GetHandleInformation(raw, &mut flags);
        let info_error = if info == 0 {
            Some(GetLastError())
        } else {
            None
        };
        // Query only the object TYPE, never its name or synchronous file state.
        let mut storage = [0usize; 1024];
        let mut length = 0;
        let status = NtQueryObject(
            raw,
            ObjectTypeInformation,
            storage.as_mut_ptr().cast(),
            std::mem::size_of_val(&storage) as u32,
            &mut length,
        );
        let mut kind = None;
        if status >= 0 {
            let header = &*storage.as_ptr().cast::<PUBLIC_OBJECT_TYPE_INFORMATION>();
            let start = header.TypeName.Buffer as usize;
            let bytes = header.TypeName.Length as usize;
            let base = storage.as_ptr() as usize;
            if bytes > 0
                && bytes.is_multiple_of(2)
                && start.is_multiple_of(2)
                && start >= base
                && start
                    .checked_add(bytes)
                    .is_some_and(|end| end <= base + std::mem::size_of_val(&storage))
            {
                kind = String::from_utf16(std::slice::from_raw_parts(
                    header.TypeName.Buffer,
                    bytes / 2,
                ))
                .ok();
            }
        }
        let standards = [
            ("stdin", STD_INPUT_HANDLE),
            ("stdout", STD_OUTPUT_HANDLE),
            ("stderr", STD_ERROR_HANDLE),
        ]
        .map(|(name, which)| {
            let handle = GetStdHandle(which);
            json!({"name":name, "handle":format!("{:#x}",handle as usize),"matches":handle==raw})
        });
        json!({"handle":format!("{:#x}",raw as usize),"matchesGuard":raw==guard,
            "standardHandles":standards,"inherit":if info!=0 {Some(flags & HANDLE_FLAG_INHERIT !=0)} else {None},
            "handleInformationError":info_error,"objectType":kind,"objectTypeStatus":format!("{:#x}",status),
            "objectTypeReturnedBytes":length,"diagnosticOnly":true})
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn known_handle_close_releases_its_sharing_lock() {
        use std::os::windows::{fs::OpenOptionsExt, io::IntoRawHandle};
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("guard");
        let file = fs::OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .share_mode(0)
            .open(&path)
            .unwrap();
        assert_eq!(
            fs::OpenOptions::new()
                .read(true)
                .open(&path)
                .unwrap_err()
                .raw_os_error(),
            Some(32)
        );
        let raw = file.into_raw_handle();
        unsafe {
            win(CloseHandle(raw)).unwrap();
        }
        assert!(fs::OpenOptions::new().read(true).open(&path).is_ok());
    }
    #[test]
    fn unavailable_diagnostic_metadata_is_not_a_proof_failure() {
        let value = describe_parent_handle(std::ptr::null_mut(), std::ptr::null_mut());
        assert_eq!(value["diagnosticOnly"], true);
        assert!(value["objectType"].is_null());
        assert!(value["handleInformationError"].as_u64().is_some());
    }
    #[test]
    fn guard_event_is_private_noninherited_and_signals_once() {
        let (_, user) = controller::caller().unwrap();
        let event = GuardEvent::create(&user).unwrap();
        assert!(GuardEvent::create_named(&user, event.name.clone()).is_err());
        let now = unsafe { GetTickCount64() };
        let probe = GuardProbe::parse(&event.name, &(now + 1000).to_string(), now).unwrap();
        let worker = open(&probe).unwrap();
        for _ in 0..2 {
            assert_eq!(
                unsafe { WaitForSingleObject(worker.as_raw_handle(), 0) },
                WAIT_TIMEOUT
            );
            event.signal().unwrap();
            wait_for_signal(&probe, &worker).unwrap();
        }
    }
}

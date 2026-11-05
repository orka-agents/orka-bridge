//! Win32 security capabilities. None of these routines changes host policy.
use std::{
    fs::{File, OpenOptions},
    io,
    mem::size_of,
    os::windows::{
        ffi::OsStrExt,
        fs::OpenOptionsExt,
        io::{FromRawHandle, RawHandle},
    },
    path::Path,
};
use windows_sys::Win32::{
    Foundation::*,
    Security::{Authorization::*, *},
    System::Threading::{GetCurrentProcess, OpenProcessToken},
};

pub struct Handle(pub(super) HANDLE);
// Kernel handles support cross-thread use; ownership remains unique and Drop closes once.
unsafe impl Send for Handle {}
unsafe impl Sync for Handle {}
impl Handle {
    pub(super) fn new(raw: HANDLE) -> io::Result<Self> {
        if raw.is_null() || raw == INVALID_HANDLE_VALUE {
            Err(io::Error::last_os_error())
        } else {
            Ok(Self(raw))
        }
    }
    pub(super) fn into_file(self) -> File {
        let raw = self.0;
        std::mem::forget(self);
        unsafe { File::from_raw_handle(raw as RawHandle) }
    }
    pub fn raw(&self) -> HANDLE {
        self.0
    }
}
impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}
struct Local(*mut core::ffi::c_void);
impl Drop for Local {
    fn drop(&mut self) {
        unsafe {
            LocalFree(self.0);
        }
    }
}
pub(super) fn check(ok: i32) -> io::Result<()> {
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}
pub(super) fn wide(s: &str) -> io::Result<Vec<u16>> {
    if s.contains('\0') {
        return Err(io::Error::from(io::ErrorKind::InvalidInput));
    }
    Ok(s.encode_utf16().chain(Some(0)).collect())
}
pub(super) fn wide_path(p: &Path) -> io::Result<Vec<u16>> {
    if !p.is_absolute() {
        return Err(io::Error::from(io::ErrorKind::InvalidInput));
    }
    let mut s: Vec<u16> = p.as_os_str().encode_wide().collect();
    if s.contains(&0) {
        return Err(io::Error::from(io::ErrorKind::InvalidInput));
    }
    s.push(0);
    Ok(s)
}
unsafe fn user(token: HANDLE) -> io::Result<Vec<usize>> {
    let mut n = 0;
    GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &mut n);
    if n < size_of::<TOKEN_USER>() as u32 {
        return Err(io::Error::last_os_error());
    }
    let mut b = vec![0usize; (n as usize).div_ceil(size_of::<usize>())];
    check(GetTokenInformation(
        token,
        TokenUser,
        b.as_mut_ptr().cast(),
        n,
        &mut n,
    ))?;
    Ok(b)
}
unsafe fn current_token(access: TOKEN_ACCESS_MASK) -> io::Result<Handle> {
    let mut token = std::ptr::null_mut();
    check(OpenProcessToken(GetCurrentProcess(), access, &mut token))?;
    Handle::new(token)
}
pub(super) fn restricted_token() -> io::Result<Handle> {
    unsafe {
        let original = current_token(
            TOKEN_QUERY | TOKEN_DUPLICATE | TOKEN_ASSIGN_PRIMARY | TOKEN_ADJUST_DEFAULT,
        )?;
        let u = user(original.0)?;
        let sid = (*(u.as_ptr().cast::<TOKEN_USER>())).User.Sid;
        let mut system = std::ptr::null_mut();
        check(ConvertStringSidToSidW(
            wide("S-1-5-18")?.as_ptr(),
            &mut system,
        ))?;
        let system = Local(system);
        let restricted = [
            SID_AND_ATTRIBUTES {
                Sid: sid,
                Attributes: 0,
            },
            SID_AND_ATTRIBUTES {
                Sid: system.0,
                Attributes: 0,
            },
        ];
        let mut token = std::ptr::null_mut();
        check(CreateRestrictedToken(
            original.0,
            DISABLE_MAX_PRIVILEGE,
            0,
            std::ptr::null(),
            0,
            std::ptr::null(),
            2,
            restricted.as_ptr(),
            &mut token,
        ))?;
        let token = Handle::new(token)?;
        let mut string = std::ptr::null_mut();
        check(ConvertSidToStringSidW(sid, &mut string))?;
        let text = Local(string.cast());
        let mut len = 0;
        while *string.add(len) != 0 {
            len += 1
        }
        let sid = String::from_utf16(std::slice::from_raw_parts(string, len))
            .map_err(io::Error::other)?;
        let mut sd = std::ptr::null_mut();
        check(ConvertStringSecurityDescriptorToSecurityDescriptorW(
            wide(&format!("D:P(A;;GA;;;SY)(A;;GA;;;{sid})"))?.as_ptr(),
            1,
            &mut sd,
            std::ptr::null_mut(),
        ))?;
        let sd = Local(sd);
        let (mut present, mut defaulted) = (0, 0);
        let mut acl = std::ptr::null_mut();
        check(GetSecurityDescriptorDacl(
            sd.0,
            &mut present,
            &mut acl,
            &mut defaulted,
        ))?;
        if present == 0 || acl.is_null() {
            return Err(io::Error::from(io::ErrorKind::PermissionDenied));
        }
        let dacl = TOKEN_DEFAULT_DACL { DefaultDacl: acl };
        check(SetTokenInformation(
            token.0,
            TokenDefaultDacl,
            (&dacl as *const TOKEN_DEFAULT_DACL).cast(),
            size_of::<TOKEN_DEFAULT_DACL>() as u32,
        ))?;
        drop(text);
        Ok(token)
    }
}
/// Require actual kernel object security; no post-connection ACL rewrite.
pub(super) fn private_object(handle: HANDLE, require_protected: bool) -> io::Result<()> {
    unsafe {
        let (mut owner, mut acl, mut sd) = (
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        );
        let error = GetSecurityInfo(
            handle,
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            &mut owner,
            std::ptr::null_mut(),
            &mut acl,
            std::ptr::null_mut(),
            &mut sd,
        );
        if error != 0 {
            return Err(io::Error::from_raw_os_error(error as i32));
        }
        let sd = Local(sd);
        let token = current_token(TOKEN_QUERY)?;
        let u = user(token.0)?;
        let sid = (*(u.as_ptr().cast::<TOKEN_USER>())).User.Sid;
        let mut system = std::ptr::null_mut();
        check(ConvertStringSidToSidW(
            wide("S-1-5-18")?.as_ptr(),
            &mut system,
        ))?;
        let system = Local(system);
        let (mut control, mut revision) = (0, 0);
        check(GetSecurityDescriptorControl(
            sd.0,
            &mut control,
            &mut revision,
        ))?;
        if EqualSid(owner, sid) == 0
            || acl.is_null()
            || (require_protected && control & SE_DACL_PROTECTED == 0)
            || (*acl).AceCount != 2
        {
            return Err(io::Error::from(io::ErrorKind::PermissionDenied));
        }
        let (mut has_user, mut has_system) = (false, false);
        for i in 0..2 {
            let mut ace = std::ptr::null_mut();
            check(GetAce(acl, i, &mut ace))?;
            let header = &*ace.cast::<ACE_HEADER>();
            if header.AceType != 0 || header.AceSize < 16 {
                return Err(io::Error::from(io::ErrorKind::PermissionDenied));
            }
            let a = &*ace.cast::<ACCESS_ALLOWED_ACE>();
            if a.Mask & GENERIC_ALL == 0 && a.Mask & 0x1f01ff != 0x1f01ff {
                return Err(io::Error::from(io::ErrorKind::PermissionDenied));
            }
            let s = std::ptr::addr_of!(a.SidStart) as PSID;
            let sid_bytes = s.cast::<u8>();
            let count = *sid_bytes.add(1) as usize;
            if *sid_bytes != 1 || count > 15 || 8 + 4 * count > header.AceSize as usize - 8 {
                return Err(io::Error::from(io::ErrorKind::PermissionDenied));
            }
            if EqualSid(s, sid) != 0 {
                has_user = true
            } else if EqualSid(s, system.0) != 0 {
                has_system = true
            } else {
                return Err(io::Error::from(io::ErrorKind::PermissionDenied));
            }
        }
        if !has_user || !has_system {
            return Err(io::Error::from(io::ErrorKind::PermissionDenied));
        }
        Ok(())
    }
}
pub(super) fn private_directory(path: &Path) -> io::Result<File> {
    use std::os::windows::fs::MetadataExt;
    let m = std::fs::symlink_metadata(path)?;
    if !m.is_dir() || m.file_attributes() & 0x400 != 0 {
        return Err(io::Error::from(io::ErrorKind::PermissionDenied));
    }
    let f = OpenOptions::new()
        .read(true)
        .share_mode(1)
        .custom_flags(0x02200000)
        .open(path)?;
    use std::os::windows::io::AsRawHandle;
    private_object(f.as_raw_handle(), false)?;
    Ok(f)
}

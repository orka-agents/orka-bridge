//! One local AF_UNIX guest stream. QMP remains on its separate standard I/O.
use super::{job::OwnedQemuProcess, launch::control_path, security};
use bridge_runtime_protocol::Uuid;
use std::{
    fs::{File, OpenOptions},
    io,
    mem::{size_of, zeroed},
    os::windows::{
        fs::OpenOptionsExt,
        io::{AsRawHandle, FromRawSocket, OwnedSocket},
    },
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, OnceLock,
    },
    time::{Duration, Instant},
};
use windows_sys::Win32::{Foundation::*, Networking::WinSock::*, System::Threading::*};

fn wsa_error() -> io::Error {
    io::Error::from_raw_os_error(unsafe { WSAGetLastError() })
}
struct Network;
impl Network {
    fn start() -> io::Result<Self> {
        let mut data = unsafe { zeroed() };
        let error = unsafe { WSAStartup(0x0202, &mut data) };
        if error != 0 {
            return Err(io::Error::from_raw_os_error(error));
        }
        Ok(Self)
    }
}
impl Drop for Network {
    fn drop(&mut self) {
        unsafe {
            WSACleanup();
        }
    }
}

struct Channel {
    // Register even a possibly connected socket before any later validation can fail.
    // All attempts are bounded by the single establishment deadline and stay owned.
    sockets: Mutex<Vec<OwnedSocket>>,
    metadata: Mutex<Option<File>>,
    active: OnceLock<SOCKET>,
    peer: OnceLock<u32>,
    terminal: AtomicBool,
    _network: Mutex<Option<Network>>,
}
impl Channel {
    fn socket(&self) -> io::Result<SOCKET> {
        // Namespace ownership needs no Winsock initialization. Keep its machinery
        // absent until connection acquisition, then retain it with the sockets.
        let mut network = self
            ._network
            .lock()
            .map_err(|_| io::Error::other("AF_UNIX network owner poisoned"))?;
        if network.is_none() {
            *network = Some(Network::start()?);
        }
        drop(network);
        let raw = unsafe {
            WSASocketW(
                AF_UNIX as i32,
                SOCK_STREAM,
                0,
                std::ptr::null(),
                0,
                WSA_FLAG_NO_HANDLE_INHERIT,
            )
        };
        if raw == INVALID_SOCKET {
            return Err(wsa_error());
        }
        let owned = unsafe { OwnedSocket::from_raw_socket(raw as _) };
        self.sockets
            .lock()
            .map_err(|_| io::Error::other("AF_UNIX socket owner poisoned"))?
            .push(owned);
        let mut nonblocking = 1;
        if unsafe { ioctlsocket(raw, FIONBIO, &mut nonblocking) } != 0 {
            return Err(wsa_error());
        }
        Ok(raw)
    }
    fn raw(&self) -> io::Result<SOCKET> {
        self.active
            .get()
            .copied()
            .ok_or_else(|| io::Error::from(io::ErrorKind::NotConnected))
    }
}

pub struct ControlEndpoint {
    path: PathBuf,
    _parent: File,
    directory: File,
    channel: Arc<Channel>,
}
impl ControlEndpoint {
    pub fn create(parent: &Path) -> io::Result<Self> {
        Self::with_nonce(parent, Uuid::new_v4())
    }
    fn with_nonce(parent: &Path, nonce: Uuid) -> io::Result<Self> {
        let path = control_path(parent, nonce)?;
        let parent_handle = security::private_af_directory(parent)?;
        // CreateDirectory fails on an existing child. Never repair or reuse stale files.
        let directory = security::create_af_directory(path.parent().expect("endpoint parent"))?;
        let channel = Arc::new(Channel {
            sockets: Mutex::new(Vec::new()),
            metadata: Mutex::new(None),
            active: OnceLock::new(),
            peer: OnceLock::new(),
            terminal: AtomicBool::new(false),
            _network: Mutex::new(None),
        });
        Ok(Self {
            path,
            _parent: parent_handle,
            directory,
            channel,
        })
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub(super) fn connect<'a>(
        &self,
        owner: &'a OwnedQemuProcess,
    ) -> io::Result<ControlConnection<'a>> {
        let until = Instant::now() + Duration::from_secs(30);
        security::private_af_object(self.directory.as_raw_handle(), true)?;
        loop {
            live(owner)?;
            match OpenOptions::new()
                .read(true)
                .share_mode(3)
                .custom_flags(0x02200000)
                .open(&self.path)
            {
                Ok(file) => {
                    let mut metadata = self
                        .channel
                        .metadata
                        .lock()
                        .map_err(|_| io::Error::other("AF_UNIX metadata owner poisoned"))?;
                    *metadata = Some(file);
                    security::private_af_object(
                        metadata
                            .as_ref()
                            .expect("registered metadata")
                            .as_raw_handle(),
                        false,
                    )?;
                    break;
                }
                Err(error) if matches!(error.raw_os_error(), Some(2 | 3)) => wait_retry(until)?,
                Err(error) => return Err(error),
            }
        }
        let address = address(&self.path)?;
        let raw = loop {
            live(owner)?;
            let socket = self.channel.socket()?;
            match establish(socket, &address, until, owner) {
                Ok(()) => break socket,
                Err(error) if error.raw_os_error() == Some(WSAECONNREFUSED) => wait_retry(until)?,
                Err(error) => return Err(error),
            }
        };
        let pid = peer_pid(raw)?;
        let process = owner.process_handle();
        if pid != unsafe { GetProcessId(process.raw()) } {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "AF_UNIX peer is not the retained QEMU process",
            ));
        }
        live(owner)?;
        security::private_af_object(self.directory.as_raw_handle(), true)?;
        let metadata = self
            .channel
            .metadata
            .lock()
            .map_err(|_| io::Error::other("AF_UNIX metadata owner poisoned"))?;
        security::private_af_object(
            metadata
                .as_ref()
                .expect("registered metadata")
                .as_raw_handle(),
            false,
        )?;
        self.channel
            .active
            .set(raw)
            .map_err(|_| io::Error::other("AF_UNIX connection already acquired"))?;
        self.channel
            .peer
            .set(pid)
            .map_err(|_| io::Error::other("AF_UNIX peer already recorded"))?;
        Ok(ControlConnection {
            owner,
            channel: self.channel.clone(),
        })
    }
}

fn live(owner: &OwnedQemuProcess) -> io::Result<()> {
    let process = owner.process_handle();
    match unsafe { WaitForSingleObject(process.raw(), 0) } {
        WAIT_TIMEOUT => Ok(()),
        WAIT_OBJECT_0 => Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "owned QEMU exited",
        )),
        _ => Err(io::Error::last_os_error()),
    }
}
fn wait_retry(until: Instant) -> io::Result<()> {
    let remaining = until.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Err(io::Error::from(io::ErrorKind::TimedOut));
    }
    std::thread::sleep(remaining.min(Duration::from_millis(10)));
    Ok(())
}
fn address(path: &Path) -> io::Result<SOCKADDR_UN> {
    let path = super::launch::validate_control_path(path)?;
    let text = path.to_str().expect("validated UTF-8");
    let mut address: SOCKADDR_UN = unsafe { zeroed() };
    address.sun_family = AF_UNIX;
    for (out, byte) in address.sun_path.iter_mut().zip(text.bytes()) {
        *out = byte as i8;
    }
    Ok(address)
}
fn poll(socket: SOCKET, events: i16, until: Instant) -> io::Result<()> {
    let remaining = until.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Err(io::Error::from(io::ErrorKind::TimedOut));
    }
    let mut descriptor = WSAPOLLFD {
        fd: socket,
        events,
        revents: 0,
    };
    if unsafe {
        WSAPoll(
            &mut descriptor,
            1,
            remaining.as_millis().clamp(1, 10) as i32,
        )
    } < 0
    {
        return Err(wsa_error());
    }
    Ok(())
}
fn establish(
    socket: SOCKET,
    address: &SOCKADDR_UN,
    until: Instant,
    owner: &OwnedQemuProcess,
) -> io::Result<()> {
    if Instant::now() >= until {
        return Err(io::Error::from(io::ErrorKind::TimedOut));
    }
    let error = if unsafe {
        connect(
            socket,
            (address as *const SOCKADDR_UN).cast(),
            size_of::<SOCKADDR_UN>() as i32,
        )
    } == 0
    {
        return Ok(());
    } else {
        unsafe { WSAGetLastError() }
    };
    if !matches!(error, WSAEWOULDBLOCK | WSAEINPROGRESS) {
        return Err(io::Error::from_raw_os_error(error));
    }
    loop {
        live(owner)?;
        poll(socket, POLLWRNORM, until)?;
        let mut error = 0i32;
        let mut bytes = size_of::<i32>() as i32;
        if unsafe {
            getsockopt(
                socket,
                SOL_SOCKET,
                SO_ERROR,
                (&mut error as *mut i32).cast(),
                &mut bytes,
            )
        } != 0
        {
            return Err(wsa_error());
        }
        if bytes != size_of::<i32>() as i32 {
            return Err(io::Error::from(io::ErrorKind::InvalidData));
        }
        if error != 0 {
            return Err(io::Error::from_raw_os_error(error));
        }
        // A zero SO_ERROR before readiness is not proof of establishment.
        let mut descriptor = WSAPOLLFD {
            fd: socket,
            events: POLLWRNORM,
            revents: 0,
        };
        if unsafe { WSAPoll(&mut descriptor, 1, 0) } < 0 {
            return Err(wsa_error());
        }
        if descriptor.revents & POLLWRNORM != 0 {
            return Ok(());
        }
    }
}
fn peer_pid(socket: SOCKET) -> io::Result<u32> {
    let mut pid = 0u32;
    let mut bytes = 0;
    if unsafe {
        WSAIoctl(
            socket,
            SIO_AF_UNIX_GETPEERPID,
            std::ptr::null(),
            0,
            (&mut pid as *mut u32).cast(),
            size_of::<u32>() as u32,
            &mut bytes,
            std::ptr::null_mut(),
            None,
        )
    } != 0
    {
        return Err(wsa_error());
    }
    // The Windows provider can report zero bytes while writing the DWORD PID.
    if pid == 0 {
        return Err(io::Error::from(io::ErrorKind::PermissionDenied));
    }
    Ok(pid)
}

pub struct ControlConnection<'a> {
    owner: &'a OwnedQemuProcess,
    channel: Arc<Channel>,
}
impl ControlConnection<'_> {
    pub fn peer_pid(&self) -> u32 {
        *self.channel.peer.get().expect("validated peer")
    }
    fn failure(&self, error: io::Error) -> io::Error {
        self.channel.terminal.store(true, Ordering::Release);
        match self.owner.abort() {
            Ok(()) => error,
            Err(stop) => io::Error::other(format!(
                "AF_UNIX terminal failure: {error}; QEMU stop unobserved: {stop}"
            )),
        }
    }
    fn usable(&self) -> io::Result<SOCKET> {
        if self.channel.terminal.load(Ordering::Acquire) {
            return Err(io::Error::from(io::ErrorKind::BrokenPipe));
        }
        live(self.owner)?;
        self.channel.raw()
    }
    pub fn read(&self, output: &mut [u8], deadline: Instant) -> io::Result<usize> {
        if output.is_empty() {
            return Ok(0);
        }
        let until = deadline.min(Instant::now() + Duration::from_secs(120));
        let mut operation = || loop {
            let socket = self.usable()?;
            if Instant::now() >= until {
                return Err(io::Error::from(io::ErrorKind::TimedOut));
            }
            let count = unsafe {
                recv(
                    socket,
                    output.as_mut_ptr(),
                    output.len().min(i32::MAX as usize) as i32,
                    0,
                )
            };
            if count > 0 {
                return Ok(count as usize);
            }
            if count == 0 {
                return Err(io::Error::from(io::ErrorKind::UnexpectedEof));
            }
            let error = wsa_error();
            if error.raw_os_error() != Some(WSAEWOULDBLOCK) {
                return Err(error);
            }
            poll(socket, POLLRDNORM, until)?;
        };
        operation().map_err(|error| self.failure(error))
    }
    pub fn write_all(&self, input: &[u8], deadline: Instant) -> io::Result<()> {
        let until = deadline.min(Instant::now() + Duration::from_secs(30));
        let operation = || {
            let mut input = input;
            while !input.is_empty() {
                let socket = self.usable()?;
                if Instant::now() >= until {
                    return Err(io::Error::from(io::ErrorKind::TimedOut));
                }
                let count = unsafe {
                    send(
                        socket,
                        input.as_ptr(),
                        input.len().min(i32::MAX as usize) as i32,
                        0,
                    )
                };
                if count > 0 {
                    input = &input[count as usize..];
                    continue;
                }
                if count == 0 {
                    return Err(io::Error::from(io::ErrorKind::WriteZero));
                }
                let error = wsa_error();
                if error.raw_os_error() != Some(WSAEWOULDBLOCK) {
                    return Err(error);
                }
                poll(socket, POLLWRNORM, until)?;
            }
            Ok(())
        };
        operation().map_err(|error| self.failure(error))
    }
}
impl Drop for ControlConnection<'_> {
    fn drop(&mut self) {
        self.channel.terminal.store(true, Ordering::Release);
        // Request termination before releasing a live socket; the owner retains it
        // and all metadata for either observed release or whole-bundle quarantine.
        let _ = self.owner.abort();
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::Security::{Authorization::*, *};
    use windows_sys::Win32::Storage::FileSystem::{READ_CONTROL, WRITE_DAC};
    struct Local(*mut core::ffi::c_void);
    impl Drop for Local {
        fn drop(&mut self) {
            unsafe {
                LocalFree(self.0);
            }
        }
    }
    fn parent() -> (tempfile::TempDir, PathBuf, File) {
        let home = std::env::var_os("USERPROFILE").expect("native Windows profile");
        let root = tempfile::tempdir_in(home).unwrap();
        let parent = root.path().join("p");
        let retained = security::create_af_directory(&parent).unwrap();
        (root, parent, retained)
    }
    #[test]
    fn namespace_creation_does_not_start_winsock_before_a_socket_is_needed() {
        let (_root, parent, _retained) = parent();
        let endpoint = ControlEndpoint::create(&parent).unwrap();
        assert!(endpoint.channel._network.lock().unwrap().is_none());
        endpoint.channel.socket().unwrap();
        assert!(endpoint.channel._network.lock().unwrap().is_some());
    }
    #[test]
    fn endpoint_creation_is_unique_protected_and_never_reuses_stale_files() {
        let (_root, parent, _retained) = parent();
        let nonce = Uuid::new_v4();
        let first = ControlEndpoint::with_nonce(&parent, nonce).unwrap();
        let second = ControlEndpoint::create(&parent).unwrap();
        assert_ne!(first.path(), second.path());
        security::private_af_object(first.directory.as_raw_handle(), true).unwrap();
        std::fs::write(first.path(), b"retained stale marker").unwrap();
        let error = ControlEndpoint::with_nonce(&parent, nonce)
            .err()
            .expect("fresh create must fail");
        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(
            std::fs::read(first.path()).unwrap(),
            b"retained stale marker"
        );
    }
    unsafe fn set_dacl(file: &File, dacl: &str) {
        let mut owner = std::ptr::null_mut();
        let mut original = std::ptr::null_mut();
        assert_eq!(
            GetSecurityInfo(
                file.as_raw_handle(),
                SE_FILE_OBJECT,
                OWNER_SECURITY_INFORMATION,
                &mut owner,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut original
            ),
            0
        );
        let _original = Local(original);
        let owner = std::ptr::NonNull::new(owner).unwrap();
        let mut text = std::ptr::null_mut();
        security::check(ConvertSidToStringSidW(owner.as_ptr(), &mut text)).unwrap();
        let _text = Local(text.cast());
        let text = std::ptr::NonNull::new(text).unwrap().as_ptr();
        let mut count = 0;
        while *text.add(count) != 0 {
            count += 1;
        }
        let sid = String::from_utf16(std::slice::from_raw_parts(text, count)).unwrap();
        let mut descriptor = std::ptr::null_mut();
        security::check(ConvertStringSecurityDescriptorToSecurityDescriptorW(
            security::wide(&dacl.replace("USER", &sid))
                .unwrap()
                .as_ptr(),
            1,
            &mut descriptor,
            std::ptr::null_mut(),
        ))
        .unwrap();
        let _descriptor = Local(descriptor);
        let mut present = 0;
        let mut defaulted = 0;
        let mut acl = std::ptr::null_mut();
        security::check(GetSecurityDescriptorDacl(
            descriptor,
            &mut present,
            &mut acl,
            &mut defaulted,
        ))
        .unwrap();
        assert_ne!(present, 0);
        assert_eq!(
            SetSecurityInfo(
                file.as_raw_handle(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                acl,
                std::ptr::null_mut()
            ),
            0
        );
    }
    #[test]
    fn socket_metadata_rejects_wrong_flags_principals_and_reparse_type() {
        let (_root, parent, _retained) = parent();
        let endpoint = ControlEndpoint::create(&parent).unwrap();
        let socket = endpoint.channel.socket().unwrap();
        let address = address(endpoint.path()).unwrap();
        unsafe {
            assert_eq!(
                bind(
                    socket,
                    (&address as *const SOCKADDR_UN).cast(),
                    size_of::<SOCKADDR_UN>() as i32
                ),
                0
            );
            assert_eq!(listen(socket, 1), 0);
        }
        let file = OpenOptions::new()
            .access_mode(
                READ_CONTROL
                    | WRITE_DAC
                    | windows_sys::Win32::Storage::FileSystem::FILE_READ_ATTRIBUTES,
            )
            .share_mode(3)
            .custom_flags(0x02200000)
            .open(endpoint.path())
            .unwrap();
        security::private_af_object(file.as_raw_handle(), false).unwrap();
        for dacl in [
            "D:P(A;OICIIO;FA;;;USER)(A;OICIIO;FA;;;SY)",
            "D:P(A;;FA;;;USER)(A;;FA;;;WD)",
        ] {
            // Negative fault injection only. Production never repairs a created ACL.
            unsafe {
                set_dacl(&file, dacl);
            }
            assert!(security::private_af_object(file.as_raw_handle(), false).is_err());
        }
        let ordinary = parent.join("ordinary");
        std::fs::write(&ordinary, b"not a socket").unwrap();
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(0x02200000)
            .open(ordinary)
            .unwrap();
        assert!(security::private_af_object(file.as_raw_handle(), false).is_err());
    }
}

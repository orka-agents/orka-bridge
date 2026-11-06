//! One diagnostic experiment, not a production transport or containment PASS.
use super::*;
use std::os::windows::io::{AsRawSocket, FromRawSocket, OwnedSocket};
use windows_sys::Win32::Networking::WinSock::*;

fn startup() -> io::Result<()> {
    static INIT: std::sync::OnceLock<i32> = std::sync::OnceLock::new();
    let code = *INIT.get_or_init(|| unsafe {
        let mut data = zeroed();
        WSAStartup(0x202, &mut data)
    });
    if code == 0 {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(code))
    }
}
fn address(path: &std::path::Path) -> io::Result<SOCKADDR_UN> {
    let text = path
        .to_str()
        .ok_or_else(|| io::Error::other("socket path is not UTF-8"))?;
    crate::runtime::windows::launch::validate_control_path(path)?;
    let mut addr: SOCKADDR_UN = unsafe { zeroed() };
    addr.sun_family = AF_UNIX;
    for (out, byte) in addr.sun_path.iter_mut().zip(text.bytes()) {
        *out = byte as i8;
    }
    Ok(addr)
}
fn socket() -> io::Result<OwnedSocket> {
    startup()?;
    unsafe {
        let raw = WSASocketW(
            AF_UNIX as i32,
            SOCK_STREAM,
            0,
            std::ptr::null(),
            0,
            WSA_FLAG_OVERLAPPED | WSA_FLAG_NO_HANDLE_INHERIT,
        );
        if raw == INVALID_SOCKET {
            Err(io::Error::from_raw_os_error(WSAGetLastError()))
        } else {
            Ok(OwnedSocket::from_raw_socket(raw as _))
        }
    }
}
fn raw(socket: &OwnedSocket) -> SOCKET {
    socket.as_raw_socket() as SOCKET
}
fn server(path: &std::path::Path) -> io::Result<OwnedSocket> {
    let socket = socket()?;
    let addr = address(path)?;
    unsafe {
        if bind(
            raw(&socket),
            (&addr as *const SOCKADDR_UN).cast(),
            size_of::<SOCKADDR_UN>() as i32,
        ) != 0
            || listen(raw(&socket), 2) != 0
        {
            return Err(io::Error::from_raw_os_error(WSAGetLastError()));
        }
    }
    Ok(socket)
}
pub(super) fn control_server() -> io::Result<(PathBuf, OwnedSocket)> {
    let stage = std::env::current_exe()?
        .parent()
        .ok_or_else(|| io::Error::other("executable stage missing"))?
        .to_owned();
    let path = stage.join("af-control").join("c.sock");
    let listener = server(&path)?;
    Ok((path, listener))
}
pub(super) fn access_proof(
    endpoint: &std::path::Path,
    child: &OwnedQemuProcess,
) -> io::Result<Value> {
    let path = std::env::current_exe()?
        .parent()
        .ok_or_else(|| io::Error::other("executable stage missing"))?
        .join("af-unix-proof.json");
    let until = Instant::now() + Duration::from_secs(30);
    while !path.exists() {
        check(!child.exited() && Instant::now() < until)?;
        std::thread::sleep(Duration::from_millis(5));
    }
    check(fs::metadata(&path)?.len() <= 16384)?;
    let proof: Value = serde_json::from_slice(&fs::read(path)?).map_err(io::Error::other)?;
    let (_, sid) = controller::caller()?;
    check(
        proof["sourceHead"] == crate::SOURCE_HEAD
            && proof["endpoint"] == endpoint.to_str().unwrap()
            && proof["primarySid"] == sid
            && proof["otherSid"].as_str().is_some_and(|s| s != sid)
            && proof["otherElevated"] == false
            && proof["controlConnected"] == true
            && proof["controlPeerPid"] == unsafe { GetCurrentProcessId() }
            && proof["privateConnected"] == false
            && proof["privateError"] == WSAEACCES,
    )?;
    Ok(proof)
}
fn connect_socket(path: &std::path::Path) -> io::Result<OwnedSocket> {
    let socket = socket()?;
    let addr = address(path)?;
    unsafe {
        let mut nonblock = 1;
        if ioctlsocket(raw(&socket), FIONBIO, &mut nonblock) != 0 {
            return Err(io::Error::from_raw_os_error(WSAGetLastError()));
        }
        if windows_sys::Win32::Networking::WinSock::connect(
            raw(&socket),
            (&addr as *const SOCKADDR_UN).cast(),
            size_of::<SOCKADDR_UN>() as i32,
        ) == 0
        {
            return Ok(socket);
        }
        let error = WSAGetLastError();
        if error != WSAEWOULDBLOCK && error != WSAEINPROGRESS {
            return Err(io::Error::from_raw_os_error(error));
        }
        let until = Instant::now() + Duration::from_secs(30);
        loop {
            check(Instant::now() < until)?;
            let mut poll = WSAPOLLFD {
                fd: raw(&socket),
                events: POLLWRNORM,
                revents: 0,
            };
            let result = WSAPoll(&mut poll, 1, 50);
            if result == SOCKET_ERROR {
                return Err(io::Error::from_raw_os_error(WSAGetLastError()));
            }
            if result == 0 {
                continue;
            }
            let mut error = 0i32;
            let mut len = size_of::<i32>() as i32;
            if getsockopt(
                raw(&socket),
                SOL_SOCKET,
                SO_ERROR,
                (&mut error as *mut i32).cast(),
                &mut len,
            ) != 0
            {
                return Err(io::Error::from_raw_os_error(WSAGetLastError()));
            }
            if error != 0 {
                return Err(io::Error::from_raw_os_error(error));
            }
            return Ok(socket);
        }
    }
}
fn peer_pid(socket: &OwnedSocket) -> io::Result<u32> {
    unsafe {
        let mut pid = 0u32;
        let mut bytes = 0;
        if WSAIoctl(
            raw(socket),
            SIO_AF_UNIX_GETPEERPID,
            std::ptr::null(),
            0,
            (&mut pid as *mut u32).cast(),
            size_of::<u32>() as u32,
            &mut bytes,
            std::ptr::null_mut(),
            None,
        ) != 0
        {
            return Err(io::Error::from_raw_os_error(WSAGetLastError()));
        }
        // Windows can return zero in the byte-count field on a successful query.
        // Acceptance depends on the DWORD PID and the caller's owned-PID comparison.
        eprintln!("[AF-UNIX-PEER] pid={pid} returnedBytes={bytes}");
        check(pid != 0)?;
        Ok(pid)
    }
}
pub(super) fn client(path: &std::path::Path) -> io::Result<()> {
    let (caller, _) = controller::caller()?;
    let result = match connect_socket(path) {
        Ok(socket) => json!({"connected":true,"peerPid":peer_pid(&socket)?}),
        Err(error) => {
            json!({"connected":false,"errorCode":error.raw_os_error(),"error":error.to_string()})
        }
    };
    emit(json!({"sourceHead":crate::SOURCE_HEAD,"caller":caller,"path":path,"result":result}))
}
pub(super) fn descriptor(handle: HANDLE) -> io::Result<Value> {
    use windows_sys::Win32::Security::{Authorization::*, *};
    unsafe {
        let flags = OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION;
        let mut sd = std::ptr::null_mut();
        let code = GetSecurityInfo(
            handle,
            SE_FILE_OBJECT,
            flags,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut sd,
        );
        if code != 0 {
            return Err(io::Error::from_raw_os_error(code as i32));
        }
        if sd.is_null() {
            return Err(io::Error::other("empty socket descriptor"));
        }
        let mut control = 0;
        let mut revision = 0;
        let mut text = std::ptr::null_mut();
        let mut chars = 0;
        let result = if GetSecurityDescriptorControl(sd, &mut control, &mut revision) != 0
            && ConvertSecurityDescriptorToStringSecurityDescriptorW(
                sd, 1, flags, &mut text, &mut chars,
            ) != 0
            && !text.is_null()
            && chars > 0
            && chars <= 32768
        {
            Ok(
                json!({"control":format!("{control:#x}"),"sddl":String::from_utf16_lossy(std::slice::from_raw_parts(text, chars as usize)).trim_end_matches('\0')}),
            )
        } else {
            Err(io::Error::last_os_error())
        };
        if !text.is_null() {
            LocalFree(text.cast());
        }
        LocalFree(sd);
        result
    }
}
pub(super) fn checked_path(path: &std::path::Path, directory: bool) -> io::Result<(File, Value)> {
    use std::os::windows::fs::OpenOptionsExt;
    let flags = if directory {
        FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT
    } else {
        FILE_FLAG_OPEN_REPARSE_POINT
    };
    let file = fs::OpenOptions::new()
        .access_mode(READ_CONTROL | FILE_READ_ATTRIBUTES)
        .share_mode(
            FILE_SHARE_READ | FILE_SHARE_WRITE | if directory { 0 } else { FILE_SHARE_DELETE },
        )
        .custom_flags(flags)
        .open(path)?;
    let security = descriptor(file.as_raw_handle())?;
    eprintln!(
        "[AF-UNIX-ACL] {}",
        json!({"path":path,"directory":directory,"security":security})
    );
    runtime::windows::check_unix_path(file.as_raw_handle(), directory).map_err(|error| {
        io::Error::other(format!(
            "AF_UNIX actual ACL rejected at {}: {error}; {security}",
            path.display()
        ))
    })?;
    let mut tag: FILE_ATTRIBUTE_TAG_INFO = unsafe { zeroed() };
    unsafe {
        win(GetFileInformationByHandleEx(
            file.as_raw_handle(),
            FileAttributeTagInfo,
            (&mut tag as *mut FILE_ATTRIBUTE_TAG_INFO).cast(),
            size_of::<FILE_ATTRIBUTE_TAG_INFO>() as u32,
        ))?;
    }
    if directory {
        check(tag.FileAttributes & FILE_ATTRIBUTE_REPARSE_POINT == 0)?;
    } else {
        check(
            tag.FileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 && tag.ReparseTag == 0x80000023,
        )?;
    }
    Ok((
        file,
        json!({"path":path,"protectedDirectory":directory,"exactUserSystemAcl":true,"attributes":tag.FileAttributes,"reparseTag":format!("{:#x}",tag.ReparseTag),"security":security}),
    ))
}
pub(super) struct Trial {
    pub(super) endpoint: PathBuf,
    control: PathBuf,
    proof: PathBuf,
    _directory: File,
    directory_info: Value,
    _control_server: OwnedSocket,
}
impl Trial {
    pub(super) fn prepare(root: &std::path::Path) -> io::Result<Self> {
        let (_, sid) = controller::caller()?;
        let private = root.join("p");
        controller::private_root(&private, &sid)?;
        let (directory, directory_info) = checked_path(&private, true)?;
        let stage = std::env::current_exe()?.parent().unwrap().to_path_buf();
        let control = stage.join("af-control").join("c.sock");
        let control_server = server(&control)?;
        let endpoint = private.join("c.sock");
        let _ = address(&endpoint)?;
        Ok(Self {
            endpoint,
            control,
            proof: stage.join("af-unix-proof.json"),
            _directory: directory,
            directory_info,
            _control_server: control_server,
        })
    }
    pub(super) fn run(
        self,
        child: &OwnedQemuProcess,
        process: HANDLE,
        stdin: &mut impl Write,
        rx: &mpsc::Receiver<io::Result<Value>>,
        errors: &Arc<std::sync::Mutex<Vec<u8>>>,
    ) -> io::Result<Value> {
        let until = Instant::now() + Duration::from_secs(30);
        while fs::symlink_metadata(&self.endpoint).is_err() {
            if child.exited() {
                child.wait_exit(Duration::from_secs(45))?;
                std::thread::sleep(Duration::from_millis(50));
                let mut code = 0;
                unsafe {
                    win(GetExitCodeProcess(process, &mut code))?;
                }
                return Ok(
                    json!({"candidatePassed":false,"failure":"QEMU exited before creating AF_UNIX endpoint","qemuExitCode":code,"stderr":String::from_utf8_lossy(&errors.lock().unwrap()).into_owned()}),
                );
            }
            check(Instant::now() < until)?;
            std::thread::sleep(Duration::from_millis(5));
        }
        let (_socket_file, endpoint_info) = checked_path(&self.endpoint, false)?;
        emit(
            json!({"stage":"af-unix-ready","sourceHead":crate::SOURCE_HEAD,"endpoint":self.endpoint,"control":self.control,"workerPid":unsafe{GetCurrentProcessId()},"directory":self.directory_info,"socket":endpoint_info}),
        )?;
        let until = Instant::now() + Duration::from_secs(30);
        while !self.proof.exists() {
            check(!child.exited() && Instant::now() < until)?;
            std::thread::sleep(Duration::from_millis(10));
        }
        let proof: Value =
            serde_json::from_slice(&fs::read(&self.proof)?).map_err(io::Error::other)?;
        let (_, sid) = controller::caller()?;
        check(
            proof["sourceHead"] == crate::SOURCE_HEAD
                && proof["endpoint"] == self.endpoint.to_str().unwrap()
                && proof["primarySid"] == sid
                && proof["otherSid"].as_str().is_some_and(|s| s != sid)
                && proof["otherElevated"] == false,
        )?;
        check(
            proof["controlConnected"] == true
                && proof["controlPeerPid"] == unsafe { GetCurrentProcessId() }
                && proof["privateConnected"] == false
                && proof["privateError"] == WSAEACCES,
        )?;
        let socket = connect_socket(&self.endpoint)?;
        let pid = peer_pid(&socket)?;
        check(pid == unsafe { GetProcessId(process) })?;
        // Re-query actual descriptor after the other-account attempt and owner connect.
        let (_again, _) = checked_path(&self.endpoint, false)?;
        receive(rx, None)?;
        for (id, command) in [(1, "qmp_capabilities"), (2, "query-status"), (3, "quit")] {
            serde_json::to_writer(&mut *stdin, &json!({"execute":command,"id":id}))
                .map_err(io::Error::other)?;
            stdin.write_all(b"\n")?;
            stdin.flush()?;
            let reply = receive(rx, Some(id))?;
            if id == 2 {
                check(
                    reply["return"]["status"] == "prelaunch" && reply["return"]["running"] == false,
                )?;
            }
        }
        child.wait_exit(Duration::from_secs(30))?;
        Ok(
            json!({"candidatePassed":true,"directory":self.directory_info,"socket":endpoint_info,"otherAccount":proof,"qemuPeerPid":pid,"qmpPrelaunch":true,"qmpQuitExitObserved":true,"guestBoot":false,"fullContainmentQualified":false}),
        )
    }
}

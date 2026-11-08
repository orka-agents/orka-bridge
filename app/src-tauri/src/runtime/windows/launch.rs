use super::inputs::{invalid, VerifiedWindowsInputs};
use crate::runtime::types::{BackendKind, LaunchSpec, RuntimeError};
use std::{
    ffi::OsString,
    io,
    path::{Path, PathBuf},
};

/// AF_UNIX uses a fixed 108-byte UTF-8 field, including the terminating NUL.
pub fn validate_control_path(path: &Path) -> io::Result<PathBuf> {
    let invalid = || {
        io::Error::new(io::ErrorKind::InvalidInput,
        "AF_UNIX requires an absolute local UTF-8 endpoint of at most 107 bytes; choose a shorter private endpoint root")
    };
    if !path.is_absolute() {
        return Err(invalid());
    }
    let text = path.to_str().ok_or_else(invalid)?;
    #[cfg(windows)]
    let text = {
        use std::path::{Component, Prefix};
        match path.components().next() {
            Some(Component::Prefix(p))
                if matches!(p.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_)) => {}
            _ => return Err(invalid()),
        }
        text.strip_prefix("\\\\?\\").unwrap_or(text)
    };
    if text.contains('\0') || text.len() >= 108 {
        return Err(invalid());
    }
    Ok(PathBuf::from(text))
}
/// A fresh directory per launch prevents stale socket reuse; no prior file is unlinked.
pub fn control_path(root: &Path, nonce: bridge_runtime_protocol::Uuid) -> io::Result<PathBuf> {
    validate_control_path(
        &root
            .join(format!("c-{}", nonce.to_string().replace('-', "")))
            .join("s"),
    )
}
pub fn qemu_value(path: &Path) -> Result<String, RuntimeError> {
    let s = path.to_str().ok_or_else(invalid)?;
    if s.contains('\0') {
        return Err(invalid());
    }
    Ok(s.replace(',', ",,"))
}
/// Windows CRT argument quoting; no shell participates in process creation.
pub fn quote_windows_arg(s: &str) -> Result<String, RuntimeError> {
    if s.contains('\0') {
        return Err(invalid());
    }
    let mut out = String::from("\"");
    let mut slashes = 0;
    for c in s.chars() {
        if c == '\\' {
            slashes += 1;
            continue;
        }
        if c == '"' {
            out.push_str(&"\\".repeat(slashes * 2 + 1));
        } else {
            out.push_str(&"\\".repeat(slashes));
        }
        slashes = 0;
        out.push(c);
    }
    out.push_str(&"\\".repeat(slashes * 2));
    out.push('"');
    Ok(out)
}
/// Validate output of the owned, read-only `qemu-img info -f qcow2 --output=json`.
pub fn validate_qcow2_info(bytes: &[u8], expected: u64) -> Result<(), RuntimeError> {
    if bytes.len() > 128 * 1024 {
        return Err(invalid());
    }
    let v: serde_json::Value = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    if v.get("format").and_then(|x| x.as_str()) != Some("qcow2")
        || v.get("virtual-size").and_then(|x| x.as_u64()) != Some(expected)
        || [
            "backing-filename",
            "full-backing-filename",
            "backing-filename-format",
        ]
        .iter()
        .any(|k| v.get(k).is_some())
        || v.get("dirty-flag").is_some_and(|x| x != false)
        || v.get("encrypted").is_some_and(|x| x != false)
        || v.pointer("/format-specific/data/data-file").is_some()
        || v.pointer("/format-specific/data/data-file-raw")
            .is_some_and(|x| x != false)
        || v.pointer("/format-specific/data/corrupt")
            .is_some_and(|x| x != false)
    {
        return Err(invalid());
    }
    Ok(())
}
pub fn launch_argv(
    inputs: &VerifiedWindowsInputs,
    spec: &LaunchSpec,
    socket: &str,
) -> Result<Vec<OsString>, RuntimeError> {
    if spec.backend != BackendKind::Qemu
        || spec.machine_id != b"pc-q35-11.0"
        || spec.state_bytes != 32 * 1024 * 1024 * 1024
        || !spec.root.is_absolute()
        || spec.state_path != spec.root.join("state.disk")
    {
        return Err(invalid());
    }
    let socket = validate_control_path(Path::new(socket))?;
    let mut args: Vec<OsString> = vec![];
    let mut pair = |a: &str, b: OsString| {
        args.push(a.into());
        args.push(b);
    };
    pair(
        "-L",
        inputs.paths.runtime.join("share/qemu").into_os_string(),
    );
    for (a, b) in [
        ("-machine", "pc-q35-11.0,usb=off"),
        ("-accel", "whpx"),
        ("-cpu", "host"),
        ("-smp", "4"),
        ("-m", "6144"),
        ("-rtc", "base=utc"),
        ("-display", "none"),
        ("-vga", "none"),
        ("-monitor", "none"),
        ("-qmp", "stdio"),
    ] {
        pair(a, b.into());
    }
    pair("-uuid", spec.installation_id.to_string().into());
    pair(
        "-kernel",
        spec.root.join(spec.image.kernel_path()).into_os_string(),
    );
    pair(
        "-initrd",
        spec.root.join(spec.image.initrd_path()).into_os_string(),
    );
    pair("-append", spec.image.command_line().into());
    // Legacy -serial file: consumes a literal path, unlike -drive key/value syntax.
    let mut serial = OsString::from("file:");
    serial.push(spec.root.join("boot.log"));
    pair("-serial", serial);
    for (id, path) in [
        ("os", spec.root.join(spec.image.disk_path())),
        ("state", spec.state_path.clone()),
    ] {
        pair(
            "-drive",
            format!(
                "if=none,id={id},format=qcow2,cache=writeback,file={}",
                qemu_value(&path)?
            )
            .into(),
        );
        pair(
            "-device",
            format!("virtio-blk-pci,drive={id},serial=bridge-{id}").into(),
        );
    }
    pair("-netdev", "user,id=net".into());
    let mac = spec
        .mac
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(":");
    pair(
        "-device",
        format!("virtio-net-pci,netdev=net,mac={mac},romfile=").into(),
    );
    pair("-device", "virtio-rng-pci".into());
    pair(
        "-chardev",
        format!(
            "socket,id=guest,path={},server=on,wait=off",
            qemu_value(&socket)?
        )
        .into(),
    );
    pair("-device", "virtio-serial-pci,id=control".into());
    pair(
        "-device",
        "virtserialport,bus=control.0,chardev=guest,name=ai.orka.bridge.control".into(),
    );
    args.extend(["-S", "-nodefaults", "-no-user-config", "-no-reboot"].map(OsString::from));
    Ok(args)
}

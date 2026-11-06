use std::{env, fs, path::PathBuf, process::Command};
fn main() {
    let repo = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap())
        .join("../../../../../native-source")
        .canonicalize()
        .unwrap();
    const EXPECTED: &str = "9c5a33efd138ff48b3ad65962a69df4ea5991a2c";
    let head = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(&repo)
        .output()
        .unwrap();
    assert!(head.status.success());
    let head = String::from_utf8(head.stdout).unwrap();
    assert_eq!(head.trim(), EXPECTED, "wrong source head");
    let clean = Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(&repo)
        .output()
        .unwrap();
    assert!(
        clean.status.success() && clean.stdout.is_empty(),
        "source checkout must be clean"
    );
    let mut s = format!(
        "pub const SOURCE_HEAD:&str={:?}; pub mod runtime {{",
        head.trim()
    );
    for name in ["types", "identity", "journal", "spawn_guard"] {
        let p = repo.join(format!("app/src-tauri/src/runtime/{name}.rs"));
        println!("cargo:rerun-if-changed={}", p.display());
        s += &format!("#[path={p:?}] pub mod {name};");
    }
    s+="pub fn probe_spec(j:&journal::Journal)->types::LaunchSpec {j.spec(bridge_runtime_protocol::Uuid::new_v4())} pub mod windows {";
    for name in ["inputs", "launch", "security", "job", "control"] {
        let p = repo.join(format!("app/src-tauri/src/runtime/windows/{name}.rs"));
        println!("cargo:rerun-if-changed={}", p.display());
        s += &format!("#[path={p:?}] pub mod {name};");
    }
    s += r#"pub(crate) fn check_token(h:windows_sys::Win32::Foundation::HANDLE)->std::io::Result<()> {unsafe {use windows_sys::Win32::Security::*;use windows_sys::Win32::System::Threading::*;let mut raw=std::ptr::null_mut();security::check(OpenProcessToken(h,TOKEN_QUERY,&mut raw))?;let token=security::Handle::new(raw)?;let mut kind=0i32;let mut n=0;security::check(GetTokenInformation(token.raw(),TokenType,(&mut kind as *mut i32).cast(),4,&mut n))?;if IsTokenRestricted(token.raw())==0 || kind!=TokenPrimary{return Err(std::io::ErrorKind::PermissionDenied.into())} Ok(())}} "#;
    s+="pub(crate) fn check_unix_path(h:windows_sys::Win32::Foundation::HANDLE,directory:bool)->std::io::Result<()> {security::private_af_object(h,directory)} ";
    s+="pub(crate) fn check_pipe(h:windows_sys::Win32::Foundation::HANDLE)->std::io::Result<()> {security::private_object(h,true,windows_sys::Win32::Security::Authorization::SE_KERNEL_OBJECT)} }}";
    fs::write(
        PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("source.rs"),
        s,
    )
    .unwrap();
    println!("cargo:rerun-if-changed={}", repo.join(".git").display());
}

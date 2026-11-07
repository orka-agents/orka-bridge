use std::{env, fs, path::PathBuf, process::Command};
fn main() {
    let repo = env::var_os("BRIDGE_NATIVE_SOURCE")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap())
                .join("../../../../../boot-source")
        })
        .canonicalize()
        .unwrap();
    const EXPECTED: &str = "325ff5faf11ad0b3ad0a53f54b4ba74a97a31cd5";
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
    let root = repo.join("app/src-tauri/src");
    let runtime = root.join("runtime/mod.rs");
    let dev_image = root.join("dev_image.rs");
    let types = root.join("engine/types.rs");
    let security = root.join("runtime/windows/security.rs");
    let fixture = repo.join("app/src-tauri/tests/support/runtime_image_fixture.rs");
    let mut s = format!("pub const SOURCE_HEAD:&str={:?}; pub mod runtime {{ include!({runtime:?});
        pub fn probe_spec(j:&journal::Journal,image:std::sync::Arc<image_install::InstalledImageSet>)->types::LaunchSpec {{
            j.spec(bridge_runtime_protocol::Uuid::new_v4(),image)
        }}
        }}
        pub mod engine {{ #[path={types:?}] pub mod types; }}
        #[cfg(debug_assertions)] #[path={dev_image:?}] pub mod dev_image;
        // Independent proof checks use the same source validator in this parent scope.
        #[allow(clippy::duplicate_mod)] #[path={security:?}] mod proof_security;
        mod fixture_runtime {{ pub use crate::runtime::*; }}
        #[path={fixture:?}] mod native_image_fixture;
        pub mod proof {{", head.trim());
    s += r#"pub(crate) fn check_token(h:windows_sys::Win32::Foundation::HANDLE)->std::io::Result<()> {
        unsafe {use windows_sys::Win32::Security::*;use windows_sys::Win32::System::Threading::*;
        let mut raw=std::ptr::null_mut();crate::proof_security::check(OpenProcessToken(h,TOKEN_QUERY,&mut raw))?;
        let token=crate::proof_security::Handle::new(raw)?;let mut kind=0i32;let mut n=0;
        crate::proof_security::check(GetTokenInformation(token.raw(),TokenType,(&mut kind as *mut i32).cast(),4,&mut n))?;
        if IsTokenRestricted(token.raw())==0 || kind!=TokenPrimary{return Err(std::io::ErrorKind::PermissionDenied.into())} Ok(())}}
        pub(crate) fn check_unix_path(h:windows_sys::Win32::Foundation::HANDLE,directory:bool)->std::io::Result<()> {crate::proof_security::private_af_object(h,directory)}
        pub(crate) fn check_pipe(h:windows_sys::Win32::Foundation::HANDLE)->std::io::Result<()> {crate::proof_security::private_object(h,true,windows_sys::Win32::Security::Authorization::SE_KERNEL_OBJECT)}
        }"#;
    fs::write(
        PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("source.rs"),
        s,
    )
    .unwrap();
    println!("cargo:rerun-if-changed={}", root.display());
    println!("cargo:rerun-if-changed={}", fixture.display());
    println!("cargo:rerun-if-changed={}", repo.join(".git").display());
}

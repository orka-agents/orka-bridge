use sha2::{Digest, Sha256};
use std::{env, fs, path::PathBuf, process::Command};
#[path = "src/job_insert.rs"]
mod job_insert;
fn main() {
    let repo = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap())
        .join("../../../../../native-source")
        .canonicalize()
        .unwrap();
    const EXPECTED: &str = "443bcc0718a777e03057c20ddc1a8dff12b159bf";
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
    // Normal builds ALWAYS import the unmodified sealed job.rs. Only the explicit
    // diagnostic feature imports a derived OUT_DIR copy, and says so in flags.rs.
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let diagnostic = env::var_os("CARGO_FEATURE_MEMBER_DIAGNOSTIC").is_some();
    let job = repo.join("app/src-tauri/src/runtime/windows/job.rs");
    let original = fs::read_to_string(&job).unwrap();
    let source_sha = format!("{:x}", Sha256::digest(original.as_bytes()));
    let (job_import, import_sha) = if diagnostic {
        let derived = job_insert::derive(&original).expect("diagnostic job.rs derivation");
        let path = out.join("job_member_diagnostic.rs");
        fs::write(&path, &derived).unwrap();
        (path, format!("{:x}", Sha256::digest(derived.as_bytes())))
    } else {
        (job.clone(), source_sha.clone())
    };
    fs::write(
        out.join("flags.rs"),
        format!(
            "#[allow(dead_code)] pub const DIAGNOSTIC_IMPORTS:bool={diagnostic}; #[allow(dead_code)] pub const JOB_SOURCE_PATH:&str={:?}; #[allow(dead_code)] pub const JOB_SOURCE_SHA256:&str={source_sha:?}; #[allow(dead_code)] pub const JOB_IMPORT_PATH:&str={:?}; #[allow(dead_code)] pub const JOB_IMPORT_SHA256:&str={import_sha:?};",
            job.to_str().unwrap(),
            job_import.to_str().unwrap()
        ),
    )
    .unwrap();
    println!("cargo:rerun-if-changed=src/job_insert.rs");
    for name in ["inputs", "launch", "security", "job", "control"] {
        let p = repo.join(format!("app/src-tauri/src/runtime/windows/{name}.rs"));
        println!("cargo:rerun-if-changed={}", p.display());
        let p = if name == "job" { job_import.clone() } else { p };
        s += &format!("#[path={p:?}] pub mod {name};");
    }
    s += r#"pub(crate) fn check_token(h:windows_sys::Win32::Foundation::HANDLE)->std::io::Result<()> {unsafe {use windows_sys::Win32::Security::*;use windows_sys::Win32::System::Threading::*;let mut raw=std::ptr::null_mut();security::check(OpenProcessToken(h,TOKEN_QUERY,&mut raw))?;let token=security::Handle::new(raw)?;let mut kind=0i32;let mut n=0;security::check(GetTokenInformation(token.raw(),TokenType,(&mut kind as *mut i32).cast(),4,&mut n))?;if IsTokenRestricted(token.raw())==0 || kind!=TokenPrimary{return Err(std::io::ErrorKind::PermissionDenied.into())} Ok(())}} "#;
    s+="pub(crate) fn check_unix_path(h:windows_sys::Win32::Foundation::HANDLE,directory:bool)->std::io::Result<()> {security::private_af_object(h,directory)} ";
    s+="pub(crate) fn check_pipe(h:windows_sys::Win32::Foundation::HANDLE)->std::io::Result<()> {security::private_object(h,true,windows_sys::Win32::Security::Authorization::SE_KERNEL_OBJECT)} }}";
    fs::write(out.join("source.rs"), s).unwrap();
    println!("cargo:rerun-if-changed={}", repo.join(".git").display());
}

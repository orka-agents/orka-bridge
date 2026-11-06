fn main() {
    println!("cargo:rerun-if-changed=native/macos");
    println!("cargo:rerun-if-changed=../../scripts/build-vm-helper-macos.sh");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos")
        && std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() == Ok("aarch64")
    {
        // Tauri copies configured resources during try_build, so this must run
        // first, including cargo test/clippy and development builds.
        let manifest = std::path::PathBuf::from(
            std::env::var_os("CARGO_MANIFEST_DIR").expect("Cargo manifest directory missing"),
        );
        let status = std::process::Command::new("bash")
            .arg(manifest.join("../../scripts/build-vm-helper-macos.sh"))
            .status()
            .expect("Apple helper build requires bash and the Xcode Swift compiler");
        assert!(status.success(), "failed to build signed Apple helper");
    }
    let attributes =
        tauri_build::Attributes::new().app_manifest(tauri_build::AppManifest::new().commands(&[
            "engine_call",
            "setup_status",
            "setup_select_local",
        ]));
    tauri_build::try_build(attributes).expect("failed to build Bridge");
}

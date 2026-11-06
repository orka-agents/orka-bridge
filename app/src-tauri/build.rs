fn main() {
    let attributes =
        tauri_build::Attributes::new().app_manifest(tauri_build::AppManifest::new().commands(&[
            "engine_call",
            "setup_status",
            "setup_select_local",
        ]));
    tauri_build::try_build(attributes).expect("failed to build Bridge");
}

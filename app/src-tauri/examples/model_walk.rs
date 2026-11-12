// Native UI fixture only. Embeds the real frontend, but never starts a VM,
// reads a credential, or runs GitHub CLI. Not linked into the Bridge executable.
#[cfg(not(all(target_os = "macos", feature = "custom-protocol")))]
fn main() {}

#[cfg(all(target_os = "macos", feature = "custom-protocol"))]
fn main() {
    use serde_json::json;
    use tauri::Manager;
    let removed_once = std::sync::atomic::AtomicBool::new(false);
    tauri::Builder::default()
        .setup(|app| {
            app.get_webview_window("main")
                .unwrap()
                .set_title("Bridge model fixture (synthetic IPC, no VM)")?;
            Ok(())
        })
        .invoke_handler(move |invoke| {
            let reply = match invoke.message.command() {
                "setup_status" => {
                    json!({"selected":"localVm", "localVm":"ready", "model":"available"})
                }
                "setup_requirements" => json!(null),
                "setup_connect_model" => json!({"ready":true}),
                "setup_remove_model" => {
                    if !removed_once.swap(true, std::sync::atomic::Ordering::SeqCst) {
                        invoke.resolver.reject(json!({"kind":"modelBusy"}));
                        return true;
                    }
                    json!({"ready":false})
                }
                _ => {
                    invoke
                        .resolver
                        .reject("Unavailable in the native model fixture");
                    return true;
                }
            };
            invoke.resolver.resolve(reply);
            true
        })
        .run(tauri::generate_context!())
        .expect("native model fixture failed");
}

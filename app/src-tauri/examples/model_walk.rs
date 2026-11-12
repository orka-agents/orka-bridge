// Native UI fixture only. Embeds the real frontend, but never starts a VM,
// reads a credential, or runs GitHub CLI. Not linked into the Bridge executable.
#[cfg(not(all(target_os = "macos", feature = "custom-protocol")))]
fn main() {}

#[cfg(all(target_os = "macos", feature = "custom-protocol"))]
fn main() {
    use serde_json::json;
    use tauri::Manager;
    let removed_once = std::sync::atomic::AtomicBool::new(false);
    let running = std::sync::atomic::AtomicBool::new(false);
    let original = include_str!("../example-agent/web-reader.agent.md");
    let agent = std::sync::Mutex::new(original.to_owned());
    let tests = "---\nschemaVersion: 1\n---\n```yaml agent-test\nid: reads-page\ninput: What did the page say?\nassertions:\n - id: text\n   type: contains\n   value: checksum\n```\n```yaml agent-test\nid: has-a-date\ninput: Which date is shown on the page?\nassertions:\n - id: date\n   type: contains\n   value: '2099-12-31'\n```\n";
    let source = bridge_runtime_protocol::SnapshotId::of(original.as_bytes(), tests.as_bytes());
    tauri::Builder::default()
        .setup(|app| {
            app.get_webview_window("main")
                .unwrap()
                .set_title("Bridge model fixture (synthetic IPC, no VM)")?;
            Ok(())
        })
        .invoke_handler(move |invoke| {
            let reply = match invoke.message.command() {
                "agent_open" | "agent_save" => {
                    let mut content = agent.lock().unwrap();
                    if invoke.message.command() == "agent_save" {
                        if let tauri::ipc::InvokeBody::Json(args) = invoke.message.payload() {
                            if let Some(value) = args["content"].as_str() {
                                *content = value.into();
                            }
                        }
                    }
                    json!({"documentId":"9b2b93c4-375c-4815-9717-39852a96e411", "name":"web-reader", "agent":*content, "tests":tests,
                        "source":bridge_runtime_protocol::SnapshotId::of(content.as_bytes(), tests.as_bytes())})
                }
                "send_history" => json!([]),
                "send_status" => json!({"documentId":"9b2b93c4-375c-4815-9717-39852a96e411", "view":null, "busy":null}),
                "agent_run_status" | "agent_run" => {
                    if invoke.message.command() == "agent_run" {
                        running.store(true, std::sync::atomic::Ordering::SeqCst);
                    }
                    let active = running.load(std::sync::atomic::Ordering::SeqCst);
                    json!({"documentId":"9b2b93c4-375c-4815-9717-39852a96e411", "busy":null,
                        "state":if active {"running"} else {"complete"}, "stage":if active {"observe"} else {"done"},
                        "model":"gpt-5.6-terra", "modelConnected":true, "canRun":!active, "source":source,
                        "acknowledged":[], "pending":null, "error":null,
                        "report":if active {json!(null)} else {json!({"verdict":"failed", "cases":[
                            {"id":"reads-page", "question":"What did the page say?", "phase":"Succeeded", "answer":"Fixture checksum.", "completedTools":["web_fetch"], "verdict":"passed", "assertions":[{"id":"text", "kind":"contains", "expected":"checksum", "verdict":"passed"}]},
                            {"id":"has-a-date", "question":"Which date is shown on the page?", "phase":"Succeeded", "answer":"No date was given.", "completedTools":[], "verdict":"failed", "assertions":[{"id":"date", "kind":"contains", "expected":"2099-12-31", "verdict":"failed"}]}
                        ]})}})
                }
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

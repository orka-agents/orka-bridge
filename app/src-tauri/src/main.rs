#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use bridge_engine::agent_folder::{AgentFolder, AgentPair};
use bridge_engine::agents::Agents;
use bridge_engine::engine::{types::*, EngineState};
use bridge_engine::local_vm::{self, ExamplePair, LocalVm, LocalVmStatus};
use bridge_engine::send::{SendOwner, SendView};
use bridge_engine::setup::{SetupError, SetupSnapshot, SetupStore};
use bridge_engine::shutdown::TerminalShutdown;
use std::sync::Arc;
use tauri::Manager;

mod lifecycle;

#[tauri::command]
async fn engine_call(
    request: EngineRequest,
    state: tauri::State<'_, EngineState>,
) -> Result<EngineReply, EngineError> {
    state.call(request).await
}
#[tauri::command]
async fn setup_status(
    state: tauri::State<'_, Arc<SetupStore>>,
    vm: tauri::State<'_, Arc<LocalVm>>,
) -> Result<SetupSnapshot, SetupError> {
    local_vm::setup_status(Arc::clone(state.inner()), vm.inner()).await
}
#[tauri::command]
async fn setup_select_local(
    state: tauri::State<'_, Arc<SetupStore>>,
    vm: tauri::State<'_, Arc<LocalVm>>,
) -> Result<SetupSnapshot, SetupError> {
    local_vm::select_local(Arc::clone(state.inner()), vm.inner()).await
}
#[tauri::command]
async fn setup_cancel_local(
    state: tauri::State<'_, Arc<SetupStore>>,
    vm: tauri::State<'_, Arc<LocalVm>>,
) -> Result<SetupSnapshot, SetupError> {
    local_vm::cancel_local(Arc::clone(state.inner()), vm.inner()).await
}
#[tauri::command]
async fn setup_change_local(
    state: tauri::State<'_, Arc<SetupStore>>,
    vm: tauri::State<'_, Arc<LocalVm>>,
) -> Result<SetupSnapshot, SetupError> {
    local_vm::change_local(Arc::clone(state.inner()), vm.inner()).await
}
#[tauri::command]
fn setup_requirements() -> Option<bridge_engine::setup::SetupRequirements> {
    bridge_engine::setup::setup_requirements()
}
#[tauri::command]
async fn setup_preflight(
    vm: tauri::State<'_, Arc<LocalVm>>,
) -> Option<bridge_engine::setup::preflight::Preflight> {
    if vm.status() == LocalVmStatus::NotIntegrated
        || bridge_engine::setup::setup_requirements().is_none()
    {
        return None;
    }
    tauri::async_runtime::spawn_blocking(bridge_engine::setup::preflight::observe)
        .await
        .ok()
        .flatten()
}
#[tauri::command]
async fn setup_open_help(topic: bridge_engine::setup::help::HelpTopic) -> Result<(), &'static str> {
    tauri::async_runtime::spawn_blocking(move || bridge_engine::setup::help::open_help(topic))
        .await
        .map_err(|_| "Could not open setup help.")?
}
#[tauri::command]
fn setup_example() -> ExamplePair {
    local_vm::example()
}

#[tauri::command]
async fn agent_open(state: tauri::State<'_, Arc<AgentFolder>>) -> Result<AgentPair, &'static str> {
    let folder = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || folder.open())
        .await
        .map_err(|_| "The local agent folder is unavailable.")?
}
#[tauri::command]
async fn agent_save(
    content: String,
    state: tauri::State<'_, Arc<AgentFolder>>,
) -> Result<AgentPair, &'static str> {
    let folder = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || folder.save(content))
        .await
        .map_err(|_| "The local agent folder is unavailable.")?
}

#[tauri::command]
async fn workspace_catalog(
    state: tauri::State<'_, Arc<SetupStore>>,
    vm: tauri::State<'_, Arc<LocalVm>>,
) -> Result<bridge_engine::catalog::CatalogReply, &'static str> {
    vm.workspace_catalog(Arc::clone(state.inner())).await
}
#[derive(serde::Serialize)]
struct ModelReply {
    ready: bool,
}
#[tauri::command]
async fn setup_connect_model(
    state: tauri::State<'_, Arc<SetupStore>>,
    vm: tauri::State<'_, Arc<LocalVm>>,
) -> Result<ModelReply, SetupError> {
    let model = vm.connect_model(Arc::clone(state.inner())).await?;
    Ok(ModelReply { ready: model.ready })
}

#[tauri::command]
fn setup_run_status(
    state: tauri::State<'_, Arc<bridge_engine::example_run::ExampleRun>>,
    vm: tauri::State<'_, Arc<LocalVm>>,
) -> bridge_engine::example_run::RunSnapshot {
    state.status(vm.inner())
}
#[tauri::command]
fn setup_run_example(
    state: tauri::State<'_, Arc<bridge_engine::example_run::ExampleRun>>,
    vm: tauri::State<'_, Arc<LocalVm>>,
) -> bridge_engine::example_run::RunSnapshot {
    state.inner().start(vm.inner().clone())
}

#[tauri::command]
async fn cluster_contexts(
    state: tauri::State<'_, bridge_engine::clusters::commands::ClusterSession>,
) -> Result<Vec<bridge_engine::clusters::ContextView>, &'static str> {
    state.contexts().await
}
#[tauri::command]
async fn cluster_discover(
    context_id: String,
    state: tauri::State<'_, bridge_engine::clusters::commands::ClusterSession>,
) -> Result<Vec<bridge_engine::clusters::TargetView>, bridge_engine::clusters::commands::AccessStatus>
{
    state.discover(&context_id).await
}
#[tauri::command]
async fn cluster_use(
    target_id: String,
    sa_name: Option<String>,
    state: tauri::State<'_, bridge_engine::clusters::commands::ClusterSession>,
) -> Result<bridge_engine::clusters::commands::AccessView, &'static str> {
    Ok(state.use_target(&target_id, sa_name).await)
}

#[tauri::command]
async fn send_preflight(
    target_id: String,
    provider_id: String,
    state: tauri::State<'_, Arc<SendOwner>>,
) -> Result<bridge_engine::send::SendCheck, &'static str> {
    Ok(state.preflight(&target_id, &provider_id).await)
}
#[tauri::command]
async fn send_start(
    target_id: String,
    provider_id: String,
    expected_source: bridge_runtime_protocol::SnapshotId,
    state: tauri::State<'_, Arc<SendOwner>>,
) -> Result<SendView, &'static str> {
    Ok(state.start(&target_id, &provider_id, expected_source).await)
}
#[tauri::command]
fn send_status(
    send_id: Option<String>,
    state: tauri::State<'_, Arc<SendOwner>>,
) -> Option<SendView> {
    state.status(send_id.as_deref())
}
#[tauri::command]
async fn send_stop(
    send_id: String,
    state: tauri::State<'_, Arc<SendOwner>>,
) -> Result<Option<SendView>, &'static str> {
    Ok(state.stop(&send_id).await)
}

fn main() {
    let shutdown = Arc::new(TerminalShutdown::default());
    tauri::Builder::default()
        .manage(shutdown.clone())
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            lifecycle::show(app)
        }))
        .setup(move |app| {
            let store = Arc::new(SetupStore::new(app.path().app_data_dir().ok()));
            let vm = local_vm::configured(
                app.path().resource_dir().ok(),
                app.path().app_local_data_dir().ok(),
            );
            app.manage(if vm.status() == LocalVmStatus::NotIntegrated {
                EngineState::configured()
            } else {
                EngineState::with_transport(vm.clone())
            });
            app.manage(store.clone());
            app.manage(vm.clone());
            let session = bridge_engine::clusters::commands::ClusterSession::default();
            app.manage(session.clone());
            let agents = Agents::new(app.path().home_dir().ok(), app.path().app_data_dir().ok());
            let folder = agents.sample();
            app.manage(agents);
            app.manage(folder.clone());
            app.manage(SendOwner::new(folder.clone(), session));
            app.manage(bridge_engine::example_run::ExampleRun::new(
                tauri::async_runtime::handle().inner().clone(),
                bridge_engine::local_vm::MODEL_IMAGE_AVAILABLE,
                folder,
            ));
            lifecycle::install(
                app.handle(),
                shutdown.clone(),
                vm.status() != LocalVmStatus::NotIntegrated,
            )?;
            tauri::async_runtime::spawn(async move {
                if local_vm::resume_selected(store, &vm).await.is_err() {
                    eprintln!("The saved setup choice could not be read.");
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            engine_call,
            setup_status,
            setup_select_local,
            setup_cancel_local,
            setup_change_local,
            setup_requirements,
            setup_preflight,
            setup_open_help,
            setup_example,
            setup_connect_model,
            setup_run_status,
            setup_run_example,
            agent_open,
            agent_save,
            workspace_catalog,
            cluster_contexts,
            cluster_discover,
            cluster_use,
            send_preflight,
            send_start,
            send_status,
            send_stop
        ])
        .build(tauri::generate_context!())
        .expect("error while building Bridge")
        .run(|app, event| {
            if app.state::<Arc<TerminalShutdown>>().completed() {
                return;
            }
            let requested = match event {
                tauri::RunEvent::WindowEvent {
                    label,
                    event: tauri::WindowEvent::CloseRequested { api, .. },
                    ..
                } if label == "main" => {
                    api.prevent_close();
                    if app.state::<lifecycle::HideOnClose>().0 {
                        if let Some(window) = app.get_webview_window("main") {
                            if window.hide().is_ok() {
                                return;
                            }
                        }
                        eprintln!("Could not hide Bridge; stopping the workspace instead.");
                    }
                    true
                }
                #[cfg(target_os = "macos")]
                tauri::RunEvent::Reopen { .. } => {
                    lifecycle::show(app);
                    false
                }
                tauri::RunEvent::ExitRequested { api, .. } => {
                    api.prevent_exit();
                    true
                }
                _ => false,
            };
            if !requested {
                return;
            }
            lifecycle::stop(app);
        });
}

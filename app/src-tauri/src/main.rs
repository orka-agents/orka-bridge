#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use bridge_engine::agent_folder::{AgentFolder, AgentPair};
use bridge_engine::engine::{types::*, EngineState};
use bridge_engine::local_vm::{self, ExamplePair, LocalVm, LocalVmStatus};
use bridge_engine::send::{SendOwner, SendView};
use bridge_engine::setup::{SetupError, SetupSnapshot, SetupStore};
use bridge_engine::shutdown::TerminalShutdown;
use std::sync::Arc;
use tauri::Manager;

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
async fn send_start(
    target_id: String,
    provider_id: String,
    state: tauri::State<'_, Arc<SendOwner>>,
) -> Result<SendView, &'static str> {
    Ok(state.start(&target_id, &provider_id).await)
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
        .setup(|app| {
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
            let folder = Arc::new(AgentFolder::new(app.path().home_dir().ok()));
            app.manage(folder.clone());
            app.manage(SendOwner::new(folder.clone(), session));
            app.manage(bridge_engine::example_run::ExampleRun::new(
                tauri::async_runtime::handle().inner().clone(),
                bridge_engine::local_vm::MODEL_IMAGE_AVAILABLE,
                folder,
            ));
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
            send_start,
            send_status,
            send_stop
        ])
        .build(tauri::generate_context!())
        .expect("error while building Bridge")
        .run(move |app, event| {
            if shutdown.completed() {
                return;
            }
            let requested = match event {
                tauri::RunEvent::WindowEvent {
                    label,
                    event: tauri::WindowEvent::CloseRequested { api, .. },
                    ..
                } if label == "main" => {
                    api.prevent_close();
                    true
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
            let vm = app.state::<Arc<LocalVm>>().inner().clone();
            if !shutdown.request(&vm, &app.state::<EngineState>()) {
                return;
            }
            let send = app.state::<Arc<SendOwner>>().inner().clone();
            let handle = app.clone();
            let shutdown = shutdown.clone();
            tauri::async_runtime::spawn(async move {
                let clean = shutdown.finish(&send, &vm).await;
                handle.exit(if clean { 0 } else { 1 });
            });
        });
}

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use bridge_engine::engine::{types::*, EngineState};
use bridge_engine::runtime::RuntimeState;
use bridge_engine::setup::{SetupError, SetupSnapshot, SetupStore};
use tauri::Manager;

#[tauri::command]
async fn engine_call(
    request: EngineRequest,
    state: tauri::State<'_, EngineState>,
) -> Result<EngineReply, EngineError> {
    state.call(request).await
}

#[tauri::command]
fn setup_status(state: tauri::State<'_, SetupStore>) -> Result<SetupSnapshot, SetupError> {
    state.status()
}

#[tauri::command]
fn setup_select_local(state: tauri::State<'_, SetupStore>) -> Result<SetupSnapshot, SetupError> {
    state.select_local()
}

fn main() {
    tauri::Builder::default()
        .manage(EngineState::configured())
        .manage(RuntimeState::default())
        .setup(|app| {
            app.manage(SetupStore::new(app.path().app_data_dir().ok()));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            engine_call,
            setup_status,
            setup_select_local
        ])
        .on_window_event(|window, event| {
            if window.label() == "main"
                && matches!(
                    event,
                    tauri::WindowEvent::CloseRequested { .. } | tauri::WindowEvent::Destroyed
                )
            {
                window.state::<EngineState>().shutdown();
                // No backend is configured yet. A future backend must observe stop.
                let _ = window.state::<RuntimeState>().shutdown();
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running Bridge");
}

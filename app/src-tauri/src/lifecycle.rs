use bridge_engine::{
    engine::EngineState, local_vm::LocalVm, send::SendOwner, shutdown::TerminalShutdown,
};
use std::sync::Arc;
use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    Manager,
};

#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

pub struct HideOnClose(pub bool);

/// Activation only exposes the existing webview; it never re-enters VM setup.
pub fn show(app: &tauri::AppHandle) {
    if app.state::<Arc<TerminalShutdown>>().closing() {
        return;
    }
    if let Some(window) = app.get_webview_window("main") {
        if let Err(error) = window
            .unminimize()
            .and_then(|_| window.show())
            .and_then(|_| window.set_focus())
        {
            eprintln!("Could not open Bridge: {error}");
        }
    }
}

/// Admission is barred on the caller's thread, before the one cleanup worker is queued.
pub fn stop(app: &tauri::AppHandle) {
    let shutdown = app.state::<Arc<TerminalShutdown>>().inner().clone();
    let vm = app.state::<Arc<LocalVm>>().inner().clone();
    if !shutdown.request(&vm, &app.state::<EngineState>()) {
        return;
    }
    let send = app.state::<Arc<SendOwner>>().inner().clone();
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        shutdown.finish(&send, &vm).await;
        shutdown.exit_unless_native(|clean| handle.exit(if clean { 0 } else { 1 }));
    });
}

pub fn install(
    app: &tauri::AppHandle,
    shutdown: Arc<TerminalShutdown>,
    integrated: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    // Hooks and a usable menu must exist before hidden mode or saved startup is admitted.
    #[cfg(any(windows, target_os = "macos"))]
    {
        let handle = app.clone();
        let request: Arc<dyn Fn() + Send + Sync> = Arc::new(move || stop(&handle));
        #[cfg(windows)]
        windows::install(shutdown, request)?;
        #[cfg(target_os = "macos")]
        macos::install(shutdown, request)?;
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    let _ = shutdown;
    if integrated {
        #[cfg(windows)]
        {
            // Muda's predefined Windows Quit posts WM_QUIT without ExitRequested.
            // A custom item must bar admission and join cleanup before leaving Tao.
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let file = tauri::menu::Submenu::with_items(app, "File", true, &[&quit])?;
            app.set_menu(Menu::with_items(app, &[&file])?)?;
            app.on_menu_event(|app, event| {
                if event.id().as_ref() == "quit" {
                    stop(app);
                }
            });
        }
        let open = MenuItem::with_id(app, "open", "Open Bridge", true, None::<&str>)?;
        let quit = MenuItem::with_id(
            app,
            "stop-quit",
            "Stop workspace and quit",
            true,
            None::<&str>,
        )?;
        let menu = Menu::with_items(app, &[&open, &quit])?;
        let icon = app
            .default_window_icon()
            .ok_or("Bridge's tray icon is missing")?
            .clone();
        TrayIconBuilder::with_id("bridge")
            .icon(icon)
            .icon_as_template(cfg!(target_os = "macos"))
            .tooltip("Bridge")
            .menu(&menu)
            .show_menu_on_left_click(true)
            .on_menu_event(|app, event| match event.id().as_ref() {
                "open" => show(app),
                "stop-quit" => stop(app),
                _ => {}
            })
            .build(app)?;
    }
    app.manage(HideOnClose(integrated));
    Ok(())
}

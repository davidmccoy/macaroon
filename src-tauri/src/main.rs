// Prevents additional console window on Windows in release
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod compositor;
mod sidecar;
mod state;
mod tray;
mod types;

use tauri::Manager;

fn main() {
    // Initialize logger
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .init();

    log::info!("Starting Now Playing menu bar app");

    tauri::Builder::default()
        .setup(|app| {
            log::info!("Setting up application");

            // Create shared state
            let state = state::create_state();

            // Initialize system tray
            tray::TrayManager::setup(app.handle(), state.clone())
                .expect("Failed to setup system tray");

            log::info!("System tray initialized");

            // Spawn sidecar process
            let mut sidecar_manager = sidecar::SidecarManager::new();
            match sidecar_manager.spawn(app.handle(), state.clone()) {
                Ok(_) => {
                    log::info!("Sidecar spawned successfully");
                }
                Err(e) => {
                    log::error!("Failed to spawn sidecar: {}", e);
                    // Continue running even if sidecar fails
                }
            }

            // Setup signal handler for Ctrl+C (SIGINT) and SIGTERM
            let sidecar_for_signal = sidecar_manager.clone();
            ctrlc::set_handler(move || {
                log::info!("Received interrupt signal (Ctrl+C), cleaning up sidecar...");
                if let Err(e) = sidecar_for_signal.stop() {
                    log::error!("Error stopping sidecar on interrupt: {}", e);
                }
                std::process::exit(0);
            })
            .expect("Failed to set Ctrl+C handler");

            // Store sidecar manager in app state for cleanup
            app.manage(sidecar_manager);

            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app_handle, event| {
            match event {
                tauri::RunEvent::Exit => {
                    log::info!("App exit event received, cleaning up sidecar...");

                    // Get the sidecar manager from managed state and stop it
                    if let Some(sidecar) = app_handle.try_state::<sidecar::SidecarManager>() {
                        if let Err(e) = sidecar.stop() {
                            log::error!("Error stopping sidecar on exit: {}", e);
                        }
                    }
                }
                tauri::RunEvent::ExitRequested { .. } => {
                    log::info!("App exit requested, cleaning up sidecar...");

                    // Get the sidecar manager from managed state and stop it
                    if let Some(sidecar) = app_handle.try_state::<sidecar::SidecarManager>() {
                        if let Err(e) = sidecar.stop() {
                            log::error!("Error stopping sidecar on exit request: {}", e);
                        }
                    }
                }
                _ => {}
            }
        });
}

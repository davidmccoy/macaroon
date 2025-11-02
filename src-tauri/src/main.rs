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

            // Store sidecar manager in app state for cleanup
            app.manage(sidecar_manager);

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

// Prevents additional console window on Windows in release
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod compositor;
mod sidecar;
mod state;
mod tray;
mod types;

use std::thread;
use std::time::Duration;
use tauri::Manager;

fn main() {
    // Initialize logger
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .init();

    log::info!("Starting Now Playing menu bar app");

    // Hide from Dock and Cmd+Tab (macOS only)
    #[cfg(target_os = "macos")]
    {
        use tauri::ActivationPolicy;
        tauri::Builder::default()
            .setup(|app| {
                // Set as accessory app (menu bar only, no dock icon)
                app.set_activation_policy(ActivationPolicy::Accessory);
                setup_app(app)
            })
            .build(tauri::generate_context!())
            .expect("error while building tauri application")
            .run(run_handler);
        return;
    }

    #[cfg(not(target_os = "macos"))]
    tauri::Builder::default()
        .setup(|app| setup_app(app))
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(run_handler);
}

fn setup_app(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    log::info!("Setting up application");

            // Create shared state
            let state = state::create_state();

            // Initialize system tray first
            tray::TrayManager::setup(app.handle(), state.clone())
                .expect("Failed to setup system tray");

            log::info!("System tray initialized");

            // Spawn sidecar process
            // Zones will arrive and populate the menu within ~500ms
            let sidecar_manager = sidecar::SidecarManager::new();
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
                // Stop sidecar and wait for it to complete before exiting
                match sidecar_for_signal.stop() {
                    Ok(_) => {
                        log::info!("Sidecar stopped successfully on interrupt");
                    }
                    Err(e) => {
                        log::error!("Error stopping sidecar on interrupt: {}", e);
                    }
                }
                // Only exit after sidecar has been stopped
                std::process::exit(0);
            })
            .expect("Failed to set Ctrl+C handler");

    // Store sidecar manager in app state for cleanup
    app.manage(sidecar_manager);

    // Spawn a thread to periodically check for dark mode changes
    // This ensures the icon updates even when music is paused
    let app_handle = app.handle().clone();
    let state_for_appearance = state.clone();
    thread::spawn(move || {
        loop {
            thread::sleep(Duration::from_secs(5));

            // Check if dark mode changed
            let current_dark_mode = matches!(dark_light::detect(), dark_light::Mode::Dark);
            let needs_update = {
                let state_guard = state_for_appearance.read();
                state_guard.last_dark_mode != Some(current_dark_mode) && state_guard.current_track.is_some()
            };

            if needs_update {
                log::debug!("Dark mode changed, triggering icon update");
                let app_clone = app_handle.clone();
                let state_clone = state_for_appearance.clone();
                if let Err(e) = app_handle.run_on_main_thread(move || {
                    if let Err(e) = tray::TrayManager::update_icon(&app_clone, &state_clone) {
                        log::error!("Failed to update icon after dark mode change: {}", e);
                    }
                }) {
                    log::error!("Failed to dispatch dark mode icon update: {}", e);
                }
            }
        }
    });

    Ok(())
}

fn run_handler(app_handle: &tauri::AppHandle, event: tauri::RunEvent) {
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
}

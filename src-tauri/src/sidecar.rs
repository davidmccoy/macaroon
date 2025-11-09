use anyhow::{Context, Result};
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use tauri::{AppHandle, Manager, Runtime};

use crate::state::SharedState;
use crate::tray::TrayManager;
use crate::types::{ConnectionStatus, NowPlayingData, SidecarMessage};

/// Manages the Node.js sidecar process
#[derive(Clone)]
pub struct SidecarManager {
    child: Arc<Mutex<Option<Child>>>,
}

impl SidecarManager {
    pub fn new() -> Self {
        Self {
            child: Arc::new(Mutex::new(None)),
        }
    }

    /// Spawn the sidecar process and start reading its output
    pub fn spawn<R: Runtime>(
        &mut self,
        app: &AppHandle<R>,
        state: SharedState,
    ) -> Result<()> {
        log::info!("Spawning sidecar process...");

        // Spawn the process based on environment
        let mut child = if cfg!(debug_assertions) {
            // Development mode: run with node directly
            // In dev mode, current_dir is the project root (where we run npm run tauri dev)
            let mut script_path = std::env::current_dir()
                .context("Failed to get current directory")?
                .join("sidecar/build/index.js");

            // If that doesn't exist, try going up one level (in case we're in src-tauri)
            if !script_path.exists() {
                script_path = std::env::current_dir()
                    .context("Failed to get current directory")?
                    .parent()
                    .context("No parent directory")?
                    .join("sidecar/build/index.js");
            }

            if !script_path.exists() {
                anyhow::bail!(
                    "Sidecar script not found at {:?}. Run 'cd sidecar && npm run build' first.",
                    script_path
                );
            }

            log::info!("Running sidecar in development mode: node {:?}", script_path);

            // Check for ROON_HOST environment variable for manual connection
            let mut cmd = Command::new("node");
            cmd.arg(&script_path)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());

            // Pass through ROON_HOST and ROON_PORT if set
            if let Ok(host) = std::env::var("ROON_HOST") {
                log::info!("Using manual Roon Core address: {}", host);
                cmd.env("ROON_HOST", host);
            }
            if let Ok(port) = std::env::var("ROON_PORT") {
                cmd.env("ROON_PORT", port);
            }

            cmd.spawn()
                .context("Failed to spawn sidecar with node")?
        } else {
            // Production mode: use bundled binary
            log::info!("Running sidecar in production mode");

            // Resolve the sidecar binary path using Tauri's resource API
            let resource_path = app.path().resource_dir()
                .context("Failed to get resource directory")?
                .join("../MacOS/roon-sidecar");

            let sidecar_path = resource_path.to_str()
                .context("Failed to convert sidecar path to string")?;

            log::info!("Sidecar path: {}", sidecar_path);

            // Check if sidecar exists
            if !resource_path.exists() {
                anyhow::bail!("Sidecar binary not found at {:?}", resource_path);
            }

            let mut cmd = Command::new(sidecar_path);
            cmd.stdout(Stdio::piped())
                .stderr(Stdio::piped());

            // Pass through ROON_HOST and ROON_PORT if set
            if let Ok(host) = std::env::var("ROON_HOST") {
                log::info!("Using manual Roon Core address: {}", host);
                cmd.env("ROON_HOST", host);
            }
            if let Ok(port) = std::env::var("ROON_PORT") {
                cmd.env("ROON_PORT", port);
            }

            cmd.spawn()
                .context("Failed to spawn sidecar process")?
        };

        log::info!("Sidecar process spawned with PID: {}", child.id());

        // Get stdout and stderr
        let stdout = child
            .stdout
            .take()
            .context("Failed to capture sidecar stdout")?;

        let stderr = child
            .stderr
            .take()
            .context("Failed to capture sidecar stderr")?;

        // Store the child process
        *self.child.lock().unwrap() = Some(child);

        // Spawn thread to read stdout (JSON messages)
        let app_handle = app.clone();
        let state_clone = state.clone();
        thread::spawn(move || {
            Self::read_stdout(stdout, app_handle, state_clone);
        });

        // Spawn thread to read stderr (debug logs)
        thread::spawn(move || {
            Self::read_stderr(stderr);
        });

        Ok(())
    }

    /// Read stdout from the sidecar (JSON messages)
    fn read_stdout<R: Runtime>(
        stdout: std::process::ChildStdout,
        app: AppHandle<R>,
        state: SharedState,
    ) {
        let reader = BufReader::new(stdout);

        for line in reader.lines() {
            match line {
                Ok(line) => {
                    if line.trim().is_empty() {
                        continue;
                    }

                    log::debug!("Sidecar stdout: {}", line);

                    // Parse JSON message
                    match serde_json::from_str::<SidecarMessage>(&line) {
                        Ok(message) => {
                            if let Err(e) = Self::handle_message(message, &app, &state) {
                                log::error!("Error handling sidecar message: {}", e);
                            }
                        }
                        Err(e) => {
                            log::error!("Failed to parse sidecar message: {} - {}", e, line);
                        }
                    }
                }
                Err(e) => {
                    log::error!("Error reading sidecar stdout: {}", e);
                    break;
                }
            }
        }

        log::warn!("Sidecar stdout reader stopped");
    }

    /// Read stderr from the sidecar (debug logs)
    fn read_stderr(stderr: std::process::ChildStderr) {
        let reader = BufReader::new(stderr);

        for line in reader.lines() {
            match line {
                Ok(line) => {
                    if !line.trim().is_empty() {
                        log::info!("[Sidecar] {}", line);
                    }
                }
                Err(e) => {
                    log::error!("Error reading sidecar stderr: {}", e);
                    break;
                }
            }
        }

        log::warn!("Sidecar stderr reader stopped");
    }

    /// Handle a message from the sidecar
    fn handle_message<R: Runtime>(
        message: SidecarMessage,
        app: &AppHandle<R>,
        state: &SharedState,
    ) -> Result<()> {
        match message {
            SidecarMessage::NowPlaying {
                title,
                artist,
                album,
                state: playback_state,
                artwork,
            } => {
                log::info!("Now playing: {} - {} ({:?})", title, artist, playback_state);

                // Update app state
                let track_data = NowPlayingData {
                    title,
                    artist,
                    album,
                    state: playback_state,
                    artwork,
                };

                // Update state using tokio runtime
                {
                    let state_clone = state.clone();
                    let track_data_clone = track_data.clone();
                    tauri::async_runtime::spawn(async move {
                        let mut state_guard = state_clone.write().await;
                        state_guard.current_track = Some(track_data_clone);
                    });
                }

                // Update tray icon
                TrayManager::update_icon(app, state.clone())?;
            }
            SidecarMessage::Status { state: status_str, message } => {
                log::info!("Sidecar status: {} - {:?}", status_str, message);

                // Update connection status
                let status = match status_str.as_str() {
                    "discovering" => ConnectionStatus::Discovering,
                    "connected" => ConnectionStatus::Connected,
                    "disconnected" => ConnectionStatus::Disconnected,
                    "not_authorized" => ConnectionStatus::Error(
                        "Not authorized. Please enable extension in Roon.".to_string(),
                    ),
                    _ => ConnectionStatus::Error(format!("Unknown status: {}", status_str)),
                };

                let state_clone = state.clone();
                tauri::async_runtime::spawn(async move {
                    let mut state_guard = state_clone.write().await;
                    state_guard.connection_status = status;
                });
            }
            SidecarMessage::Error { message } => {
                log::error!("Sidecar error: {}", message);

                let state_clone = state.clone();
                tauri::async_runtime::spawn(async move {
                    let mut state_guard = state_clone.write().await;
                    state_guard.connection_status = ConnectionStatus::Error(message);
                });
            }
        }

        Ok(())
    }

    /// Check if the sidecar is still running
    pub fn is_running(&self) -> bool {
        let mut child_guard = self.child.lock().unwrap();
        if let Some(child) = child_guard.as_mut() {
            match child.try_wait() {
                Ok(Some(_status)) => {
                    log::warn!("Sidecar process has exited");
                    false
                }
                Ok(None) => true,
                Err(e) => {
                    log::error!("Error checking sidecar status: {}", e);
                    false
                }
            }
        } else {
            false
        }
    }

    /// Stop the sidecar process
    pub fn stop(&self) -> Result<()> {
        let child_option = self.child.lock().unwrap().take();
        if let Some(mut child) = child_option {
            log::info!("Stopping sidecar process with PID {}...", child.id());

            // Send SIGTERM for graceful shutdown
            #[cfg(unix)]
            {
                use std::process::Command;
                let pid = child.id();
                log::info!("Sending SIGTERM to sidecar process {}", pid);

                // Use kill command to send SIGTERM
                // This is more portable than using libc directly
                let _ = Command::new("kill")
                    .arg("-TERM")
                    .arg(pid.to_string())
                    .output();
            }

            // On Windows, just try to kill it
            #[cfg(windows)]
            {
                log::info!("Killing sidecar process (Windows)");
                child.kill().ok();
            }

            // Wait for graceful shutdown (up to 2 seconds)
            let max_wait_ms = 2000;
            let check_interval_ms = 100;
            let mut waited_ms = 0;

            while waited_ms < max_wait_ms {
                thread::sleep(Duration::from_millis(check_interval_ms));
                waited_ms += check_interval_ms;

                match child.try_wait() {
                    Ok(Some(status)) => {
                        log::info!("Sidecar process exited gracefully with status: {:?}", status);
                        return Ok(());
                    }
                    Ok(None) => {
                        // Still running, continue waiting
                        continue;
                    }
                    Err(e) => {
                        log::error!("Error checking sidecar status: {}", e);
                        break;
                    }
                }
            }

            // If we get here, the process didn't exit gracefully
            log::warn!("Sidecar didn't stop after {}ms, sending SIGKILL...", max_wait_ms);
            child.kill().context("Failed to kill sidecar process")?;
            child.wait().context("Failed to wait for sidecar process")?;
            log::info!("Sidecar process forcefully terminated");
        }

        Ok(())
    }
}

impl Drop for SidecarManager {
    fn drop(&mut self) {
        log::info!("SidecarManager Drop called, cleaning up...");
        if let Err(e) = self.stop() {
            log::error!("Error stopping sidecar in Drop: {}", e);
        }
    }
}

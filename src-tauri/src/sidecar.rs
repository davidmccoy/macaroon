use anyhow::{Context, Result};
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use tauri::{AppHandle, Manager, Runtime};

use crate::state::SharedState;
use crate::tray::TrayManager;
use crate::types::{ConnectionStatus, NowPlayingData, SidecarMessage, Zone};
use std::time::Instant;

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

    /// Check if zones have meaningfully changed (number, IDs, names, or states)
    fn zones_changed(old_zones: &[Zone], new_zones: &[Zone]) -> bool {
        // Different number of zones
        if old_zones.len() != new_zones.len() {
            return true;
        }

        // Check each zone
        for new_zone in new_zones {
            match old_zones.iter().find(|z| z.zone_id == new_zone.zone_id) {
                None => return true, // New zone appeared
                Some(old_zone) => {
                    // Check if display name or state changed
                    if old_zone.display_name != new_zone.display_name || old_zone.state != new_zone.state {
                        return true;
                    }
                }
            }
        }

        // Check if any old zones disappeared
        for old_zone in old_zones {
            if !new_zones.iter().any(|z| z.zone_id == old_zone.zone_id) {
                return true;
            }
        }

        false
    }

    /// Handle a message from the sidecar
    fn handle_message<R: Runtime>(
        message: SidecarMessage,
        app: &AppHandle<R>,
        state: &SharedState,
    ) -> Result<()> {
        match message {
            SidecarMessage::NowPlaying {
                zone_id,
                title,
                artist,
                album,
                state: playback_state,
                artwork,
            } => {
                log::info!("Now playing in zone {}: {} - {} ({:?})", zone_id, title, artist, playback_state);

                // Update app state
                let track_data = NowPlayingData {
                    title,
                    artist,
                    album,
                    state: playback_state,
                    artwork,
                };

                // Update state
                {
                    let mut state_guard = state.write();
                    state_guard.current_track = Some(track_data.clone());

                    // Also update the specific zone's now_playing data
                    if let Some(zone) = state_guard.all_zones.iter_mut().find(|z| z.zone_id == zone_id) {
                        zone.now_playing = Some(track_data);
                        zone.state_changed_at = Instant::now();
                    }
                }

                // Update tray icon (state is now guaranteed to be updated)
                TrayManager::update_icon(app, state.clone())?;
            }
            SidecarMessage::ZoneList { zones } => {
                log::warn!(">>> ZONE_LIST received: {} zones", zones.len());
                for z in &zones {
                    log::warn!("    - {} ({}): {:?}", z.display_name, z.zone_id, z.state);
                }

                // Compute derived values while holding the lock
                let (zones_changed, is_initial_load, new_zones_count, needs_rebuild) = {
                    let mut state_guard = state.write();

                    // Capture old zone count for initial load detection
                    let old_zones_count = state_guard.all_zones.len();

                    // Convert ZoneInfo to Zone
                    let now = Instant::now();
                    let new_zones: Vec<Zone> = zones.into_iter().map(|zone_info| {
                        // Find existing zone to preserve state_changed_at
                        let state_changed_at = state_guard.all_zones
                            .iter()
                            .find(|z| z.zone_id == zone_info.zone_id)
                            .map(|z| z.state_changed_at)
                            .unwrap_or(now);

                        let state_clone = zone_info.state.clone();
                        Zone {
                            zone_id: zone_info.zone_id,
                            display_name: zone_info.display_name,
                            state: zone_info.state,
                            now_playing: zone_info.now_playing.map(|np| NowPlayingData {
                                title: np.title,
                                artist: np.artist,
                                album: np.album,
                                state: state_clone.clone(),
                                artwork: np.artwork,
                            }),
                            state_changed_at,
                        }
                    }).collect();

                    // Check if zones actually changed (to avoid unnecessary rebuilds)
                    let zones_changed = Self::zones_changed(&state_guard.all_zones, &new_zones);

                    // Check if this is the initial zone load BEFORE updating state
                    let is_initial_load = old_zones_count == 0 && !new_zones.is_empty();
                    let new_zones_count = new_zones.len();

                    log::warn!("!!! zones_changed={}, is_initial_load={}, old_count={}, new_count={}",
                        zones_changed, is_initial_load, old_zones_count, new_zones_count);

                    state_guard.all_zones = new_zones;

                    if zones_changed {
                        log::warn!("!!! ZONES CHANGED - updating state");
                        for zone in &state_guard.all_zones {
                            log::warn!("    Zone: {} ({}): {:?}", zone.display_name, zone.zone_id, zone.state);
                        }
                    } else {
                        log::warn!("!!! ZONES NOT CHANGED - no state update");
                    }

                    // Determine if we need to rebuild the menu
                    let needs_rebuild = if is_initial_load {
                        // Skip rebuild for the very first zone load to prevent menu closing on first interaction
                        log::warn!("!!! INITIAL LOAD: Skipping rebuild (0 → {})", new_zones_count);
                        log::warn!("!!! Setting needs_menu_rebuild flag for next update");
                        state_guard.needs_menu_rebuild = true;
                        false // Don't rebuild now
                    } else {
                        let force_rebuild = state_guard.needs_menu_rebuild;

                        if force_rebuild {
                            log::warn!("!!! FORCED REBUILD: needs_menu_rebuild flag is set");
                            true
                        } else if zones_changed {
                            // Zones actually changed - check debounce
                            let should_rebuild = match state_guard.last_menu_rebuild {
                                None => true,
                                Some(last_rebuild) => last_rebuild.elapsed().as_secs() >= 1,
                            };

                            if should_rebuild {
                                log::warn!("!!! ZONES CHANGED: Rebuilding menu");
                                true
                            } else {
                                log::warn!("!!! ZONES CHANGED but debounced (too soon)");
                                false
                            }
                        } else {
                            log::warn!("!!! ZONES NOT CHANGED - no rebuild needed");
                            false
                        }
                    };

                    (zones_changed, is_initial_load, new_zones_count, needs_rebuild)
                };

                if needs_rebuild {
                    log::warn!("╔═══ EXECUTING REBUILD ═══");
                    if let Err(e) = TrayManager::rebuild_menu(app, state) {
                        log::error!("Failed to rebuild menu: {}", e);
                    } else {
                        let mut state_guard = state.write();
                        state_guard.last_menu_rebuild = Some(Instant::now());
                        state_guard.needs_menu_rebuild = false; // Clear the flag
                        log::warn!("╚═══ REBUILD COMPLETE ═══");
                    }
                }

                // Suppress unused variable warnings
                let _ = (zones_changed, is_initial_load, new_zones_count);
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

                let mut state_guard = state.write();
                state_guard.connection_status = status;
            }
            SidecarMessage::Error { message } => {
                log::error!("Sidecar error: {}", message);

                let mut state_guard = state.write();
                state_guard.connection_status = ConnectionStatus::Error(message);
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

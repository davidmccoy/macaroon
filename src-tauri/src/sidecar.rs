use anyhow::{Context, Result};
use parking_lot::Mutex;
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager, Runtime};

use crate::state::SharedState;
use crate::tray::TrayManager;
use crate::types::{ConnectionStatus, NowPlayingData, SidecarMessage, Zone, ZonePreference};

/// Maximum size for a single IPC message line (1MB should be plenty for base64 artwork)
const MAX_MESSAGE_SIZE: usize = 1024 * 1024;

/// Configuration for sidecar restart backoff
const RESTART_INITIAL_DELAY_MS: u64 = 1000;
const RESTART_MAX_DELAY_MS: u64 = 30000;
const RESTART_MULTIPLIER: u64 = 2;

/// Lightweight handle for restart operations - does NOT have Drop impl
/// This is passed to reader threads to avoid deadlock when the thread exits
#[derive(Clone)]
struct RestartHandle {
    restart_count: Arc<Mutex<u32>>,
    shutdown_flag: Arc<AtomicBool>,
}

impl RestartHandle {
    /// Reset the restart counter (call after successful connection)
    fn reset_restart_count(&self) {
        *self.restart_count.lock() = 0;
    }

    /// Get the current restart delay based on exponential backoff
    fn get_restart_delay(&self) -> Duration {
        let count = *self.restart_count.lock();
        let delay_ms = RESTART_INITIAL_DELAY_MS * RESTART_MULTIPLIER.saturating_pow(count);
        Duration::from_millis(delay_ms.min(RESTART_MAX_DELAY_MS))
    }

    /// Increment restart count and return the new value
    fn increment_restart_count(&self) -> u32 {
        let mut count = self.restart_count.lock();
        *count = count.saturating_add(1);
        *count
    }
}

/// Manages the Node.js sidecar process
#[derive(Clone)]
pub struct SidecarManager {
    child: Arc<Mutex<Option<Child>>>,
    /// Keep stdin handle alive to prevent sidecar from detecting parent death
    stdin_handle: Arc<Mutex<Option<std::process::ChildStdin>>>,
    reader_handles: Arc<Mutex<Vec<JoinHandle<()>>>>,
    shutdown_flag: Arc<AtomicBool>,
    restart_count: Arc<Mutex<u32>>,
}

impl SidecarManager {
    pub fn new() -> Self {
        Self {
            child: Arc::new(Mutex::new(None)),
            stdin_handle: Arc::new(Mutex::new(None)),
            reader_handles: Arc::new(Mutex::new(Vec::new())),
            shutdown_flag: Arc::new(AtomicBool::new(false)),
            restart_count: Arc::new(Mutex::new(0)),
        }
    }

    /// Create a lightweight restart handle for use in reader threads
    fn restart_handle(&self) -> RestartHandle {
        RestartHandle {
            restart_count: self.restart_count.clone(),
            shutdown_flag: self.shutdown_flag.clone(),
        }
    }

    /// Spawn the sidecar process and start reading its output
    pub fn spawn<R: Runtime>(
        &self,
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
                .stdin(Stdio::piped())
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
            cmd.stdin(Stdio::piped())
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
                .context("Failed to spawn sidecar process")?
        };

        log::info!("Sidecar process spawned with PID: {}", child.id());

        // Get stdin, stdout and stderr
        let stdin = child
            .stdin
            .take()
            .context("Failed to capture sidecar stdin")?;

        let stdout = child
            .stdout
            .take()
            .context("Failed to capture sidecar stdout")?;

        let stderr = child
            .stderr
            .take()
            .context("Failed to capture sidecar stderr")?;

        // Store the child process and stdin handle
        // Keeping stdin handle alive prevents the sidecar from detecting parent death
        *self.child.lock() = Some(child);
        *self.stdin_handle.lock() = Some(stdin);

        // Reset shutdown flag for new spawn
        self.shutdown_flag.store(false, Ordering::SeqCst);

        // Spawn thread to read stdout (JSON messages)
        let app_handle = app.clone();
        let state_clone = state.clone();
        let shutdown_flag_stdout = self.shutdown_flag.clone();
        let restart_handle = self.restart_handle();
        let stdout_handle = thread::spawn(move || {
            Self::read_stdout(stdout, app_handle, state_clone, shutdown_flag_stdout, restart_handle);
        });

        // Spawn thread to read stderr (debug logs)
        let shutdown_flag_stderr = self.shutdown_flag.clone();
        let stderr_handle = thread::spawn(move || {
            Self::read_stderr(stderr, shutdown_flag_stderr);
        });

        // Store handles for joining later
        {
            let mut handles = self.reader_handles.lock();
            handles.push(stdout_handle);
            handles.push(stderr_handle);
        }

        Ok(())
    }

    /// Read stdout from the sidecar (JSON messages)
    /// Uses RestartHandle instead of SidecarManager to avoid deadlock on drop
    fn read_stdout<R: Runtime>(
        stdout: std::process::ChildStdout,
        app: AppHandle<R>,
        state: SharedState,
        shutdown_flag: Arc<AtomicBool>,
        restart_handle: RestartHandle,
    ) {
        let reader = BufReader::new(stdout);

        for line in reader.lines() {
            // Check if we should stop
            if shutdown_flag.load(Ordering::SeqCst) {
                log::debug!("Sidecar stdout reader received shutdown signal");
                break;
            }

            match line {
                Ok(line) => {
                    if line.trim().is_empty() {
                        continue;
                    }

                    // Validate message size to prevent OOM attacks
                    if line.len() > MAX_MESSAGE_SIZE {
                        log::error!(
                            "Sidecar message exceeds size limit ({} > {}), discarding",
                            line.len(), MAX_MESSAGE_SIZE
                        );
                        continue;
                    }

                    log::debug!("Sidecar stdout: {}", &line[..line.len().min(200)]);

                    // Parse JSON message
                    match serde_json::from_str::<SidecarMessage>(&line) {
                        Ok(message) => {
                            // Reset restart count on successful connection
                            if matches!(message, SidecarMessage::Status { ref state, .. } if state == "connected") {
                                restart_handle.reset_restart_count();
                            }

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
                    if !shutdown_flag.load(Ordering::SeqCst) {
                        log::error!("Error reading sidecar stdout: {}", e);
                    }
                    break;
                }
            }
        }

        // Check if we should attempt restart
        if !shutdown_flag.load(Ordering::SeqCst) {
            log::warn!("Sidecar stdout reader stopped unexpectedly, scheduling restart...");

            // Update connection status to show disconnection
            {
                let mut state_guard = state.write();
                state_guard.connection_status = ConnectionStatus::Error("Sidecar process exited".to_string());
            }

            // Trigger icon update to show disconnected state
            let app_for_icon = app.clone();
            let state_for_icon = state.clone();
            if let Err(e) = app.run_on_main_thread(move || {
                if let Err(e) = TrayManager::update_icon(&app_for_icon, &state_for_icon) {
                    log::error!("Failed to update icon after sidecar exit: {}", e);
                }
            }) {
                log::error!("Failed to dispatch icon update: {}", e);
            }

            // Schedule restart with backoff in a new thread
            // We use RestartHandle (not SidecarManager) to avoid deadlock when this thread exits
            let restart_handle_clone = restart_handle.clone();
            let restart_app = app.clone();
            let restart_state = state.clone();

            thread::spawn(move || {
                let restart_count = restart_handle_clone.increment_restart_count();
                let delay = restart_handle_clone.get_restart_delay();
                log::info!("Sidecar restart #{} scheduled in {:?}", restart_count, delay);

                // Sleep for backoff delay
                thread::sleep(delay);

                // Check if we should still restart
                if !restart_handle_clone.shutdown_flag.load(Ordering::SeqCst) {
                    log::info!("Attempting to restart sidecar...");
                    // Get the SidecarManager from app state to spawn
                    if let Some(manager) = restart_app.try_state::<SidecarManager>() {
                        if let Err(e) = manager.spawn(&restart_app, restart_state.clone()) {
                            log::error!("Failed to restart sidecar: {}", e);
                            // Update status to show error
                            let mut state_guard = restart_state.write();
                            state_guard.connection_status = ConnectionStatus::Error(format!("Restart failed: {}", e));
                        } else {
                            log::info!("Sidecar restarted successfully");
                        }
                    } else {
                        log::error!("SidecarManager not found in app state, cannot restart");
                    }
                }
            });
        } else {
            log::debug!("Sidecar stdout reader stopped (shutdown)");
        }
    }

    /// Read stderr from the sidecar (debug logs)
    fn read_stderr(stderr: std::process::ChildStderr, shutdown_flag: Arc<AtomicBool>) {
        let reader = BufReader::new(stderr);

        for line in reader.lines() {
            // Check if we should stop
            if shutdown_flag.load(Ordering::SeqCst) {
                log::debug!("Sidecar stderr reader received shutdown signal");
                break;
            }

            match line {
                Ok(line) => {
                    if !line.trim().is_empty() {
                        log::info!("[Sidecar] {}", line);
                    }
                }
                Err(e) => {
                    if !shutdown_flag.load(Ordering::SeqCst) {
                        log::error!("Error reading sidecar stderr: {}", e);
                    }
                    break;
                }
            }
        }

        if !shutdown_flag.load(Ordering::SeqCst) {
            log::warn!("Sidecar stderr reader stopped unexpectedly");
        } else {
            log::debug!("Sidecar stderr reader stopped (shutdown)");
        }
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
                // Handle sentinel zone_id indicating disconnection
                if zone_id == "__disconnected__" {
                    log::info!("Received disconnection signal from sidecar");
                    let mut state_guard = state.write();
                    state_guard.current_track = None;
                    state_guard.active_zone_id = None;
                    drop(state_guard);

                    let app_clone = app.clone();
                    let state_clone = state.clone();
                    if let Err(e) = app.run_on_main_thread(move || {
                        if let Err(e) = TrayManager::update_icon(&app_clone, &state_clone) {
                            log::error!("Failed to update icon after disconnect: {}", e);
                        }
                    }) {
                        log::error!("Failed to dispatch icon update to main thread: {}", e);
                    }
                    return Ok(());
                }

                log::debug!("Now playing in zone {}: {} - {} ({:?})", zone_id, title, artist, playback_state);

                // Update app state
                let track_data = NowPlayingData {
                    title,
                    artist,
                    album,
                    state: playback_state,
                    artwork,
                };

                // Update state - only update current_track if this is the selected zone
                let should_update_icon = {
                    let mut state_guard = state.write();

                    // Always update the specific zone's now_playing data
                    if let Some(zone) = state_guard.all_zones.iter_mut().find(|z| z.zone_id == zone_id) {
                        zone.now_playing = Some(track_data.clone());
                        zone.state_changed_at = Instant::now();
                    }

                    // Check if this zone is the one we should display
                    let is_selected_zone = match &state_guard.zone_preference {
                        ZonePreference::Auto => {
                            // In Auto mode:
                            // 1. If we already have an active zone showing this content, keep showing it
                            // 2. If no active zone, prefer a playing zone over just any zone
                            if state_guard.active_zone_id.as_ref() == Some(&zone_id) {
                                true
                            } else if state_guard.active_zone_id.is_none() {
                                // Only auto-select if this zone is actually playing
                                // This prevents showing the first paused/stopped zone arbitrarily
                                track_data.state == crate::types::PlaybackState::Playing
                            } else {
                                false
                            }
                        }
                        ZonePreference::Selected { zone_id: selected_id, .. } => {
                            selected_id == &zone_id
                        }
                    };

                    if is_selected_zone {
                        state_guard.current_track = Some(track_data);
                        state_guard.active_zone_id = Some(zone_id.clone());
                        true
                    } else {
                        false
                    }
                };

                // Only update tray icon if this was the selected zone
                // Must run on main thread for macOS compatibility
                if should_update_icon {
                    let app_clone = app.clone();
                    let state_clone = state.clone();
                    if let Err(e) = app.run_on_main_thread(move || {
                        if let Err(e) = TrayManager::update_icon(&app_clone, &state_clone) {
                            log::error!("Failed to update icon: {}", e);
                        }
                    }) {
                        log::error!("Failed to dispatch icon update to main thread: {}", e);
                    }
                }
            }
            SidecarMessage::ZoneList { zones } => {
                log::debug!("Zone list received: {} zones", zones.len());

                // Compute derived values while holding the lock
                let (needs_rebuild, needs_icon_update) = {
                    let mut state_guard = state.write();

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

                    // Check if active zone changed to stopped/paused - need to update icon
                    let needs_icon_update = if let Some(active_id) = &state_guard.active_zone_id {
                        if let Some(new_zone) = new_zones.iter().find(|z| &z.zone_id == active_id) {
                            // Update current_track state from zone list
                            if let Some(ref mut current) = state_guard.current_track {
                                if current.state != new_zone.state {
                                    current.state = new_zone.state.clone();
                                    true
                                } else {
                                    false
                                }
                            } else {
                                false
                            }
                        } else {
                            false
                        }
                    } else {
                        false
                    };

                    state_guard.all_zones = new_zones;

                    // Determine if we need to rebuild the menu
                    // Use simple debounce: rebuild if zones changed and 1 second has passed
                    let needs_rebuild = if zones_changed {
                        match state_guard.last_menu_rebuild {
                            None => true, // First rebuild ever
                            Some(last_rebuild) => last_rebuild.elapsed().as_secs() >= 1,
                        }
                    } else {
                        false
                    };

                    // Update last_menu_rebuild timestamp atomically with the decision
                    // This prevents race conditions where multiple updates could trigger rebuilds
                    if needs_rebuild {
                        state_guard.last_menu_rebuild = Some(Instant::now());
                    }

                    (needs_rebuild, needs_icon_update)
                };

                if needs_rebuild {
                    // Must run on main thread for macOS compatibility
                    let app_clone = app.clone();
                    let state_clone = state.clone();
                    if let Err(e) = app.run_on_main_thread(move || {
                        if let Err(e) = TrayManager::rebuild_menu(&app_clone, &state_clone) {
                            log::error!("Failed to rebuild menu: {}", e);
                        }
                    }) {
                        log::error!("Failed to dispatch menu rebuild to main thread: {}", e);
                    }
                }

                // Update icon if active zone's state changed (e.g., to stopped)
                if needs_icon_update {
                    let app_clone = app.clone();
                    let state_clone = state.clone();
                    if let Err(e) = app.run_on_main_thread(move || {
                        if let Err(e) = TrayManager::update_icon(&app_clone, &state_clone) {
                            log::error!("Failed to update icon after zone state change: {}", e);
                        }
                    }) {
                        log::error!("Failed to dispatch icon update to main thread: {}", e);
                    }
                }
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

                {
                    let mut state_guard = state.write();
                    state_guard.connection_status = status;
                }

                // Rebuild menu to show status
                let app_clone = app.clone();
                let state_clone = state.clone();
                if let Err(e) = app.run_on_main_thread(move || {
                    if let Err(e) = TrayManager::rebuild_menu(&app_clone, &state_clone) {
                        log::error!("Failed to rebuild menu after status change: {}", e);
                    }
                }) {
                    log::error!("Failed to dispatch menu rebuild to main thread: {}", e);
                }
            }
            SidecarMessage::Error { message } => {
                log::error!("Sidecar error: {}", message);

                {
                    let mut state_guard = state.write();
                    state_guard.connection_status = ConnectionStatus::Error(message);
                }

                // Rebuild menu to show error
                let app_clone = app.clone();
                let state_clone = state.clone();
                if let Err(e) = app.run_on_main_thread(move || {
                    if let Err(e) = TrayManager::rebuild_menu(&app_clone, &state_clone) {
                        log::error!("Failed to rebuild menu after error: {}", e);
                    }
                }) {
                    log::error!("Failed to dispatch menu rebuild to main thread: {}", e);
                }
            }
        }

        Ok(())
    }

    /// Check if the sidecar is still running
    #[allow(dead_code)]
    pub fn is_running(&self) -> bool {
        let mut child_guard = self.child.lock();
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
        // Signal reader threads to stop
        self.shutdown_flag.store(true, Ordering::SeqCst);

        // Drop stdin handle first - this signals the sidecar that parent is closing
        // and allows it to shut down gracefully before we send SIGTERM
        let _ = self.stdin_handle.lock().take();

        let child_option = self.child.lock().take();
        if let Some(mut child) = child_option {
            log::info!("Stopping sidecar process with PID {}...", child.id());

            // Send SIGTERM for graceful shutdown
            #[cfg(unix)]
            {
                use nix::sys::signal::{kill, Signal};
                use nix::unistd::Pid;

                let pid = Pid::from_raw(child.id() as i32);
                log::info!("Sending SIGTERM to sidecar process {}", pid);

                if let Err(e) = kill(pid, Signal::SIGTERM) {
                    log::warn!("Failed to send SIGTERM: {}", e);
                }
            }

            // On Windows, just try to kill it
            #[cfg(windows)]
            {
                log::info!("Killing sidecar process (Windows)");
                child.kill().ok();
            }

            // Wait for graceful shutdown with timeout
            const GRACEFUL_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);
            const CHECK_INTERVAL: Duration = Duration::from_millis(100);
            let start = Instant::now();

            while start.elapsed() < GRACEFUL_SHUTDOWN_TIMEOUT {
                match child.try_wait() {
                    Ok(Some(status)) => {
                        log::info!("Sidecar process exited gracefully with status: {:?}", status);
                        // Join reader threads (they should exit when pipes close)
                        self.join_reader_threads();
                        return Ok(());
                    }
                    Ok(None) => {
                        thread::sleep(CHECK_INTERVAL);
                    }
                    Err(e) => {
                        log::error!("Error checking sidecar status: {}", e);
                        break;
                    }
                }
            }

            // Process didn't exit gracefully, force kill
            log::warn!("Sidecar didn't stop after {:?}, sending SIGKILL...", GRACEFUL_SHUTDOWN_TIMEOUT);
            child.kill().context("Failed to kill sidecar process")?;
            child.wait().context("Failed to wait for sidecar process")?;
            log::info!("Sidecar process forcefully terminated");
        }

        // Join reader threads
        self.join_reader_threads();

        Ok(())
    }

    /// Join all reader threads, with a timeout
    fn join_reader_threads(&self) {
        let handles: Vec<JoinHandle<()>> = {
            let mut handles_guard = self.reader_handles.lock();
            std::mem::take(&mut *handles_guard)
        };

        for handle in handles {
            // Give threads a short time to finish
            // They should exit quickly once the process is dead
            if handle.join().is_err() {
                log::warn!("Failed to join a reader thread");
            }
        }
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

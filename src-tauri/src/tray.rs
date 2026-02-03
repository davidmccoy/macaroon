use anyhow::{Context, Result};
use tauri::{
    image::Image,
    menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem},
    tray::TrayIconBuilder,
    AppHandle, Manager, Runtime,
};

use crate::compositor::Compositor;
use crate::state::SharedState;
use crate::types::{ConnectionStatus, PlaybackState, ZonePreference};

/// TrayManager is stored as a singleton in Tauri's app state.
/// It owns the Compositor which loads the font once at startup.
pub struct TrayManager {
    compositor: Compositor,
}

impl TrayManager {
    /// Create a new TrayManager. This should only be called once during app setup.
    /// If the font fails to load, returns an error that should terminate the app.
    pub fn new() -> Result<Self> {
        let compositor = Compositor::new()
            .context("Failed to initialize compositor - font may be missing")?;
        Ok(Self { compositor })
    }

    /// Initialize the system tray and store TrayManager as app state
    pub fn setup<R: Runtime>(app: &AppHandle<R>, state: SharedState) -> Result<()> {
        // Create the TrayManager singleton - exit if font loading fails
        let manager = TrayManager::new()?;

        // Create initial menu
        let menu = Self::build_menu_internal(app, &state)?;

        // Create initial icon
        let initial_icon = manager.create_placeholder_icon()?;

        // Clone state for menu event handler
        let state_for_menu = state.clone();

        // Build tray icon
        let tray = TrayIconBuilder::new()
            .icon(initial_icon)
            .menu(&menu)
            .on_menu_event(move |app, event| {
                Self::handle_menu_event(app, event, &state_for_menu);
            })
            .build(app)?;

        // Store tray icon in app state
        app.manage(tray);

        // Store TrayManager singleton in app state
        app.manage(manager);

        // Store shared state
        app.manage(state);

        Ok(())
    }

    /// Build the tray menu with zones and status
    fn build_menu_internal<R: Runtime>(app: &AppHandle<R>, state: &SharedState) -> Result<Menu<R>> {
        let state_guard = state.read();
        let menu = Menu::new(app)?;

        // Show connection status if not connected
        match &state_guard.connection_status {
            ConnectionStatus::Disconnected => {
                let item = MenuItem::with_id(app, "status", "Disconnected from Roon", false, None::<&str>)?;
                menu.append(&item)?;
                let separator = PredefinedMenuItem::separator(app)?;
                menu.append(&separator)?;
            }
            ConnectionStatus::Discovering => {
                let item = MenuItem::with_id(app, "status", "Searching for Roon Core...", false, None::<&str>)?;
                menu.append(&item)?;
                let separator = PredefinedMenuItem::separator(app)?;
                menu.append(&separator)?;
            }
            ConnectionStatus::Error(msg) => {
                let label = format!("Error: {}", msg);
                let item = MenuItem::with_id(app, "status", &label, false, None::<&str>)?;
                menu.append(&item)?;
                let separator = PredefinedMenuItem::separator(app)?;
                menu.append(&separator)?;
            }
            ConnectionStatus::Connected => {
                // Connected - show zones below
            }
        }

        // Add zone items
        if state_guard.all_zones.is_empty() {
            let no_zones = MenuItem::with_id(app, "no_zones", "No zones available", false, None::<&str>)?;
            menu.append(&no_zones)?;
        } else {
            for zone in &state_guard.all_zones {
                let is_selected = match &state_guard.zone_preference {
                    ZonePreference::Selected { zone_id, .. } => zone_id == &zone.zone_id,
                    ZonePreference::Auto => false,
                };

                let state_str = match zone.state {
                    PlaybackState::Playing => "Playing",
                    PlaybackState::Paused => "Paused",
                    PlaybackState::Stopped => "Stopped",
                    PlaybackState::Loading => "Loading",
                };

                let label = format!("{} ({})", zone.display_name, state_str);

                let item = CheckMenuItem::with_id(
                    app,
                    &zone.zone_id,
                    label,
                    true,
                    is_selected,
                    None::<&str>,
                )?;
                menu.append(&item)?;
            }
        }

        // Separator and quit
        let separator = PredefinedMenuItem::separator(app)?;
        menu.append(&separator)?;

        let quit_item = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
        menu.append(&quit_item)?;

        Ok(menu)
    }

    /// Handle menu events
    fn handle_menu_event<R: Runtime>(
        app: &AppHandle<R>,
        event: tauri::menu::MenuEvent,
        state: &SharedState,
    ) {
        let menu_id = event.id().as_ref();

        match menu_id {
            "quit" => {
                app.exit(0);
            }
            "no_zones" | "status" => {
                // Disabled items, do nothing
            }
            zone_id => {
                // Zone selection
                log::info!("Zone selected: {}", zone_id);

                {
                    let mut state_guard = state.write();
                    state_guard.zone_preference = ZonePreference::Selected {
                        zone_id: zone_id.to_string(),
                        smart_switching: true,
                        grace_period_mins: 5,
                    };

                    // Reset smart-switch state since user explicitly selected a zone
                    state_guard.is_smart_switched = false;
                    state_guard.preferred_zone_stopped_at = None;

                    // Load the selected zone's now_playing data
                    let zone_data = state_guard.all_zones.iter()
                        .find(|z| z.zone_id == zone_id)
                        .map(|z| (z.now_playing.clone(), z.display_name.clone()));

                    if let Some((now_playing, display_name)) = zone_data {
                        // Only update current_track if the zone has data
                        // Keep existing track if zone is playing but data hasn't arrived yet
                        if now_playing.is_some() {
                            state_guard.current_track = now_playing;
                        }
                        // Always update active zone ID - the track data will arrive shortly
                        state_guard.active_zone_id = Some(zone_id.to_string());
                        log::info!("Selected zone: {}", display_name);
                    } else {
                        // Zone doesn't exist in our list - this shouldn't normally happen
                        // Keep existing track to avoid flicker
                        state_guard.active_zone_id = Some(zone_id.to_string());
                        log::warn!("Selected zone not found in zone list: {}", zone_id);
                    }

                    state_guard.last_menu_rebuild = Some(std::time::Instant::now());
                }

                // Rebuild menu and update icon
                if let Err(e) = Self::rebuild_menu(app, state) {
                    log::error!("Failed to rebuild menu: {}", e);
                }
                if let Err(e) = Self::update_icon(app, state) {
                    log::error!("Failed to update icon after zone selection: {}", e);
                }
            }
        }
    }

    /// Rebuild the tray menu (called when zones change or preference changes)
    pub fn rebuild_menu<R: Runtime>(app: &AppHandle<R>, state: &SharedState) -> Result<()> {
        let new_menu = Self::build_menu_internal(app, state)?;

        if let Some(tray) = app.try_state::<tauri::tray::TrayIcon<R>>() {
            tray.set_menu(Some(new_menu))?;
        }

        Ok(())
    }

    /// Create a placeholder icon (no track playing)
    fn create_placeholder_icon(&self) -> Result<Image<'static>> {
        let icon_bytes = self.compositor.create_menu_bar_icon(None, "", "")?;
        Image::from_bytes(&icon_bytes).context("Failed to create placeholder icon")
    }

    /// Check if dark mode has changed and return true if icon needs updating
    fn check_dark_mode_changed(state: &SharedState) -> bool {
        let current_dark_mode = matches!(dark_light::detect(), dark_light::Mode::Dark);
        let mut state_guard = state.write();
        let changed = state_guard.last_dark_mode != Some(current_dark_mode);
        if changed {
            state_guard.last_dark_mode = Some(current_dark_mode);
            log::info!("System appearance changed to {} mode", if current_dark_mode { "dark" } else { "light" });
        }
        changed
    }

    /// Update the tray icon with current track info.
    /// Uses the TrayManager singleton stored in app state.
    /// Also checks for dark mode changes and re-renders if needed.
    pub fn update_icon<R: Runtime>(app: &AppHandle<R>, state: &SharedState) -> Result<()> {
        // Check if dark mode changed - this updates state.last_dark_mode
        let _ = Self::check_dark_mode_changed(state);

        let manager = app.try_state::<TrayManager>()
            .context("TrayManager not found in app state")?;

        let state_guard = state.read();

        let icon_bytes = match &state_guard.current_track {
            Some(track) if track.state == PlaybackState::Playing => {
                // Show track info with artwork when playing
                manager.compositor.create_menu_bar_icon(
                    track.artwork.as_deref(),
                    &track.title,
                    &track.artist,
                )?
            }
            Some(track) if track.state == PlaybackState::Loading => {
                // Show loading text
                manager.compositor.create_menu_bar_icon(None, "Loading...", "")?
            }
            _ => {
                // Paused, stopped, or no track - show placeholder
                manager.compositor.create_menu_bar_icon(None, "", "")?
            }
        };

        let image = Image::from_bytes(&icon_bytes)
            .context("Failed to create image from bytes")?;

        if let Some(tray) = app.try_state::<tauri::tray::TrayIcon<R>>() {
            tray.set_icon(Some(image))?;
        }

        Ok(())
    }
}

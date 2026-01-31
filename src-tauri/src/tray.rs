use anyhow::{Context, Result};
use tauri::{
    image::Image,
    menu::{Menu, MenuItem, PredefinedMenuItem, Submenu, CheckMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager, Runtime,
};

use crate::compositor::Compositor;
use crate::state::SharedState;
use crate::types::{PlaybackState, ZonePreference};

pub struct TrayManager {
    compositor: Compositor,
}

impl TrayManager {
    pub fn new() -> Result<Self> {
        let compositor = Compositor::new()?;
        Ok(Self { compositor })
    }

    /// Initialize the system tray
    pub fn setup<R: Runtime>(app: &AppHandle<R>, state: SharedState) -> Result<()> {
        // Set initial menu rebuild time
        {
            let mut state_guard = state.write();
            state_guard.last_menu_rebuild = Some(std::time::Instant::now());
        }

        // Create initial menu (should have zones by now if sidecar connected)
        let menu = Self::build_menu(app, &state)?;

        // Create initial tray icon
        let manager = TrayManager::new()?;
        let initial_icon = manager.create_initial_icon()?;

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

        // Store tray in app state for later updates
        app.manage(tray);

        // Store shared state
        app.manage(state);

        Ok(())
    }

    /// Build the tray menu with zones submenu (for rebuild)
    fn build_menu_for_rebuild<R: Runtime>(app: &AppHandle<R>, state: &SharedState) -> Result<Menu<R>> {
        let state_guard = state.read();

        log::warn!(">>> Building menu (rebuild) with {} zones", state_guard.all_zones.len());
        for zone in &state_guard.all_zones {
            log::warn!("    Menu will include: {} ({:?})", zone.display_name, zone.state);
        }

        // Build zones submenu
        let zones_submenu = Self::build_zones_submenu(app, &state_guard)?;

        // Create separator
        let separator = PredefinedMenuItem::separator(app)?;

        // Create quit item
        let quit_item = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;

        // Build final menu
        let menu = Menu::with_items(app, &[
            &zones_submenu,
            &separator,
            &quit_item,
        ])?;

        Ok(menu)
    }

    /// Build the tray menu with zones submenu (for initial setup)
    fn build_menu<R: Runtime>(app: &AppHandle<R>, state: &SharedState) -> Result<Menu<R>> {
        let state_guard = state.read();

        log::info!("Building menu with {} zones", state_guard.all_zones.len());

        // Build zones submenu
        let zones_submenu = Self::build_zones_submenu(app, &state_guard)?;

        // Create separator
        let separator = PredefinedMenuItem::separator(app)?;

        // Create quit item
        let quit_item = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;

        // Build final menu
        let menu = Menu::with_items(app, &[
            &zones_submenu,
            &separator,
            &quit_item,
        ])?;

        Ok(menu)
    }

    /// Build the zones submenu
    fn build_zones_submenu<R: Runtime>(
        app: &AppHandle<R>,
        state_guard: &parking_lot::RwLockReadGuard<crate::types::AppState>,
    ) -> Result<Submenu<R>> {
        // Create submenu first
        let submenu = Submenu::new(app, "Select Zone", true)?;

        if state_guard.all_zones.is_empty() {
            // No zones available yet
            let no_zones = MenuItem::with_id(
                app,
                "no_zones",
                "No zones available",
                false, // disabled
                None::<&str>,
            )?;

            submenu.append(&no_zones)?;
            return Ok(submenu);
        }

        // Add zone menu items
        for zone in &state_guard.all_zones {
            // Check if this is the preferred zone
            let is_preferred = match &state_guard.zone_preference {
                ZonePreference::Selected { zone_id, .. } => zone_id == &zone.zone_id,
                ZonePreference::Auto => false,
            };

            // Check if this zone is currently being displayed
            let is_showing = state_guard.active_zone_id.as_ref() == Some(&zone.zone_id);
            let show_indicator = is_showing && state_guard.is_smart_switched;

            // Format state name
            let state_str = match zone.state {
                PlaybackState::Playing => "Playing",
                PlaybackState::Paused => "Paused",
                PlaybackState::Stopped => "Stopped",
                PlaybackState::Loading => "Loading",
            };

            // Format label
            let label = if show_indicator {
                format!("{} ({}) ← Showing", zone.display_name, state_str)
            } else {
                format!("{} ({})", zone.display_name, state_str)
            };

            // Create check menu item and append to submenu
            let item = CheckMenuItem::with_id(
                app,
                &zone.zone_id,
                label,
                true,         // enabled
                is_preferred, // checked
                None::<&str>,
            )?;

            submenu.append(&item)?;
        }

        Ok(submenu)
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
            "no_zones" => {
                // Disabled item, do nothing
            }
            zone_id => {
                // This is a zone selection
                log::info!("Zone selected: {}", zone_id);

                // Update zone preference
                {
                    let mut state_guard = state.write();
                    state_guard.zone_preference = ZonePreference::Selected {
                        zone_id: zone_id.to_string(),
                        smart_switching: true,  // Default enabled
                        grace_period_mins: 5,   // Default 5 minutes
                    };

                    // Reset smart-switch state since user explicitly selected a zone
                    state_guard.is_smart_switched = false;
                    state_guard.preferred_zone_stopped_at = None;

                    log::info!("Zone preference updated to: {}", zone_id);
                }

                // Rebuild menu to show checkmark on selected zone
                if let Err(e) = Self::rebuild_menu(app, state) {
                    log::error!("Failed to rebuild menu: {}", e);
                }

                // Update last rebuild time
                {
                    let mut state_guard = state.write();
                    state_guard.last_menu_rebuild = Some(std::time::Instant::now());
                }

                // Update tray icon to display the selected zone
                if let Err(e) = Self::update_icon(app, state.clone()) {
                    log::error!("Failed to update icon after zone selection: {}", e);
                }
            }
        }
    }

    /// Rebuild the tray menu (called when zones change or preference changes)
    pub fn rebuild_menu<R: Runtime>(app: &AppHandle<R>, state: &SharedState) -> Result<()> {
        log::warn!("╔═══════════════════════════════");
        log::warn!("║ REBUILD_MENU CALLED");
        log::warn!("╚═══════════════════════════════");

        let new_menu = Self::build_menu_for_rebuild(app, state)?;

        if let Some(tray) = app.try_state::<tauri::tray::TrayIcon>() {
            tray.set_menu(Some(new_menu))?;
            log::warn!(">>> Menu rebuilt and applied to tray");
        }

        Ok(())
    }

    /// Create an initial placeholder icon
    fn create_initial_icon(&self) -> Result<Image> {
        let icon_bytes = self.compositor.create_menu_bar_icon(
            None,
            "Now Playing",
            "Waiting for music...",
        )?;

        Image::from_bytes(&icon_bytes)
            .context("Failed to create image from bytes")
    }

    /// Update the tray icon with current track info
    pub fn update_icon<R: Runtime>(
        app: &AppHandle<R>,
        state: SharedState,
    ) -> Result<()> {
        let manager = TrayManager::new()?;

        // Read current state
        let state_guard = state.read();

        if let Some(track) = &state_guard.current_track {
            match track.state {
                PlaybackState::Playing => {
                    // Show track info with artwork when playing
                    let icon_bytes = manager.compositor.create_menu_bar_icon(
                        track.artwork.as_deref(),
                        &track.title,
                        &track.artist,
                    ).unwrap_or_else(|e| {
                        log::error!("Failed to create icon: {}, using fallback", e);
                        manager.create_fallback_icon()
                            .expect("Fallback icon creation should never fail")
                    });

                    let image = Image::from_bytes(&icon_bytes)
                        .context("Failed to create image from bytes")?;

                    if let Some(tray) = app.try_state::<tauri::tray::TrayIcon>() {
                        tray.set_icon(Some(image))?;
                    }
                }
                PlaybackState::Paused => {
                    // Show just placeholder image with no text when paused
                    let icon_bytes = manager.compositor.create_menu_bar_icon(
                        None,  // No artwork - will show purple placeholder
                        "",    // No title
                        "",    // No artist
                    ).unwrap_or_else(|e| {
                        log::error!("Failed to create paused icon: {}, using fallback", e);
                        manager.create_fallback_icon()
                            .expect("Fallback icon creation should never fail")
                    });

                    let image = Image::from_bytes(&icon_bytes)
                        .context("Failed to create image from bytes")?;

                    if let Some(tray) = app.try_state::<tauri::tray::TrayIcon>() {
                        tray.set_icon(Some(image))?;
                    }
                }
                PlaybackState::Loading => {
                    // Show loading state (similar to paused for now)
                    let icon_bytes = manager.compositor.create_menu_bar_icon(
                        None,  // No artwork - will show purple placeholder
                        "Loading...",
                        "",
                    ).unwrap_or_else(|e| {
                        log::error!("Failed to create loading icon: {}, using fallback", e);
                        manager.create_fallback_icon()
                            .expect("Fallback icon creation should never fail")
                    });

                    let image = Image::from_bytes(&icon_bytes)
                        .context("Failed to create image from bytes")?;

                    if let Some(tray) = app.try_state::<tauri::tray::TrayIcon>() {
                        tray.set_icon(Some(image))?;
                    }
                }
                PlaybackState::Stopped => {
                    // Don't update icon when stopped
                }
            }
        }

        Ok(())
    }

    /// Create a fallback icon when normal icon generation fails
    fn create_fallback_icon(&self) -> Result<Vec<u8>> {
        // Create minimal icon with music note symbol
        self.compositor.create_menu_bar_icon(
            None,
            "♪",  // Music note symbol
            "",
        )
    }

    /// Update icon with test data (for Phase 0 development)
    pub fn update_test_icon<R: Runtime>(
        app: &AppHandle<R>,
        title: &str,
        artist: &str,
    ) -> Result<()> {
        let manager = TrayManager::new()?;

        let icon_bytes = manager.compositor.create_menu_bar_icon(
            None,
            title,
            artist,
        ).unwrap_or_else(|e| {
            log::error!("Failed to create test icon: {}, using fallback", e);
            manager.create_fallback_icon()
                .expect("Fallback icon creation should never fail")
        });

        let image = Image::from_bytes(&icon_bytes)
            .context("Failed to create image from bytes")?;

        if let Some(tray) = app.try_state::<tauri::tray::TrayIcon>() {
            tray.set_icon(Some(image))?;
        }

        Ok(())
    }
}

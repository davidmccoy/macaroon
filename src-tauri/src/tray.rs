use anyhow::{Context, Result};
use tauri::{
    image::Image,
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager, Runtime,
};

use crate::compositor::Compositor;
use crate::state::SharedState;
use crate::types::PlaybackState;

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
        // Create menu items
        let quit_item = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
        let menu = Menu::with_items(app, &[&quit_item])?;

        // Create initial tray icon
        let manager = TrayManager::new()?;
        let initial_icon = manager.create_initial_icon()?;

        // Build tray icon
        let tray = TrayIconBuilder::new()
            .icon(initial_icon)
            .menu(&menu)
            .on_menu_event(move |app, event| match event.id().as_ref() {
                "quit" => {
                    app.exit(0);
                }
                _ => {}
            })
            .on_tray_icon_event(|_tray, event| {
                if let TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                } = event
                {
                    log::info!("Tray icon clicked");
                    // Future: Show popover window
                }
            })
            .build(app)?;

        // Store tray in app state for later updates
        app.manage(tray);

        // Store shared state for Phase 2 sidecar integration
        app.manage(state);

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
        let state_guard = state.blocking_read();

        if let Some(track) = &state_guard.current_track {
            // Only show icon when playing
            if track.state == PlaybackState::Playing || track.state == PlaybackState::Paused {
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

                // Get tray and update icon
                if let Some(tray) = app.try_state::<tauri::tray::TrayIcon>() {
                    tray.set_icon(Some(image))?;
                }
            } else {
                // Hide tray when stopped
                if let Some(tray) = app.try_state::<tauri::tray::TrayIcon>() {
                    // For now, just use a minimal icon
                    // In the future, we can hide the tray entirely
                    let minimal_icon = manager.create_initial_icon()?;
                    tray.set_icon(Some(minimal_icon))?;
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

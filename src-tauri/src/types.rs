use serde::{Deserialize, Serialize};
use std::time::Instant;

/// Sidecar message types - these match the JSON output from the Node.js sidecar
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SidecarMessage {
    NowPlaying {
        zone_id: String, // NEW: Zone identifier
        title: String,
        artist: String,
        album: String,
        state: PlaybackState,
        artwork: Option<String>,
    },
    ZoneList {
        // NEW: List of all zones
        zones: Vec<ZoneInfo>,
    },
    Status {
        state: String,
        message: Option<String>,
    },
    Error {
        message: String,
    },
}

/// Zone information from sidecar
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZoneInfo {
    pub zone_id: String,
    pub display_name: String,
    pub state: PlaybackState,
    pub now_playing: Option<NowPlayingInfo>,
}

/// Minimal now playing info embedded in zone list
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NowPlayingInfo {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub artwork: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NowPlayingData {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub state: PlaybackState,
    pub artwork: Option<String>, // base64 data URL
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum PlaybackState {
    Playing,
    Paused,
    Stopped,
    Loading, // NEW: Loading state
}

/// Zone data tracked in Rust
#[derive(Debug, Clone)]
pub struct Zone {
    pub zone_id: String,
    pub display_name: String,
    pub state: PlaybackState,
    pub now_playing: Option<NowPlayingData>,
    pub state_changed_at: Instant,
}

/// Zone preference - which zone to display
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum ZonePreference {
    Auto,
    Selected {
        zone_id: String,
        #[serde(default = "default_smart_switching")]
        smart_switching: bool,
        #[serde(default = "default_grace_period")]
        grace_period_mins: u32,
    },
}

fn default_smart_switching() -> bool {
    true
}

fn default_grace_period() -> u32 {
    5
}

impl Default for ZonePreference {
    fn default() -> Self {
        ZonePreference::Auto
    }
}

#[derive(Debug, Clone)]
pub struct AppState {
    // Existing fields
    pub current_track: Option<NowPlayingData>,
    pub connection_status: ConnectionStatus,

    // New zone management fields
    pub all_zones: Vec<Zone>,
    pub zone_preference: ZonePreference,
    pub active_zone_id: Option<String>,
    pub preferred_zone_stopped_at: Option<Instant>,
    pub is_smart_switched: bool,
    pub last_menu_rebuild: Option<Instant>,
    pub needs_menu_rebuild: bool, // Force rebuild on next opportunity
}

#[derive(Debug, Clone, PartialEq)]
pub enum ConnectionStatus {
    Disconnected,
    Discovering,
    Connected,
    Error(String),
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            current_track: None,
            connection_status: ConnectionStatus::Disconnected,
            all_zones: Vec::new(),
            zone_preference: ZonePreference::Auto,
            active_zone_id: None,
            preferred_zone_stopped_at: None,
            is_smart_switched: false,
            last_menu_rebuild: None,
            needs_menu_rebuild: false,
        }
    }
}

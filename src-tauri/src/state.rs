use std::sync::Arc;
use parking_lot::RwLock;

use crate::types::AppState;

pub type SharedState = Arc<RwLock<AppState>>;

pub fn create_state() -> SharedState {
    Arc::new(RwLock::new(AppState::default()))
}

//! Tauri commands for managing marketplace sources.
//!
//! Thin wrappers around [`kiro_market_core::service::MarketplaceService`].

use kiro_market_core::git::GitProtocol;
use kiro_market_core::service::{MarketplaceAddResult, UpdateResult};

use crate::commands::make_service;
use crate::error::CommandError;

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// Add a new marketplace source.
#[tauri::command]
#[specta::specta]
pub async fn add_marketplace(
    source: String,
    protocol: Option<GitProtocol>,
) -> Result<MarketplaceAddResult, CommandError> {
    let svc = make_service()?;
    let protocol = protocol.unwrap_or_default();
    svc.add(&source, protocol).map_err(CommandError::from)
}

/// Remove a registered marketplace and its cached data.
#[tauri::command]
#[specta::specta]
pub async fn remove_marketplace(name: String) -> Result<(), CommandError> {
    let svc = make_service()?;
    svc.remove(&name).map_err(CommandError::from)
}

/// Update marketplace clone(s) from remote.
#[tauri::command]
#[specta::specta]
pub async fn update_marketplace(name: Option<String>) -> Result<UpdateResult, CommandError> {
    let svc = make_service()?;
    svc.update(name.as_deref()).map_err(CommandError::from)
}

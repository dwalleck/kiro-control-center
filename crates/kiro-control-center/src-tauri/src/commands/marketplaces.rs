//! Tauri commands for managing marketplace sources.
//!
//! Thin wrappers around [`kiro_market_core::service::MarketplaceService`].

use kiro_market_core::cache::CacheDir;
use kiro_market_core::git::{GitProtocol, GixCliBackend};
use kiro_market_core::service::{MarketplaceAddResult, MarketplaceService, UpdateResult};

use crate::error::{CommandError, ErrorType};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Obtain the `CacheDir`, returning a `CommandError` if the data directory
/// cannot be determined.
fn get_cache() -> Result<CacheDir, CommandError> {
    CacheDir::default_location().ok_or_else(|| {
        CommandError::new(
            "could not determine data directory; is $HOME set?",
            ErrorType::IoError,
        )
    })
}

fn service() -> Result<MarketplaceService, CommandError> {
    let cache = get_cache()?;
    Ok(MarketplaceService::new(cache, GixCliBackend::default()))
}

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
    let svc = service()?;
    let protocol = protocol.unwrap_or_default();
    svc.add(&source, protocol).map_err(CommandError::from)
}

/// Remove a registered marketplace and its cached data.
#[tauri::command]
#[specta::specta]
pub async fn remove_marketplace(name: String) -> Result<(), CommandError> {
    let svc = service()?;
    svc.remove(&name).map_err(CommandError::from)
}

/// Update marketplace clone(s) from remote.
#[tauri::command]
#[specta::specta]
pub async fn update_marketplace(name: Option<String>) -> Result<UpdateResult, CommandError> {
    let svc = service()?;
    svc.update(name.as_deref()).map_err(CommandError::from)
}

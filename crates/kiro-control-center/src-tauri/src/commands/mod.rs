pub mod browse;
pub mod installed;
pub mod kiro_settings;
pub mod marketplaces;
pub mod settings;
pub mod steering;

use kiro_market_core::cache::CacheDir;
use kiro_market_core::git::GixCliBackend;
use kiro_market_core::service::MarketplaceService;

use crate::error::{CommandError, ErrorType};

/// Construct a [`MarketplaceService`] for read-side and install-only
/// command handlers. Centralized here so every `#[tauri::command]` wrapper
/// resolves the cache directory and `GitBackend` the same way; previously
/// the body was duplicated in every command file.
///
/// All current callers are read-only or install-only; the [`GixCliBackend`]
/// is unused on every code path, so the default backend is fine. If a
/// command grows that needs a different backend, take the service as a
/// parameter on the `_impl` instead of branching here.
pub(in crate::commands) fn make_service() -> Result<MarketplaceService, CommandError> {
    let cache = CacheDir::default_location().ok_or_else(|| {
        CommandError::new(
            "could not determine data directory; is $HOME set?",
            ErrorType::IoError,
        )
    })?;
    Ok(MarketplaceService::new(cache, GixCliBackend::default()))
}

/// Fail-fast validation of a Tauri-supplied `project_path` before it
/// flows into [`kiro_market_core::project::KiroProject::new`] (which is
/// infallible). Rejects:
///
/// - an empty string — frontend default-construction would otherwise
///   silently write to `./.kiro/...` relative to the Tauri process cwd
///   instead of the user's project,
/// - a non-existent path,
/// - a path with no `.kiro/` subdirectory.
///
/// Without this guard, the install layer's per-file failures wouldn't
/// fire (it would create `.kiro/` on the wrong root), so the user sees
/// "install succeeded" with bytes landing nowhere near their project.
pub(in crate::commands) fn validate_kiro_project_path(
    project_path: &str,
) -> Result<(), CommandError> {
    if project_path.is_empty() {
        return Err(CommandError::new(
            "project_path must not be empty",
            ErrorType::Validation,
        ));
    }
    let path = std::path::Path::new(project_path);
    if !path.exists() {
        return Err(CommandError::new(
            format!("project_path `{project_path}` does not exist"),
            ErrorType::Validation,
        ));
    }
    if !path.join(".kiro").is_dir() {
        return Err(CommandError::new(
            format!(
                "project_path `{project_path}` is not a Kiro project \
                 (missing `.kiro/` directory)"
            ),
            ErrorType::Validation,
        ));
    }
    Ok(())
}

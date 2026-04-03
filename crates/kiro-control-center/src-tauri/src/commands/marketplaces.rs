//! Commands for managing marketplace sources.

use std::fs;
use std::path::PathBuf;

use serde::Serialize;
use tracing::{debug, warn};

use kiro_market_core::cache::{CacheDir, KnownMarketplace, MarketplaceSource};
use kiro_market_core::git;
use kiro_market_core::marketplace::Marketplace;
use kiro_market_core::validation;

use crate::error::{CommandError, ErrorType};

// ---------------------------------------------------------------------------
// Response types
// ---------------------------------------------------------------------------

/// Result of adding a new marketplace, including the discovered plugins.
#[derive(Clone, Debug, Serialize, specta::Type)]
pub struct MarketplaceAddResult {
    pub name: String,
    pub plugins: Vec<PluginBasicInfo>,
}

/// Basic information about a plugin within a marketplace.
#[derive(Clone, Debug, Serialize, specta::Type)]
pub struct PluginBasicInfo {
    pub name: String,
    pub description: Option<String>,
}

/// Result of updating one or more marketplaces.
#[derive(Clone, Debug, Serialize, specta::Type)]
pub struct UpdateResult {
    pub updated: Vec<String>,
    pub failed: Vec<FailedUpdate>,
    pub skipped: Vec<String>,
}

/// A marketplace that failed to update, with the reason.
#[derive(Clone, Debug, Serialize, specta::Type)]
pub struct FailedUpdate {
    pub name: String,
    pub error: String,
}

// ---------------------------------------------------------------------------
// Source detection
// ---------------------------------------------------------------------------

/// Classify a user-provided source string into a `MarketplaceSource`.
///
/// Mirrors the CLI `detect_source` logic in `kiro-market`.
fn detect_source(source: &str) -> MarketplaceSource {
    if source.starts_with("http://")
        || source.starts_with("https://")
        || source.starts_with("git@")
    {
        MarketplaceSource::GitUrl {
            url: source.to_owned(),
        }
    } else if source.starts_with('/')
        || source.starts_with("./")
        || source.starts_with("../")
        || source.starts_with('~')
    {
        MarketplaceSource::LocalPath {
            path: source.to_owned(),
        }
    } else {
        // Treat as GitHub owner/repo shorthand.
        MarketplaceSource::GitHub {
            repo: source.to_owned(),
        }
    }
}

/// Resolve a local path string to an absolute path.
///
/// Handles `~` expansion and canonicalization.
fn resolve_local_path(path_str: &str) -> Result<PathBuf, CommandError> {
    let expanded = if let Some(rest) = path_str.strip_prefix('~') {
        let home = dirs::home_dir().ok_or_else(|| {
            CommandError::new(
                "could not determine home directory for ~ expansion",
                ErrorType::IoError,
            )
        })?;
        if rest.is_empty() {
            home
        } else {
            home.join(rest.trim_start_matches('/'))
        }
    } else {
        PathBuf::from(path_str)
    };

    expanded.canonicalize().map_err(|e| {
        CommandError::new(
            format!("failed to resolve path '{path_str}': {e}"),
            ErrorType::IoError,
        )
    })
}

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

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// Add a new marketplace source.
///
/// Mirrors the CLI `marketplace add` flow:
/// 1. Detect source type (GitHub shorthand, git URL, local path).
/// 2. Clone or symlink into a temp directory inside the cache.
/// 3. Read the marketplace manifest to discover the canonical name.
/// 4. Validate the name, rename to its final location.
/// 5. Register in `known_marketplaces.json`.
/// 6. Return the name and discovered plugins.
#[tauri::command]
#[specta::specta]
pub async fn add_marketplace(source: String) -> Result<MarketplaceAddResult, CommandError> {
    let ms = detect_source(&source);
    let cache = get_cache()?;
    cache.ensure_dirs().map_err(|e| {
        CommandError::new(
            format!("failed to create cache directories: {e}"),
            ErrorType::IoError,
        )
    })?;

    // Clone or symlink into a temporary name first, then rename once we
    // know the real marketplace name from the manifest.
    let temp_name = format!("_pending_{}", std::process::id());
    let temp_dir = cache.marketplace_path(&temp_name);

    // Clean up any leftover temp directory from a prior interrupted run.
    if temp_dir.exists() {
        fs::remove_dir_all(&temp_dir).map_err(|e| {
            CommandError::new(
                format!("failed to clean up {}: {e}", temp_dir.display()),
                ErrorType::IoError,
            )
        })?;
    }

    match &ms {
        MarketplaceSource::GitHub { repo } => {
            let url = git::github_repo_to_url(repo);
            debug!(url = %url, dest = %temp_dir.display(), "cloning GitHub marketplace");
            git::clone_repo(&url, &temp_dir, None).map_err(|e| {
                CommandError::new(
                    format!("failed to clone {repo}: {e}"),
                    ErrorType::GitError,
                )
            })?;
        }
        MarketplaceSource::GitUrl { url } => {
            debug!(url = %url, dest = %temp_dir.display(), "cloning git marketplace");
            git::clone_repo(url, &temp_dir, None).map_err(|e| {
                CommandError::new(
                    format!("failed to clone {url}: {e}"),
                    ErrorType::GitError,
                )
            })?;
        }
        MarketplaceSource::LocalPath { path } => {
            let src = resolve_local_path(path)?;
            debug!(src = %src.display(), dest = %temp_dir.display(), "symlinking local marketplace");
            #[cfg(unix)]
            std::os::unix::fs::symlink(&src, &temp_dir).map_err(|e| {
                CommandError::new(
                    format!(
                        "failed to symlink {} -> {}: {e}",
                        src.display(),
                        temp_dir.display()
                    ),
                    ErrorType::IoError,
                )
            })?;
            #[cfg(not(unix))]
            return Err(CommandError::new(
                "local path marketplaces are only supported on Unix",
                ErrorType::Validation,
            ));
        }
    }

    // Read marketplace manifest to get the canonical name.
    let manifest_path = temp_dir.join(kiro_market_core::MARKETPLACE_MANIFEST_PATH);
    let manifest_bytes = fs::read(&manifest_path).map_err(|e| {
        // Clean up temp on failure.
        let _ = fs::remove_dir_all(&temp_dir);
        CommandError::new(
            format!(
                "marketplace manifest not found at {}: {e}",
                manifest_path.display()
            ),
            ErrorType::NotFound,
        )
    })?;

    let manifest = Marketplace::from_json(&manifest_bytes).map_err(|e| {
        let _ = fs::remove_dir_all(&temp_dir);
        CommandError::new(
            format!("failed to parse marketplace manifest: {e}"),
            ErrorType::ParseError,
        )
    })?;

    let name = manifest.name.clone();

    // Validate name before any filesystem operation that uses it.
    validation::validate_name(&name).map_err(|e| {
        let _ = fs::remove_dir_all(&temp_dir);
        CommandError::from(kiro_market_core::error::Error::from(e))
    })?;

    // Rename temp dir to the real marketplace name.
    let final_dir = cache.marketplace_path(&name);
    if final_dir.exists() {
        let _ = fs::remove_dir_all(&temp_dir);
        return Err(CommandError::new(
            format!("marketplace '{name}' already exists"),
            ErrorType::AlreadyExists,
        ));
    }

    fs::rename(&temp_dir, &final_dir).map_err(|e| {
        CommandError::new(
            format!(
                "failed to rename {} -> {}: {e}",
                temp_dir.display(),
                final_dir.display()
            ),
            ErrorType::IoError,
        )
    })?;

    // Register in known_marketplaces.json.
    let entry = KnownMarketplace {
        name: name.clone(),
        source: ms,
        added_at: chrono::Utc::now(),
    };
    cache
        .add_known_marketplace(entry)
        .map_err(CommandError::from)?;

    // Build plugin list for the response.
    let plugins: Vec<PluginBasicInfo> = manifest
        .plugins
        .iter()
        .map(|p| PluginBasicInfo {
            name: p.name.clone(),
            description: p.description.clone(),
        })
        .collect();

    debug!(marketplace = %name, plugin_count = plugins.len(), "marketplace added");

    Ok(MarketplaceAddResult { name, plugins })
}

/// Remove a registered marketplace and its cached data.
#[tauri::command]
#[specta::specta]
pub async fn remove_marketplace(name: String) -> Result<(), CommandError> {
    let cache = get_cache()?;

    // Remove from the registry first.
    cache
        .remove_known_marketplace(&name)
        .map_err(CommandError::from)?;

    // Remove the cloned directory or symlink.
    let mp_path = cache.marketplace_path(&name);
    if mp_path.is_symlink() {
        fs::remove_file(&mp_path).map_err(|e| {
            CommandError::new(
                format!("failed to remove symlink {}: {e}", mp_path.display()),
                ErrorType::IoError,
            )
        })?;
    } else if mp_path.exists() {
        fs::remove_dir_all(&mp_path).map_err(|e| {
            CommandError::new(
                format!("failed to remove directory {}: {e}", mp_path.display()),
                ErrorType::IoError,
            )
        })?;
    }

    debug!(marketplace = %name, "marketplace removed");

    Ok(())
}

/// Update marketplace clone(s) from remote.
///
/// If `name` is provided, only that marketplace is updated. Otherwise all
/// registered marketplaces are updated. Symlinked (local) marketplaces are
/// skipped since they always reflect the latest state on disk.
#[tauri::command]
#[specta::specta]
pub async fn update_marketplace(
    name: Option<String>,
) -> Result<UpdateResult, CommandError> {
    let cache = get_cache()?;
    let entries = cache
        .load_known_marketplaces()
        .map_err(CommandError::from)?;

    if entries.is_empty() {
        return Ok(UpdateResult {
            updated: Vec::new(),
            failed: Vec::new(),
            skipped: Vec::new(),
        });
    }

    // Filter by name if provided.
    let targets: Vec<&KnownMarketplace> = if let Some(ref filter_name) = name {
        let filtered: Vec<_> = entries.iter().filter(|e| e.name == *filter_name).collect();
        if filtered.is_empty() {
            return Err(CommandError::new(
                format!("marketplace '{filter_name}' is not registered"),
                ErrorType::NotFound,
            ));
        }
        filtered
    } else {
        entries.iter().collect()
    };

    let mut result = UpdateResult {
        updated: Vec::new(),
        failed: Vec::new(),
        skipped: Vec::new(),
    };

    for entry in &targets {
        let mp_path = cache.marketplace_path(&entry.name);

        // Skip symlinked (local) marketplaces -- they always reflect the
        // latest state on disk.
        if mp_path.is_symlink() {
            debug!(marketplace = %entry.name, "skipping local marketplace");
            result.skipped.push(entry.name.clone());
            continue;
        }

        match git::pull_repo(&mp_path) {
            Ok(()) => {
                debug!(marketplace = %entry.name, "marketplace updated");
                result.updated.push(entry.name.clone());
            }
            Err(e) => {
                warn!(marketplace = %entry.name, error = %e, "failed to update marketplace");
                result.failed.push(FailedUpdate {
                    name: entry.name.clone(),
                    error: e.to_string(),
                });
            }
        }
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_source_github_shorthand() {
        let source = detect_source("microsoft/dotnet-skills");
        assert!(
            matches!(source, MarketplaceSource::GitHub { repo } if repo == "microsoft/dotnet-skills")
        );
    }

    #[test]
    fn detect_source_https_url() {
        let source = detect_source("https://github.com/owner/repo.git");
        assert!(
            matches!(source, MarketplaceSource::GitUrl { url } if url == "https://github.com/owner/repo.git")
        );
    }

    #[test]
    fn detect_source_git_ssh_url() {
        let source = detect_source("git@github.com:owner/repo.git");
        assert!(
            matches!(source, MarketplaceSource::GitUrl { url } if url == "git@github.com:owner/repo.git")
        );
    }

    #[test]
    fn detect_source_http_url() {
        let source = detect_source("http://example.com/repo.git");
        assert!(matches!(source, MarketplaceSource::GitUrl { .. }));
    }

    #[test]
    fn detect_source_absolute_path() {
        let source = detect_source("/home/user/marketplace");
        assert!(
            matches!(source, MarketplaceSource::LocalPath { path } if path == "/home/user/marketplace")
        );
    }

    #[test]
    fn detect_source_relative_dot() {
        let source = detect_source("./my-marketplace");
        assert!(matches!(source, MarketplaceSource::LocalPath { .. }));
    }

    #[test]
    fn detect_source_relative_dotdot() {
        let source = detect_source("../other/marketplace");
        assert!(matches!(source, MarketplaceSource::LocalPath { .. }));
    }

    #[test]
    fn detect_source_tilde() {
        let source = detect_source("~/marketplaces/mine");
        assert!(matches!(source, MarketplaceSource::LocalPath { .. }));
    }
}

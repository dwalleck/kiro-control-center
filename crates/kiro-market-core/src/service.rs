//! Marketplace lifecycle operations.
//!
//! [`MarketplaceService`] owns the add/remove/update/list logic that was
//! previously duplicated between the CLI and Tauri frontends.

use std::fs;
use std::path::Path;

use serde::Serialize;
use tracing::{debug, warn};

use crate::cache::{CacheDir, KnownMarketplace, MarketplaceSource};
use crate::error::{Error, MarketplaceError};
use crate::git::{self, CloneOptions, GitBackend, GitProtocol};
use crate::marketplace::Marketplace;
use crate::{platform, validation};

// ---------------------------------------------------------------------------
// Result types
// ---------------------------------------------------------------------------

/// Result of adding a new marketplace.
#[derive(Clone, Debug, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct MarketplaceAddResult {
    pub name: String,
    pub plugins: Vec<PluginBasicInfo>,
}

/// Basic information about a plugin within a marketplace.
#[derive(Clone, Debug, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct PluginBasicInfo {
    pub name: String,
    pub description: Option<String>,
}

/// Result of updating one or more marketplaces.
#[derive(Clone, Debug, Default, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct UpdateResult {
    pub updated: Vec<String>,
    pub failed: Vec<FailedUpdate>,
    pub skipped: Vec<String>,
}

/// A marketplace that failed to update, with the reason.
#[derive(Clone, Debug, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct FailedUpdate {
    pub name: String,
    pub error: String,
}

// ---------------------------------------------------------------------------
// Service
// ---------------------------------------------------------------------------

/// Manages the marketplace lifecycle: add, remove, update, list.
///
/// Uses `Box<dyn GitBackend>` rather than a generic parameter to keep
/// handler signatures clean. The vtable cost is negligible relative to
/// git I/O.
pub struct MarketplaceService {
    cache: CacheDir,
    git: Box<dyn GitBackend>,
}

impl MarketplaceService {
    /// Create a new service with the given cache directory and git backend.
    pub fn new(cache: CacheDir, git: impl GitBackend + 'static) -> Self {
        Self {
            cache,
            git: Box::new(git),
        }
    }

    /// Add a new marketplace source.
    ///
    /// 1. Detect source type (GitHub, git URL, local path).
    /// 2. Clone or link into a temp directory in the cache.
    /// 3. Read the marketplace manifest to discover the canonical name.
    /// 4. Validate the name, rename to final location.
    /// 5. Register in `known_marketplaces.json`.
    ///
    /// # Errors
    ///
    /// Returns an error if the clone/link fails, the manifest is missing or
    /// invalid, the marketplace name fails validation, or a marketplace with
    /// the same name is already registered.
    pub fn add(&self, source: &str, protocol: GitProtocol) -> Result<MarketplaceAddResult, Error> {
        let ms = MarketplaceSource::detect(source);
        self.cache.ensure_dirs()?;

        let temp_name = format!("_pending_{}", std::process::id());
        let temp_dir = self.cache.marketplace_path(&temp_name);

        // Clean up any leftover temp directory from a prior interrupted run.
        if temp_dir.exists()
            && let Err(e) = fs::remove_dir_all(&temp_dir)
        {
            warn!(
                path = %temp_dir.display(),
                error = %e,
                "failed to clean up leftover temp directory"
            );
        }

        // Clone or link based on source type.
        if let Err(e) = self.clone_or_link(&ms, protocol, &temp_dir) {
            if let Err(cleanup_err) = fs::remove_dir_all(&temp_dir) {
                warn!(
                    path = %temp_dir.display(),
                    error = %cleanup_err,
                    "failed to clean up temp directory after clone failure"
                );
            }
            return Err(e);
        }

        // Read marketplace manifest.
        let manifest_path = temp_dir.join(crate::MARKETPLACE_MANIFEST_PATH);
        let manifest = Self::read_and_validate_manifest(&manifest_path, &temp_dir)?;

        let name = manifest.name.clone();

        // Validate name before any filesystem operation that uses it.
        if let Err(e) = validation::validate_name(&name) {
            if let Err(cleanup_err) = fs::remove_dir_all(&temp_dir) {
                warn!(
                    path = %temp_dir.display(),
                    error = %cleanup_err,
                    "failed to clean up temp directory after validation failure"
                );
            }
            return Err(e.into());
        }

        // Rename temp dir to final location.
        let final_dir = self.cache.marketplace_path(&name);
        if final_dir.exists() {
            if let Err(cleanup_err) = fs::remove_dir_all(&temp_dir) {
                warn!(
                    path = %temp_dir.display(),
                    error = %cleanup_err,
                    "failed to clean up temp directory"
                );
            }
            return Err(MarketplaceError::AlreadyRegistered { name: name.clone() }.into());
        }

        fs::rename(&temp_dir, &final_dir)?;

        // Register in known_marketplaces.json.
        let entry = KnownMarketplace {
            name: name.clone(),
            source: ms,
            protocol: Some(protocol),
            added_at: chrono::Utc::now(),
        };
        self.cache.add_known_marketplace(entry)?;

        let plugins = manifest
            .plugins
            .iter()
            .map(|p| PluginBasicInfo {
                name: p.name.clone(),
                description: p.description.clone(),
            })
            .collect();

        debug!(marketplace = %name, "marketplace added");

        Ok(MarketplaceAddResult { name, plugins })
    }

    /// Remove a registered marketplace and its cached data.
    ///
    /// # Errors
    ///
    /// Returns an error if the marketplace is not registered or its cached
    /// data cannot be removed from disk.
    pub fn remove(&self, name: &str) -> Result<(), Error> {
        self.cache.remove_known_marketplace(name)?;

        let mp_path = self.cache.marketplace_path(name);
        if platform::is_local_link(&mp_path) {
            platform::remove_local_link(&mp_path)?;
        } else if mp_path.exists() {
            fs::remove_dir_all(&mp_path)?;
        }

        debug!(marketplace = %name, "marketplace removed");
        Ok(())
    }

    /// Update marketplace clone(s) from remote.
    ///
    /// If `name` is provided, only that marketplace is updated. Locally
    /// linked marketplaces are skipped since they always reflect disk state.
    ///
    /// # Errors
    ///
    /// Returns an error if the registry cannot be read, or if a specific
    /// marketplace name was requested but is not registered.
    pub fn update(&self, name: Option<&str>) -> Result<UpdateResult, Error> {
        let entries = self.cache.load_known_marketplaces()?;

        let targets: Vec<_> = if let Some(filter_name) = name {
            let filtered: Vec<_> = entries.iter().filter(|e| e.name == *filter_name).collect();
            if filtered.is_empty() {
                return Err(MarketplaceError::NotFound {
                    name: filter_name.to_owned(),
                }
                .into());
            }
            filtered
        } else {
            if entries.is_empty() {
                return Ok(UpdateResult::default());
            }
            entries.iter().collect()
        };

        let mut result = UpdateResult::default();

        for entry in &targets {
            let mp_path = self.cache.marketplace_path(&entry.name);

            // Skip locally linked marketplaces -- they always reflect disk state.
            if platform::is_local_link(&mp_path) {
                debug!(marketplace = %entry.name, "skipping local marketplace (linked)");
                result.skipped.push(entry.name.clone());
                continue;
            }

            // Skip local path sources that used copy fallback (not a git repo).
            if matches!(entry.source, MarketplaceSource::LocalPath { .. }) {
                debug!(
                    marketplace = %entry.name,
                    "skipping local marketplace (directory copy)"
                );
                result.skipped.push(entry.name.clone());
                continue;
            }

            match self.git.pull_repo(&mp_path) {
                Ok(()) => {
                    debug!(marketplace = %entry.name, "marketplace updated");
                    result.updated.push(entry.name.clone());
                }
                Err(e) => {
                    warn!(marketplace = %entry.name, error = %e, "failed to update");
                    result.failed.push(FailedUpdate {
                        name: entry.name.clone(),
                        error: e.to_string(),
                    });
                }
            }
        }

        Ok(result)
    }

    /// List all registered marketplaces.
    ///
    /// # Errors
    ///
    /// Returns an error if the registry file cannot be read or parsed.
    pub fn list(&self) -> Result<Vec<KnownMarketplace>, Error> {
        self.cache.load_known_marketplaces()
    }

    // -----------------------------------------------------------------------
    // Private helpers
    // -----------------------------------------------------------------------

    fn clone_or_link(
        &self,
        ms: &MarketplaceSource,
        protocol: GitProtocol,
        dest: &Path,
    ) -> Result<(), Error> {
        match ms {
            MarketplaceSource::GitHub { repo } => {
                let url = git::github_repo_to_url(repo, protocol);
                debug!(url = %url, dest = %dest.display(), "cloning GitHub marketplace");
                let opts = CloneOptions::default();
                self.git.clone_repo(&url, dest, &opts)?;
            }
            MarketplaceSource::GitUrl { url } => {
                if protocol != GitProtocol::default() {
                    warn!(
                        "protocol parameter ignored for full git URL; the URL's own scheme is used"
                    );
                }
                debug!(url = %url, dest = %dest.display(), "cloning git marketplace");
                let opts = CloneOptions::default();
                self.git.clone_repo(url, dest, &opts)?;
            }
            MarketplaceSource::LocalPath { path } => {
                let src = crate::cache::resolve_local_path(path)?;
                debug!(src = %src.display(), dest = %dest.display(), "linking local marketplace");
                platform::create_local_link(&src, dest)?;
            }
        }
        Ok(())
    }

    fn read_and_validate_manifest(
        manifest_path: &Path,
        temp_dir: &Path,
    ) -> Result<Marketplace, Error> {
        let manifest_bytes = match fs::read(manifest_path) {
            Ok(bytes) => bytes,
            Err(_e) => {
                if let Err(cleanup_err) = fs::remove_dir_all(temp_dir) {
                    warn!(
                        path = %temp_dir.display(),
                        error = %cleanup_err,
                        "failed to clean up temp directory after manifest read failure"
                    );
                }
                return Err(MarketplaceError::ManifestNotFound {
                    path: manifest_path.to_path_buf(),
                }
                .into());
            }
        };

        match Marketplace::from_json(&manifest_bytes) {
            Ok(m) => Ok(m),
            Err(e) => {
                if let Err(cleanup_err) = fs::remove_dir_all(temp_dir) {
                    warn!(
                        path = %temp_dir.display(),
                        error = %cleanup_err,
                        "failed to clean up temp directory after manifest parse failure"
                    );
                }
                Err(MarketplaceError::InvalidManifest {
                    reason: e.to_string(),
                }
                .into())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::cache::CacheDir;
    use crate::error::GitError;
    use crate::git::CloneOptions;

    /// Mock git backend that records calls and creates a minimal marketplace
    /// manifest in the destination directory during clone.
    #[derive(Debug, Default)]
    struct MockGitBackend {
        calls: Mutex<Vec<String>>,
    }

    impl GitBackend for MockGitBackend {
        fn clone_repo(
            &self,
            url: &str,
            dest: &Path,
            _opts: &CloneOptions,
        ) -> Result<(), GitError> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("clone:{url}"));
            // Create dest with a minimal marketplace manifest.
            let mp_dir = dest.join(".claude-plugin");
            fs::create_dir_all(&mp_dir).unwrap();
            fs::write(
                mp_dir.join("marketplace.json"),
                r#"{"name":"mock-market","owner":{"name":"Test"},"plugins":[{"name":"mock-plugin","description":"A mock plugin","source":"./plugins/mock"}]}"#,
            )
            .unwrap();
            Ok(())
        }

        fn pull_repo(&self, path: &Path) -> Result<(), GitError> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("pull:{}", path.display()));
            Ok(())
        }

        fn verify_sha(&self, _path: &Path, _expected: &str) -> Result<(), GitError> {
            Ok(())
        }
    }

    fn temp_service() -> (tempfile::TempDir, MarketplaceService) {
        let dir = tempfile::tempdir().expect("tempdir");
        let cache = CacheDir::with_root(dir.path().to_path_buf());
        cache.ensure_dirs().expect("ensure_dirs");
        let svc = MarketplaceService::new(cache, MockGitBackend::default());
        (dir, svc)
    }

    #[test]
    fn add_marketplace_registers_and_returns_plugins() {
        let (_dir, svc) = temp_service();
        let result = svc
            .add("owner/repo", GitProtocol::Https)
            .expect("add should succeed");

        assert_eq!(result.name, "mock-market");
        assert_eq!(result.plugins.len(), 1);
        assert_eq!(result.plugins[0].name, "mock-plugin");

        let known = svc.list().expect("list");
        assert_eq!(known.len(), 1);
        assert_eq!(known[0].name, "mock-market");
    }

    #[test]
    fn add_duplicate_marketplace_returns_error() {
        let (_dir, svc) = temp_service();
        svc.add("owner/repo", GitProtocol::Https)
            .expect("first add");

        let err = svc
            .add("owner/repo", GitProtocol::Https)
            .expect_err("duplicate should fail");

        assert!(
            err.to_string().contains("already"),
            "expected 'already' in error: {err}"
        );
    }

    #[test]
    fn remove_marketplace_cleans_up() {
        let (_dir, svc) = temp_service();
        svc.add("owner/repo", GitProtocol::Https)
            .expect("add");

        svc.remove("mock-market").expect("remove");

        let known = svc.list().expect("list");
        assert!(known.is_empty());
    }

    #[test]
    fn update_calls_pull_on_cloned_repos() {
        let (_dir, svc) = temp_service();
        svc.add("owner/repo", GitProtocol::Https)
            .expect("add");

        let result = svc.update(None).expect("update");

        assert_eq!(result.updated.len(), 1);
        assert_eq!(result.updated[0], "mock-market");
        assert!(result.failed.is_empty());
        assert!(result.skipped.is_empty());
    }

    #[test]
    fn update_nonexistent_returns_error() {
        let (_dir, svc) = temp_service();

        let err = svc
            .update(Some("nope"))
            .expect_err("should fail for unknown marketplace");

        assert!(
            err.to_string().contains("not found"),
            "expected 'not found' in error: {err}"
        );
    }

    #[test]
    fn list_empty_returns_empty_vec() {
        let (_dir, svc) = temp_service();

        let known = svc.list().expect("list");

        assert!(known.is_empty());
    }
}

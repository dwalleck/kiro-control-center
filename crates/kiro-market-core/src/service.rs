//! Marketplace lifecycle operations.
//!
//! [`MarketplaceService`] centralizes add/remove/update/list logic so that
//! CLI and Tauri frontends remain thin presentation wrappers.

use std::error::Error as _;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;
use tracing::{debug, warn};

use crate::cache::{CacheDir, KnownMarketplace, MarketplaceSource};
use crate::error::{Error, MarketplaceError};
use crate::git::{self, CloneOptions, GitBackend, GitProtocol};
use crate::marketplace::Marketplace;
use crate::platform::LinkResult;
use crate::{platform, validation};

// ---------------------------------------------------------------------------
// Temp directory cleanup guard
// ---------------------------------------------------------------------------

/// RAII guard that removes a temp directory on drop unless defused.
/// Prevents orphaned `_pending_*` directories when `add()` fails.
struct TempDirGuard {
    path: PathBuf,
    defused: bool,
}

impl TempDirGuard {
    fn new(path: PathBuf) -> Self {
        Self {
            path,
            defused: false,
        }
    }

    /// Prevent cleanup on drop (call after successful rename).
    fn defuse(&mut self) {
        self.defused = true;
    }
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        if !self.defused
            && let Err(e) = fs::remove_dir_all(&self.path)
            && e.kind() != std::io::ErrorKind::NotFound
        {
            warn!(
                path = %self.path.display(),
                error = %e,
                "failed to clean up temp directory — remove it manually"
            );
        }
    }
}

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
    /// 3. Try to read `marketplace.json`; if missing, scan for `plugin.json` files.
    /// 4. Merge manifest plugins with discovered plugins, deduplicating by
    ///    relative path (for `RelativePath` sources) or by name (for
    ///    `Structured` sources).
    /// 5. Validate the name, rename to final location.
    /// 6. Register in `known_marketplaces.json`.
    ///
    /// # Errors
    ///
    /// Returns an error if the clone/link fails, a non-`NotFound` I/O error
    /// occurs when reading the manifest, no plugins are found (neither via
    /// manifest nor scan), the marketplace name fails validation, or a
    /// marketplace with the same name is already registered.
    #[allow(clippy::too_many_lines)]
    pub fn add(&self, source: &str, protocol: GitProtocol) -> Result<MarketplaceAddResult, Error> {
        let ms = MarketplaceSource::detect(source);
        self.cache.ensure_dirs()?;

        let temp_name = format!("_pending_{}", std::process::id());
        let temp_dir = self.cache.marketplace_path(&temp_name);

        if temp_dir.exists()
            && let Err(e) = fs::remove_dir_all(&temp_dir)
        {
            warn!(
                path = %temp_dir.display(),
                error = %e,
                "failed to clean up leftover temp directory"
            );
        }

        let mut guard = TempDirGuard::new(temp_dir.clone());

        let link_result = self.clone_or_link(&ms, protocol, &temp_dir)?;

        if link_result == LinkResult::Copied {
            warn!(
                source = %source,
                "marketplace was copied, not linked — local changes will NOT be live-tracked"
            );
        }

        // Try to read marketplace manifest (optional).
        let manifest = Self::try_read_manifest(&temp_dir)?;

        // Scan for plugin.json files.
        let discovered = crate::plugin::discover_plugins(&temp_dir, 3);

        // Build the merged plugin list and derive the marketplace name.
        let registry_entries = Self::build_registry_entries(manifest.as_ref(), &discovered);

        let name = if let Some(m) = &manifest {
            m.name.clone()
        } else if discovered.is_empty() {
            // Check if a manifest file exists but was malformed.
            let manifest_path = temp_dir.join(crate::MARKETPLACE_MANIFEST_PATH);
            if manifest_path.exists() {
                return Err(MarketplaceError::InvalidManifest {
                    reason: "marketplace.json exists but could not be parsed, and no plugin.json files were found via scan".into(),
                }
                .into());
            }
            return Err(MarketplaceError::NoPluginsFound {
                path: temp_dir.clone(),
            }
            .into());
        } else {
            ms.fallback_name().ok_or_else(|| {
                MarketplaceError::InvalidManifest {
                    reason: "no marketplace.json found and could not derive a name from the source; use --name to specify one".into(),
                }
            })?
        };

        let plugins: Vec<PluginBasicInfo> = registry_entries
            .iter()
            .map(|p| PluginBasicInfo {
                name: p.name.clone(),
                description: p.description.clone(),
            })
            .collect();

        validation::validate_name(&name)?;

        let final_dir = self.cache.marketplace_path(&name);
        if final_dir.exists() {
            return Err(MarketplaceError::AlreadyRegistered { name: name.clone() }.into());
        }

        fs::rename(&temp_dir, &final_dir)?;
        // Point the guard at the renamed location so its Drop targets the
        // right path if we bail out before defusing.
        guard.path.clone_from(&final_dir);

        let entry = KnownMarketplace {
            name: name.clone(),
            source: ms,
            protocol: Some(protocol),
            added_at: chrono::Utc::now(),
        };
        if let Err(e) = self.cache.add_known_marketplace(entry) {
            warn!(
                path = %final_dir.display(),
                error = %e,
                "registry write failed after rename; rolling back"
            );
            if let Err(rb) = fs::remove_dir_all(&final_dir) {
                warn!(
                    path = %final_dir.display(),
                    rollback_error = %rb,
                    "failed to roll back renamed directory — remove it manually"
                );
            }
            // Defuse so the guard doesn't attempt a second removal of the
            // same path (or log a spurious warning if rollback succeeded).
            guard.defuse();
            return Err(e);
        }
        guard.defuse();

        // Persist the merged plugin list so browse/install commands don't
        // need to re-scan the repo on every access.
        if let Err(e) = self.cache.write_plugin_registry(&name, &registry_entries) {
            warn!(
                marketplace = %name,
                error = %e,
                "failed to write plugin registry — run 'update {name}' to regenerate"
            );
        }

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
        let mp_path = self.cache.marketplace_path(name);

        // Verify it's registered before trying to delete.
        let entries = self.cache.load_known_marketplaces()?;
        if !entries.iter().any(|e| e.name == name) {
            return Err(MarketplaceError::NotFound {
                name: name.to_owned(),
            }
            .into());
        }

        // Delete the directory first — if this fails, the marketplace stays
        // registered and the user can retry.
        if platform::is_local_link(&mp_path) {
            platform::remove_local_link(&mp_path)?;
        } else if mp_path.exists() {
            fs::remove_dir_all(&mp_path)?;
        }

        // Clean up the plugin registry file (best-effort).
        let registry_path = self.cache.plugin_registry_path(name);
        if registry_path.exists()
            && let Err(e) = fs::remove_file(&registry_path)
        {
            warn!(
                path = %registry_path.display(),
                error = %e,
                "failed to remove plugin registry file"
            );
        }

        // Now unregister — directory is already gone.
        self.cache.remove_known_marketplace(name)?;

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
                    // Regenerate the plugin registry after pulling new content.
                    self.regenerate_plugin_registry(&entry.name, &mp_path);
                    debug!(marketplace = %entry.name, "marketplace updated");
                    result.updated.push(entry.name.clone());
                }
                Err(e) => {
                    warn!(marketplace = %entry.name, error = %e, "failed to update");
                    // Walk the error source chain for a complete message.
                    let mut detail = e.to_string();
                    let mut source: Option<&dyn std::error::Error> = e.source();
                    while let Some(cause) = source {
                        detail.push_str(": ");
                        detail.push_str(&cause.to_string());
                        source = cause.source();
                    }
                    result.failed.push(FailedUpdate {
                        name: entry.name.clone(),
                        error: detail,
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
    ) -> Result<LinkResult, Error> {
        match ms {
            MarketplaceSource::GitHub { repo } => {
                let url = git::github_repo_to_url(repo, protocol);
                debug!(url = %url, dest = %dest.display(), "cloning GitHub marketplace");
                let opts = CloneOptions::default();
                self.git.clone_repo(&url, dest, &opts)?;
                Ok(LinkResult::Linked)
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
                Ok(LinkResult::Linked)
            }
            MarketplaceSource::LocalPath { path } => {
                let src = crate::cache::resolve_local_path(path)?;
                debug!(src = %src.display(), dest = %dest.display(), "linking local marketplace");
                Ok(platform::create_local_link(&src, dest)?)
            }
        }
    }

    /// Re-scan the marketplace and write an updated plugin registry.
    ///
    /// Called after `update()` pulls new content. Best-effort — a failure
    /// here does not block the update from succeeding.
    fn regenerate_plugin_registry(&self, name: &str, mp_path: &Path) {
        let manifest = match Self::try_read_manifest(mp_path) {
            Ok(m) => m,
            Err(e) => {
                warn!(
                    marketplace = %name,
                    error = %e,
                    "could not read manifest during registry regeneration"
                );
                None
            }
        };
        let discovered = crate::plugin::discover_plugins(mp_path, 3);

        let entries = Self::build_registry_entries(manifest.as_ref(), &discovered);

        if let Err(e) = self.cache.write_plugin_registry(name, &entries) {
            warn!(
                marketplace = %name,
                error = %e,
                "failed to write plugin registry after update"
            );
        }
    }

    /// Build a merged list of `PluginEntry` from manifest + discovered plugins.
    ///
    /// Uses [`plugin_entry_from_discovered`] to construct entries from
    /// scanned `DiscoveredPlugin` values.
    fn build_registry_entries(
        manifest: Option<&Marketplace>,
        discovered: &[crate::plugin::DiscoveredPlugin],
    ) -> Vec<crate::marketplace::PluginEntry> {
        if let Some(m) = manifest {
            let mut entries = m.plugins.clone();

            let listed_paths: Vec<String> = m
                .plugins
                .iter()
                .filter_map(|p| match &p.source {
                    crate::marketplace::PluginSource::RelativePath(rel) => Some(
                        rel.trim_start_matches("./")
                            .trim_start_matches(".\\")
                            .trim_end_matches(['/', '\\'])
                            .replace('\\', "/"),
                    ),
                    crate::marketplace::PluginSource::Structured(_) => None,
                })
                .collect();

            let listed_names: Vec<&str> = m.plugins.iter().map(|p| p.name.as_str()).collect();

            for dp in discovered {
                let dp_path = dp.relative_path_unix();
                if !listed_paths.contains(&dp_path) && !listed_names.contains(&dp.name()) {
                    entries.push(plugin_entry_from_discovered(dp));
                }
            }

            entries
        } else {
            discovered
                .iter()
                .map(plugin_entry_from_discovered)
                .collect()
        }
    }

    /// Try to read the marketplace manifest.
    ///
    /// Returns `Ok(Some(manifest))` if found and valid, `Ok(None)` if the file
    /// is missing (logged at `debug`) or malformed (logged at `warn`).
    /// Non-`NotFound` I/O errors (permission denied, disk errors) are
    /// propagated as `Err` — they indicate a real problem, not an absent file.
    fn try_read_manifest(repo_dir: &Path) -> Result<Option<Marketplace>, Error> {
        let manifest_path = repo_dir.join(crate::MARKETPLACE_MANIFEST_PATH);
        match fs::read(&manifest_path) {
            Ok(bytes) => match Marketplace::from_json(&bytes) {
                Ok(m) => Ok(Some(m)),
                Err(e) => {
                    warn!(
                        path = %manifest_path.display(),
                        error = %e,
                        "marketplace.json is malformed, falling back to plugin scan"
                    );
                    Ok(None)
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                debug!(
                    path = %manifest_path.display(),
                    "no marketplace.json found, will discover plugins via scan"
                );
                Ok(None)
            }
            Err(e) => Err(e.into()),
        }
    }
}

/// Convert a [`DiscoveredPlugin`] into a [`PluginEntry`] with a relative-path source.
fn plugin_entry_from_discovered(
    dp: &crate::plugin::DiscoveredPlugin,
) -> crate::marketplace::PluginEntry {
    crate::marketplace::PluginEntry {
        name: dp.name().to_owned(),
        description: dp.description().map(String::from),
        source: crate::marketplace::PluginSource::RelativePath(dp.as_relative_path_string()),
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
        fn clone_repo(&self, url: &str, dest: &Path, _opts: &CloneOptions) -> Result<(), GitError> {
            self.calls.lock().unwrap().push(format!("clone:{url}"));
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
    fn add_marketplace_writes_plugin_registry() {
        let (dir, svc) = temp_service();
        svc.add("owner/repo", GitProtocol::Https)
            .expect("add should succeed");

        let cache = CacheDir::with_root(dir.path().to_path_buf());
        let registry = cache
            .load_plugin_registry("mock-market")
            .expect("load should succeed")
            .expect("registry should exist");

        assert_eq!(registry.len(), 1);
        assert_eq!(registry[0].name, "mock-plugin");
    }

    #[test]
    fn remove_marketplace_deletes_plugin_registry() {
        let (dir, svc) = temp_service();
        svc.add("owner/repo", GitProtocol::Https).expect("add");

        let cache = CacheDir::with_root(dir.path().to_path_buf());
        assert!(
            cache
                .load_plugin_registry("mock-market")
                .expect("load")
                .is_some(),
            "registry should exist after add"
        );

        svc.remove("mock-market").expect("remove");

        assert!(
            cache
                .load_plugin_registry("mock-market")
                .expect("load")
                .is_none(),
            "registry should be gone after remove"
        );
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
        svc.add("owner/repo", GitProtocol::Https).expect("add");

        svc.remove("mock-market").expect("remove");

        let known = svc.list().expect("list");
        assert!(known.is_empty());
    }

    #[test]
    fn update_calls_pull_on_cloned_repos() {
        let (_dir, svc) = temp_service();
        svc.add("owner/repo", GitProtocol::Https).expect("add");

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

    // -----------------------------------------------------------------------
    // Additional tests for review findings
    // -----------------------------------------------------------------------

    /// A git backend that always fails on clone.
    #[derive(Debug, Default)]
    struct FailingGitBackend;

    impl GitBackend for FailingGitBackend {
        fn clone_repo(
            &self,
            url: &str,
            _dest: &Path,
            _opts: &CloneOptions,
        ) -> Result<(), GitError> {
            Err(GitError::CloneFailed {
                url: url.to_owned(),
                source: "simulated failure".to_owned().into(),
            })
        }

        fn pull_repo(&self, path: &Path) -> Result<(), GitError> {
            Err(GitError::PullFailed {
                path: path.to_path_buf(),
                source: "simulated pull failure".to_owned().into(),
            })
        }

        fn verify_sha(&self, _path: &Path, _expected: &str) -> Result<(), GitError> {
            Ok(())
        }
    }

    #[test]
    fn add_with_clone_failure_cleans_up_temp_dir() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cache = CacheDir::with_root(dir.path().to_path_buf());
        cache.ensure_dirs().expect("ensure_dirs");
        let svc = MarketplaceService::new(cache, FailingGitBackend);

        let err = svc
            .add("owner/repo", GitProtocol::Https)
            .expect_err("should fail");

        assert!(
            err.to_string().contains("clone"),
            "expected clone error: {err}"
        );

        // Verify no _pending_ directory remains.
        let marketplaces_dir = dir.path().join("marketplaces");
        if marketplaces_dir.exists() {
            let entries: Vec<_> = fs::read_dir(&marketplaces_dir).expect("read dir").collect();
            assert!(
                entries.is_empty(),
                "expected no leftover temp dirs, found: {entries:?}"
            );
        }
    }

    #[test]
    fn add_with_git_url_passes_url_verbatim() {
        let (_dir, svc) = temp_service();
        let result = svc
            .add("https://github.com/owner/repo.git", GitProtocol::Https)
            .expect("add with git URL");

        assert_eq!(result.name, "mock-market");

        // Verify the mock received the verbatim URL, not a GitHub-reformatted one.
        // The mock backend is inside the Box, so we check via the registry.
        let known = svc.list().expect("list");
        assert_eq!(known.len(), 1);
        assert!(
            matches!(
                &known[0].source,
                MarketplaceSource::GitUrl { url } if url == "https://github.com/owner/repo.git"
            ),
            "expected GitUrl source, got {:?}",
            known[0].source
        );
    }

    #[test]
    fn update_with_pull_failure_records_in_failed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cache = CacheDir::with_root(dir.path().to_path_buf());
        cache.ensure_dirs().expect("ensure_dirs");

        // First add a marketplace with the working mock.
        let svc = MarketplaceService::new(
            CacheDir::with_root(dir.path().to_path_buf()),
            MockGitBackend::default(),
        );
        svc.add("owner/repo", GitProtocol::Https).expect("add");

        // Now create a new service with the failing backend to test update.
        let svc = MarketplaceService::new(
            CacheDir::with_root(dir.path().to_path_buf()),
            FailingGitBackend,
        );
        let result = svc
            .update(None)
            .expect("update should return Ok with failures");

        assert!(result.updated.is_empty());
        assert_eq!(result.failed.len(), 1);
        assert_eq!(result.failed[0].name, "mock-market");
        assert!(
            result.failed[0].error.contains("pull"),
            "expected pull error: {}",
            result.failed[0].error
        );
    }

    #[test]
    fn update_specific_marketplace_by_name() {
        let (_dir, svc) = temp_service();
        svc.add("owner/repo", GitProtocol::Https).expect("add");

        let result = svc.update(Some("mock-market")).expect("update by name");

        assert_eq!(result.updated.len(), 1);
        assert_eq!(result.updated[0], "mock-market");
    }

    // -----------------------------------------------------------------------
    // Scan-and-merge tests
    // -----------------------------------------------------------------------

    /// Mock git backend that creates a repo with plugin.json files but no marketplace.json.
    #[derive(Debug, Default)]
    struct NoManifestGitBackend;

    impl GitBackend for NoManifestGitBackend {
        fn clone_repo(
            &self,
            _url: &str,
            dest: &Path,
            _opts: &CloneOptions,
        ) -> Result<(), GitError> {
            let plugin_a = dest.join("plugins/alpha");
            fs::create_dir_all(&plugin_a).unwrap();
            fs::write(
                plugin_a.join("plugin.json"),
                r#"{"name":"alpha","description":"Alpha plugin","skills":["./skills/"]}"#,
            )
            .unwrap();

            let plugin_b = dest.join("plugins/beta");
            fs::create_dir_all(&plugin_b).unwrap();
            fs::write(
                plugin_b.join("plugin.json"),
                r#"{"name":"beta","skills":["./skills/"]}"#,
            )
            .unwrap();

            Ok(())
        }

        fn pull_repo(&self, _path: &Path) -> Result<(), GitError> {
            Ok(())
        }

        fn verify_sha(&self, _path: &Path, _expected: &str) -> Result<(), GitError> {
            Ok(())
        }
    }

    /// Mock that creates a repo with a marketplace.json AND an unlisted plugin.
    #[derive(Debug, Default)]
    struct MixedGitBackend;

    impl GitBackend for MixedGitBackend {
        fn clone_repo(
            &self,
            _url: &str,
            dest: &Path,
            _opts: &CloneOptions,
        ) -> Result<(), GitError> {
            let mp_dir = dest.join(".claude-plugin");
            fs::create_dir_all(&mp_dir).unwrap();
            fs::write(
                mp_dir.join("marketplace.json"),
                r#"{"name":"mixed-market","owner":{"name":"Test"},"plugins":[{"name":"listed","description":"A listed plugin","source":"./plugins/listed"}]}"#,
            )
            .unwrap();

            let listed = dest.join("plugins/listed");
            fs::create_dir_all(&listed).unwrap();
            fs::write(
                listed.join("plugin.json"),
                r#"{"name":"listed","description":"A listed plugin","skills":["./skills/"]}"#,
            )
            .unwrap();

            let unlisted = dest.join("plugins/unlisted");
            fs::create_dir_all(&unlisted).unwrap();
            fs::write(
                unlisted.join("plugin.json"),
                r#"{"name":"unlisted","description":"An unlisted plugin","skills":["./skills/"]}"#,
            )
            .unwrap();

            Ok(())
        }

        fn pull_repo(&self, _path: &Path) -> Result<(), GitError> {
            Ok(())
        }

        fn verify_sha(&self, _path: &Path, _expected: &str) -> Result<(), GitError> {
            Ok(())
        }
    }

    #[test]
    fn add_repo_without_manifest_discovers_plugins_via_scan() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cache = CacheDir::with_root(dir.path().to_path_buf());
        cache.ensure_dirs().expect("ensure_dirs");
        let svc = MarketplaceService::new(cache, NoManifestGitBackend);

        let result = svc
            .add("owner/skills", GitProtocol::Https)
            .expect("add should succeed");

        assert_eq!(result.name, "skills");
        assert_eq!(result.plugins.len(), 2);

        let names: Vec<&str> = result.plugins.iter().map(|p| p.name.as_str()).collect();
        assert!(names.contains(&"alpha"), "should find alpha: {names:?}");
        assert!(names.contains(&"beta"), "should find beta: {names:?}");
    }

    #[test]
    fn add_repo_with_manifest_and_unlisted_plugins_merges_both() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cache = CacheDir::with_root(dir.path().to_path_buf());
        cache.ensure_dirs().expect("ensure_dirs");
        let svc = MarketplaceService::new(cache, MixedGitBackend);

        let result = svc
            .add("owner/repo", GitProtocol::Https)
            .expect("add should succeed");

        assert_eq!(result.name, "mixed-market");
        assert_eq!(result.plugins.len(), 2);

        let names: Vec<&str> = result.plugins.iter().map(|p| p.name.as_str()).collect();
        assert!(names.contains(&"listed"), "should find listed: {names:?}");
        assert!(
            names.contains(&"unlisted"),
            "should find unlisted: {names:?}"
        );
    }

    #[test]
    fn add_repo_with_manifest_deduplicates_listed_plugins() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cache = CacheDir::with_root(dir.path().to_path_buf());
        cache.ensure_dirs().expect("ensure_dirs");
        let svc = MarketplaceService::new(cache, MixedGitBackend);

        let result = svc
            .add("owner/repo", GitProtocol::Https)
            .expect("add should succeed");

        let listed_count = result.plugins.iter().filter(|p| p.name == "listed").count();
        assert_eq!(listed_count, 1, "listed plugin should not be duplicated");
    }

    #[test]
    fn add_empty_repo_returns_no_plugins_found_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cache = CacheDir::with_root(dir.path().to_path_buf());
        cache.ensure_dirs().expect("ensure_dirs");

        #[derive(Debug)]
        struct EmptyRepoBackend;

        impl GitBackend for EmptyRepoBackend {
            fn clone_repo(
                &self,
                _url: &str,
                dest: &Path,
                _opts: &CloneOptions,
            ) -> Result<(), GitError> {
                fs::create_dir_all(dest).unwrap();
                Ok(())
            }

            fn pull_repo(&self, _path: &Path) -> Result<(), GitError> {
                Ok(())
            }

            fn verify_sha(&self, _path: &Path, _expected: &str) -> Result<(), GitError> {
                Ok(())
            }
        }

        let svc = MarketplaceService::new(cache, EmptyRepoBackend);
        let err = svc
            .add("owner/empty", GitProtocol::Https)
            .expect_err("should fail");

        assert!(
            err.to_string().contains("no plugins found"),
            "expected 'no plugins found' error, got: {err}"
        );
    }

    // -----------------------------------------------------------------------
    // Malformed manifest fallback test
    // -----------------------------------------------------------------------

    /// Mock that creates a repo with a malformed marketplace.json AND valid plugin.json files.
    #[derive(Debug)]
    struct MalformedManifestGitBackend;

    impl GitBackend for MalformedManifestGitBackend {
        fn clone_repo(
            &self,
            _url: &str,
            dest: &Path,
            _opts: &CloneOptions,
        ) -> Result<(), GitError> {
            // Create malformed marketplace.json.
            let mp_dir = dest.join(".claude-plugin");
            fs::create_dir_all(&mp_dir).unwrap();
            fs::write(mp_dir.join("marketplace.json"), "not valid json").unwrap();

            // Create a valid plugin.
            let plugin_dir = dest.join("plugins/fallback");
            fs::create_dir_all(&plugin_dir).unwrap();
            fs::write(
                plugin_dir.join("plugin.json"),
                r#"{"name":"fallback","description":"Found via scan","skills":["./skills/"]}"#,
            )
            .unwrap();

            Ok(())
        }

        fn pull_repo(&self, _path: &Path) -> Result<(), GitError> {
            Ok(())
        }

        fn verify_sha(&self, _path: &Path, _expected: &str) -> Result<(), GitError> {
            Ok(())
        }
    }

    #[test]
    fn add_repo_with_malformed_manifest_falls_back_to_scan() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cache = CacheDir::with_root(dir.path().to_path_buf());
        cache.ensure_dirs().expect("ensure_dirs");
        let svc = MarketplaceService::new(cache, MalformedManifestGitBackend);

        let result = svc
            .add("owner/fallback-repo", GitProtocol::Https)
            .expect("add should succeed via scan fallback");

        // Name derived from repo since manifest is malformed.
        assert_eq!(result.name, "fallback-repo");
        assert_eq!(result.plugins.len(), 1);
        assert_eq!(result.plugins[0].name, "fallback");
    }

    // -----------------------------------------------------------------------
    // Trailing-slash dedup test
    // -----------------------------------------------------------------------

    /// Mock that creates a repo with a marketplace.json using trailing-slash source paths
    /// AND a matching plugin.json, to test dedup with trailing slashes.
    #[derive(Debug)]
    struct TrailingSlashGitBackend;

    impl GitBackend for TrailingSlashGitBackend {
        fn clone_repo(
            &self,
            _url: &str,
            dest: &Path,
            _opts: &CloneOptions,
        ) -> Result<(), GitError> {
            let mp_dir = dest.join(".claude-plugin");
            fs::create_dir_all(&mp_dir).unwrap();
            fs::write(
                mp_dir.join("marketplace.json"),
                r#"{"name":"slash-market","owner":{"name":"Test"},"plugins":[{"name":"trailing","description":"Has trailing slash","source":"./plugins/trailing/"}]}"#,
            )
            .unwrap();

            let plugin_dir = dest.join("plugins/trailing");
            fs::create_dir_all(&plugin_dir).unwrap();
            fs::write(
                plugin_dir.join("plugin.json"),
                r#"{"name":"trailing","description":"Has trailing slash","skills":["./skills/"]}"#,
            )
            .unwrap();

            Ok(())
        }

        fn pull_repo(&self, _path: &Path) -> Result<(), GitError> {
            Ok(())
        }

        fn verify_sha(&self, _path: &Path, _expected: &str) -> Result<(), GitError> {
            Ok(())
        }
    }

    #[test]
    fn add_repo_deduplicates_with_trailing_slash_in_source() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cache = CacheDir::with_root(dir.path().to_path_buf());
        cache.ensure_dirs().expect("ensure_dirs");
        let svc = MarketplaceService::new(cache, TrailingSlashGitBackend);

        let result = svc
            .add("owner/repo", GitProtocol::Https)
            .expect("add should succeed");

        assert_eq!(result.name, "slash-market");
        // Should have exactly 1 plugin, not 2 (dedup should handle trailing slash).
        assert_eq!(
            result.plugins.len(),
            1,
            "trailing slash should not cause duplicate: {:?}",
            result.plugins
        );
    }

    // -----------------------------------------------------------------------
    // Manifest name validation test (security)
    // -----------------------------------------------------------------------

    /// Mock that creates a repo with a marketplace.json whose name contains path traversal.
    #[derive(Debug)]
    struct InvalidNameGitBackend;

    impl GitBackend for InvalidNameGitBackend {
        fn clone_repo(
            &self,
            _url: &str,
            dest: &Path,
            _opts: &CloneOptions,
        ) -> Result<(), GitError> {
            let mp_dir = dest.join(".claude-plugin");
            fs::create_dir_all(&mp_dir).unwrap();
            fs::write(
                mp_dir.join("marketplace.json"),
                r#"{"name":"../escape","owner":{"name":"Evil"},"plugins":[{"name":"evil","description":"Bad","source":"./plugins/evil"}]}"#,
            )
            .unwrap();
            Ok(())
        }

        fn pull_repo(&self, _path: &Path) -> Result<(), GitError> {
            Ok(())
        }

        fn verify_sha(&self, _path: &Path, _expected: &str) -> Result<(), GitError> {
            Ok(())
        }
    }

    #[test]
    fn add_repo_with_path_traversal_name_returns_validation_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cache = CacheDir::with_root(dir.path().to_path_buf());
        cache.ensure_dirs().expect("ensure_dirs");
        let svc = MarketplaceService::new(cache, InvalidNameGitBackend);

        let err = svc
            .add("owner/evil", GitProtocol::Https)
            .expect_err("should reject path traversal name");

        assert!(
            err.to_string().contains("invalid name"),
            "expected validation error, got: {err}"
        );

        // Verify no directory was left behind (TempDirGuard should clean up).
        let marketplaces_dir = dir.path().join("marketplaces");
        if marketplaces_dir.exists() {
            let entries: Vec<_> = fs::read_dir(&marketplaces_dir)
                .expect("read dir")
                .filter_map(Result::ok)
                .filter(|e| {
                    let name = e.file_name();
                    let name = name.to_string_lossy();
                    !name.starts_with('_')
                })
                .collect();
            assert!(
                entries.is_empty(),
                "no marketplace directory should remain after validation failure: {entries:?}"
            );
        }
    }

    // -----------------------------------------------------------------------
    // build_registry_entries
    // -----------------------------------------------------------------------

    #[test]
    fn build_registry_entries_merges_manifest_and_discovered() {
        use crate::plugin::discover_plugins;

        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();

        let mp_dir = root.join(".claude-plugin");
        fs::create_dir_all(&mp_dir).unwrap();
        fs::write(
            mp_dir.join("marketplace.json"),
            r#"{"name":"test","owner":{"name":"T"},"plugins":[{"name":"listed","description":"Listed","source":"./plugins/listed"}]}"#,
        )
        .unwrap();

        let listed_dir = root.join("plugins/listed");
        fs::create_dir_all(&listed_dir).unwrap();
        fs::write(
            listed_dir.join("plugin.json"),
            r#"{"name":"listed","description":"Listed","skills":["./skills/"]}"#,
        )
        .unwrap();

        let unlisted_dir = root.join("plugins/unlisted");
        fs::create_dir_all(&unlisted_dir).unwrap();
        fs::write(
            unlisted_dir.join("plugin.json"),
            r#"{"name":"unlisted","description":"Unlisted","skills":["./skills/"]}"#,
        )
        .unwrap();

        let manifest_bytes = fs::read(mp_dir.join("marketplace.json")).unwrap();
        let manifest = Marketplace::from_json(&manifest_bytes).unwrap();
        let discovered = discover_plugins(root, 3);

        let entries = MarketplaceService::build_registry_entries(Some(&manifest), &discovered);

        assert_eq!(entries.len(), 2, "should have listed + unlisted");
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert!(
            names.contains(&"listed"),
            "should include listed: {names:?}"
        );
        assert!(
            names.contains(&"unlisted"),
            "should include unlisted: {names:?}"
        );
    }

    #[test]
    fn build_registry_entries_deduplicates_by_path() {
        use crate::plugin::discover_plugins;

        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();

        let mp_dir = root.join(".claude-plugin");
        fs::create_dir_all(&mp_dir).unwrap();
        fs::write(
            mp_dir.join("marketplace.json"),
            r#"{"name":"test","owner":{"name":"T"},"plugins":[{"name":"alpha","description":"Alpha","source":"./plugins/alpha"}]}"#,
        )
        .unwrap();

        let plugin_dir = root.join("plugins/alpha");
        fs::create_dir_all(&plugin_dir).unwrap();
        fs::write(
            plugin_dir.join("plugin.json"),
            r#"{"name":"alpha","description":"Alpha","skills":["./skills/"]}"#,
        )
        .unwrap();

        let manifest_bytes = fs::read(mp_dir.join("marketplace.json")).unwrap();
        let manifest = Marketplace::from_json(&manifest_bytes).unwrap();
        let discovered = discover_plugins(root, 3);

        let entries = MarketplaceService::build_registry_entries(Some(&manifest), &discovered);

        let alpha_count = entries.iter().filter(|e| e.name == "alpha").count();
        assert_eq!(alpha_count, 1, "alpha should not be duplicated");
    }

    #[test]
    fn build_registry_entries_without_manifest_uses_discovered() {
        use crate::plugin::discover_plugins;

        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();

        let plugin_dir = root.join("plugins/solo");
        fs::create_dir_all(&plugin_dir).unwrap();
        fs::write(
            plugin_dir.join("plugin.json"),
            r#"{"name":"solo","description":"Solo plugin","skills":["./skills/"]}"#,
        )
        .unwrap();

        let discovered = discover_plugins(root, 3);
        let entries = MarketplaceService::build_registry_entries(None, &discovered);

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "solo");
    }
}

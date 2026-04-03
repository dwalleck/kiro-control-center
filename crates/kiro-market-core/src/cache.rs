//! Cache directory and marketplace registry management.
//!
//! All persistent state lives under `~/.local/share/kiro-market/` (or the
//! platform-appropriate data directory). This module provides [`CacheDir`] for
//! creating the directory structure and managing the `known_marketplaces.json`
//! registry file.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::MarketplaceError;
use crate::validation;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// How a marketplace was sourced when it was added.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum MarketplaceSource {
    /// A GitHub `owner/repo` shorthand.
    #[serde(rename = "github")]
    GitHub { repo: String },
    /// A full Git clone URL.
    #[serde(rename = "git_url")]
    GitUrl { url: String },
    /// A path on the local filesystem.
    #[serde(rename = "local")]
    LocalPath { path: String },
}

impl MarketplaceSource {
    /// Classify a user-provided source string into a `MarketplaceSource`.
    ///
    /// Heuristics:
    /// - Starts with `http://`, `https://`, or `git@` → `GitUrl`
    /// - Starts with `/`, `./`, `../`, or `~` → `LocalPath`
    /// - Anything else → `GitHub` (owner/repo shorthand)
    #[must_use]
    pub fn detect(source: &str) -> Self {
        if source.starts_with("http://")
            || source.starts_with("https://")
            || source.starts_with("git@")
        {
            Self::GitUrl {
                url: source.to_owned(),
            }
        } else if source.starts_with('/')
            || source.starts_with("./")
            || source.starts_with("../")
            || source.starts_with('~')
        {
            Self::LocalPath {
                path: source.to_owned(),
            }
        } else {
            Self::GitHub {
                repo: source.to_owned(),
            }
        }
    }

    /// Return a human-readable label for this source type.
    #[must_use]
    pub fn label(&self) -> &'static str {
        match self {
            Self::GitHub { .. } => "github",
            Self::GitUrl { .. } => "git",
            Self::LocalPath { .. } => "local",
        }
    }
}

/// Resolve a local path string to an absolute path.
///
/// Handles `~` expansion (via `dirs::home_dir()`) and canonicalization.
///
/// # Errors
///
/// Returns an I/O error if the home directory cannot be determined or the
/// path cannot be canonicalized (e.g. does not exist).
pub fn resolve_local_path(path_str: &str) -> std::io::Result<PathBuf> {
    let expanded = if let Some(rest) = path_str.strip_prefix('~') {
        let home = dirs::home_dir().ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "could not determine home directory for ~ expansion",
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

    expanded.canonicalize()
}

/// An entry in the known-marketplaces registry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnownMarketplace {
    pub name: String,
    pub source: MarketplaceSource,
    pub added_at: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// CacheDir
// ---------------------------------------------------------------------------

/// Manages the on-disk cache layout for kiro-market.
///
/// ```text
/// <root>/
///   known_marketplaces.json
///   marketplaces/
///   plugins/
/// ```
#[derive(Debug, Clone)]
pub struct CacheDir {
    root: PathBuf,
}

/// Name of the registry file that tracks added marketplaces.
const KNOWN_MARKETPLACES_FILE: &str = "known_marketplaces.json";

impl CacheDir {
    /// Return the platform default cache root, if one can be determined.
    ///
    /// Uses `dirs::data_dir()` (e.g. `~/.local/share` on Linux) joined with
    /// `kiro-market`. Returns `None` in environments without a home directory
    /// (e.g. bare containers, some CI runners).
    #[must_use]
    pub fn default_location() -> Option<Self> {
        dirs::data_dir().map(|data| Self {
            root: data.join("kiro-market"),
        })
    }

    /// Create a `CacheDir` rooted at an arbitrary path (useful for testing).
    #[must_use]
    pub fn with_root(root: PathBuf) -> Self {
        Self { root }
    }

    /// The cache root directory.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Directory where cloned marketplace repos are stored.
    #[must_use]
    pub fn marketplaces_dir(&self) -> PathBuf {
        self.root.join("marketplaces")
    }

    /// Directory where extracted plugin artefacts are stored.
    #[must_use]
    pub fn plugins_dir(&self) -> PathBuf {
        self.root.join("plugins")
    }

    /// Path to a specific marketplace clone directory.
    #[must_use]
    pub fn marketplace_path(&self, name: &str) -> PathBuf {
        self.marketplaces_dir().join(name)
    }

    /// Path to a specific plugin directory within a marketplace.
    #[must_use]
    pub fn plugin_path(&self, marketplace: &str, plugin: &str) -> PathBuf {
        self.plugins_dir().join(marketplace).join(plugin)
    }

    /// Create all required subdirectories if they do not already exist.
    ///
    /// # Errors
    ///
    /// Returns an [`std::io::Error`] if directory creation fails.
    pub fn ensure_dirs(&self) -> std::io::Result<()> {
        fs::create_dir_all(self.marketplaces_dir())?;
        fs::create_dir_all(self.plugins_dir())?;
        Ok(())
    }

    // -- known marketplaces registry ----------------------------------------

    /// Path to the `known_marketplaces.json` file.
    fn registry_path(&self) -> PathBuf {
        self.root.join(KNOWN_MARKETPLACES_FILE)
    }

    /// Load the list of known marketplaces from disk.
    ///
    /// Returns an empty `Vec` if the registry file does not exist yet.
    ///
    /// # Errors
    ///
    /// Returns [`crate::error::Error`] on I/O or JSON parse failures.
    pub fn load_known_marketplaces(&self) -> crate::error::Result<Vec<KnownMarketplace>> {
        let path = self.registry_path();

        match fs::read(&path) {
            Ok(bytes) => {
                let entries: Vec<KnownMarketplace> = serde_json::from_slice(&bytes)?;
                Ok(entries)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(e) => Err(e.into()),
        }
    }

    /// Add a marketplace to the registry, persisting to disk.
    ///
    /// # Errors
    ///
    /// - [`MarketplaceError::AlreadyRegistered`] if a marketplace with the
    ///   same name already exists.
    /// - I/O or JSON serialisation errors.
    pub fn add_known_marketplace(&self, entry: KnownMarketplace) -> crate::error::Result<()> {
        validation::validate_name(&entry.name)?;
        let mut entries = self.load_known_marketplaces()?;

        if entries.iter().any(|e| e.name == entry.name) {
            return Err(MarketplaceError::AlreadyRegistered { name: entry.name }.into());
        }

        entries.push(entry);
        self.write_registry(&entries)
    }

    /// Remove a marketplace from the registry by name, persisting to disk.
    ///
    /// # Errors
    ///
    /// - [`MarketplaceError::NotFound`] if no marketplace with the given name
    ///   exists.
    /// - I/O or JSON serialisation errors.
    pub fn remove_known_marketplace(&self, name: &str) -> crate::error::Result<()> {
        let mut entries = self.load_known_marketplaces()?;
        let before_len = entries.len();
        entries.retain(|e| e.name != name);

        if entries.len() == before_len {
            return Err(MarketplaceError::NotFound {
                name: name.to_owned(),
            }
            .into());
        }

        self.write_registry(&entries)
    }

    /// Serialise and write the registry to disk atomically.
    ///
    /// Writes to a `.tmp` sibling first, then renames into place so that a
    /// crash mid-write cannot leave truncated JSON on disk.
    fn write_registry(&self, entries: &[KnownMarketplace]) -> crate::error::Result<()> {
        let path = self.registry_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(entries)?;
        atomic_write(&path, json.as_bytes())?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Atomic write helper
// ---------------------------------------------------------------------------

/// Write data to a file atomically by writing to a temp file then renaming.
///
/// The temp file is created in the same directory as the target to guarantee
/// a same-filesystem rename (which is atomic on POSIX).
pub(crate) fn atomic_write(path: &Path, data: &[u8]) -> io::Result<()> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, data)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_cache() -> (tempfile::TempDir, CacheDir) {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        let cache = CacheDir::with_root(dir.path().to_path_buf());
        (dir, cache)
    }

    #[test]
    fn cache_dir_creates_structure() {
        let (_dir, cache) = temp_cache();
        cache.ensure_dirs().expect("ensure_dirs should succeed");

        assert!(cache.marketplaces_dir().is_dir());
        assert!(cache.plugins_dir().is_dir());
    }

    #[test]
    fn known_marketplaces_roundtrip() {
        let (_dir, cache) = temp_cache();
        cache.ensure_dirs().expect("ensure_dirs");

        let entry = KnownMarketplace {
            name: "test-market".into(),
            source: MarketplaceSource::GitHub {
                repo: "owner/repo".into(),
            },
            added_at: Utc::now(),
        };

        cache
            .add_known_marketplace(entry.clone())
            .expect("add should succeed");

        let loaded = cache
            .load_known_marketplaces()
            .expect("load should succeed");

        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].name, "test-market");

        match &loaded[0].source {
            MarketplaceSource::GitHub { repo } => assert_eq!(repo, "owner/repo"),
            other => panic!("expected GitHub source, got {other:?}"),
        }
    }

    #[test]
    fn known_marketplaces_rejects_duplicate() {
        let (_dir, cache) = temp_cache();
        cache.ensure_dirs().expect("ensure_dirs");

        let entry = KnownMarketplace {
            name: "dup-market".into(),
            source: MarketplaceSource::GitUrl {
                url: "https://example.com/repo.git".into(),
            },
            added_at: Utc::now(),
        };

        cache
            .add_known_marketplace(entry.clone())
            .expect("first add should succeed");

        let err = cache
            .add_known_marketplace(entry)
            .expect_err("second add should fail");

        let msg = err.to_string();
        assert!(
            msg.contains("already registered"),
            "expected 'already registered' in error, got: {msg}"
        );
    }

    #[test]
    fn load_known_marketplaces_returns_empty_when_no_file() {
        let (_dir, cache) = temp_cache();
        let loaded = cache
            .load_known_marketplaces()
            .expect("load should succeed");
        assert!(loaded.is_empty());
    }

    #[test]
    fn remove_known_marketplace_succeeds() {
        let (_dir, cache) = temp_cache();
        cache.ensure_dirs().expect("ensure_dirs");

        let entry = KnownMarketplace {
            name: "removable".into(),
            source: MarketplaceSource::LocalPath {
                path: "/tmp/market".into(),
            },
            added_at: Utc::now(),
        };

        cache
            .add_known_marketplace(entry)
            .expect("add should succeed");
        cache
            .remove_known_marketplace("removable")
            .expect("remove should succeed");

        let loaded = cache
            .load_known_marketplaces()
            .expect("load should succeed");
        assert!(loaded.is_empty());
    }

    #[test]
    fn remove_known_marketplace_errors_when_not_found() {
        let (_dir, cache) = temp_cache();
        cache.ensure_dirs().expect("ensure_dirs");

        let err = cache
            .remove_known_marketplace("nonexistent")
            .expect_err("remove should fail");

        let msg = err.to_string();
        assert!(
            msg.contains("not found"),
            "expected 'not found' in error, got: {msg}"
        );
    }

    #[test]
    fn add_known_marketplace_rejects_path_traversal() {
        let (_dir, cache) = temp_cache();
        cache.ensure_dirs().expect("ensure_dirs");

        let entry = KnownMarketplace {
            name: "../escape".into(),
            source: MarketplaceSource::GitHub {
                repo: "evil/repo".into(),
            },
            added_at: Utc::now(),
        };

        let err = cache
            .add_known_marketplace(entry)
            .expect_err("should reject path traversal");
        let msg = err.to_string();
        assert!(
            msg.contains("invalid name"),
            "expected 'invalid name', got: {msg}"
        );
    }

    #[test]
    fn add_known_marketplace_rejects_slash_in_name() {
        let (_dir, cache) = temp_cache();
        cache.ensure_dirs().expect("ensure_dirs");

        let entry = KnownMarketplace {
            name: "sub/dir".into(),
            source: MarketplaceSource::GitHub {
                repo: "evil/repo".into(),
            },
            added_at: Utc::now(),
        };

        let err = cache
            .add_known_marketplace(entry)
            .expect_err("should reject slash");
        let msg = err.to_string();
        assert!(
            msg.contains("path separator"),
            "expected 'path separator', got: {msg}"
        );
    }

    #[test]
    fn marketplace_path_and_plugin_path_structure() {
        let (_dir, cache) = temp_cache();

        let mp = cache.marketplace_path("my-market");
        assert!(mp.ends_with("marketplaces/my-market"));

        let pp = cache.plugin_path("my-market", "my-plugin");
        assert!(pp.ends_with("plugins/my-market/my-plugin"));
    }

    #[test]
    fn atomic_write_produces_valid_json() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("test.json");

        let data = serde_json::json!({"key": "value"});
        let bytes = serde_json::to_string_pretty(&data).expect("serialize");

        atomic_write(&path, bytes.as_bytes()).expect("atomic write should succeed");

        let read_back = fs::read(&path).expect("read");
        let parsed: serde_json::Value =
            serde_json::from_slice(&read_back).expect("should be valid JSON");
        assert_eq!(parsed["key"], "value");

        // The temp file should not remain.
        assert!(
            !path.with_extension("tmp").exists(),
            ".tmp file should be gone after rename"
        );
    }

    #[test]
    fn write_registry_produces_valid_json_after_add() {
        let (_dir, cache) = temp_cache();
        cache.ensure_dirs().expect("ensure_dirs");

        let entry = KnownMarketplace {
            name: "atomic-test".into(),
            source: MarketplaceSource::GitHub {
                repo: "owner/repo".into(),
            },
            added_at: Utc::now(),
        };

        cache
            .add_known_marketplace(entry)
            .expect("add should succeed");

        let raw = fs::read(cache.registry_path()).expect("read registry");
        let parsed: Vec<KnownMarketplace> =
            serde_json::from_slice(&raw).expect("registry should be valid JSON");
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].name, "atomic-test");
    }

    // -----------------------------------------------------------------------
    // MarketplaceSource::detect
    // -----------------------------------------------------------------------

    #[test]
    fn detect_github_shorthand() {
        let source = MarketplaceSource::detect("microsoft/dotnet-skills");
        assert!(
            matches!(source, MarketplaceSource::GitHub { repo } if repo == "microsoft/dotnet-skills")
        );
    }

    #[test]
    fn detect_https_url() {
        let source = MarketplaceSource::detect("https://github.com/owner/repo.git");
        assert!(
            matches!(source, MarketplaceSource::GitUrl { url } if url == "https://github.com/owner/repo.git")
        );
    }

    #[test]
    fn detect_git_ssh_url() {
        let source = MarketplaceSource::detect("git@github.com:owner/repo.git");
        assert!(
            matches!(source, MarketplaceSource::GitUrl { url } if url == "git@github.com:owner/repo.git")
        );
    }

    #[test]
    fn detect_http_url() {
        let source = MarketplaceSource::detect("http://example.com/repo.git");
        assert!(matches!(source, MarketplaceSource::GitUrl { .. }));
    }

    #[test]
    fn detect_absolute_path() {
        let source = MarketplaceSource::detect("/home/user/marketplace");
        assert!(
            matches!(source, MarketplaceSource::LocalPath { path } if path == "/home/user/marketplace")
        );
    }

    #[test]
    fn detect_relative_dot() {
        let source = MarketplaceSource::detect("./my-marketplace");
        assert!(matches!(source, MarketplaceSource::LocalPath { .. }));
    }

    #[test]
    fn detect_relative_dotdot() {
        let source = MarketplaceSource::detect("../other/marketplace");
        assert!(matches!(source, MarketplaceSource::LocalPath { .. }));
    }

    #[test]
    fn detect_tilde() {
        let source = MarketplaceSource::detect("~/marketplaces/mine");
        assert!(matches!(source, MarketplaceSource::LocalPath { .. }));
    }

    #[test]
    fn label_values() {
        assert_eq!(
            MarketplaceSource::GitHub {
                repo: String::new()
            }
            .label(),
            "github"
        );
        assert_eq!(
            MarketplaceSource::GitUrl { url: String::new() }.label(),
            "git"
        );
        assert_eq!(
            MarketplaceSource::LocalPath {
                path: String::new()
            }
            .label(),
            "local"
        );
    }
}

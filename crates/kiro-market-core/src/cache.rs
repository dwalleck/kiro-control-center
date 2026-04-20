//! Cache directory and marketplace registry management.
//!
//! All persistent state lives under `~/.local/share/kiro-market/` (or the
//! platform-appropriate data directory). This module provides [`CacheDir`] for
//! creating the directory structure and managing the `known_marketplaces.json`
//! registry file.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use tracing::debug;

use crate::error::MarketplaceError;
use crate::git::GitProtocol;
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
    /// - Starts with `http://`, `https://`, `file://`, or `git@` → `GitUrl`
    /// - Is an absolute path or starts with `./`, `../`, `~` → `LocalPath`
    /// - Anything else → `GitHub` (owner/repo shorthand)
    #[must_use]
    pub fn detect(source: &str) -> Self {
        if source.starts_with("http://")
            || source.starts_with("https://")
            || source.starts_with("file://")
            || source.starts_with("git@")
        {
            Self::GitUrl {
                url: source.to_owned(),
            }
        } else if Path::new(source).is_absolute()
            || source.starts_with('/')
            || source.starts_with("./")
            || source.starts_with("../")
            || source.starts_with(".\\")
            || source.starts_with("..\\")
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

    /// Derive a marketplace name from the source when no manifest provides one.
    ///
    /// Extracts the last path/URL segment, strips a `.git` suffix if present,
    /// and validates the result. Returns `None` if the derived name fails
    /// validation.
    #[must_use]
    pub fn fallback_name(&self) -> Option<String> {
        let raw = match self {
            Self::GitHub { repo } => repo.rsplit('/').next(),
            Self::GitUrl { url } => url.rsplit('/').next().or_else(|| url.rsplit(':').next()),
            Self::LocalPath { path } => {
                let trimmed = path.trim_end_matches(['/', '\\']);
                trimmed.rsplit(['/', '\\']).next()
            }
        };

        let segment = raw?;
        let name = segment.strip_suffix(".git").unwrap_or(segment);

        if name.is_empty() {
            return None;
        }

        if let Err(e) = validation::validate_name(name) {
            debug!(
                name = name,
                error = %e,
                "derived marketplace name fails validation"
            );
            return None;
        }
        Some(name.to_owned())
    }
}

/// Resolve a local path string to an absolute path.
///
/// Handles `~` expansion (via `dirs::home_dir()`) and canonicalization.
///
/// # Trust boundary
///
/// This function intentionally trusts whatever path the caller hands it.
/// Canonicalization follows symlinks all the way to the target, so any
/// chain (`~/marketplaces/foo` → `/etc`) resolves wherever it points.
/// That is the correct behavior when the user explicitly asks the CLI to
/// link an arbitrary path — they have authority over their own filesystem
/// and the symlink chain is part of that input.
///
/// Embedders that route untrusted strings into this function (e.g. the
/// Tauri desktop app accepting a path from a renderer message) MUST NOT
/// rely on it to confine the result. Use [`resolve_local_path_restricted`]
/// instead, which fails when the canonical target falls outside an
/// allowed-roots list.
///
/// # Errors
///
/// Returns an I/O error if the home directory cannot be determined or the
/// path cannot be canonicalized (e.g. does not exist).
pub fn resolve_local_path(path_str: &str) -> std::io::Result<PathBuf> {
    let expanded = expand_tilde(path_str)?;
    expanded.canonicalize()
}

/// Resolve a local path string and require the canonical target to lie
/// inside one of `allowed_roots`. Defense-in-depth wrapper for callers
/// (Tauri commands, multi-tenant servers, future plugin sandbox) that
/// need to confine untrusted input.
///
/// Each entry in `allowed_roots` is itself canonicalized, so `~/foo` and
/// `./foo/../foo` both work as roots. The check uses canonical-prefix
/// match, so a path that traverses through a symlink ending outside the
/// allowed set is rejected even if the original string looked OK.
///
/// `allowed_roots` being empty rejects every path — same posture as
/// "deny by default" firewall rules.
///
/// # Errors
///
/// - I/O errors from canonicalization of either the input path or any
///   allowed root.
/// - [`std::io::ErrorKind::PermissionDenied`] if the canonical target is
///   not under any allowed root. The error message names the rejected
///   path so the caller can surface it.
pub fn resolve_local_path_restricted(
    path_str: &str,
    allowed_roots: &[&Path],
) -> std::io::Result<PathBuf> {
    let resolved = resolve_local_path(path_str)?;

    for root in allowed_roots {
        let canonical_root = root.canonicalize()?;
        if resolved.starts_with(&canonical_root) {
            return Ok(resolved);
        }
    }

    Err(std::io::Error::new(
        std::io::ErrorKind::PermissionDenied,
        format!(
            "resolved path `{}` is outside the allowed root set",
            resolved.display()
        ),
    ))
}

/// Helper: expand a leading `~` to the user's home directory.
fn expand_tilde(path_str: &str) -> std::io::Result<PathBuf> {
    if let Some(rest) = path_str.strip_prefix('~') {
        let home = dirs::home_dir().ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "could not determine home directory for ~ expansion",
            )
        })?;
        Ok(if rest.is_empty() {
            home
        } else {
            home.join(rest.trim_start_matches(['/', '\\']))
        })
    } else {
        Ok(PathBuf::from(path_str))
    }
}

/// One per-path failure reported by [`CacheDir::prune_orphans`]. The
/// error is captured as a string rather than a typed `io::Error` so the
/// report can be cloned and sent across threads / over IPC without
/// additional plumbing.
///
/// `#[non_exhaustive]` so adding context fields later (e.g. a typed
/// `kind: PruneFailureKind`) is additive for external consumers.
#[derive(Clone, Debug, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[non_exhaustive]
pub struct PruneFailure {
    pub path: PathBuf,
    pub error: String,
}

/// Whether [`CacheDir::prune_orphans`] should actually delete or only
/// report what it would delete. Replaces a `dry_run: bool` parameter
/// (and the `removed` / `would_remove` parallel-Vec encoding on the
/// report). Callers ask "what mode produced this report?" and the
/// answer tells them whether `targets` references files that exist or
/// files that would have been deleted.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum PruneMode {
    /// Default: prune actually removes orphaned entries from disk.
    #[default]
    Apply,
    /// Identify orphans without deleting them. The resulting
    /// [`PruneReport::targets`] lists what *would* be removed.
    DryRun,
}

/// Outcome of a [`CacheDir::prune_orphans`] call.
///
/// The `mode` field disambiguates whether `targets` records actual
/// deletions ([`PruneMode::Apply`]) or a list of would-be deletions
/// ([`PruneMode::DryRun`]). Earlier versions encoded this as parallel
/// `removed` / `would_remove` `Vec`s, which let callers construct
/// nonsensical reports (both populated) and made `match` statements
/// against the report shape harder to write.
///
/// `#[non_exhaustive]` so extending the report with new summary fields
/// (totals, time-taken, …) is additive.
#[derive(Clone, Debug, Default, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[non_exhaustive]
pub struct PruneReport {
    /// Which mode produced the report.
    pub mode: PruneMode,
    /// Paths the prune acted on (when `mode == Apply`) or would have
    /// acted on (when `mode == DryRun`).
    pub targets: Vec<PathBuf>,
    /// Per-path failures during deletion. Reported rather than thrown so
    /// a single permission error doesn't abort the whole prune.
    pub failed: Vec<PruneFailure>,
}

/// An entry in the known-marketplaces registry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnownMarketplace {
    pub name: String,
    pub source: MarketplaceSource,
    /// Git protocol used when cloning GitHub shorthand sources.
    /// `None` for entries created before protocol selection was added.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protocol: Option<GitProtocol>,
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
        // Allow overriding the data directory for testing and CI. The `dirs`
        // crate uses platform-native APIs on macOS/Windows that ignore
        // `XDG_DATA_HOME`, so this env var provides cross-platform isolation.
        if let Ok(path) = std::env::var("KIRO_MARKET_DATA_DIR") {
            return Some(Self {
                root: PathBuf::from(path),
            });
        }
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

    /// Directory where per-marketplace plugin registries are stored.
    #[must_use]
    fn registries_dir(&self) -> PathBuf {
        self.root.join("registries")
    }

    /// Path to a marketplace's plugin registry file.
    #[must_use]
    pub fn plugin_registry_path(&self, marketplace: &str) -> PathBuf {
        self.registries_dir().join(format!("{marketplace}.json"))
    }

    /// Create all required subdirectories if they do not already exist.
    ///
    /// # Errors
    ///
    /// Returns an [`std::io::Error`] if directory creation fails.
    pub fn ensure_dirs(&self) -> std::io::Result<()> {
        fs::create_dir_all(self.marketplaces_dir())?;
        fs::create_dir_all(self.plugins_dir())?;
        fs::create_dir_all(self.registries_dir())?;
        Ok(())
    }

    /// Load the plugin registry for a marketplace.
    ///
    /// Returns `None` if the registry file does not exist (e.g. marketplace
    /// was added before the registry feature). The caller should fall back
    /// to reading `marketplace.json` directly and regenerate the registry.
    ///
    /// # Errors
    ///
    /// Returns an error on I/O or JSON parse failures.
    pub fn load_plugin_registry(
        &self,
        marketplace: &str,
    ) -> Result<Option<Vec<crate::marketplace::PluginEntry>>, crate::error::Error> {
        let path = self.plugin_registry_path(marketplace);
        match fs::read(&path) {
            Ok(bytes) => {
                let entries = serde_json::from_slice(&bytes)?;
                Ok(Some(entries))
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Write the plugin registry for a marketplace.
    ///
    /// # Errors
    ///
    /// Returns an error if directory creation or file write fails.
    pub fn write_plugin_registry(
        &self,
        marketplace: &str,
        plugins: &[crate::marketplace::PluginEntry],
    ) -> Result<(), crate::error::Error> {
        let path = self.plugin_registry_path(marketplace);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(plugins)?;
        atomic_write(&path, json.as_bytes())?;
        Ok(())
    }

    // -- known marketplaces registry ----------------------------------------

    /// Path to the `known_marketplaces.json` file.
    ///
    /// Visible to the rest of the crate so the service layer can take the
    /// same advisory lock used here. Without that, the service-level
    /// "directory exists?" check and the registry-level "name already
    /// registered?" check would each acquire their own lock, leaving a
    /// race window between them where two `add` calls for the same name
    /// can both pass the exists check and clobber each other on rename.
    pub(crate) fn registry_path(&self) -> PathBuf {
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

    /// Remove cached marketplace clones and plugin clones whose
    /// marketplace is no longer in `known_marketplaces.json`.
    ///
    /// Two trees get walked:
    /// - `marketplaces/<name>/` — the cloned (or copy-fallback) repo.
    ///   A registered marketplace whose source is `LocalPath` is linked
    ///   via [`crate::platform::create_local_link`] which leaves a
    ///   symlink/junction or a directory copy here; either way it's
    ///   represented as one entry under `marketplaces/`.
    /// - `plugins/<marketplace>/` — per-marketplace plugin clones
    ///   created lazily by [`crate::service::MarketplaceService::resolve_plugin_dir`]
    ///   for `Structured` plugin sources. A whole subtree is orphaned
    ///   when its marketplace is unregistered.
    ///
    /// We deliberately do NOT prune individual plugin directories under
    /// a still-registered marketplace, because they may be mid-install
    /// on another process or kept as a warm cache for the next install.
    /// The advisory-locking story would also have to account for
    /// per-plugin races, which is more cost than the benefit warrants.
    ///
    /// `_pending_*` staging directories from a partially-failed `add`
    /// are also swept — those are guaranteed unreferenced because the
    /// successful path retargets the guard onto the final name.
    ///
    /// # Errors
    ///
    /// Returns an [`Error`] only for failures loading the registry. Per
    /// path I/O failures during deletion are collected into the report's
    /// `failed` field rather than aborting the prune.
    pub fn prune_orphans(&self, mode: PruneMode) -> crate::error::Result<PruneReport> {
        let registered: std::collections::HashSet<String> = self
            .load_known_marketplaces()?
            .into_iter()
            .map(|e| e.name)
            .collect();

        let mut report = PruneReport {
            mode,
            ..PruneReport::default()
        };

        for (label, dir) in [
            ("marketplaces", self.marketplaces_dir()),
            ("plugins", self.plugins_dir()),
        ] {
            Self::prune_dir_orphans(label, &dir, &registered, &mut report);
        }

        // The `registries/` directory holds `<marketplace>.json` files,
        // not subdirectories. Sweep separately because the filename has
        // an extension and the entry is a file. Defense in depth against
        // a remove-vs-add race that briefly drops the registry file
        // out from under an in-flight add (the add path now writes the
        // plugin registry inside the registry lock, but a stale entry
        // from a pre-fix install could still linger).
        Self::prune_registries_orphans(&self.registries_dir(), &registered, &mut report);

        // Stale per-plugin `.lock` files inside *registered* marketplace
        // dirs. `with_file_lock(dest)` puts a `<plugin>.lock` sibling
        // next to `<plugin>/` and never removes it (removing during use
        // would race with concurrent install attempts that re-create
        // the file under a different inode and break flock's mutual
        // exclusion). Lock files for *unregistered* marketplaces are
        // already swept by the marketplaces/plugins parent removal
        // above; here we only handle the case where the user manually
        // deleted `<plugin>/` and left the `.lock` orphan.
        for mp in &registered {
            Self::prune_stale_plugin_locks(&self.plugins_dir().join(mp), &mut report);
        }

        Ok(report)
    }

    /// Walk a single marketplace's plugins/ subdir and remove `.lock`
    /// files whose corresponding `<plugin>/` directory is gone. Caller
    /// guarantees the parent dir is for a *registered* marketplace
    /// (locks under unregistered marketplaces are handled by the parent
    /// directory removal in `prune_dir_orphans`).
    ///
    /// Skipped if the dir doesn't exist (a registered marketplace with
    /// no plugin clones yet). Per-entry I/O failures land in
    /// `report.failed`; transient errors don't abort the sweep.
    fn prune_stale_plugin_locks(plugins_subdir: &Path, report: &mut PruneReport) {
        let entries = match fs::read_dir(plugins_subdir) {
            Ok(e) => e,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return,
            Err(e) => {
                report.failed.push(PruneFailure {
                    path: plugins_subdir.to_path_buf(),
                    error: format!("read_dir(plugin lock sweep): {e}"),
                });
                return;
            }
        };
        for entry in entries {
            let entry = match entry {
                Ok(e) => e,
                Err(e) => {
                    report.failed.push(PruneFailure {
                        path: plugins_subdir.to_path_buf(),
                        error: format!("read_dir entry in plugin lock sweep: {e}"),
                    });
                    continue;
                }
            };
            let path = entry.path();
            // Only act on `<plugin>.lock` files. Other files in the dir
            // (none should exist, but defensively) are ignored.
            if path.extension().and_then(|e| e.to_str()) != Some("lock") {
                continue;
            }
            // Map `<plugin>.lock` → `<plugin>/` and check if the plugin
            // dir is gone. If it still exists, the lock is potentially
            // in use (or kept warm for the next install) — leave it.
            let Some(stem) = path.file_stem() else {
                continue;
            };
            let plugin_dir = plugins_subdir.join(stem);
            if plugin_dir.exists() {
                continue;
            }
            if matches!(report.mode, PruneMode::DryRun) {
                report.targets.push(path);
                continue;
            }
            match fs::remove_file(&path) {
                Ok(()) => report.targets.push(path),
                Err(e) => report.failed.push(PruneFailure {
                    path,
                    error: e.to_string(),
                }),
            }
        }
    }

    /// Walk the `registries/` directory removing `<marketplace>.json`
    /// entries whose stem is not in `registered`. Per-entry I/O failures
    /// land in `report.failed`; the function never propagates an error
    /// for a single entry so a transient EACCES on one stale registry
    /// does not block the rest of the sweep.
    fn prune_registries_orphans(
        dir: &Path,
        registered: &std::collections::HashSet<String>,
        report: &mut PruneReport,
    ) {
        let entries = match fs::read_dir(dir) {
            Ok(e) => e,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return,
            Err(e) => {
                report.failed.push(PruneFailure {
                    path: dir.to_path_buf(),
                    error: format!("read_dir(registries): {e}"),
                });
                return;
            }
        };
        for entry in entries {
            let entry = match entry {
                Ok(e) => e,
                Err(e) => {
                    report.failed.push(PruneFailure {
                        path: dir.to_path_buf(),
                        error: format!("read_dir entry in registries: {e}"),
                    });
                    continue;
                }
            };
            let path = entry.path();
            // Only act on `<name>.json` files. Anything else (a stray
            // dotfile, a partial write, an unrelated artefact) is left
            // alone — the prune contract is "remove orphan registry
            // entries," not "clean the directory."
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            if registered.contains(stem) {
                continue;
            }
            if matches!(report.mode, PruneMode::DryRun) {
                report.targets.push(path);
                continue;
            }
            match fs::remove_file(&path) {
                Ok(()) => report.targets.push(path),
                Err(e) => report.failed.push(PruneFailure {
                    path,
                    error: e.to_string(),
                }),
            }
        }
    }

    /// Walk `dir` removing entries whose name is not in `registered`.
    /// `_pending_*` entries are always orphaned (a successful add
    /// renames them away). Errors per entry land in `report.failed`.
    ///
    /// Free function rather than method: it doesn't read `CacheDir`
    /// state, just operates on the supplied path and registered set.
    /// Keeping it inside the impl block via `Self::` would force a
    /// `&self` it doesn't need.
    fn prune_dir_orphans(
        label: &str,
        dir: &Path,
        registered: &std::collections::HashSet<String>,
        report: &mut PruneReport,
    ) {
        let entries = match fs::read_dir(dir) {
            Ok(e) => e,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return,
            Err(e) => {
                report.failed.push(PruneFailure {
                    path: dir.to_path_buf(),
                    error: format!("read_dir({label}): {e}"),
                });
                return;
            }
        };
        for entry in entries {
            let entry = match entry {
                Ok(e) => e,
                Err(e) => {
                    report.failed.push(PruneFailure {
                        path: dir.to_path_buf(),
                        error: format!("read_dir entry in {label}: {e}"),
                    });
                    continue;
                }
            };
            let name = entry.file_name();
            let Some(name_str) = name.to_str() else {
                continue;
            };
            // Orphan iff the entry's name is not in the registry.
            //
            // The `_pending_*` staging dirs that service.rs leaves behind on a
            // crashed `add` already satisfy this: they're never registered
            // because the registration step happens AFTER the rename away from
            // `_pending_*`. So registered-check alone catches them — adding
            // an explicit `starts_with("_pending_")` short-circuit ahead of
            // the registered check would mean a *legitimately registered*
            // marketplace named `_pending_<x>` (validate_name does not reject
            // leading underscores) gets deleted on every prune. Don't.
            if registered.contains(name_str) {
                continue;
            }
            let path = entry.path();
            if matches!(report.mode, PruneMode::DryRun) {
                report.targets.push(path);
                continue;
            }
            match fs::remove_dir_all(&path) {
                Ok(()) => report.targets.push(path),
                Err(e) => report.failed.push(PruneFailure {
                    path,
                    error: e.to_string(),
                }),
            }
        }
    }

    /// Add a marketplace to the registry, persisting to disk.
    ///
    /// Acquires the registry advisory lock for the read-modify-write cycle.
    /// Callers that already hold the lock (e.g. the service layer wrapping
    /// a directory rename + register together) should use
    /// [`Self::register_known_marketplace_unlocked`] instead — taking the
    /// same lock twice from the same process would self-contend: the
    /// inner acquire opens a fresh fd whose `try_lock_exclusive` cannot
    /// succeed until the outer fd is dropped, so the inner caller stalls
    /// the polling loop in `with_file_lock` until `LOCK_TIMEOUT` (10 s)
    /// elapses and returns `ErrorKind::TimedOut`.
    ///
    /// # Errors
    ///
    /// - [`MarketplaceError::AlreadyRegistered`] if a marketplace with the
    ///   same name already exists.
    /// - I/O or JSON serialisation errors.
    pub fn add_known_marketplace(&self, entry: KnownMarketplace) -> crate::error::Result<()> {
        crate::file_lock::with_file_lock(&self.registry_path(), move || {
            self.register_known_marketplace_unlocked(entry)
        })
    }

    /// Append `entry` to the registry without acquiring the registry lock.
    ///
    /// **Caller must already hold** `with_file_lock(self.registry_path(), ...)`
    /// or risk a torn read-modify-write. Used by the service layer's
    /// `add` flow so the existence check, directory rename, and registry
    /// write all happen inside one critical section.
    ///
    /// # Errors
    ///
    /// Same as [`Self::add_known_marketplace`].
    pub(crate) fn register_known_marketplace_unlocked(
        &self,
        entry: KnownMarketplace,
    ) -> crate::error::Result<()> {
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
        crate::file_lock::with_file_lock(&self.registry_path(), || {
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
        })
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
///
/// Durability contract: after this function returns `Ok(())`, the bytes at
/// `path` survive a power loss or kernel crash. Achieved in three steps:
///
/// 1. Write the tmp file and `sync_all()` it before close. Without this, a
///    crash after `rename(2)` but before the page-cache flush can surface
///    the new path with zero bytes (the rename is durable, the data is
///    not).
/// 2. `rename(2)` the tmp into place.
/// 3. **Unix only:** `fsync` the parent directory so the rename itself is
///    journaled. Without step 3 the rename can be reordered out of the
///    on-disk journal even though the file's data is durable. Windows
///    does not expose a portable directory-fsync; on NTFS we rely on the
///    `MoveFileEx` semantics + the journal commit chain that follows a
///    successful rename. (Stricter durability than that requires
///    `FlushFileBuffers` on a directory handle, which is not available
///    on every Windows file system.)
///
/// # Edge case: parent fsync after rename
///
/// If `rename` succeeds but the parent-dir `sync_all()` then errors,
/// the new content IS visible at `path` (the rename committed) but this
/// function returns `Err`. A caller that retries on error will re-write
/// the same content — idempotent and safe — but a caller that interprets
/// the error as "nothing happened" would be wrong about the on-disk
/// state. The error is rare (parent fsync errors usually indicate an
/// underlying disk failure that will keep failing) and the alternative
/// (swallow the error) would weaken the durability guarantee for the
/// common path. Document and propagate.
///
/// # `path` must be absolute
///
/// `debug_assert` on absolute path because the parent-fsync fallback
/// to `"."` for relative inputs would silently fsync the wrong
/// directory if the caller's CWD changed between resolving the path
/// and reaching here. All callers in this crate construct paths from
/// [`CacheDir`] / [`crate::project::KiroProject`], both of which carry
/// an absolute root.
///
/// Callers are expected to hold a [`crate::file_lock`] around concurrent
/// writes to the same target — the `.tmp` filename is shared and would
/// otherwise race.
pub(crate) fn atomic_write(path: &Path, data: &[u8]) -> io::Result<()> {
    debug_assert!(
        path.is_absolute(),
        "atomic_write requires an absolute path; got `{}`. \
         Relative paths fsync the wrong directory if CWD changes during the call.",
        path.display()
    );

    let tmp = path.with_extension("tmp");

    // Open + write + sync the tmp file in its own scope so the handle
    // is closed (and any OS-side dirty buffers flushed for that fd)
    // before we issue the rename.
    {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&tmp)?;
        file.write_all(data)?;
        file.sync_all()?;
    }

    fs::rename(&tmp, path)?;

    // POSIX: the rename's metadata change must itself be flushed to the
    // directory inode before we can claim durability. Opening the parent
    // O_RDONLY and `sync_all()` on the resulting handle is the portable
    // way (no `fsync(dir_fd)` needs raw libc). See the function-level
    // doc on the rename-then-fsync-fails edge case.
    #[cfg(unix)]
    {
        // `path.parent()` returns `Some("")` for a bare filename like
        // `state.json`; treat that as the current working directory so the
        // open call doesn't error on an empty path. (In release builds
        // the debug_assert above is a no-op, so this fallback is the
        // safety net for an accidentally-relative path.)
        let parent = path.parent().filter(|p| !p.as_os_str().is_empty());
        let dir_path = parent.unwrap_or_else(|| Path::new("."));
        fs::File::open(dir_path)?.sync_all()?;
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use rstest::rstest;

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
            protocol: None,
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
            protocol: None,
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
            protocol: None,
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
            protocol: None,
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
            protocol: None,
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
        // Use Path::ends_with which compares by components (cross-platform).
        assert!(mp.ends_with(Path::new("marketplaces").join("my-market")));

        let pp = cache.plugin_path("my-market", "my-plugin");
        assert!(pp.ends_with(Path::new("plugins").join("my-market").join("my-plugin")));
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
            protocol: None,
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
    // Plugin registry
    // -----------------------------------------------------------------------

    #[test]
    fn plugin_registry_roundtrip() {
        let (_dir, cache) = temp_cache();
        cache.ensure_dirs().expect("ensure_dirs");

        let entries = vec![
            crate::marketplace::PluginEntry {
                name: "dotnet".into(),
                description: Some("Core .NET skills".into()),
                source: crate::marketplace::PluginSource::RelativePath(
                    crate::validation::RelativePath::new("./plugins/dotnet").unwrap(),
                ),
            },
            crate::marketplace::PluginEntry {
                name: "dotnet-experimental".into(),
                description: Some("Experimental skills".into()),
                source: crate::marketplace::PluginSource::RelativePath(
                    crate::validation::RelativePath::new("./plugins/dotnet-experimental").unwrap(),
                ),
            },
        ];

        cache
            .write_plugin_registry("my-market", &entries)
            .expect("write should succeed");

        let loaded = cache
            .load_plugin_registry("my-market")
            .expect("load should succeed")
            .expect("registry should exist");

        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].name, "dotnet");
        assert_eq!(loaded[1].name, "dotnet-experimental");
    }

    #[test]
    fn load_plugin_registry_returns_none_when_no_file() {
        let (_dir, cache) = temp_cache();
        cache.ensure_dirs().expect("ensure_dirs");

        let result = cache
            .load_plugin_registry("nonexistent")
            .expect("load should succeed");

        assert!(result.is_none());
    }

    #[test]
    fn plugin_registry_roundtrip_preserves_source() {
        let (_dir, cache) = temp_cache();
        cache.ensure_dirs().expect("ensure_dirs");

        let entries = vec![crate::marketplace::PluginEntry {
            name: "dotnet".into(),
            description: Some("Core .NET skills".into()),
            source: crate::marketplace::PluginSource::RelativePath(
                crate::validation::RelativePath::new("./plugins/dotnet").unwrap(),
            ),
        }];

        cache
            .write_plugin_registry("source-test", &entries)
            .expect("write should succeed");

        let loaded = cache
            .load_plugin_registry("source-test")
            .expect("load should succeed")
            .expect("registry should exist");

        assert_eq!(loaded.len(), 1);
        match &loaded[0].source {
            crate::marketplace::PluginSource::RelativePath(p) => {
                assert_eq!(p, "./plugins/dotnet");
            }
            crate::marketplace::PluginSource::Structured(s) => {
                panic!("expected RelativePath source, got Structured({s:?})")
            }
        }
    }

    #[test]
    fn remove_known_marketplace_leaves_other_entries() {
        let (_dir, cache) = temp_cache();
        cache.ensure_dirs().expect("ensure_dirs");

        for name in ["alpha", "beta", "gamma"] {
            let entry = KnownMarketplace {
                name: name.into(),
                source: MarketplaceSource::GitHub {
                    repo: format!("owner/{name}"),
                },
                protocol: None,
                added_at: Utc::now(),
            };
            cache
                .add_known_marketplace(entry)
                .expect("add should succeed");
        }

        cache
            .remove_known_marketplace("beta")
            .expect("remove should succeed");

        let remaining = cache
            .load_known_marketplaces()
            .expect("load should succeed");

        assert_eq!(remaining.len(), 2);
        assert!(
            !remaining.iter().any(|e| e.name == "beta"),
            "beta should have been removed"
        );
    }

    #[test]
    fn known_marketplace_deserializes_without_protocol_field() {
        let json = r#"{
            "name": "legacy-market",
            "source": {"type": "github", "repo": "owner/repo"},
            "added_at": "2025-01-01T00:00:00Z"
        }"#;
        let entry: KnownMarketplace =
            serde_json::from_str(json).expect("should deserialize without protocol");
        assert_eq!(entry.name, "legacy-market");
        assert!(entry.protocol.is_none());
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
    fn detect_file_url() {
        let source = MarketplaceSource::detect("file:///home/user/marketplace");
        assert!(
            matches!(source, MarketplaceSource::GitUrl { url } if url == "file:///home/user/marketplace")
        );
    }

    #[cfg(windows)]
    #[test]
    fn detect_windows_drive_path() {
        let source = MarketplaceSource::detect(r"C:\Users\runner\marketplace");
        assert!(
            matches!(source, MarketplaceSource::LocalPath { .. }),
            "expected LocalPath for Windows drive path, got {source:?}"
        );
    }

    #[cfg(windows)]
    #[test]
    fn detect_windows_drive_path_forward_slash() {
        let source = MarketplaceSource::detect("D:/repos/marketplace");
        assert!(
            matches!(source, MarketplaceSource::LocalPath { .. }),
            "expected LocalPath for Windows drive path, got {source:?}"
        );
    }

    #[cfg(windows)]
    #[test]
    fn detect_unc_path() {
        let source = MarketplaceSource::detect(r"\\server\share\marketplace");
        assert!(
            matches!(source, MarketplaceSource::LocalPath { .. }),
            "expected LocalPath for UNC path, got {source:?}"
        );
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

    // -----------------------------------------------------------------------
    // MarketplaceSource::fallback_name
    // -----------------------------------------------------------------------

    #[rstest]
    #[case::github("owner/skills", "skills")]
    #[case::github_nested("org/sub-repo", "sub-repo")]
    #[case::git_url_https("https://github.com/dotnet/skills.git", "skills")]
    #[case::git_url_no_suffix("https://github.com/dotnet/skills", "skills")]
    #[case::git_ssh("git@github.com:owner/repo.git", "repo")]
    #[case::local_path("/home/user/my-plugins", "my-plugins")]
    #[case::local_tilde("~/marketplaces/mine", "mine")]
    #[case::local_relative("./my-market", "my-market")]
    fn fallback_name_derives_from_source(#[case] source_str: &str, #[case] expected: &str) {
        let source = MarketplaceSource::detect(source_str);
        let name = source.fallback_name();
        assert_eq!(
            name.as_deref(),
            Some(expected),
            "fallback name for '{source_str}'"
        );
    }

    #[test]
    fn fallback_name_returns_none_for_invalid_name() {
        let source = MarketplaceSource::LocalPath {
            path: "/home/user/..".into(),
        };
        assert!(
            source.fallback_name().is_none(),
            "should return None for invalid name"
        );
    }

    // -----------------------------------------------------------------------
    // prune_orphans
    // -----------------------------------------------------------------------

    #[test]
    fn prune_orphans_returns_empty_report_when_cache_is_clean() {
        let (_dir, cache) = temp_cache();
        cache.ensure_dirs().expect("ensure_dirs");
        cache
            .add_known_marketplace(KnownMarketplace {
                name: "current".into(),
                source: MarketplaceSource::GitHub {
                    repo: "owner/repo".into(),
                },
                protocol: None,
                added_at: Utc::now(),
            })
            .expect("add");
        // Pretend a clone exists for the registered marketplace.
        fs::create_dir_all(cache.marketplace_path("current")).expect("mkdir");
        fs::create_dir_all(cache.plugins_dir().join("current")).expect("mkdir");

        let report = cache.prune_orphans(PruneMode::Apply).expect("prune");
        assert!(
            report.targets.is_empty() && report.failed.is_empty(),
            "registered marketplace must not be touched: {report:?}"
        );
    }

    #[test]
    fn prune_orphans_removes_unregistered_marketplace_dir() {
        let (_dir, cache) = temp_cache();
        cache.ensure_dirs().expect("ensure_dirs");
        // No marketplace registered, but a stale clone exists on disk.
        let stale = cache.marketplace_path("ghost");
        fs::create_dir_all(&stale).expect("mkdir stale");
        fs::write(stale.join("README.md"), "leftover").expect("write");

        let report = cache.prune_orphans(PruneMode::Apply).expect("prune");
        assert_eq!(report.targets, vec![stale.clone()]);
        assert!(!stale.exists(), "orphan must be deleted");
    }

    #[test]
    fn prune_orphans_dry_run_lists_without_deleting() {
        let (_dir, cache) = temp_cache();
        cache.ensure_dirs().expect("ensure_dirs");
        let stale = cache.marketplace_path("ghost");
        fs::create_dir_all(&stale).expect("mkdir stale");

        let report = cache
            .prune_orphans(PruneMode::DryRun)
            .expect("prune dry-run");
        assert_eq!(report.mode, PruneMode::DryRun);
        assert_eq!(
            report.targets,
            vec![stale.clone()],
            "dry-run report should list the would-be-deleted path under `targets`"
        );
        assert!(stale.exists(), "dry-run mode must NOT touch the filesystem");
    }

    #[test]
    fn prune_orphans_sweeps_pending_staging_dirs() {
        let (_dir, cache) = temp_cache();
        cache.ensure_dirs().expect("ensure_dirs");
        // The `_pending_<pid>_<seq>` prefix is what service.rs::add uses
        // for staging directories before the rename. Such a name is never
        // user-supplied and never registered — registered-check alone
        // sweeps it as orphan.
        let pending = cache.marketplaces_dir().join("_pending_99999_0");
        fs::create_dir_all(&pending).expect("mkdir pending");

        let report = cache.prune_orphans(PruneMode::Apply).expect("prune");
        assert_eq!(report.targets, vec![pending.clone()]);
        assert!(!pending.exists());
    }

    #[test]
    fn prune_orphans_preserves_registered_marketplace_with_pending_name() {
        // Regression test for a real bug found in PR review: an earlier
        // version of prune_orphans short-circuited on `name_str.starts_with("_pending_")`
        // BEFORE the registered-check, so a user who registered a
        // marketplace named `_pending_acme` (validate_name does not reject
        // leading underscores) would have it deleted on the next prune.
        // Data loss. The fix is to drop the special-case and rely solely
        // on the registered check — `_pending_*` staging dirs are never
        // registered by construction, so they still get swept.
        let (_dir, cache) = temp_cache();
        cache.ensure_dirs().expect("ensure_dirs");

        let entry = KnownMarketplace {
            name: "_pending_legit".into(),
            source: MarketplaceSource::GitHub {
                repo: "owner/repo".into(),
            },
            protocol: None,
            added_at: Utc::now(),
        };
        cache
            .add_known_marketplace(entry)
            .expect("registering a `_pending_*` name must succeed (validate_name allows it)");

        // Materialize the marketplace's clone dir + plugin dir as
        // prune_orphans walks the parent directories.
        fs::create_dir_all(cache.marketplace_path("_pending_legit"))
            .expect("create marketplace dir");
        fs::create_dir_all(cache.plugins_dir().join("_pending_legit"))
            .expect("create plugins subdir");

        let report = cache.prune_orphans(PruneMode::Apply).expect("prune");
        assert!(
            report.targets.is_empty(),
            "registered `_pending_legit` must NOT be swept: {report:?}"
        );
        assert!(
            cache.marketplace_path("_pending_legit").exists(),
            "marketplace dir was deleted despite being registered — data loss bug regressed"
        );
        assert!(
            cache.plugins_dir().join("_pending_legit").exists(),
            "plugins subdir was deleted despite the marketplace being registered"
        );
    }

    #[test]
    fn prune_orphans_sweeps_orphaned_plugin_registry_file() {
        // Defense-in-depth: even though the add path now writes the
        // plugin registry inside the registry lock, a stale
        // `registries/<name>.json` from a prior install (before the
        // lock was extended, or from a manually-deleted marketplace)
        // should be cleaned up by prune.
        let (_dir, cache) = temp_cache();
        cache.ensure_dirs().expect("ensure_dirs");

        // Plant an orphan registry file with no corresponding
        // known_marketplaces entry.
        let orphan_path = cache.plugin_registry_path("ghostly");
        fs::write(&orphan_path, b"[]").expect("write orphan registry");
        assert!(orphan_path.exists());

        let report = cache.prune_orphans(PruneMode::Apply).expect("prune");
        assert!(
            report.targets.contains(&orphan_path),
            "orphaned registry file must be deleted: {report:?}"
        );
        assert!(!orphan_path.exists());
    }

    #[test]
    fn prune_orphans_preserves_registry_for_registered_marketplace() {
        let (_dir, cache) = temp_cache();
        cache.ensure_dirs().expect("ensure_dirs");
        cache
            .add_known_marketplace(KnownMarketplace {
                name: "live".into(),
                source: MarketplaceSource::GitHub {
                    repo: "owner/repo".into(),
                },
                protocol: None,
                added_at: Utc::now(),
            })
            .expect("register live marketplace");

        let live_registry = cache.plugin_registry_path("live");
        fs::write(&live_registry, b"[]").expect("write live registry");

        let report = cache.prune_orphans(PruneMode::Apply).expect("prune");
        assert!(
            !report.targets.contains(&live_registry),
            "registry for a registered marketplace must NOT be deleted: {report:?}"
        );
        assert!(
            live_registry.exists(),
            "registry file should remain on disk"
        );
    }

    #[test]
    fn prune_orphans_ignores_non_json_files_in_registries_dir() {
        // Stray files (an editor backup, a partial write) are out of
        // scope for prune; we should not delete them, only `<name>.json`
        // entries that correspond to no registered marketplace.
        let (_dir, cache) = temp_cache();
        cache.ensure_dirs().expect("ensure_dirs");

        let stray = cache.registries_dir().join("README");
        fs::write(&stray, b"docs").expect("write stray file");

        let report = cache.prune_orphans(PruneMode::Apply).expect("prune");
        assert!(
            !report.targets.contains(&stray),
            "non-`.json` files must be left alone: {report:?}"
        );
        assert!(stray.exists());
    }

    #[test]
    fn prune_orphans_sweeps_stale_plugin_lock_when_plugin_dir_is_gone() {
        // The user manually deleted `<plugin>/` but `<plugin>.lock` from a
        // prior `with_file_lock(...)` call lingers. Prune must remove it
        // so the lock files don't accumulate one-per-ever-installed-plugin.
        let (_dir, cache) = temp_cache();
        cache.ensure_dirs().expect("ensure_dirs");
        cache
            .add_known_marketplace(KnownMarketplace {
                name: "live".into(),
                source: MarketplaceSource::GitHub {
                    repo: "owner/repo".into(),
                },
                protocol: None,
                added_at: Utc::now(),
            })
            .expect("register live marketplace");

        let plugins_subdir = cache.plugins_dir().join("live");
        fs::create_dir_all(&plugins_subdir).expect("mkdir plugins/live");
        let stale_lock = plugins_subdir.join("orphan.lock");
        fs::write(&stale_lock, b"").expect("write orphan lock");

        let report = cache.prune_orphans(PruneMode::Apply).expect("prune");
        assert!(
            report.targets.contains(&stale_lock),
            "stale .lock with no matching dir must be deleted: {report:?}"
        );
        assert!(!stale_lock.exists());
    }

    #[test]
    fn prune_orphans_preserves_plugin_lock_when_plugin_dir_exists() {
        // The plugin is cached and its .lock is potentially in use (or
        // kept warm for the next install). Even if the marketplace's
        // registry is consulted, an active lock must not be removed —
        // unlinking under flock breaks mutual exclusion.
        let (_dir, cache) = temp_cache();
        cache.ensure_dirs().expect("ensure_dirs");
        cache
            .add_known_marketplace(KnownMarketplace {
                name: "live".into(),
                source: MarketplaceSource::GitHub {
                    repo: "owner/repo".into(),
                },
                protocol: None,
                added_at: Utc::now(),
            })
            .expect("register live marketplace");

        let plugins_subdir = cache.plugins_dir().join("live");
        fs::create_dir_all(plugins_subdir.join("active")).expect("mkdir active plugin");
        let active_lock = plugins_subdir.join("active.lock");
        fs::write(&active_lock, b"").expect("write active lock");

        let report = cache.prune_orphans(PruneMode::Apply).expect("prune");
        assert!(
            !report.targets.contains(&active_lock),
            "lock for an existing plugin dir must NOT be deleted: {report:?}"
        );
        assert!(active_lock.exists());
    }

    #[test]
    fn prune_orphans_handles_missing_directories_gracefully() {
        // Fresh cache with no plugins/ or marketplaces/ directory: the
        // prune must not error on NotFound.
        let dir = tempfile::tempdir().expect("tempdir");
        let cache = CacheDir::with_root(dir.path().to_path_buf());
        // Don't call ensure_dirs — directories don't exist.

        let report = cache.prune_orphans(PruneMode::Apply).expect("prune");
        assert!(report.targets.is_empty());
        assert!(report.failed.is_empty());
    }

    // -----------------------------------------------------------------------
    // resolve_local_path_restricted
    // -----------------------------------------------------------------------

    #[test]
    fn resolve_local_path_restricted_accepts_path_inside_root() {
        let dir = tempfile::tempdir().expect("tempdir");
        let nested = dir.path().join("plugins/foo");
        fs::create_dir_all(&nested).expect("mkdir");

        let resolved = resolve_local_path_restricted(nested.to_str().expect("utf8"), &[dir.path()])
            .expect("path inside root must be allowed");

        assert!(
            resolved.starts_with(dir.path().canonicalize().unwrap()),
            "resolved path should canonicalize within the allowed root"
        );
    }

    #[test]
    fn resolve_local_path_restricted_rejects_path_outside_root() {
        // Two sibling temp dirs; the input is in `outside`, the only
        // allowed root is `inside`. Without confinement the resolver would
        // happily return the outside path; with confinement it returns
        // PermissionDenied.
        let parent = tempfile::tempdir().expect("tempdir");
        let inside = parent.path().join("inside");
        let outside = parent.path().join("outside");
        fs::create_dir_all(&inside).expect("mkdir inside");
        fs::create_dir_all(&outside).expect("mkdir outside");

        let err =
            resolve_local_path_restricted(outside.to_str().expect("utf8"), &[inside.as_path()])
                .expect_err("path outside root must be rejected");

        assert_eq!(
            err.kind(),
            std::io::ErrorKind::PermissionDenied,
            "expected PermissionDenied, got {err:?}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn resolve_local_path_restricted_rejects_symlink_escaping_root() {
        // The classic confinement-bypass: input is inside the allowed
        // root, but it's a symlink pointing OUTSIDE. Naive prefix-check
        // on the input string would accept this; the canonicalization
        // step in resolve_local_path resolves the symlink, and the
        // post-canonicalization prefix check then rejects it.
        let parent = tempfile::tempdir().expect("tempdir");
        let inside = parent.path().join("inside");
        let outside_target = parent.path().join("outside_target");
        fs::create_dir_all(&inside).expect("mkdir inside");
        fs::create_dir_all(&outside_target).expect("mkdir outside");

        let escape = inside.join("escape");
        std::os::unix::fs::symlink(&outside_target, &escape).expect("create symlink");

        let err =
            resolve_local_path_restricted(escape.to_str().expect("utf8"), &[inside.as_path()])
                .expect_err("symlink escaping the root must be rejected");

        assert_eq!(err.kind(), std::io::ErrorKind::PermissionDenied);
    }

    #[test]
    fn resolve_local_path_restricted_with_empty_root_list_rejects_everything() {
        // Defense-in-depth posture: passing no allowed roots means deny
        // by default, never silently allow.
        let dir = tempfile::tempdir().expect("tempdir");
        let err = resolve_local_path_restricted(dir.path().to_str().unwrap(), &[])
            .expect_err("empty allow-list must reject everything");
        assert_eq!(err.kind(), std::io::ErrorKind::PermissionDenied);
    }

    #[test]
    fn resolve_local_path_restricted_accepts_input_equal_to_root() {
        // Boundary: input is exactly the allowed root. starts_with is
        // reflexive, so this should succeed. A regression that used
        // strict-prefix (e.g. swapped to `path != root && path.starts_with`)
        // would fail here.
        let dir = tempfile::tempdir().expect("tempdir");
        let resolved =
            resolve_local_path_restricted(dir.path().to_str().expect("utf8"), &[dir.path()])
                .expect("input == root must be allowed (starts_with is reflexive)");
        assert_eq!(resolved, dir.path().canonicalize().unwrap());
    }

    #[test]
    fn resolve_local_path_restricted_resolves_relative_input_against_cwd() {
        // A relative input is canonicalized against the process CWD,
        // not against the allowed root. Document this behaviour: the
        // allowed-roots check happens AFTER canonicalization, so a
        // `../escape` input either fails canonicalization (path doesn't
        // exist relative to CWD) or resolves to a CWD-anchored absolute
        // path that the allow-list won't contain.
        let parent = tempfile::tempdir().expect("tempdir");
        let inside = parent.path().join("inside");
        fs::create_dir_all(&inside).expect("mkdir inside");

        // `../inside` from CWD is almost certainly NOT a registered
        // root, so this must reject.
        let err = resolve_local_path_restricted("../this/is/probably/not/in/the/cwd", &[&inside])
            .expect_err(
                "relative path canonicalizes against CWD; nothing should be both \
                  in CWD AND inside `inside`",
            );
        // Either NotFound (canonicalize failed) or PermissionDenied
        // (canonicalized but not under root) is acceptable — both prove
        // the relative input didn't get smuggled past the allow-list.
        assert!(
            matches!(
                err.kind(),
                std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::NotFound
            ),
            "expected PermissionDenied or NotFound, got {err:?}"
        );
    }
}

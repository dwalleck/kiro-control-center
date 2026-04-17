//! Marketplace lifecycle operations.
//!
//! [`MarketplaceService`] centralizes add/remove/update/list logic so that
//! CLI and Tauri frontends remain thin presentation wrappers.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicU64;

use serde::Serialize;
use tracing::{debug, warn};

use crate::cache::{CacheDir, KnownMarketplace, MarketplaceSource};
use crate::error::{Error, MarketplaceError, error_full_chain};
use crate::git::{self, CloneOptions, GitBackend, GitProtocol};
use crate::marketplace::Marketplace;
use crate::platform::LinkResult;
use crate::{platform, validation};

/// Process-local sequence used to disambiguate concurrent `_pending_*` temp
/// directories during `add()`. Combined with `process::id()` so two threads
/// in the same process never collide on the staging path.
static PENDING_COUNTER: AtomicU64 = AtomicU64::new(0);

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
    /// How the marketplace contents are stored on disk. `Linked` means
    /// changes to the source are reflected immediately; `Copied` (Windows
    /// fallback when junctions fail) means the user must re-add to pick up
    /// upstream edits. The frontend should surface this for `Copied` so
    /// users aren't surprised that "live" updates do not work.
    pub storage: MarketplaceStorage,
}

/// How a registered marketplace's contents are stored on disk.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum MarketplaceStorage {
    /// Cloned from a remote git repository.
    Cloned,
    /// Linked to a local directory (symlink on Unix, junction on Windows).
    /// Edits to the source are visible immediately.
    Linked,
    /// Copied from a local directory (Windows fallback when junctions fail).
    /// Edits to the source require re-adding the marketplace.
    Copied,
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

/// Filter applied to a multi-skill install operation.
///
/// `All` installs every discovered skill. `Names(set)` keeps only skills
/// whose `SKILL.md` frontmatter `name` appears in the set; any names in
/// the set that are NOT matched at the end are reported as `Failed` (so
/// the caller can warn the user about typos).
pub enum InstallFilter<'a> {
    All,
    Names(&'a [String]),
    SingleName(&'a str),
}

/// Outcome of installing a list of skill directories from one plugin.
#[derive(Clone, Debug, Default, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct InstallSkillsResult {
    /// Skill names successfully installed.
    pub installed: Vec<String>,
    /// Skill names already installed and skipped (only when `force = false`).
    pub skipped: Vec<String>,
    /// Skill names whose install attempt failed (read/parse/install error,
    /// or — for `Names(_)` filter — names requested but not found).
    pub failed: Vec<FailedSkill>,
}

/// A skill that failed to install, with the reason.
#[derive(Clone, Debug, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct FailedSkill {
    pub name: String,
    pub error: String,
}

/// Outcome of installing the agents from one plugin.
///
/// Mirrors [`InstallSkillsResult`]: per-agent successes and failures are
/// collected so a single broken agent never aborts the rest of the batch,
/// and accumulated warnings always reach the caller even when some agents
/// fail.
#[derive(Clone, Debug, Default, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct InstallAgentsResult {
    /// Agent names successfully installed.
    pub installed: Vec<String>,
    /// Agent names that were already installed and left untouched.
    pub skipped: Vec<String>,
    /// Agents whose install attempt failed (parse, validation, or fs error).
    pub failed: Vec<FailedAgent>,
    /// Non-fatal issues (unmapped tools, skipped non-agent files).
    pub warnings: Vec<InstallWarning>,
}

/// An agent that failed to install, with the reason.
#[derive(Clone, Debug, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct FailedAgent {
    /// Best-known identifier — the agent name if parse reached that far,
    /// otherwise the source file path.
    pub name: String,
    pub error: String,
}

/// Non-fatal issue produced during install. Surfaced in install results
/// so the CLI / Tauri frontend can render them without blocking the install.
///
/// Carries structured reason enums (not pre-rendered strings) so consumers
/// can switch on them — the CLI formats for a human, the Tauri frontend
/// can localize or map to its own UI states.
#[derive(Clone, Debug, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[non_exhaustive]
pub enum InstallWarning {
    /// A source-declared tool had no Kiro equivalent and was dropped.
    /// The emitted agent will inherit the full parent toolset for that slot.
    UnmappedTool {
        agent: String,
        tool: String,
        reason: crate::agent::tools::UnmappedReason,
    },
    /// An agent file could not be parsed; it was skipped.
    AgentParseFailed {
        path: PathBuf,
        failure: crate::agent::ParseFailure,
    },
}

impl std::fmt::Display for InstallWarning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use crate::agent::tools::UnmappedReason;
        match self {
            InstallWarning::UnmappedTool {
                agent,
                tool,
                reason,
            } => {
                let why = match reason {
                    UnmappedReason::NoKiroEquivalent => "no Kiro equivalent",
                    UnmappedReason::BareCopilotName => "Copilot bare name; not portable",
                };
                write!(f, "agent `{agent}`: tool `{tool}` dropped ({why})")
            }
            InstallWarning::AgentParseFailed { path, failure } => {
                write!(f, "skipped agent at {}: {failure}", path.display())
            }
        }
    }
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
        use std::sync::atomic::Ordering;

        let ms = MarketplaceSource::detect(source);
        self.cache.ensure_dirs()?;

        let pending_seq = PENDING_COUNTER.fetch_add(1, Ordering::Relaxed);
        let temp_name = format!("_pending_{}_{}", std::process::id(), pending_seq);
        let temp_dir = self.cache.marketplace_path(&temp_name);

        // The unique name should make collisions impossible, but tolerate a
        // leftover dir on the off-chance of pid+seq reuse across runs.
        match fs::remove_dir_all(&temp_dir) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                warn!(
                    path = %temp_dir.display(),
                    error = %e,
                    "failed to clean up leftover temp directory"
                );
            }
        }

        let mut guard = TempDirGuard::new(temp_dir.clone());

        let link_result = self.clone_or_link(&ms, protocol, &temp_dir)?;
        let storage = storage_from_source_and_link(&ms, link_result);

        if storage == MarketplaceStorage::Copied {
            warn!(
                source = %source,
                "marketplace was copied, not linked — local changes will NOT be live-tracked"
            );
        }

        // Try to read marketplace manifest (optional).
        let manifest = Self::try_read_manifest(&temp_dir)?;

        // Scan for plugin.json files. A read failure on the repo root is
        // bubbled up as `Error::Io`, so the caller sees the real reason
        // (e.g. permission denied) rather than a misleading "no plugins".
        let discovered = crate::plugin::discover_plugins(&temp_dir, 3)?;

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

        Ok(MarketplaceAddResult {
            name,
            plugins,
            storage,
        })
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

        // Clean up the plugin registry file (best-effort). Match on the
        // operation result rather than `exists()` + `remove_file()` to avoid
        // a TOCTOU window where the file disappears between the two calls.
        let registry_path = self.cache.plugin_registry_path(name);
        match fs::remove_file(&registry_path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => warn!(
                path = %registry_path.display(),
                error = %e,
                "failed to remove plugin registry file"
            ),
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
                    result.failed.push(FailedUpdate {
                        name: entry.name.clone(),
                        error: error_full_chain(&e),
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

    /// On-disk location of a registered marketplace's contents.
    ///
    /// Exposed so Tauri/CLI handlers do not need to keep a separate
    /// `CacheDir` reference alongside the service.
    #[must_use]
    pub fn marketplace_path(&self, name: &str) -> PathBuf {
        self.cache.marketplace_path(name)
    }

    /// Resolve the canonical plugin list for a registered marketplace.
    ///
    /// Tries the persisted plugin registry first (fast path). Falls back to
    /// reading `marketplace.json` directly when the registry does not exist
    /// (e.g. marketplace was added before the registry feature) or is
    /// corrupt — a corrupt registry is logged at `warn` so users see the
    /// signal to run `update` to regenerate it.
    ///
    /// This encapsulates the registry-first-then-manifest decision so CLI
    /// and Tauri frontends do not duplicate the strategy. If we ever add a
    /// recovery path (e.g. invalidate-and-rescan on a registry version
    /// mismatch), it lives here in one place.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Marketplace`] with [`MarketplaceError::NotFound`]
    /// when neither the registry nor a `marketplace.json` exists for the
    /// given name. Other I/O or parse failures propagate.
    pub fn list_plugin_entries(
        &self,
        marketplace_name: &str,
    ) -> Result<Vec<crate::marketplace::PluginEntry>, Error> {
        match self.cache.load_plugin_registry(marketplace_name) {
            Ok(Some(entries)) => return Ok(entries),
            Ok(None) => {
                debug!(
                    marketplace = marketplace_name,
                    "no plugin registry found, falling back to marketplace manifest"
                );
            }
            Err(e) => {
                warn!(
                    marketplace = marketplace_name,
                    error = %e,
                    "plugin registry is corrupt or unreadable — falling back to manifest; \
                     run 'update' to regenerate"
                );
            }
        }

        let mp_path = self.cache.marketplace_path(marketplace_name);
        match Self::try_read_manifest(&mp_path)? {
            Some(manifest) => Ok(manifest.plugins),
            None => Err(MarketplaceError::NotFound {
                name: marketplace_name.to_owned(),
            }
            .into()),
        }
    }

    /// Install one or more skills (each represented by a SKILL.md-bearing
    /// directory) into a Kiro project under a single marketplace + plugin
    /// attribution. Centralises the SKILL.md → frontmatter → filter →
    /// `install_skill_from_dir(_force)` loop that the CLI and Tauri
    /// frontends previously duplicated.
    ///
    /// `version` is recorded in the per-skill tracking metadata.
    ///
    /// # Errors
    ///
    /// Returns `Err` only for unrecoverable per-call setup errors. Per-skill
    /// failures (read errors, parse errors, install errors, requested-but-
    /// missing names) are reported in the `failed` field of the result so
    /// the caller can render a partial-success summary.
    #[allow(clippy::too_many_arguments)]
    pub fn install_skills(
        &self,
        project: &crate::project::KiroProject,
        skill_dirs: &[PathBuf],
        filter: &InstallFilter<'_>,
        force: bool,
        marketplace: &str,
        plugin: &str,
        version: Option<&str>,
    ) -> InstallSkillsResult {
        let mut result = InstallSkillsResult::default();
        let mut processed: std::collections::HashSet<String> = std::collections::HashSet::new();

        for skill_dir in skill_dirs {
            let skill_md_path = skill_dir.join("SKILL.md");
            let content = match fs::read_to_string(&skill_md_path) {
                Ok(c) => c,
                Err(e) => {
                    warn!(
                        path = %skill_md_path.display(),
                        error = %e,
                        "failed to read SKILL.md, skipping"
                    );
                    continue;
                }
            };

            let (frontmatter, _body_offset) = match crate::skill::parse_frontmatter(&content) {
                Ok(r) => r,
                Err(e) => {
                    warn!(
                        path = %skill_md_path.display(),
                        error = %e,
                        "failed to parse SKILL.md frontmatter, skipping"
                    );
                    continue;
                }
            };

            if !filter_matches(filter, &frontmatter.name) {
                continue;
            }
            processed.insert(frontmatter.name.clone());

            let meta = crate::project::InstalledSkillMeta {
                marketplace: marketplace.to_owned(),
                plugin: plugin.to_owned(),
                version: version.map(str::to_owned),
                installed_at: chrono::Utc::now(),
            };

            let outcome = if force {
                project.install_skill_from_dir_force(&frontmatter.name, skill_dir, meta)
            } else {
                project.install_skill_from_dir(&frontmatter.name, skill_dir, meta)
            };

            match outcome {
                Ok(()) => {
                    debug!(skill = %frontmatter.name, "skill installed");
                    result.installed.push(frontmatter.name);
                }
                Err(Error::Skill(crate::error::SkillError::AlreadyInstalled { .. })) => {
                    debug!(skill = %frontmatter.name, "skill already installed, skipping");
                    result.skipped.push(frontmatter.name);
                }
                Err(e) => {
                    warn!(skill = %frontmatter.name, error = %e, "skill install failed");
                    result.failed.push(FailedSkill {
                        name: frontmatter.name,
                        error: error_full_chain(&e),
                    });
                }
            }
        }

        // For Names(_) filters, surface unmatched requests as failures so
        // typos and stale references don't become silent no-ops.
        if let InstallFilter::Names(requested) = *filter {
            for name in requested {
                if !processed.contains(name) {
                    warn!(skill = %name, plugin = %plugin, "requested skill not found in plugin");
                    result.failed.push(FailedSkill {
                        name: name.clone(),
                        error: format!("skill '{name}' not found in plugin '{plugin}'"),
                    });
                }
            }
        }

        result
    }

    /// Discover, parse, and install all agents from a plugin directory.
    ///
    /// All per-agent outcomes are collected into the returned
    /// [`InstallAgentsResult`] — a single broken agent never aborts the
    /// batch, and accumulated warnings always reach the caller. Each file
    /// is parsed exactly once; the parsed `AgentDefinition` flows straight
    /// into `project.install_agent` without re-reading the source.
    ///
    /// Returns:
    /// - `installed`: agent names the call wrote to disk.
    /// - `skipped`: agents that were already installed (left untouched).
    /// - `failed`: agents whose parse / validation / install raised an
    ///   error. The CLI surfaces these with a non-zero exit status.
    /// - `warnings`: non-fatal issues (unmapped tools, README-like files
    ///   skipped, missing-name frontmatter).
    pub fn install_plugin_agents(
        &self,
        project: &crate::project::KiroProject,
        plugin_dir: &Path,
        scan_paths: &[String],
        marketplace: &str,
        plugin: &str,
        version: Option<&str>,
    ) -> InstallAgentsResult {
        let files = crate::agent::discover::discover_agents_in_dirs(plugin_dir, scan_paths);
        let mut result = InstallAgentsResult::default();

        for path in files {
            let def = match crate::agent::parse_agent_file(&path) {
                Ok(d) => d,
                Err(crate::error::AgentError::ParseFailed {
                    path: err_path,
                    failure,
                }) => {
                    // Demote "no frontmatter at all" to debug — these are
                    // almost always human-readable docs sharing the agents
                    // directory, not broken agent files.
                    if matches!(failure, crate::agent::ParseFailure::MissingFrontmatter) {
                        debug!(path = %err_path.display(), "skipping non-agent markdown");
                    } else {
                        result.warnings.push(InstallWarning::AgentParseFailed {
                            path: err_path,
                            failure,
                        });
                    }
                    continue;
                }
                Err(e) => {
                    // Install-layer variants (AlreadyInstalled/NotInstalled)
                    // shouldn't come from parse_agent_file, but we collect
                    // them as failures rather than crashing the batch.
                    result.failed.push(FailedAgent {
                        name: path.display().to_string(),
                        error: crate::error::error_full_chain(&e),
                    });
                    continue;
                }
            };

            let (mapped, unmapped) = match def.dialect {
                crate::agent::AgentDialect::Claude => {
                    crate::agent::tools::map_claude_tools(&def.source_tools)
                }
                crate::agent::AgentDialect::Copilot => {
                    crate::agent::tools::map_copilot_tools(&def.source_tools)
                }
            };
            for u in unmapped {
                result.warnings.push(InstallWarning::UnmappedTool {
                    agent: def.name.clone(),
                    tool: u.source,
                    reason: u.reason,
                });
            }

            let meta = crate::project::InstalledAgentMeta {
                marketplace: marketplace.to_string(),
                plugin: plugin.to_string(),
                version: version.map(String::from),
                installed_at: chrono::Utc::now(),
                dialect: def.dialect,
            };
            match project.install_agent(&def, &mapped, meta) {
                Ok(()) => result.installed.push(def.name),
                Err(Error::Agent(crate::error::AgentError::AlreadyInstalled { name })) => {
                    result.skipped.push(name);
                }
                Err(e) => {
                    result.failed.push(FailedAgent {
                        name: def.name,
                        error: crate::error::error_full_chain(&e),
                    });
                }
            }
        }

        result
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
        let discovered = match crate::plugin::discover_plugins(mp_path, 3) {
            Ok(d) => d,
            Err(e) => {
                // Best-effort regeneration: an unreadable repo means we
                // cannot find new plugins, but the prior registry stays in
                // place so installs can still work against the old contents.
                warn!(
                    marketplace = %name,
                    error = %e,
                    "could not scan repo for plugins during registry regeneration"
                );
                Vec::new()
            }
        };

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
        let Some(m) = manifest else {
            return discovered
                .iter()
                .map(plugin_entry_from_discovered)
                .collect();
        };

        let mut entries = m.plugins.clone();

        // O(1) membership instead of O(n) Vec::contains so dedup against a
        // large manifest stays linear in `discovered`.
        let listed_paths: std::collections::HashSet<String> = m
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
        let listed_names: std::collections::HashSet<&str> =
            m.plugins.iter().map(|p| p.name.as_str()).collect();

        for dp in discovered {
            let dp_path = dp.relative_path_unix();
            if !listed_paths.contains(&dp_path) && !listed_names.contains(dp.name()) {
                entries.push(plugin_entry_from_discovered(dp));
            }
        }

        entries
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

/// Decide whether a skill name passes the install filter.
fn filter_matches(filter: &InstallFilter<'_>, name: &str) -> bool {
    match filter {
        InstallFilter::All => true,
        InstallFilter::SingleName(target) => name == *target,
        InstallFilter::Names(set) => set.iter().any(|n| n == name),
    }
}

/// Map the source kind + link outcome into the public `MarketplaceStorage` signal.
/// Git sources are always `Cloned` regardless of link result; local paths
/// map to `Linked` or `Copied`.
fn storage_from_source_and_link(ms: &MarketplaceSource, link: LinkResult) -> MarketplaceStorage {
    match ms {
        MarketplaceSource::GitHub { .. } | MarketplaceSource::GitUrl { .. } => {
            MarketplaceStorage::Cloned
        }
        MarketplaceSource::LocalPath { .. } => match link {
            LinkResult::Linked => MarketplaceStorage::Linked,
            LinkResult::Copied => MarketplaceStorage::Copied,
        },
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

    #[test]
    fn install_warning_unmapped_tool_renders_with_reason() {
        use crate::agent::tools::UnmappedReason;
        let w = InstallWarning::UnmappedTool {
            agent: "reviewer".into(),
            tool: "NotebookEdit".into(),
            reason: UnmappedReason::NoKiroEquivalent,
        };
        let s = w.to_string();
        assert!(s.contains("reviewer"));
        assert!(s.contains("NotebookEdit"));
        assert!(s.contains("no Kiro equivalent"));
    }

    #[test]
    fn install_warning_bare_copilot_name_reason_rendered() {
        use crate::agent::tools::UnmappedReason;
        let w = InstallWarning::UnmappedTool {
            agent: "tester".into(),
            tool: "codebase".into(),
            reason: UnmappedReason::BareCopilotName,
        };
        let s = w.to_string();
        assert!(s.contains("Copilot bare name"));
    }

    #[test]
    fn install_warning_agent_parse_failed_renders_path_and_failure() {
        use crate::agent::ParseFailure;
        let w = InstallWarning::AgentParseFailed {
            path: PathBuf::from("/tmp/bad.md"),
            failure: ParseFailure::InvalidYaml("unexpected token".into()),
        };
        let s = w.to_string();
        assert!(s.contains("/tmp/bad.md"));
        assert!(s.contains("invalid YAML"));
        assert!(s.contains("unexpected token"));
    }

    #[test]
    fn install_warning_agent_parse_failed_missing_name_renders_cleanly() {
        use crate::agent::ParseFailure;
        let w = InstallWarning::AgentParseFailed {
            path: PathBuf::from("/tmp/noname.md"),
            failure: ParseFailure::MissingName,
        };
        let s = w.to_string();
        assert!(s.contains("name"));
    }

    #[test]
    fn install_plugin_agents_emits_json_and_warnings_per_file() {
        use crate::agent::tools::UnmappedReason;
        use crate::project::KiroProject;

        let (_dir, svc) = temp_service();
        let plugin_tmp = tempfile::tempdir().unwrap();
        let agents_dir = plugin_tmp.path().join("agents");
        fs::create_dir_all(&agents_dir).unwrap();

        // Claude agent with a mappable tool and an unmapped one.
        fs::write(
            agents_dir.join("reviewer.md"),
            "---\nname: reviewer\ndescription: Reviews\ntools: [Read, NotebookEdit]\n---\nYou are a reviewer.\n",
        ).unwrap();
        // Copilot agent with a bare (unmapped) tool and an MCP ref.
        fs::write(
            agents_dir.join("tester.agent.md"),
            "---\nname: tester\ntools: ['codebase', 'terraform/*']\n---\nBody.\n",
        )
        .unwrap();
        // A README that should be silently excluded.
        fs::write(agents_dir.join("README.md"), "# agents").unwrap();

        let project_tmp = tempfile::tempdir().unwrap();
        let project = KiroProject::new(project_tmp.path().to_path_buf());

        let result = svc.install_plugin_agents(
            &project,
            plugin_tmp.path(),
            &["./agents/".to_string()],
            "mp",
            "plugin-x",
            None,
        );
        let warnings = &result.warnings;

        assert_eq!(
            result.installed.len(),
            2,
            "both agents installed, README excluded"
        );
        assert!(result.failed.is_empty(), "no failures: {:?}", result.failed);
        assert!(
            project_tmp
                .path()
                .join(".kiro/agents/reviewer.json")
                .exists()
        );
        assert!(project_tmp.path().join(".kiro/agents/tester.json").exists());
        assert!(
            project_tmp
                .path()
                .join(".kiro/agents/prompts/reviewer.md")
                .exists()
        );

        // Warnings are structured.
        let unmapped: Vec<_> = warnings
            .iter()
            .filter_map(|w| match w {
                InstallWarning::UnmappedTool { tool, reason, .. } => Some((tool.as_str(), *reason)),
                InstallWarning::AgentParseFailed { .. } => None,
            })
            .collect();
        assert!(
            unmapped.contains(&("NotebookEdit", UnmappedReason::NoKiroEquivalent)),
            "expected NotebookEdit unmapped: {unmapped:?}"
        );
        assert!(
            unmapped.contains(&("codebase", UnmappedReason::BareCopilotName)),
            "expected codebase unmapped: {unmapped:?}"
        );
        // No parse-failed warning for README (silently demoted in discover/service).
        assert!(
            !warnings
                .iter()
                .any(|w| matches!(w, InstallWarning::AgentParseFailed { .. })),
            "README should not produce a parse-failed warning"
        );
    }

    #[test]
    fn install_plugin_agents_surfaces_parse_failures_other_than_missing_fence() {
        use crate::project::KiroProject;

        let (_dir, svc) = temp_service();
        let plugin_tmp = tempfile::tempdir().unwrap();
        let agents_dir = plugin_tmp.path().join("agents");
        fs::create_dir_all(&agents_dir).unwrap();
        // Well-formed fence but YAML inside is invalid — should surface a warning.
        fs::write(
            agents_dir.join("broken.md"),
            "---\nname: [unclosed\n---\nbody\n",
        )
        .unwrap();

        let project_tmp = tempfile::tempdir().unwrap();
        let project = KiroProject::new(project_tmp.path().to_path_buf());

        let result = svc.install_plugin_agents(
            &project,
            plugin_tmp.path(),
            &["./agents/".to_string()],
            "mp",
            "p",
            None,
        );
        assert!(result.installed.is_empty());
        assert!(
            result
                .warnings
                .iter()
                .any(|w| matches!(w, InstallWarning::AgentParseFailed { .. })),
            "expected AgentParseFailed: {:?}",
            result.warnings
        );
    }

    #[test]
    fn install_plugin_agents_partial_success_preserves_warnings_and_failures() {
        use crate::project::KiroProject;

        let (_dir, svc) = temp_service();
        let plugin_tmp = tempfile::tempdir().unwrap();
        let agents_dir = plugin_tmp.path().join("agents");
        fs::create_dir_all(&agents_dir).unwrap();
        // Agent A: well-formed, will install.
        fs::write(
            agents_dir.join("a.md"),
            "---\nname: aaa\ntools: [NotebookEdit]\n---\nbody a\n",
        )
        .unwrap();
        // Agent B: pre-existing orphan file makes install fail.
        fs::write(agents_dir.join("b.md"), "---\nname: bbb\n---\nbody b\n").unwrap();

        let project_tmp = tempfile::tempdir().unwrap();
        let project = KiroProject::new(project_tmp.path().to_path_buf());
        // Pre-plant orphan file for "bbb" so its install_agent fails.
        let agents_out = project_tmp.path().join(".kiro/agents");
        fs::create_dir_all(&agents_out).unwrap();
        fs::write(agents_out.join("bbb.json"), b"{}").unwrap();

        let result = svc.install_plugin_agents(
            &project,
            plugin_tmp.path(),
            &["./agents/".to_string()],
            "mp",
            "p",
            None,
        );

        // A succeeded, B failed, and the unmapped-tool warning for A still
        // surfaces despite B's failure.
        assert_eq!(result.installed, vec!["aaa".to_string()]);
        assert_eq!(result.failed.len(), 1);
        assert_eq!(result.failed[0].name, "bbb");
        let has_unmapped = result.warnings.iter().any(|w| {
            matches!(
                w,
                InstallWarning::UnmappedTool { tool, .. } if tool == "NotebookEdit"
            )
        });
        assert!(
            has_unmapped,
            "warnings should include unmapped NotebookEdit even when a later agent fails: {:?}",
            result.warnings
        );
    }

    #[test]
    fn install_plugin_agents_already_installed_goes_to_skipped() {
        use crate::project::KiroProject;

        let (_dir, svc) = temp_service();
        let plugin_tmp = tempfile::tempdir().unwrap();
        let agents_dir = plugin_tmp.path().join("agents");
        fs::create_dir_all(&agents_dir).unwrap();
        fs::write(agents_dir.join("dup.md"), "---\nname: dup\n---\nbody\n").unwrap();

        let project_tmp = tempfile::tempdir().unwrap();
        let project = KiroProject::new(project_tmp.path().to_path_buf());

        // First install: should succeed.
        let r1 = svc.install_plugin_agents(
            &project,
            plugin_tmp.path(),
            &["./agents/".to_string()],
            "mp",
            "p",
            None,
        );
        assert_eq!(r1.installed, vec!["dup".to_string()]);
        assert!(r1.failed.is_empty());

        // Second install: should be reported as skipped, not failed.
        let r2 = svc.install_plugin_agents(
            &project,
            plugin_tmp.path(),
            &["./agents/".to_string()],
            "mp",
            "p",
            None,
        );
        assert!(r2.installed.is_empty());
        assert_eq!(r2.skipped, vec!["dup".to_string()]);
        assert!(r2.failed.is_empty(), "AlreadyInstalled must not be failed");
    }

    #[test]
    fn install_plugin_agents_rejects_frontmatter_path_traversal_end_to_end() {
        use crate::agent::ParseFailure;
        use crate::project::KiroProject;

        let (_dir, svc) = temp_service();
        let plugin_tmp = tempfile::tempdir().unwrap();
        let agents_dir = plugin_tmp.path().join("agents");
        fs::create_dir_all(&agents_dir).unwrap();
        // Attack: name in YAML attempts to escape the agents directory.
        fs::write(
            agents_dir.join("evil.md"),
            "---\nname: ../escape\n---\nbody\n",
        )
        .unwrap();

        let project_tmp = tempfile::tempdir().unwrap();
        let project = KiroProject::new(project_tmp.path().to_path_buf());

        let result = svc.install_plugin_agents(
            &project,
            plugin_tmp.path(),
            &["./agents/".to_string()],
            "mp",
            "p",
            None,
        );
        assert!(result.installed.is_empty());
        // Rejection happens at parse time with a typed InvalidName.
        let has_invalid_name = result.warnings.iter().any(|w| {
            matches!(
                w,
                InstallWarning::AgentParseFailed {
                    failure: ParseFailure::InvalidName(_),
                    ..
                }
            )
        });
        assert!(
            has_invalid_name,
            "expected InvalidName warning: {:?}",
            result.warnings
        );
        // Nothing should have been written outside project_tmp.
        assert!(
            !project_tmp.path().parent().unwrap().join("escape").exists(),
            "traversal must not have escaped project root"
        );
    }

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
        assert_eq!(
            result.storage,
            MarketplaceStorage::Cloned,
            "GitHub source must be reported as Cloned"
        );

        let known = svc.list().expect("list");
        assert_eq!(known.len(), 1);
        assert_eq!(known[0].name, "mock-market");
    }

    #[test]
    fn storage_from_source_and_link_maps_correctly() {
        // Git sources always report Cloned, regardless of link result.
        assert_eq!(
            storage_from_source_and_link(
                &MarketplaceSource::GitHub { repo: "x/y".into() },
                LinkResult::Linked
            ),
            MarketplaceStorage::Cloned
        );
        assert_eq!(
            storage_from_source_and_link(
                &MarketplaceSource::GitUrl {
                    url: "https://example.com/r.git".into()
                },
                LinkResult::Linked
            ),
            MarketplaceStorage::Cloned
        );
        // Local + true link → Linked.
        assert_eq!(
            storage_from_source_and_link(
                &MarketplaceSource::LocalPath {
                    path: "/tmp".into()
                },
                LinkResult::Linked
            ),
            MarketplaceStorage::Linked
        );
        // Local + copy fallback → Copied (so frontends can warn).
        assert_eq!(
            storage_from_source_and_link(
                &MarketplaceSource::LocalPath {
                    path: "/tmp".into()
                },
                LinkResult::Copied
            ),
            MarketplaceStorage::Copied
        );
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
    fn list_plugin_entries_reads_persisted_registry() {
        let (_dir, svc) = temp_service();
        svc.add("owner/repo", GitProtocol::Https).expect("add");

        let entries = svc
            .list_plugin_entries("mock-market")
            .expect("registry path should succeed");

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "mock-plugin");
    }

    #[test]
    fn list_plugin_entries_falls_back_to_manifest_when_registry_missing() {
        // Add a marketplace, then delete the persisted plugin-registry file
        // so list_plugin_entries must fall back to reading marketplace.json.
        let (dir, svc) = temp_service();
        svc.add("owner/repo", GitProtocol::Https).expect("add");

        let cache = CacheDir::with_root(dir.path().to_path_buf());
        let registry_path = cache.plugin_registry_path("mock-market");
        fs::remove_file(&registry_path).expect("remove registry");
        assert!(!registry_path.exists());

        let entries = svc
            .list_plugin_entries("mock-market")
            .expect("manifest fallback should succeed");

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "mock-plugin");
    }

    #[test]
    fn list_plugin_entries_returns_not_found_for_unknown_marketplace() {
        let (_dir, svc) = temp_service();

        let err = svc
            .list_plugin_entries("does-not-exist")
            .expect_err("unknown marketplace must error, not return empty");

        assert!(
            matches!(err, Error::Marketplace(MarketplaceError::NotFound { .. })),
            "expected NotFound, got {err:?}"
        );
    }

    #[test]
    fn marketplace_path_returns_cache_path() {
        let (dir, svc) = temp_service();
        let p = svc.marketplace_path("acme");
        assert!(p.starts_with(dir.path()));
        assert!(
            p.ends_with("acme"),
            "should end with marketplace name, got {}",
            p.display()
        );
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

    #[test]
    fn add_empty_repo_returns_no_plugins_found_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cache = CacheDir::with_root(dir.path().to_path_buf());
        cache.ensure_dirs().expect("ensure_dirs");

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
        let discovered = discover_plugins(root, 3).expect("discover should succeed");

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
        let discovered = discover_plugins(root, 3).expect("discover should succeed");

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

        let discovered = discover_plugins(root, 3).expect("discover should succeed");
        let entries = MarketplaceService::build_registry_entries(None, &discovered);

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "solo");
    }
}

//! Marketplace lifecycle operations.
//!
//! [`MarketplaceService`] centralizes add/remove/update/list logic so that
//! CLI and Tauri frontends remain thin presentation wrappers.

pub mod browse;

pub use browse::{
    BulkSkillsResult, PluginSkillsResult, SkillInfo, SkippedPlugin, SkippedReason, SkippedSkill,
    SkippedSkillReason,
};

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicU64;

use serde::Serialize;
use tracing::{debug, warn};

use crate::cache::{CacheDir, KnownMarketplace, MarketplaceSource};
use crate::error::{Error, MarketplaceError, PluginError, error_full_chain};
use crate::git::{self, CloneOptions, GitBackend, GitProtocol, GitRef};
use crate::marketplace::{Marketplace, PluginEntry, PluginSource, StructuredSource};
use crate::platform::LinkResult;
use crate::{platform, validation};

/// Process-local sequence used to disambiguate concurrent `_pending_*` temp
/// directories during `add()`. Combined with `process::id()` so two threads
/// in the same process never collide on the staging path.
static PENDING_COUNTER: AtomicU64 = AtomicU64::new(0);

// ---------------------------------------------------------------------------
// Temp directory cleanup guard
// ---------------------------------------------------------------------------

// `TempDirGuard` was extracted into the shared `crate::raii::DirCleanupGuard`
// — same shape, same Drop semantics, same retarget+defuse API. The
// platform.rs Windows StagingGuard now uses the same primitive, so
// future fixes to cleanup ordering or warn severity apply once.

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

/// Whether `http://` marketplace URLs are permitted. Replaces a `bool`
/// field that read identically at struct-literal call sites and could
/// be silently flipped by a typo (`allow_insecure_http: true` looks no
/// different from `allow_insecure_http: false` in a code review). The
/// enum variants name the security posture explicitly.
///
/// `#[non_exhaustive]` so a future tightening (e.g. `AllowOnLocalhost`)
/// is an additive change.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum InsecureHttpPolicy {
    /// Refuse `http://` URLs. The strict default — plaintext HTTP is
    /// unauthenticated and a network attacker can substitute the entire
    /// marketplace contents, gaining persistent code execution via
    /// skills/agents/MCP servers that the cache then keeps around.
    #[default]
    Reject,
    /// Allow `http://` URLs. Only flip this when TLS truly isn't
    /// available on the source's network — the resulting marketplace
    /// install is trust-on-first-use against any MITM during the clone
    /// window.
    Allow,
}

/// Options controlling [`MarketplaceService::add`].
///
/// `#[non_exhaustive]` so adding future fields (`require_sha`,
/// `allow_self_signed_tls`, …) is an additive change. External callers
/// must therefore use the builder methods rather than struct-expression
/// construction:
///
/// ```ignore
/// MarketplaceAddOptions::new(GitProtocol::Https)
///     .allow_insecure_http()
/// ```
///
/// The `From<GitProtocol>` impl preserves the convenience of passing a
/// bare protocol — `svc.add(source, GitProtocol::Https)` still compiles
/// against the strict defaults.
#[derive(Clone, Copy, Debug, Default)]
#[non_exhaustive]
pub struct MarketplaceAddOptions {
    /// Git protocol used for GitHub `owner/repo` shorthand sources.
    pub protocol: GitProtocol,
    /// Policy for plaintext `http://` source URLs. See
    /// [`InsecureHttpPolicy`] for the per-variant rationale.
    pub insecure_http: InsecureHttpPolicy,
}

impl MarketplaceAddOptions {
    /// Construct an options bag with the given git protocol and the
    /// strict default for every other field. Builder methods follow.
    #[must_use]
    pub fn new(protocol: GitProtocol) -> Self {
        Self {
            protocol,
            insecure_http: InsecureHttpPolicy::Reject,
        }
    }

    /// Set the [`InsecureHttpPolicy`] explicitly. Useful when the
    /// caller has the policy as a value already (e.g. mapped from a
    /// CLI bool flag).
    #[must_use]
    pub fn with_insecure_http(mut self, policy: InsecureHttpPolicy) -> Self {
        self.insecure_http = policy;
        self
    }

    /// Shorthand for `with_insecure_http(InsecureHttpPolicy::Allow)`.
    /// Reads naturally at call sites that decide statically to opt in.
    #[must_use]
    pub fn allow_insecure_http(self) -> Self {
        self.with_insecure_http(InsecureHttpPolicy::Allow)
    }
}

impl From<GitProtocol> for MarketplaceAddOptions {
    /// Convenience for callers that only need to choose a protocol and
    /// accept the strict defaults for everything else.
    fn from(protocol: GitProtocol) -> Self {
        Self::new(protocol)
    }
}

/// Whether an install should overwrite existing entries of the same name.
///
/// Used by [`MarketplaceService::install_skills`] and
/// [`MarketplaceService::install_plugin_agents`] to replace the earlier
/// `force: bool` parameter. Named variants prevent boolean-blindness at
/// the call site and leave room for future modes (e.g. `DryRun`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InstallMode {
    /// Default: existing installs are preserved and reported as skipped.
    New,
    /// Overwrite any existing install of the same name.
    Force,
}

impl InstallMode {
    /// Returns `true` when the mode is [`InstallMode::Force`].
    #[must_use]
    pub fn is_force(self) -> bool {
        matches!(self, Self::Force)
    }
}

impl From<bool> for InstallMode {
    /// Convenience conversion for CLIs that parse a `--force` boolean flag.
    /// `true` → `Force`, `false` → `New`.
    fn from(force: bool) -> Self {
        if force { Self::Force } else { Self::New }
    }
}

/// Outcome of installing a list of skill directories from one plugin.
#[derive(Clone, Debug, Default, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct InstallSkillsResult {
    /// Skill names successfully installed.
    pub installed: Vec<String>,
    /// Skill names already installed and skipped (only when `force = false`).
    pub skipped: Vec<String>,
    /// Skill names whose install attempt failed (install error, or — for
    /// `Names(_)` filter — names requested but not found). Distinct from
    /// [`Self::skipped_skills`], which tracks entries we couldn't even
    /// read / parse before attempting to install them.
    pub failed: Vec<FailedSkill>,
    /// Skill-source entries that could not be read or parsed, so no
    /// install was attempted. Surfaces what previously vanished into
    /// `warn!`-then-`continue`; mirrors
    /// [`crate::service::browse::BulkSkillsResult::skipped_skills`].
    pub skipped_skills: Vec<browse::SkippedSkill>,
}

/// A skill that failed to install, with the reason.
///
/// `error` is the human-readable Display (suitable for log lines or UI
/// direct rendering); `kind` is the stable programmatic contract that
/// frontends should `match` on when deciding how to render the failure.
/// The two are deliberately redundant — `error` can rephrase freely over
/// time, while `kind` stays stable.
///
/// Fields are `pub(crate)` so external callers cannot desync the two —
/// construction routes exclusively through [`Self::install_failed`] and
/// [`Self::requested_but_not_found`], each of which derives `error` and
/// `kind` together from a single source. Read access from outside the
/// crate happens via the [`Self::name`] / [`Self::error`] / [`Self::kind`]
/// accessors, and via the Serde/specta boundary (the generated
/// TypeScript type still exposes all three fields, because Serde ignores
/// Rust visibility). This mirrors the [`crate::service::browse::SkippedPlugin`]
/// enforcement pattern so the two redundant-by-design types stay
/// symmetric.
#[derive(Clone, Debug, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct FailedSkill {
    pub(crate) name: String,
    pub(crate) error: String,
    pub(crate) kind: FailedSkillReason,
}

impl FailedSkill {
    /// Build a [`FailedSkill`] for an install-time failure (filesystem
    /// copy error, tracking-file write error, etc.). Derives the
    /// human-readable `error` from [`crate::error::error_full_chain`]
    /// and sets `kind = InstallFailed` in lockstep.
    ///
    /// This is one of exactly two constructors (the other is
    /// [`Self::requested_but_not_found`]); fields being `pub(crate)`
    /// guarantees no external caller can produce a `FailedSkill` with
    /// a mismatched `kind` and `error`.
    #[must_use]
    pub(crate) fn install_failed(name: String, err: &Error) -> Self {
        Self {
            name,
            error: error_full_chain(err),
            kind: FailedSkillReason::InstallFailed,
        }
    }

    /// Build a [`FailedSkill`] for a `Names(_)` filter miss — the
    /// caller asked for a skill name that no discovered `SKILL.md`
    /// produced. Composes the user-facing error string and pins
    /// `kind = RequestedButNotFound { plugin }` so the frontend can
    /// render a typo banner distinct from an install error.
    #[must_use]
    pub(crate) fn requested_but_not_found(name: String, plugin: String) -> Self {
        Self {
            error: format!("skill '{name}' not found in plugin '{plugin}'"),
            name,
            kind: FailedSkillReason::RequestedButNotFound { plugin },
        }
    }

    /// Name of the skill that failed. The equivalent read via the
    /// Serde-generated TypeScript type is `FailedSkill.name`.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Human-readable failure message (rendered source-error chain or
    /// a user-facing composition, depending on `kind`). Use
    /// [`Self::kind`] for programmatic matching; use this for log
    /// lines and simple UI labels.
    #[must_use]
    pub fn error(&self) -> &str {
        &self.error
    }

    /// Structured classification of the failure. Stable contract for
    /// frontends that render variant-specific affordances.
    #[must_use]
    pub fn kind(&self) -> &FailedSkillReason {
        &self.kind
    }
}

/// Why a skill install failed. Separates "we tried to install and it
/// went wrong" ([`Self::InstallFailed`]) from "the caller named a skill
/// that isn't in this plugin" ([`Self::RequestedButNotFound`]) so
/// frontends can render a typo banner distinct from an install error
/// without substring-matching `FailedSkill::error`.
///
/// [`Self::InstallFailed`] is unit-shaped: the human-readable error
/// message lives on [`FailedSkill::error`] and the typed `kind` here
/// exists to tell the typo case apart from every other failure mode.
/// Duplicating the error string into this variant would be redundant
/// since `FailedSkill.error` is always populated.
#[derive(Clone, Debug, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum FailedSkillReason {
    /// The install was attempted (`SKILL.md` read and frontmatter
    /// parsed) but the filesystem copy / metadata write failed. See
    /// [`FailedSkill::error`] for the human-readable reason.
    InstallFailed,
    /// The caller's `Names(_)` filter included a name that no skill in
    /// the plugin's discovered list produced — typically a typo or a
    /// stale reference. The `plugin` field carries the plugin context
    /// so a flat UI list can attribute the miss.
    RequestedButNotFound { plugin: String },
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
    /// An agent declares MCP servers but the install was not opted in
    /// to MCP. The agent was skipped — its prompt would otherwise
    /// install with a `mcpServers` block that runs subprocesses or
    /// opens network connections without the user's explicit consent.
    /// Listed transports help the user see the risk surface (e.g.
    /// `["stdio", "stdio", "http"]`) before they re-run with the
    /// `--accept-mcp` opt-in.
    McpServersRequireOptIn {
        agent: String,
        transports: Vec<String>,
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
            InstallWarning::McpServersRequireOptIn { agent, transports } => {
                write!(
                    f,
                    "agent `{agent}` brings {} MCP server{} ({}); skipped — re-run with `--accept-mcp` to install",
                    transports.len(),
                    if transports.len() == 1 { "" } else { "s" },
                    transports.join(", ")
                )
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
    pub fn add(
        &self,
        source: &str,
        opts: impl Into<MarketplaceAddOptions>,
    ) -> Result<MarketplaceAddResult, Error> {
        use std::sync::atomic::Ordering;

        let opts = opts.into();
        let protocol = opts.protocol;

        let ms = MarketplaceSource::detect(source);

        // Refuse plaintext HTTP unless the caller's policy allows it.
        // Matching against the raw source string (not the parsed
        // MarketplaceSource::GitUrl) is enough because GitHub shorthands
        // and local paths can never carry an http scheme. The
        // remediation message names the caller-facing knob.
        if matches!(opts.insecure_http, InsecureHttpPolicy::Reject)
            && let MarketplaceSource::GitUrl { url } = &ms
            && url.starts_with("http://")
        {
            return Err(MarketplaceError::InsecureSource { url: url.clone() }.into());
        }

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

        let mut guard =
            crate::raii::DirCleanupGuard::new(temp_dir.clone(), "marketplace temp directory");

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
        let discovered =
            crate::plugin::discover_plugins(&temp_dir, crate::plugin::DEFAULT_DISCOVERY_MAX_DEPTH)?;

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

        // Take the registry lock once for the whole "claim the name +
        // rename + register" sequence. Without this single lock, two
        // concurrent `add` calls for the same name could both pass the
        // `final_dir.exists()` check, then race the rename (one wins, one
        // fails with a confusing IO error or — worse on some platforms —
        // both succeed and clobber each other's content).
        //
        // `register_known_marketplace_unlocked` is the inner counterpart
        // to `add_known_marketplace` that assumes the caller already holds
        // the lock. Calling the locking variant here would self-contend
        // — the second acquire opens a fresh fd whose `try_lock_exclusive`
        // can't succeed until the outer fd is dropped, so the polling
        // loop in `with_file_lock` would stall for `LOCK_TIMEOUT` (10s)
        // and surface `ErrorKind::TimedOut`.
        let entry = KnownMarketplace {
            name: name.clone(),
            source: ms,
            protocol: Some(protocol),
            added_at: chrono::Utc::now(),
        };

        crate::file_lock::with_file_lock(&self.cache.registry_path(), || -> Result<(), Error> {
            if final_dir.exists() {
                return Err(MarketplaceError::AlreadyRegistered { name: name.clone() }.into());
            }

            fs::rename(&temp_dir, &final_dir)?;
            // The temp dir no longer exists under its old name; from
            // here on, any cleanup-on-failure must target `final_dir`.
            guard.retarget(final_dir.clone());

            if let Err(e) = self.cache.register_known_marketplace_unlocked(entry) {
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
                // Defuse so the guard doesn't try to remove what we
                // already attempted to remove (or log a spurious
                // warning if the rollback succeeded).
                guard.defuse();
                return Err(e);
            }

            // Persist the merged plugin list INSIDE the registry
            // lock. Outside it, a concurrent `remove(name)` could
            // complete (deleting plugin_registry_path) between our
            // register call and this write, leaving an orphaned
            // `registries/<name>.json` for an unregistered
            // marketplace. Holding the lock makes the
            // register-and-write sequence atomic from the
            // marketplace registry's perspective. A write failure is
            // still a soft fail — the user can re-run `update <name>`
            // to regenerate — so we warn rather than roll back the
            // marketplace registration.
            if let Err(e) = self.cache.write_plugin_registry(&name, &registry_entries) {
                warn!(
                    marketplace = %name,
                    error = %e,
                    "failed to write plugin registry — run 'update {name}' to regenerate"
                );
            }

            guard.defuse();
            Ok(())
        })?;

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
        mode: InstallMode,
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
                    result.skipped_skills.push(browse::SkippedSkill {
                        plugin: plugin.to_owned(),
                        name_hint: browse::name_hint_from_skill_dir(skill_dir),
                        path: skill_md_path,
                        reason: browse::SkippedSkillReason::ReadFailed {
                            reason: e.to_string(),
                        },
                    });
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
                    result.skipped_skills.push(browse::SkippedSkill {
                        plugin: plugin.to_owned(),
                        name_hint: browse::name_hint_from_skill_dir(skill_dir),
                        path: skill_md_path,
                        reason: browse::SkippedSkillReason::FrontmatterInvalid {
                            reason: e.to_string(),
                        },
                    });
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

            let outcome = if mode.is_force() {
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
                    warn!(
                        skill = %frontmatter.name,
                        error = %error_full_chain(&e),
                        "skill install failed"
                    );
                    result
                        .failed
                        .push(FailedSkill::install_failed(frontmatter.name, &e));
                }
            }
        }

        // For Names(_) filters, surface unmatched requests as failures so
        // typos and stale references don't become silent no-ops.
        if let InstallFilter::Names(requested) = *filter {
            for name in requested {
                if !processed.contains(name) {
                    warn!(skill = %name, plugin = %plugin, "requested skill not found in plugin");
                    result.failed.push(FailedSkill::requested_but_not_found(
                        name.clone(),
                        plugin.to_owned(),
                    ));
                }
            }
        }

        result
    }

    /// Resolve a plugin's on-disk location from its marketplace entry.
    ///
    /// For `PluginSource::RelativePath`, validates the path and verifies the
    /// directory exists inside the marketplace tree. For `PluginSource::Structured`,
    /// ensures the plugin's cache directory exists (cloning if necessary),
    /// optionally verifies a pinned SHA, and returns the final path —
    /// possibly a sub-directory for `git-subdir` sources.
    ///
    /// Shared between frontends (CLI today, Tauri in the future) so the
    /// `RelativePath` + `Structured` resolution flow isn't duplicated
    /// per-frontend, matching the project's "domain logic is never duplicated
    /// between frontends" rule.
    ///
    /// # Errors
    ///
    /// - [`Error::Validation`] for malformed relative paths or git refs.
    /// - [`Error::Plugin`] ([`PluginError::DirectoryMissing`]) if a
    ///   `RelativePath` source points to a missing directory.
    /// - [`Error::Git`] for clone or SHA-verification failures.
    pub fn resolve_plugin_dir(
        &self,
        entry: &PluginEntry,
        marketplace_path: &Path,
        marketplace_name: &str,
        protocol: GitProtocol,
    ) -> Result<PathBuf, Error> {
        match &entry.source {
            PluginSource::RelativePath(rel) => {
                // `rel` is a validated `RelativePath` — no traversal check
                // needed; construction through `RelativePath::new` is the
                // only way to obtain one, and it validates.
                let resolved = marketplace_path.join(rel);
                // Use symlink_metadata (does NOT follow symlinks) so a
                // malicious marketplace cannot point `rel` at a symlink
                // that resolves outside the marketplace tree. Matches the
                // symlink-refuse policy in project::copy_dir_recursive,
                // agent::discover_agents_in_dirs, and load_plugin_manifest.
                let is_real_dir = fs::symlink_metadata(&resolved).is_ok_and(|m| m.is_dir());
                if !is_real_dir {
                    return Err(PluginError::DirectoryMissing { path: resolved }.into());
                }
                Ok(resolved)
            }
            PluginSource::Structured(structured) => {
                self.resolve_structured_source(structured, marketplace_name, &entry.name, protocol)
            }
        }
    }

    /// Clone a structured source into the cache plugins directory (if not
    /// already present) and return the resolved path. Used by
    /// [`resolve_plugin_dir`].
    fn resolve_structured_source(
        &self,
        source: &StructuredSource,
        marketplace_name: &str,
        plugin_name: &str,
        protocol: GitProtocol,
    ) -> Result<PathBuf, Error> {
        self.cache.ensure_dirs()?;

        let dest = self.cache.plugin_path(marketplace_name, plugin_name);

        // Extract the varying parts from each source variant.
        let (url, subdir, git_ref, sha, label) = match source {
            StructuredSource::GitHub { repo, git_ref, sha } => (
                git::github_repo_to_url(repo, protocol),
                None,
                git_ref.as_deref(),
                sha.as_deref(),
                repo.clone(),
            ),
            StructuredSource::GitUrl { url, git_ref, sha } => (
                url.clone(),
                None,
                git_ref.as_deref(),
                sha.as_deref(),
                url.clone(),
            ),
            StructuredSource::GitSubdir {
                url,
                path,
                git_ref,
                sha,
            } => (
                url.clone(),
                Some(path.as_str()),
                git_ref.as_deref(),
                sha.as_deref(),
                url.clone(),
            ),
        };

        // No re-validation needed: `path` is typed as `RelativePath`, which
        // cannot hold an unvalidated string. Serde and programmatic callers
        // both go through `RelativePath::new`.

        // Serialize concurrent callers on the same cache path. Without this,
        // two processes racing on `kiro-market install foo@bar` for a
        // not-yet-cached plugin would both see `!dest.exists()` and both
        // attempt `clone_repo`, one clobbering the other. The lock also
        // lets us recover from a partially-cloned directory left behind by
        // a prior interrupted attempt (detected via missing `.git/`).
        crate::file_lock::with_file_lock(&dest, || -> Result<PathBuf, Error> {
            if dest.exists() {
                // A complete clone leaves `.git/` behind. Its absence means
                // the directory is partial (prior crash, interrupted clone)
                // and must be removed before a retry can succeed.
                if dest.join(".git").exists() {
                    debug!(dest = %dest.display(), "plugin already cached, reusing");
                    if let Some(expected) = sha {
                        self.git.verify_sha(&dest, expected)?;
                    }
                    return Ok(match subdir {
                        Some(path) => dest.join(path),
                        None => dest.clone(),
                    });
                }
                warn!(
                    dest = %dest.display(),
                    "removing partial plugin clone from prior interrupted attempt"
                );
                fs::remove_dir_all(&dest)?;
            }

            debug!(url = %url, dest = %dest.display(), label = %label, "cloning plugin");
            let validated_ref = git_ref.map(GitRef::new).transpose()?;
            let opts = CloneOptions {
                git_ref: validated_ref,
            };
            self.git.clone_repo(&url, &dest, &opts)?;

            if let Some(expected) = sha {
                self.git.verify_sha(&dest, expected)?;
            }

            Ok(match subdir {
                Some(path) => dest.join(path),
                None => dest.clone(),
            })
        })
    }

    /// Discover, parse, and install all agents from a plugin directory.
    ///
    /// All per-agent outcomes are collected into the returned
    /// [`InstallAgentsResult`] — a single broken agent never aborts the
    /// batch, and accumulated warnings always reach the caller. Each file
    /// is parsed exactly once; the parsed `AgentDefinition` flows straight
    /// into `project.install_agent` without re-reading the source.
    ///
    /// When `force` is `true`, existing agents of the same name are
    /// overwritten (mirrors the CLI `--force` flag for skills). When
    /// `false`, already-installed agents are left untouched and recorded
    /// in `skipped`.
    ///
    /// Returns:
    /// - `installed`: agent names the call wrote to disk.
    /// - `skipped`: agents that were already installed (left untouched).
    /// - `failed`: agents whose parse / validation / install raised an
    ///   error. The CLI surfaces these with a non-zero exit status.
    /// - `warnings`: non-fatal issues (unmapped tools, README-like files
    ///   skipped, missing-name frontmatter).
    #[allow(clippy::too_many_arguments)] // eight non-self params, each a distinct
    // input from upstream: project + plugin_dir + scan_paths + mode +
    // accept_mcp + marketplace + plugin + version. A builder would add
    // indirection without reducing arity at the call site.
    pub fn install_plugin_agents(
        &self,
        project: &crate::project::KiroProject,
        plugin_dir: &Path,
        scan_paths: &[String],
        mode: InstallMode,
        accept_mcp: bool,
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

            // MCP opt-in gate. An agent that brings MCP servers can run
            // arbitrary subprocesses (Stdio) or open network connections
            // (Http/Sse) on the user's host. The cache persists, so a
            // one-time install is a long-lived foothold. Default policy:
            // skip + warn so the user sees the risk surface; re-running
            // with `--accept-mcp` flips the gate.
            if !accept_mcp && !def.mcp_servers.is_empty() {
                let transports: Vec<String> = def
                    .mcp_servers
                    .values()
                    .map(|cfg| cfg.transport_label().to_owned())
                    .collect();
                result
                    .warnings
                    .push(InstallWarning::McpServersRequireOptIn {
                        agent: def.name.clone(),
                        transports,
                    });
                continue;
            }

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
            let install_result = if mode.is_force() {
                project.install_agent_force(&def, &mapped, meta)
            } else {
                project.install_agent(&def, &mapped, meta)
            };
            match install_result {
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
        let discovered = match crate::plugin::discover_plugins(
            mp_path,
            crate::plugin::DEFAULT_DISCOVERY_MAX_DEPTH,
        ) {
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
                    rel.as_str()
                        .trim_start_matches("./")
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
    // `as_relative_path_string` always returns `./<unix-path>` and the
    // components originate from our own filesystem scan, so validation
    // cannot legitimately fail here. `expect` documents the invariant:
    // if it ever panics, discovery has admitted an unsafe name that it
    // should have rejected upstream.
    let rel = crate::validation::RelativePath::new(dp.as_relative_path_string())
        .expect("discovered plugin paths are always valid relative paths");
    crate::marketplace::PluginEntry {
        name: dp.name().to_owned(),
        description: dp.description().map(String::from),
        source: crate::marketplace::PluginSource::RelativePath(rel),
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
            InstallMode::New,
            false, // accept_mcp: existing fixtures don't carry MCP servers
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
                InstallWarning::AgentParseFailed { .. }
                | InstallWarning::McpServersRequireOptIn { .. } => None,
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
            InstallMode::New,
            false, // accept_mcp: existing fixtures don't carry MCP servers
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
            InstallMode::New,
            false, // accept_mcp: existing fixtures don't carry MCP servers
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
    fn install_plugin_agents_demotes_missing_fence_for_non_readme_files() {
        // Coverage for the service-layer demotion path: a plain `.md` file
        // (not README/CONTRIBUTING/CHANGELOG) with no frontmatter fence
        // must not surface as a warning — it should be debug-logged and
        // dropped. Previously only the README exclusion in `discover`
        // was tested, which short-circuited this path.
        use crate::project::KiroProject;

        let (_dir, svc) = temp_service();
        let plugin_tmp = tempfile::tempdir().unwrap();
        let agents_dir = plugin_tmp.path().join("agents");
        fs::create_dir_all(&agents_dir).unwrap();
        // A non-excluded filename that lacks frontmatter entirely.
        fs::write(
            agents_dir.join("notes.md"),
            "# just notes, no frontmatter\n",
        )
        .unwrap();

        let project_tmp = tempfile::tempdir().unwrap();
        let project = KiroProject::new(project_tmp.path().to_path_buf());

        let result = svc.install_plugin_agents(
            &project,
            plugin_tmp.path(),
            &["./agents/".to_string()],
            InstallMode::New,
            false, // accept_mcp: existing fixtures don't carry MCP servers
            "mp",
            "p",
            None,
        );
        assert!(result.installed.is_empty());
        assert!(result.failed.is_empty());
        assert!(
            !result
                .warnings
                .iter()
                .any(|w| matches!(w, InstallWarning::AgentParseFailed { .. })),
            "missing-fence non-README file must be demoted silently, got: {:?}",
            result.warnings
        );
    }

    #[test]
    fn install_plugin_agents_skips_mcp_agents_without_opt_in() {
        // An agent declaring an MCP server must NOT be installed when
        // accept_mcp is false. Default safety: a passing-by user shouldn't
        // accidentally accept arbitrary subprocess execution. The skip
        // surfaces as a McpServersRequireOptIn warning so the user can
        // see what got skipped and how to opt in.
        use crate::project::KiroProject;

        let (_dir, svc) = temp_service();
        let plugin_tmp = tempfile::tempdir().unwrap();
        let agents_dir = plugin_tmp.path().join("agents");
        fs::create_dir_all(&agents_dir).unwrap();

        // Copilot-style .agent.md with one stdio MCP entry.
        fs::write(
            agents_dir.join("terraformer.agent.md"),
            "---\nname: terraformer\ndescription: TF\nmcp-servers:\n  tf:\n    type: 'local'\n    command: 'docker'\n    args: ['run', '-i']\n---\nbody\n",
        )
        .unwrap();

        let project_tmp = tempfile::tempdir().unwrap();
        let project = KiroProject::new(project_tmp.path().to_path_buf());

        let result = svc.install_plugin_agents(
            &project,
            plugin_tmp.path(),
            &["./agents/".to_string()],
            InstallMode::New,
            false, // accept_mcp = false → gate must fire
            "mp",
            "p",
            None,
        );

        assert!(
            result.installed.is_empty(),
            "MCP agent must not be installed without opt-in: {:?}",
            result.installed
        );
        assert!(
            result.warnings.iter().any(
                |w| matches!(w, InstallWarning::McpServersRequireOptIn { agent, transports }
                    if agent == "terraformer" && transports == &vec!["stdio".to_string()])
            ),
            "expected McpServersRequireOptIn warning naming the agent and stdio transport, got {:?}",
            result.warnings
        );
        // No JSON written for the skipped agent.
        assert!(
            !project_tmp
                .path()
                .join(".kiro/agents/terraformer.json")
                .exists()
        );
    }

    #[test]
    fn install_plugin_agents_installs_mcp_agents_when_opted_in() {
        // accept_mcp = true unlocks installation, including the
        // mcpServers block in the emitted JSON.
        use crate::project::KiroProject;

        let (_dir, svc) = temp_service();
        let plugin_tmp = tempfile::tempdir().unwrap();
        let agents_dir = plugin_tmp.path().join("agents");
        fs::create_dir_all(&agents_dir).unwrap();
        fs::write(
            agents_dir.join("terraformer.agent.md"),
            "---\nname: terraformer\ndescription: TF\nmcp-servers:\n  tf:\n    type: 'local'\n    command: 'docker'\n    args: ['run', '-i']\n---\nbody\n",
        )
        .unwrap();

        let project_tmp = tempfile::tempdir().unwrap();
        let project = KiroProject::new(project_tmp.path().to_path_buf());

        let result = svc.install_plugin_agents(
            &project,
            plugin_tmp.path(),
            &["./agents/".to_string()],
            InstallMode::New,
            true, // accept_mcp = true → gate is bypassed
            "mp",
            "p",
            None,
        );

        assert_eq!(result.installed, vec!["terraformer".to_string()]);
        assert!(
            !result
                .warnings
                .iter()
                .any(|w| matches!(w, InstallWarning::McpServersRequireOptIn { .. })),
            "no MCP-opt-in warning when opted in: {:?}",
            result.warnings
        );

        // The emitted JSON contains the typed-and-normalized mcpServers block:
        // `type: 'local'` came in via the Copilot alias and is emitted as `stdio`.
        let json_path = project_tmp.path().join(".kiro/agents/terraformer.json");
        let json: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&json_path).unwrap()).unwrap();
        assert_eq!(json["mcpServers"]["tf"]["type"], "stdio");
        assert_eq!(json["mcpServers"]["tf"]["command"], "docker");
    }

    #[test]
    fn install_plugin_agents_lists_all_mcp_transports_in_warning() {
        // An agent with multiple MCP servers of different transports
        // should surface ALL of them in the warning's `transports` vec.
        // A regression where only one transport gates the install (or
        // only one is reported) would leave the user blind to part of
        // the risk surface.
        use crate::project::KiroProject;

        let (_dir, svc) = temp_service();
        let plugin_tmp = tempfile::tempdir().unwrap();
        let agents_dir = plugin_tmp.path().join("agents");
        fs::create_dir_all(&agents_dir).unwrap();
        fs::write(
            agents_dir.join("multi.agent.md"),
            "---\n\
             name: multi\n\
             description: \"Multiple MCP transports\"\n\
             mcp-servers:\n  \
               local_tool:\n    \
                 type: 'local'\n    \
                 command: 'docker'\n  \
               http_tool:\n    \
                 type: http\n    \
                 url: https://mcp.example.com\n  \
               another_local:\n    \
                 type: stdio\n    \
                 command: 'node'\n\
             ---\n\
             body\n",
        )
        .unwrap();

        let project_tmp = tempfile::tempdir().unwrap();
        let project = KiroProject::new(project_tmp.path().to_path_buf());

        let result = svc.install_plugin_agents(
            &project,
            plugin_tmp.path(),
            &["./agents/".to_string()],
            InstallMode::New,
            false,
            "mp",
            "p",
            None,
        );
        assert!(result.installed.is_empty(), "MCP agent must be skipped");

        let mcp_warning = result
            .warnings
            .iter()
            .find_map(|w| match w {
                InstallWarning::McpServersRequireOptIn { agent, transports } => {
                    Some((agent, transports))
                }
                _ => None,
            })
            .expect("expected McpServersRequireOptIn warning");
        assert_eq!(mcp_warning.0, "multi");
        // Transports come out in BTreeMap iteration order on the agent's
        // `mcp_servers` keys (alphabetical): another_local, http_tool, local_tool.
        assert_eq!(
            mcp_warning.1,
            &vec!["stdio".to_string(), "http".to_string(), "stdio".to_string()],
            "all transports must appear in the warning so the user sees the full risk surface"
        );
    }

    #[test]
    fn install_plugin_agents_force_does_not_bypass_mcp_gate() {
        // Regression guard: a future change that wires `mode == Force`
        // to skip the MCP opt-in check would silently install
        // subprocess-spawning agents. The gate must fire even under
        // --force; --accept-mcp is the only opt-in.
        use crate::project::KiroProject;

        let (_dir, svc) = temp_service();
        let plugin_tmp = tempfile::tempdir().unwrap();
        let agents_dir = plugin_tmp.path().join("agents");
        fs::create_dir_all(&agents_dir).unwrap();
        fs::write(
            agents_dir.join("force-test.agent.md"),
            "---\nname: forcetest\nmcp-servers:\n  s:\n    type: 'local'\n    command: 'docker'\n---\nbody\n",
        )
        .unwrap();

        let project_tmp = tempfile::tempdir().unwrap();
        let project = KiroProject::new(project_tmp.path().to_path_buf());

        let result = svc.install_plugin_agents(
            &project,
            plugin_tmp.path(),
            &["./agents/".to_string()],
            InstallMode::Force, // <-- force, but...
            false,              // <-- accept_mcp = false should still gate
            "mp",
            "p",
            None,
        );
        assert!(
            result.installed.is_empty(),
            "force MUST NOT bypass the MCP opt-in: {:?}",
            result.installed
        );
        assert!(
            result
                .warnings
                .iter()
                .any(|w| matches!(w, InstallWarning::McpServersRequireOptIn { .. })),
            "MCP warning still expected under force when accept_mcp is false"
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
            InstallMode::New,
            false, // accept_mcp: existing fixtures don't carry MCP servers
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
            InstallMode::New,
            false, // accept_mcp: existing fixtures don't carry MCP servers
            "mp",
            "p",
            None,
        );
        assert!(r2.installed.is_empty());
        assert_eq!(r2.skipped, vec!["dup".to_string()]);
        assert!(r2.failed.is_empty(), "AlreadyInstalled must not be failed");
    }

    #[test]
    fn install_plugin_agents_force_overwrites_already_installed() {
        // Regression: the CLI --force flag was previously dropped on the
        // agent path, so re-installing with --force silently routed agents
        // to `skipped`. Threading force=true through install_plugin_agents
        // must now put them back into `installed`.
        use crate::project::KiroProject;

        let (_dir, svc) = temp_service();
        let plugin_tmp = tempfile::tempdir().unwrap();
        let agents_dir = plugin_tmp.path().join("agents");
        fs::create_dir_all(&agents_dir).unwrap();
        fs::write(
            agents_dir.join("dup.md"),
            "---\nname: dup\n---\nfirst body\n",
        )
        .unwrap();

        let project_tmp = tempfile::tempdir().unwrap();
        let project = KiroProject::new(project_tmp.path().to_path_buf());

        let r1 = svc.install_plugin_agents(
            &project,
            plugin_tmp.path(),
            &["./agents/".to_string()],
            InstallMode::New,
            false, // accept_mcp: existing fixtures don't carry MCP servers
            "mp",
            "p",
            Some("1.0.0"),
        );
        assert_eq!(r1.installed, vec!["dup".to_string()]);

        // Update the source, re-install with force=true and a new version —
        // both the prompt body and the tracking metadata should reflect
        // the replacement.
        fs::write(
            agents_dir.join("dup.md"),
            "---\nname: dup\n---\nsecond body\n",
        )
        .unwrap();

        let r2 = svc.install_plugin_agents(
            &project,
            plugin_tmp.path(),
            &["./agents/".to_string()],
            InstallMode::Force,
            false, // accept_mcp: this fixture's agents have no MCP entries
            "mp",
            "p",
            Some("2.0.0"),
        );
        assert_eq!(
            r2.installed,
            vec!["dup".to_string()],
            "force install must replace, not skip"
        );
        assert!(
            r2.skipped.is_empty(),
            "force must not route already-installed to skipped: {:?}",
            r2.skipped
        );

        let prompt =
            fs::read_to_string(project_tmp.path().join(".kiro/agents/prompts/dup.md")).unwrap();
        assert!(
            prompt.contains("second body"),
            "prompt must reflect the replaced source, got: {prompt}"
        );

        // Tracking JSON must also reflect the overwrite — previously a
        // refactor could have updated disk files but not tracking, and
        // this test would still pass without this assertion.
        let tracking = project.load_installed_agents().expect("load tracking");
        let meta = tracking
            .agents
            .get("dup")
            .expect("tracking entry for 'dup'");
        assert_eq!(
            meta.version.as_deref(),
            Some("2.0.0"),
            "tracking metadata must reflect the force-installed version, got: {:?}",
            meta.version
        );
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
            InstallMode::New,
            false, // accept_mcp: existing fixtures don't carry MCP servers
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
            // Create dest with a minimal marketplace manifest and a
            // `.git/HEAD` marker that `resolve_structured_source` checks
            // to distinguish a complete clone from a partial one.
            let mp_dir = dest.join(".claude-plugin");
            fs::create_dir_all(&mp_dir).unwrap();
            fs::write(
                mp_dir.join("marketplace.json"),
                r#"{"name":"mock-market","owner":{"name":"Test"},"plugins":[{"name":"mock-plugin","description":"A mock plugin","source":"./plugins/mock"}]}"#,
            )
            .unwrap();
            let git_dir = dest.join(".git");
            fs::create_dir_all(&git_dir).unwrap();
            fs::write(git_dir.join("HEAD"), b"ref: refs/heads/main\n").unwrap();
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

    // -------------------------------------------------------------------
    // resolve_plugin_dir
    // -------------------------------------------------------------------

    #[test]
    fn resolve_plugin_dir_relative_path_returns_joined_path() {
        let (dir, svc) = temp_service();
        let marketplace_path = dir.path().join("marketplace");
        let plugin_dir_on_disk = marketplace_path.join("plugins/foo");
        fs::create_dir_all(&plugin_dir_on_disk).expect("create plugin dir");

        let entry = PluginEntry {
            name: "foo".to_string(),
            description: None,
            source: PluginSource::RelativePath(
                crate::validation::RelativePath::new("./plugins/foo").unwrap(),
            ),
        };

        let resolved = svc
            .resolve_plugin_dir(&entry, &marketplace_path, "mp", GitProtocol::Https)
            .expect("happy path");
        assert_eq!(resolved, plugin_dir_on_disk);
    }

    #[test]
    fn resolve_plugin_dir_relative_path_missing_returns_directory_missing() {
        // Regression guard for PluginError::DirectoryMissing — the scan
        // fallback or a stale manifest can point to a directory that does
        // not exist on disk, and that must be a typed error.
        let (dir, svc) = temp_service();
        let marketplace_path = dir.path().join("marketplace");
        fs::create_dir_all(&marketplace_path).expect("create marketplace root");

        let entry = PluginEntry {
            name: "ghost".to_string(),
            description: None,
            source: PluginSource::RelativePath(
                crate::validation::RelativePath::new("./plugins/ghost").unwrap(),
            ),
        };

        let err = svc
            .resolve_plugin_dir(&entry, &marketplace_path, "mp", GitProtocol::Https)
            .expect_err("missing dir must error");
        assert!(
            matches!(err, Error::Plugin(PluginError::DirectoryMissing { .. })),
            "expected PluginError::DirectoryMissing, got: {err:?}"
        );
    }

    #[test]
    fn relative_path_newtype_rejects_traversal_at_construction() {
        // The newtype closes the programmatic-bypass vector entirely:
        // `RelativePath::new("../../etc")` fails before a `PluginSource`
        // can be constructed. This replaces the earlier
        // `resolve_plugin_dir_rejects_programmatic_*_traversal` tests
        // that exercised the belt-and-braces use-site checks.
        assert!(crate::validation::RelativePath::new("../../etc").is_err());
        assert!(crate::validation::RelativePath::new("/etc/passwd").is_err());
        assert!(crate::validation::RelativePath::new("\0").is_err());
        assert!(crate::validation::RelativePath::new("sub\\..\\etc").is_err());
        assert!(crate::validation::RelativePath::new("ok/path").is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn resolve_plugin_dir_refuses_symlinked_relative_path() {
        // Regression: a malicious marketplace could drop a symlink at
        // `plugins/foo -> /etc` and, because `Path::exists()` follows
        // symlinks, an earlier resolve_plugin_dir would return the
        // resolved symlink target — letting the install pull files
        // from outside the marketplace tree. The fix uses
        // `fs::symlink_metadata` and refuses any symlinked path as
        // "directory missing."
        let (dir, svc) = temp_service();
        let marketplace_path = dir.path().join("marketplace");
        fs::create_dir_all(&marketplace_path).expect("create marketplace root");

        let target = dir.path().join("outside");
        fs::create_dir_all(&target).expect("create target dir");
        std::os::unix::fs::symlink(&target, marketplace_path.join("plugins"))
            .expect("create symlink");

        let entry = PluginEntry {
            name: "foo".to_string(),
            description: None,
            source: PluginSource::RelativePath(
                crate::validation::RelativePath::new("./plugins").unwrap(),
            ),
        };

        let err = svc
            .resolve_plugin_dir(&entry, &marketplace_path, "mp", GitProtocol::Https)
            .expect_err("symlink must be refused");
        assert!(
            matches!(err, Error::Plugin(PluginError::DirectoryMissing { .. })),
            "expected DirectoryMissing for symlinked path, got: {err:?}"
        );
    }

    #[test]
    fn resolve_structured_source_recovers_from_partial_clone() {
        // Regression: if a prior clone crashed mid-way, `dest` exists
        // but `.git/` is missing — earlier code treated the partial
        // directory as a valid cached clone and returned it without
        // re-cloning, so the install would proceed with whatever
        // half-fetched files happened to be on disk. The resolver must
        // detect the partial state via `.git/` absence, wipe it, and
        // re-clone.
        let (_dir, svc) = temp_service();
        svc.cache.ensure_dirs().unwrap();

        let dest = svc.cache.plugin_path("mp", "mock-plugin");
        fs::create_dir_all(&dest).unwrap();
        fs::write(dest.join("stale.txt"), b"left over from crash").unwrap();
        assert!(
            !dest.join(".git").exists(),
            "fixture: pre-partial-clone dir must not contain .git/"
        );

        let entry = PluginEntry {
            name: "mock-plugin".to_string(),
            description: None,
            source: PluginSource::Structured(StructuredSource::GitHub {
                repo: "owner/repo".to_string(),
                git_ref: None,
                sha: None,
            }),
        };

        let resolved = svc
            .resolve_plugin_dir(&entry, Path::new("/unused"), "mp", GitProtocol::Https)
            .expect("partial clone should be recovered and cloned fresh");

        assert_eq!(resolved, dest);
        assert!(
            resolved.join(".git/HEAD").exists(),
            "fresh clone must have replaced partial dir"
        );
        assert!(
            !resolved.join("stale.txt").exists(),
            "stale file from partial clone must be removed"
        );
    }

    #[test]
    fn resolve_structured_source_reuses_complete_clone() {
        // Sanity check: a directory with `.git/` is treated as a valid
        // cached clone and re-used without re-calling clone_repo.
        let (_dir, svc) = temp_service();
        svc.cache.ensure_dirs().unwrap();

        let entry = PluginEntry {
            name: "mock-plugin".to_string(),
            description: None,
            source: PluginSource::Structured(StructuredSource::GitHub {
                repo: "owner/repo".to_string(),
                git_ref: None,
                sha: None,
            }),
        };

        // First call triggers a clone.
        svc.resolve_plugin_dir(&entry, Path::new("/unused"), "mp", GitProtocol::Https)
            .expect("first resolve");
        // Mark a distinguishing file so we can assert it survives the
        // second call (i.e. no re-clone happened).
        let dest = svc.cache.plugin_path("mp", "mock-plugin");
        fs::write(dest.join("sentinel.txt"), b"should survive").unwrap();

        svc.resolve_plugin_dir(&entry, Path::new("/unused"), "mp", GitProtocol::Https)
            .expect("second resolve");

        assert!(
            dest.join("sentinel.txt").exists(),
            "complete clone must be reused, not re-cloned"
        );
    }

    // No explicit test for "programmatic GitSubdir.path traversal" is
    // needed: `path: RelativePath` on the struct makes such an attack
    // uninstantiable in safe Rust. `relative_path_newtype_rejects_traversal_at_construction`
    // above verifies the single choke-point that used to be duplicated
    // as belt-and-braces use-site checks.

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
    fn add_serializes_concurrent_same_name_adds() {
        // Many threads racing to add the same marketplace name. Without
        // the outer registry lock spanning existence-check + rename +
        // register, multiple threads could pass `final_dir.exists()` and
        // race the rename — leaving losers with confusing IO errors and
        // potentially clobbered final_dir content. With the lock, exactly
        // one thread wins with Ok and every other gets AlreadyRegistered.
        //
        // Fanout 8 (was 2): a broken lock that just happens to win the
        // race on a 2-thread fight passes the smaller test. Eight
        // contenders make the failure mode visible.
        const FANOUT: usize = 8;
        let (_dir, svc) = temp_service();
        let svc = std::sync::Arc::new(svc);

        let barrier = std::sync::Arc::new(std::sync::Barrier::new(FANOUT));

        let handles: Vec<_> = (0..FANOUT)
            .map(|_| {
                let svc = std::sync::Arc::clone(&svc);
                let barrier = std::sync::Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    svc.add("owner/repo", GitProtocol::Https)
                })
            })
            .collect();

        let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        let ok_count = results.iter().filter(|r| r.is_ok()).count();
        let already_count = results
            .iter()
            .filter(|r| {
                matches!(
                    r,
                    Err(Error::Marketplace(
                        MarketplaceError::AlreadyRegistered { .. }
                    ))
                )
            })
            .count();
        assert_eq!(ok_count, 1, "exactly one concurrent add should succeed");
        assert_eq!(
            already_count,
            FANOUT - 1,
            "every loser must surface AlreadyRegistered, not a generic IO error: {:?}",
            results
                .iter()
                .filter_map(|r| r.as_ref().err().map(ToString::to_string))
                .collect::<Vec<_>>()
        );

        // No `_pending_*` staging dirs may leak. Both threads create their
        // own pid+seq-suffixed temp dir; the loser's dir must be cleaned up
        // by the DirCleanupGuard before its add() returns.
        let marketplaces_dir = svc.cache.marketplaces_dir();
        let leftovers: Vec<_> = fs::read_dir(&marketplaces_dir)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().starts_with("_pending_"))
            .collect();
        assert!(
            leftovers.is_empty(),
            "no _pending_ staging dirs should remain after a concurrent add race"
        );

        // The winner's marketplace is registered exactly once.
        let known = svc.list().expect("list");
        assert_eq!(known.len(), 1);
        assert_eq!(known[0].name, "mock-market");
    }

    #[test]
    fn add_and_remove_race_leaves_consistent_state() {
        // Concurrent add(mock-market) + remove(mock-market). The two
        // operations both take the registry lock — they must serialise
        // without leaving the cache half-deleted, half-registered. Either:
        //   - add wins first, then remove succeeds (registry empty, dir gone)
        //   - remove runs first (NotFound), then add succeeds (registry has 1)
        // Importantly: never both registered AND directory deleted.
        let (_dir, svc) = temp_service();
        let svc = std::sync::Arc::new(svc);
        // Pre-add so remove has something to race against.
        svc.add("owner/repo", GitProtocol::Https)
            .expect("seed marketplace");

        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let svc_a = std::sync::Arc::clone(&svc);
        let bar_a = std::sync::Arc::clone(&barrier);
        let svc_r = std::sync::Arc::clone(&svc);
        let bar_r = std::sync::Arc::clone(&barrier);

        // Thread A: re-add (collides with the existing entry → AlreadyRegistered
        // unless remove runs first → succeeds).
        let h_add = std::thread::spawn(move || {
            bar_a.wait();
            svc_a.add("owner/repo", GitProtocol::Https)
        });
        // Thread R: remove (succeeds unless add hasn't completed yet, in
        // which case... well, the seed already ran, so it must succeed).
        let h_rm = std::thread::spawn(move || {
            bar_r.wait();
            svc_r.remove("mock-market")
        });

        let add_result = h_add.join().unwrap();
        let rm_result = h_rm.join().unwrap();

        // After serialisation, the registry must be in one of the two
        // valid steady states:
        //   - empty (remove won the race): add returned AlreadyRegistered
        //     IF it ran before remove, OR succeeded if it ran after.
        //   - has the marketplace (add re-added after remove): remove
        //     succeeded, then add succeeded.
        let known = svc.list().expect("list");
        match known.len() {
            0 => {
                // Remove ran last. Add must have returned AlreadyRegistered
                // (which it did before remove). Either result is acceptable.
                assert!(
                    rm_result.is_ok(),
                    "remove must have succeeded: {rm_result:?}"
                );
            }
            1 => {
                // Add ran last → registered marketplace dir must exist.
                assert_eq!(known[0].name, "mock-market");
                assert!(
                    svc.cache.marketplace_path("mock-market").exists(),
                    "registered marketplace must have its on-disk dir"
                );
                assert!(
                    add_result.is_ok(),
                    "add must have succeeded: {add_result:?}"
                );
            }
            n => panic!(
                "registry must end with 0 or 1 entries after add/remove race, got {n}: \
                 add={add_result:?}, rm={rm_result:?}"
            ),
        }
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

    #[test]
    fn add_rejects_http_url_by_default() {
        // Plaintext HTTP is unauthenticated. Without TLS a network
        // attacker can swap the marketplace contents and gain
        // long-lived code execution via skills/agents/MCP servers
        // that the cache then keeps. The default rejects with
        // InsecureSource so users have to explicitly opt in.
        let (_dir, svc) = temp_service();
        let err = svc
            .add("http://example.com/repo.git", GitProtocol::Https)
            .expect_err("http:// must be rejected by default");
        assert!(
            matches!(
                err,
                Error::Marketplace(MarketplaceError::InsecureSource { .. })
            ),
            "expected InsecureSource, got {err:?}"
        );
        // The error names the opt-in knob so the user knows the workaround.
        let msg = err.to_string();
        assert!(
            msg.contains("http://") && msg.contains("allow-insecure-http"),
            "error must point at the remediation: {msg}"
        );
    }

    #[test]
    fn add_accepts_http_url_when_explicitly_opted_in() {
        // Setting InsecureHttpPolicy::Allow MUST let http:// proceed.
        // Plumbed end-to-end so a CLI flag like --allow-insecure-http or
        // a Tauri checkbox can flip it.
        let (_dir, svc) = temp_service();
        let result = svc.add(
            "http://example.com/repo.git",
            MarketplaceAddOptions::new(GitProtocol::Https).allow_insecure_http(),
        );
        // The mock backend will succeed regardless of URL scheme; we're
        // proving here that the http guard does not fire when opted in.
        assert!(
            result.is_ok(),
            "opted-in http:// add should succeed against the mock, got {result:?}"
        );
    }

    #[test]
    fn add_accepts_https_url_without_opt_in() {
        // The strict default must NOT reject `https://`; only `http://`
        // is gated. Otherwise we'd block the common case (a private git
        // server with a TLS cert).
        let (_dir, svc) = temp_service();
        svc.add("https://example.com/repo.git", GitProtocol::Https)
            .expect("https:// is the safe path and must work without opt-in");
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

        // Verify no directory was left behind (DirCleanupGuard should clean up).
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

    // -------------------------------------------------------------------
    // install_skills: per-skill surfacing + FailedSkillReason (audit #30)
    // -------------------------------------------------------------------
    //
    // These tests pin the behavior introduced by the issue-#30 audit fold-in:
    // install_skills used to vanish per-skill read/parse failures into
    // `warn!` + `continue`. They now surface as structured `skipped_skills`
    // entries, and requested-but-missing names carry the typed
    // `FailedSkillReason::RequestedButNotFound` variant so frontends can
    // distinguish a typo from an install error.

    /// Per-skill frontmatter parse failures inside `install_skills` used
    /// to silently drop. They now land in `skipped_skills` as structured
    /// entries — the install count stays accurate even when some skill
    /// directories have broken `SKILL.md` files.
    #[test]
    fn install_skills_surfaces_malformed_skill_md_as_skipped_skill() {
        use crate::project::KiroProject;

        let (_dir, svc) = temp_service();
        let project_tmp = tempfile::tempdir().unwrap();
        let project = KiroProject::new(project_tmp.path().to_path_buf());

        let plugin_tmp = tempfile::tempdir().unwrap();
        let ok_dir = plugin_tmp.path().join("ok");
        fs::create_dir_all(&ok_dir).unwrap();
        fs::write(
            ok_dir.join("SKILL.md"),
            "---\nname: ok\ndescription: works\n---\nbody\n",
        )
        .unwrap();
        let broken_dir = plugin_tmp.path().join("broken");
        fs::create_dir_all(&broken_dir).unwrap();
        // Missing closing `---` breaks the frontmatter parse.
        fs::write(broken_dir.join("SKILL.md"), "---\nname: broken\n").unwrap();

        let skill_dirs = vec![ok_dir, broken_dir];
        let result = svc.install_skills(
            &project,
            &skill_dirs,
            &InstallFilter::All,
            InstallMode::New,
            "mp1",
            "plug1",
            None,
        );

        assert_eq!(result.installed, vec!["ok".to_string()]);
        assert_eq!(result.skipped_skills.len(), 1);
        assert_eq!(
            result.skipped_skills[0].name_hint.as_deref(),
            Some("broken")
        );
        assert!(
            matches!(
                result.skipped_skills[0].reason,
                browse::SkippedSkillReason::FrontmatterInvalid { .. }
            ),
            "expected FrontmatterInvalid, got: {:?}",
            result.skipped_skills[0].reason
        );
    }

    /// A `Names(_)` filter requesting a skill that no discovered
    /// SKILL.md produces must surface as
    /// `FailedSkillReason::RequestedButNotFound` — distinguishable from
    /// an install error so the frontend can render typo UX separately
    /// from an I/O or filesystem failure.
    #[test]
    fn install_skills_requested_but_not_found_uses_typed_reason() {
        use crate::project::KiroProject;

        let (_dir, svc) = temp_service();
        let project_tmp = tempfile::tempdir().unwrap();
        let project = KiroProject::new(project_tmp.path().to_path_buf());

        let plugin_tmp = tempfile::tempdir().unwrap();
        let only_dir = plugin_tmp.path().join("present");
        fs::create_dir_all(&only_dir).unwrap();
        fs::write(
            only_dir.join("SKILL.md"),
            "---\nname: present\ndescription: here\n---\nbody\n",
        )
        .unwrap();

        let requested = vec!["absent".to_string()];
        let result = svc.install_skills(
            &project,
            &[only_dir],
            &InstallFilter::Names(&requested),
            InstallMode::New,
            "mp1",
            "plug1",
            None,
        );

        assert!(result.installed.is_empty(), "nothing should install");
        assert_eq!(result.failed.len(), 1);
        assert_eq!(result.failed[0].name, "absent");
        match &result.failed[0].kind {
            FailedSkillReason::RequestedButNotFound { plugin } => {
                assert_eq!(plugin, "plug1");
            }
            other => panic!("expected RequestedButNotFound, got: {other:?}"),
        }
    }

    // -------------------------------------------------------------------
    // FailedSkillReason wire-format regression guards
    // -------------------------------------------------------------------
    //
    // `FailedSkillReason` crosses the Tauri FFI via `InstallSkillsResult`,
    // so its JSON shape is a public contract with the frontend. Pin the
    // exact representation here so a silent serde-tag rename or casing
    // flip surfaces as a failing unit test in this crate BEFORE a
    // bindings.ts regeneration ever reaches the UI.

    #[test]
    fn failed_skill_reason_install_failed_json_shape() {
        let reason = FailedSkillReason::InstallFailed;
        let json = serde_json::to_value(&reason).expect("serialize");
        assert_eq!(
            json,
            serde_json::json!({ "kind": "install_failed" }),
            "InstallFailed is unit-shaped; the wire carries only the \
             discriminant (no payload). FailedSkill.error holds the \
             human-readable detail."
        );
    }

    #[test]
    fn failed_skill_reason_requested_but_not_found_json_shape() {
        let reason = FailedSkillReason::RequestedButNotFound {
            plugin: "plug1".into(),
        };
        let json = serde_json::to_value(&reason).expect("serialize");
        assert_eq!(
            json,
            serde_json::json!({
                "kind": "requested_but_not_found",
                "plugin": "plug1",
            })
        );
    }

    /// Regression guard for the `install_skills` per-skill
    /// `read_to_string` failure path. Before the #30 audit fold-in,
    /// this vanished into `warn!` + `continue`; now it must surface as
    /// a structured `SkippedSkill` with `ReadFailed`. The sibling
    /// `list_all_skills_surfaces_unreadable_skill_md_as_skipped_skill`
    /// exists in the browse module and covers the identical branch in
    /// `collect_skills_for_plugin_into` — this test catches a
    /// divergence where the two structurally-identical codepaths gain
    /// different error-wrapping over time.
    #[cfg(unix)]
    #[test]
    fn install_skills_surfaces_unreadable_skill_md_as_skipped_skill() {
        use std::os::unix::fs::PermissionsExt;

        use crate::project::KiroProject;

        let (_dir, svc) = temp_service();
        let project_tmp = tempfile::tempdir().unwrap();
        let project = KiroProject::new(project_tmp.path().to_path_buf());

        let plugin_tmp = tempfile::tempdir().unwrap();
        let skill_dir = plugin_tmp.path().join("vault");
        fs::create_dir_all(&skill_dir).unwrap();
        let skill_md = skill_dir.join("SKILL.md");
        fs::write(
            &skill_md,
            "---\nname: vault\ndescription: locked\n---\nbody\n",
        )
        .expect("write SKILL.md");
        // Remove all permissions so read_to_string fails with EACCES.
        fs::set_permissions(&skill_md, fs::Permissions::from_mode(0o000))
            .expect("chmod 000 SKILL.md");

        let result = svc.install_skills(
            &project,
            std::slice::from_ref(&skill_dir),
            &InstallFilter::All,
            InstallMode::New,
            "mp1",
            "plug1",
            None,
        );
        // Restore permissions BEFORE assertions so tempdir cleanup can
        // delete the file even if an assertion panics.
        fs::set_permissions(&skill_md, fs::Permissions::from_mode(0o644)).expect("restore perms");

        assert!(
            result.installed.is_empty(),
            "unreadable skill must not install"
        );
        assert_eq!(result.skipped_skills.len(), 1);
        assert_eq!(result.skipped_skills[0].name_hint.as_deref(), Some("vault"));
        assert_eq!(result.skipped_skills[0].plugin, "plug1");
        assert!(
            matches!(
                result.skipped_skills[0].reason,
                browse::SkippedSkillReason::ReadFailed { .. }
            ),
            "expected ReadFailed, got: {:?}",
            result.skipped_skills[0].reason
        );
    }

    /// Regression guard for `FailedSkillReason::InstallFailed`. Induces
    /// an install failure (a regular file where `.kiro/skills` would
    /// need to be a directory) and pins that the error routes to
    /// `failed` with `kind = InstallFailed`, not to `skipped_skills`
    /// or anywhere else.
    ///
    /// Cross-platform (no chmod needed): a file sitting at the skills
    /// directory path causes `fs::create_dir_all` to fail on every
    /// OS we support. Install-time errors used to only carry a string;
    /// the typed `kind` here is the programmatic contract that
    /// survives Display rewording.
    #[test]
    fn install_skills_surfaces_typed_install_failed_on_fs_error() {
        use crate::project::KiroProject;

        let (_dir, svc) = temp_service();
        let project_tmp = tempfile::tempdir().unwrap();
        // Seed `.kiro/skills` as a regular file so create_dir_all fails.
        let kiro = project_tmp.path().join(".kiro");
        fs::create_dir_all(&kiro).unwrap();
        fs::write(kiro.join("skills"), b"not a directory").expect("write blocker");

        let project = KiroProject::new(project_tmp.path().to_path_buf());

        let plugin_tmp = tempfile::tempdir().unwrap();
        let skill_dir = plugin_tmp.path().join("target");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: target\ndescription: will-fail-install\n---\nbody\n",
        )
        .unwrap();

        let result = svc.install_skills(
            &project,
            &[skill_dir],
            &InstallFilter::All,
            InstallMode::New,
            "mp1",
            "plug1",
            None,
        );

        assert!(
            result.installed.is_empty(),
            "install must fail, got installed: {:?}",
            result.installed
        );
        assert!(
            result.skipped_skills.is_empty(),
            "per-skill read/parse succeeded; failure belongs to `failed`, \
             not `skipped_skills`: {:?}",
            result.skipped_skills
        );
        assert_eq!(result.failed.len(), 1);
        assert_eq!(result.failed[0].name, "target");
        assert!(
            !result.failed[0].error.is_empty(),
            "FailedSkill.error must carry the human-readable chain, \
             got empty string"
        );
        assert!(
            matches!(result.failed[0].kind, FailedSkillReason::InstallFailed),
            "expected InstallFailed, got: {:?}",
            result.failed[0].kind
        );
    }
}

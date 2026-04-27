//! Browse-side service methods: enumerate skills across marketplaces and
//! plugins, cross-referenced with the target project's installed set.
//!
//! Frontends (CLI, Tauri) remain thin wrappers — they decide how to
//! construct the [`MarketplaceService`] and how to frame errors, but
//! they do not duplicate the enumeration loop or the per-skill
//! frontmatter-parsing logic.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;
use tracing::{debug, error, warn};

use crate::error::{Error, PluginError, error_full_chain};
use crate::marketplace::{PluginEntry, PluginSource, StructuredSource};
use crate::plugin::{PluginManifest, discover_skill_dirs};
use crate::project::InstalledSkills;
use crate::service::MarketplaceService;
use crate::skill::parse_frontmatter;

// ---------------------------------------------------------------------------
// Response types
// ---------------------------------------------------------------------------

/// Information about a single skill, cross-referenced with the target
/// project's installed set.
///
/// `installed` is a point-in-time snapshot — the project's
/// `.kiro/installed.json` at the moment the listing was built. Callers
/// that want a live view must re-query.
#[derive(Clone, Debug, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct SkillInfo {
    pub name: String,
    pub description: String,
    pub plugin: String,
    pub marketplace: String,
    pub installed: bool,
}

/// Result of a marketplace-wide skill listing. The bulk path continues
/// past per-plugin errors (missing directory, malformed manifest) to
/// preserve the partial listing; `skipped` records plugin-level drops
/// and `skipped_skills` records per-skill drops inside otherwise-working
/// plugins, so the frontend can show a warning rather than silently
/// shrinking the count.
#[derive(Clone, Debug, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct BulkSkillsResult {
    pub skills: Vec<SkillInfo>,
    pub skipped: Vec<SkippedPlugin>,
    pub skipped_skills: Vec<SkippedSkill>,
}

/// A plugin that was excluded from a bulk listing. Carries both a
/// human-readable `reason` (the error's rendered Display, suitable for
/// direct UI rendering or log lines) and a structured `kind` that
/// frontends match on for variant-specific affordances (e.g. a "clone"
/// button for [`SkippedReason::RemoteSourceNotLocal`]). The two are
/// deliberately redundant: `reason` is free-form text that may rephrase
/// over time, while `kind` is the stable programmatic contract.
///
/// Fields are `pub(crate)` so external callers cannot desync the two
/// — construction routes exclusively through
/// [`SkippedPlugin::from_plugin_error`], which derives both from a
/// single source error. Read access from outside the crate happens via
/// the Serde/specta boundary (the generated TypeScript type still
/// exposes all three fields, because Serde ignores Rust visibility).
#[derive(Clone, Debug, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct SkippedPlugin {
    pub(crate) name: String,
    pub(crate) reason: String,
    pub(crate) kind: SkippedReason,
}

impl SkippedPlugin {
    /// Construct a [`SkippedPlugin`] from the plugin's name and the
    /// [`Error`] that caused it to be skipped, keeping `reason` (the
    /// error's rendered Display) and `kind` (the programmatic
    /// projection) in lockstep. Returns `None` when the error is not a
    /// plugin-level skip — callers must propagate such errors instead
    /// of folding them into the response.
    ///
    /// This is the ONLY way to build a [`SkippedPlugin`] outside the
    /// service module (fields are `pub(crate)`), so `reason` and `kind`
    /// cannot drift from the underlying error. Subsumes the previous
    /// free helper `plugin_skip_reason(&Error) -> Option<SkippedReason>`
    /// — callers that only need the kind still have
    /// [`SkippedReason::from_plugin_error`].
    #[must_use]
    pub(crate) fn from_plugin_error(name: String, err: &Error) -> Option<Self> {
        let Error::Plugin(pe) = err else { return None };
        let kind = SkippedReason::from_plugin_error(pe)?;
        Some(Self {
            name,
            // `error_full_chain`, not `err.to_string()` — variants like
            // `PluginError::DirectoryUnreadable` and `ManifestReadFailed`
            // carry an `io::Error` via `#[source]`, and their Display
            // deliberately omits the source's detail ("could not access
            // plugin directory at {path}" with no "permission denied"
            // suffix). `err.to_string()` would drop that detail at the
            // Tauri FFI boundary where it becomes `SkippedPlugin.reason`.
            // CLAUDE.md mandates `error_full_chain` at such boundaries.
            // The sibling constructor `FailedSkill::install_failed`
            // already uses this; keep the two lockstep.
            reason: error_full_chain(err),
            kind,
        })
    }

    /// Name of the plugin that was skipped. Public accessor so tests
    /// and (future) crate-external code that only reads the value can
    /// stay out of the wire-format derivation. The equivalent read via
    /// the Serde-generated TypeScript type is `SkippedPlugin.name`.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Human-readable failure message (the source error's rendered
    /// Display). Use [`Self::kind`] for programmatic matching; use
    /// this for log lines and simple UI labels.
    #[must_use]
    pub fn reason(&self) -> &str {
        &self.reason
    }

    /// Structured classification of the skip reason. Stable contract
    /// for frontends that render variant-specific affordances.
    #[must_use]
    pub fn kind(&self) -> &SkippedReason {
        &self.kind
    }
}

impl SkippedReason {
    /// Project a [`PluginError`] into its [`SkippedReason`] counterpart
    /// if — and only if — the variant represents a plugin-level skip
    /// that the bulk/listing paths should fold into the response rather
    /// than propagate as an `Err`. Returns `None` for non-plugin-level
    /// variants (e.g. [`PluginError::NotFound`], which is a "caller
    /// asked for the wrong thing" bug, not a damaged plugin).
    ///
    /// This is the single source of truth for both the classification
    /// (which variants skip vs. propagate) and the wire-format projection
    /// (how each variant maps to a frontend-serializable shape). Keeping
    /// them in one function means a new plugin-level variant on
    /// [`PluginError`] either lands here (and is automatically surfaced
    /// to the frontend) or does not (and will propagate as an error) —
    /// the two classifications cannot drift.
    #[must_use]
    pub fn from_plugin_error(err: &PluginError) -> Option<Self> {
        match err {
            PluginError::DirectoryMissing { path } => {
                Some(Self::DirectoryMissing { path: path.clone() })
            }
            PluginError::NotADirectory { path } => Some(Self::NotADirectory { path: path.clone() }),
            PluginError::SymlinkRefused { path } => {
                Some(Self::SymlinkRefused { path: path.clone() })
            }
            PluginError::DirectoryUnreadable { path, source } => Some(Self::DirectoryUnreadable {
                path: path.clone(),
                reason: error_full_chain(source),
            }),
            PluginError::InvalidManifest { path, reason } => Some(Self::InvalidManifest {
                path: path.clone(),
                reason: reason.clone(),
            }),
            PluginError::ManifestReadFailed { path, source } => Some(Self::ManifestReadFailed {
                path: path.clone(),
                reason: error_full_chain(source),
            }),
            PluginError::RemoteSourceNotLocal {
                plugin,
                plugin_source,
            } => Some(Self::RemoteSourceNotLocal {
                plugin: plugin.clone(),
                source: plugin_source.clone(),
            }),
            PluginError::NoSkills { path, .. } => Some(Self::NoSkills { path: path.clone() }),
            // Explicit match on non-skip variants rather than `_ => None`
            // so adding a new PluginError variant triggers a compiler
            // error until the author decides whether it's plugin-level.
            // `NotFound` and `ManifestNotFound` stay here because they
            // represent "caller asked for the wrong thing" — a user-input
            // bug, not a damaged plugin to fold into `skipped`.
            PluginError::NotFound { .. } | PluginError::ManifestNotFound { .. } => None,
        }
    }
}

/// Why a plugin was excluded from a bulk listing. Structured counterpart
/// to [`SkippedPlugin::reason`] so frontends can match on the cause
/// (rendering variant-specific UI like a "clone" button for
/// [`Self::RemoteSourceNotLocal`]) instead of substring-matching a
/// rendered error message.
///
/// Mirrors the plugin-level subset of [`PluginError`] — exactly the
/// variants the bulk path folds into `skipped` rather than propagating.
/// Kept as a distinct type because [`PluginError`] is a Display-oriented
/// error carrying non-`Clone` `io::Error` chains, while `SkippedReason`
/// is `Serialize + specta::Type` data that crosses the FFI boundary.
/// Translating one to the other is the service layer's job, performed in
/// a single place so the projection stays consistent with the error
/// classifier.
#[derive(Clone, Debug, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum SkippedReason {
    DirectoryMissing {
        path: PathBuf,
    },
    NotADirectory {
        path: PathBuf,
    },
    SymlinkRefused {
        path: PathBuf,
    },
    DirectoryUnreadable {
        path: PathBuf,
        reason: String,
    },
    InvalidManifest {
        path: PathBuf,
        reason: String,
    },
    ManifestReadFailed {
        path: PathBuf,
        reason: String,
    },
    RemoteSourceNotLocal {
        plugin: String,
        source: StructuredSource,
    },
    /// The plugin exists and its manifest is well-formed, but it
    /// declares no skills. Defensive classification — today no producer
    /// in the bulk/listing path returns [`PluginError::NoSkills`], but
    /// folding it into `skipped` means a future caller that DOES surface
    /// it can't accidentally abort the bulk listing. The plugin name
    /// lives on the wrapping [`SkippedPlugin`]; this variant carries
    /// only `path` for UI remediation (the directory the user might
    /// populate).
    NoSkills {
        path: PathBuf,
    },
}

/// A skill that was excluded from a listing because its `SKILL.md` or
/// frontmatter could not be read. Surfaces what previously vanished
/// into `warn!`-then-`continue` so the frontend can render "N skills
/// failed to load" with a drill-down rather than silently shrinking the
/// listed count.
#[derive(Clone, Debug, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct SkippedSkill {
    /// Name of the plugin this skill was being enumerated under. The
    /// bulk path [`MarketplaceService::list_all_skills`] accumulates
    /// `SkippedSkill`s across every plugin in a marketplace, so without
    /// this attribution the frontend would have no way to group "N
    /// skills failed to load in plugin X" — making the structured
    /// surface strictly less useful than the per-plugin `warn!` it
    /// replaced. Per-plugin callers already have the plugin context
    /// but carry it anyway so both code paths produce identical shapes.
    pub plugin: String,
    /// Directory name of the skill as a best-effort label. Not a
    /// guarantee the skill *would* have had this name — the frontmatter
    /// `name` is authoritative, and parsing it is precisely what failed.
    /// `None` when `Path::file_name()` cannot extract a component
    /// (empty path, root, or a path terminating in `..`). Encoded as
    /// `Option<String>` rather than a sentinel empty string so the
    /// frontend's type system forces the "no label available" branch
    /// to be handled explicitly — specta renders it as `string | null`.
    pub name_hint: Option<String>,
    /// Path to the `SKILL.md` file that could not be consumed.
    pub path: PathBuf,
    pub reason: SkippedSkillReason,
}

/// Why an individual skill was excluded from a listing. Both variants
/// describe a working plugin with a single broken skill file — a
/// plugin-level failure surfaces as [`SkippedPlugin`] / [`SkippedReason`]
/// instead.
#[derive(Clone, Debug, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum SkippedSkillReason {
    /// Reading `SKILL.md` failed (permission denied, I/O error, invalid
    /// UTF-8, etc.). `reason` carries the underlying error's Display.
    ReadFailed { reason: String },
    /// `SKILL.md` read successfully but the frontmatter could not be
    /// parsed (missing fences, malformed YAML, missing `name`, etc.).
    FrontmatterInvalid { reason: String },
}

/// Result of [`MarketplaceService::list_skills_for_plugin`]. Mirrors
/// [`BulkSkillsResult`] for the single-plugin case so per-skill read
/// failures surface structurally rather than only via `warn!` logs.
#[derive(Clone, Debug, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct PluginSkillsResult {
    pub skills: Vec<SkillInfo>,
    pub skipped_skills: Vec<SkippedSkill>,
}

/// Result of [`MarketplaceService::count_skills_for_plugin`].
/// Distinguishes the three cases the frontend must render differently:
/// a known count, a remote plugin (not locally countable), and a local
/// plugin whose directory or manifest could not be loaded. Replaces the
/// prior `usize` that collapsed failures into a silent `0`.
#[derive(Clone, Debug, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(tag = "state", rename_all = "snake_case")]
#[non_exhaustive]
pub enum SkillCount {
    /// The plugin directory was readable; `count` is the number of
    /// discovered skill directories (including the legitimate zero case).
    Known { count: u32 },

    /// Plugin source is remote (GitHub / git URL). Skills cannot be
    /// enumerated without cloning, which the listing path never does.
    /// Distinct from `ManifestFailed { reason: RemoteSourceNotLocal }`:
    /// here we know the plugin is remote by construction and never
    /// attempt the local resolution.
    RemoteNotCounted,

    /// The plugin is local but something about its directory or
    /// `plugin.json` prevented a skill count.
    ///
    /// `SkippedReason` is reused as the error payload to share the
    /// [`SkippedReason::from_plugin_error`] classifier. Reachable from
    /// this path:
    ///
    /// From the `MarketplaceService::resolve_local_plugin_dir` pre-check:
    /// - [`SkippedReason::DirectoryMissing`] — `plugin_dir` not found.
    /// - [`SkippedReason::NotADirectory`] — `plugin_dir` is a file.
    /// - [`SkippedReason::SymlinkRefused`] — `plugin_dir` is a symlink.
    /// - [`SkippedReason::DirectoryUnreadable`] — stat failed for any
    ///   other reason (permission denied, transient I/O, etc.).
    ///
    /// From the `plugin.json` load:
    /// - [`SkippedReason::InvalidManifest`] — `plugin.json` malformed.
    /// - [`SkippedReason::ManifestReadFailed`] — `plugin.json` read
    ///   failed after a successful stat.
    ///
    /// [`SkippedReason::NoSkills`] is not produced anywhere in this
    /// path; [`SkippedReason::RemoteSourceNotLocal`] is pre-empted by
    /// [`Self::RemoteNotCounted`] before resolution is attempted.
    /// Frontends typed against `SkippedReason` will not get
    /// compile-time narrowing for those two — accepted because
    /// consolidating the projection is more valuable than a narrower
    /// wire type.
    ManifestFailed { reason: SkippedReason },
}

/// Inputs that [`MarketplaceService::install_skills`] and
/// [`MarketplaceService::install_plugin_agents`] need for a single-plugin
/// install.
///
/// Constructed by [`MarketplaceService::resolve_plugin_install_context`]
/// (registry-driven) or
/// [`MarketplaceService::resolve_plugin_install_context_from_dir`]
/// (directory-driven, for fetch-aware CLI callers).
/// Rust-internal only — never crosses the FFI boundary, so no `Serialize`
/// or `specta::Type` derive. The type is `pub` so frontend handlers can
/// hold onto the resolved inputs between the context-resolution call and
/// the install call without pulling the preamble logic back into each
/// handler.
#[derive(Clone, Debug)]
pub struct PluginInstallContext {
    pub version: Option<String>,
    pub skill_dirs: Vec<PathBuf>,
    /// Directories to scan for agent `.md` files inside the plugin.
    /// Derived from `plugin.json`'s `agents` field, or
    /// [`crate::DEFAULT_AGENT_PATHS`] when the manifest is absent or
    /// declares no agents. Consumed by
    /// [`MarketplaceService::install_plugin_agents`].
    pub agent_scan_paths: Vec<String>,
    /// Directories to scan for steering `.md` files inside the plugin.
    /// Derived from `plugin.json`'s `steering` field, or
    /// [`crate::DEFAULT_STEERING_PATHS`] when the manifest is absent or
    /// declares no steering paths. Consumed by
    /// [`MarketplaceService::install_plugin_steering`].
    pub steering_scan_paths: Vec<String>,
    /// Authoring format declared by the plugin manifest. Drives dispatch
    /// in [`MarketplaceService::install_plugin_agents`]: `Some(KiroCli)`
    /// validates-and-copies native JSON; `None` (legacy) parses-and-translates
    /// markdown agents.
    pub format: Option<crate::plugin::PluginFormat>,
}

// ---------------------------------------------------------------------------
// Service methods
// ---------------------------------------------------------------------------

impl MarketplaceService {
    /// Resolve a plugin's on-disk location, local-only. Returns
    /// [`PluginError::RemoteSourceNotLocal`] for structured sources
    /// rather than cloning them — browse and list paths never want
    /// network I/O.
    ///
    /// Distinct from [`MarketplaceService::resolve_plugin_dir`], which
    /// clones remote sources on demand. Callers that can't tolerate a
    /// clone (enumerations, counts, read-only listings) use this
    /// method; callers that expect the directory to exist one way or
    /// another (install, update) use the cloning variant.
    ///
    /// # Errors
    ///
    /// - [`Error::Plugin`] ([`PluginError::DirectoryMissing`]) if a
    ///   `RelativePath` points to a missing directory.
    /// - [`Error::Plugin`] ([`PluginError::NotADirectory`]) if the path
    ///   exists but is a regular file (or other non-directory).
    /// - [`Error::Plugin`] ([`PluginError::SymlinkRefused`]) if the path
    ///   is a symlink — refused rather than followed as a security
    ///   measure.
    /// - [`Error::Plugin`] ([`PluginError::DirectoryUnreadable`]) if
    ///   stat'ing the path fails (permission denied, I/O error, etc.).
    /// - [`Error::Plugin`] ([`PluginError::RemoteSourceNotLocal`]) if
    ///   the source is structured (GitHub / Git URL / Git subdir).
    pub fn resolve_local_plugin_dir(
        &self,
        entry: &PluginEntry,
        marketplace_path: &Path,
    ) -> Result<PathBuf, Error> {
        match &entry.source {
            PluginSource::RelativePath(rel) => {
                // `rel` is a validated `RelativePath` — no traversal
                // check needed. `symlink_metadata` refuses to follow
                // symlinks, matching the hardening in
                // `resolve_plugin_dir`. Metadata outcomes split into
                // five arms: is_dir success, symlink → SymlinkRefused
                // (security refusal), non-directory → NotADirectory
                // (shape mismatch), NotFound → DirectoryMissing, and
                // other I/O → DirectoryUnreadable carrying the
                // underlying io::Error via #[source]. Splitting
                // NotFound from the catch-all ensures a permissions
                // problem surfaces as "could not access" with
                // ErrorKind preserved, not as a misleading "does not
                // exist."
                let resolved = marketplace_path.join(rel);
                match fs::symlink_metadata(&resolved) {
                    Ok(m) if m.file_type().is_symlink() => {
                        Err(PluginError::SymlinkRefused { path: resolved }.into())
                    }
                    Ok(m) if m.is_dir() => Ok(resolved),
                    Ok(_) => Err(PluginError::NotADirectory { path: resolved }.into()),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                        Err(PluginError::DirectoryMissing { path: resolved }.into())
                    }
                    Err(e) => Err(PluginError::DirectoryUnreadable {
                        path: resolved,
                        source: e,
                    }
                    .into()),
                }
            }
            PluginSource::Structured(s) => Err(PluginError::RemoteSourceNotLocal {
                plugin: entry.name.clone(),
                plugin_source: s.clone(),
            }
            .into()),
        }
    }

    /// List every skill defined by a single plugin, cross-referenced
    /// with the project's installed set.
    ///
    /// Per-skill errors inside a working plugin (unreadable `SKILL.md`,
    /// malformed frontmatter) land in [`PluginSkillsResult::skipped_skills`]
    /// so they surface structurally rather than vanishing into a `warn!`.
    /// A plugin-level error (missing directory, malformed manifest,
    /// remote source) propagates as `Err` — callers who selected this
    /// plugin explicitly should see a real error rather than an empty
    /// list.
    ///
    /// # Errors
    ///
    /// - [`Error::Marketplace`] / [`Error::Io`] from
    ///   [`Self::list_plugin_entries`] (unknown marketplace, corrupt or
    ///   unreadable registry).
    /// - [`Error::Plugin`] ([`PluginError::NotFound`]) if `plugin`
    ///   does not appear in the marketplace.
    /// - [`Error::Plugin`] ([`PluginError::DirectoryMissing`] /
    ///   [`PluginError::DirectoryUnreadable`] /
    ///   [`PluginError::InvalidManifest`] /
    ///   [`PluginError::ManifestReadFailed`] /
    ///   [`PluginError::RemoteSourceNotLocal`]) for plugin-level
    ///   resolution failures.
    pub fn list_skills_for_plugin(
        &self,
        marketplace: &str,
        plugin: &str,
        installed: &InstalledSkills,
    ) -> Result<PluginSkillsResult, Error> {
        let marketplace_path = self.marketplace_path(marketplace);
        let plugin_entries = self.list_plugin_entries(marketplace)?;

        let plugin_entry = plugin_entries
            .iter()
            .find(|p| p.name == plugin)
            .ok_or_else(|| {
                Error::Plugin(PluginError::NotFound {
                    plugin: plugin.to_owned(),
                    marketplace: marketplace.to_owned(),
                })
            })?;

        let mut skills: Vec<SkillInfo> = Vec::new();
        let mut skipped_skills: Vec<SkippedSkill> = Vec::new();
        collect_skills_for_plugin_into(
            self,
            plugin_entry,
            &marketplace_path,
            marketplace,
            installed,
            &mut skills,
            &mut skipped_skills,
        )?;
        Ok(PluginSkillsResult {
            skills,
            skipped_skills,
        })
    }

    /// List every skill across every plugin in a marketplace,
    /// cross-referenced with the project's installed set.
    ///
    /// Plugin-level errors (missing directory, malformed manifest,
    /// remote source) are folded into [`BulkSkillsResult::skipped`]
    /// so a single bad plugin doesn't hide its siblings. Per-skill
    /// errors inside a working plugin go to
    /// [`BulkSkillsResult::skipped_skills`], matching
    /// [`Self::list_skills_for_plugin`]'s contract.
    ///
    /// The `skills` and `skipped` vectors are pre-allocated with the
    /// plugin count as a baseline — `skills` usually grows past it
    /// (multiple skills per plugin) and `skipped` is bounded above
    /// by it, so this avoids the first few reallocations in the
    /// common case. `skipped_skills` stays at default capacity because
    /// the common case is zero per-skill failures; paying for an
    /// allocation that usually goes unused is the wrong default.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Marketplace`] / [`Error::Io`] from
    /// [`Self::list_plugin_entries`] when the marketplace is unknown
    /// or its registry is corrupt / unreadable. Non-plugin-level
    /// errors during iteration propagate; plugin-level errors
    /// (see [`SkippedReason::from_plugin_error`]) go to `skipped`.
    pub fn list_all_skills(
        &self,
        marketplace: &str,
        installed: &InstalledSkills,
    ) -> Result<BulkSkillsResult, Error> {
        let marketplace_path = self.marketplace_path(marketplace);
        let plugin_entries = self.list_plugin_entries(marketplace)?;

        let mut skills: Vec<SkillInfo> = Vec::with_capacity(plugin_entries.len());
        let mut skipped: Vec<SkippedPlugin> = Vec::with_capacity(plugin_entries.len());
        let mut skipped_skills: Vec<SkippedSkill> = Vec::new();

        for plugin_entry in &plugin_entries {
            match collect_skills_for_plugin_into(
                self,
                plugin_entry,
                &marketplace_path,
                marketplace,
                installed,
                &mut skills,
                &mut skipped_skills,
            ) {
                Ok(()) => {}
                Err(err) => {
                    match SkippedPlugin::from_plugin_error(plugin_entry.name.clone(), &err) {
                        Some(sp) => {
                            warn!(
                                plugin = %plugin_entry.name,
                                error = %sp.reason,
                                "skipping plugin in bulk skill listing"
                            );
                            skipped.push(sp);
                        }
                        None => return Err(err),
                    }
                }
            }
        }

        Ok(BulkSkillsResult {
            skills,
            skipped,
            skipped_skills,
        })
    }

    /// Count skills for a single plugin entry without loading skill bodies.
    ///
    /// Returns [`SkillCount::RemoteNotCounted`] for remote sources,
    /// [`SkillCount::ManifestFailed`] if the plugin directory or its
    /// `plugin.json` cannot be read or parsed, and [`SkillCount::Known`]
    /// otherwise (including the legitimate zero case where the manifest
    /// is absent or declares no skills).
    ///
    /// Takes the pre-resolved [`PluginEntry`] and `marketplace_path` so
    /// the batch caller in `list_plugins` pays the registry-parse cost
    /// once per marketplace rather than once per plugin. Errors are
    /// never propagated as `Err` — every outcome fits the three-way
    /// union.
    ///
    /// The plugin-directory pre-check delegates to
    /// [`Self::resolve_local_plugin_dir`] so the hardening (symlink
    /// refusal, `is_dir` check, `NotFound` / other-I/O classification) stays
    /// consistent with the bulk-listing path and does not duplicate.
    #[must_use]
    pub fn count_skills_for_plugin(
        &self,
        plugin: &PluginEntry,
        marketplace_path: &Path,
    ) -> SkillCount {
        // Short-circuit remote sources before `resolve_local_plugin_dir`
        // is called — it would return `PluginError::RemoteSourceNotLocal`
        // which we would then translate to `ManifestFailed`, conflating
        // "remote by design" with "should have been local but resolved
        // remote." The two need distinct UI states.
        if matches!(plugin.source, PluginSource::Structured(_)) {
            return SkillCount::RemoteNotCounted;
        }

        let plugin_dir = match self.resolve_local_plugin_dir(plugin, marketplace_path) {
            Ok(p) => p,
            Err(err) => {
                // Compute the intended path for defensive fallback logging;
                // `resolve_local_plugin_dir`'s success path would have
                // returned this value. For `Structured` sources we'd
                // never reach here (the remote short-circuit above
                // caught it), so the `rel` branch is the only case.
                let plugin_dir_hint = match &plugin.source {
                    PluginSource::RelativePath(rel) => marketplace_path.join(rel),
                    PluginSource::Structured(_) => marketplace_path.to_path_buf(),
                };
                return SkillCount::ManifestFailed {
                    reason: skipped_reason_from_resolve_error(&plugin.name, &plugin_dir_hint, err),
                };
            }
        };

        match load_plugin_manifest(&plugin_dir) {
            Ok(manifest) => {
                let count = discover_skills_for_plugin(&plugin_dir, manifest.as_ref()).len();
                let saturated = u32::try_from(count).unwrap_or_else(|_| {
                    warn!(
                        plugin = %plugin.name,
                        path = %plugin_dir.display(),
                        original = count,
                        "skill count exceeds u32::MAX; saturating"
                    );
                    u32::MAX
                });
                SkillCount::Known { count: saturated }
            }
            Err(err) => SkillCount::ManifestFailed {
                reason: skipped_reason_from_manifest_error(&plugin.name, &plugin_dir, err),
            },
        }
    }

    /// Resolve the inputs [`Self::install_skills`] needs for a single plugin.
    ///
    /// Performs the registry lookup, plugin-directory resolution,
    /// `plugin.json` load, and skill-directory enumeration that Tauri
    /// and CLI callers previously assembled by hand.
    ///
    /// # Errors
    ///
    /// - [`Error::Marketplace`] / [`Error::Io`] from
    ///   [`Self::list_plugin_entries`] (unknown marketplace, corrupt or
    ///   unreadable registry).
    /// - [`Error::Plugin`] ([`PluginError::NotFound`]) if `plugin` is not
    ///   in the marketplace.
    /// - [`Error::Plugin`] ([`PluginError::DirectoryMissing`] /
    ///   [`PluginError::NotADirectory`] / [`PluginError::SymlinkRefused`] /
    ///   [`PluginError::DirectoryUnreadable`] /
    ///   [`PluginError::RemoteSourceNotLocal`]) from
    ///   [`Self::resolve_local_plugin_dir`].
    /// - [`Error::Plugin`] ([`PluginError::InvalidManifest`] /
    ///   [`PluginError::ManifestReadFailed`]) from
    ///   [`Self::resolve_plugin_install_context_from_dir`] if `plugin.json`
    ///   is present but malformed or unreadable.
    ///
    /// All errors propagate rather than fold into a partial-success shape
    /// — the caller explicitly asked to install this plugin, so missing
    /// directories, malformed manifests, and remote sources are hard
    /// failures, not skips.
    pub fn resolve_plugin_install_context(
        &self,
        marketplace: &str,
        plugin: &str,
    ) -> Result<PluginInstallContext, Error> {
        let marketplace_path = self.marketplace_path(marketplace);
        let plugin_entries = self.list_plugin_entries(marketplace)?;
        let plugin_entry = plugin_entries
            .iter()
            .find(|p| p.name == plugin)
            .ok_or_else(|| {
                Error::Plugin(PluginError::NotFound {
                    plugin: plugin.to_owned(),
                    marketplace: marketplace.to_owned(),
                })
            })?;
        let plugin_dir = self.resolve_local_plugin_dir(plugin_entry, &marketplace_path)?;
        Self::resolve_plugin_install_context_from_dir(&plugin_dir)
    }

    /// Build a [`PluginInstallContext`] from an already-resolved plugin
    /// directory. Loads `plugin.json` (refusing symlinked manifests),
    /// enumerates skill directories, and derives agent-scan paths.
    ///
    /// Companion to [`Self::resolve_plugin_install_context`], which
    /// starts from a `(marketplace, plugin)` reference and drives a
    /// local-only resolution. This variant takes the directory as input,
    /// so callers that have already resolved `plugin_dir` by other means
    /// — including fetch-aware CLI callers that cloned a remote source
    /// first — can share the manifest-loading and path-discovery logic
    /// without re-entering the registry lookup.
    ///
    /// # Errors
    ///
    /// - [`Error::Plugin`] ([`PluginError::InvalidManifest`] /
    ///   [`PluginError::ManifestReadFailed`]) if `plugin.json` is
    ///   present but malformed or unreadable.
    pub fn resolve_plugin_install_context_from_dir(
        plugin_dir: &Path,
    ) -> Result<PluginInstallContext, Error> {
        let manifest = load_plugin_manifest(plugin_dir)?;
        let version = manifest.as_ref().and_then(|m| m.version.clone());
        let skill_dirs = discover_skills_for_plugin(plugin_dir, manifest.as_ref());
        let agent_scan_paths = agent_scan_paths_for_plugin(manifest.as_ref());
        let steering_scan_paths = steering_scan_paths_for_plugin(manifest.as_ref());
        let format = manifest.as_ref().and_then(|m| m.format);
        Ok(PluginInstallContext {
            version,
            skill_dirs,
            agent_scan_paths,
            steering_scan_paths,
            format,
        })
    }
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Append every readable skill defined by `plugin_entry` to `out`,
/// cross-referenced against `installed`. Plugin-level errors (missing
/// dir, malformed manifest, remote source) propagate as `Err`; per-skill
/// errors (unreadable `SKILL.md`, malformed frontmatter) are appended to
/// `skipped_skills` as structured [`SkippedSkill`] entries so the bulk
/// and per-plugin public entry points both surface them to the caller.
///
/// Shared between the per-plugin and bulk public entry points so the
/// per-skill skip philosophy and plugin-level error classification live
/// in exactly one place.
fn collect_skills_for_plugin_into(
    service: &MarketplaceService,
    plugin_entry: &PluginEntry,
    marketplace_path: &Path,
    marketplace_name: &str,
    installed: &InstalledSkills,
    out: &mut Vec<SkillInfo>,
    skipped_skills: &mut Vec<SkippedSkill>,
) -> Result<(), Error> {
    let plugin_dir = service.resolve_local_plugin_dir(plugin_entry, marketplace_path)?;
    let plugin_manifest = load_plugin_manifest(&plugin_dir)?;
    let skill_dirs = discover_skills_for_plugin(&plugin_dir, plugin_manifest.as_ref());
    out.reserve(skill_dirs.len());

    for skill_dir in &skill_dirs {
        let skill_md_path = skill_dir.join("SKILL.md");
        let content = match fs::read_to_string(&skill_md_path) {
            Ok(c) => c,
            Err(e) => {
                warn!(
                    marketplace = %marketplace_name,
                    plugin = %plugin_entry.name,
                    path = %skill_md_path.display(),
                    error = %e,
                    "failed to read SKILL.md, skipping"
                );
                skipped_skills.push(SkippedSkill {
                    plugin: plugin_entry.name.clone(),
                    name_hint: name_hint_from_skill_dir(skill_dir),
                    path: skill_md_path,
                    reason: SkippedSkillReason::ReadFailed {
                        reason: error_full_chain(&e),
                    },
                });
                continue;
            }
        };

        let (frontmatter, _body_offset) = match parse_frontmatter(&content) {
            Ok(result) => result,
            Err(e) => {
                warn!(
                    marketplace = %marketplace_name,
                    plugin = %plugin_entry.name,
                    path = %skill_md_path.display(),
                    error = %e,
                    "failed to parse SKILL.md frontmatter, skipping"
                );
                skipped_skills.push(SkippedSkill {
                    plugin: plugin_entry.name.clone(),
                    name_hint: name_hint_from_skill_dir(skill_dir),
                    path: skill_md_path,
                    reason: SkippedSkillReason::FrontmatterInvalid {
                        reason: error_full_chain(&e),
                    },
                });
                continue;
            }
        };

        let is_installed = installed.skills.contains_key(&frontmatter.name);
        out.push(SkillInfo {
            name: frontmatter.name,
            description: frontmatter.description,
            plugin: plugin_entry.name.clone(),
            marketplace: marketplace_name.to_owned(),
            installed: is_installed,
        });
    }

    Ok(())
}

/// Best-effort label for a skill whose real (frontmatter) name is
/// unreachable — used as [`SkippedSkill::name_hint`]. Returns `None`
/// when [`Path::file_name`] cannot extract a final component (degenerate
/// inputs: empty path, root `/`, or a path terminating in `..`); in
/// practice `skill_dir` always comes from [`discover_skill_dirs`] so
/// the `None` arm is defensive rather than expected.
///
/// `pub(crate)` so the install path in [`super::MarketplaceService::install_skills`]
/// can populate [`SkippedSkill::name_hint`] consistently with the
/// listing path — the two codepaths used to both reach for
/// `skill_dir.file_name()` inline; sharing the helper means a future
/// tweak (e.g. normalising Unicode) lands once.
pub(crate) fn name_hint_from_skill_dir(skill_dir: &Path) -> Option<String> {
    skill_dir
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
}

/// Resolve the skill-discovery paths for a plugin. Uses
/// `manifest.skills` when the manifest specifies any, otherwise falls
/// back to [`crate::DEFAULT_SKILL_PATHS`]. The manifest-empty-list case
/// also falls back — an empty `skills` field means "no custom paths",
/// not "no skills."
fn discover_skills_for_plugin(
    plugin_dir: &Path,
    manifest: Option<&PluginManifest>,
) -> Vec<PathBuf> {
    let skill_paths: Vec<&str> = if let Some(m) = manifest.filter(|m| !m.skills.is_empty()) {
        m.skills.iter().map(String::as_str).collect()
    } else {
        crate::DEFAULT_SKILL_PATHS.to_vec()
    };

    discover_skill_dirs(plugin_dir, &skill_paths)
}

/// Resolve the list of agent-scan paths a plugin declares, falling
/// back to [`crate::DEFAULT_AGENT_PATHS`] when the manifest is absent
/// or its `agents` list is empty. Mirrors the "empty list means no
/// custom paths, not no agents" fallback policy used by
/// [`discover_skills_for_plugin`].
fn agent_scan_paths_for_plugin(manifest: Option<&PluginManifest>) -> Vec<String> {
    if let Some(m) = manifest.filter(|m| !m.agents.is_empty()) {
        m.agents.clone()
    } else {
        crate::DEFAULT_AGENT_PATHS
            .iter()
            .map(|s| (*s).to_string())
            .collect()
    }
}

/// Resolve the list of steering-scan paths a plugin declares, falling
/// back to [`crate::DEFAULT_STEERING_PATHS`] when the manifest is absent
/// or its `steering` list is empty. Mirrors
/// [`agent_scan_paths_for_plugin`].
fn steering_scan_paths_for_plugin(manifest: Option<&PluginManifest>) -> Vec<String> {
    if let Some(m) = manifest.filter(|m| !m.steering.is_empty()) {
        m.steering.clone()
    } else {
        crate::DEFAULT_STEERING_PATHS
            .iter()
            .map(|s| (*s).to_string())
            .collect()
    }
}

/// Project a `resolve_local_plugin_dir` error into a [`SkippedReason`].
///
/// `resolve_local_plugin_dir` only returns [`PluginError`] variants
/// that [`SkippedReason::from_plugin_error`] classifies as
/// plugin-level skips (`DirectoryMissing`, `NotADirectory`,
/// `SymlinkRefused`, `DirectoryUnreadable`, plus
/// `RemoteSourceNotLocal` — pre-empted at the caller). The defensive
/// `unwrap_or_else` branch exists for forward-compatibility: if a
/// future `PluginError` variant lands and the classifier returns
/// `None`, we fold it into `DirectoryUnreadable` with an `error!`
/// (a missing classification is a code defect, not a runtime warning)
/// rather than regress to a silent `0`.
///
/// `plugin_dir_hint` is the intended plugin directory the caller would
/// have resolved on the success path; it is used to populate the
/// `DirectoryUnreadable.path` field in the defensive fallbacks so the
/// UI can render something more informative than an empty path.
fn skipped_reason_from_resolve_error(
    plugin_name: &str,
    plugin_dir_hint: &Path,
    err: Error,
) -> SkippedReason {
    let Error::Plugin(pe) = err else {
        // `resolve_local_plugin_dir` only returns `Error::Plugin` today,
        // but `Error` is `#[non_exhaustive]` — defensive.
        warn!(
            plugin = %plugin_name,
            error = %error_full_chain(&err),
            "unexpected non-plugin error resolving plugin_dir; reporting as DirectoryUnreadable"
        );
        return SkippedReason::DirectoryUnreadable {
            path: plugin_dir_hint.to_path_buf(),
            reason: error_full_chain(&err),
        };
    };
    SkippedReason::from_plugin_error(&pe).unwrap_or_else(|| {
        error!(
            plugin = %plugin_name,
            error = ?pe,
            "unclassified PluginError from resolve_local_plugin_dir; reporting as DirectoryUnreadable"
        );
        SkippedReason::DirectoryUnreadable {
            path: plugin_dir_hint.to_path_buf(),
            reason: error_full_chain(&pe),
        }
    })
}

/// Project a `load_plugin_manifest` error into a [`SkippedReason`].
///
/// `load_plugin_manifest` returns [`PluginError::InvalidManifest`] or
/// [`PluginError::ManifestReadFailed`] today. Same defensive pattern
/// as [`skipped_reason_from_resolve_error`]: an unclassified variant
/// folds into `ManifestReadFailed` with an `error!` — a missing
/// classification indicates a new `PluginError` variant was added
/// without a corresponding branch in `SkippedReason::from_plugin_error`.
fn skipped_reason_from_manifest_error(
    plugin_name: &str,
    plugin_dir: &Path,
    err: Error,
) -> SkippedReason {
    let Error::Plugin(pe) = err else {
        warn!(
            plugin = %plugin_name,
            error = %error_full_chain(&err),
            "unexpected non-plugin error loading plugin.json; reporting as ManifestReadFailed"
        );
        return SkippedReason::ManifestReadFailed {
            path: plugin_dir.join("plugin.json"),
            reason: error_full_chain(&err),
        };
    };
    SkippedReason::from_plugin_error(&pe).unwrap_or_else(|| {
        error!(
            plugin = %plugin_name,
            error = ?pe,
            "unclassified PluginError from load_plugin_manifest; reporting as ManifestReadFailed"
        );
        SkippedReason::ManifestReadFailed {
            path: plugin_dir.join("plugin.json"),
            reason: error_full_chain(&pe),
        }
    })
}

/// Load a `plugin.json` from the given directory.
///
/// Returns:
/// - `Ok(Some(manifest))` on success.
/// - `Ok(None)` when the file is genuinely absent (`NotFound`) or when
///   it is a symlink — a symlinked `plugin.json` inside an untrusted
///   cloned repository could point at arbitrary host files, so it is
///   treated as absent with a `warn!`.
/// - `Err(PluginError::InvalidManifest)` if the file exists but could
///   not be parsed.
/// - `Err(PluginError::ManifestReadFailed)` for any other read or stat
///   failure (permission denied, transient I/O, etc.). Classified as
///   plugin-level so bulk listings skip the plugin rather than aborting.
fn load_plugin_manifest(plugin_dir: &Path) -> Result<Option<PluginManifest>, Error> {
    let manifest_path = plugin_dir.join("plugin.json");

    // Refuse to follow symlinks. plugin_dir lives inside a cloned
    // (untrusted) repository; a symlinked plugin.json could leak host
    // file contents through the InvalidManifest error path's `reason`
    // field (which includes serde's parse error over the target bytes).
    match fs::symlink_metadata(&manifest_path) {
        Ok(m) if m.file_type().is_symlink() => {
            warn!(
                path = %manifest_path.display(),
                "plugin.json is a symlink, refusing to follow; treating as missing"
            );
            return Ok(None);
        }
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            debug!(
                path = %manifest_path.display(),
                "plugin.json not found, using defaults"
            );
            return Ok(None);
        }
        Err(e) => {
            warn!(
                path = %manifest_path.display(),
                error = %e,
                "failed to stat plugin.json"
            );
            return Err(PluginError::ManifestReadFailed {
                path: manifest_path,
                source: e,
            }
            .into());
        }
    }

    let bytes = match fs::read(&manifest_path) {
        Ok(b) => b,
        Err(e) => {
            warn!(
                path = %manifest_path.display(),
                error = %e,
                "failed to read plugin.json"
            );
            return Err(PluginError::ManifestReadFailed {
                path: manifest_path,
                source: e,
            }
            .into());
        }
    };

    match PluginManifest::from_json(&bytes) {
        Ok(manifest) => {
            debug!(name = %manifest.name, "loaded plugin manifest");
            Ok(Some(manifest))
        }
        Err(e) => {
            warn!(
                path = %manifest_path.display(),
                error = %e,
                "plugin.json is malformed"
            );
            Err(PluginError::InvalidManifest {
                path: manifest_path,
                reason: error_full_chain(&e),
            }
            .into())
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::path::Path;

    #[cfg(unix)]
    use tempfile::tempdir;

    use super::*;
    use crate::marketplace::{PluginSource, StructuredSource};
    use crate::service::test_support::{
        make_plugin_with_skills, relative_path_entry, seed_marketplace_with_registry, temp_service,
    };

    /// Build an `io::Error` whose Custom repr wraps a two-link error chain.
    /// Regression tests that observe chain preservation need depth beyond
    /// the `io::Error`'s top-level Display — `io::Error::from(ErrorKind)`
    /// alone has no source, so `source.to_string()` and
    /// `error_full_chain(source)` would produce identical output and
    /// render the tests tautological. The returned error has Display
    /// = `outer_msg` and `source().to_string()` = `inner_msg`.
    fn chained_io_error(
        kind: std::io::ErrorKind,
        outer_msg: &'static str,
        inner_msg: &'static str,
    ) -> std::io::Error {
        use std::error::Error as StdError;
        use std::fmt;

        #[derive(Debug)]
        struct Inner(&'static str);
        impl fmt::Display for Inner {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.0)
            }
        }
        impl StdError for Inner {}

        #[derive(Debug)]
        struct Outer {
            display: &'static str,
            source: Inner,
        }
        impl fmt::Display for Outer {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.display)
            }
        }
        impl StdError for Outer {
            fn source(&self) -> Option<&(dyn StdError + 'static)> {
                Some(&self.source)
            }
        }

        std::io::Error::new(
            kind,
            Outer {
                display: outer_msg,
                source: Inner(inner_msg),
            },
        )
    }

    // -----------------------------------------------------------------------
    // resolve_local_plugin_dir
    // -----------------------------------------------------------------------

    #[test]
    fn resolve_local_plugin_dir_relative_path_exists() {
        let (dir, svc) = temp_service();
        let marketplace_path = dir.path().join("marketplace");
        let plugin_dir = marketplace_path.join("plugins/my-plugin");
        fs::create_dir_all(&plugin_dir).expect("create plugin dir");

        let entry = relative_path_entry("my-plugin", "plugins/my-plugin");
        let resolved = svc
            .resolve_local_plugin_dir(&entry, &marketplace_path)
            .expect("happy path");
        assert_eq!(resolved, plugin_dir);
    }

    #[test]
    fn resolve_local_plugin_dir_missing_returns_directory_missing() {
        let (dir, svc) = temp_service();
        let marketplace_path = dir.path().join("marketplace");
        fs::create_dir_all(&marketplace_path).expect("create marketplace root");

        let entry = relative_path_entry("ghost", "plugins/ghost");
        let err = svc
            .resolve_local_plugin_dir(&entry, &marketplace_path)
            .expect_err("missing dir must error");
        assert!(
            matches!(err, Error::Plugin(PluginError::DirectoryMissing { .. })),
            "expected DirectoryMissing, got: {err:?}"
        );
    }

    #[test]
    fn resolve_local_plugin_dir_structured_returns_remote_source_not_local() {
        let (dir, svc) = temp_service();
        let marketplace_path = dir.path().join("marketplace");

        let entry = PluginEntry {
            name: "remote".into(),
            description: None,
            source: PluginSource::Structured(StructuredSource::GitHub {
                repo: "owner/repo".into(),
                git_ref: None,
                sha: None,
            }),
        };

        let err = svc
            .resolve_local_plugin_dir(&entry, &marketplace_path)
            .expect_err("structured source must refuse local resolution");
        assert!(
            matches!(err, Error::Plugin(PluginError::RemoteSourceNotLocal { .. })),
            "expected RemoteSourceNotLocal, got: {err:?}"
        );
    }

    // -----------------------------------------------------------------------
    // collect_skills_for_plugin_into (helper-level tests)
    // -----------------------------------------------------------------------

    #[test]
    fn collect_skills_for_plugin_into_happy_path() {
        let (dir, svc) = temp_service();
        make_plugin_with_skills(dir.path(), "good", &["alpha", "beta"]);
        let entry = relative_path_entry("good", "plugins/good");

        let mut out: Vec<SkillInfo> = Vec::new();
        let mut skipped_skills: Vec<SkippedSkill> = Vec::new();
        let installed = InstalledSkills::default();
        collect_skills_for_plugin_into(
            &svc,
            &entry,
            dir.path(),
            "mp1",
            &installed,
            &mut out,
            &mut skipped_skills,
        )
        .expect("happy path");

        assert_eq!(out.len(), 2);
        assert!(out.iter().any(|s| s.name == "alpha"));
        assert!(out.iter().any(|s| s.name == "beta"));
        assert!(
            out.iter()
                .all(|s| s.plugin == "good" && s.marketplace == "mp1")
        );
        assert!(out.iter().all(|s| !s.installed));
        assert!(
            skipped_skills.is_empty(),
            "happy path must not skip any skills, got: {skipped_skills:?}"
        );
    }

    #[test]
    fn collect_skills_for_plugin_into_missing_dir_errors() {
        let (dir, svc) = temp_service();
        let entry = relative_path_entry("ghost", "plugins/ghost");

        let mut out: Vec<SkillInfo> = Vec::new();
        let mut skipped_skills: Vec<SkippedSkill> = Vec::new();
        let installed = InstalledSkills::default();
        let err = collect_skills_for_plugin_into(
            &svc,
            &entry,
            dir.path(),
            "mp1",
            &installed,
            &mut out,
            &mut skipped_skills,
        )
        .expect_err("missing dir must propagate");

        assert!(
            matches!(err, Error::Plugin(PluginError::DirectoryMissing { .. })),
            "expected DirectoryMissing, got: {err:?}"
        );
        assert!(out.is_empty());
        assert!(skipped_skills.is_empty());
    }

    #[test]
    fn collect_skills_for_plugin_into_malformed_manifest_errors() {
        let (dir, svc) = temp_service();
        let plugin_dir = dir.path().join("plugins").join("broken");
        fs::create_dir_all(&plugin_dir).expect("create plugin dir");
        fs::write(plugin_dir.join("plugin.json"), "{ not valid json").expect("write manifest");
        let entry = relative_path_entry("broken", "plugins/broken");

        let mut out: Vec<SkillInfo> = Vec::new();
        let mut skipped_skills: Vec<SkippedSkill> = Vec::new();
        let installed = InstalledSkills::default();
        let err = collect_skills_for_plugin_into(
            &svc,
            &entry,
            dir.path(),
            "mp1",
            &installed,
            &mut out,
            &mut skipped_skills,
        )
        .expect_err("malformed manifest must propagate");

        assert!(
            matches!(err, Error::Plugin(PluginError::InvalidManifest { .. })),
            "expected InvalidManifest, got: {err:?}"
        );
        assert!(out.is_empty());
        assert!(skipped_skills.is_empty());
    }

    #[test]
    fn collect_skills_for_plugin_into_surfaces_bad_frontmatter_as_skipped_skill() {
        let (dir, svc) = temp_service();
        let skills_dir = dir.path().join("plugins").join("mixed").join("skills");
        fs::create_dir_all(skills_dir.join("good-skill")).expect("create skill dir");
        fs::create_dir_all(skills_dir.join("bad-skill")).expect("create skill dir");
        fs::write(
            skills_dir.join("good-skill").join("SKILL.md"),
            "---\nname: good-skill\ndescription: works\n---\n",
        )
        .expect("write good skill");
        // Missing closing `---` makes frontmatter parsing fail.
        fs::write(
            skills_dir.join("bad-skill").join("SKILL.md"),
            "---\nname: bad\n",
        )
        .expect("write bad skill");
        let entry = relative_path_entry("mixed", "plugins/mixed");

        let mut out: Vec<SkillInfo> = Vec::new();
        let mut skipped_skills: Vec<SkippedSkill> = Vec::new();
        let installed = InstalledSkills::default();
        collect_skills_for_plugin_into(
            &svc,
            &entry,
            dir.path(),
            "mp1",
            &installed,
            &mut out,
            &mut skipped_skills,
        )
        .expect("per-skill errors should not propagate");

        // Regression guard: previously the bad frontmatter vanished into
        // a warn! log. Now it must surface as a structured SkippedSkill.
        assert_eq!(out.len(), 1, "bad frontmatter should not be in skills");
        assert_eq!(out[0].name, "good-skill");
        assert_eq!(skipped_skills.len(), 1, "bad frontmatter must be skipped");
        assert_eq!(skipped_skills[0].name_hint.as_deref(), Some("bad-skill"));
        assert_eq!(
            skipped_skills[0].plugin, "mixed",
            "per-skill skips must carry plugin attribution so bulk callers \
             can group failures by plugin"
        );
        assert!(
            matches!(
                skipped_skills[0].reason,
                SkippedSkillReason::FrontmatterInvalid { .. }
            ),
            "expected FrontmatterInvalid, got: {:?}",
            skipped_skills[0].reason
        );
    }

    // -----------------------------------------------------------------------
    // list_skills_for_plugin (public API integration)
    // -----------------------------------------------------------------------

    #[test]
    fn list_skills_for_plugin_unknown_marketplace_errors() {
        let (_dir, svc) = temp_service();
        let installed = InstalledSkills::default();
        let err = svc
            .list_skills_for_plugin("does-not-exist", "foo", &installed)
            .expect_err("unknown marketplace must error");

        // MarketplaceError::NotFound or similar — the exact variant is
        // an implementation detail of list_plugin_entries; we only
        // guarantee the top-level Error::Marketplace shape here.
        assert!(
            matches!(err, Error::Marketplace(_)),
            "expected Error::Marketplace, got: {err:?}"
        );
    }

    // -----------------------------------------------------------------------
    // list_all_skills (bulk public API)
    // -----------------------------------------------------------------------

    #[test]
    fn list_all_skills_happy_path_enumerates_across_plugins() {
        let (dir, svc) = temp_service();
        let entries = vec![
            relative_path_entry("alpha-plug", "plugins/alpha-plug"),
            relative_path_entry("beta-plug", "plugins/beta-plug"),
        ];
        let marketplace_path = seed_marketplace_with_registry(dir.path(), &svc, "mp1", &entries);
        make_plugin_with_skills(&marketplace_path, "alpha-plug", &["skill-a1", "skill-a2"]);
        make_plugin_with_skills(&marketplace_path, "beta-plug", &["skill-b1"]);

        let installed = InstalledSkills::default();
        let result = svc.list_all_skills("mp1", &installed).expect("happy path");

        assert_eq!(result.skills.len(), 3);
        assert!(result.skipped.is_empty());
        assert!(result.skills.iter().any(|s| s.name == "skill-a1"));
        assert!(result.skills.iter().any(|s| s.name == "skill-a2"));
        assert!(result.skills.iter().any(|s| s.name == "skill-b1"));
    }

    #[test]
    fn list_all_skills_skips_one_broken_plugin_keeps_the_rest() {
        let (dir, svc) = temp_service();
        let entries = vec![
            relative_path_entry("good", "plugins/good"),
            relative_path_entry("ghost", "plugins/ghost"),
        ];
        let marketplace_path = seed_marketplace_with_registry(dir.path(), &svc, "mp1", &entries);
        make_plugin_with_skills(&marketplace_path, "good", &["alpha"]);
        // Deliberately do not create `plugins/ghost` — it must land in
        // `skipped` rather than cause the whole bulk call to fail.

        let installed = InstalledSkills::default();
        let result = svc
            .list_all_skills("mp1", &installed)
            .expect("bulk call must succeed despite one broken plugin");

        assert_eq!(result.skills.len(), 1);
        assert_eq!(result.skills[0].name, "alpha");
        assert_eq!(result.skipped.len(), 1);
        assert_eq!(result.skipped[0].name, "ghost");
        assert!(
            result.skipped[0].reason.contains("does not exist"),
            "skipped reason should name the failure mode, got: {}",
            result.skipped[0].reason
        );
    }

    // -----------------------------------------------------------------------
    // Symlink-refusal regression tests (plugin dir + plugin.json)
    // -----------------------------------------------------------------------

    /// Regression guard: `resolve_local_plugin_dir` uses
    /// `symlink_metadata` combined with an explicit `is_symlink()`
    /// check rather than `Path::exists()`, so a symlink at the plugin
    /// path is classified as [`PluginError::SymlinkRefused`] rather
    /// than traversed. This test fails if the symlink arm is replaced
    /// by `Path::exists()` (which would follow the link) or by a
    /// weaker shape check (which would let the symlink fall through
    /// to [`PluginError::NotADirectory`] and hide the security
    /// semantic). Mirrors `resolve_plugin_dir_refuses_symlinked_relative_path`
    /// for the cloning sibling in `service/mod.rs`.
    #[cfg(unix)]
    #[test]
    fn resolve_local_plugin_dir_refuses_symlinked_relative_path() {
        let (dir, svc) = temp_service();
        let marketplace_path = dir.path().join("marketplace");
        fs::create_dir_all(&marketplace_path).expect("create marketplace root");

        let outside = dir.path().join("outside-marketplace");
        fs::create_dir_all(&outside).expect("create outside target");

        let link_path = marketplace_path.join("plugins").join("escape");
        fs::create_dir_all(link_path.parent().expect("plugins dir parent"))
            .expect("create plugins dir");
        std::os::unix::fs::symlink(&outside, &link_path).expect("create symlink");

        let entry = relative_path_entry("escape", "plugins/escape");
        let err = svc
            .resolve_local_plugin_dir(&entry, &marketplace_path)
            .expect_err("symlinked plugin directory must be refused");
        assert!(
            matches!(err, Error::Plugin(PluginError::SymlinkRefused { .. })),
            "expected SymlinkRefused for symlink, got: {err:?}"
        );
    }

    /// Regression guard: `load_plugin_manifest` treats a symlinked
    /// `plugin.json` as absent. A symlinked manifest inside a cloned
    /// repo could leak host file contents through the `InvalidManifest`
    /// error path, which embeds the serde parse error over the target
    /// bytes.
    #[cfg(unix)]
    #[test]
    fn load_plugin_manifest_refuses_symlinked_manifest() {
        let tmp = tempdir().expect("tempdir");
        let plugin_dir = tmp.path().join("plugin");
        fs::create_dir_all(&plugin_dir).expect("create plugin dir");

        // A "sensitive" target with valid-looking JSON so we can tell
        // absence from "parsed but wrong."
        let sensitive = tmp.path().join("secrets.json");
        fs::write(&sensitive, br#"{"name":"leaked","version":"1.0"}"#).expect("write target");

        std::os::unix::fs::symlink(&sensitive, plugin_dir.join("plugin.json"))
            .expect("create symlink");

        let result = load_plugin_manifest(&plugin_dir).expect("symlink must be Ok(None)");
        assert!(
            result.is_none(),
            "symlinked plugin.json must be treated as absent, got: {result:?}"
        );
    }

    // -----------------------------------------------------------------------
    // resolve_local_plugin_dir: Unreadable vs Missing classification
    // -----------------------------------------------------------------------

    /// Regression guard: a regular file sitting at the plugin path
    /// must classify as [`PluginError::NotADirectory`] rather than
    /// [`PluginError::DirectoryMissing`] (which would mislead users
    /// into thinking the path is absent) or
    /// [`PluginError::DirectoryUnreadable`] (which implies an I/O
    /// failure and loses the structural semantic). Pins the four-way
    /// split on `resolve_local_plugin_dir`.
    #[test]
    fn resolve_local_plugin_dir_file_path_returns_not_a_directory() {
        let (dir, svc) = temp_service();
        let marketplace_path = dir.path().join("marketplace");
        fs::create_dir_all(marketplace_path.join("plugins")).expect("create plugins dir");
        fs::write(
            marketplace_path.join("plugins").join("not-a-dir"),
            b"this is a regular file",
        )
        .expect("write file");

        let entry = relative_path_entry("not-a-dir", "plugins/not-a-dir");
        let err = svc
            .resolve_local_plugin_dir(&entry, &marketplace_path)
            .expect_err("regular file must not resolve as a plugin directory");
        assert!(
            matches!(err, Error::Plugin(PluginError::NotADirectory { .. })),
            "expected NotADirectory for non-directory, got: {err:?}"
        );
    }

    // -----------------------------------------------------------------------
    // SkippedPlugin::from_plugin_error covers the new plugin-level variants
    // -----------------------------------------------------------------------

    /// Regression guard: the bulk path relies on this classifier to
    /// decide skip-vs-propagate. Before the fix, `ManifestReadFailed`
    /// propagated as an `Error::Io` that slipped past the `matches!`,
    /// aborting the entire listing on one unreadable `plugin.json`.
    #[rstest::rstest]
    #[case::directory_missing(Error::Plugin(PluginError::DirectoryMissing {
        path: "/tmp/x".into(),
    }))]
    #[case::not_a_directory(Error::Plugin(PluginError::NotADirectory {
        path: "/tmp/x".into(),
    }))]
    #[case::symlink_refused(Error::Plugin(PluginError::SymlinkRefused {
        path: "/tmp/x".into(),
    }))]
    #[case::directory_unreadable(Error::Plugin(PluginError::DirectoryUnreadable {
        path: "/tmp/x".into(),
        source: std::io::Error::from(std::io::ErrorKind::PermissionDenied),
    }))]
    #[case::invalid_manifest(Error::Plugin(PluginError::InvalidManifest {
        path: "/tmp/x/plugin.json".into(),
        reason: "missing name".into(),
    }))]
    #[case::manifest_read_failed(Error::Plugin(PluginError::ManifestReadFailed {
        path: "/tmp/x/plugin.json".into(),
        source: std::io::Error::from(std::io::ErrorKind::PermissionDenied),
    }))]
    #[case::remote_source_not_local(Error::Plugin(PluginError::RemoteSourceNotLocal {
        plugin: "remote-plug".into(),
        plugin_source: StructuredSource::GitHub {
            repo: "owner/repo".into(),
            git_ref: None,
            sha: None,
        },
    }))]
    #[case::no_skills(Error::Plugin(PluginError::NoSkills {
        name: "empty-plug".into(),
        path: "/tmp/x".into(),
    }))]
    fn skipped_plugin_from_plugin_error_accepts_plugin_level_variants(#[case] err: Error) {
        let sp = SkippedPlugin::from_plugin_error("test-plug".into(), &err);
        assert!(sp.is_some(), "expected bulk-path skip for: {err:?}");
        let sp = sp.expect("just checked");
        assert_eq!(
            sp.name, "test-plug",
            "constructor must preserve the name argument"
        );
        assert_eq!(
            sp.reason,
            error_full_chain(&err),
            "reason must equal the full source chain so `io::Error` \
             details behind `#[source]` survive the Tauri FFI boundary — \
             `err.to_string()` would strip them"
        );
    }

    /// Regression guard for the `err.to_string()` → `error_full_chain(err)`
    /// fix on `SkippedPlugin::from_plugin_error`. Before the fix, this
    /// field lost `io::Error` detail at the Tauri FFI boundary for any
    /// `PluginError` variant that carried `#[source]`. Using
    /// `DirectoryUnreadable` here because it's the simplest variant with
    /// a non-trivial source chain — the Display says only "could not
    /// access plugin directory at {path}" with no mention of the
    /// underlying `io::ErrorKind`, so the chain walk is load-bearing.
    ///
    /// Explicitly asserts the source detail appears in `reason`;
    /// `err.to_string()` would fail this assertion because its output
    /// stops at the top-level Display.
    #[test]
    fn skipped_plugin_reason_preserves_io_source_detail_from_chain() {
        let err = Error::Plugin(PluginError::DirectoryUnreadable {
            path: PathBuf::from("/tmp/plugins/locked"),
            source: std::io::Error::new(std::io::ErrorKind::PermissionDenied, "forbidden zone"),
        });
        let sp = SkippedPlugin::from_plugin_error("locked-plug".into(), &err)
            .expect("DirectoryUnreadable is plugin-level, must construct");

        assert!(
            sp.reason.contains("could not access plugin directory"),
            "reason must include the variant's Display text, got: {}",
            sp.reason
        );
        assert!(
            sp.reason.contains("forbidden zone"),
            "reason must include the io::Error's message from the \
             source chain (regression guard against `err.to_string()`), \
             got: {}",
            sp.reason
        );
    }

    /// Regression guard for the `source.to_string()` → `error_full_chain(source)`
    /// fix on `SkippedReason::from_plugin_error`. Sibling to the
    /// `SkippedPlugin::from_plugin_error` test above, but pinning the
    /// inner projection (which builds the wire-format `reason` string
    /// directly from the `#[source]` `io::Error`, not through the outer
    /// `Error::Plugin` wrapping). Before the fix, deeper causes wrapped
    /// inside an `io::Error` were dropped.
    ///
    /// Uses [`chained_io_error`] to construct an `io::Error` with
    /// observable chain depth. A bare `io::Error::from(ErrorKind)` has
    /// no source and would make `source.to_string()` and
    /// `error_full_chain(source)` produce identical output, rendering
    /// this test tautological.
    #[test]
    fn skipped_reason_directory_unreadable_preserves_io_source_chain() {
        let plugin_err = PluginError::DirectoryUnreadable {
            path: PathBuf::from("/tmp/plugins/locked"),
            source: chained_io_error(
                std::io::ErrorKind::PermissionDenied,
                "forbidden zone",
                "deep cause from filesystem driver",
            ),
        };

        let Some(SkippedReason::DirectoryUnreadable { reason, .. }) =
            SkippedReason::from_plugin_error(&plugin_err)
        else {
            panic!("DirectoryUnreadable must classify as skip");
        };

        assert!(
            reason.contains("forbidden zone"),
            "reason must include io::Error top-level Display, got: {reason}"
        );
        assert!(
            reason.contains("deep cause from filesystem driver"),
            "reason must include deeper source chain via error_full_chain, got: {reason}"
        );
        assert!(
            reason.contains(": deep cause from filesystem driver"),
            "chain segments must be joined by `: `, got: {reason}"
        );
    }

    /// Sibling of the `DirectoryUnreadable` regression test — same
    /// chain-preservation contract on `ManifestReadFailed`. The two
    /// `source.to_string()` sites were patched together; keeping the
    /// tests paired so future divergence fails both, not one.
    #[test]
    fn skipped_reason_manifest_read_failed_preserves_io_source_chain() {
        let plugin_err = PluginError::ManifestReadFailed {
            path: PathBuf::from("/tmp/plugins/corrupt/plugin.json"),
            source: chained_io_error(
                std::io::ErrorKind::Other,
                "read failure",
                "deep cause from parser layer",
            ),
        };

        let Some(SkippedReason::ManifestReadFailed { reason, .. }) =
            SkippedReason::from_plugin_error(&plugin_err)
        else {
            panic!("ManifestReadFailed must classify as skip");
        };

        assert!(
            reason.contains("read failure"),
            "reason must include io::Error top-level Display, got: {reason}"
        );
        assert!(
            reason.contains("deep cause from parser layer"),
            "reason must include deeper source chain via error_full_chain, got: {reason}"
        );
        assert!(
            reason.contains(": deep cause from parser layer"),
            "chain segments must be joined by `: `, got: {reason}"
        );
    }

    #[test]
    fn skipped_plugin_from_plugin_error_rejects_non_plugin_errors() {
        let io_err = Error::Io(std::io::Error::other("disk full"));
        assert!(
            SkippedPlugin::from_plugin_error("x".into(), &io_err).is_none(),
            "generic I/O errors must propagate, not skip"
        );
    }

    /// Regression guard: [`PluginError::NotFound`] represents a caller
    /// asking for a plugin the marketplace doesn't list — a user-input
    /// bug, not a damaged plugin. It must propagate rather than fold
    /// into `skipped`, or bulk listings would silently hide lookup
    /// errors too.
    #[test]
    fn skipped_plugin_from_plugin_error_rejects_plugin_not_found() {
        let err = Error::Plugin(PluginError::NotFound {
            plugin: "ghost".into(),
            marketplace: "mp1".into(),
        });
        assert!(
            SkippedPlugin::from_plugin_error("x".into(), &err).is_none(),
            "NotFound must propagate, not skip (it's a caller bug, not a broken plugin)"
        );
    }

    // -----------------------------------------------------------------------
    // list_skills_for_plugin: happy path + NotFound branch + installed
    // -----------------------------------------------------------------------

    /// Single installed-skill fixture so the cross-reference branch
    /// `installed.skills.contains_key(&frontmatter.name) == true` gets
    /// exercised. All production `SkillInfo.installed` consumers depend
    /// on this being correct; historically every test used
    /// `InstalledSkills::default()`, so only the `false` branch was
    /// covered.
    fn installed_with(skill_name: &str, plugin: &str, marketplace: &str) -> InstalledSkills {
        use std::collections::HashMap;

        use chrono::Utc;

        use crate::project::InstalledSkillMeta;

        let mut skills = HashMap::new();
        skills.insert(
            skill_name.to_owned(),
            InstalledSkillMeta {
                marketplace: marketplace.to_owned(),
                plugin: plugin.to_owned(),
                version: None,
                installed_at: Utc::now(),
                source_hash: None,
                installed_hash: None,
            },
        );
        InstalledSkills { skills }
    }

    #[test]
    fn list_skills_for_plugin_happy_path() {
        let (dir, svc) = temp_service();
        let entries = vec![relative_path_entry("alpha", "plugins/alpha")];
        let marketplace_path = seed_marketplace_with_registry(dir.path(), &svc, "mp1", &entries);
        make_plugin_with_skills(&marketplace_path, "alpha", &["skill-a"]);

        let installed = InstalledSkills::default();
        let result = svc
            .list_skills_for_plugin("mp1", "alpha", &installed)
            .expect("happy path");

        assert_eq!(result.skills.len(), 1);
        assert_eq!(result.skills[0].name, "skill-a");
        assert_eq!(result.skills[0].plugin, "alpha");
        assert_eq!(result.skills[0].marketplace, "mp1");
        assert!(!result.skills[0].installed);
        assert!(result.skipped_skills.is_empty());
    }

    #[test]
    fn list_skills_for_plugin_unknown_plugin_errors() {
        let (dir, svc) = temp_service();
        let entries = vec![relative_path_entry("alpha", "plugins/alpha")];
        let marketplace_path = seed_marketplace_with_registry(dir.path(), &svc, "mp1", &entries);
        make_plugin_with_skills(&marketplace_path, "alpha", &["skill-a"]);

        let installed = InstalledSkills::default();
        let err = svc
            .list_skills_for_plugin("mp1", "does-not-exist", &installed)
            .expect_err("unknown plugin must error");

        assert!(
            matches!(
                err,
                Error::Plugin(PluginError::NotFound { ref plugin, .. })
                    if plugin == "does-not-exist"
            ),
            "expected PluginError::NotFound, got: {err:?}"
        );
    }

    #[test]
    fn list_skills_for_plugin_marks_installed_skills_true() {
        let (dir, svc) = temp_service();
        let entries = vec![relative_path_entry("alpha", "plugins/alpha")];
        let marketplace_path = seed_marketplace_with_registry(dir.path(), &svc, "mp1", &entries);
        make_plugin_with_skills(&marketplace_path, "alpha", &["already-installed", "fresh"]);

        let installed = installed_with("already-installed", "alpha", "mp1");
        let result = svc
            .list_skills_for_plugin("mp1", "alpha", &installed)
            .expect("happy path");

        let marked_installed: Vec<_> = result.skills.iter().filter(|s| s.installed).collect();
        assert_eq!(marked_installed.len(), 1);
        assert_eq!(marked_installed[0].name, "already-installed");
        assert!(
            result
                .skills
                .iter()
                .any(|s| s.name == "fresh" && !s.installed),
            "fresh skill should not be marked installed"
        );
    }

    // -----------------------------------------------------------------------
    // list_all_skills: additional skip branches + installed cross-ref
    // -----------------------------------------------------------------------

    /// Bulk path must fold a plugin with an unparseable `plugin.json`
    /// into `skipped`. Previously only the `DirectoryMissing` skip
    /// branch was covered; a narrowed classifier could pass CI without
    /// this.
    #[test]
    fn list_all_skills_skips_plugin_with_invalid_manifest() {
        let (dir, svc) = temp_service();
        let entries = vec![
            relative_path_entry("good", "plugins/good"),
            relative_path_entry("broken", "plugins/broken"),
        ];
        let marketplace_path = seed_marketplace_with_registry(dir.path(), &svc, "mp1", &entries);
        make_plugin_with_skills(&marketplace_path, "good", &["alpha"]);
        let broken_dir = marketplace_path.join("plugins").join("broken");
        fs::create_dir_all(&broken_dir).expect("create broken plugin dir");
        fs::write(broken_dir.join("plugin.json"), "{ not valid json")
            .expect("write malformed manifest");

        let installed = InstalledSkills::default();
        let result = svc
            .list_all_skills("mp1", &installed)
            .expect("bulk call must succeed with one broken plugin");

        assert_eq!(result.skills.len(), 1);
        assert_eq!(result.skills[0].name, "alpha");
        assert_eq!(result.skipped.len(), 1);
        assert_eq!(result.skipped[0].name, "broken");
        // The structured `kind` is the programmatic contract; the
        // `reason` Display-string is a human-readable convenience and
        // may rephrase freely. Previously this test substring-matched
        // on `reason` before the structured SkippedReason existed; the
        // `matches!` below survives any Display rewording.
        assert!(
            matches!(
                result.skipped[0].kind,
                SkippedReason::InvalidManifest { .. }
            ),
            "skipped kind should be InvalidManifest, got: {:?}",
            result.skipped[0].kind
        );
    }

    /// Bulk path must fold a plugin whose source is remote into
    /// `skipped`, not propagate. Without this, listing a marketplace
    /// that mixes local and remote plugins would abort on the first
    /// remote entry.
    #[test]
    fn list_all_skills_skips_plugin_with_remote_source() {
        let (dir, svc) = temp_service();
        let local = relative_path_entry("local", "plugins/local");
        let remote = PluginEntry {
            name: "remote".into(),
            description: None,
            source: PluginSource::Structured(StructuredSource::GitHub {
                repo: "owner/repo".into(),
                git_ref: None,
                sha: None,
            }),
        };
        let marketplace_path =
            seed_marketplace_with_registry(dir.path(), &svc, "mp1", &[local, remote]);
        make_plugin_with_skills(&marketplace_path, "local", &["local-skill"]);

        let installed = InstalledSkills::default();
        let result = svc
            .list_all_skills("mp1", &installed)
            .expect("bulk call must succeed with one remote plugin");

        assert_eq!(result.skills.len(), 1);
        assert_eq!(result.skills[0].name, "local-skill");
        assert_eq!(result.skipped.len(), 1);
        assert_eq!(result.skipped[0].name, "remote");
        // Structured `kind` doubles as a guard that the embedded
        // `StructuredSource` payload made it through the Error →
        // SkippedReason projection. If `plugin_source` ever got dropped
        // from the projection, the `source: StructuredSource::GitHub`
        // arm below would fail to match.
        match &result.skipped[0].kind {
            SkippedReason::RemoteSourceNotLocal { plugin, source } => {
                assert_eq!(plugin, "remote");
                assert!(
                    matches!(
                        source,
                        StructuredSource::GitHub { repo, .. } if repo == "owner/repo"
                    ),
                    "expected GitHub source round-tripped through projection, got: {source:?}"
                );
            }
            other => panic!("expected SkippedReason::RemoteSourceNotLocal, got: {other:?}"),
        }
    }

    /// Regression guard: a regular file sitting at the plugin path must
    /// fold into `skipped` with `kind = NotADirectory`, not propagate
    /// or mis-classify as `DirectoryMissing`. Previously this class was
    /// only covered at the `resolve_local_plugin_dir` unit layer; the
    /// e2e assertion catches a regression where the bulk loop gets
    /// narrowed to a subset of plugin-level variants.
    #[test]
    fn list_all_skills_skips_plugin_with_regular_file_at_path() {
        let (dir, svc) = temp_service();
        let entries = vec![
            relative_path_entry("good", "plugins/good"),
            relative_path_entry("not-a-dir", "plugins/not-a-dir"),
        ];
        let marketplace_path = seed_marketplace_with_registry(dir.path(), &svc, "mp1", &entries);
        make_plugin_with_skills(&marketplace_path, "good", &["alpha"]);
        // A regular file where the plugin directory should be.
        fs::create_dir_all(marketplace_path.join("plugins")).expect("plugins dir");
        fs::write(
            marketplace_path.join("plugins").join("not-a-dir"),
            b"file, not a directory",
        )
        .expect("write blocker file");

        let installed = InstalledSkills::default();
        let result = svc
            .list_all_skills("mp1", &installed)
            .expect("bulk call must succeed past the misshapen plugin path");

        assert_eq!(result.skills.len(), 1);
        assert_eq!(result.skipped.len(), 1);
        assert_eq!(result.skipped[0].name, "not-a-dir");
        assert!(
            matches!(result.skipped[0].kind, SkippedReason::NotADirectory { .. }),
            "expected NotADirectory, got: {:?}",
            result.skipped[0].kind
        );
    }

    /// Regression guard: a symlink at the plugin path must classify as
    /// `SymlinkRefused` end-to-end. Previously only the
    /// `resolve_local_plugin_dir` unit test covered this; the bulk
    /// classifier could regress silently (symlink falls through to a
    /// different variant).
    #[cfg(unix)]
    #[test]
    fn list_all_skills_skips_plugin_with_symlinked_path() {
        let (dir, svc) = temp_service();
        let entries = vec![
            relative_path_entry("good", "plugins/good"),
            relative_path_entry("escape", "plugins/escape"),
        ];
        let marketplace_path = seed_marketplace_with_registry(dir.path(), &svc, "mp1", &entries);
        make_plugin_with_skills(&marketplace_path, "good", &["alpha"]);
        // Create a symlink at plugins/escape pointing outside the
        // marketplace tree. The service must refuse to follow it.
        let outside = dir.path().join("outside");
        fs::create_dir_all(&outside).expect("create outside");
        fs::create_dir_all(marketplace_path.join("plugins")).expect("plugins dir");
        std::os::unix::fs::symlink(&outside, marketplace_path.join("plugins").join("escape"))
            .expect("create symlink");

        let installed = InstalledSkills::default();
        let result = svc
            .list_all_skills("mp1", &installed)
            .expect("bulk call must succeed past the symlinked plugin path");

        assert_eq!(result.skills.len(), 1);
        assert_eq!(result.skipped.len(), 1);
        assert_eq!(result.skipped[0].name, "escape");
        assert!(
            matches!(result.skipped[0].kind, SkippedReason::SymlinkRefused { .. }),
            "expected SymlinkRefused, got: {:?}",
            result.skipped[0].kind
        );
    }

    /// Regression guard: a plugin directory whose stat fails (e.g.
    /// chmod 000 on the parent) must classify as `DirectoryUnreadable`
    /// rather than `DirectoryMissing`. The distinction matters for UI
    /// remediation: "permission denied" is a different user action
    /// from "directory deleted."
    #[cfg(unix)]
    #[test]
    fn list_all_skills_skips_plugin_with_unreadable_directory() {
        use std::os::unix::fs::PermissionsExt;

        let (dir, svc) = temp_service();
        let entries = vec![
            relative_path_entry("good", "plugins/good"),
            relative_path_entry("locked", "plugins/locked"),
        ];
        let marketplace_path = seed_marketplace_with_registry(dir.path(), &svc, "mp1", &entries);
        make_plugin_with_skills(&marketplace_path, "good", &["alpha"]);
        // Create `plugins/locked/` as a directory, then chmod its
        // PARENT (`plugins/`) so stat'ing `plugins/locked` fails with
        // EACCES rather than ENOENT. Chmodding the leaf itself would
        // let stat succeed (read-bit on parent is what syscalls need).
        let plugins_dir = marketplace_path.join("plugins");
        fs::create_dir_all(plugins_dir.join("locked")).expect("create locked plugin");
        fs::set_permissions(&plugins_dir, fs::Permissions::from_mode(0o000))
            .expect("chmod 000 on plugins dir");

        let installed = InstalledSkills::default();
        let result = svc.list_all_skills("mp1", &installed);
        // Restore permissions BEFORE assertions so tempdir cleanup can
        // delete the directory even if an assertion panics.
        fs::set_permissions(&plugins_dir, fs::Permissions::from_mode(0o755))
            .expect("restore perms");

        let result = result.expect("bulk call must succeed past the unreadable plugin path");

        // With the plugins/ directory chmod-0'd, the "good" plugin
        // ALSO becomes unstat-able — both plugins land in `skipped`
        // with DirectoryUnreadable. That's the correct behavior: the
        // classifier preserves semantics over every plugin-level arm,
        // not just the one we're targeting.
        assert_eq!(
            result.skipped.len(),
            2,
            "both plugins should be unreadable when their parent is chmod-0"
        );
        for sp in &result.skipped {
            assert!(
                matches!(sp.kind, SkippedReason::DirectoryUnreadable { .. }),
                "expected DirectoryUnreadable for plugin `{}`, got: {:?}",
                sp.name,
                sp.kind
            );
        }
    }

    /// Previously, a single malformed `SKILL.md` inside an otherwise-
    /// working plugin vanished into `warn!` + `continue`, leaving the
    /// frontend to guess why the skill count shrank. The bulk path now
    /// folds it into [`BulkSkillsResult::skipped_skills`] as a
    /// structured entry.
    #[test]
    fn list_all_skills_surfaces_malformed_skill_as_skipped_skill() {
        let (dir, svc) = temp_service();
        let entries = vec![relative_path_entry("mixed", "plugins/mixed")];
        let marketplace_path = seed_marketplace_with_registry(dir.path(), &svc, "mp1", &entries);
        let skills_dir = marketplace_path
            .join("plugins")
            .join("mixed")
            .join("skills");
        fs::create_dir_all(skills_dir.join("ok")).expect("create ok skill dir");
        fs::create_dir_all(skills_dir.join("malformed")).expect("create malformed skill dir");
        fs::write(
            skills_dir.join("ok").join("SKILL.md"),
            "---\nname: ok\ndescription: works\n---\n",
        )
        .expect("write ok skill");
        // Missing closing `---`: frontmatter parser fails.
        fs::write(
            skills_dir.join("malformed").join("SKILL.md"),
            "---\nname: malformed\n",
        )
        .expect("write malformed skill");

        let installed = InstalledSkills::default();
        let result = svc.list_all_skills("mp1", &installed).expect("bulk ok");

        assert_eq!(result.skills.len(), 1);
        assert_eq!(result.skills[0].name, "ok");
        assert!(
            result.skipped.is_empty(),
            "plugin-level skipped must be empty when only a skill is bad"
        );
        assert_eq!(result.skipped_skills.len(), 1);
        assert_eq!(
            result.skipped_skills[0].name_hint.as_deref(),
            Some("malformed")
        );
        assert_eq!(
            result.skipped_skills[0].plugin, "mixed",
            "bulk-path per-skill skips must carry plugin attribution so \
             the frontend can group by plugin"
        );
        assert!(
            matches!(
                result.skipped_skills[0].reason,
                SkippedSkillReason::FrontmatterInvalid { .. }
            ),
            "expected FrontmatterInvalid, got: {:?}",
            result.skipped_skills[0].reason
        );
    }

    /// Regression guard (Unix-only because chmod is the tool): an
    /// unreadable `SKILL.md` must surface via
    /// [`SkippedSkillReason::ReadFailed`] rather than vanish. On Windows
    /// the equivalent case (access-denied via ACLs) exists but isn't
    /// exercised here — we have UNIX coverage for the classification.
    #[cfg(unix)]
    #[test]
    fn list_all_skills_surfaces_unreadable_skill_md_as_skipped_skill() {
        use std::os::unix::fs::PermissionsExt;

        let (dir, svc) = temp_service();
        let entries = vec![relative_path_entry("locked", "plugins/locked")];
        let marketplace_path = seed_marketplace_with_registry(dir.path(), &svc, "mp1", &entries);
        let skill_dir = marketplace_path
            .join("plugins")
            .join("locked")
            .join("skills")
            .join("vault");
        fs::create_dir_all(&skill_dir).expect("create skill dir");
        let skill_md = skill_dir.join("SKILL.md");
        fs::write(&skill_md, "---\nname: vault\ndescription: locked\n---\n")
            .expect("write SKILL.md");
        // Remove all permissions so read_to_string fails with EACCES.
        fs::set_permissions(&skill_md, fs::Permissions::from_mode(0o000))
            .expect("chmod 000 SKILL.md");

        let installed = InstalledSkills::default();
        let result = svc.list_all_skills("mp1", &installed).expect("bulk ok");
        // Restore permissions so tempdir cleanup can delete the file.
        // Placed before assertions so a panic still cleans up.
        fs::set_permissions(&skill_md, fs::Permissions::from_mode(0o644)).expect("restore perms");

        assert!(
            result.skills.is_empty(),
            "no skills should land in happy path, got: {:?}",
            result.skills
        );
        assert_eq!(result.skipped_skills.len(), 1);
        assert_eq!(result.skipped_skills[0].name_hint.as_deref(), Some("vault"));
        assert!(
            matches!(
                result.skipped_skills[0].reason,
                SkippedSkillReason::ReadFailed { .. }
            ),
            "expected ReadFailed, got: {:?}",
            result.skipped_skills[0].reason
        );
    }

    #[test]
    fn list_all_skills_marks_installed_skills_true() {
        let (dir, svc) = temp_service();
        let entries = vec![relative_path_entry("alpha", "plugins/alpha")];
        let marketplace_path = seed_marketplace_with_registry(dir.path(), &svc, "mp1", &entries);
        make_plugin_with_skills(&marketplace_path, "alpha", &["installed", "fresh"]);

        let installed = installed_with("installed", "alpha", "mp1");
        let result = svc.list_all_skills("mp1", &installed).expect("happy path");

        let marked: Vec<_> = result.skills.iter().filter(|s| s.installed).collect();
        assert_eq!(marked.len(), 1);
        assert_eq!(marked[0].name, "installed");
    }

    // -----------------------------------------------------------------------
    // SkippedReason / SkippedSkillReason wire-format regression guards
    // -----------------------------------------------------------------------
    //
    // These types cross the Tauri FFI and land in TypeScript via specta, so
    // their JSON shape is a public contract — a silent serde-tag rename or
    // variant-casing change would ripple into a frontend parse error that
    // fires at runtime, not compile time. Pin the exact wire representation
    // here so any such change surfaces as a failing unit test in this crate
    // before a bindings.ts regeneration ever reaches the UI.

    /// Regression guard: `name_hint_from_skill_dir` must return `None`
    /// (not an empty string) for degenerate paths so the
    /// `SkippedSkill.name_hint: Option<String>` contract is honored end
    /// to end. Before this was an `Option` it was a sentinel empty
    /// string and the two cases were indistinguishable at the wire.
    #[test]
    fn name_hint_from_skill_dir_returns_none_for_degenerate_paths() {
        assert_eq!(name_hint_from_skill_dir(Path::new("")), None);
        assert_eq!(name_hint_from_skill_dir(Path::new("foo/..")), None);
        // A normal skill directory yields Some(file_name).
        assert_eq!(
            name_hint_from_skill_dir(Path::new("/plugins/acme/skills/alpha")).as_deref(),
            Some("alpha")
        );
    }

    /// Wire-format pin for every path-shaped `SkippedReason` variant
    /// (the five that carry only `path`, plus the three that add a
    /// `reason` string). `RemoteSourceNotLocal` is covered by a
    /// dedicated test because it embeds a `StructuredSource` payload
    /// that needs its own round-trip guard.
    ///
    /// Parametric to keep the cost of adding a future path-shaped
    /// variant low — one new `#[case]` line pins its JSON shape.
    #[rstest::rstest]
    #[case::directory_missing(
        SkippedReason::DirectoryMissing { path: PathBuf::from("/tmp/plugins/ghost") },
        serde_json::json!({ "kind": "directory_missing", "path": "/tmp/plugins/ghost" })
    )]
    #[case::not_a_directory(
        SkippedReason::NotADirectory { path: PathBuf::from("/tmp/plugins/file") },
        serde_json::json!({ "kind": "not_a_directory", "path": "/tmp/plugins/file" })
    )]
    #[case::symlink_refused(
        SkippedReason::SymlinkRefused { path: PathBuf::from("/tmp/plugins/link") },
        serde_json::json!({ "kind": "symlink_refused", "path": "/tmp/plugins/link" })
    )]
    #[case::directory_unreadable(
        SkippedReason::DirectoryUnreadable {
            path: PathBuf::from("/tmp/plugins/noaccess"),
            reason: "permission denied".into(),
        },
        serde_json::json!({
            "kind": "directory_unreadable",
            "path": "/tmp/plugins/noaccess",
            "reason": "permission denied",
        })
    )]
    #[case::invalid_manifest(
        SkippedReason::InvalidManifest {
            path: PathBuf::from("/tmp/plugins/x/plugin.json"),
            reason: "missing name".into(),
        },
        serde_json::json!({
            "kind": "invalid_manifest",
            "path": "/tmp/plugins/x/plugin.json",
            "reason": "missing name",
        })
    )]
    #[case::manifest_read_failed(
        SkippedReason::ManifestReadFailed {
            path: PathBuf::from("/tmp/plugins/x/plugin.json"),
            reason: "permission denied".into(),
        },
        serde_json::json!({
            "kind": "manifest_read_failed",
            "path": "/tmp/plugins/x/plugin.json",
            "reason": "permission denied",
        })
    )]
    #[case::no_skills(
        SkippedReason::NoSkills { path: PathBuf::from("/tmp/plugins/empty") },
        serde_json::json!({ "kind": "no_skills", "path": "/tmp/plugins/empty" })
    )]
    fn skipped_reason_path_variants_json_shape(
        #[case] reason: SkippedReason,
        #[case] expected: serde_json::Value,
    ) {
        let json = serde_json::to_value(&reason).expect("serialize");
        assert_eq!(json, expected);
    }

    #[test]
    fn skipped_reason_remote_source_not_local_embeds_structured_source() {
        let reason = SkippedReason::RemoteSourceNotLocal {
            plugin: "acme".into(),
            source: StructuredSource::GitHub {
                repo: "owner/repo".into(),
                git_ref: Some("main".into()),
                sha: None,
            },
        };
        let json = serde_json::to_value(&reason).expect("serialize");
        assert_eq!(
            json,
            serde_json::json!({
                "kind": "remote_source_not_local",
                "plugin": "acme",
                "source": {
                    "source": "github",
                    "repo": "owner/repo",
                    "ref": "main",
                    "sha": null,
                },
            }),
            "StructuredSource must round-trip via its existing serde tag \
             (`source`) inside the SkippedReason envelope"
        );
    }

    #[test]
    fn skipped_skill_reason_json_shapes() {
        let read_failed = SkippedSkillReason::ReadFailed {
            reason: "permission denied".into(),
        };
        assert_eq!(
            serde_json::to_value(&read_failed).expect("serialize"),
            serde_json::json!({
                "kind": "read_failed",
                "reason": "permission denied",
            })
        );

        let frontmatter_invalid = SkippedSkillReason::FrontmatterInvalid {
            reason: "missing closing ---".into(),
        };
        assert_eq!(
            serde_json::to_value(&frontmatter_invalid).expect("serialize"),
            serde_json::json!({
                "kind": "frontmatter_invalid",
                "reason": "missing closing ---",
            })
        );
    }

    // -----------------------------------------------------------------------
    // SkillCount wire-format pins
    // -----------------------------------------------------------------------

    #[test]
    fn skill_count_serde_known_wire_format() {
        let json = serde_json::to_value(SkillCount::Known { count: 7 }).expect("serialize");
        assert_eq!(json, serde_json::json!({"state": "known", "count": 7}));
    }

    #[test]
    fn skill_count_serde_remote_not_counted_wire_format() {
        let json = serde_json::to_value(SkillCount::RemoteNotCounted).expect("serialize");
        assert_eq!(json, serde_json::json!({"state": "remote_not_counted"}));
    }

    #[test]
    fn skill_count_serde_manifest_failed_wire_format() {
        let sc = SkillCount::ManifestFailed {
            reason: SkippedReason::InvalidManifest {
                path: std::path::PathBuf::from("/tmp/plug/plugin.json"),
                reason: "expected `}`".into(),
            },
        };
        let json = serde_json::to_value(sc).expect("serialize");
        assert_eq!(json["state"], "manifest_failed");
        assert_eq!(json["reason"]["kind"], "invalid_manifest");
        assert_eq!(json["reason"]["path"], "/tmp/plug/plugin.json");
        assert_eq!(json["reason"]["reason"], "expected `}`");
    }

    // -----------------------------------------------------------------------
    // count_skills_for_plugin
    // -----------------------------------------------------------------------

    #[test]
    fn count_skills_for_plugin_returns_known_for_local_plugin() {
        let (dir, svc) = temp_service();
        let marketplace_path = dir.path().join("marketplace");
        make_plugin_with_skills(&marketplace_path, "my-plugin", &["alpha", "beta", "gamma"]);

        let entry = relative_path_entry("my-plugin", "plugins/my-plugin");
        let result = svc.count_skills_for_plugin(&entry, &marketplace_path);
        assert!(
            matches!(result, SkillCount::Known { count: 3 }),
            "expected Known {{ count: 3 }}, got: {result:?}"
        );
    }

    #[test]
    fn count_skills_for_plugin_returns_known_with_zero_when_no_skills() {
        let (dir, svc) = temp_service();
        let marketplace_path = dir.path().join("marketplace");
        let plugin_dir = marketplace_path.join("plugins/lonely");
        fs::create_dir_all(&plugin_dir).expect("create plugin dir");
        // A plugin.json with no custom skill paths → default paths apply,
        // but no skills/ directory exists, so count is 0.
        fs::write(
            plugin_dir.join("plugin.json"),
            r#"{"name": "lonely", "version": "0.0.0"}"#,
        )
        .expect("write plugin.json");

        let entry = relative_path_entry("lonely", "plugins/lonely");
        let result = svc.count_skills_for_plugin(&entry, &marketplace_path);
        assert!(
            matches!(result, SkillCount::Known { count: 0 }),
            "expected Known {{ count: 0 }}, got: {result:?}"
        );
    }

    #[test]
    fn count_skills_for_plugin_returns_known_when_manifest_absent() {
        let (dir, svc) = temp_service();
        let marketplace_path = dir.path().join("marketplace");
        // No plugin.json → defaults kick in. make_plugin_with_skills creates
        // skills/ subdirs (not plugin.json), so the count comes from those
        // subdirs alone.
        make_plugin_with_skills(&marketplace_path, "defaults", &["alpha", "beta"]);

        let entry = relative_path_entry("defaults", "plugins/defaults");
        let result = svc.count_skills_for_plugin(&entry, &marketplace_path);
        assert!(
            matches!(result, SkillCount::Known { count: 2 }),
            "expected Known {{ count: 2 }}, got: {result:?}"
        );
    }

    #[test]
    fn count_skills_for_plugin_returns_remote_for_structured_source() {
        let (_dir, svc) = temp_service();
        let marketplace_path = Path::new("/tmp/nonexistent-marketplace");

        let entry = PluginEntry {
            name: "remote".into(),
            description: None,
            source: PluginSource::Structured(StructuredSource::GitHub {
                repo: "owner/repo".into(),
                git_ref: None,
                sha: None,
            }),
        };

        let result = svc.count_skills_for_plugin(&entry, marketplace_path);
        assert!(
            matches!(result, SkillCount::RemoteNotCounted),
            "expected RemoteNotCounted, got: {result:?}"
        );
    }

    #[test]
    fn count_skills_for_plugin_returns_manifest_failed_on_missing_plugin_dir() {
        let (dir, svc) = temp_service();
        let marketplace_path = dir.path().join("marketplace");
        fs::create_dir_all(&marketplace_path).expect("create marketplace root");

        let entry = relative_path_entry("ghost", "plugins/ghost");
        let result = svc.count_skills_for_plugin(&entry, &marketplace_path);
        assert!(
            matches!(
                result,
                SkillCount::ManifestFailed {
                    reason: SkippedReason::DirectoryMissing { .. }
                }
            ),
            "expected ManifestFailed/DirectoryMissing, got: {result:?}"
        );
    }

    #[test]
    fn count_skills_for_plugin_returns_manifest_failed_when_plugin_dir_is_a_file() {
        let (dir, svc) = temp_service();
        let marketplace_path = dir.path().join("marketplace");
        let plugins_root = marketplace_path.join("plugins");
        fs::create_dir_all(&plugins_root).expect("create plugins root");
        // Create a regular file where the plugin dir should be.
        fs::write(plugins_root.join("not-a-dir"), b"i am a file").expect("write file");

        let entry = relative_path_entry("not-a-dir", "plugins/not-a-dir");
        let result = svc.count_skills_for_plugin(&entry, &marketplace_path);
        assert!(
            matches!(
                result,
                SkillCount::ManifestFailed {
                    reason: SkippedReason::NotADirectory { .. }
                }
            ),
            "expected ManifestFailed/NotADirectory, got: {result:?}"
        );
    }

    #[test]
    #[cfg(unix)]
    fn count_skills_for_plugin_returns_manifest_failed_on_symlinked_plugin_dir() {
        use std::os::unix::fs::symlink;

        let (dir, svc) = temp_service();
        let marketplace_path = dir.path().join("marketplace");
        let plugins_root = marketplace_path.join("plugins");
        fs::create_dir_all(&plugins_root).expect("create plugins root");
        // Symlink target must exist so the symlink itself is what triggers
        // the refusal, not a broken-symlink variant.
        let real_target = dir.path().join("real-plugin");
        fs::create_dir_all(&real_target).expect("create real target");
        symlink(&real_target, plugins_root.join("symlinked")).expect("create symlink");

        let entry = relative_path_entry("symlinked", "plugins/symlinked");
        let result = svc.count_skills_for_plugin(&entry, &marketplace_path);
        assert!(
            matches!(
                result,
                SkillCount::ManifestFailed {
                    reason: SkippedReason::SymlinkRefused { .. }
                }
            ),
            "expected ManifestFailed/SymlinkRefused, got: {result:?}"
        );
    }

    #[test]
    fn count_skills_for_plugin_returns_manifest_failed_on_malformed_json() {
        let (dir, svc) = temp_service();
        let marketplace_path = dir.path().join("marketplace");
        let plugin_dir = marketplace_path.join("plugins/broken");
        fs::create_dir_all(&plugin_dir).expect("create plugin dir");
        fs::write(plugin_dir.join("plugin.json"), b"{not json").expect("write plugin.json");

        let entry = relative_path_entry("broken", "plugins/broken");
        let result = svc.count_skills_for_plugin(&entry, &marketplace_path);
        assert!(
            matches!(
                result,
                SkillCount::ManifestFailed {
                    reason: SkippedReason::InvalidManifest { .. }
                }
            ),
            "expected ManifestFailed/InvalidManifest, got: {result:?}"
        );
    }

    #[test]
    fn count_skills_for_plugin_returns_manifest_failed_when_plugin_json_is_a_directory() {
        let (dir, svc) = temp_service();
        let marketplace_path = dir.path().join("marketplace");
        let plugin_dir = marketplace_path.join("plugins/json-is-a-dir");
        fs::create_dir_all(&plugin_dir).expect("create plugin dir");
        // Create `plugin.json` as a directory (not a regular file). stat
        // succeeds (not a symlink, not NotFound), so load_plugin_manifest
        // proceeds to fs::read which fails with ErrorKind::IsADirectory →
        // PluginError::ManifestReadFailed. Pins the ManifestReadFailed
        // branch portably without requiring chmod or root-awareness.
        fs::create_dir(plugin_dir.join("plugin.json")).expect("create plugin.json as dir");

        let entry = relative_path_entry("json-is-a-dir", "plugins/json-is-a-dir");
        let result = svc.count_skills_for_plugin(&entry, &marketplace_path);
        assert!(
            matches!(
                result,
                SkillCount::ManifestFailed {
                    reason: SkippedReason::ManifestReadFailed { .. }
                }
            ),
            "expected ManifestFailed/ManifestReadFailed, got: {result:?}"
        );
    }

    #[test]
    #[cfg(unix)]
    fn count_skills_for_plugin_treats_symlinked_plugin_json_as_missing() {
        use std::os::unix::fs::symlink;

        let (dir, svc) = temp_service();
        let marketplace_path = dir.path().join("marketplace");
        let plugin_dir = marketplace_path.join("plugins/symjson");
        fs::create_dir_all(&plugin_dir).expect("create plugin dir");
        // Symlinked plugin.json is treated as absent by load_plugin_manifest
        // (security hardening; see its symlink_metadata branch). That means
        // we fall back to default skill paths — no skills/ dir exists here,
        // so count is 0. Regression pin for this specific interaction.
        let real_manifest = dir.path().join("real-plugin.json");
        fs::write(&real_manifest, b"{\"name\":\"irrelevant\"}").expect("write real manifest");
        symlink(&real_manifest, plugin_dir.join("plugin.json")).expect("create symlink");

        let entry = relative_path_entry("symjson", "plugins/symjson");
        let result = svc.count_skills_for_plugin(&entry, &marketplace_path);
        assert!(
            matches!(result, SkillCount::Known { count: 0 }),
            "expected Known {{ count: 0 }}, got: {result:?}"
        );
    }

    // -----------------------------------------------------------------------
    // resolve_plugin_install_context
    // -----------------------------------------------------------------------

    #[test]
    fn resolve_plugin_install_context_returns_context_for_local_plugin() {
        let (dir, svc) = temp_service();
        let entries = vec![relative_path_entry("myplugin", "plugins/myplugin")];
        let marketplace_path = seed_marketplace_with_registry(dir.path(), &svc, "mp1", &entries);
        make_plugin_with_skills(&marketplace_path, "myplugin", &["alpha", "beta", "gamma"]);
        fs::write(
            marketplace_path
                .join("plugins")
                .join("myplugin")
                .join("plugin.json"),
            br#"{"name": "myplugin", "version": "1.2.3"}"#,
        )
        .expect("write plugin.json");

        let ctx = svc
            .resolve_plugin_install_context("mp1", "myplugin")
            .expect("happy path");
        assert_eq!(ctx.version.as_deref(), Some("1.2.3"));
        let mut names: Vec<String> = ctx
            .skill_dirs
            .iter()
            .map(|p| {
                p.file_name()
                    .and_then(|s| s.to_str())
                    .expect("skill dir has valid UTF-8 name")
                    .to_string()
            })
            .collect();
        names.sort();
        assert_eq!(
            names,
            vec!["alpha".to_string(), "beta".to_string(), "gamma".to_string()]
        );
        assert!(
            ctx.skill_dirs.iter().all(|p| p.join("SKILL.md").is_file()),
            "every skill dir must contain a SKILL.md: {:?}",
            ctx.skill_dirs
        );
    }

    #[test]
    fn resolve_plugin_install_context_returns_empty_skill_dirs_when_no_skills() {
        let (dir, svc) = temp_service();
        let entries = vec![relative_path_entry("lonely", "plugins/lonely")];
        let marketplace_path = seed_marketplace_with_registry(dir.path(), &svc, "mp1", &entries);
        let plugin_dir = marketplace_path.join("plugins").join("lonely");
        fs::create_dir_all(&plugin_dir).expect("create plugin dir");
        fs::write(
            plugin_dir.join("plugin.json"),
            br#"{"name": "lonely", "version": "0.1.0"}"#,
        )
        .expect("write plugin.json");

        let ctx = svc
            .resolve_plugin_install_context("mp1", "lonely")
            .expect("happy path");
        assert_eq!(ctx.version.as_deref(), Some("0.1.0"));
        assert!(ctx.skill_dirs.is_empty());
    }

    #[test]
    fn resolve_plugin_install_context_returns_none_version_when_manifest_has_no_version() {
        let (dir, svc) = temp_service();
        let entries = vec![relative_path_entry("nover", "plugins/nover")];
        let marketplace_path = seed_marketplace_with_registry(dir.path(), &svc, "mp1", &entries);
        make_plugin_with_skills(&marketplace_path, "nover", &["one"]);
        fs::write(
            marketplace_path
                .join("plugins")
                .join("nover")
                .join("plugin.json"),
            br#"{"name": "nover"}"#,
        )
        .expect("write plugin.json");

        let ctx = svc
            .resolve_plugin_install_context("mp1", "nover")
            .expect("happy path");
        assert!(
            ctx.version.is_none(),
            "expected no version, got: {:?}",
            ctx.version
        );
        assert_eq!(ctx.skill_dirs.len(), 1);
    }

    #[test]
    fn resolve_plugin_install_context_uses_manifest_skills_when_declared() {
        let (dir, svc) = temp_service();
        let entries = vec![relative_path_entry("withcustom", "plugins/withcustom")];
        let marketplace_path = seed_marketplace_with_registry(dir.path(), &svc, "mp1", &entries);
        let plugin_dir = marketplace_path.join("plugins").join("withcustom");
        let agents_dir = plugin_dir.join("agents");
        for skill in &["foo", "bar"] {
            let skill_dir = agents_dir.join(skill);
            fs::create_dir_all(&skill_dir).expect("create agent skill dir");
            fs::write(
                skill_dir.join("SKILL.md"),
                format!("---\nname: {skill}\ndescription: test\n---\n"),
            )
            .expect("write SKILL.md");
        }
        fs::write(
            plugin_dir.join("plugin.json"),
            br#"{"name": "withcustom", "version": "0.1.0", "skills": ["./agents/"], "agents": ["./custom-agents/"]}"#,
        )
        .expect("write plugin.json");

        let ctx = svc
            .resolve_plugin_install_context("mp1", "withcustom")
            .expect("happy path");
        assert_eq!(ctx.version.as_deref(), Some("0.1.0"));
        assert_eq!(
            ctx.agent_scan_paths,
            vec!["./custom-agents/".to_string()],
            "wrapper must thread agent_scan_paths through from the delegate"
        );
        let mut names: Vec<String> = ctx
            .skill_dirs
            .iter()
            .map(|p| {
                p.file_name()
                    .and_then(|s| s.to_str())
                    .expect("skill dir has valid UTF-8 name")
                    .to_string()
            })
            .collect();
        names.sort();
        assert_eq!(names, vec!["bar".to_string(), "foo".to_string()]);
        assert!(
            ctx.skill_dirs.iter().all(|p| {
                p.components()
                    .any(|c| c.as_os_str() == std::ffi::OsStr::new("agents"))
            }),
            "every skill dir must live under agents/: {:?}",
            ctx.skill_dirs
        );
        assert!(
            ctx.skill_dirs.iter().all(|p| {
                !p.components()
                    .any(|c| c.as_os_str() == std::ffi::OsStr::new("skills"))
            }),
            "no skill dir should live under the default skills/ tree: {:?}",
            ctx.skill_dirs
        );
    }

    #[test]
    fn resolve_plugin_install_context_errors_on_unknown_marketplace() {
        let (_dir, svc) = temp_service();
        let err = svc
            .resolve_plugin_install_context("does-not-exist", "anyplugin")
            .expect_err("unknown marketplace must error");
        // The inner MarketplaceError variant is an implementation detail
        // of list_plugin_entries; pin only the top-level Error::Marketplace
        // shape, matching the sibling list_skills_for_plugin_unknown_marketplace_errors
        // test.
        assert!(
            matches!(err, Error::Marketplace(_)),
            "expected Error::Marketplace, got: {err:?}"
        );
    }

    #[test]
    fn resolve_plugin_install_context_errors_on_plugin_not_found() {
        let (dir, svc) = temp_service();
        let entries = vec![relative_path_entry("alpha", "plugins/alpha")];
        let _ = seed_marketplace_with_registry(dir.path(), &svc, "mp1", &entries);

        let err = svc
            .resolve_plugin_install_context("mp1", "does-not-exist")
            .expect_err("unknown plugin must error");
        assert!(
            matches!(
                err,
                Error::Plugin(PluginError::NotFound { ref plugin, .. })
                    if plugin == "does-not-exist"
            ),
            "expected Plugin::NotFound, got: {err:?}"
        );
    }

    #[test]
    fn resolve_plugin_install_context_errors_on_missing_plugin_dir() {
        let (dir, svc) = temp_service();
        // Registry entry claims the plugin lives at "plugins/ghost", but
        // the directory is never created — the resolver must surface
        // DirectoryMissing rather than silently falling back to defaults.
        let entries = vec![relative_path_entry("ghost", "plugins/ghost")];
        let _ = seed_marketplace_with_registry(dir.path(), &svc, "mp1", &entries);

        let err = svc
            .resolve_plugin_install_context("mp1", "ghost")
            .expect_err("missing plugin_dir must error");
        assert!(
            matches!(err, Error::Plugin(PluginError::DirectoryMissing { .. })),
            "expected Plugin::DirectoryMissing, got: {err:?}"
        );
    }

    #[test]
    fn resolve_plugin_install_context_errors_on_malformed_plugin_json() {
        let (dir, svc) = temp_service();
        let entries = vec![relative_path_entry("broken", "plugins/broken")];
        let marketplace_path = seed_marketplace_with_registry(dir.path(), &svc, "mp1", &entries);
        let plugin_dir = marketplace_path.join("plugins").join("broken");
        fs::create_dir_all(&plugin_dir).expect("create plugin dir");
        fs::write(plugin_dir.join("plugin.json"), b"{not json").expect("write plugin.json");

        let err = svc
            .resolve_plugin_install_context("mp1", "broken")
            .expect_err("malformed plugin.json must error");
        assert!(
            matches!(err, Error::Plugin(PluginError::InvalidManifest { .. })),
            "expected Plugin::InvalidManifest, got: {err:?}"
        );
    }

    #[test]
    fn resolve_plugin_install_context_errors_on_remote_source() {
        let (dir, svc) = temp_service();
        let entries = vec![PluginEntry {
            name: "remote-plugin".into(),
            description: None,
            source: PluginSource::Structured(StructuredSource::GitHub {
                repo: "owner/repo".into(),
                git_ref: None,
                sha: None,
            }),
        }];
        let _ = seed_marketplace_with_registry(dir.path(), &svc, "mp1", &entries);

        let err = svc
            .resolve_plugin_install_context("mp1", "remote-plugin")
            .expect_err("remote source must refuse local resolution");
        assert!(
            matches!(err, Error::Plugin(PluginError::RemoteSourceNotLocal { .. })),
            "expected Plugin::RemoteSourceNotLocal, got: {err:?}"
        );
    }

    // -----------------------------------------------------------------------
    // resolve_plugin_install_context_from_dir
    // -----------------------------------------------------------------------

    #[test]
    #[cfg(unix)]
    fn resolve_plugin_install_context_from_dir_refuses_symlinked_manifest() {
        use std::os::unix::fs::symlink;

        let (dir, _svc) = temp_service();
        let plugin_dir = dir.path().join("plugin");
        fs::create_dir_all(&plugin_dir).expect("create plugin dir");

        // Real manifest lives elsewhere; symlink it into plugin_dir. A
        // hardened loader must treat the symlink as absent — following it
        // would leak `real-plugin.json` into the plugin's identity.
        let real_manifest = dir.path().join("real-plugin.json");
        fs::write(&real_manifest, br#"{"name":"smuggled","version":"9.9.9"}"#)
            .expect("write real manifest");
        symlink(&real_manifest, plugin_dir.join("plugin.json")).expect("create symlink");

        let ctx = MarketplaceService::resolve_plugin_install_context_from_dir(&plugin_dir)
            .expect("symlinked manifest must be treated as absent, not error");
        assert!(
            ctx.version.is_none(),
            "symlinked manifest must not leak its version, got: {:?}",
            ctx.version
        );
        assert!(
            ctx.skill_dirs.is_empty(),
            "no skills/ tree exists, expected empty, got: {:?}",
            ctx.skill_dirs
        );
        assert_eq!(
            ctx.agent_scan_paths,
            crate::DEFAULT_AGENT_PATHS
                .iter()
                .map(|s| (*s).to_string())
                .collect::<Vec<_>>(),
            "symlinked manifest must fall back to DEFAULT_AGENT_PATHS, not leak target's agents"
        );
    }

    #[test]
    fn resolve_plugin_install_context_from_dir_falls_back_to_default_agent_paths_when_manifest_absent()
     {
        let (dir, _svc) = temp_service();
        let plugin_dir = dir.path().join("plugin");
        fs::create_dir_all(&plugin_dir).expect("create plugin dir");

        let ctx = MarketplaceService::resolve_plugin_install_context_from_dir(&plugin_dir)
            .expect("missing manifest must yield default agent paths, not error");
        assert_eq!(
            ctx.agent_scan_paths,
            crate::DEFAULT_AGENT_PATHS
                .iter()
                .map(|s| (*s).to_string())
                .collect::<Vec<_>>(),
            "absent manifest must fall back to DEFAULT_AGENT_PATHS"
        );
    }

    #[test]
    fn resolve_plugin_install_context_from_dir_uses_manifest_agents_when_declared() {
        let (dir, _svc) = temp_service();
        let plugin_dir = dir.path().join("plugin");
        fs::create_dir_all(&plugin_dir).expect("create plugin dir");
        fs::write(
            plugin_dir.join("plugin.json"),
            br#"{"name": "p", "agents": ["./custom-agents/"]}"#,
        )
        .expect("write plugin.json");

        let ctx = MarketplaceService::resolve_plugin_install_context_from_dir(&plugin_dir)
            .expect("happy path");
        assert_eq!(ctx.agent_scan_paths, vec!["./custom-agents/".to_string()]);
        assert!(
            ctx.version.is_none(),
            "manifest has no version field, got: {:?}",
            ctx.version
        );
        assert!(
            ctx.skill_dirs.is_empty(),
            "manifest declares no skills and no skills/ tree exists, got: {:?}",
            ctx.skill_dirs
        );
    }

    #[test]
    fn resolve_plugin_install_context_uses_default_steering_when_absent() {
        let (dir, _svc) = temp_service();
        let plugin_dir = dir.path().join("plugin");
        fs::create_dir_all(&plugin_dir).expect("create plugin dir");

        let ctx = MarketplaceService::resolve_plugin_install_context_from_dir(&plugin_dir)
            .expect("absent manifest must yield default steering paths");
        assert_eq!(
            ctx.steering_scan_paths,
            crate::DEFAULT_STEERING_PATHS
                .iter()
                .map(|s| (*s).to_string())
                .collect::<Vec<_>>(),
            "absent manifest must fall back to DEFAULT_STEERING_PATHS"
        );
    }

    #[test]
    fn resolve_plugin_install_context_uses_manifest_steering_when_declared() {
        let (dir, _svc) = temp_service();
        let plugin_dir = dir.path().join("plugin");
        fs::create_dir_all(&plugin_dir).expect("create plugin dir");
        fs::write(
            plugin_dir.join("plugin.json"),
            br#"{"name": "p", "steering": ["./guide/", "./extras/"]}"#,
        )
        .expect("write plugin.json");

        let ctx = MarketplaceService::resolve_plugin_install_context_from_dir(&plugin_dir)
            .expect("happy path");
        assert_eq!(
            ctx.steering_scan_paths,
            vec!["./guide/".to_string(), "./extras/".to_string()]
        );
    }

    #[test]
    fn resolve_plugin_install_context_reads_format_kiro_cli() {
        let (dir, _svc) = temp_service();
        let plugin_dir = dir.path().join("plugin");
        fs::create_dir_all(&plugin_dir).expect("create plugin dir");
        fs::write(
            plugin_dir.join("plugin.json"),
            br#"{"name": "p", "format": "kiro-cli"}"#,
        )
        .expect("write plugin.json");

        let ctx = MarketplaceService::resolve_plugin_install_context_from_dir(&plugin_dir)
            .expect("happy path");
        assert_eq!(ctx.format, Some(crate::plugin::PluginFormat::KiroCli));
    }

    #[test]
    fn resolve_plugin_install_context_format_absent_is_none() {
        let (dir, _svc) = temp_service();
        let plugin_dir = dir.path().join("plugin");
        fs::create_dir_all(&plugin_dir).expect("create plugin dir");
        fs::write(plugin_dir.join("plugin.json"), br#"{"name": "p"}"#).expect("write plugin.json");

        let ctx = MarketplaceService::resolve_plugin_install_context_from_dir(&plugin_dir)
            .expect("happy path");
        assert!(ctx.format.is_none());
    }
}

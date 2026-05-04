//! Kiro project state management.
//!
//! Manages the `.kiro/skills/` directory layout and an
//! `installed-skills.json` tracking file that records which skills have been
//! installed, from which marketplace and plugin, and when.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

use crate::agent::tools::MappedTool;
use crate::agent::{AgentDefinition, AgentDialect};
use crate::error::{AgentError, SkillError};
use crate::validation;
use crate::validation::{MarketplaceName, PluginName, RelativePath};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Metadata recorded for each installed skill.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledSkillMeta {
    /// Name of the marketplace the skill came from. Routed through
    /// [`MarketplaceName::Deserialize`] so a tampered tracking file with
    /// a malformed marketplace value is rejected at `serde_json::from_slice`
    /// time, not later via a follow-up walker.
    pub marketplace: MarketplaceName,
    /// Name of the plugin that owns the skill. Same parse-time validation
    /// as [`Self::marketplace`].
    pub plugin: PluginName,
    /// Optional version string from the plugin manifest.
    pub version: Option<String>,
    /// Timestamp when the skill was installed.
    pub installed_at: DateTime<Utc>,

    /// Tree-hash of the skill source as it existed in the marketplace at
    /// install time. Required after PR #100 review I2 — every install
    /// path computes this, and the install↔detect symmetry pass made
    /// `source_scan_root` required which means any post-Stage-1+
    /// tracking entry has both. The deferred `legacy_fallback` plumbing
    /// in `scan_plugin_for_content_drift` was unreachable in practice
    /// once `source_scan_root` became required (no entry could have
    /// `scan_root` without `source_hash`) and is gone.
    pub source_hash: String,

    /// Tree-hash of the skill as it was copied into the project.
    /// Required after PR #100 review I2; see [`Self::source_hash`].
    pub installed_hash: String,

    /// Scan root (relative to `plugin_dir`) that this skill was
    /// installed from. Required at install time; populated from the
    /// `DiscoveredSkill.scan_root` field which `discover_skill_dirs`
    /// returns alongside each found skill directory. Drift detection at
    /// [`crate::service::MarketplaceService::scan_plugin_for_content_drift`]
    /// uses this directly to locate the source skill dir for hash
    /// recomputation, closing #97 (the hardcoded
    /// `plugin_dir.join("skills")` bug).
    pub source_scan_root: RelativePath,
}

/// The on-disk structure of `installed-skills.json`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InstalledSkills {
    /// Map from skill name to its installation metadata.
    pub skills: HashMap<String, InstalledSkillMeta>,
}

/// Metadata recorded for each installed agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledAgentMeta {
    /// Name of the marketplace the agent came from. See
    /// [`InstalledSkillMeta::marketplace`] for the parse-time validation contract.
    pub marketplace: MarketplaceName,
    /// Name of the plugin that owns the agent.
    pub plugin: PluginName,
    /// Optional version string from the plugin manifest.
    pub version: Option<String>,
    /// Timestamp when the agent was installed.
    pub installed_at: DateTime<Utc>,
    /// Which source dialect the agent was parsed from. Persisted via the
    /// enum's serde rename so the wire format stays `"claude"` / `"copilot"`.
    pub dialect: AgentDialect,
    /// Relative path under the plugin tree of the source file that was
    /// installed. Required at install time; populated via
    /// [`crate::validation::RelativePath::from_path_under`] from the
    /// discovered agent file's path. Drift detection at
    /// [`crate::service::MarketplaceService::scan_plugin_for_content_drift`]
    /// uses this directly to locate the source for hash recomputation —
    /// no dialect-fallback, no probe.
    ///
    /// Wrapped in [`RelativePath`] so `serde_json::from_slice` rejects
    /// path-traversal attempts (`"../../etc/passwd"`) at tracking-file
    /// load time per CLAUDE.md's "Parse, don't validate" rule. The
    /// `RelativePath::Deserialize` impl routes through `RelativePath::new`,
    /// which forbids `..`, absolute paths, NUL bytes, and embedded
    /// backslashes.
    ///
    /// **Was `Option<RelativePath>`** before the install↔detect symmetry
    /// pass; tightened to required when the no-users assumption let us
    /// drop the legacy-fallback machinery (the dialect-fallback branch
    /// in `agent_hash_inputs` and the I-N7 actionable-error branch in
    /// the agents loop). A tracking file written before the field
    /// existed now fails to deserialize — pinned by the
    /// `load_installed_agents_rejects_legacy_entry` test.
    pub source_path: RelativePath,

    /// Tree-hash of the agent source as it existed in the marketplace at
    /// install time. Required after PR #100 review I2; mirrors the
    /// `InstalledSkillMeta::source_hash` tightening — every install
    /// path computes it, and the post-symmetry-pass schema makes a
    /// missing-source_hash entry impossible (the matching
    /// `source_path` and `dialect` fields are also required).
    pub source_hash: String,

    /// Tree-hash of the agent as it was copied into the project.
    /// Required after PR #100 review I2; see [`Self::source_hash`].
    pub installed_hash: String,
}

/// Tracking entry for a plugin's companion file bundle that lives under
/// `.kiro/agents/`. Populated by:
///
/// - The translated-agent install path (this stage): each translated
///   agent's `prompts/<name>.md` body file is added to its plugin's
///   bundle entry. This makes the file plugin-owned from day one, so a
///   later native plugin install at the same path is correctly flagged
///   as a cross-plugin clash rather than a free-for-the-taking orphan.
/// - The native-agent install path: plugin-wide companion
///   bundles discovered alongside native agent JSONs.
///
/// Ownership is at the plugin level (not per-agent), so this entry
/// tracks the union of files installed for one plugin.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledNativeCompanionsMeta {
    /// Marketplace the bundle was installed from. See
    /// [`InstalledSkillMeta::marketplace`] for the parse-time validation
    /// contract. Disambiguates same-named plugins across marketplaces — the
    /// outer `HashMap<String, _>` is keyed by plugin name alone, so the
    /// `marketplace` field is what makes A-16's "only-removes-matching-
    /// marketplace" comparison correct.
    pub marketplace: MarketplaceName,
    pub plugin: PluginName,
    pub version: Option<String>,
    pub installed_at: DateTime<Utc>,
    /// Relative paths under `.kiro/agents/` of every companion file owned
    /// by this plugin. Used for collision detection (cross-plugin path
    /// overlap) and for uninstall.
    pub files: Vec<PathBuf>,
    pub source_hash: String,
    pub installed_hash: String,
    /// Scan root (relative to `plugin_dir`) that this companion bundle
    /// was installed from. Required at install time. The
    /// single-scan-root invariant is enforced upstream by
    /// `multiple_companion_scan_roots`, so all `files` resolve under
    /// this single root. Drift detection uses it directly, replacing
    /// PR #96's `hash_artifact_in_scan_paths` probe.
    pub source_scan_root: RelativePath,
}

/// The on-disk structure of `installed-agents.json`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InstalledAgents {
    /// Map from agent name to its installation metadata.
    pub agents: HashMap<String, InstalledAgentMeta>,
    /// Per-plugin companion file ownership. Defaults to empty for
    /// backward compat with legacy tracking files; omitted from serialized
    /// output when empty so round-trips are byte-identical.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub native_companions: HashMap<String, InstalledNativeCompanionsMeta>,
}

/// Tracking entry for one installed steering file.
///
/// One entry per file under `.kiro/steering/`, keyed in
/// [`InstalledSteering::files`] by the relative path under that
/// directory (which is also the file's user-facing identity — there's
/// no synthetic id). `source_hash` and `installed_hash` are the
/// blake3-prefixed hashes computed against the source file's bytes
/// and the installed bytes respectively; for steering they're
/// always equal because there is no parse-and-translate step.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledSteeringMeta {
    /// Marketplace the file was installed from. See
    /// [`InstalledSkillMeta::marketplace`] for the parse-time validation
    /// contract.
    pub marketplace: MarketplaceName,
    pub plugin: PluginName,
    pub version: Option<String>,
    pub installed_at: DateTime<Utc>,
    pub source_hash: String,
    pub installed_hash: String,
    /// Scan root (relative to `plugin_dir`) that this steering file
    /// was installed from. Required at install time; populated from
    /// `DiscoveredNativeFile.scan_root` (which steering shares with
    /// the agent discover sites). Drift detection at
    /// [`crate::service::MarketplaceService::scan_plugin_for_content_drift`]
    /// uses this directly to locate the source file for hash
    /// recomputation, replacing PR #96's `hash_artifact_in_scan_paths`
    /// probe helper.
    pub source_scan_root: RelativePath,
}

/// Compute the install-time `source_scan_root` for a discovered
/// steering file, mapping the path-not-under-plugin-dir error to a
/// `SteeringError::ScanRootInvalid`. Extracted from
/// `install_steering_file_locked` to keep that function under
/// clippy's 100-line cap after the install↔detect symmetry pass.
///
/// `ScanRootInvalid` is structurally distinct from `SourceReadFailed`:
/// the latter is an I/O failure on a content file, the former is a
/// structural validation failure on the scan-root *path*. Wrapping it
/// as a synthetic `io::Error` (the pre-PR-#100-review shape) lied
/// about the failure mode in the wire-format `reason` field — this
/// constructor preserves the real `ValidationError` via `#[source]`
/// so chained renderers see the precise cause.
pub(crate) fn required_steering_scan_root(
    scan_root: &std::path::Path,
    plugin_dir: &std::path::Path,
) -> Result<RelativePath, crate::steering::SteeringError> {
    RelativePath::from_path_under(scan_root, plugin_dir).map_err(|source| {
        crate::steering::SteeringError::ScanRootInvalid {
            path: scan_root.to_path_buf(),
            plugin_dir: plugin_dir.to_path_buf(),
            source,
        }
    })
}

/// On-disk structure of `installed-steering.json`. Map key is the
/// file's relative path under `.kiro/steering/`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InstalledSteering {
    /// Per-file ownership. Defaults to empty for backward compat with
    /// projects that pre-date steering install; omitted from serialized
    /// output when empty so round-trips are byte-identical.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub files: HashMap<PathBuf, InstalledSteeringMeta>,
}

/// What happened during one native install call. Three states are
/// distinct variants rather than a `(was_idempotent: bool,
/// forced_overwrite: bool)` pair so that the contradictory
/// `(true, true)` state is unrepresentable by construction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum InstallOutcomeKind {
    /// Verified no-op — `source_hash` matched the existing tracking
    /// entry's `source_hash`. No bytes were written.
    Idempotent,
    /// Clean first install — no prior tracking entry, no orphan on disk.
    Installed,
    /// Force-mode overwrote a tracked path (same plugin's prior content,
    /// another plugin's content via ownership transfer, or an orphan
    /// without tracking).
    ForceOverwrote,
}

/// In-memory outcome of one [`KiroProject::install_native_agent`] call.
///
/// Carries enough detail for the service layer to render an install-summary
/// row without re-reading tracking — name, the resolved destination JSON
/// path, what kind of install happened, and both content hashes.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct InstalledNativeAgentOutcome {
    pub name: String,
    pub json_path: PathBuf,
    pub kind: InstallOutcomeKind,
    pub source_hash: String,
    pub installed_hash: String,
}

/// Output of any classifier that decides between "early-return idempotent
/// outcome" and "proceed with install, possibly with `forced_overwrite`".
/// The idempotent variant boxes its payload to keep the enum size small
/// when the outcome type is large. Used by three classifiers:
/// [`KiroProject::classify_native_collision`] (with
/// [`InstalledNativeAgentOutcome`]),
/// `classify_companion_collision` (with
/// [`InstalledNativeCompanionsOutcome`]), and
/// `classify_steering_collision` (with
/// [`SteeringIdempotentEcho`] — *not* the full
/// `InstalledSteeringOutcome`; see that struct's docs for why).
enum CollisionDecision<T> {
    Idempotent(Box<T>),
    Proceed { forced_overwrite: bool },
}

/// Returned in [`CollisionDecision::Idempotent`] from
/// [`KiroProject::classify_steering_collision`]. Carries only the prior
/// `installed_hash` because the classifier doesn't see the original
/// source path — the caller (`install_steering_file_locked`) holds
/// `source.source` and constructs the full
/// [`crate::steering::InstalledSteeringOutcome`] there.
///
/// Splitting the construction prevents the bug where the classifier
/// would otherwise have to fall back to setting `source = dest` (the
/// classifier never receives the source path), which would leak the
/// destination path into the wire-format `source` field on idempotent
/// reinstalls — visible to Tauri callers via the
/// `#[derive(specta::Type)]` on `InstalledSteeringOutcome`.
struct SteeringIdempotentEcho {
    prior_installed_hash: String,
}

/// Input bundle for [`KiroProject::install_native_companions`]. Groups the
/// immutable refs the install needs so the public signature stays at one
/// parameter.
///
/// The caller is responsible for verifying that all `rel_paths` belong to
/// a single `scan_root` — multi-scan-root native plugins are rejected at
/// the service layer (see [`AgentError::MultipleScanRootsNotSupported`])
/// before this function is called, so the install can assume the invariant.
#[derive(Debug)]
pub struct NativeCompanionsInput<'a> {
    /// The plugin's agents/ scan root. Used as the hashing base.
    pub scan_root: &'a Path,
    /// Companion file paths relative to `scan_root` (e.g.
    /// `prompts/reviewer.md`). Also the relative paths under
    /// `.kiro/agents/` they install to.
    pub rel_paths: &'a [PathBuf],
    pub marketplace: &'a MarketplaceName,
    pub plugin: &'a PluginName,
    pub version: Option<&'a str>,
    pub source_hash: &'a str,
    pub mode: crate::service::InstallMode,
    /// Plugin root directory; used to compute
    /// [`InstalledNativeCompanionsMeta::source_scan_root`] from
    /// `scan_root` at install time so detection can lookup the
    /// source location directly without probing.
    pub plugin_dir: &'a Path,
}

/// Input bundle for [`KiroProject::install_native_agent`]. Groups the
/// immutable refs the install needs so the public signature stays at
/// one parameter (paralleling [`NativeCompanionsInput`]). The
/// `source_path` field was added for the install↔detect symmetry pass —
/// `InstalledAgentMeta.source_path` is required, so the install must
/// receive it to populate the meta.
#[derive(Debug)]
pub struct NativeAgentInstallInput<'a> {
    pub bundle: &'a crate::agent::NativeAgentBundle,
    pub marketplace: &'a MarketplaceName,
    pub plugin: &'a PluginName,
    pub version: Option<&'a str>,
    pub source_hash: &'a str,
    pub source_path: &'a crate::validation::RelativePath,
    pub mode: crate::service::InstallMode,
}

/// Output of `promote_native_companions`: paths placed at their final
/// destinations, plus a list of `(original, backup)` pairs the caller
/// must restore on later failure or delete on success.
struct CompanionPromotion {
    placed: Vec<PathBuf>,
    backups: Vec<(PathBuf, PathBuf)>,
}

/// In-memory outcome of one [`KiroProject::install_native_companions`] call.
///
/// Plugin-scoped (companion bundles are owned per-plugin, not per-agent),
/// so callers see one entry for the whole bundle rather than one per file.
/// `files` is the absolute destination paths of every companion file
/// installed for this plugin.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct InstalledNativeCompanionsOutcome {
    pub plugin: String,
    pub files: Vec<PathBuf>,
    pub kind: InstallOutcomeKind,
    pub source_hash: String,
    pub installed_hash: String,
}

/// Aggregated view of a single installed plugin — the union of
/// what's tracked across `installed-skills.json`,
/// `installed-steering.json`, and `installed-agents.json` for a
/// given `(marketplace, plugin)` pair.
///
/// Returned by [`KiroProject::installed_plugins`]. The frontend
/// renders one row per `InstalledPluginInfo`.
///
/// `installed_version` is the version of the **most recent install**
/// by `installed_at` timestamp — not a lexicographic max. See A-6
/// in the plan amendments doc for why string-compare is wrong here.
///
/// `earliest_install` and `latest_install` are RFC3339-formatted
/// strings to match the FFI shape used by `InstalledSkillInfo`
/// (specta's chrono feature isn't enabled in this crate).
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct InstalledPluginInfo {
    pub marketplace: MarketplaceName,
    pub plugin: PluginName,
    pub installed_version: Option<String>,
    pub skill_count: u32,
    pub steering_count: u32,
    pub agent_count: u32,
    pub installed_skills: Vec<String>,
    pub installed_steering: Vec<std::path::PathBuf>,
    pub installed_agents: Vec<String>,
    /// RFC3339-formatted timestamp.
    pub earliest_install: String,
    /// RFC3339-formatted timestamp.
    pub latest_install: String,
}

/// Wire-format wrapper around [`Vec<InstalledPluginInfo>`] that also
/// carries per-tracking-file load failures so the UI can render a
/// partial state when one of the three `installed-*.json` files is
/// corrupt or unreadable (I13).
///
/// Rationale: previously, `installed_plugins()` failed the entire
/// aggregator on any `?`-chain load failure, leaving the user with
/// "zero installed plugins" even when two of three tracking files
/// loaded cleanly. The view here surfaces what loaded AND what
/// didn't, so the UI can show a "partial state — N tracking files
/// failed to load" banner instead of a misleading empty list.
#[derive(Debug, Clone, Default, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct InstalledPluginsView {
    /// Per-`(marketplace, plugin)` rows assembled from the tracking
    /// files that loaded successfully. May be empty if every file
    /// failed; the [`Self::partial_load_warnings`] vec then carries
    /// the explanation.
    pub plugins: Vec<InstalledPluginInfo>,
    /// One entry per `installed-*.json` whose load failed. The
    /// corresponding content type's contributions are missing from
    /// `plugins`. Empty on a clean state.
    ///
    /// `serde(default)` is kept for legacy-JSON tolerance.
    /// `skip_serializing_if` is intentionally absent — `tauri-specta`
    /// 2.0.0-rc.24 unified mode rejects it (see A-25 in plan
    /// amendments). Empty Vec serializes as `[]` rather than being
    /// omitted, matching `InstallSkillsResult.failed` etc.
    #[serde(default)]
    pub partial_load_warnings: Vec<TrackingLoadWarning>,
}

/// Per-tracking-file load failure surfaced by
/// [`KiroProject::installed_plugins`] (I13).
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct TrackingLoadWarning {
    /// Filename relative to `.kiro/`: `"installed-skills.json"` /
    /// `"installed-steering.json"` / `"installed-agents.json"`. Just
    /// the basename — the absolute path leaks layout information the
    /// frontend doesn't need.
    pub tracking_file: String,
    /// Rendered error chain via [`crate::error::error_full_chain`] —
    /// wire format per CLAUDE.md "in any wire-format `reason`/`error:
    /// String` field that crosses the FFI, use `error_full_chain(&err)`".
    pub error: String,
}

/// Per-content-type sub-result for [`RemovePluginResult`]. Mirrors
/// the install-side [`InstallSkillsResult`] shape.
#[derive(Clone, Debug, Default, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct RemoveSkillsResult {
    #[serde(default)]
    pub removed: Vec<String>, // skill names
    #[serde(default)]
    pub failures: Vec<RemoveItemFailure>,
}

/// Per-content-type sub-result for [`RemovePluginResult`]. Mirrors
/// the install-side `InstallSteeringResult` shape.
#[derive(Clone, Debug, Default, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct RemoveSteeringResult {
    #[serde(default)]
    pub removed: Vec<String>, // rendered via Path::display()
    #[serde(default)]
    pub failures: Vec<RemoveItemFailure>,
}

/// Per-content-type sub-result for [`RemovePluginResult`]. Mirrors
/// the install-side [`crate::service::InstallAgentsResult`] shape.
/// `removed` is a flat vec of translated agent names + native agent
/// names. Native companion file paths are NOT itemized (P2a-3
/// decision α) — the `native_companions` cascade step succeeds with
/// no per-file entries. If the FE later wants per-companion
/// granularity, that's an additive field change.
#[derive(Clone, Debug, Default, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct RemoveAgentsResult {
    #[serde(default)]
    pub removed: Vec<String>,
    #[serde(default)]
    pub failures: Vec<RemoveItemFailure>,
}

/// One failure during a per-content-type removal step. The discriminator
/// (which content type) is the parent type — no `content_type: String`
/// field needed (it's expressed structurally via the parent's field
/// name in [`RemovePluginResult`]).
#[derive(Clone, Debug, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct RemoveItemFailure {
    /// The skill/agent name or steering rel-path rendered via
    /// [`std::path::Path::display`].
    pub item: String,
    /// Rendered error chain via [`crate::error::error_full_chain`] —
    /// wire format per CLAUDE.md FFI rule.
    pub error: String,
}

/// Result of [`KiroProject::remove_plugin`] — per-content-type
/// sub-results, symmetric with [`crate::service::InstallPluginResult`].
/// Native companions fold into [`RemoveAgentsResult`] (matches the
/// install-side asymmetry where native companions are agent-side
/// artifacts).
///
/// No `marketplace` / `plugin` echo fields — caller already passed
/// those args to `remove_plugin`. (Different from
/// [`crate::service::InstallPluginResult`] which gained `marketplace`
/// in Phase 1.5 A4 because that type lives in lists where
/// self-identification is needed.)
#[derive(Clone, Debug, Default, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct RemovePluginResult {
    pub skills: RemoveSkillsResult,
    pub steering: RemoveSteeringResult,
    pub agents: RemoveAgentsResult,
}

/// Per-`(marketplace, plugin)` accumulator used by
/// [`KiroProject::installed_plugins`] while folding the three tracking
/// files into [`InstalledPluginInfo`] rows.
#[derive(Default)]
struct Acc {
    /// `(installed_at, version)` of the most recent tracking entry seen
    /// across the three content types — the version of the latest
    /// install, which is what the UI wants under "this plugin's
    /// installed version."
    latest: Option<(chrono::DateTime<chrono::Utc>, Option<String>)>,
    earliest: Option<chrono::DateTime<chrono::Utc>>,
    skills: Vec<String>,
    steering: Vec<std::path::PathBuf>,
    agents: Vec<String>,
}

fn update_latest(
    acc: &mut Acc,
    version: Option<&str>,
    installed_at: chrono::DateTime<chrono::Utc>,
) {
    let new_version = version.map(str::to_string);
    // `>` (not `>=`): on tied timestamps, first-seen wins. The
    // aggregator iterates skills → steering → agents, so on equal
    // timestamps the first of those three with a tracking entry
    // contributes the displayed version. Stable across process
    // restarts; `>=` would leak HashMap iteration order into the
    // wire format.
    let should_replace = acc
        .latest
        .as_ref()
        .is_none_or(|(when, _)| installed_at > *when);
    if should_replace {
        acc.latest = Some((installed_at, new_version));
    }
    acc.earliest = Some(acc.earliest.map_or(installed_at, |e| e.min(installed_at)));
}

// ---------------------------------------------------------------------------
// KiroProject
// ---------------------------------------------------------------------------

/// Name of the skill tracking file inside `.kiro/`.
const INSTALLED_SKILLS_FILE: &str = "installed-skills.json";

/// Name of the agent tracking file inside `.kiro/`.
const INSTALLED_AGENTS_FILE: &str = "installed-agents.json";

/// Name of the steering tracking file inside `.kiro/`.
const INSTALLED_STEERING_FILE: &str = "installed-steering.json";

/// Recursively copy a directory tree from `src` to `dest`.
///
/// Creates `dest` and all intermediate directories. Files are copied
/// preserving the relative directory structure.
///
/// **Symlinks are skipped** to prevent path traversal attacks where a
/// malicious skill package could include symlinks pointing to sensitive
/// host files.
///
/// **Hardlinks (nlink > 1) are skipped on Unix** because the entry could
/// share an inode with a sensitive file outside the source tree (e.g.
/// `~/.ssh/id_rsa`). Symlinks expose the same risk via the kernel's
/// resolution; hardlinks expose it via the inode itself, so they need
/// the same treatment. The skip is logged at `warn` so a user wondering
/// "why is `LICENSE` missing from my install?" gets a clear signal.
/// Inside a cloned git repo this never fires (git can't store hardlinks);
/// it matters for `LocalPath` marketplaces where the user-pointed
/// directory may have been crafted to expose data via hardlinks.
///
/// # Errors
///
/// Returns an I/O error if any directory creation or file copy fails.
/// The error includes the path that caused the failure.
fn copy_dir_recursive(src: &Path, dest: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dest)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let target = dest.join(entry.file_name());
        // Use symlink_metadata (does NOT follow symlinks) so we can
        // detect and skip symlinks. Skill source directories are
        // untrusted input — a symlink could point to sensitive files.
        let metadata = fs::symlink_metadata(entry.path()).map_err(|e| {
            std::io::Error::new(
                e.kind(),
                format!(
                    "failed to read metadata for {}: {e}",
                    entry.path().display()
                ),
            )
        })?;
        if metadata.is_symlink() {
            debug!(
                path = %entry.path().display(),
                "skipping symlink in skill directory"
            );
            continue;
        }
        // Hardlink check (Unix only). Files with nlink > 1 share an inode
        // with at least one other path; we cannot tell from here whether
        // the other path is benign (a dedup tool's twin) or malicious
        // (linked into ~/.ssh). Refuse rather than guess. Windows / NTFS
        // also supports hardlinks (CreateHardLink) but lacks a portable
        // nlink accessor in std; the platform.rs Windows copy path mirrors
        // this posture by skipping reparse points instead.
        #[cfg(unix)]
        if metadata.is_file() {
            use std::os::unix::fs::MetadataExt;
            if metadata.nlink() > 1 {
                warn!(
                    path = %entry.path().display(),
                    nlink = metadata.nlink(),
                    "skipping hardlinked file in skill source; cannot prove its inode \
                     is not also linked to a sensitive file outside the source tree"
                );
                continue;
            }
        }
        if metadata.is_dir() {
            copy_dir_recursive(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), &target).map_err(|e| {
                std::io::Error::new(
                    e.kind(),
                    format!(
                        "failed to copy {} to {}: {e}",
                        entry.path().display(),
                        target.display()
                    ),
                )
            })?;
        }
    }
    Ok(())
}

/// Manages skill installation within a Kiro project directory.
///
/// The project layout:
///
/// ```text
/// <root>/
///   .kiro/
///     installed-skills.json
///     skills/
///       <skill-name>/
///         SKILL.md
///         references/    (optional companion files)
///           *.md
/// ```
#[derive(Debug, Clone)]
pub struct KiroProject {
    root: PathBuf,
}

/// Input bundle for [`KiroProject::synthesize_companion_entry`]. Groups the
/// 5 immutable refs that the helper needs so the public-ish signature stays
/// at two parameters (the `&mut InstalledAgents` plus the bundle).
///
/// Rollback on hash failure is the caller's responsibility: this helper
/// does not touch `agents_root` (it only mutates `installed`), so the
/// caller — which still holds the `(json_target, prompt_target, backups)`
/// from the promote phase — is the right place to restore on error.
struct CompanionInput<'a> {
    marketplace: &'a MarketplaceName,
    plugin: &'a PluginName,
    version: Option<&'a str>,
    agents_root: &'a Path,
    prompt_rel: &'a Path,
    /// Scan root (relative to `plugin_dir`) under which the original
    /// agent .md was discovered; used to populate
    /// [`InstalledNativeCompanionsMeta::source_scan_root`]. For the
    /// translated-agents companion synthesis path the prompts are
    /// extracted (not separately stored in the source tree), so this
    /// value is the agent's source-side scan root by convention —
    /// closing C-1 (the install/detect hash recipe asymmetry for
    /// translated companions) is tracked at issue #99.
    source_scan_root: &'a RelativePath,
}

/// Output of [`KiroProject::promote_staged_agent`]: paths placed at their
/// final destinations plus a list of `(original, backup)` pairs the caller
/// must restore on later failure or delete on success. Mirrors
/// [`CompanionPromotion`] for the 2-file translated agent install path.
struct PromotedAgent {
    json_target: PathBuf,
    prompt_target: PathBuf,
    backups: Vec<(PathBuf, PathBuf)>,
}

/// Validate that a tracking-file path entry is safe for `base.join(rel)`
/// resolution against the install root. Walks `Path::components` so the
/// validation is platform-aware: on Windows a path like
/// `prompts\..\..\etc\passwd` decomposes into `ParentDir` components which
/// are rejected; on Unix the same string is a single `Normal` component
/// (backslash is a literal filename char) which is harmless because no
/// traversal can occur. The forward-slash form (`prompts/../etc`) is
/// caught as `ParentDir` on both platforms.
///
/// Distinct from [`crate::validation::validate_relative_path`] which
/// validates manifest-side strings and unconditionally rejects backslash
/// (because manifests should be portable). Tracking paths come from
/// `PathBuf` values the install code itself wrote and may carry the
/// platform-native separator on Windows.
fn validate_tracking_path_entry(rel: &Path) -> Result<(), &'static str> {
    use std::path::Component;
    if rel.as_os_str().is_empty() {
        return Err("path must not be empty");
    }
    if rel.has_root() {
        return Err("must not be an absolute path");
    }
    let bytes = rel.as_os_str().to_string_lossy();
    if bytes.contains('\0') {
        return Err("contains NUL byte");
    }
    for component in rel.components() {
        match component {
            Component::ParentDir => return Err("contains `..` (parent-dir) component"),
            Component::RootDir | Component::Prefix(_) => return Err("absolute path component"),
            Component::CurDir | Component::Normal(_) => {}
        }
    }
    Ok(())
}

/// Validate every companion file path in a freshly-loaded
/// [`InstalledAgents`] tracking record. Tracking files are user-owned —
/// a tampered or hand-edited entry containing `prompts/../../etc/passwd`
/// would, without this validation, flow into `agents_dir.join(rel)` at
/// hash recompute time (`hash::hash_artifact`) or removal time
/// (`fs::remove_file`) and read or delete files outside the install
/// boundary.
fn validate_tracking_companion_files(
    installed: &InstalledAgents,
    tracking_path: &Path,
) -> crate::error::Result<()> {
    for (plugin, meta) in &installed.native_companions {
        for file in &meta.files {
            if let Err(reason) = validate_tracking_path_entry(file) {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!(
                        "tracking file {} has invalid companion path for plugin `{}`: `{}`: {}",
                        tracking_path.display(),
                        plugin,
                        file.display(),
                        reason
                    ),
                )
                .into());
            }
        }
    }
    Ok(())
}

/// Validate every name key in a freshly-loaded [`InstalledSkills`]
/// tracking record. Tracking files are user-owned; a tampered or
/// hand-edited entry containing `../../etc/passwd` as a skill name
/// would, without this check, flow into `skills_dir.join(name)` at
/// removal time (`fs::remove_dir_all`) and delete files outside the
/// install boundary. Mirrors [`validate_tracking_steering_files`]'s
/// shape.
fn validate_tracking_skill_keys(
    installed: &InstalledSkills,
    tracking_path: &Path,
) -> crate::error::Result<()> {
    for name in installed.skills.keys() {
        if let Err(reason) = validation::validate_name(name) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "tracking file {} has invalid skill name `{}`: {}",
                    tracking_path.display(),
                    name,
                    reason
                ),
            )
            .into());
        }
    }
    Ok(())
}

/// Validate every name key in a freshly-loaded [`InstalledAgents`]
/// tracking record — both per-agent (`agents`) and per-plugin
/// (`native_companions`). Same rationale as
/// [`validate_tracking_skill_keys`]: a tampered name reaches
/// `agents_dir.join(format!("{name}.json"))` /
/// `agent_prompts_dir.join(format!("{name}.md"))` at removal time,
/// or shows up unfiltered in `installed_plugins()` on the wire.
fn validate_tracking_agent_keys(
    installed: &InstalledAgents,
    tracking_path: &Path,
) -> crate::error::Result<()> {
    for name in installed.agents.keys() {
        if let Err(reason) = validation::validate_name(name) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "tracking file {} has invalid agent name `{}`: {}",
                    tracking_path.display(),
                    name,
                    reason
                ),
            )
            .into());
        }
    }
    for plugin in installed.native_companions.keys() {
        if let Err(reason) = validation::validate_name(plugin) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "tracking file {} has invalid native_companions plugin key `{}`: {}",
                    tracking_path.display(),
                    plugin,
                    reason
                ),
            )
            .into());
        }
    }
    Ok(())
}

/// Validate every relative path key in a freshly-loaded
/// [`InstalledSteering`] tracking record. Same rationale as
/// [`validate_tracking_companion_files`]: a tampered key like
/// `../../etc/passwd` would otherwise reach `steering_dir.join(rel)`
/// at install / removal time and escape the install boundary.
fn validate_tracking_steering_files(
    installed: &InstalledSteering,
    tracking_path: &Path,
) -> crate::error::Result<()> {
    for file_path in installed.files.keys() {
        if let Err(reason) = validate_tracking_path_entry(file_path) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "tracking file {} has invalid steering path: `{}`: {}",
                    tracking_path.display(),
                    file_path.display(),
                    reason
                ),
            )
            .into());
        }
    }
    Ok(())
}

impl KiroProject {
    /// Create a new project handle rooted at the given directory.
    #[must_use]
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    /// The `.kiro/` directory.
    fn kiro_dir(&self) -> PathBuf {
        self.root.join(".kiro")
    }

    /// The `.kiro/skills/` directory.
    fn skills_dir(&self) -> PathBuf {
        self.kiro_dir().join("skills")
    }

    /// Path to the tracking file.
    fn tracking_path(&self) -> PathBuf {
        self.kiro_dir().join(INSTALLED_SKILLS_FILE)
    }

    /// Path to a specific skill directory.
    fn skill_dir(&self, name: &str) -> PathBuf {
        self.skills_dir().join(name)
    }

    /// Load the installed-skills tracking file.
    ///
    /// Returns a default (empty) [`InstalledSkills`] if the file does not
    /// exist.
    ///
    /// # Errors
    ///
    /// Returns an error on I/O or JSON parse failures.
    pub fn load_installed(&self) -> crate::error::Result<InstalledSkills> {
        let path = self.tracking_path();

        match fs::read(&path) {
            Ok(bytes) => {
                let installed: InstalledSkills = serde_json::from_slice(&bytes)?;
                validate_tracking_skill_keys(&installed, &path)?;
                Ok(installed)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                debug!(path = %path.display(), "tracking file not found, returning default");
                Ok(InstalledSkills::default())
            }
            Err(e) => Err(e.into()),
        }
    }

    /// Remove an installed skill.
    ///
    /// Drops the entry from `installed-skills.json` and unlinks the
    /// skill directory. `NotInstalled` is reserved for "no tracking
    /// entry" — a tracking row whose on-disk dir was hand-deleted is
    /// recovered: the tracking row is dropped and the call returns
    /// `Ok` (I3). Mirrors [`Self::remove_steering_file`] /
    /// [`Self::remove_agent`]'s shape so the cascade in
    /// [`Self::remove_plugin`] sees a uniform contract.
    ///
    /// # Errors
    ///
    /// - [`SkillError::NotInstalled`] if `name` has no tracking entry.
    /// - I/O or JSON serialisation errors. A `NotFound` on the
    ///   on-disk directory itself is treated as success (orphan
    ///   tracking row was just dropped).
    pub fn remove_skill(&self, name: &str) -> crate::error::Result<()> {
        validation::validate_name(name)?;

        crate::file_lock::with_file_lock(&self.tracking_path(), || -> crate::error::Result<()> {
            let dir = self.skill_dir(name);

            // Tracking-first removal (I3). Mirrors `remove_steering_file`
            // and `remove_agent`: drop the tracking entry before any fs
            // op so a crash between the two leaves the directory on
            // disk (harmless) rather than a phantom tracking entry
            // (which would resurrect the plugin in `installed_plugins()`).
            let mut installed = self.load_installed()?;
            let saved_meta =
                installed
                    .skills
                    .remove(name)
                    .ok_or_else(|| SkillError::NotInstalled {
                        name: name.to_owned(),
                    })?;
            self.write_tracking(&installed)?;

            match fs::remove_dir_all(&dir) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    // Orphan recovery: tracking row dropped, dir was
                    // already gone (user hand-deleted it). I3 closes
                    // the gap that previously made the cascade's
                    // A-12 path leave persistent orphans.
                    debug!(
                        name,
                        path = %dir.display(),
                        "skill dir already absent on disk; orphan tracking row dropped"
                    );
                }
                Err(e) => {
                    // Restore tracking on non-NotFound fs failure so
                    // the file system stays consistent with tracking.
                    warn!(
                        name,
                        error = %e,
                        "failed to delete skill directory after tracking update; \
                         restoring tracking entry"
                    );
                    installed.skills.insert(name.to_owned(), saved_meta);
                    if let Err(restore_err) = self.write_tracking(&installed) {
                        warn!(
                            name,
                            error = %restore_err,
                            "failed to restore tracking entry — skill may be \
                             untracked on disk"
                        );
                    }
                    return Err(e.into());
                }
            }

            Ok(())
        })?;

        debug!(name, "skill removed");
        Ok(())
    }

    /// Install a skill by copying an entire source directory into the project.
    ///
    /// Recursively copies `source_dir` to `.kiro/skills/<name>/`, preserving
    /// companion files (e.g. `references/`) for Kiro's lazy loading. Files
    /// are staged in a temp directory, then renamed into place so a crash
    /// during the copy phase cannot leave a partially installed skill
    /// directory. The tracking file is updated separately after the rename.
    ///
    /// # Errors
    ///
    /// - [`SkillError::AlreadyInstalled`] if a skill with this name already exists.
    /// - I/O or JSON serialisation errors.
    pub fn install_skill_from_dir(
        &self,
        name: &str,
        source_dir: &Path,
        meta: InstalledSkillMeta,
    ) -> crate::error::Result<()> {
        validation::validate_name(name)?;
        let source_hash = crate::hash::hash_dir_tree(source_dir)?;
        self.write_skill_dir(name, source_dir, meta, false, source_hash)
    }

    /// Install a skill by copying a source directory, overwriting any existing installation.
    ///
    /// New content is staged in a temp directory first, then the old directory
    /// is removed and the temp is renamed into place. The tracking file is
    /// updated separately after the rename.
    ///
    /// # Errors
    ///
    /// I/O or JSON serialisation errors.
    pub fn install_skill_from_dir_force(
        &self,
        name: &str,
        source_dir: &Path,
        meta: InstalledSkillMeta,
    ) -> crate::error::Result<()> {
        validation::validate_name(name)?;
        let source_hash = crate::hash::hash_dir_tree(source_dir)?;
        self.write_skill_dir(name, source_dir, meta, true, source_hash)
    }

    // -- agent installation ------------------------------------------------

    /// The `.kiro/agents/` directory.
    fn agents_dir(&self) -> PathBuf {
        self.kiro_dir().join("agents")
    }

    /// The `.kiro/agents/prompts/` directory.
    fn agent_prompts_dir(&self) -> PathBuf {
        self.agents_dir().join("prompts")
    }

    /// Path to the agent tracking file.
    fn agent_tracking_path(&self) -> PathBuf {
        self.kiro_dir().join(INSTALLED_AGENTS_FILE)
    }

    /// Load the installed-agents tracking file.
    ///
    /// Returns a default (empty) [`InstalledAgents`] if the file does not
    /// exist.
    ///
    /// # Errors
    ///
    /// Returns an error on I/O or JSON parse failures.
    pub fn load_installed_agents(&self) -> crate::error::Result<InstalledAgents> {
        let path = self.agent_tracking_path();
        match fs::read(&path) {
            Ok(bytes) => {
                let installed: InstalledAgents = serde_json::from_slice(&bytes)?;
                validate_tracking_agent_keys(&installed, &path)?;
                validate_tracking_companion_files(&installed, &path)?;
                Ok(installed)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                debug!(path = %path.display(), "agent tracking file not found, returning default");
                Ok(InstalledAgents::default())
            }
            Err(e) => Err(e.into()),
        }
    }

    /// Persist the agent tracking file to disk atomically.
    fn write_agent_tracking(&self, installed: &InstalledAgents) -> crate::error::Result<()> {
        let path = self.agent_tracking_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(installed)?;
        crate::cache::atomic_write(&path, json.as_bytes())?;
        Ok(())
    }

    // -- steering installation ---------------------------------------------

    /// The `.kiro/steering/` directory.
    #[must_use]
    pub fn steering_dir(&self) -> PathBuf {
        self.kiro_dir().join("steering")
    }

    /// Path to the steering tracking file.
    fn steering_tracking_path(&self) -> PathBuf {
        self.kiro_dir().join(INSTALLED_STEERING_FILE)
    }

    /// Load the installed-steering tracking file.
    ///
    /// Returns a default (empty) [`InstalledSteering`] if the file does
    /// not exist — pre-steering projects have no `installed-steering.json`,
    /// and that's a valid starting state, not an error.
    ///
    /// # Errors
    ///
    /// I/O failures (other than `NotFound`) or JSON parse failures.
    pub fn load_installed_steering(&self) -> crate::error::Result<InstalledSteering> {
        let path = self.steering_tracking_path();
        match fs::read(&path) {
            Ok(bytes) => {
                let installed: InstalledSteering = serde_json::from_slice(&bytes)?;
                validate_tracking_steering_files(&installed, &path)?;
                Ok(installed)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                debug!(
                    path = %path.display(),
                    "steering tracking file not found, returning default"
                );
                Ok(InstalledSteering::default())
            }
            Err(e) => Err(e.into()),
        }
    }

    /// Aggregate the three `installed-*.json` tracking files into one
    /// row per `(marketplace, plugin)` pair, the shape the
    /// `InstalledTab` UI renders.
    ///
    /// The returned `installed_version` reflects the *most recent*
    /// install across all three content types (skills, steering,
    /// agents), measured by `installed_at`. Tied timestamps keep
    /// the first-iterated version (skills → steering → agents) — see
    /// [`update_latest`] for the rationale.
    ///
    /// # Partial-load tolerance (I13)
    ///
    /// A failure to load one of the three tracking files is recorded
    /// in [`InstalledPluginsView::partial_load_warnings`] and the
    /// other two files still contribute. Previously a single corrupt
    /// tracking file would surface as the user seeing zero installed
    /// plugins; the view now lets the UI render the partial state with
    /// an explicit "N tracking files failed to load" banner.
    ///
    /// # Errors
    ///
    /// Infallible at the function level — even if every tracking
    /// file fails to load, the returned view carries the warnings
    /// and an empty `plugins` Vec. The `Result` return type is
    /// preserved purely for API stability and future-proofing
    /// (a panic-on-corrupt-tempdir style failure would fit there).
    pub fn installed_plugins(&self) -> crate::error::Result<InstalledPluginsView> {
        use std::collections::BTreeMap;

        // Newtype keys: both `MarketplaceName` and `PluginName` derive `Ord`
        // (added in Phase 1.5 Task 1) so the lexicographic ordering is
        // identical to a `(String, String)` key — wire-format ordering is
        // preserved across the migration.
        let mut by_pair: BTreeMap<(MarketplaceName, PluginName), Acc> = BTreeMap::new();
        let mut warnings: Vec<TrackingLoadWarning> = Vec::new();

        // I13: each tracking-file load failure is recorded as a
        // warning; the other two files still contribute. The
        // `unwrap_or_else` shape is a deliberate fall-back to the
        // `Default` value (empty maps) so the rest of the fold
        // doesn't have to special-case missing data.
        let skills = self.load_installed().unwrap_or_else(|e| {
            warnings.push(TrackingLoadWarning {
                tracking_file: INSTALLED_SKILLS_FILE.to_string(),
                error: crate::error::error_full_chain(&e),
            });
            InstalledSkills::default()
        });
        for (name, meta) in &skills.skills {
            let acc = by_pair
                .entry((meta.marketplace.clone(), meta.plugin.clone()))
                .or_default();
            acc.skills.push(name.clone());
            update_latest(acc, meta.version.as_deref(), meta.installed_at);
        }

        let steering = self.load_installed_steering().unwrap_or_else(|e| {
            warnings.push(TrackingLoadWarning {
                tracking_file: INSTALLED_STEERING_FILE.to_string(),
                error: crate::error::error_full_chain(&e),
            });
            InstalledSteering::default()
        });
        for (rel, meta) in &steering.files {
            let acc = by_pair
                .entry((meta.marketplace.clone(), meta.plugin.clone()))
                .or_default();
            acc.steering.push(rel.clone());
            update_latest(acc, meta.version.as_deref(), meta.installed_at);
        }

        let agents = self.load_installed_agents().unwrap_or_else(|e| {
            warnings.push(TrackingLoadWarning {
                tracking_file: INSTALLED_AGENTS_FILE.to_string(),
                error: crate::error::error_full_chain(&e),
            });
            InstalledAgents::default()
        });
        for (name, meta) in &agents.agents {
            let acc = by_pair
                .entry((meta.marketplace.clone(), meta.plugin.clone()))
                .or_default();
            acc.agents.push(name.clone());
            update_latest(acc, meta.version.as_deref(), meta.installed_at);
        }

        // Hoist `Utc::now()` out of the map closure (A-9): one syscall
        // not N. Also makes the fallback substitution explicit at one
        // site — a missing `installed_at` is a degenerate state the
        // tracking file shouldn't produce; substituting "now" is a
        // fallback the UI accepts but a future reader can scrutinize.
        let now = chrono::Utc::now();
        let plugins: Vec<InstalledPluginInfo> = by_pair
            .into_iter()
            .map(|((marketplace, plugin), mut acc)| {
                let (latest_install_dt, installed_version) =
                    acc.latest.map_or_else(|| (now, None), |(t, v)| (t, v));
                let earliest_install_dt = acc.earliest.unwrap_or(now);
                // Sort the per-plugin item Vecs (I1) so the wire-format
                // ordering doesn't depend on HashMap iteration order.
                // Without this, the same tracking files could yield
                // different `installed_skills` orderings across runs —
                // breaks UI snapshot tests and confuses humans reading
                // the JSON.
                acc.skills.sort();
                acc.steering.sort();
                acc.agents.sort();
                InstalledPluginInfo {
                    marketplace,
                    plugin,
                    installed_version,
                    skill_count: u32::try_from(acc.skills.len()).unwrap_or(u32::MAX),
                    steering_count: u32::try_from(acc.steering.len()).unwrap_or(u32::MAX),
                    agent_count: u32::try_from(acc.agents.len()).unwrap_or(u32::MAX),
                    installed_skills: acc.skills,
                    installed_steering: acc.steering,
                    installed_agents: acc.agents,
                    earliest_install: earliest_install_dt.to_rfc3339(),
                    latest_install: latest_install_dt.to_rfc3339(),
                }
            })
            .collect();
        Ok(InstalledPluginsView {
            plugins,
            partial_load_warnings: warnings,
        })
    }

    /// Remove a single installed steering file from
    /// `installed-steering.json` and unlink the file under
    /// `.kiro/steering/`.
    ///
    /// Defense in depth: even though `load_installed_steering` already
    /// validates each tracking-file key against
    /// [`validate_tracking_path_entry`], this method re-runs the same
    /// check against `rel` before joining it onto the steering dir.
    /// A future caller that constructs an `InstalledSteering` in
    /// memory and bypasses the load path would otherwise be free to
    /// reach `steering_dir.join("../../etc/passwd")`.
    ///
    /// # Errors
    ///
    /// - [`crate::steering::SteeringError::NotInstalled`] if `rel`
    ///   has no entry in the tracking file. The cascade in
    ///   [`Self::remove_plugin`] treats this as a recoverable
    ///   orphan-tracking case (A-12).
    /// - I/O / JSON failures from loading or writing the tracking
    ///   file. A `NotFound` on the on-disk steering file itself is
    ///   treated as success — the user may have hand-deleted the
    ///   file before invoking remove.
    pub fn remove_steering_file(&self, rel: &Path) -> crate::error::Result<()> {
        if let Err(reason) = validate_tracking_path_entry(rel) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "refused to remove steering file with invalid path `{}`: {}",
                    rel.display(),
                    reason
                ),
            )
            .into());
        }

        let tracking_path = self.steering_tracking_path();
        crate::file_lock::with_file_lock(&tracking_path, || -> crate::error::Result<()> {
            let mut installed = self.load_installed_steering()?;
            let saved_meta = installed.files.remove(rel).ok_or_else(|| {
                crate::steering::SteeringError::NotInstalled {
                    rel: rel.to_path_buf(),
                }
            })?;

            let dest = self.steering_dir().join(rel);
            // Update tracking BEFORE unlinking so a crash between the
            // two leaves a stray on-disk file (harmless) rather than a
            // phantom tracking entry. Mirrors `remove_skill`'s ordering.
            self.write_steering_tracking(&installed)?;

            match fs::remove_file(&dest) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    debug!(
                        path = %dest.display(),
                        "steering file already absent on disk; tracking entry was orphan"
                    );
                }
                Err(e) => {
                    // I4: restore tracking on non-NotFound fs failure
                    // so the tracking file stays consistent with the
                    // file system (and a retry will see the same
                    // tracking entry).
                    warn!(
                        rel = %rel.display(),
                        path = %dest.display(),
                        error = %e,
                        "failed to unlink steering file after tracking update; \
                         restoring tracking entry"
                    );
                    installed.files.insert(rel.to_path_buf(), saved_meta);
                    if let Err(restore_err) = self.write_steering_tracking(&installed) {
                        warn!(
                            rel = %rel.display(),
                            error = %restore_err,
                            "failed to restore steering tracking entry — \
                             file may be untracked on disk"
                        );
                    }
                    return Err(e.into());
                }
            }

            debug!(rel = %rel.display(), "steering file removed");
            Ok(())
        })
    }

    /// Remove a single installed agent: drop the `installed-agents.json`
    /// entry and unlink both on-disk files (`<name>.json` and
    /// `prompts/<name>.md`). Mirrors [`Self::remove_steering_file`]'s
    /// shape — tracking written before file ops, `NotFound` treated
    /// as success.
    ///
    /// **Companions are not touched here.** A native plugin's
    /// `native_companions` map entry is shared across all of that
    /// plugin's agents; removing one agent must not nuke the plugin-wide
    /// companion bundle. The cascade in [`Self::remove_plugin`] calls
    /// [`Self::remove_native_companions_for_plugin`] separately after
    /// per-agent cleanup completes.
    ///
    /// # Errors
    ///
    /// - [`AgentError::NotInstalled`] if `name` has no tracking entry.
    /// - I/O / JSON failures from loading or writing the tracking file.
    pub fn remove_agent(&self, name: &str) -> crate::error::Result<()> {
        validation::validate_name(name)?;

        let tracking_path = self.agent_tracking_path();
        crate::file_lock::with_file_lock(&tracking_path, || -> crate::error::Result<()> {
            let mut installed = self.load_installed_agents()?;
            let saved_meta =
                installed
                    .agents
                    .remove(name)
                    .ok_or_else(|| AgentError::NotInstalled {
                        name: name.to_owned(),
                    })?;

            let json_target = self.agents_dir().join(format!("{name}.json"));
            let prompt_target = self.agent_prompts_dir().join(format!("{name}.md"));

            // Update tracking BEFORE unlinking so a crash between the
            // two leaves stray on-disk files (harmless) rather than a
            // phantom tracking entry.
            self.write_agent_tracking(&installed)?;

            for path in [&json_target, &prompt_target] {
                match fs::remove_file(path) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                        debug!(
                            path = %path.display(),
                            "agent file already absent on disk; tracking entry was orphan"
                        );
                    }
                    Err(e) => {
                        // I4: restore tracking on non-NotFound fs
                        // failure. The dual-file unlink can be
                        // half-successful — one file unlinked, the
                        // other failed. Restoring tracking keeps the
                        // file system consistent with tracking, even
                        // though the on-disk state is now partial.
                        // The caller can retry or inspect.
                        warn!(
                            agent = name,
                            path = %path.display(),
                            error = %e,
                            "failed to unlink agent file after tracking update; \
                             restoring tracking entry"
                        );
                        installed.agents.insert(name.to_owned(), saved_meta);
                        if let Err(restore_err) = self.write_agent_tracking(&installed) {
                            warn!(
                                agent = name,
                                error = %restore_err,
                                "failed to restore agent tracking entry — \
                                 agent may be untracked on disk"
                            );
                        }
                        return Err(e.into());
                    }
                }
            }

            debug!(agent = name, "agent removed");
            Ok(())
        })
    }

    /// Remove the `native_companions` tracking entry for a given
    /// `(plugin, marketplace)` pair and unlink every companion file
    /// recorded under it.
    ///
    /// The companions map is keyed by **plugin name alone**
    /// ([`InstalledAgents::native_companions`] is `HashMap<String, …>`),
    /// but each value carries a `marketplace` field that
    /// disambiguates same-named plugins across marketplaces (A-16).
    /// This method only removes the entry when **both** match. An
    /// entry belonging to a different marketplace, or no entry at
    /// all, is treated as a no-op so the cascade in
    /// [`Self::remove_plugin`] can call this unconditionally.
    ///
    /// On-disk companion files are unlinked best-effort via
    /// [`Self::remove_companion_files_best_effort`] — companions are
    /// installed at `agents_dir.join(rel)` (which can include
    /// `prompts/` paths shared with per-agent prompts), so the
    /// per-agent [`Self::remove_agent`] does not cover them.
    ///
    /// # Errors
    ///
    /// I/O or JSON failures from loading or writing
    /// `installed-agents.json`. The on-disk unlink is best-effort and
    /// does not propagate per-file failures.
    pub fn remove_native_companions_for_plugin(
        &self,
        plugin: &PluginName,
        marketplace: &MarketplaceName,
    ) -> crate::error::Result<()> {
        let tracking_path = self.agent_tracking_path();
        crate::file_lock::with_file_lock(&tracking_path, || -> crate::error::Result<()> {
            let mut installed = self.load_installed_agents()?;
            // The companions map is keyed by plugin NAME (a `String` —
            // out of scope per Phase 1.5 design — HashMap key migration
            // would require a follow-up). Look up by `plugin.as_str()`.
            // The marketplace check below uses newtype `PartialEq` to
            // stay A-16-correct.
            let should_remove = installed
                .native_companions
                .get(plugin.as_str())
                .is_some_and(|meta| meta.marketplace == *marketplace);
            if !should_remove {
                return Ok(());
            }

            // Take the entry so we can unlink the companion files
            // referenced by it after the tracking write succeeds.
            let Some(removed) = installed.native_companions.remove(plugin.as_str()) else {
                return Ok(());
            };

            self.write_agent_tracking(&installed)?;

            // Best-effort on-disk cleanup — see
            // `remove_companion_files_best_effort` for the symlink /
            // reparse-point defense and the rationale for warn!-and-
            // continue rather than propagating per-file failures.
            Self::remove_companion_files_best_effort(&removed.files, &self.agents_dir(), plugin);

            debug!(
                plugin = plugin.as_str(),
                marketplace = marketplace.as_str(),
                files = removed.files.len(),
                "native companions tracking entry removed"
            );
            Ok(())
        })
    }

    /// Cascade-remove every tracked entry from `(marketplace, plugin)`
    /// across all three `installed-*.json` tracking files. Unlinks
    /// the on-disk files, updates the tracking JSON files atomically,
    /// and returns per-content-type sub-results.
    ///
    /// **Orphan-tracking recovery (A-12 + I3).** If a tracking entry
    /// references a path that no longer exists on disk (state
    /// divergence: user ran `rm -rf .kiro/skills/<name>/` manually),
    /// the per-content `remove_*` drops the tracking row and treats
    /// the missing on-disk file as success. The cascade therefore
    /// counts the orphan as removed AND the tracking row no longer
    /// resurrects the plugin in `installed_plugins()`.
    ///
    /// **Per-step failures (I5).** A per-content removal that fails
    /// mid-cascade does NOT abort. The failure is recorded in the
    /// appropriate sub-result's `failures` vec (e.g.
    /// `result.skills.failures`); the cascade keeps going on the
    /// remaining content types so partial progress isn't lost. Same
    /// policy as `InstallPluginResult`'s sub-result `failed` vecs
    /// (A-15).
    ///
    /// **`native_companions` cleanup (A-3 + A-16).** After per-agent
    /// removals, the plugin-level `native_companions` entry is
    /// dropped if its `marketplace` field matches `marketplace` (the
    /// map is keyed by plugin name alone, so the marketplace check
    /// disambiguates same-named plugins across marketplaces). A
    /// failure here lands in `result.agents.failures` rather than
    /// short-circuiting the cascade.
    ///
    /// # Errors
    ///
    /// Reserved for "failed to even read the initial tracking files"
    /// (the three `load_installed*()` calls). Per-step errors during
    /// the loop go into the sub-results' `failures` vecs.
    /// Tracking-file loads can fail with I/O errors, JSON parse
    /// errors, or path-traversal validation errors (A-4).
    pub fn remove_plugin(
        &self,
        marketplace: &MarketplaceName,
        plugin: &PluginName,
    ) -> crate::error::Result<RemovePluginResult> {
        let mut result = RemovePluginResult::default();

        // Skills cascade
        let skills = self.load_installed()?;
        let skills_to_remove: Vec<String> = skills
            .skills
            .iter()
            .filter(|(_, meta)| meta.marketplace == *marketplace && meta.plugin == *plugin)
            .map(|(name, _)| name.clone())
            .collect();
        for name in &skills_to_remove {
            match self.remove_skill(name) {
                Ok(()) => {
                    result.skills.removed.push(name.clone());
                }
                Err(e) => {
                    warn!(
                        skill = %name,
                        plugin = plugin.as_str(),
                        marketplace = marketplace.as_str(),
                        error = %e,
                        "remove_plugin: skill removal failed; recording in failures"
                    );
                    result.skills.failures.push(RemoveItemFailure {
                        item: name.clone(),
                        error: crate::error::error_full_chain(&e),
                    });
                }
            }
        }

        // Steering cascade
        let steering = self.load_installed_steering()?;
        let steering_to_remove: Vec<PathBuf> = steering
            .files
            .iter()
            .filter(|(_, meta)| meta.marketplace == *marketplace && meta.plugin == *plugin)
            .map(|(rel, _)| rel.clone())
            .collect();
        for rel in &steering_to_remove {
            match self.remove_steering_file(rel) {
                Ok(()) => {
                    result.steering.removed.push(rel.display().to_string());
                }
                Err(e) => {
                    warn!(
                        rel = %rel.display(),
                        plugin = plugin.as_str(),
                        marketplace = marketplace.as_str(),
                        error = %e,
                        "remove_plugin: steering removal failed; recording in failures"
                    );
                    result.steering.failures.push(RemoveItemFailure {
                        item: rel.display().to_string(),
                        error: crate::error::error_full_chain(&e),
                    });
                }
            }
        }

        // Agents cascade
        let agents = self.load_installed_agents()?;
        let agents_to_remove: Vec<String> = agents
            .agents
            .iter()
            .filter(|(_, meta)| meta.marketplace == *marketplace && meta.plugin == *plugin)
            .map(|(name, _)| name.clone())
            .collect();
        for name in &agents_to_remove {
            match self.remove_agent(name) {
                Ok(()) => {
                    result.agents.removed.push(name.clone());
                }
                Err(e) => {
                    warn!(
                        agent = %name,
                        plugin = plugin.as_str(),
                        marketplace = marketplace.as_str(),
                        error = %e,
                        "remove_plugin: agent removal failed; recording in failures"
                    );
                    result.agents.failures.push(RemoveItemFailure {
                        item: name.clone(),
                        error: crate::error::error_full_chain(&e),
                    });
                }
            }
        }

        // Native companions cleanup (A-3 + A-16). Idempotent.
        if let Err(e) = self.remove_native_companions_for_plugin(plugin, marketplace) {
            warn!(
                plugin = plugin.as_str(),
                marketplace = marketplace.as_str(),
                error = %e,
                "remove_plugin: native_companions cleanup failed; recording in failures"
            );
            result.agents.failures.push(RemoveItemFailure {
                item: format!("native_companions:{}", plugin.as_str()),
                error: crate::error::error_full_chain(&e),
            });
        }

        Ok(result)
    }

    /// Persist the steering tracking file to disk atomically.
    fn write_steering_tracking(&self, installed: &InstalledSteering) -> crate::error::Result<()> {
        let path = self.steering_tracking_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(installed)?;
        crate::cache::atomic_write(&path, json.as_bytes())?;
        Ok(())
    }

    /// Promote a staged steering file into its final destination using
    /// the backup-then-swap pattern. In `forced_overwrite` mode any
    /// existing destination is renamed to `<dest>.kiro-bak` before the
    /// staging-rename so a later failure (tracking write) can restore
    /// the user's prior install.
    ///
    /// Returns the `(original, backup)` pairs the caller must restore on
    /// later failure or delete on success. Empty when nothing was backed
    /// up (clean install or non-existent destination).
    ///
    /// On rename failure, partially-promoted state is rolled back via
    /// [`Self::rollback_companion_promotion`] before returning.
    fn promote_staged_steering(
        staged_file: &Path,
        dest: &Path,
        forced_overwrite: bool,
    ) -> Result<Vec<(PathBuf, PathBuf)>, crate::steering::SteeringError> {
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent).map_err(|src| {
                crate::steering::SteeringError::DestinationDirFailed {
                    path: parent.to_path_buf(),
                    source: src,
                }
            })?;
        }

        let mut backups: Vec<(PathBuf, PathBuf)> = Vec::new();
        if forced_overwrite && dest.exists() {
            let backup = Self::companion_backup_path(dest);
            if let Err(src) = fs::rename(dest, &backup) {
                return Err(crate::steering::SteeringError::DestinationDirFailed {
                    path: dest.to_path_buf(),
                    source: src,
                });
            }
            backups.push((dest.to_path_buf(), backup));
        }

        if let Err(src) = fs::rename(staged_file, dest) {
            Self::rollback_companion_promotion(&[], &backups);
            return Err(crate::steering::SteeringError::DestinationDirFailed {
                path: dest.to_path_buf(),
                source: src,
            });
        }

        Ok(backups)
    }

    /// Stage a steering source file into a fresh [`tempfile::TempDir`]
    /// rooted under `.kiro/`, then compute `installed_hash` against the
    /// staged copy BEFORE any destructive op on `.kiro/steering/` (P-1).
    ///
    /// Staging mirrors the final layout (the file lands at `rel_path`
    /// under the staging dir) so hashing the staged copy yields the
    /// same value as hashing after promotion.
    ///
    /// Returns `(staging_dir, staged_file_path, installed_hash)` on
    /// success. The `TempDir` is RAII — on any later error the caller's
    /// `?` propagation triggers Drop which cleans up the staging dir.
    fn stage_steering_file(
        &self,
        source: &Path,
        rel_path: &Path,
    ) -> crate::error::Result<(tempfile::TempDir, PathBuf, String)> {
        let kiro_dir = self.kiro_dir();
        fs::create_dir_all(&kiro_dir).map_err(|src| {
            crate::steering::SteeringError::DestinationDirFailed {
                path: kiro_dir.clone(),
                source: src,
            }
        })?;
        let stem = rel_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("steering");
        let staging = tempfile::Builder::new()
            .prefix(&format!("_installing-steering-{stem}-"))
            .tempdir_in(&kiro_dir)
            .map_err(|src| crate::steering::SteeringError::StagingWriteFailed {
                path: kiro_dir.clone(),
                source: src,
            })?;

        let staged_file = staging.path().join(rel_path);
        if let Some(parent) = staged_file.parent() {
            fs::create_dir_all(parent).map_err(|src| {
                crate::steering::SteeringError::DestinationDirFailed {
                    path: parent.to_path_buf(),
                    source: src,
                }
            })?;
        }
        // Refuse hardlinked sources before allocating the read; see
        // `SteeringError::SourceHardlinked` (and the canonical statement
        // on `NativeParseFailure::HardlinkRefused`) for the threat model.
        // Windows lacks a portable nlink accessor in std — platform.rs's
        // reparse-point check covers junctions, the analogous Windows risk.
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let md = fs::symlink_metadata(source).map_err(|src| {
                crate::steering::SteeringError::SourceReadFailed {
                    path: source.to_path_buf(),
                    source: src,
                }
            })?;
            if md.is_file() && md.nlink() > 1 {
                return Err(crate::steering::SteeringError::SourceHardlinked {
                    path: source.to_path_buf(),
                    nlink: md.nlink(),
                }
                .into());
            }
        }

        let source_bytes =
            fs::read(source).map_err(|src| crate::steering::SteeringError::SourceReadFailed {
                path: source.to_path_buf(),
                source: src,
            })?;
        fs::write(&staged_file, &source_bytes).map_err(|src| {
            crate::steering::SteeringError::StagingWriteFailed {
                path: staged_file.clone(),
                source: src,
            }
        })?;

        let installed_hash = crate::hash::hash_artifact(
            staging.path(),
            std::slice::from_ref(&rel_path.to_path_buf()),
        )
        .map_err(|src| crate::steering::SteeringError::HashFailed {
            path: staged_file.clone(),
            source: src,
        })?;

        Ok((staging, staged_file, installed_hash))
    }

    /// Decide what `install_steering_file` should do given the existing
    /// tracking state and on-disk state. Mirrors
    /// [`Self::classify_native_collision`] over steering's collision matrix:
    ///
    /// 1. Tracked + same plugin + same hash → idempotent no-op.
    /// 2. Tracked + same plugin + different hash → `ContentChangedRequiresForce`
    ///    (or proceed-with-`forced_overwrite` under [`InstallMode::Force`]).
    /// 3. Tracked + different plugin → `PathOwnedByOtherPlugin`
    ///    (or proceed-with-`forced_overwrite`).
    /// 4. Untracked + on-disk → `OrphanFileAtDestination`
    ///    (or proceed-with-`forced_overwrite`).
    /// 5. Untracked + clean → `Proceed { forced_overwrite: false }`.
    ///
    /// Exhaustive over the same-plugin / cross-plugin / orphan / clean
    /// states — no `_ => default` arms.
    ///
    /// [`InstallMode::Force`]: crate::service::InstallMode::Force
    fn classify_steering_collision(
        installed: &InstalledSteering,
        rel_path: &Path,
        plugin: &PluginName,
        source_hash: &str,
        dest: &Path,
        mode: crate::service::InstallMode,
    ) -> Result<CollisionDecision<SteeringIdempotentEcho>, crate::steering::SteeringError> {
        match installed.files.get(rel_path) {
            Some(existing) if existing.plugin == *plugin => {
                if existing.source_hash == source_hash {
                    return Ok(CollisionDecision::Idempotent(Box::new(
                        SteeringIdempotentEcho {
                            prior_installed_hash: existing.installed_hash.clone(),
                        },
                    )));
                }
                if !mode.is_force() {
                    return Err(
                        crate::steering::SteeringError::ContentChangedRequiresForce {
                            rel: rel_path.to_path_buf(),
                        },
                    );
                }
                Ok(CollisionDecision::Proceed {
                    forced_overwrite: true,
                })
            }
            Some(existing) => {
                if !mode.is_force() {
                    return Err(crate::steering::SteeringError::PathOwnedByOtherPlugin {
                        rel: rel_path.to_path_buf(),
                        owner: existing.plugin.clone(),
                    });
                }
                Ok(CollisionDecision::Proceed {
                    forced_overwrite: true,
                })
            }
            None if dest.exists() => {
                if !mode.is_force() {
                    return Err(crate::steering::SteeringError::OrphanFileAtDestination {
                        path: dest.to_path_buf(),
                    });
                }
                Ok(CollisionDecision::Proceed {
                    forced_overwrite: true,
                })
            }
            None => Ok(CollisionDecision::Proceed {
                forced_overwrite: false,
            }),
        }
    }

    /// Install one steering file into `.kiro/steering/`.
    ///
    /// `source.scan_root` is the plugin's steering scan directory; the
    /// file's relative path under that root is also its tracking key under
    /// `.kiro/steering/`. The same path-as-key invariant means cross-plugin
    /// collisions surface naturally without any plugin-wide bundle concept.
    ///
    /// # Collision semantics
    ///
    /// - **Idempotent reinstall** (same plugin + same `source_hash`): no
    ///   bytes written, returns the prior `installed_hash`.
    /// - **Same plugin, different `source_hash`**:
    ///   [`SteeringError::ContentChangedRequiresForce`] under
    ///   [`InstallMode::New`]; under [`InstallMode::Force`] the existing
    ///   file is backed up, replaced, and the backup deleted on success.
    /// - **Different plugin**: [`SteeringError::PathOwnedByOtherPlugin`]
    ///   under [`InstallMode::New`]; under [`InstallMode::Force`]
    ///   ownership transfers and the previous owner's tracking entry is
    ///   overwritten.
    /// - **Untracked file on disk**:
    ///   [`SteeringError::OrphanFileAtDestination`] under
    ///   [`InstallMode::New`]; under [`InstallMode::Force`] the orphan is
    ///   overwritten and ownership recorded.
    ///
    /// # Atomicity
    ///
    /// Adopts the staging-before-rename + backup-then-swap pattern:
    /// `installed_hash` is computed against the staged copy *before* any
    /// destructive op on `.kiro/steering/`. In force mode, the existing
    /// destination is renamed to `<dest>.kiro-bak` before the staging
    /// rename; on tracking-write failure the backup is restored and the
    /// new file removed. Same guarantee as
    /// [`Self::install_native_agent`].
    ///
    /// Staging lives under `.kiro/` (NOT inside `.kiro/steering/`) via
    /// [`tempfile::TempDir`] — RAII Drop cleans up on every code path,
    /// including panics.
    ///
    /// # Errors
    ///
    /// See the collision matrix above for user-facing errors. All
    /// infrastructure failures (I/O, hash, JSON) carry the offending
    /// `path: PathBuf` for easier debugging.
    ///
    /// [`InstallMode::New`]: crate::service::InstallMode::New
    /// [`InstallMode::Force`]: crate::service::InstallMode::Force
    pub fn install_steering_file(
        &self,
        source: &crate::agent::DiscoveredNativeFile,
        source_hash: &str,
        ctx: crate::steering::SteeringInstallContext<'_>,
    ) -> Result<crate::steering::InstalledSteeringOutcome, crate::steering::SteeringError> {
        let rel_path = match source.source.strip_prefix(&source.scan_root) {
            Ok(p) => p.to_path_buf(),
            Err(_) => {
                return Err(crate::steering::SteeringError::SourceReadFailed {
                    path: source.source.clone(),
                    source: std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "steering source not under scan_root",
                    ),
                });
            }
        };
        let dest = self.steering_dir().join(&rel_path);
        let tracking_path = self.steering_tracking_path();

        let result: crate::error::Result<crate::steering::InstalledSteeringOutcome> =
            crate::file_lock::with_file_lock(&tracking_path, || {
                self.install_steering_file_locked(source, &rel_path, &dest, source_hash, ctx)
            });

        result.map_err(|e| match e {
            crate::error::Error::Steering(steering_err) => steering_err,
            crate::error::Error::Json(json_err) => {
                crate::steering::tracking_malformed(tracking_path, &json_err)
            }
            other => crate::steering::SteeringError::TrackingIoFailed {
                path: tracking_path,
                // error_full_chain walks #[source] so the underlying
                // io::Error or hash failure reaches the user (CLAUDE.md
                // FFI rule). `to_string()` would drop everything below
                // Error's top-level Display.
                source: std::io::Error::other(crate::error::error_full_chain(&other)),
            },
        })
    }

    /// Inside-the-lock body of [`Self::install_steering_file`]. Extracted
    /// to keep the public entry point small; the closure-with-lock dance
    /// and the error-projection live in the caller.
    fn install_steering_file_locked(
        &self,
        source: &crate::agent::DiscoveredNativeFile,
        rel_path: &Path,
        dest: &Path,
        source_hash: &str,
        ctx: crate::steering::SteeringInstallContext<'_>,
    ) -> crate::error::Result<crate::steering::InstalledSteeringOutcome> {
        let tracking_path = self.steering_tracking_path();
        let mut installed = self.load_installed_steering().map_err(|e| match e {
            // A malformed installed-steering.json is a distinct condition
            // from "couldn't read the file at all" — give it the typed
            // variant the steering error surface declares.
            crate::error::Error::Json(json_err) => {
                crate::steering::tracking_malformed(tracking_path.clone(), &json_err)
            }
            other => crate::steering::SteeringError::TrackingIoFailed {
                path: tracking_path.clone(),
                source: std::io::Error::other(crate::error::error_full_chain(&other)),
            },
        })?;

        let forced_overwrite = match Self::classify_steering_collision(
            &installed,
            rel_path,
            ctx.plugin,
            source_hash,
            dest,
            ctx.mode,
        )? {
            // Idempotent: assemble the full outcome here where `source.source`
            // is in scope. The classifier returned only the prior installed_hash
            // so it couldn't accidentally substitute `dest` for the missing
            // source path.
            CollisionDecision::Idempotent(echo) => {
                return Ok(crate::steering::InstalledSteeringOutcome {
                    source: source.source.clone(),
                    destination: dest.to_path_buf(),
                    kind: InstallOutcomeKind::Idempotent,
                    source_hash: source_hash.to_owned(),
                    installed_hash: echo.prior_installed_hash,
                });
            }
            CollisionDecision::Proceed { forced_overwrite } => forced_overwrite,
        };

        // Compute source_scan_root BEFORE staging/promotion. The
        // validation has no dependency on staging (only on
        // source.scan_root + ctx.plugin_dir), and a defensive failure
        // here AFTER promote would leak placed files + backups since
        // bare ? skips rollback. Atomicity contract: every failure
        // mode after promote_staged_steering must call
        // rollback_companion_promotion; computing this upfront keeps
        // the contract honest by removing one failure source from the
        // post-promote span.
        let source_scan_root = required_steering_scan_root(&source.scan_root, ctx.plugin_dir)?;

        // `_staging` is held as a RAII guard so its TempDir Drop sweeps
        // the staging directory at end-of-scope, including on early
        // returns from the rest of this function.
        let (_staging, staged_file, installed_hash) =
            self.stage_steering_file(&source.source, rel_path)?;

        let backups = Self::promote_staged_steering(&staged_file, dest, forced_overwrite)?;
        let placed = [dest.to_path_buf()];

        // If we're transferring ownership from another plugin, scrub the
        // prior owner's entry so the same path isn't tracked twice.
        if let Some(existing) = installed.files.get(rel_path)
            && existing.plugin != *ctx.plugin
        {
            installed.files.remove(rel_path);
        }

        installed.files.insert(
            rel_path.to_path_buf(),
            InstalledSteeringMeta {
                marketplace: ctx.marketplace.clone(),
                plugin: ctx.plugin.clone(),
                version: ctx.version.map(str::to_owned),
                installed_at: chrono::Utc::now(),
                source_hash: source_hash.to_owned(),
                installed_hash: installed_hash.clone(),
                source_scan_root,
            },
        );

        if let Err(e) = self.write_steering_tracking(&installed) {
            warn!(
                rel = %rel_path.display(),
                error = %e,
                "steering tracking update failed; restoring backups"
            );
            Self::rollback_companion_promotion(&placed, &backups);
            return Err(crate::steering::SteeringError::TrackingIoFailed {
                path: tracking_path,
                source: std::io::Error::other(crate::error::error_full_chain(&e)),
            }
            .into());
        }

        // Success — drop backup files. Best-effort.
        for (_orig, backup) in &backups {
            if let Err(e) = fs::remove_file(backup)
                && e.kind() != std::io::ErrorKind::NotFound
            {
                warn!(
                    path = %backup.display(),
                    error = %e,
                    "failed to remove steering backup after success"
                );
            }
        }

        debug!(
            rel = %rel_path.display(),
            force = ctx.mode.is_force(),
            "steering file installed"
        );

        Ok(crate::steering::InstalledSteeringOutcome {
            source: source.source.clone(),
            destination: dest.to_path_buf(),
            kind: if forced_overwrite {
                InstallOutcomeKind::ForceOverwrote
            } else {
                InstallOutcomeKind::Installed
            },
            source_hash: source_hash.to_owned(),
            installed_hash,
        })
    }

    /// Install a parsed agent into the Kiro project.
    ///
    /// Pass `source_path` as the `.md` file the definition was parsed from to
    /// populate `source_hash` in the tracking entry. Pass `None` to leave it
    /// unrecorded (e.g. for synthetic test agents).
    ///
    /// # Errors
    ///
    /// - [`AgentError::AlreadyInstalled`] if an agent with this name already exists.
    /// - Validation errors for unsafe names.
    /// - I/O errors or JSON serialization failures.
    pub fn install_agent(
        &self,
        def: &AgentDefinition,
        mapped_tools: &[MappedTool],
        meta: InstalledAgentMeta,
        source_path: Option<&Path>,
    ) -> crate::error::Result<()> {
        self.install_agent_inner(def, mapped_tools, meta, false, source_path)
    }

    /// Install a parsed agent, overwriting any existing agent of the same
    /// name. Mirrors [`install_skill_from_dir_force`] for the agent path so
    /// the CLI's `--force` flag can honor its documented contract.
    ///
    /// Pass `source_path` as the `.md` file the definition was parsed from to
    /// populate `source_hash` in the tracking entry. Pass `None` to leave it
    /// unrecorded (e.g. for synthetic test agents).
    ///
    /// If an agent with the same name is already tracked, its JSON + prompt
    /// files are removed before the new ones are renamed into place. Orphaned
    /// files on disk (no tracking entry) are also removed rather than
    /// rejected, since the caller has explicitly opted into overwrite.
    ///
    /// # Errors
    ///
    /// - Validation errors for unsafe names.
    /// - I/O errors or JSON serialization failures.
    pub fn install_agent_force(
        &self,
        def: &AgentDefinition,
        mapped_tools: &[MappedTool],
        meta: InstalledAgentMeta,
        source_path: Option<&Path>,
    ) -> crate::error::Result<()> {
        self.install_agent_inner(def, mapped_tools, meta, true, source_path)
    }

    /// Hash a translated-agent source file against its parent
    /// directory + filename. Returns `Ok(None)` for `None` input
    /// (test fixtures sometimes synthesize an `AgentDefinition` from
    /// thin air); otherwise delegates to [`crate::hash::hash_artifact`]
    /// for the canonical hash format. Lifted out of `install_agent_inner`
    /// to keep that function under the line cap.
    fn hash_translated_source(source_path: Option<&Path>) -> crate::error::Result<String> {
        // PR #100 review I2 tightened `InstalledAgentMeta::source_hash`
        // to required `String` (was `Option<String>`). Production
        // callers always pass `Some(path)` — the only `None` callers
        // are test fixtures that synthesise an `AgentDefinition` from
        // thin air and don't care about drift detection. For those the
        // empty-string sentinel is the deliberate "no source-side
        // hash recorded" marker; in production it's never observed.
        let Some(p) = source_path else {
            return Ok(String::new());
        };
        // Production callers always pass a fully-qualified file path so
        // both `parent()` and `file_name()` are guaranteed `Some`. The
        // typed error branches exist so test fixtures and any future
        // exotic caller (e.g. `Path::new("/")`, a bare filename) get a
        // structured error instead of a panic.
        let parent = p.parent().ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("source path `{}` has no parent dir", p.display()),
            )
        })?;
        let filename = p.file_name().ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("source path `{}` has no file name", p.display()),
            )
        })?;
        Ok(crate::hash::hash_artifact(
            parent,
            &[std::path::PathBuf::from(filename)],
        )?)
    }

    fn install_agent_inner(
        &self,
        def: &AgentDefinition,
        mapped_tools: &[MappedTool],
        mut meta: InstalledAgentMeta,
        force: bool,
        source_path: Option<&Path>,
    ) -> crate::error::Result<()> {
        validation::validate_name(&def.name)?;

        // CPU-bound work outside the lock to keep the critical section short.
        let json = crate::agent::emit::build_kiro_json(def, mapped_tools)?;
        let json_bytes = serde_json::to_vec_pretty(&json)?;

        // Compute source_hash outside the lock — it's a read-only I/O
        // operation on the source file and need not block other installers.
        let source_hash = Self::hash_translated_source(source_path)?;

        crate::file_lock::with_file_lock(
            &self.agent_tracking_path(),
            || -> crate::error::Result<()> {
                let mut installed = self.load_installed_agents()?;
                if !force && installed.agents.contains_key(&def.name) {
                    return Err(AgentError::AlreadyInstalled {
                        name: def.name.clone(),
                    }
                    .into());
                }

                let (staging, json_rel, prompt_rel, installed_hash) =
                    self.stage_agent_files(&def.name, &json_bytes, def.prompt_body.as_bytes())?;

                let PromotedAgent {
                    json_target,
                    prompt_target,
                    backups,
                } = self.promote_staged_agent(
                    &def.name,
                    staging.path(),
                    &json_rel,
                    &prompt_rel,
                    force,
                )?;
                // staging is a TempDir and drops at end of scope, cleaning
                // up the now-empty staging directory.

                // installed_hash was computed pre-destructive (against staging).
                let agents_root = self.agents_dir(); // needed for companion hash below

                meta.source_hash = source_hash;
                meta.installed_hash = installed_hash;

                // Capture plugin identity before moving meta into the map.
                let marketplace = meta.marketplace.clone();
                let plugin = meta.plugin.clone();
                let version = meta.version.clone();
                // Source-side scan root for the synthesized companion
                // entry: prompt is extracted from agent .md, so use
                // the agent's source_path.parent() (typically
                // "agents") by convention. C-1 (issue #99) tracks
                // install/detect hash recipe alignment.
                let companion_scan_root = meta
                    .source_path
                    .as_str()
                    .rsplit_once('/')
                    .and_then(|(parent, _)| crate::validation::RelativePath::new(parent).ok())
                    .unwrap_or_else(crate::validation::RelativePath::agents_root);

                installed.agents.insert(def.name.clone(), meta);

                // Cross-plugin force-transfer: if a prior owner of the
                // same prompt path is some OTHER plugin, scrub the prompt
                // from its `native_companions.files` so the entry no
                // longer claims a file we just overwrote. Mirrors the
                // native path's call in `install_native_companions_locked`
                // and closes the v1 gap documented in Stage 1 Task 14.
                //
                // Only meaningful in force mode — non-force mode bails
                // earlier on `AlreadyInstalled` so no transfer happens.
                let placed = [json_target.clone(), prompt_target.clone()];

                if force
                    && let Err(e) = Self::strip_transferred_paths_from_other_plugins(
                        &mut installed,
                        &plugin,
                        std::slice::from_ref(&prompt_rel),
                        &agents_root,
                    )
                {
                    warn!(
                        name = %def.name,
                        error = %e,
                        "cross-plugin transfer hash recompute failed; restoring backups"
                    );
                    Self::rollback_companion_promotion(&placed, &backups);
                    return Err(e);
                }

                if let Err(e) = Self::synthesize_companion_entry(
                    &mut installed,
                    &CompanionInput {
                        marketplace: &marketplace,
                        plugin: &plugin,
                        version: version.as_deref(),
                        agents_root: &agents_root,
                        prompt_rel: &prompt_rel,
                        source_scan_root: &companion_scan_root,
                    },
                ) {
                    warn!(
                        name = %def.name,
                        error = %e,
                        "companion entry synthesis failed after rename; restoring backups"
                    );
                    Self::rollback_companion_promotion(&placed, &backups);
                    return Err(e);
                }

                if let Err(e) = self.write_agent_tracking(&installed) {
                    warn!(
                        name = %def.name,
                        error = %e,
                        "agent tracking update failed after rename; restoring backups"
                    );
                    Self::rollback_companion_promotion(&placed, &backups);
                    return Err(e);
                }

                Self::drop_install_backups_best_effort(&backups);
                debug!(name = %def.name, force, "agent installed");
                Ok(())
            },
        )
    }

    /// Best-effort removal of `.kiro-bak` backup files after a successful
    /// install. An orphan backup is a curiosity, not a correctness issue,
    /// so failures are logged at `warn!` and don't surface to the caller.
    fn drop_install_backups_best_effort(backups: &[(PathBuf, PathBuf)]) {
        for (_orig, backup) in backups {
            if let Err(e) = fs::remove_file(backup)
                && e.kind() != std::io::ErrorKind::NotFound
            {
                warn!(
                    path = %backup.display(),
                    error = %e,
                    "failed to remove install backup after success"
                );
            }
        }
    }

    /// Move staged agent files from `staging` into their final locations
    /// under `agents_root` using a backup-then-swap promote. In force mode,
    /// each existing target is renamed to `<dest>.kiro-bak` before the
    /// staging-rename so a later failure (companion hash, tracking write)
    /// can restore the user's prior install rather than leaving the
    /// destination empty. In non-force mode, any pre-existing target file
    /// (e.g. from a prior crash) causes an `AlreadyExists` error without
    /// touching `agents_root`.
    ///
    /// On any rename failure, partially-promoted state is rolled back via
    /// [`Self::rollback_companion_promotion`] before returning.
    ///
    /// Returns the [`PromotedAgent`] (target paths plus original→backup
    /// pairs) so the caller can restore on later failure or drop the
    /// backups on success.
    fn promote_staged_agent(
        &self,
        name: &str,
        staging: &Path,
        json_rel: &Path,
        prompt_rel: &Path,
        force: bool,
    ) -> crate::error::Result<PromotedAgent> {
        // The caller passes `staging.path()` from a `tempfile::TempDir`
        // that drops at the caller's scope exit, so any error return
        // below propagates and the caller's TempDir Drop cleans up.
        let staging_json = staging.join(json_rel);
        let staging_prompt = staging.join(prompt_rel);

        fs::create_dir_all(self.agent_prompts_dir())?;

        let json_target = self.agents_dir().join(format!("{name}.json"));
        let prompt_target = self.agent_prompts_dir().join(format!("{name}.md"));

        if !force && (json_target.exists() || prompt_target.exists()) {
            // Non-force install: a prior crash could leave orphaned files
            // on disk without a tracking entry. Refuse to silently clobber
            // — the user either manually cleans up or re-invokes with
            // `install_agent_force`.
            return Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                format!(
                    "agent files for `{name}` exist on disk but have no tracking entry; \
                     remove {} and {} manually before re-installing",
                    json_target.display(),
                    prompt_target.display(),
                ),
            )
            .into());
        }

        // Backup phase — back up each existing target so a later failure
        // can restore the user's prior install. Reuses the
        // `companion_backup_path` / `rollback_companion_promotion` helpers
        // so the backup-suffix convention stays uniform across install
        // paths.
        let mut backups: Vec<(PathBuf, PathBuf)> = Vec::new();
        if force {
            for target in [&json_target, &prompt_target] {
                if target.exists() {
                    let backup = Self::companion_backup_path(target);
                    if let Err(e) = fs::rename(target, &backup) {
                        Self::rollback_companion_promotion(&[], &backups);
                        return Err(e.into());
                    }
                    backups.push((target.clone(), backup));
                }
            }
        }

        // Promote phase — rename JSON first, then prompt. On any failure,
        // roll back already-placed files and restore backups.
        if let Err(e) = fs::rename(&staging_json, &json_target) {
            Self::rollback_companion_promotion(&[], &backups);
            return Err(e.into());
        }
        if let Err(e) = fs::rename(&staging_prompt, &prompt_target) {
            Self::rollback_companion_promotion(std::slice::from_ref(&json_target), &backups);
            return Err(e.into());
        }

        Ok(PromotedAgent {
            json_target,
            prompt_target,
            backups,
        })
    }

    /// Write agent JSON and prompt into a fresh staging directory, then compute
    /// `installed_hash` against the staged copies BEFORE any destructive
    /// operations on `agents_root`. Returns `(staging, json_rel, prompt_rel,
    /// installed_hash)`. On any failure the staging directory is cleaned up and
    /// an error is returned — `agents_root` is guaranteed untouched.
    ///
    /// Staging mirrors the final layout (`<name>.json` + `prompts/<name>.md`)
    /// so hashing staging with `agents_root`-relative paths yields the same
    /// value as hashing after rename.
    fn stage_agent_files(
        &self,
        name: &str,
        json_bytes: &[u8],
        prompt_bytes: &[u8],
    ) -> crate::error::Result<(tempfile::TempDir, PathBuf, PathBuf, String)> {
        // TempDir RAII: any `?` propagation below cleans up the staging
        // dir on Drop, so error branches don't need explicit cleanup.
        let staging = tempfile::Builder::new()
            .prefix(&format!("_installing-agent-{name}-"))
            .tempdir_in(self.kiro_dir())?;
        let json_rel = PathBuf::from(format!("{name}.json"));
        let prompt_rel = PathBuf::from(format!("prompts/{name}.md"));
        let staging_json = staging.path().join(&json_rel);
        let staging_prompt_dir = staging.path().join("prompts");
        let staging_prompt = staging.path().join(&prompt_rel);

        fs::create_dir_all(&staging_prompt_dir)?;
        fs::write(&staging_json, json_bytes)
            .and_then(|()| fs::write(&staging_prompt, prompt_bytes))?;

        let installed_hash = match crate::hash::hash_artifact(
            staging.path(),
            &[json_rel.clone(), prompt_rel.clone()],
        ) {
            Ok(h) => h,
            Err(e) => {
                warn!(
                    name,
                    error = %e,
                    "installed_hash computation failed on staging; removing staging dir"
                );
                return Err(e.into());
            }
        };

        Ok((staging, json_rel, prompt_rel, installed_hash))
    }

    /// Synthesize/update the per-plugin `native_companions` tracking entry
    /// to register this agent's prompt file as plugin-owned. Called from
    /// the translated agent install path.
    ///
    /// Recomputes the per-plugin companion hash over the full union of
    /// prompt files for this plugin. On hash failure, returns the error
    /// without touching `agents_root` — the caller (`install_agent_inner`)
    /// owns the backup-restore path because it's the only frame that
    /// holds the [`PromotedAgent::backups`] from the promote phase.
    ///
    /// # Atomicity
    ///
    /// Pairs with [`Self::promote_staged_agent`]'s backup-then-swap to
    /// give force-mode translated installs the same all-or-nothing
    /// guarantee the native install paths have: a hash failure here
    /// triggers a backup restore in the caller, leaving the user's prior
    /// install on disk.
    fn synthesize_companion_entry(
        installed: &mut InstalledAgents,
        input: &CompanionInput<'_>,
    ) -> crate::error::Result<()> {
        // Synthesize/update the companion entry for this plugin's prompt
        // files. We track the union of installed prompt paths so the
        // native install path sees them as plugin-owned, not orphaned.
        //
        // Hash semantics: source_hash == installed_hash because the
        // translated path does not separately track original .md source
        // files; both equal the hash over the prompt-bundle bytes.
        //
        // The HashMap key is still `String` (out of scope per Phase 1.5
        // design); only the meta-value's `marketplace`/`plugin` fields
        // adopt the newtypes.
        let companion_entry = installed
            .native_companions
            .entry(input.plugin.as_str().to_owned())
            .or_insert_with(|| InstalledNativeCompanionsMeta {
                marketplace: input.marketplace.clone(),
                plugin: input.plugin.clone(),
                version: input.version.map(str::to_owned),
                installed_at: chrono::Utc::now(),
                files: Vec::new(),
                source_hash: String::new(),
                installed_hash: String::new(),
                source_scan_root: input.source_scan_root.clone(),
            });
        // Refresh marketplace/version/timestamp + source_scan_root on
        // every install. PR #100 review I3: the post-insert refresh
        // initially missed `source_scan_root`, so a manifest that
        // changed its scan paths between installs would keep the stale
        // root recorded forever — the install-time scan_root is what
        // detection consults to locate the source side, so a stale
        // value is a latent false-drift bug. Issue #99 currently masks
        // this for translated companions (they don't yet read the
        // field at detect time), but the moment that lands the
        // staleness becomes a real bug.
        companion_entry.marketplace = input.marketplace.clone();
        companion_entry.version = input.version.map(str::to_owned);
        companion_entry.installed_at = chrono::Utc::now();
        companion_entry.source_scan_root = input.source_scan_root.clone();
        if !companion_entry
            .files
            .contains(&input.prompt_rel.to_path_buf())
        {
            companion_entry.files.push(input.prompt_rel.to_path_buf());
        }
        // Recompute hashes over the full prompt set for this plugin.
        let companion_files_snapshot = companion_entry.files.clone();
        let companion_hash =
            crate::hash::hash_artifact(input.agents_root, &companion_files_snapshot)?;
        companion_entry.source_hash = companion_hash.clone();
        companion_entry.installed_hash = companion_hash;
        Ok(())
    }

    /// Decide what `install_native_agent` should do given the existing
    /// tracking state and on-disk state. Returns either an early-exit
    /// idempotent outcome or a `forced_overwrite` flag for the caller
    /// to thread through staging + promote.
    ///
    /// The classifier is exhaustive over the five possible states:
    /// (a) tracked + same plugin + same hash → idempotent no-op,
    /// (b) tracked + same plugin + different hash → `ContentChanged`,
    /// (c) tracked + different plugin → `NameClash`,
    /// (d) untracked + file on disk → `Orphan`,
    /// (e) untracked + clean destination → clean install.
    /// Each (b)/(c)/(d) is a hard error under [`InstallMode::New`] and a
    /// `forced_overwrite: true` proceed under [`InstallMode::Force`].
    fn classify_native_collision(
        installed: &InstalledAgents,
        agent_name: &str,
        plugin: &PluginName,
        source_hash: &str,
        json_target: &Path,
        mode: crate::service::InstallMode,
    ) -> crate::error::Result<CollisionDecision<InstalledNativeAgentOutcome>> {
        match installed.agents.get(agent_name) {
            Some(existing) if existing.plugin == *plugin => {
                if existing.source_hash == source_hash {
                    return Ok(CollisionDecision::Idempotent(Box::new(
                        InstalledNativeAgentOutcome {
                            name: agent_name.to_owned(),
                            json_path: json_target.to_path_buf(),
                            kind: InstallOutcomeKind::Idempotent,
                            source_hash: source_hash.to_owned(),
                            installed_hash: existing.installed_hash.clone(),
                        },
                    )));
                }
                if !mode.is_force() {
                    return Err(AgentError::ContentChangedRequiresForce {
                        name: agent_name.to_owned(),
                    }
                    .into());
                }
                Ok(CollisionDecision::Proceed {
                    forced_overwrite: true,
                })
            }
            Some(existing) => {
                if !mode.is_force() {
                    return Err(AgentError::NameClashWithOtherPlugin {
                        name: agent_name.to_owned(),
                        // `owner` is the wire-format `String` field on
                        // `AgentError`. We project the newtype to its
                        // string view via `Display` so consumers see the
                        // same value they did before the migration.
                        owner: existing.plugin.to_string(),
                    }
                    .into());
                }
                Ok(CollisionDecision::Proceed {
                    forced_overwrite: true,
                })
            }
            None if json_target.exists() => {
                if !mode.is_force() {
                    return Err(AgentError::OrphanFileAtDestination {
                        path: json_target.to_path_buf(),
                    }
                    .into());
                }
                Ok(CollisionDecision::Proceed {
                    forced_overwrite: true,
                })
            }
            None => Ok(CollisionDecision::Proceed {
                forced_overwrite: false,
            }),
        }
    }

    /// Install one native Kiro agent JSON.
    ///
    /// Writes [`NativeAgentBundle::raw_bytes`] verbatim to
    /// `.kiro/agents/<name>.json` and records the installation in
    /// `installed-agents.json` with [`AgentDialect::Native`].
    ///
    /// # Collision semantics
    ///
    /// The behavior on a name collision depends on `mode` and on what's
    /// already tracked at this name:
    ///
    /// - **Idempotent reinstall**: same plugin, same `source_hash`. The
    ///   call is a verified no-op and returns the prior `installed_hash`.
    /// - **Same plugin, different `source_hash`**: returns
    ///   [`AgentError::ContentChangedRequiresForce`] under
    ///   [`InstallMode::New`]; under [`InstallMode::Force`] the existing
    ///   file is backed up, replaced, and the backup deleted on success.
    /// - **Different plugin**: returns
    ///   [`AgentError::NameClashWithOtherPlugin`] under
    ///   [`InstallMode::New`]; under [`InstallMode::Force`] ownership
    ///   transfers and the previous owner's tracking entry is overwritten.
    /// - **No tracking entry but file exists on disk**: returns
    ///   [`AgentError::OrphanFileAtDestination`] under
    ///   [`InstallMode::New`]; under [`InstallMode::Force`] the orphan
    ///   is overwritten and ownership recorded.
    ///
    /// # Atomicity
    ///
    /// Adopts the staging-before-rename + backup-then-swap pattern:
    /// `installed_hash` is computed against the staged copy *before* any
    /// destructive op on `.kiro/agents/`. In force mode, the existing
    /// destination is renamed to `<name>.json.kiro-bak` before the
    /// staging-rename; on tracking-write failure the backup is restored
    /// and the new file removed. This closes the data-loss window where
    /// a hash or tracking failure mid-install would otherwise leave the
    /// user with no install on disk.
    ///
    /// # Errors
    ///
    /// - [`AgentError::ContentChangedRequiresForce`] /
    ///   [`AgentError::NameClashWithOtherPlugin`] /
    ///   [`AgentError::OrphanFileAtDestination`] per the collision matrix.
    /// - [`AgentError::InstallFailed`] for any I/O / hash / tracking
    ///   failure during stage / promote / write.
    ///
    /// [`InstallMode::New`]: crate::service::InstallMode::New
    /// [`InstallMode::Force`]: crate::service::InstallMode::Force
    pub fn install_native_agent(
        &self,
        input: &NativeAgentInstallInput<'_>,
    ) -> Result<InstalledNativeAgentOutcome, AgentError> {
        let json_target = self
            .agents_dir()
            .join(format!("{}.json", &input.bundle.name));
        let agent_name = input.bundle.name.to_string();
        let json_target_for_err = json_target.clone();

        let result: crate::error::Result<InstalledNativeAgentOutcome> =
            crate::file_lock::with_file_lock(&self.agent_tracking_path(), || {
                let mut installed = self.load_installed_agents()?;

                // Collision matrix — return early or set `forced_overwrite`.
                let forced_overwrite = match Self::classify_native_collision(
                    &installed,
                    &agent_name,
                    input.plugin,
                    input.source_hash,
                    &json_target,
                    input.mode,
                )? {
                    CollisionDecision::Idempotent(outcome) => return Ok(*outcome),
                    CollisionDecision::Proceed { forced_overwrite } => forced_overwrite,
                };

                let (staging, json_rel, installed_hash) =
                    self.stage_native_agent_file(&agent_name, &input.bundle.raw_bytes)?;

                let backup = self.promote_native_agent(
                    staging.path(),
                    &json_rel,
                    &json_target,
                    forced_overwrite,
                )?;
                // staging is a TempDir; drops at scope exit and cleans
                // up the now-empty staging directory.

                installed.agents.insert(
                    agent_name.clone(),
                    InstalledAgentMeta {
                        marketplace: input.marketplace.clone(),
                        plugin: input.plugin.clone(),
                        version: input.version.map(String::from),
                        installed_at: chrono::Utc::now(),
                        dialect: AgentDialect::Native,
                        source_path: input.source_path.clone(),
                        source_hash: input.source_hash.to_string(),
                        installed_hash: installed_hash.clone(),
                    },
                );

                if let Err(e) = self.write_agent_tracking(&installed) {
                    warn!(
                        name = %agent_name,
                        error = %e,
                        "agent tracking update failed; rolling back files"
                    );
                    if let Err(rb_err) = fs::remove_file(&json_target)
                        && rb_err.kind() != std::io::ErrorKind::NotFound
                    {
                        warn!(
                            path = %json_target.display(),
                            error = %rb_err,
                            "failed to remove placed agent JSON during rollback"
                        );
                    }
                    if let Some(ref bak) = backup
                        && let Err(restore_err) = fs::rename(bak, &json_target)
                    {
                        warn!(
                            backup = %bak.display(),
                            target = %json_target.display(),
                            error = %restore_err,
                            "failed to restore backup after tracking write failure — \
                             user may need to rename .kiro-bak file manually"
                        );
                    }
                    return Err(e);
                }

                // Success — drop the backup file. Best-effort; an orphan
                // .kiro-bak left here is a curiosity, not a correctness issue.
                if let Some(ref bak) = backup
                    && let Err(e) = fs::remove_file(bak)
                    && e.kind() != std::io::ErrorKind::NotFound
                {
                    warn!(
                        path = %bak.display(),
                        error = %e,
                        "failed to remove install backup after success"
                    );
                }

                debug!(name = %agent_name, force = input.mode.is_force(), "native agent installed");

                Ok(InstalledNativeAgentOutcome {
                    name: agent_name,
                    json_path: json_target,
                    kind: if forced_overwrite {
                        InstallOutcomeKind::ForceOverwrote
                    } else {
                        InstallOutcomeKind::Installed
                    },
                    source_hash: input.source_hash.to_string(),
                    installed_hash,
                })
            });

        result.map_err(|e| match e {
            crate::error::Error::Agent(agent_err) => agent_err,
            other => AgentError::InstallFailed {
                path: json_target_for_err,
                source: Box::new(other),
            },
        })
    }

    /// Stage a native agent's `raw_bytes` into a fresh staging directory
    /// using the final filename `<name>.json` so hashing the staged copy
    /// produces the same value as hashing after promotion. Computes
    /// `installed_hash` against staging BEFORE any destructive op on
    /// `agents_root` — a hash failure leaves `agents_root` untouched.
    ///
    /// Returns `(staging_dir, json_rel, installed_hash)` on success.
    fn stage_native_agent_file(
        &self,
        name: &str,
        raw_bytes: &[u8],
    ) -> crate::error::Result<(tempfile::TempDir, PathBuf, String)> {
        let staging = tempfile::Builder::new()
            .prefix(&format!("_installing-agent-{name}-"))
            .tempdir_in(self.kiro_dir())?;
        let json_rel = PathBuf::from(format!("{name}.json"));
        let staging_json = staging.path().join(&json_rel);

        fs::write(&staging_json, raw_bytes)?;

        let installed_hash =
            match crate::hash::hash_artifact(staging.path(), std::slice::from_ref(&json_rel)) {
                Ok(h) => h,
                Err(e) => {
                    warn!(
                        name,
                        error = %e,
                        "installed_hash computation failed on staging; removing staging dir"
                    );
                    return Err(e.into());
                }
            };

        Ok((staging, json_rel, installed_hash))
    }

    /// Move a staged native agent JSON into its final destination, backing
    /// the existing file up to a `.kiro-bak` sibling when `forced_overwrite`
    /// is set. Returns the backup path if one was made, so the caller can
    /// restore on tracking failure or drop the backup on success.
    ///
    /// Pre-conditions: caller has already done the collision check; under
    /// `forced_overwrite == false` the destination is guaranteed to not
    /// exist (no tracking entry, no orphan on disk). Caller's
    /// `tempfile::TempDir` drops at scope exit, cleaning up the (now
    /// empty) staging directory.
    fn promote_native_agent(
        &self,
        staging: &Path,
        json_rel: &Path,
        json_target: &Path,
        forced_overwrite: bool,
    ) -> crate::error::Result<Option<PathBuf>> {
        let staging_json = staging.join(json_rel);

        fs::create_dir_all(self.agents_dir())?;

        // Backup phase — only when overwriting an existing file.
        let backup_target = Self::companion_backup_path(json_target);
        let backup = if forced_overwrite && json_target.exists() {
            fs::rename(json_target, &backup_target)?;
            Some(backup_target.clone())
        } else {
            None
        };

        // Promote phase.
        if let Err(e) = fs::rename(&staging_json, json_target) {
            // Restore backup if we made one.
            if backup.is_some()
                && let Err(restore_err) = fs::rename(&backup_target, json_target)
            {
                warn!(
                    backup = %backup_target.display(),
                    target = %json_target.display(),
                    error = %restore_err,
                    "failed to restore backup after rename failure"
                );
            }
            return Err(e.into());
        }
        Ok(backup)
    }

    /// Install a plugin's native companion file bundle as one atomic unit.
    ///
    /// The bundle's files are validated against tracking BEFORE any writes:
    /// a same-plugin idempotent reinstall is a verified no-op; an
    /// idempotent-mismatch under [`InstallMode::New`] returns
    /// [`AgentError::ContentChangedRequiresForce`]; a cross-plugin path
    /// conflict returns [`AgentError::PathOwnedByOtherPlugin`]; a file on
    /// disk with no tracking entry returns
    /// [`AgentError::OrphanFileAtDestination`]. All three are upgraded to
    /// proceed-with-`forced_overwrite` under [`InstallMode::Force`].
    ///
    /// Each file is staged at its rel layout under a per-plugin staging
    /// dir, hashed there before any destructive op, then promoted with
    /// per-file backups. On any later failure (rename, tracking write)
    /// the backups are restored — the bundle is either fully installed
    /// or fully rolled back.
    ///
    /// Diff-and-removes orphans from a prior install of *this* plugin
    /// when the file set shrinks (e.g. a companion `prompts/old.md`
    /// removed from the source manifest).
    ///
    /// In force mode, cross-plugin transfers update the previous owner's
    /// tracking entry to drop the transferred paths; if that empties the
    /// owner's `files`, the entry is removed entirely.
    ///
    /// Empty `rel_paths` returns an idempotent no-op outcome with no
    /// tracking write — the bundle has nothing to install.
    ///
    /// [`InstallMode::New`]: crate::service::InstallMode::New
    /// [`InstallMode::Force`]: crate::service::InstallMode::Force
    ///
    /// # Errors
    ///
    /// See the collision matrix above for the user-facing variants;
    /// [`AgentError::InstallFailed`] wraps any underlying I/O / hash /
    /// tracking failure.
    pub fn install_native_companions(
        &self,
        input: &NativeCompanionsInput<'_>,
    ) -> Result<InstalledNativeCompanionsOutcome, AgentError> {
        let agents_dir = self.agents_dir();

        if input.rel_paths.is_empty() {
            return Ok(InstalledNativeCompanionsOutcome {
                plugin: input.plugin.to_string(),
                files: Vec::new(),
                kind: InstallOutcomeKind::Idempotent,
                source_hash: input.source_hash.to_string(),
                installed_hash: input.source_hash.to_string(),
            });
        }

        let plugin_for_err = input.plugin.to_string();
        let result: crate::error::Result<InstalledNativeCompanionsOutcome> =
            crate::file_lock::with_file_lock(&self.agent_tracking_path(), || {
                self.install_native_companions_locked(input, &agents_dir)
            });

        result.map_err(|e| match e {
            crate::error::Error::Agent(agent_err) => agent_err,
            other => AgentError::InstallFailed {
                path: agents_dir.join(format!("_companions-{plugin_for_err}")),
                source: Box::new(other),
            },
        })
    }

    /// Inside-the-lock body of [`Self::install_native_companions`].
    /// Extracted so the outer function stays under the line cap; the
    /// closure-with-lock dance and the error-projection live there.
    fn install_native_companions_locked(
        &self,
        input: &NativeCompanionsInput<'_>,
        agents_dir: &Path,
    ) -> crate::error::Result<InstalledNativeCompanionsOutcome> {
        let mut installed = self.load_installed_agents()?;

        let forced_overwrite =
            match Self::classify_companion_collision(&installed, input, agents_dir)? {
                CollisionDecision::Idempotent(outcome) => return Ok(*outcome),
                CollisionDecision::Proceed { forced_overwrite } => forced_overwrite,
            };

        // Compute source_scan_root BEFORE staging/promotion. The
        // validation has no dependency on staging (only on
        // input.scan_root + input.plugin_dir), and a defensive failure
        // here AFTER promote would leak placed files + backups since
        // bare ? skips rollback. Atomicity contract: every failure
        // mode after promote_native_companions must call
        // rollback_companion_promotion; computing this upfront removes
        // one failure source from the post-promote span.
        let source_scan_root =
            crate::validation::RelativePath::from_path_under(input.scan_root, input.plugin_dir)
                .map_err(|e| AgentError::InstallFailed {
                    path: input.scan_root.to_path_buf(),
                    source: Box::new(crate::error::Error::Io(std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        format!(
                            "native companion scan_root `{}` not under plugin_dir `{}`: {e}",
                            input.scan_root.display(),
                            input.plugin_dir.display(),
                        ),
                    ))),
                })?;

        let (staging, installed_hash) =
            self.stage_native_companion_files(input.plugin, input.scan_root, input.rel_paths)?;

        let CompanionPromotion { placed, backups } = Self::promote_native_companions(
            staging.path(),
            input.rel_paths,
            agents_dir,
            forced_overwrite,
        )?;
        // staging is a TempDir; drops at scope exit and cleans up the
        // now-empty (or partially-promoted-out-of) staging directory.

        if forced_overwrite
            && let Err(e) = Self::strip_transferred_paths_from_other_plugins(
                &mut installed,
                input.plugin,
                input.rel_paths,
                agents_dir,
            )
        {
            warn!(
                plugin = %input.plugin,
                error = %e,
                "cross-plugin transfer hash recompute failed; restoring backups"
            );
            Self::rollback_companion_promotion(&placed, &backups);
            return Err(e);
        }

        // Capture the prior file set BEFORE replacing the tracking entry
        // so we can remove diffed-out files post-tracking-write (atomicity
        // fix per code-reviewer #1 / silent-failure-hunter #2). Removing
        // them here would leave the user with deleted files AND phantom
        // tracking on a write failure.
        let diffed_prior_files =
            Self::diff_prior_companion_files(&installed, input.plugin, input.rel_paths);

        installed.native_companions.insert(
            // HashMap key remains `String` (out of scope per Phase 1.5
            // design — only the meta-value's `plugin: PluginName` field
            // gets the newtype).
            input.plugin.as_str().to_owned(),
            InstalledNativeCompanionsMeta {
                marketplace: input.marketplace.clone(),
                plugin: input.plugin.clone(),
                version: input.version.map(String::from),
                installed_at: chrono::Utc::now(),
                files: input.rel_paths.to_vec(),
                source_hash: input.source_hash.to_string(),
                installed_hash: installed_hash.clone(),
                source_scan_root,
            },
        );

        if let Err(e) = self.write_agent_tracking(&installed) {
            warn!(
                plugin = %input.plugin,
                error = %e,
                "companion tracking update failed; rolling back files"
            );
            Self::rollback_companion_promotion(&placed, &backups);
            return Err(e);
        }

        // Tracking succeeded — NOW remove any prior-install files the
        // shrunk file set doesn't claim. Best-effort: a failure here
        // leaves slightly more files on disk than tracking claims, which
        // is strictly better than removing them before the tracking
        // write and losing them if the write fails.
        Self::remove_companion_files_best_effort(&diffed_prior_files, agents_dir, input.plugin);

        // Success — drop the backup files. Best-effort.
        for (_orig, backup) in &backups {
            if let Err(e) = fs::remove_file(backup)
                && e.kind() != std::io::ErrorKind::NotFound
            {
                warn!(
                    path = %backup.display(),
                    error = %e,
                    "failed to remove companion backup after success"
                );
            }
        }

        debug!(
            plugin = %input.plugin,
            files = placed.len(),
            force = input.mode.is_force(),
            "native companions installed"
        );

        Ok(InstalledNativeCompanionsOutcome {
            plugin: input.plugin.to_string(),
            files: placed,
            kind: if forced_overwrite {
                InstallOutcomeKind::ForceOverwrote
            } else {
                InstallOutcomeKind::Installed
            },
            source_hash: input.source_hash.to_string(),
            installed_hash,
        })
    }

    /// Decide whether the companion install proceeds, idempotently no-ops,
    /// or rejects. Exhaustive over the same-plugin / cross-plugin / orphan
    /// states.
    fn classify_companion_collision(
        installed: &InstalledAgents,
        input: &NativeCompanionsInput<'_>,
        agents_dir: &Path,
    ) -> crate::error::Result<CollisionDecision<InstalledNativeCompanionsOutcome>> {
        let mut forced_overwrite = false;

        // Same-plugin check first — idempotent or content-changed.
        // The HashMap key is still `String` (out of scope), so look up
        // by string view; equality below uses the same shape.
        if let Some(existing) = installed.native_companions.get(input.plugin.as_str()) {
            if existing.source_hash == input.source_hash {
                return Ok(CollisionDecision::Idempotent(Box::new(
                    InstalledNativeCompanionsOutcome {
                        plugin: input.plugin.to_string(),
                        files: existing.files.iter().map(|p| agents_dir.join(p)).collect(),
                        kind: InstallOutcomeKind::Idempotent,
                        source_hash: input.source_hash.to_string(),
                        installed_hash: existing.installed_hash.clone(),
                    },
                )));
            }
            if !input.mode.is_force() {
                return Err(AgentError::ContentChangedRequiresForce {
                    name: format!("{}/companions", input.plugin),
                }
                .into());
            }
            forced_overwrite = true;
        }

        // Cross-plugin path conflict + orphan-on-disk checks.
        for rel in input.rel_paths {
            for (other_plugin, other_meta) in &installed.native_companions {
                if other_plugin.as_str() == input.plugin.as_str() {
                    continue;
                }
                if other_meta.files.contains(rel) {
                    if !input.mode.is_force() {
                        return Err(AgentError::PathOwnedByOtherPlugin {
                            path: agents_dir.join(rel),
                            owner: other_plugin.clone(),
                        }
                        .into());
                    }
                    forced_overwrite = true;
                }
            }
            // Orphan check: file exists on disk but no plugin owns it.
            let dest = agents_dir.join(rel);
            if dest.exists() {
                let owned_by_any = installed
                    .native_companions
                    .values()
                    .any(|m| m.files.contains(rel));
                if !owned_by_any {
                    if !input.mode.is_force() {
                        return Err(AgentError::OrphanFileAtDestination { path: dest }.into());
                    }
                    forced_overwrite = true;
                }
            }
        }

        Ok(CollisionDecision::Proceed { forced_overwrite })
    }

    /// Stage every companion file at its relative layout under a fresh
    /// per-plugin staging dir, then compute `installed_hash` against the
    /// staged copies BEFORE any destructive op on `agents_root`. A hash
    /// failure leaves `agents_root` untouched.
    ///
    /// Returns `(staging_dir, installed_hash)` on success.
    fn stage_native_companion_files(
        &self,
        plugin: &PluginName,
        scan_root: &Path,
        rel_paths: &[PathBuf],
    ) -> crate::error::Result<(tempfile::TempDir, String)> {
        let staging = tempfile::Builder::new()
            .prefix(&format!("_installing-companions-{}-", plugin.as_str()))
            .tempdir_in(self.kiro_dir())?;

        for rel in rel_paths {
            let src = scan_root.join(rel);
            // Refuse hardlinked sources before fs::copy. Same threat
            // model as stage_steering_file; see
            // `NativeParseFailure::HardlinkRefused` for the canonical
            // statement.
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                // Attach src to the stat error so a permission-denied or
                // racy-delete here surfaces with the failed path, not as
                // a path-less `Error::Io`.
                let md = fs::symlink_metadata(&src).map_err(|e| AgentError::InstallFailed {
                    path: src.clone(),
                    source: Box::new(crate::error::Error::Io(e)),
                })?;
                if md.is_file() && md.nlink() > 1 {
                    return Err(AgentError::SourceHardlinked {
                        path: src.clone(),
                        nlink: md.nlink(),
                    }
                    .into());
                }
            }
            let dest = staging.path().join(rel);
            if let Some(parent) = dest.parent() {
                fs::create_dir_all(parent).map_err(|e| AgentError::InstallFailed {
                    path: parent.to_path_buf(),
                    source: Box::new(crate::error::Error::Io(e)),
                })?;
            }
            fs::copy(&src, &dest).map_err(|e| AgentError::InstallFailed {
                path: src.clone(),
                source: Box::new(crate::error::Error::Io(e)),
            })?;
        }

        let installed_hash = match crate::hash::hash_artifact(staging.path(), rel_paths) {
            Ok(h) => h,
            Err(e) => {
                warn!(
                    plugin = plugin.as_str(),
                    error = %e,
                    "installed_hash computation failed on staging; removing staging dir"
                );
                return Err(e.into());
            }
        };

        Ok((staging, installed_hash))
    }

    /// Move every staged companion file into its destination under
    /// `agents_root`, backing each existing file up to a `.kiro-bak`
    /// sibling when `forced_overwrite` is set. Returns the
    /// [`CompanionPromotion`] (placed paths plus original→backup pairs)
    /// so the caller can roll back on later failure.
    ///
    /// `backups` is `Vec<(original_path, backup_path)>` — restoring is
    /// `fs::rename(backup, original)`.
    fn promote_native_companions(
        staging: &Path,
        rel_paths: &[PathBuf],
        agents_dir: &Path,
        forced_overwrite: bool,
    ) -> crate::error::Result<CompanionPromotion> {
        let mut placed: Vec<PathBuf> = Vec::with_capacity(rel_paths.len());
        let mut backups: Vec<(PathBuf, PathBuf)> = Vec::new();

        for rel in rel_paths {
            let src = staging.join(rel);
            let dest = agents_dir.join(rel);
            if let Some(parent) = dest.parent()
                && let Err(e) = fs::create_dir_all(parent)
            {
                Self::rollback_companion_promotion(&placed, &backups);
                return Err(AgentError::InstallFailed {
                    path: parent.to_path_buf(),
                    source: Box::new(crate::error::Error::Io(e)),
                }
                .into());
            }
            // Backup the existing destination if we'll overwrite it.
            if forced_overwrite && dest.exists() {
                let backup = Self::companion_backup_path(&dest);
                if let Err(e) = fs::rename(&dest, &backup) {
                    Self::rollback_companion_promotion(&placed, &backups);
                    return Err(AgentError::InstallFailed {
                        path: dest.clone(),
                        source: Box::new(crate::error::Error::Io(e)),
                    }
                    .into());
                }
                backups.push((dest.clone(), backup));
            }
            if let Err(e) = fs::rename(&src, &dest) {
                Self::rollback_companion_promotion(&placed, &backups);
                return Err(AgentError::InstallFailed {
                    path: dest.clone(),
                    source: Box::new(crate::error::Error::Io(e)),
                }
                .into());
            }
            placed.push(dest);
        }

        Ok(CompanionPromotion { placed, backups })
    }

    /// Compute the `.kiro-bak` sibling path for a companion file.
    /// Appends `.kiro-bak` to the full path (preserving any existing
    /// extension) so a `foo.md` companion becomes `foo.md.kiro-bak`
    /// and the original extension survives in the backup name —
    /// useful for recovery if the user spots leftover backups on disk.
    fn companion_backup_path(dest: &Path) -> PathBuf {
        let mut bak = dest.as_os_str().to_owned();
        bak.push(".kiro-bak");
        PathBuf::from(bak)
    }

    /// Rollback helper: remove every newly-placed file and restore each
    /// backup to its original path. Best-effort — failures are logged but
    /// don't abort the rollback.
    fn rollback_companion_promotion(placed: &[PathBuf], backups: &[(PathBuf, PathBuf)]) {
        for p in placed {
            if let Err(e) = fs::remove_file(p)
                && e.kind() != std::io::ErrorKind::NotFound
            {
                warn!(
                    path = %p.display(),
                    error = %e,
                    "failed to remove placed companion file during rollback"
                );
            }
        }
        for (orig, backup) in backups {
            if let Err(e) = fs::rename(backup, orig) {
                warn!(
                    backup = %backup.display(),
                    target = %orig.display(),
                    error = %e,
                    "failed to restore companion backup during rollback — \
                     user may need to rename .kiro-bak file manually"
                );
            }
        }
    }

    /// In force mode: drop transferred `rel_paths` from any other plugin's
    /// tracking entry, recompute that plugin's `source_hash` /
    /// `installed_hash` over the surviving file set, and remove emptied
    /// entries entirely. Caller has just promoted the files, so the
    /// previous owner has lost ownership.
    ///
    /// # Why recompute hashes
    ///
    /// Closes silent-failure-hunter #1: dropping files from
    /// `meta.files` without recomputing leaves the prior plugin's hash
    /// claiming the OLD file set. A future drift-check command would
    /// then report a phantom mismatch on every cross-plugin force
    /// transfer. Both `source_hash` and `installed_hash` are set to
    /// the hash of the surviving files at `agents_dir` — post-transfer
    /// the destination IS the canonical truth for what this plugin
    /// owns, since the prior plugin's original source bundle is no
    /// longer accessible.
    ///
    /// # Errors
    ///
    /// Returns the hash error if recomputing any modified plugin's
    /// hash fails. Caller is responsible for rolling back the file
    /// promotion since this happens AFTER promote.
    fn strip_transferred_paths_from_other_plugins(
        installed: &mut InstalledAgents,
        plugin: &PluginName,
        rel_paths: &[PathBuf],
        agents_dir: &Path,
    ) -> crate::error::Result<()> {
        let new_set: std::collections::HashSet<&Path> =
            rel_paths.iter().map(PathBuf::as_path).collect();
        // HashMap key on `installed.native_companions` is still `String`,
        // so the filter compares plugin-name string-views.
        let other_plugins: Vec<String> = installed
            .native_companions
            .keys()
            .filter(|p| p.as_str() != plugin.as_str())
            .cloned()
            .collect();
        let mut modified: Vec<String> = Vec::new();
        for p in other_plugins {
            if let Some(meta) = installed.native_companions.get_mut(&p) {
                let len_before = meta.files.len();
                meta.files.retain(|f| !new_set.contains(f.as_path()));
                if meta.files.len() != len_before {
                    modified.push(p);
                }
            }
        }
        // Recompute hashes BEFORE pruning empties — pruning consumes
        // the entry, and we'd need to special-case "empty entries
        // don't need a hash recompute". Cleaner to recompute first,
        // then prune.
        for p in &modified {
            if let Some(meta) = installed.native_companions.get_mut(p)
                && !meta.files.is_empty()
            {
                let new_hash = crate::hash::hash_artifact(agents_dir, &meta.files)?;
                new_hash.clone_into(&mut meta.source_hash);
                meta.installed_hash = new_hash;
            }
        }
        installed
            .native_companions
            .retain(|_, meta| !meta.files.is_empty());
        Ok(())
    }

    /// Compute the prior tracked companion files for `plugin` that are
    /// NOT present in the new `rel_paths` set. Pure: doesn't touch disk
    /// or mutate `installed`. Caller should compute this BEFORE
    /// replacing the plugin's tracking entry, then remove the files
    /// AFTER `write_agent_tracking` succeeds — see
    /// [`Self::remove_companion_files_best_effort`] for the removal
    /// half.
    fn diff_prior_companion_files(
        installed: &InstalledAgents,
        plugin: &PluginName,
        rel_paths: &[PathBuf],
    ) -> Vec<PathBuf> {
        let Some(prior) = installed.native_companions.get(plugin.as_str()) else {
            return Vec::new();
        };
        let new_set: std::collections::HashSet<&Path> =
            rel_paths.iter().map(PathBuf::as_path).collect();
        prior
            .files
            .iter()
            .filter(|f| !new_set.contains(f.as_path()))
            .cloned()
            .collect()
    }

    /// Best-effort removal of prior-install companion files that have
    /// dropped out of the new tracking entry. Failures are logged but
    /// don't propagate — the file set is already canonical in
    /// tracking, and a stray on-disk file is strictly less harmful
    /// than rolling back a successful install.
    ///
    /// Pair with [`Self::diff_prior_companion_files`] computed BEFORE
    /// the tracking write — call this AFTER `write_agent_tracking`
    /// succeeds so a tracking-write failure can't leave files removed
    /// with phantom tracking still claiming them.
    ///
    /// Re-stats each path with `symlink_metadata` before removal and
    /// skips reparse points / symlinks. Defense in depth: if some
    /// out-of-band actor (or a stale tracking entry from a prior
    /// install gone wrong) replaced a tracked path with a symlink,
    /// `fs::remove_file` would remove the symlink itself, not the
    /// target — operationally fine but the audit trail is murkier.
    /// Skipping with a warn! makes the unusual state visible.
    fn remove_companion_files_best_effort(
        rel_paths: &[PathBuf],
        agents_dir: &Path,
        plugin: &PluginName,
    ) {
        for rel in rel_paths {
            let abs = agents_dir.join(rel);
            match fs::symlink_metadata(&abs) {
                Ok(md) if crate::platform::is_reparse_or_symlink(&md) => {
                    warn!(
                        plugin = plugin.as_str(),
                        path = %abs.display(),
                        "tracked companion is a symlink/reparse point; skipping orphan-removal"
                    );
                    continue;
                }
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    // Already gone — nothing to do.
                    continue;
                }
                Err(e) => {
                    warn!(
                        plugin = plugin.as_str(),
                        path = %abs.display(),
                        error = %e,
                        "failed to stat orphaned prior companion file; skipping"
                    );
                    continue;
                }
            }
            if let Err(e) = fs::remove_file(&abs)
                && e.kind() != std::io::ErrorKind::NotFound
            {
                warn!(
                    plugin = plugin.as_str(),
                    path = %abs.display(),
                    error = %e,
                    "failed to remove orphaned prior companion file post-success"
                );
            }
        }
    }

    // -- internal helpers --------------------------------------------------

    /// Copy a source skill directory and update tracking.
    ///
    /// The entire flow — existence check, staging copy, rename, and tracking
    /// update — runs under a single advisory lock on the tracking file so
    /// two concurrent installs of the same skill name cannot both pass the
    /// existence check and clobber each other's staging directory.
    ///
    /// Per-attempt staging is a `tempfile::TempDir` rooted under
    /// `self.skills_dir()` with prefix `_installing-skill-<name>-`;
    /// `tempfile::Builder` appends a random suffix so two threads in
    /// the same process always have distinct staging paths, and the
    /// `TempDir` RAII Drop sweeps the directory on `?`-propagation,
    /// panic-unwind, or scope exit.
    fn write_skill_dir(
        &self,
        name: &str,
        source_dir: &Path,
        mut meta: InstalledSkillMeta,
        force: bool,
        source_hash: String,
    ) -> crate::error::Result<()> {
        crate::file_lock::with_file_lock(&self.tracking_path(), || -> crate::error::Result<()> {
            let dir = self.skill_dir(name);

            if !force && dir.exists() {
                return Err(SkillError::AlreadyInstalled {
                    name: name.to_owned(),
                }
                .into());
            }

            // Ensure the skills parent directory exists.
            fs::create_dir_all(self.skills_dir())?;

            // Stage the copy into a fresh temp dir. TempDir RAII cleans
            // up on Drop, so any `?`-propagation below (or panic) leaves
            // no orphan staging dir behind.
            let staging = tempfile::Builder::new()
                .prefix(&format!("_installing-skill-{name}-"))
                .tempdir_in(self.skills_dir())?;
            copy_dir_recursive(source_dir, staging.path())?;

            // Compute installed_hash on the staged copy BEFORE the destructive
            // rename. Any hash failure here leaves the previous install (if
            // force mode) intact on disk — the rename hasn't happened yet.
            // Staging contains the same bytes that will land, so the hash value
            // is identical to what we'd compute post-rename. This is the
            // correct TOCTOU stance: `installed_hash` is the source of truth
            // for what the user has, computed over the bytes we're about to
            // commit to disk.
            let installed_hash = match crate::hash::hash_dir_tree(staging.path()) {
                Ok(h) => h,
                Err(e) => {
                    warn!(
                        name,
                        error = %e,
                        "installed_hash computation failed on staging; removing staging dir"
                    );
                    return Err(e.into());
                }
            };

            // Only now do the destructive swap — hash is already in hand so
            // any failure from here is unrelated to the hash computation.
            if dir.exists() {
                debug!(name, "removing existing skill directory for force install");
                fs::remove_dir_all(&dir)?;
            }

            // Rename staging to final location. After this, the directory
            // entry that staging.path() pointed at is gone; TempDir's Drop
            // will see NotFound and silently skip cleanup.
            fs::rename(staging.path(), &dir)?;
            meta.source_hash = source_hash;
            meta.installed_hash = installed_hash;

            // Update tracking. If this fails, roll back the rename so the
            // filesystem and tracking file stay consistent.
            let tracking_result = self.load_installed().and_then(|mut installed| {
                installed.skills.insert(name.to_owned(), meta);
                self.write_tracking(&installed)
            });

            if let Err(e) = tracking_result {
                warn!(
                    name,
                    error = %e,
                    "tracking update failed after rename, rolling back"
                );
                if let Err(rollback_err) = fs::remove_dir_all(&dir) {
                    warn!(
                        path = %dir.display(),
                        error = %rollback_err,
                        "failed to roll back skill directory after tracking failure — \
                         skill is installed on disk but not tracked"
                    );
                }
                return Err(e);
            }

            debug!(name, "skill installed from directory");
            Ok(())
        })
    }

    /// Persist the tracking file to disk atomically.
    ///
    /// Uses [`crate::cache::atomic_write`] so that a crash mid-write cannot
    /// leave truncated JSON.
    fn write_tracking(&self, installed: &InstalledSkills) -> crate::error::Result<()> {
        let path = self.tracking_path();

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let json = serde_json::to_string_pretty(installed)?;
        crate::cache::atomic_write(&path, json.as_bytes())?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::test_support::{mp, pn};

    fn temp_project() -> (tempfile::TempDir, KiroProject) {
        let dir = tempfile::tempdir().expect("tempdir");
        let project = KiroProject::new(dir.path().to_path_buf());
        (dir, project)
    }

    fn sample_meta() -> InstalledSkillMeta {
        InstalledSkillMeta {
            marketplace: mp("test-market"),
            plugin: pn("test-plugin"),
            version: Some("1.0.0".into()),
            installed_at: Utc::now(),
            // Empty placeholders — `install_skill_from_dir` overwrites
            // both with real hashes during the install path. Tests that
            // pre-stamp tracking entries (no install call) and assert
            // on the field carry their own real hash.
            source_hash: String::new(),
            installed_hash: String::new(),
            source_scan_root: RelativePath::new("skills").expect("valid"),
        }
    }

    #[test]
    fn installed_agent_meta_roundtrips_json() {
        let meta = InstalledAgentMeta {
            marketplace: mp("mp"),
            plugin: pn("pr-review-toolkit"),
            version: Some("1.2.3".into()),
            installed_at: Utc::now(),
            dialect: AgentDialect::Claude,
            source_path: RelativePath::new("agents/reviewer.md").expect("valid"),
            source_hash: String::new(),
            installed_hash: String::new(),
        };
        let json = serde_json::to_string(&meta).unwrap();
        let back: InstalledAgentMeta = serde_json::from_str(&json).unwrap();
        assert_eq!(back.plugin, "pr-review-toolkit");
        assert_eq!(back.dialect, AgentDialect::Claude);
        // Spot-check the wire format: dialect serializes lowercase.
        assert!(
            json.contains("\"dialect\":\"claude\""),
            "unexpected wire format: {json}"
        );
    }

    #[test]
    fn installed_agent_meta_roundtrips_copilot_dialect() {
        let meta = InstalledAgentMeta {
            marketplace: mp("mp"),
            plugin: pn("p"),
            version: None,
            installed_at: Utc::now(),
            dialect: AgentDialect::Copilot,
            source_path: RelativePath::new("agents/reviewer.agent.md").expect("valid"),
            source_hash: String::new(),
            installed_hash: String::new(),
        };
        let json = serde_json::to_string(&meta).unwrap();
        assert!(json.contains("\"dialect\":\"copilot\""));
        let back: InstalledAgentMeta = serde_json::from_str(&json).unwrap();
        assert_eq!(back.dialect, AgentDialect::Copilot);
    }

    #[test]
    fn installed_agents_default_is_empty() {
        let ia = InstalledAgents::default();
        assert!(ia.agents.is_empty());
    }

    #[test]
    fn installed_steering_loads_legacy_empty_object() {
        // Old projects without any steering install: file may not exist,
        // or may be `{}`. Both must deserialize to an empty wrapper.
        let from_empty: InstalledSteering = serde_json::from_slice(b"{}").unwrap();
        assert!(from_empty.files.is_empty());
    }

    #[test]
    fn installed_steering_round_trips_through_serde() {
        let mut steering = InstalledSteering::default();
        steering.files.insert(
            std::path::PathBuf::from("review-process.md"),
            InstalledSteeringMeta {
                marketplace: mp("m"),
                plugin: pn("p"),
                version: Some("0.1.0".into()),
                installed_at: chrono::Utc::now(),
                source_hash: "blake3:abc".into(),
                installed_hash: "blake3:abc".into(),
                source_scan_root: RelativePath::new("steering").expect("valid"),
            },
        );
        let bytes = serde_json::to_vec(&steering).unwrap();
        let back: InstalledSteering = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(back.files.len(), 1);
        assert!(
            back.files
                .contains_key(std::path::Path::new("review-process.md"))
        );
    }

    #[test]
    fn installed_steering_skips_serializing_empty_files_map() {
        // P-4: empty `files` must not appear in the wire format. Pre-steering
        // tracking files round-trip byte-identical through this type.
        let empty = InstalledSteering::default();
        let bytes = serde_json::to_vec(&empty).unwrap();
        let s = std::str::from_utf8(&bytes).unwrap();
        assert!(
            !s.contains("files"),
            "empty InstalledSteering must omit the `files` key, got: {s}"
        );
    }

    #[test]
    fn load_installed_steering_returns_default_when_file_missing() {
        let (_dir, project) = temp_project();
        let installed = project.load_installed_steering().unwrap();
        assert!(installed.files.is_empty());
    }

    #[test]
    fn load_installed_steering_rejects_path_traversal_in_files_key() {
        // Closes marketplace-security-reviewer Important finding: a
        // tampered `installed-steering.json` containing a path-traversal
        // key like `../../etc/passwd` would, without validation, flow
        // into `steering_dir.join(rel)` at install / removal time and
        // escape the install boundary.
        let (_dir, project) = temp_project();
        let tracking_path = project.root.join(".kiro/installed-steering.json");
        fs::create_dir_all(tracking_path.parent().unwrap()).unwrap();
        // Hand-craft a tampered tracking file. The invalid key bypasses
        // the install path's validation entirely (it's never installed
        // — the user fabricated it).
        let tampered = serde_json::json!({
            "files": {
                "../../etc/passwd": {
                    "marketplace": "m",
                    "plugin": "p",
                    "version": null,
                    "installed_at": chrono::Utc::now(),
                    "source_hash": "blake3:abc",
                    "installed_hash": "blake3:abc",
                    "source_scan_root": "steering",
                }
            }
        });
        fs::write(&tracking_path, tampered.to_string()).unwrap();

        let err = project
            .load_installed_steering()
            .expect_err("traversal must be refused at load time");
        match err {
            crate::error::Error::Io(io_err) => {
                assert_eq!(io_err.kind(), std::io::ErrorKind::InvalidData);
                let msg = io_err.to_string();
                assert!(
                    msg.contains("../../etc/passwd"),
                    "error must name the offending path, got: {msg}"
                );
            }
            other => panic!("expected Error::Io(InvalidData), got {other:?}"),
        }
    }

    #[test]
    fn load_installed_rejects_path_traversal_in_skills_key() {
        // I9 regression: a tampered `installed-skills.json` containing
        // a path-traversal key like `../../etc` as a skill name would,
        // without validation, flow into `skills_dir.join(name)` at
        // removal time (`fs::remove_dir_all`) and escape the install
        // boundary. Mirrors
        // [`load_installed_steering_rejects_path_traversal_in_files_key`].
        let (_dir, project) = temp_project();
        let tracking_path = project.root.join(".kiro/installed-skills.json");
        fs::create_dir_all(tracking_path.parent().unwrap()).unwrap();
        let tampered = serde_json::json!({
            "skills": {
                "../../etc": {
                    "marketplace": "m",
                    "plugin": "p",
                    "version": null,
                    "installed_at": chrono::Utc::now(),
                    "source_hash": "blake3:abc",
                    "installed_hash": "blake3:abc",
                    "source_scan_root": "skills",
                }
            }
        });
        fs::write(&tracking_path, tampered.to_string()).unwrap();

        let err = project
            .load_installed()
            .expect_err("traversal must be refused at load time");
        match err {
            crate::error::Error::Io(io_err) => {
                assert_eq!(io_err.kind(), std::io::ErrorKind::InvalidData);
                let msg = io_err.to_string();
                assert!(
                    msg.contains("../../etc"),
                    "error must name the offending key, got: {msg}"
                );
            }
            other => panic!("expected Error::Io(InvalidData), got {other:?}"),
        }
    }

    #[test]
    fn load_installed_agents_rejects_path_traversal_in_agents_key() {
        // I9 regression: a tampered `installed-agents.json` containing
        // a path-traversal key like `../../etc` as an agent name
        // would, without validation, flow into both
        // `agents_dir.join(format!("{name}.json"))` and
        // `agent_prompts_dir.join(format!("{name}.md"))` at removal
        // time, escaping the install boundary.
        let (_dir, project) = temp_project();
        let tracking_path = project.root.join(".kiro/installed-agents.json");
        fs::create_dir_all(tracking_path.parent().unwrap()).unwrap();
        let tampered = serde_json::json!({
            "agents": {
                "../../etc": {
                    "marketplace": "m",
                    "plugin": "p",
                    "version": null,
                    "installed_at": chrono::Utc::now(),
                    "dialect": "claude",
                    "source_path": "agents/etc.md",
                    "source_hash": "x",
                    "installed_hash": "x",
                }
            }
        });
        fs::write(&tracking_path, tampered.to_string()).unwrap();

        let err = project
            .load_installed_agents()
            .expect_err("traversal must be refused at load time");
        match err {
            crate::error::Error::Io(io_err) => {
                assert_eq!(io_err.kind(), std::io::ErrorKind::InvalidData);
                let msg = io_err.to_string();
                assert!(
                    msg.contains("../../etc"),
                    "error must name the offending agent name, got: {msg}"
                );
            }
            other => panic!("expected Error::Io(InvalidData), got {other:?}"),
        }
    }

    #[test]
    fn load_installed_agents_rejects_path_traversal_in_native_companions_key() {
        // I9 regression: a tampered `installed-agents.json` whose
        // `native_companions` map keys (plugin names) contain a
        // traversal entry would, without validation, flow into
        // `installed_plugins()` on the wire and corrupt downstream
        // marketplace/plugin string handling.
        let (_dir, project) = temp_project();
        let tracking_path = project.root.join(".kiro/installed-agents.json");
        fs::create_dir_all(tracking_path.parent().unwrap()).unwrap();
        let tampered = serde_json::json!({
            "agents": {},
            "native_companions": {
                "../../evil": {
                    "marketplace": "m",
                    "plugin": "evil",
                    "version": null,
                    "installed_at": chrono::Utc::now(),
                    "files": [],
                    "source_hash": "blake3:abc",
                    "installed_hash": "blake3:abc",
                    "source_scan_root": "agents",
                }
            }
        });
        fs::write(&tracking_path, tampered.to_string()).unwrap();

        let err = project
            .load_installed_agents()
            .expect_err("traversal must be refused at load time");
        match err {
            crate::error::Error::Io(io_err) => {
                assert_eq!(io_err.kind(), std::io::ErrorKind::InvalidData);
                let msg = io_err.to_string();
                assert!(
                    msg.contains("../../evil"),
                    "error must name the offending plugin key, got: {msg}"
                );
            }
            other => panic!("expected Error::Io(InvalidData), got {other:?}"),
        }
    }

    #[test]
    fn load_installed_agents_rejects_path_traversal_in_companion_files() {
        // Closes marketplace-security-reviewer Important finding: a
        // tampered `installed-agents.json` containing a traversal entry
        // in `native_companions[*].files` would otherwise reach
        // `hash_artifact(agents_dir, rel)` at cross-plugin transfer
        // recompute time AND `agents_dir.join(rel)` at orphan-removal
        // time, escaping the install boundary in both directions.
        let (_dir, project) = temp_project();
        let tracking_path = project.root.join(".kiro/installed-agents.json");
        fs::create_dir_all(tracking_path.parent().unwrap()).unwrap();
        let tampered = serde_json::json!({
            "agents": {},
            "native_companions": {
                "evil-plugin": {
                    "marketplace": "m",
                    "plugin": "evil-plugin",
                    "version": null,
                    "installed_at": chrono::Utc::now(),
                    "files": ["../../etc/passwd"],
                    "source_hash": "blake3:abc",
                    "installed_hash": "blake3:abc",
                    "source_scan_root": "agents",
                }
            }
        });
        fs::write(&tracking_path, tampered.to_string()).unwrap();

        let err = project
            .load_installed_agents()
            .expect_err("traversal must be refused at load time");
        match err {
            crate::error::Error::Io(io_err) => {
                assert_eq!(io_err.kind(), std::io::ErrorKind::InvalidData);
                let msg = io_err.to_string();
                assert!(
                    msg.contains("../../etc/passwd"),
                    "error must name the offending path, got: {msg}"
                );
                assert!(
                    msg.contains("evil-plugin"),
                    "error must name the owning plugin, got: {msg}"
                );
            }
            other => panic!("expected Error::Io(InvalidData), got {other:?}"),
        }
    }

    /// NC2 (PR #96 re-review): a tampered `installed-agents.json`
    /// whose per-agent `source_path` contains a traversal entry would,
    /// without the `RelativePath` newtype on `InstalledAgentMeta`,
    /// reach `hash_artifact(agents_dir, &[rel])` at update-detection
    /// time and read arbitrary host files. The newtype's `Deserialize`
    /// impl routes through `RelativePath::new` which rejects `..`,
    /// absolute paths, NUL, and embedded backslashes — failing at
    /// `serde_json::from_slice` time, before any path joins happen.
    #[test]
    fn load_installed_agents_rejects_path_traversal_in_source_path() {
        let (_dir, project) = temp_project();
        let tracking_path = project.root.join(".kiro/installed-agents.json");
        fs::create_dir_all(tracking_path.parent().unwrap()).unwrap();
        let tampered = serde_json::json!({
            "agents": {
                "victim": {
                    "marketplace": "m",
                    "plugin": "p",
                    "version": null,
                    "installed_at": chrono::Utc::now(),
                    "dialect": "claude",
                    "source_path": "../../etc/passwd",
                }
            }
        });
        fs::write(&tracking_path, tampered.to_string()).unwrap();

        let err = project
            .load_installed_agents()
            .expect_err("traversal in source_path must be refused at load time");
        // serde_json deserialize errors land in Error::Json
        // (mapped via #[error(transparent)] on Error::Json).
        match err {
            crate::error::Error::Json(_) => {}
            other => panic!("expected Error::Json (RelativePath rejection), got {other:?}"),
        }
    }

    /// NC2 + parse-don't-validate sanity: a well-formed `source_path`
    /// (forward-slash relative path under `agents/`) must round-trip
    /// through serde.
    #[test]
    fn installed_agent_meta_round_trips_valid_source_path() {
        let meta = InstalledAgentMeta {
            marketplace: mp("m"),
            plugin: pn("p"),
            version: None,
            installed_at: Utc::now(),
            dialect: AgentDialect::Claude,
            source_path: RelativePath::new("subdir/agent.md").expect("valid rel path"),
            source_hash: String::new(),
            installed_hash: String::new(),
        };
        let json = serde_json::to_string(&meta).unwrap();
        assert!(
            json.contains("\"source_path\":\"subdir/agent.md\""),
            "wire format must remain a flat string, got: {json}"
        );
        let back: InstalledAgentMeta = serde_json::from_str(&json).unwrap();
        assert_eq!(back.source_path.as_str(), "subdir/agent.md");
    }

    #[test]
    fn remove_skills_result_json_shape_default_empty() {
        let result = RemoveSkillsResult::default();
        let json = serde_json::to_value(&result).expect("serialize");
        assert_eq!(json["removed"], serde_json::json!([]));
        assert_eq!(json["failures"], serde_json::json!([]));
    }

    #[test]
    fn remove_skills_result_json_shape_with_populated_removed() {
        let result = RemoveSkillsResult {
            removed: vec!["alpha".into(), "beta".into()],
            failures: vec![],
        };
        let json = serde_json::to_value(&result).expect("serialize");
        assert_eq!(json["removed"], serde_json::json!(["alpha", "beta"]));
        assert_eq!(json["failures"], serde_json::json!([]));
    }

    #[test]
    fn remove_skills_result_json_shape_with_populated_failure() {
        let result = RemoveSkillsResult {
            removed: vec![],
            failures: vec![RemoveItemFailure {
                item: "broken".into(),
                error: "io: permission denied".into(),
            }],
        };
        let json = serde_json::to_value(&result).expect("serialize");
        assert_eq!(json["failures"][0]["item"], "broken");
        assert_eq!(json["failures"][0]["error"], "io: permission denied");
    }

    // Symmetric tests for RemoveSteeringResult
    #[test]
    fn remove_steering_result_json_shape_default_empty() {
        let result = RemoveSteeringResult::default();
        let json = serde_json::to_value(&result).expect("serialize");
        assert_eq!(json["removed"], serde_json::json!([]));
        assert_eq!(json["failures"], serde_json::json!([]));
    }

    #[test]
    fn remove_steering_result_json_shape_with_populated_removed() {
        let result = RemoveSteeringResult {
            removed: vec!["guide.md".into()],
            failures: vec![],
        };
        let json = serde_json::to_value(&result).expect("serialize");
        assert_eq!(json["removed"], serde_json::json!(["guide.md"]));
        assert_eq!(json["failures"], serde_json::json!([]));
    }

    #[test]
    fn remove_steering_result_json_shape_with_populated_failure() {
        let result = RemoveSteeringResult {
            removed: vec![],
            failures: vec![RemoveItemFailure {
                item: "broken.md".into(),
                error: "io: permission denied".into(),
            }],
        };
        let json = serde_json::to_value(&result).expect("serialize");
        assert_eq!(json["failures"][0]["item"], "broken.md");
        assert_eq!(json["failures"][0]["error"], "io: permission denied");
    }

    // Symmetric tests for RemoveAgentsResult
    #[test]
    fn remove_agents_result_json_shape_default_empty() {
        let result = RemoveAgentsResult::default();
        let json = serde_json::to_value(&result).expect("serialize");
        assert_eq!(json["removed"], serde_json::json!([]));
        assert_eq!(json["failures"], serde_json::json!([]));
    }

    #[test]
    fn remove_agents_result_json_shape_with_populated_removed() {
        let result = RemoveAgentsResult {
            removed: vec!["reviewer".into(), "companions/prompt.md".into()],
            failures: vec![],
        };
        let json = serde_json::to_value(&result).expect("serialize");
        assert_eq!(
            json["removed"],
            serde_json::json!(["reviewer", "companions/prompt.md"])
        );
        assert_eq!(json["failures"], serde_json::json!([]));
    }

    #[test]
    fn remove_agents_result_json_shape_with_populated_failure() {
        let result = RemoveAgentsResult {
            removed: vec![],
            failures: vec![RemoveItemFailure {
                item: "broken-agent".into(),
                error: "io: permission denied".into(),
            }],
        };
        let json = serde_json::to_value(&result).expect("serialize");
        assert_eq!(json["failures"][0]["item"], "broken-agent");
        assert_eq!(json["failures"][0]["error"], "io: permission denied");
    }

    #[test]
    fn load_installed_steering_round_trips_through_disk() {
        let (_dir, project) = temp_project();

        let mut to_save = InstalledSteering::default();
        to_save.files.insert(
            PathBuf::from("guide.md"),
            InstalledSteeringMeta {
                marketplace: mp("m"),
                plugin: pn("p"),
                version: None,
                installed_at: chrono::Utc::now(),
                source_hash: "blake3:abc".into(),
                installed_hash: "blake3:abc".into(),
                source_scan_root: RelativePath::new("steering").expect("valid"),
            },
        );
        project.write_steering_tracking(&to_save).unwrap();

        let loaded = project.load_installed_steering().unwrap();
        assert_eq!(loaded.files.len(), 1);
        assert!(loaded.files.contains_key(std::path::Path::new("guide.md")));
    }

    #[test]
    fn installed_plugins_groups_skills_steering_agents_by_marketplace_plugin_pair() {
        use chrono::Utc;
        let dir = tempfile::tempdir().expect("tempdir");
        let project = KiroProject::new(dir.path().to_path_buf());
        std::fs::create_dir_all(project.kiro_dir()).expect("kiro dir");

        let now = Utc::now();
        let skills_json = serde_json::json!({
            "skills": {
                "alpha": {
                    "marketplace": "mp",
                    "plugin": "plug-a",
                    "version": "1.0.0",
                    "installed_at": now,
                    "source_hash": "deadbeef",
                    "installed_hash": "deadbeef",
                    "source_scan_root": "skills"
                }
            }
        });
        std::fs::write(
            project.kiro_dir().join("installed-skills.json"),
            serde_json::to_vec_pretty(&skills_json).expect("ser skills"),
        )
        .expect("skills tracking");

        let steering_json = serde_json::json!({
            "files": {
                "guide.md": {
                    "marketplace": "mp",
                    "plugin": "plug-a",
                    "version": "1.0.0",
                    "installed_at": now,
                    "source_hash": "cafebabe",
                    "installed_hash": "cafebabe",
                    "source_scan_root": "steering"
                },
                "review.md": {
                    "marketplace": "mp",
                    "plugin": "plug-b",
                    "version": "0.5.0",
                    "installed_at": now,
                    "source_hash": "feedface",
                    "installed_hash": "feedface",
                    "source_scan_root": "steering"
                }
            }
        });
        std::fs::write(
            project.kiro_dir().join("installed-steering.json"),
            serde_json::to_vec_pretty(&steering_json).expect("ser steering"),
        )
        .expect("steering tracking");

        let result = project.installed_plugins().expect("installed_plugins");
        assert_eq!(result.plugins.len(), 2, "two plugins expected");
        assert!(
            result.partial_load_warnings.is_empty(),
            "clean state must produce no warnings"
        );

        let plug_a = result
            .plugins
            .iter()
            .find(|p| p.plugin == "plug-a")
            .expect("plug-a present");
        assert_eq!(plug_a.skill_count, 1);
        assert_eq!(plug_a.steering_count, 1);
        assert_eq!(plug_a.agent_count, 0);
        assert_eq!(plug_a.installed_skills, vec!["alpha".to_string()]);
        assert_eq!(plug_a.installed_version.as_deref(), Some("1.0.0"));

        let plug_b = result
            .plugins
            .iter()
            .find(|p| p.plugin == "plug-b")
            .expect("plug-b present");
        assert_eq!(plug_b.skill_count, 0);
        assert_eq!(plug_b.steering_count, 1);
        assert_eq!(plug_b.agent_count, 0);
    }

    #[test]
    fn installed_plugins_uses_strict_greater_for_latest_tie_break() {
        use chrono::Utc;
        let dir = tempfile::tempdir().expect("tempdir");
        let project = KiroProject::new(dir.path().to_path_buf());
        std::fs::create_dir_all(project.kiro_dir()).expect("kiro dir");

        // Same timestamp on skill (v1) and steering (v2). Skills are
        // iterated first; `>` keeps skill's version. `>=` would let
        // steering (iterated second) overwrite, producing v2.
        let same_time = Utc::now();
        let skills_json = serde_json::json!({
            "skills": {
                "alpha": {
                    "marketplace": "mp", "plugin": "p",
                    "version": "1.0.0", "installed_at": same_time,
                    "source_hash": "deadbeef",
                    "installed_hash": "deadbeef",
                    "source_scan_root": "skills"
                }
            }
        });
        std::fs::write(
            project.kiro_dir().join("installed-skills.json"),
            serde_json::to_vec_pretty(&skills_json).expect("ser"),
        )
        .expect("skills");

        let steering_json = serde_json::json!({
            "files": {
                "guide.md": {
                    "marketplace": "mp", "plugin": "p",
                    "version": "2.0.0", "installed_at": same_time,
                    "source_hash": "cafebabe", "installed_hash": "cafebabe",
                    "source_scan_root": "steering"
                }
            }
        });
        std::fs::write(
            project.kiro_dir().join("installed-steering.json"),
            serde_json::to_vec_pretty(&steering_json).expect("ser"),
        )
        .expect("steering");

        let result = project.installed_plugins().expect("installed_plugins");
        assert_eq!(result.plugins.len(), 1);
        assert_eq!(
            result.plugins[0].installed_version.as_deref(),
            Some("1.0.0"),
            "tied timestamps must keep first-iterated version (skills); \
             got steering's 2.0.0 — A-17 `>` tie-break broken"
        );
    }

    /// Build a tracking JSON map with three entries keyed by `keys`,
    /// where each value is a tracking-meta object built by `value_for`.
    /// Used by [`installed_plugins_returns_sorted_vecs_per_plugin`] to
    /// keep the test under clippy's `too_many_lines` threshold.
    fn build_tracking_map<F>(keys: [&str; 3], value_for: F) -> serde_json::Value
    where
        F: Fn() -> serde_json::Value,
    {
        let mut map = serde_json::Map::new();
        for k in keys {
            map.insert(k.to_string(), value_for());
        }
        serde_json::Value::Object(map)
    }

    #[test]
    fn installed_plugins_returns_sorted_vecs_per_plugin() {
        // I1 regression: per-plugin Vecs (`installed_skills`,
        // `installed_steering`, `installed_agents`) are pushed in
        // HashMap iteration order, which is nondeterministic. Sorting
        // post-fold gives a stable wire format. Seed three skills,
        // three steering files, and three agents in non-alphabetic
        // tracking order; the result Vecs must be alphabetically
        // sorted regardless of which order the HashMap chose to yield.
        use chrono::Utc;
        let dir = tempfile::tempdir().expect("tempdir");
        let project = KiroProject::new(dir.path().to_path_buf());
        std::fs::create_dir_all(project.kiro_dir()).expect("kiro dir");

        let now = Utc::now();
        let skill_meta = || {
            serde_json::json!({
                "marketplace": "mp", "plugin": "p",
                "version": "1.0.0", "installed_at": now,
                "source_hash": "x",
                "installed_hash": "x",
                "source_scan_root": "skills"
            })
        };
        let steering_meta = || {
            serde_json::json!({
                "marketplace": "mp", "plugin": "p",
                "version": "1.0.0", "installed_at": now,
                "source_hash": "x", "installed_hash": "x",
                "source_scan_root": "steering"
            })
        };
        let agent_meta = || {
            serde_json::json!({
                "marketplace": "mp", "plugin": "p",
                "version": "1.0.0", "installed_at": now,
                "dialect": "claude",
                "source_path": "agents/x.md",
                "source_hash": "x",
                "installed_hash": "x"
            })
        };

        // Non-alphabetic order on every tracking file — exercises
        // the sort-post-fold contract regardless of HashMap order.
        let skills_json = serde_json::json!({
            "skills": build_tracking_map(["gamma", "alpha", "beta"], skill_meta),
        });
        let steering_json = serde_json::json!({
            "files": build_tracking_map(["z-guide.md", "a-guide.md", "m-guide.md"], steering_meta),
        });
        let agents_json = serde_json::json!({
            "agents": build_tracking_map(["zeta", "alpha-agent", "mid-agent"], agent_meta),
        });

        std::fs::write(
            project.kiro_dir().join("installed-skills.json"),
            serde_json::to_vec_pretty(&skills_json).expect("ser skills"),
        )
        .expect("skills");
        std::fs::write(
            project.kiro_dir().join("installed-steering.json"),
            serde_json::to_vec_pretty(&steering_json).expect("ser steering"),
        )
        .expect("steering");
        std::fs::write(
            project.kiro_dir().join("installed-agents.json"),
            serde_json::to_vec_pretty(&agents_json).expect("ser agents"),
        )
        .expect("agents");

        let result = project.installed_plugins().expect("installed_plugins");
        assert_eq!(result.plugins.len(), 1);
        assert_eq!(
            result.plugins[0].installed_skills,
            vec!["alpha".to_string(), "beta".to_string(), "gamma".to_string()],
            "skills must be sorted post-fold; HashMap iteration order leak"
        );
        assert_eq!(
            result.plugins[0].installed_steering,
            vec![
                std::path::PathBuf::from("a-guide.md"),
                std::path::PathBuf::from("m-guide.md"),
                std::path::PathBuf::from("z-guide.md"),
            ],
            "steering must be sorted post-fold"
        );
        assert_eq!(
            result.plugins[0].installed_agents,
            vec![
                "alpha-agent".to_string(),
                "mid-agent".to_string(),
                "zeta".to_string(),
            ],
            "agents must be sorted post-fold"
        );
    }

    #[test]
    fn installed_plugins_returns_full_view_with_empty_warnings_on_clean_state() {
        // I13 regression: a clean (no tracking files) state must
        // surface as an empty view with no warnings — not as an error.
        let (_dir, project) = temp_project();

        let view = project.installed_plugins().expect("clean state");
        assert!(
            view.plugins.is_empty(),
            "no plugins installed yet, got: {:?}",
            view.plugins
        );
        assert!(
            view.partial_load_warnings.is_empty(),
            "missing tracking files are NOT corruption — warnings must be empty, got: {:?}",
            view.partial_load_warnings
        );
    }

    #[test]
    fn installed_plugins_returns_partial_view_when_one_tracking_file_corrupt() {
        // I13 regression: a corrupt tracking file (invalid JSON) must
        // NOT abort the aggregator. The other two tracking files
        // contribute their plugins; the corruption is recorded as a
        // `partial_load_warnings` entry so the UI can render a banner.
        use chrono::Utc;
        let (_dir, project) = temp_project();
        std::fs::create_dir_all(project.kiro_dir()).expect("kiro dir");

        // Steering: clean — must contribute its plugin to the view.
        let now = Utc::now();
        std::fs::write(
            project.kiro_dir().join("installed-steering.json"),
            serde_json::to_vec_pretty(&serde_json::json!({
                "files": {
                    "guide.md": {
                        "marketplace": "mp", "plugin": "p",
                        "version": "1.0.0", "installed_at": now,
                        "source_hash": "x", "installed_hash": "x",
                        "source_scan_root": "steering",
                    }
                }
            }))
            .expect("ser steering"),
        )
        .expect("write steering");

        // Skills: corrupt JSON — must surface as a load warning,
        // not abort the aggregator.
        std::fs::write(
            project.kiro_dir().join("installed-skills.json"),
            "{ this is not valid JSON",
        )
        .expect("write corrupt skills");

        let view = project.installed_plugins().expect("partial-load tolerant");
        assert_eq!(
            view.plugins.len(),
            1,
            "steering loaded; one plugin expected, got: {:?}",
            view.plugins
        );
        assert_eq!(view.plugins[0].plugin, "p");
        assert_eq!(view.plugins[0].steering_count, 1);
        assert_eq!(view.plugins[0].skill_count, 0);

        assert_eq!(
            view.partial_load_warnings.len(),
            1,
            "expected one warning for corrupt skills tracking, got: {:?}",
            view.partial_load_warnings
        );
        assert_eq!(
            view.partial_load_warnings[0].tracking_file, "installed-skills.json",
            "warning must identify which tracking file failed"
        );
        assert!(
            !view.partial_load_warnings[0].error.is_empty(),
            "warning's error string must be populated via error_full_chain"
        );
    }

    /// `rstest` fixture for steering install collision tests. Stages a
    /// single steering source file and the project root; tests reuse
    /// the fixture's `install_steering` helper rather than re-typing
    /// the `SteeringInstallContext` bundle. Mirrors the
    /// [`CompanionBundle`] shape from
    /// `install_native_companions_idempotent_when_source_hash_matches`.
    struct SteeringFile {
        /// Owns the tempdir lifetime AND exposes its path for tests
        /// that need to stage sibling source trees (e.g. cross-plugin
        /// transfer).
        scratch: tempfile::TempDir,
        project: KiroProject,
        scan_root: PathBuf,
        rel_path: PathBuf,
        source_hash: String,
    }

    impl SteeringFile {
        /// Re-stage the source with new content and recompute the hash,
        /// preserving the same `rel_path`. Used by the content-changed
        /// test to bump the body without rebuilding the whole fixture.
        fn rewrite_source(&mut self, body: &[u8]) {
            fs::write(self.scan_root.join(&self.rel_path), body).expect("rewrite source");
            self.source_hash =
                crate::hash::hash_artifact(&self.scan_root, std::slice::from_ref(&self.rel_path))
                    .expect("re-hash");
        }

        /// Path to the absolute source file the discovered handle points at.
        fn source_path(&self) -> PathBuf {
            self.scan_root.join(&self.rel_path)
        }
    }

    fn install_steering(
        f: &SteeringFile,
        plugin: &str,
        mode: crate::service::InstallMode,
    ) -> Result<crate::steering::InstalledSteeringOutcome, crate::steering::SteeringError> {
        let discovered = crate::agent::DiscoveredNativeFile {
            source: f.source_path(),
            scan_root: f.scan_root.clone(),
        };
        let mp_name = mp("m");
        let pn_name = pn(plugin);
        f.project.install_steering_file(
            &discovered,
            &f.source_hash,
            crate::steering::SteeringInstallContext {
                mode,
                marketplace: &mp_name,
                plugin: &pn_name,
                version: None,
                plugin_dir: f.scratch.path(),
            },
        )
    }

    #[fixture]
    fn steering_file() -> SteeringFile {
        let dir = tempfile::tempdir().expect("tempdir");
        let project = KiroProject::new(dir.path().to_path_buf());
        let scan_root = dir.path().join("steering-src");
        fs::create_dir_all(&scan_root).expect("create scan_root");
        let rel_path = PathBuf::from("guide.md");
        fs::write(scan_root.join(&rel_path), b"v1 body").expect("write source");
        let source_hash =
            crate::hash::hash_artifact(&scan_root, std::slice::from_ref(&rel_path)).expect("hash");
        SteeringFile {
            scratch: dir,
            project,
            scan_root,
            rel_path,
            source_hash,
        }
    }

    #[rstest]
    fn install_steering_idempotent_when_source_hash_matches(steering_file: SteeringFile) {
        let first = install_steering(&steering_file, "p", crate::service::InstallMode::New)
            .expect("first install");
        assert_eq!(first.kind, InstallOutcomeKind::Installed);

        let second = install_steering(&steering_file, "p", crate::service::InstallMode::New)
            .expect("second install");
        assert_eq!(second.kind, InstallOutcomeKind::Idempotent);
        assert_eq!(
            second.installed_hash, first.installed_hash,
            "idempotent reinstall must report the prior installed_hash"
        );
        // Wire-format regression guard: the idempotent path must report the
        // ORIGINAL source path, not the destination. The classifier doesn't
        // see the source, so before SteeringIdempotentEcho landed it would
        // fall back to setting `source = dest`, which then leaks the
        // `.kiro/steering/...` path through the specta-derived TS binding
        // for `InstalledSteeringOutcome`.
        assert_eq!(
            second.source,
            steering_file.source_path(),
            "idempotent outcome.source must be the original source path, not the destination"
        );
        assert_ne!(
            second.source, second.destination,
            "outcome.source must not equal outcome.destination on idempotent reinstall"
        );
    }

    #[rstest]
    fn install_steering_content_changed_requires_force(mut steering_file: SteeringFile) {
        install_steering(&steering_file, "p", crate::service::InstallMode::New)
            .expect("first install");

        steering_file.rewrite_source(b"v2 body");

        let err = install_steering(&steering_file, "p", crate::service::InstallMode::New)
            .expect_err("content change without force must fail");
        assert!(
            matches!(
                err,
                crate::steering::SteeringError::ContentChangedRequiresForce { .. }
            ),
            "expected ContentChangedRequiresForce, got {err:?}"
        );

        let outcome = install_steering(&steering_file, "p", crate::service::InstallMode::Force)
            .expect("force install");
        assert_eq!(outcome.kind, InstallOutcomeKind::ForceOverwrote);
        // The new content must have landed on disk.
        assert_eq!(
            fs::read(steering_file.project.steering_dir().join("guide.md")).unwrap(),
            b"v2 body"
        );
    }

    #[rstest]
    fn install_steering_cross_plugin_clash_fails_loudly(steering_file: SteeringFile) {
        // Plugin A installs first, then plugin B tries to install at the
        // same rel path.
        install_steering(&steering_file, "plugin-a", crate::service::InstallMode::New)
            .expect("plugin-a first install");

        // Stage a sibling source for plugin-b with different content.
        let scan_b = steering_file.scratch.path().join("b-src");
        fs::create_dir_all(&scan_b).unwrap();
        let rel_b = PathBuf::from("guide.md");
        fs::write(scan_b.join(&rel_b), b"from-b").unwrap();
        let source_hash_b =
            crate::hash::hash_artifact(&scan_b, std::slice::from_ref(&rel_b)).unwrap();
        let discovered_b = crate::agent::DiscoveredNativeFile {
            source: scan_b.join(&rel_b),
            scan_root: scan_b.clone(),
        };

        let mp_name = mp("m");
        let pn_name = pn("plugin-b");
        let err = steering_file
            .project
            .install_steering_file(
                &discovered_b,
                &source_hash_b,
                crate::steering::SteeringInstallContext {
                    mode: crate::service::InstallMode::New,
                    marketplace: &mp_name,
                    plugin: &pn_name,
                    version: None,
                    plugin_dir: steering_file.scratch.path(),
                },
            )
            .expect_err("cross-plugin clash must fail");
        match err {
            crate::steering::SteeringError::PathOwnedByOtherPlugin { rel, owner } => {
                assert_eq!(rel, PathBuf::from("guide.md"));
                assert_eq!(owner, "plugin-a");
            }
            other => panic!("expected PathOwnedByOtherPlugin, got {other:?}"),
        }

        // Force mode transfers ownership.
        let outcome = steering_file
            .project
            .install_steering_file(
                &discovered_b,
                &source_hash_b,
                crate::steering::SteeringInstallContext {
                    mode: crate::service::InstallMode::Force,
                    marketplace: &mp_name,
                    plugin: &pn_name,
                    version: None,
                    plugin_dir: steering_file.scratch.path(),
                },
            )
            .expect("force-mode transfer");
        assert_eq!(outcome.kind, InstallOutcomeKind::ForceOverwrote);

        let tracking = steering_file.project.load_installed_steering().unwrap();
        let entry = tracking
            .files
            .get(std::path::Path::new("guide.md"))
            .expect("tracking entry");
        assert_eq!(
            entry.plugin, "plugin-b",
            "ownership must transfer to plugin-b under force"
        );
    }

    #[cfg(unix)]
    #[rstest]
    fn install_steering_refuses_hardlinked_source(steering_file: SteeringFile) {
        // A hardlinked steering source could exfiltrate sensitive host
        // files (`~/.ssh/id_rsa`) into `.kiro/steering/`. Discovery's
        // symlink/junction filter doesn't catch hardlinks (the share is
        // at the inode level, not the path).
        let target = steering_file.scratch.path().join("real.md");
        fs::write(&target, b"sensitive").unwrap();
        let linked = steering_file.scan_root.join("linked.md");
        fs::hard_link(&target, &linked).expect("create hardlink");

        let source_hash = crate::hash::hash_artifact(
            &steering_file.scan_root,
            std::slice::from_ref(&PathBuf::from("linked.md")),
        )
        .unwrap();
        let discovered = crate::agent::DiscoveredNativeFile {
            source: linked.clone(),
            scan_root: steering_file.scan_root.clone(),
        };

        let mp_name = mp("m");
        let pn_name = pn("p");
        let err = steering_file
            .project
            .install_steering_file(
                &discovered,
                &source_hash,
                crate::steering::SteeringInstallContext {
                    mode: crate::service::InstallMode::New,
                    marketplace: &mp_name,
                    plugin: &pn_name,
                    version: None,
                    plugin_dir: steering_file.scratch.path(),
                },
            )
            .expect_err("hardlinked source must be refused");
        match err {
            crate::steering::SteeringError::SourceHardlinked { path, nlink } => {
                assert_eq!(path, linked);
                assert!(nlink >= 2, "nlink must reflect the hardlink share");
            }
            other => panic!("expected SourceHardlinked, got {other:?}"),
        }

        // Hardlinked source must NOT have landed in the project.
        assert!(
            !steering_file
                .project
                .steering_dir()
                .join("linked.md")
                .exists(),
            "destination must remain untouched after hardlink rejection"
        );
    }

    #[rstest]
    fn install_steering_orphan_at_destination_fails_loudly(steering_file: SteeringFile) {
        // Pre-create an unrelated file at the destination path with no
        // tracking entry — should fail without --force.
        fs::create_dir_all(steering_file.project.steering_dir()).unwrap();
        fs::write(
            steering_file.project.steering_dir().join("guide.md"),
            b"orphan",
        )
        .unwrap();

        let err = install_steering(&steering_file, "p", crate::service::InstallMode::New)
            .expect_err("orphan must fail without force");
        assert!(
            matches!(
                err,
                crate::steering::SteeringError::OrphanFileAtDestination { .. }
            ),
            "expected OrphanFileAtDestination, got {err:?}"
        );

        let outcome = install_steering(&steering_file, "p", crate::service::InstallMode::Force)
            .expect("force install over orphan");
        assert_eq!(outcome.kind, InstallOutcomeKind::ForceOverwrote);
    }

    #[test]
    fn install_steering_file_writes_to_kiro_steering_with_hashes() {
        let (_dir, project) = temp_project();

        let scan_root = project.root.join("source-steering");
        fs::create_dir_all(&scan_root).unwrap();
        let src = scan_root.join("guide.md");
        fs::write(&src, b"# Steering Guide\n\nbody").unwrap();

        let source_hash =
            crate::hash::hash_artifact(&scan_root, &[PathBuf::from("guide.md")]).unwrap();

        let discovered = crate::agent::DiscoveredNativeFile {
            source: src.clone(),
            scan_root: scan_root.clone(),
        };

        let mp_name = mp("marketplace-x");
        let pn_name = pn("plugin-y");
        let outcome = project
            .install_steering_file(
                &discovered,
                &source_hash,
                crate::steering::SteeringInstallContext {
                    mode: crate::service::InstallMode::New,
                    marketplace: &mp_name,
                    plugin: &pn_name,
                    version: Some("0.1.0"),
                    plugin_dir: &project.root,
                },
            )
            .expect("install_steering_file");

        let dest = project.steering_dir().join("guide.md");
        assert_eq!(outcome.destination, dest);
        assert!(dest.exists(), "destination file must exist on disk");
        assert_eq!(fs::read(&dest).unwrap(), b"# Steering Guide\n\nbody");
        assert_eq!(outcome.source_hash, source_hash);
        assert!(outcome.installed_hash.starts_with("blake3:"));
        assert_eq!(outcome.kind, InstallOutcomeKind::Installed);

        // Tracking entry must be present.
        let tracking = project.load_installed_steering().unwrap();
        let entry = tracking
            .files
            .get(std::path::Path::new("guide.md"))
            .expect("tracking entry written");
        assert_eq!(entry.plugin, "plugin-y");
        assert_eq!(entry.marketplace, "marketplace-x");
        assert_eq!(entry.version.as_deref(), Some("0.1.0"));
    }

    fn write_agent(tmp: &Path, name: &str, body: &str) -> PathBuf {
        let p = tmp.join(format!("{name}.md"));
        fs::write(&p, body).unwrap();
        p
    }

    fn parse_and_map(source: &Path) -> (AgentDefinition, Vec<MappedTool>) {
        let def = crate::agent::parse_agent_file(source).expect("parse");
        let (mapped, _unmapped) = match def.dialect {
            AgentDialect::Claude => crate::agent::tools::map_claude_tools(&def.source_tools),
            AgentDialect::Copilot => crate::agent::tools::map_copilot_tools(&def.source_tools),
            AgentDialect::Native => panic!("translated test helper does not support Native"),
        };
        (def, mapped)
    }

    fn sample_agent_meta() -> InstalledAgentMeta {
        InstalledAgentMeta {
            marketplace: mp("mp"),
            plugin: pn("p"),
            version: None,
            installed_at: Utc::now(),
            dialect: AgentDialect::Claude,
            source_path: RelativePath::new("agents/reviewer.md").expect("valid"),
            source_hash: String::new(),
            installed_hash: String::new(),
        }
    }

    #[test]
    fn install_agent_writes_json_and_prompt() {
        let (_dir, project) = temp_project();
        let src_tmp = tempfile::tempdir().unwrap();
        let src = write_agent(
            src_tmp.path(),
            "reviewer",
            "---\nname: reviewer\ndescription: Reviews\n---\nYou are a reviewer.\n",
        );
        let (def, mapped) = parse_and_map(&src);

        project
            .install_agent(&def, &mapped, sample_agent_meta(), None)
            .expect("install");

        let json_path = project.root.join(".kiro/agents/reviewer.json");
        let prompt_path = project.root.join(".kiro/agents/prompts/reviewer.md");
        assert!(json_path.exists(), "JSON written");
        assert!(prompt_path.exists(), "prompt markdown written");

        let json: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&json_path).unwrap()).unwrap();
        assert_eq!(json["name"], "reviewer");
        assert_eq!(json["prompt"], "file://./prompts/reviewer.md");
        assert_eq!(json["description"], "Reviews");

        let prompt = fs::read_to_string(&prompt_path).unwrap();
        assert!(
            prompt.starts_with("You are a reviewer."),
            "prompt body written without frontmatter, got: {prompt:?}"
        );
    }

    #[test]
    fn install_agent_rejects_duplicate() {
        let (_dir, project) = temp_project();
        let src_tmp = tempfile::tempdir().unwrap();
        let src = write_agent(src_tmp.path(), "a", "---\nname: a\n---\nbody\n");
        let (def, mapped) = parse_and_map(&src);

        project
            .install_agent(&def, &mapped, sample_agent_meta(), None)
            .unwrap();
        let err = project
            .install_agent(&def, &mapped, sample_agent_meta(), None)
            .unwrap_err();
        assert!(matches!(
            err,
            crate::error::Error::Agent(AgentError::AlreadyInstalled { .. })
        ));
    }

    #[test]
    fn install_agent_force_overwrites_existing_tracked_agent() {
        let (_dir, project) = temp_project();
        let src_tmp = tempfile::tempdir().unwrap();
        let src_v1 = write_agent(
            src_tmp.path(),
            "rev",
            "---\nname: rev\n---\nversion one body\n",
        );
        let (def_v1, mapped_v1) = parse_and_map(&src_v1);
        project
            .install_agent(&def_v1, &mapped_v1, sample_agent_meta(), None)
            .expect("first install");

        let src_v2 = write_agent(
            src_tmp.path(),
            "rev2",
            "---\nname: rev\n---\nversion two body\n",
        );
        let (def_v2, mapped_v2) = parse_and_map(&src_v2);
        project
            .install_agent_force(&def_v2, &mapped_v2, sample_agent_meta(), None)
            .expect("force install should overwrite");

        let prompt = fs::read_to_string(project.root.join(".kiro/agents/prompts/rev.md")).unwrap();
        assert!(
            prompt.contains("version two body"),
            "prompt should be replaced with v2, got: {prompt}"
        );
    }

    #[test]
    fn install_agent_force_overwrites_orphaned_files() {
        // Pre-plant orphan files (no tracking entry) — force install must
        // clean them up rather than error with AlreadyExists.
        let (_dir, project) = temp_project();
        fs::create_dir_all(project.root.join(".kiro/agents/prompts")).unwrap();
        fs::write(project.root.join(".kiro/agents/orphan.json"), b"{}").unwrap();
        fs::write(
            project.root.join(".kiro/agents/prompts/orphan.md"),
            b"stale prompt",
        )
        .unwrap();

        let src_tmp = tempfile::tempdir().unwrap();
        let src = write_agent(
            src_tmp.path(),
            "orphan",
            "---\nname: orphan\n---\nfresh body\n",
        );
        let (def, mapped) = parse_and_map(&src);

        project
            .install_agent_force(&def, &mapped, sample_agent_meta(), None)
            .expect("force install should overwrite orphans");

        let prompt =
            fs::read_to_string(project.root.join(".kiro/agents/prompts/orphan.md")).unwrap();
        assert!(prompt.contains("fresh body"), "got: {prompt}");
    }

    #[test]
    fn install_agent_force_still_rejects_unsafe_name() {
        // --force is not a bypass for name validation. The parser rejects
        // unsafe names at frontmatter time, so construct the definition
        // directly to exercise the validate_name guard inside install_agent_inner.
        let (_dir, project) = temp_project();
        let def = AgentDefinition {
            name: "../escape".to_string(),
            description: None,
            prompt_body: "body".to_string(),
            model: None,
            source_tools: Vec::new(),
            mcp_servers: std::collections::BTreeMap::new(),
            dialect: AgentDialect::Claude,
        };

        let err = project
            .install_agent_force(&def, &[], sample_agent_meta(), None)
            .expect_err("unsafe name must be rejected under force");
        assert!(
            matches!(
                err,
                crate::error::Error::Validation(crate::error::ValidationError::InvalidName { .. })
            ),
            "expected InvalidName, got: {err:?}"
        );
    }

    #[test]
    fn install_agent_updates_tracking() {
        let (_dir, project) = temp_project();
        let src_tmp = tempfile::tempdir().unwrap();
        let src = write_agent(src_tmp.path(), "a", "---\nname: a\n---\nbody\n");
        let (def, mapped) = parse_and_map(&src);

        project
            .install_agent(&def, &mapped, sample_agent_meta(), None)
            .unwrap();

        let tracking_path = project.root.join(".kiro/installed-agents.json");
        let tracking: InstalledAgents =
            serde_json::from_str(&fs::read_to_string(tracking_path).unwrap()).unwrap();
        assert!(tracking.agents.contains_key("a"));
        assert_eq!(tracking.agents["a"].dialect, AgentDialect::Claude);
    }

    #[test]
    fn install_agent_rejects_unsafe_name() {
        let (_dir, project) = temp_project();
        let src_tmp = tempfile::tempdir().unwrap();
        let src = write_agent(src_tmp.path(), "x", "---\nname: x\n---\nbody\n");
        let (mut def, mapped) = parse_and_map(&src);
        def.name = "../escape".into();
        let err = project
            .install_agent(&def, &mapped, sample_agent_meta(), None)
            .unwrap_err();
        assert!(matches!(err, crate::error::Error::Validation(_)));
    }

    #[test]
    fn install_agent_emits_tools_and_allowed_tools_from_mapping() {
        let (_dir, project) = temp_project();
        let src_tmp = tempfile::tempdir().unwrap();
        let src = write_agent(
            src_tmp.path(),
            "mixed",
            "---\nname: mixed\ntools: [Read, Bash]\n---\nbody\n",
        );
        let (def, mapped) = parse_and_map(&src);
        assert_eq!(mapped.len(), 2, "sanity: both tools mapped");

        project
            .install_agent(&def, &mapped, sample_agent_meta(), None)
            .expect("install");

        let json_path = project.root.join(".kiro/agents/mixed.json");
        let json: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&json_path).unwrap()).unwrap();
        let allowed = json["allowedTools"].as_array().unwrap();
        // Native tools go to allowedTools, not tools.
        assert!(allowed.contains(&serde_json::Value::String("read".into())));
        assert!(allowed.contains(&serde_json::Value::String("shell".into())));
        assert!(json.get("tools").is_none(), "no MCP refs here");
    }

    #[test]
    fn install_agent_no_staging_dir_left_behind_on_success() {
        let (_dir, project) = temp_project();
        let src_tmp = tempfile::tempdir().unwrap();
        let src = write_agent(src_tmp.path(), "clean", "---\nname: clean\n---\nbody\n");
        let (def, mapped) = parse_and_map(&src);

        project
            .install_agent(&def, &mapped, sample_agent_meta(), None)
            .unwrap();

        // Staging lives directly under .kiro/, not under agents/.
        let kiro_dir = project.root.join(".kiro");
        let leftovers: Vec<_> = fs::read_dir(&kiro_dir)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| {
                e.file_name()
                    .to_str()
                    .is_some_and(|s| s.starts_with("_installing-agent"))
            })
            .collect();
        assert!(
            leftovers.is_empty(),
            "no staging directories should remain after successful install"
        );
    }

    #[test]
    fn install_agent_refuses_to_clobber_orphaned_files() {
        let (_dir, project) = temp_project();
        let src_tmp = tempfile::tempdir().unwrap();
        let src = write_agent(src_tmp.path(), "orphan", "---\nname: orphan\n---\nbody\n");
        let (def, mapped) = parse_and_map(&src);

        // Pre-create an orphan JSON (no tracking entry) — a prior crash or
        // manual tinkering.
        let agents_dir = project.root.join(".kiro/agents");
        fs::create_dir_all(&agents_dir).unwrap();
        fs::write(agents_dir.join("orphan.json"), b"{}").unwrap();

        let err = project
            .install_agent(&def, &mapped, sample_agent_meta(), None)
            .unwrap_err();
        // Surfaced as an Io error (AlreadyExists) with a message pointing at
        // the offending files.
        assert!(matches!(err, crate::error::Error::Io(_)));
        assert!(err.to_string().contains("orphan"));
    }

    #[test]
    fn install_agent_rollback_removes_json_when_prompt_target_already_a_dir() {
        // Force `fs::rename(staging_prompt, prompt_target)` to fail by making
        // prompt_target a non-empty directory. After the failure, the JSON
        // rollback must remove `.kiro/agents/<name>.json`.
        let (_dir, project) = temp_project();
        let src_tmp = tempfile::tempdir().unwrap();
        let src = write_agent(src_tmp.path(), "rb", "---\nname: rb\n---\nbody\n");
        let (def, mapped) = parse_and_map(&src);

        // Pre-create a non-empty directory where the prompt file would go.
        let prompts_dir = project.root.join(".kiro/agents/prompts");
        fs::create_dir_all(prompts_dir.join("rb.md")).unwrap();
        fs::write(prompts_dir.join("rb.md").join("inside.txt"), b"x").unwrap();

        let err = project
            .install_agent(&def, &mapped, sample_agent_meta(), None)
            .unwrap_err();
        assert!(matches!(err, crate::error::Error::Io(_)));

        // JSON target must not exist (rolled back).
        assert!(
            !project.root.join(".kiro/agents/rb.json").exists(),
            "JSON file should have been rolled back after prompt-rename failure"
        );
        // Tracking must not contain the agent.
        let tracking = project.load_installed_agents().unwrap();
        assert!(!tracking.agents.contains_key("rb"));
    }

    #[test]
    fn install_agent_serializes_concurrent_same_name_installs() {
        // Mirrors `install_skill_from_dir_serializes_concurrent_same_name_installs`:
        // two threads racing to install the same agent name. Exactly one
        // should succeed; the other must see AlreadyInstalled. No staging
        // dirs may leak under `.kiro/`.
        let (_dir, project) = temp_project();
        let project = std::sync::Arc::new(project);

        let src_tmp = tempfile::tempdir().unwrap();
        let src = write_agent(src_tmp.path(), "racey", "---\nname: racey\n---\nbody\n");
        let (def, mapped) = parse_and_map(&src);
        let def = std::sync::Arc::new(def);
        let mapped = std::sync::Arc::new(mapped);

        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));

        let handles: Vec<_> = (0..2)
            .map(|_| {
                let project = std::sync::Arc::clone(&project);
                let barrier = std::sync::Arc::clone(&barrier);
                let def = std::sync::Arc::clone(&def);
                let mapped = std::sync::Arc::clone(&mapped);
                std::thread::spawn(move || {
                    barrier.wait();
                    project.install_agent(&def, &mapped, sample_agent_meta(), None)
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
                    Err(crate::error::Error::Agent(
                        AgentError::AlreadyInstalled { .. }
                    ))
                )
            })
            .count();
        assert_eq!(ok_count, 1, "exactly one install should succeed");
        assert_eq!(already_count, 1, "the other should be AlreadyInstalled");

        let kiro = project.root.join(".kiro");
        let leftover: Vec<_> = fs::read_dir(&kiro)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with("_installing-agent-")
            })
            .collect();
        assert!(
            leftover.is_empty(),
            "no agent staging dirs should remain after race: {leftover:?}"
        );
    }

    #[test]
    fn install_agent_rollback_removes_files_when_tracking_write_fails() {
        // Pre-create the tracking path as a directory — `write_agent_tracking`
        // will fail, and the flow should roll back both files.
        let (_dir, project) = temp_project();
        let src_tmp = tempfile::tempdir().unwrap();
        let src = write_agent(src_tmp.path(), "trkfail", "---\nname: trkfail\n---\nbody\n");
        let (def, mapped) = parse_and_map(&src);

        // `.kiro/installed-agents.json` as a directory → atomic_write fails.
        fs::create_dir_all(project.root.join(".kiro/installed-agents.json")).unwrap();

        let err = project
            .install_agent(&def, &mapped, sample_agent_meta(), None)
            .unwrap_err();
        assert!(matches!(err, crate::error::Error::Io(_)));

        assert!(
            !project.root.join(".kiro/agents/trkfail.json").exists(),
            "JSON file should have been rolled back after tracking failure"
        );
        assert!(
            !project
                .root
                .join(".kiro/agents/prompts/trkfail.md")
                .exists(),
            "prompt file should have been rolled back after tracking failure"
        );
    }

    #[test]
    fn install_agent_force_restores_backups_when_companion_hash_fails() {
        // Regression test for the P-6 (backup-then-swap) atomicity gap that
        // used to live in `synthesize_companion_entry`. Setup:
        //   1. Install agent `keepme` (plugin P, content v1).
        //   2. Install agent `gone` (plugin P) — extends companion_entry.files
        //      to [prompts/keepme.md, prompts/gone.md].
        //   3. Delete `agents/prompts/gone.md` from disk by hand.
        //   4. Force-reinstall `keepme` with new content v2.
        //
        // The promote phase backs up A's existing JSON + prompt to .kiro-bak.
        // synthesize_companion_entry then walks the full companion file set
        // — but `prompts/gone.md` is missing, so hash_artifact errors. The
        // caller must restore the backups so `keepme`'s prior install
        // (v1 content) survives intact rather than being clobbered.
        let (_dir, project) = temp_project();
        let src_tmp = tempfile::tempdir().unwrap();

        // Step 1: install keepme v1.
        let src_v1 = write_agent(
            src_tmp.path(),
            "keepme",
            "---\nname: keepme\n---\nv1 prompt body\n",
        );
        let (def_v1, mapped_v1) = parse_and_map(&src_v1);
        project
            .install_agent(&def_v1, &mapped_v1, sample_agent_meta(), None)
            .expect("v1 install");

        let json_target = project.root.join(".kiro/agents/keepme.json");
        let prompt_target = project.root.join(".kiro/agents/prompts/keepme.md");
        let prompt_v1_bytes = fs::read(&prompt_target).unwrap();
        let json_v1_bytes = fs::read(&json_target).unwrap();

        // Step 2: install a second agent under the same plugin so the
        // companion entry's files vec grows to two entries.
        let src_gone = write_agent(src_tmp.path(), "gone", "---\nname: gone\n---\nbody\n");
        let (def_gone, mapped_gone) = parse_and_map(&src_gone);
        project
            .install_agent(&def_gone, &mapped_gone, sample_agent_meta(), None)
            .expect("gone install");

        // Step 3: delete the second agent's prompt file from disk so the
        // companion-hash walk will fail mid-install on the next force call.
        fs::remove_file(project.root.join(".kiro/agents/prompts/gone.md"))
            .expect("remove gone prompt");

        // Step 4: force-reinstall keepme with v2 content. The hash failure
        // should trigger backup restoration.
        let src_v2 = src_tmp.path().join("keepme_v2.md");
        fs::write(&src_v2, "---\nname: keepme\n---\nv2 prompt body\n").unwrap();
        let (def_v2, mapped_v2) = parse_and_map(&src_v2);

        let err = project
            .install_agent_force(&def_v2, &mapped_v2, sample_agent_meta(), None)
            .expect_err("force install must fail when companion hash fails");
        assert!(
            matches!(err, crate::error::Error::Hash(_)),
            "expected Error::Hash, got {err:?}"
        );

        // The user's prior install must be intact — backups restored.
        assert!(
            json_target.exists(),
            "keepme.json must survive the failed force install"
        );
        assert!(
            prompt_target.exists(),
            "prompts/keepme.md must survive the failed force install"
        );
        assert_eq!(
            fs::read(&json_target).unwrap(),
            json_v1_bytes,
            "keepme.json content must match v1 (backup restored, not v2)"
        );
        assert_eq!(
            fs::read(&prompt_target).unwrap(),
            prompt_v1_bytes,
            "prompts/keepme.md content must match v1 (backup restored, not v2)"
        );

        // No leftover .kiro-bak files — rollback path renamed them back.
        let agents_dir = project.root.join(".kiro/agents");
        for entry in fs::read_dir(&agents_dir).unwrap() {
            let path = entry.unwrap().path();
            assert!(
                !path.to_string_lossy().ends_with(".kiro-bak"),
                "no leftover backup file expected: {}",
                path.display()
            );
        }
    }

    #[test]
    fn install_agent_force_transfers_companion_ownership_across_plugins() {
        // Regression test for the v1 limitation Stage 1 Task 14 documented:
        // a translated agent overwritten by a different plugin via --force
        // used to leave the prior plugin's native_companions entry still
        // listing the prompt path. The fix mirrors the native install
        // path's `strip_transferred_paths_from_other_plugins` call so the
        // prior owner's tracking truthfully reflects what's on disk.
        let (_dir, project) = temp_project();
        let src_tmp = tempfile::tempdir().unwrap();

        // Plugin A installs agent `shared`.
        let src_a = write_agent(src_tmp.path(), "shared", "---\nname: shared\n---\nfrom A\n");
        let (def_a, mapped_a) = parse_and_map(&src_a);
        let mut meta_a = sample_agent_meta();
        meta_a.plugin = pn("plugin-a");
        project
            .install_agent(&def_a, &mapped_a, meta_a, None)
            .expect("plugin-a install");

        // Plugin A owns prompts/shared.md.
        let installed_after_a = project.load_installed_agents().unwrap();
        assert!(
            installed_after_a
                .native_companions
                .get("plugin-a")
                .expect("plugin-a companion entry")
                .files
                .contains(&PathBuf::from("prompts/shared.md"))
        );

        // Plugin B force-installs an agent at the same name + prompt path.
        let src_b = src_tmp.path().join("shared_b.md");
        fs::write(&src_b, "---\nname: shared\n---\nfrom B\n").unwrap();
        let (def_b, mapped_b) = parse_and_map(&src_b);
        let mut meta_b = sample_agent_meta();
        meta_b.plugin = pn("plugin-b");
        project
            .install_agent_force(&def_b, &mapped_b, meta_b, None)
            .expect("plugin-b force install");

        // Ownership has transferred. Plugin A's companion entry must no
        // longer list prompts/shared.md (the file plugin B just took
        // over); plugin B owns it now.
        let installed_after_b = project.load_installed_agents().unwrap();
        assert!(
            installed_after_b
                .native_companions
                .get("plugin-a")
                .is_none_or(|m| !m.files.contains(&PathBuf::from("prompts/shared.md"))),
            "plugin-a must not still claim prompts/shared.md after transfer; native_companions: {:?}",
            installed_after_b.native_companions
        );
        assert!(
            installed_after_b
                .native_companions
                .get("plugin-b")
                .expect("plugin-b companion entry")
                .files
                .contains(&PathBuf::from("prompts/shared.md")),
            "plugin-b must claim prompts/shared.md"
        );

        // The agent itself reflects the new owner.
        assert_eq!(
            installed_after_b
                .agents
                .get("shared")
                .expect("agent tracked")
                .plugin,
            "plugin-b"
        );
    }

    #[test]
    fn install_native_companions_force_transfer_partial_overlap_recomputes_prior_hash() {
        // Closes silent-failure-hunter #1 + pr-test-analyzer C2.
        //
        // Scenario: plugin-a owns [keep.md, transfer.md]; plugin-b
        // force-installs at transfer.md only. Plugin-a's entry must
        // SURVIVE with [keep.md] + a recomputed hash that matches the
        // current bytes of keep.md, NOT the stale hash over the
        // original [keep.md, transfer.md] pair.
        let (_dir, project) = temp_project();

        // Plugin-a stages 2 files.
        let scratch_a = tempfile::tempdir().unwrap();
        let scan_a = scratch_a.path().join("src");
        fs::create_dir_all(scan_a.join("prompts")).unwrap();
        fs::write(scan_a.join("prompts/keep.md"), b"keep body").unwrap();
        fs::write(scan_a.join("prompts/transfer.md"), b"a-transfer").unwrap();
        let rel_paths_a = vec![
            PathBuf::from("prompts/keep.md"),
            PathBuf::from("prompts/transfer.md"),
        ];
        let h_a = crate::hash::hash_artifact(&scan_a, &rel_paths_a).unwrap();
        let mp_m = mp("m");
        let pn_a = pn("plugin-a");
        project
            .install_native_companions(&NativeCompanionsInput {
                scan_root: &scan_a,
                rel_paths: &rel_paths_a,
                marketplace: &mp_m,
                plugin: &pn_a,
                version: None,
                source_hash: &h_a,
                mode: crate::service::InstallMode::New,
                plugin_dir: scratch_a.path(),
            })
            .expect("plugin-a install");

        let stale_a_hash = project
            .load_installed_agents()
            .unwrap()
            .native_companions
            .get("plugin-a")
            .unwrap()
            .installed_hash
            .clone();

        // Plugin-b takes only transfer.md with different bytes.
        let scratch_b = tempfile::tempdir().unwrap();
        let scan_b = scratch_b.path().join("src");
        fs::create_dir_all(scan_b.join("prompts")).unwrap();
        fs::write(scan_b.join("prompts/transfer.md"), b"b-transfer").unwrap();
        let rel_paths_b = vec![PathBuf::from("prompts/transfer.md")];
        let h_b = crate::hash::hash_artifact(&scan_b, &rel_paths_b).unwrap();
        let pn_b = pn("plugin-b");
        project
            .install_native_companions(&NativeCompanionsInput {
                scan_root: &scan_b,
                rel_paths: &rel_paths_b,
                marketplace: &mp_m,
                plugin: &pn_b,
                version: None,
                source_hash: &h_b,
                mode: crate::service::InstallMode::Force,
                plugin_dir: scratch_b.path(),
            })
            .expect("plugin-b force install");

        let after = project.load_installed_agents().unwrap();

        // Plugin-a's entry survived with keep.md only.
        let a_entry = after
            .native_companions
            .get("plugin-a")
            .expect("plugin-a entry must survive");
        assert_eq!(a_entry.files, vec![PathBuf::from("prompts/keep.md")]);

        // Hashes must reflect the surviving file set, NOT the original
        // pair. The new hash is hash_artifact(agents_dir, [keep.md]).
        let agents_dir = project.root.join(".kiro/agents");
        let expected_a_hash =
            crate::hash::hash_artifact(&agents_dir, &[PathBuf::from("prompts/keep.md")]).unwrap();
        assert_eq!(
            a_entry.installed_hash, expected_a_hash,
            "installed_hash must be recomputed over surviving files"
        );
        assert_eq!(
            a_entry.source_hash, expected_a_hash,
            "source_hash must equal installed_hash post-transfer (canonical truth = current bytes on disk)"
        );
        assert_ne!(
            a_entry.installed_hash, stale_a_hash,
            "post-transfer hash must DIFFER from the original [keep.md, transfer.md] hash"
        );

        // Plugin-b owns transfer.md.
        let b_entry = after
            .native_companions
            .get("plugin-b")
            .expect("plugin-b entry exists");
        assert_eq!(b_entry.files, vec![PathBuf::from("prompts/transfer.md")]);

        // Files on disk reflect the new ownership.
        assert_eq!(
            fs::read(agents_dir.join("prompts/keep.md")).unwrap(),
            b"keep body",
            "plugin-a's keep.md untouched"
        );
        assert_eq!(
            fs::read(agents_dir.join("prompts/transfer.md")).unwrap(),
            b"b-transfer",
            "plugin-b's bytes won the transfer"
        );
    }

    #[test]
    fn remove_skill_deletes_directory_and_tracking() {
        let (_dir, project) = temp_project();
        let src = tempfile::tempdir().expect("tempdir");
        fs::write(
            src.path().join("SKILL.md"),
            "---\nname: removable\ndescription: Goes away\n---\n",
        )
        .expect("write");

        project
            .install_skill_from_dir("removable", src.path(), sample_meta())
            .expect("install");

        project
            .remove_skill("removable")
            .expect("remove should succeed");

        assert!(
            !project.skill_dir("removable").exists(),
            "skill directory should be gone"
        );

        let installed = project.load_installed().expect("load");
        assert!(
            !installed.skills.contains_key("removable"),
            "tracking entry should be removed"
        );
    }

    #[test]
    fn remove_nonexistent_skill_returns_error() {
        let (_dir, project) = temp_project();

        let err = project.remove_skill("nope").expect_err("should fail");

        let msg = err.to_string();
        assert!(
            msg.contains("not installed"),
            "expected 'not installed', got: {msg}"
        );
    }

    #[test]
    fn load_installed_returns_default_when_no_file() {
        let (_dir, project) = temp_project();

        let installed = project.load_installed().expect("load should succeed");
        assert!(installed.skills.is_empty());
    }

    #[test]
    fn remove_skill_rejects_path_traversal() {
        let (_dir, project) = temp_project();

        let err = project
            .remove_skill("../escape")
            .expect_err("should reject path traversal");
        let msg = err.to_string();
        assert!(
            msg.contains("invalid name"),
            "expected 'invalid name', got: {msg}"
        );
    }

    #[test]
    fn remove_skill_drops_tracking_on_orphan_dir() {
        // I3: when the on-disk skill dir is missing but a tracking
        // entry exists, `remove_skill` must drop the tracking row and
        // return `Ok(())`. Closes A-24 — the cascade's A-12 path no
        // longer creates persistent orphans because the per-method
        // contract now drives to absence.
        use chrono::Utc;
        let (_dir, project) = temp_project();
        std::fs::create_dir_all(project.kiro_dir()).expect("kiro dir");

        let now = Utc::now();
        std::fs::write(
            project.tracking_path(),
            serde_json::to_vec_pretty(&serde_json::json!({
                "skills": {
                    "orphan": {
                        "marketplace": "mp", "plugin": "p",
                        "version": "1.0.0", "installed_at": now,
                        "source_hash": "deadbeef",
                        "installed_hash": "deadbeef",
                        "source_scan_root": "skills",
                    }
                }
            }))
            .expect("ser"),
        )
        .expect("skills tracking");
        // Note: NO skills/orphan dir on disk.

        project
            .remove_skill("orphan")
            .expect("orphan removal must succeed");

        let installed = project.load_installed().expect("reload");
        assert!(
            installed.skills.is_empty(),
            "I3: orphan tracking row must be dropped on remove_skill"
        );
    }

    // -- remove_steering_file ---------------------------------------------

    #[test]
    fn remove_steering_file_unlinks_and_updates_tracking() {
        use chrono::Utc;
        let (_dir, project) = temp_project();
        std::fs::create_dir_all(project.kiro_dir().join("steering")).expect("dirs");

        let now = Utc::now();
        std::fs::write(
            project.kiro_dir().join("installed-steering.json"),
            serde_json::to_vec_pretty(&serde_json::json!({
                "files": {
                    "guide.md": {
                        "marketplace": "mp",
                        "plugin": "p",
                        "version": "1.0.0",
                        "installed_at": now,
                        "source_hash": "feedface",
                        "installed_hash": "feedface",
                        "source_scan_root": "steering",
                    }
                }
            }))
            .expect("ser"),
        )
        .expect("write tracking");
        std::fs::write(project.kiro_dir().join("steering/guide.md"), "# guide\n")
            .expect("write steering file");

        project
            .remove_steering_file(Path::new("guide.md"))
            .expect("remove ok");

        assert!(
            !project.kiro_dir().join("steering/guide.md").exists(),
            "on-disk steering file must be unlinked"
        );
        let tracking = project.load_installed_steering().expect("load post-remove");
        assert!(
            tracking.files.is_empty(),
            "tracking entry must be gone after remove"
        );
    }

    #[test]
    fn remove_steering_file_returns_not_installed_for_unknown_rel() {
        let (_dir, project) = temp_project();
        std::fs::create_dir_all(project.kiro_dir()).expect("kiro dir");

        let err = project
            .remove_steering_file(Path::new("absent.md"))
            .expect_err("expected NotInstalled");
        assert!(
            matches!(
                err,
                crate::error::Error::Steering(crate::steering::SteeringError::NotInstalled { .. })
            ),
            "expected SteeringError::NotInstalled, got {err:?}"
        );
    }

    #[test]
    fn remove_steering_file_succeeds_when_on_disk_file_missing() {
        // A-12 orphan-tracking case at the per-method granularity:
        // tracking entry exists but on-disk file was hand-deleted.
        // remove_steering_file must drop the tracking entry and return
        // Ok(), letting the cascade count it as removed.
        use chrono::Utc;
        let (_dir, project) = temp_project();
        std::fs::create_dir_all(project.kiro_dir()).expect("kiro dir");

        let now = Utc::now();
        std::fs::write(
            project.kiro_dir().join("installed-steering.json"),
            serde_json::to_vec_pretty(&serde_json::json!({
                "files": {
                    "orphan.md": {
                        "marketplace": "mp",
                        "plugin": "p",
                        "version": "1.0.0",
                        "installed_at": now,
                        "source_hash": "x",
                        "installed_hash": "x",
                        "source_scan_root": "steering",
                    }
                }
            }))
            .expect("ser"),
        )
        .expect("write tracking");
        // Note: NO steering/orphan.md on disk.

        project
            .remove_steering_file(Path::new("orphan.md"))
            .expect("orphan recovery: must NOT abort");

        let tracking = project.load_installed_steering().expect("load post-remove");
        assert!(
            tracking.files.is_empty(),
            "tracking entry must be gone even when on-disk file was orphan"
        );
    }

    #[test]
    fn remove_steering_file_restores_tracking_on_unlink_failure() {
        // I4: when fs::remove_file fails with a non-NotFound error,
        // the tracking entry must be restored so the file system
        // and tracking stay consistent. Trick: stage a directory at
        // the destination path, so `remove_file` returns EISDIR
        // (kind Other / IsADirectory) — not NotFound — which the
        // restore branch handles.
        use chrono::Utc;
        let (_dir, project) = temp_project();
        std::fs::create_dir_all(project.kiro_dir().join("steering/guide.md"))
            .expect("stage directory at destination so unlink fails with EISDIR");

        let now = Utc::now();
        std::fs::write(
            project.kiro_dir().join("installed-steering.json"),
            serde_json::to_vec_pretty(&serde_json::json!({
                "files": {
                    "guide.md": {
                        "marketplace": "mp",
                        "plugin": "p",
                        "version": "1.0.0",
                        "installed_at": now,
                        "source_hash": "x",
                        "installed_hash": "x",
                        "source_scan_root": "steering",
                    }
                }
            }))
            .expect("ser"),
        )
        .expect("write tracking");

        let err = project
            .remove_steering_file(Path::new("guide.md"))
            .expect_err("EISDIR must surface as Err");
        assert!(
            matches!(err, crate::error::Error::Io(_)),
            "expected Error::Io, got {err:?}"
        );

        let tracking = project
            .load_installed_steering()
            .expect("reload steering tracking");
        assert!(
            tracking.files.contains_key(Path::new("guide.md")),
            "I4: tracking entry must be restored after unlink failure"
        );
    }

    #[test]
    fn remove_steering_file_rejects_path_traversal_argument() {
        // Defense-in-depth: even though `load_installed_steering`
        // already rejects traversal entries, a future caller might
        // construct an `InstalledSteering` in memory and bypass the
        // load path. `remove_steering_file` must independently
        // refuse a traversal arg before joining onto `steering_dir`.
        let (_dir, project) = temp_project();
        std::fs::create_dir_all(project.kiro_dir()).expect("kiro dir");

        let err = project
            .remove_steering_file(Path::new("../../etc/passwd"))
            .expect_err("traversal arg must be refused");
        match err {
            crate::error::Error::Io(io_err) => {
                assert_eq!(io_err.kind(), std::io::ErrorKind::InvalidData);
                let msg = io_err.to_string();
                assert!(
                    msg.contains("../../etc/passwd"),
                    "error must name the offending path, got: {msg}"
                );
            }
            other => panic!("expected Error::Io(InvalidData), got {other:?}"),
        }
    }

    // -- remove_agent ------------------------------------------------------

    #[test]
    fn remove_agent_unlinks_files_and_updates_tracking() {
        use chrono::Utc;
        let (_dir, project) = temp_project();
        std::fs::create_dir_all(project.agent_prompts_dir()).expect("agents/prompts");

        let now = Utc::now();
        std::fs::write(
            project.agent_tracking_path(),
            serde_json::to_vec_pretty(&serde_json::json!({
                "agents": {
                    "reviewer": {
                        "marketplace": "mp",
                        "plugin": "p",
                        "version": "1.0.0",
                        "installed_at": now,
                        "dialect": "native",
                        "source_path": "agents/reviewer.json",
                        "source_hash": "deadbeef",
                        "installed_hash": "deadbeef",
                    }
                }
            }))
            .expect("ser"),
        )
        .expect("write tracking");
        std::fs::write(project.agents_dir().join("reviewer.json"), b"{}\n").expect("write json");
        std::fs::write(
            project.agent_prompts_dir().join("reviewer.md"),
            "# reviewer\n",
        )
        .expect("write prompt");

        project.remove_agent("reviewer").expect("remove ok");

        assert!(
            !project.agents_dir().join("reviewer.json").exists(),
            "agent JSON must be unlinked"
        );
        assert!(
            !project.agent_prompts_dir().join("reviewer.md").exists(),
            "agent prompt must be unlinked"
        );
        let tracking = project.load_installed_agents().expect("load");
        assert!(
            !tracking.agents.contains_key("reviewer"),
            "tracking entry must be gone after remove"
        );
    }

    #[test]
    fn remove_agent_returns_not_installed_for_unknown_name() {
        let (_dir, project) = temp_project();
        std::fs::create_dir_all(project.kiro_dir()).expect("kiro dir");

        let err = project
            .remove_agent("missing")
            .expect_err("expected NotInstalled");
        assert!(
            matches!(
                err,
                crate::error::Error::Agent(AgentError::NotInstalled { .. })
            ),
            "expected AgentError::NotInstalled, got {err:?}"
        );
    }

    #[test]
    fn remove_agent_succeeds_when_on_disk_files_missing() {
        // A-12 at per-method granularity for agents.
        use chrono::Utc;
        let (_dir, project) = temp_project();
        std::fs::create_dir_all(project.kiro_dir()).expect("kiro dir");

        let now = Utc::now();
        std::fs::write(
            project.agent_tracking_path(),
            serde_json::to_vec_pretty(&serde_json::json!({
                "agents": {
                    "ghost": {
                        "marketplace": "mp",
                        "plugin": "p",
                        "version": null,
                        "installed_at": now,
                        "dialect": "native",
                        "source_path": "agents/ghost.json",
                        "source_hash": "x",
                        "installed_hash": "x",
                    }
                }
            }))
            .expect("ser"),
        )
        .expect("write tracking");
        // Note: NO ghost.json or prompts/ghost.md on disk.

        project
            .remove_agent("ghost")
            .expect("orphan recovery: must NOT abort");

        let tracking = project.load_installed_agents().expect("load");
        assert!(
            !tracking.agents.contains_key("ghost"),
            "tracking entry must be gone even when on-disk files were orphan"
        );
    }

    #[test]
    fn remove_agent_restores_tracking_on_unlink_failure() {
        // I4: stage a directory at the agent JSON destination so
        // `fs::remove_file` returns EISDIR; the tracking row must be
        // restored. This exercises the half-success case (one file
        // unlinks, the other fails) by relying on the JSON path
        // being unlinkable as a directory.
        use chrono::Utc;
        let (_dir, project) = temp_project();
        // Create the parent so we can stage a directory at the JSON path.
        std::fs::create_dir_all(project.agents_dir().join("reviewer.json"))
            .expect("stage directory at agent JSON path so unlink fails with EISDIR");
        std::fs::create_dir_all(project.agent_prompts_dir()).expect("agents/prompts dir");
        // Empty prompt file so the second unlink would succeed —
        // the failure must come from the directory at reviewer.json.
        std::fs::write(
            project.agent_prompts_dir().join("reviewer.md"),
            "# reviewer\n",
        )
        .expect("write prompt");

        let now = Utc::now();
        std::fs::write(
            project.agent_tracking_path(),
            serde_json::to_vec_pretty(&serde_json::json!({
                "agents": {
                    "reviewer": {
                        "marketplace": "mp",
                        "plugin": "p",
                        "version": "1.0.0",
                        "installed_at": now,
                        "dialect": "native",
                        "source_path": "agents/reviewer.json",
                        "source_hash": "deadbeef",
                        "installed_hash": "deadbeef",
                    }
                }
            }))
            .expect("ser"),
        )
        .expect("write tracking");

        let err = project
            .remove_agent("reviewer")
            .expect_err("EISDIR must surface as Err");
        assert!(
            matches!(err, crate::error::Error::Io(_)),
            "expected Error::Io, got {err:?}"
        );

        let tracking = project
            .load_installed_agents()
            .expect("reload agent tracking");
        assert!(
            tracking.agents.contains_key("reviewer"),
            "I4: tracking entry must be restored after unlink failure"
        );
    }

    // -- remove_native_companions_for_plugin -------------------------------

    #[test]
    fn remove_native_companions_for_plugin_only_removes_matching_marketplace() {
        // A-16: companions map is keyed by plugin name alone, but
        // marketplace lives on the value. Removing mp-b's
        // "code-reviewer" must NOT touch mp-a's record.
        use chrono::Utc;
        let (_dir, project) = temp_project();
        std::fs::create_dir_all(project.kiro_dir()).expect("kiro dir");

        let now = Utc::now();
        std::fs::write(
            project.agent_tracking_path(),
            serde_json::to_vec_pretty(&serde_json::json!({
                "agents": {},
                "native_companions": {
                    "code-reviewer": {
                        "marketplace": "mp-a",
                        "plugin": "code-reviewer",
                        "version": null,
                        "installed_at": now,
                        "files": [],
                        "source_hash": "x",
                        "installed_hash": "x",
                        "source_scan_root": "agents",
                    }
                }
            }))
            .expect("ser"),
        )
        .expect("write tracking");

        // mp-b's "code-reviewer" entry doesn't exist — no-op.
        project
            .remove_native_companions_for_plugin(&pn("code-reviewer"), &mp("mp-b"))
            .expect("remove ok (no-op)");

        let tracking = project.load_installed_agents().expect("load");
        assert!(
            tracking.native_companions.contains_key("code-reviewer"),
            "mp-a's record must remain — A-16 marketplace disambiguation"
        );

        // mp-a's removal takes the entry out.
        project
            .remove_native_companions_for_plugin(&pn("code-reviewer"), &mp("mp-a"))
            .expect("remove ok");
        let tracking = project.load_installed_agents().expect("load");
        assert!(
            tracking.native_companions.is_empty(),
            "matching marketplace must remove the entry"
        );
    }

    #[test]
    fn remove_native_companions_for_plugin_unlinks_companion_files() {
        use chrono::Utc;
        let (_dir, project) = temp_project();
        std::fs::create_dir_all(project.agent_prompts_dir()).expect("agents/prompts");

        let now = Utc::now();
        std::fs::write(
            project.agent_tracking_path(),
            serde_json::to_vec_pretty(&serde_json::json!({
                "agents": {},
                "native_companions": {
                    "p": {
                        "marketplace": "mp",
                        "plugin": "p",
                        "version": null,
                        "installed_at": now,
                        "files": ["prompts/helper.md"],
                        "source_hash": "x",
                        "installed_hash": "x",
                        "source_scan_root": "agents",
                    }
                }
            }))
            .expect("ser"),
        )
        .expect("write tracking");
        let companion = project.agents_dir().join("prompts/helper.md");
        std::fs::write(&companion, b"# helper\n").expect("write companion file");

        project
            .remove_native_companions_for_plugin(&pn("p"), &mp("mp"))
            .expect("remove ok");

        assert!(
            !companion.exists(),
            "companion file must be unlinked when its plugin entry is removed"
        );
    }

    #[test]
    fn remove_native_companions_for_plugin_is_noop_when_entry_absent() {
        let (_dir, project) = temp_project();
        std::fs::create_dir_all(project.kiro_dir()).expect("kiro dir");

        // No tracking file at all → load returns default → no entry.
        project
            .remove_native_companions_for_plugin(&pn("p"), &mp("mp"))
            .expect("must be a no-op when no tracking exists");
    }

    // -- remove_plugin cascade --------------------------------------------

    #[test]
    fn remove_plugin_cascades_through_skills_steering_agents_tracking() {
        use chrono::Utc;
        let (_dir, project) = temp_project();
        std::fs::create_dir_all(project.kiro_dir().join("steering")).expect("dirs");
        std::fs::create_dir_all(project.skills_dir().join("alpha")).expect("skill dir");
        std::fs::write(
            project.skills_dir().join("alpha/SKILL.md"),
            "---\nname: alpha\ndescription: A\n---\n",
        )
        .expect("skill file");

        let now = Utc::now();
        std::fs::write(
            project.tracking_path(),
            serde_json::to_vec_pretty(&serde_json::json!({
                "skills": {
                    "alpha": {
                        "marketplace": "mp", "plugin": "p",
                        "version": "1.0.0", "installed_at": now,
                        "source_hash": "deadbeef",
                        "installed_hash": "deadbeef",
                        "source_scan_root": "skills",
                    }
                }
            }))
            .expect("ser"),
        )
        .expect("skills tracking");

        std::fs::write(
            project.kiro_dir().join("installed-steering.json"),
            serde_json::to_vec_pretty(&serde_json::json!({
                "files": {
                    "guide.md": {
                        "marketplace": "mp", "plugin": "p",
                        "version": "1.0.0", "installed_at": now,
                        "source_hash": "feedface", "installed_hash": "feedface",
                        "source_scan_root": "steering",
                    }
                }
            }))
            .expect("ser"),
        )
        .expect("steering tracking");
        std::fs::write(project.kiro_dir().join("steering/guide.md"), "# guide\n")
            .expect("steering file");

        let result = project
            .remove_plugin(&mp("mp"), &pn("p"))
            .expect("remove_plugin");
        assert_eq!(result.skills.removed, vec!["alpha"]);
        assert_eq!(result.steering.removed, vec!["guide.md"]);
        assert!(result.agents.removed.is_empty());
        assert!(result.skills.failures.is_empty());
        assert!(result.steering.failures.is_empty());
        assert!(result.agents.failures.is_empty());

        let post = project
            .installed_plugins()
            .expect("installed_plugins post-remove");
        assert!(
            post.plugins.iter().all(|p| p.plugin != "p"),
            "plugin p must be gone from the aggregated view"
        );
        assert!(
            !project.skills_dir().join("alpha").exists(),
            "skill dir must be unlinked"
        );
        assert!(
            !project.kiro_dir().join("steering/guide.md").exists(),
            "on-disk steering file must be unlinked"
        );
    }

    #[test]
    fn remove_plugin_only_removes_matching_marketplace_plugin_pair() {
        // Same plugin name across two marketplaces: removing mp-a/p
        // must NOT touch mp-b/p's entries.
        use chrono::Utc;
        let (_dir, project) = temp_project();
        std::fs::create_dir_all(project.kiro_dir().join("steering")).expect("dirs");

        let now = Utc::now();
        std::fs::write(
            project.kiro_dir().join("installed-steering.json"),
            serde_json::to_vec_pretty(&serde_json::json!({
                "files": {
                    "a.md": {
                        "marketplace": "mp-a", "plugin": "p",
                        "version": "1.0.0", "installed_at": now,
                        "source_hash": "1", "installed_hash": "1",
                        "source_scan_root": "steering",
                    },
                    "b.md": {
                        "marketplace": "mp-b", "plugin": "p",
                        "version": "1.0.0", "installed_at": now,
                        "source_hash": "2", "installed_hash": "2",
                        "source_scan_root": "steering",
                    }
                }
            }))
            .expect("ser"),
        )
        .expect("steering tracking");
        std::fs::write(project.kiro_dir().join("steering/a.md"), "# a\n").expect("a");
        std::fs::write(project.kiro_dir().join("steering/b.md"), "# b\n").expect("b");

        let result = project
            .remove_plugin(&mp("mp-a"), &pn("p"))
            .expect("remove mp-a/p");
        assert_eq!(result.steering.removed, vec!["a.md"], "only mp-a/p's entry");
        assert!(result.skills.removed.is_empty());
        assert!(result.agents.removed.is_empty());
        assert!(result.skills.failures.is_empty());
        assert!(result.steering.failures.is_empty());
        assert!(result.agents.failures.is_empty());

        let tracking = project.load_installed_steering().expect("load");
        assert!(
            tracking.files.contains_key(Path::new("b.md")),
            "mp-b/p's entry must remain"
        );
        assert!(
            project.kiro_dir().join("steering/b.md").exists(),
            "mp-b/p's on-disk file must remain"
        );
        assert!(
            !project.kiro_dir().join("steering/a.md").exists(),
            "mp-a/p's on-disk file must be unlinked"
        );
    }

    #[test]
    fn remove_plugin_recovers_from_orphan_skill_tracking_entry() {
        // A-12 + I3 regression at the cascade level: tracking entry
        // exists but on-disk dir doesn't. The cascade must NOT abort
        // AND the tracking row must be dropped so the plugin doesn't
        // resurrect on the next `installed_plugins()` call.
        use chrono::Utc;
        let (_dir, project) = temp_project();
        std::fs::create_dir_all(project.kiro_dir()).expect("kiro dir");

        let now = Utc::now();
        std::fs::write(
            project.tracking_path(),
            serde_json::to_vec_pretty(&serde_json::json!({
                "skills": {
                    "orphan": {
                        "marketplace": "mp", "plugin": "p",
                        "version": "1.0.0", "installed_at": now,
                        "source_hash": "deadbeef",
                        "installed_hash": "deadbeef",
                        "source_scan_root": "skills",
                    }
                }
            }))
            .expect("ser"),
        )
        .expect("skills tracking");
        // Note: NO skills/orphan dir on disk.

        let result = project
            .remove_plugin(&mp("mp"), &pn("p"))
            .expect("orphan recovery: must NOT abort");
        assert_eq!(
            result.skills.removed,
            vec!["orphan"],
            "orphan tracking entry counts as removed (A-12)"
        );
        assert!(
            result.skills.failures.is_empty(),
            "I3: orphan recovery is no longer a `failed` entry; remove_skill \
             drops the tracking row and treats missing on-disk dir as success"
        );
        assert!(result.steering.removed.is_empty());
        assert!(result.agents.removed.is_empty());
        assert!(result.steering.failures.is_empty());
        assert!(result.agents.failures.is_empty());
        // I3: remove_skill now drops the tracking row on the orphan
        // path, so installed_plugins() must NOT surface this plugin
        // anymore. Closes A-24.
        let installed = project.load_installed().expect("reload skills tracking");
        assert!(
            installed.skills.is_empty(),
            "I3: orphan tracking row must be dropped post-cascade"
        );
        let plugins = project.installed_plugins().expect("installed_plugins");
        assert!(
            plugins.plugins.is_empty(),
            "I3: cleared tracking means `installed_plugins()` reports nothing — \
             plugin no longer resurrects after a `Remove` click"
        );
    }

    #[test]
    fn remove_plugin_partial_failure_collects_failed_and_keeps_succeeded() {
        // I5: stage one removable skill plus a steering tracking row
        // whose unlink fails (directory at destination). The cascade
        // must:
        //   - count the skill as removed
        //   - record the steering failure in `result.steering.failures`
        //     (NOT short-circuit)
        //   - still return Ok(result)
        use chrono::Utc;
        let (_dir, project) = temp_project();
        std::fs::create_dir_all(project.kiro_dir()).expect("kiro dir");

        // Removable skill: install a real one so remove_skill drives
        // to absence cleanly.
        let src = tempfile::tempdir().expect("skill tempdir");
        std::fs::write(
            src.path().join("SKILL.md"),
            "---\nname: removable\ndescription: Goes away\n---\n",
        )
        .expect("write skill");
        project
            .install_skill_from_dir(
                "removable",
                src.path(),
                InstalledSkillMeta {
                    marketplace: mp("mp"),
                    plugin: pn("p"),
                    version: Some("1.0.0".into()),
                    installed_at: Utc::now(),
                    source_hash: "deadbeef".into(),
                    installed_hash: "deadbeef".into(),
                    source_scan_root: RelativePath::new("skills").expect("valid"),
                },
            )
            .expect("install skill");

        // Stage steering tracking entry whose dest is a directory →
        // remove_file returns EISDIR → I5 captures the failure.
        std::fs::create_dir_all(project.kiro_dir().join("steering/guide.md"))
            .expect("stage directory at steering destination");
        let now = Utc::now();
        std::fs::write(
            project.kiro_dir().join("installed-steering.json"),
            serde_json::to_vec_pretty(&serde_json::json!({
                "files": {
                    "guide.md": {
                        "marketplace": "mp",
                        "plugin": "p",
                        "version": "1.0.0",
                        "installed_at": now,
                        "source_hash": "x",
                        "installed_hash": "x",
                        "source_scan_root": "steering",
                    }
                }
            }))
            .expect("ser"),
        )
        .expect("write steering tracking");

        let result = project
            .remove_plugin(&mp("mp"), &pn("p"))
            .expect("I5: cascade returns Ok even with per-step failures");

        assert_eq!(
            result.skills.removed,
            vec!["removable"],
            "I5: skill removal succeeded and counted"
        );
        assert!(
            result.steering.removed.is_empty(),
            "I5: steering removal failed — must NOT appear in removed"
        );
        assert_eq!(
            result.steering.failures.len(),
            1,
            "I5: exactly one steering failure expected, got {:?}",
            result.steering.failures
        );
        assert_eq!(
            result.steering.failures[0].item, "guide.md",
            "I5: item must identify which entry errored"
        );
        assert!(
            !result.steering.failures[0].error.is_empty(),
            "I5: error string must be populated via error_full_chain"
        );
        assert!(result.skills.failures.is_empty());
        assert!(result.agents.removed.is_empty());
        assert!(result.agents.failures.is_empty());
    }

    #[test]
    fn remove_plugin_result_json_shape_locks_default_empty() {
        let result = RemovePluginResult::default();
        let json = serde_json::to_value(&result).expect("serialize");
        assert!(json["skills"].is_object());
        assert!(json["steering"].is_object());
        assert!(json["agents"].is_object());
        assert_eq!(json["skills"]["removed"], serde_json::json!([]));
        assert_eq!(json["skills"]["failures"], serde_json::json!([]));
    }

    #[test]
    fn remove_plugin_result_json_shape_with_populated_removed_and_failures() {
        let result = RemovePluginResult {
            skills: RemoveSkillsResult {
                removed: vec!["alpha".into()],
                failures: vec![],
            },
            steering: RemoveSteeringResult {
                removed: vec![],
                failures: vec![RemoveItemFailure {
                    item: "broken.md".into(),
                    error: "io: permission denied".into(),
                }],
            },
            agents: RemoveAgentsResult::default(),
        };
        let json = serde_json::to_value(&result).expect("serialize");
        assert_eq!(json["skills"]["removed"][0], "alpha");
        assert_eq!(json["steering"]["failures"][0]["item"], "broken.md");
        assert_eq!(json["agents"]["removed"], serde_json::json!([]));
    }

    #[test]
    fn remove_plugin_propagates_load_time_path_traversal_error() {
        // A-4 regression: the cascade reads tracking files via
        // load_installed_steering, which validates path entries.
        // A traversal entry must surface as a load-time Err and the
        // cascade must propagate, not silently skip.
        use chrono::Utc;
        let (_dir, project) = temp_project();
        std::fs::create_dir_all(project.kiro_dir()).expect("kiro dir");

        let now = Utc::now();
        std::fs::write(
            project.kiro_dir().join("installed-steering.json"),
            serde_json::to_vec_pretty(&serde_json::json!({
                "files": {
                    "../../etc/passwd": {
                        "marketplace": "mp", "plugin": "p",
                        "version": "1.0.0", "installed_at": now,
                        "source_hash": "x", "installed_hash": "x",
                        "source_scan_root": "steering",
                    }
                }
            }))
            .expect("ser"),
        )
        .expect("steering tracking");

        let err = project
            .remove_plugin(&mp("mp"), &pn("p"))
            .expect_err("traversal entry must surface as Err");
        match err {
            crate::error::Error::Io(io_err) => {
                assert_eq!(io_err.kind(), std::io::ErrorKind::InvalidData);
            }
            other => {
                panic!("expected Error::Io(InvalidData) from load-time validator, got {other:?}")
            }
        }
    }

    #[test]
    fn remove_plugin_drops_native_companions_entry_for_matching_marketplace() {
        // A-3 + A-16: cascade must call
        // remove_native_companions_for_plugin with the correct
        // marketplace, and only matching entries get dropped.
        use chrono::Utc;
        let (_dir, project) = temp_project();
        std::fs::create_dir_all(project.kiro_dir()).expect("kiro dir");

        let now = Utc::now();
        std::fs::write(
            project.agent_tracking_path(),
            serde_json::to_vec_pretty(&serde_json::json!({
                "agents": {},
                "native_companions": {
                    "p": {
                        "marketplace": "mp",
                        "plugin": "p",
                        "version": null,
                        "installed_at": now,
                        "files": [],
                        "source_hash": "x",
                        "installed_hash": "x",
                        "source_scan_root": "agents",
                    }
                }
            }))
            .expect("ser"),
        )
        .expect("agents tracking");

        let result = project
            .remove_plugin(&mp("mp"), &pn("p"))
            .expect("remove_plugin");

        // P2a-3 sub-decision α: agents.removed does NOT itemize companion
        // files; the step succeeds with an empty removed vec.
        assert!(
            result.agents.removed.is_empty(),
            "native_companions success yields no per-file entries in agents.removed"
        );
        assert!(
            result.agents.failures.is_empty(),
            "native_companions cleanup must not fail for matching marketplace"
        );

        let tracking = project.load_installed_agents().expect("load");
        assert!(
            tracking.native_companions.is_empty(),
            "native_companions entry must be dropped for matching marketplace"
        );
    }

    #[test]
    fn load_installed_returns_installed_skills() {
        let (_dir, project) = temp_project();
        let src = tempfile::tempdir().expect("tempdir");
        fs::write(
            src.path().join("SKILL.md"),
            "---\nname: listed\ndescription: Listed\n---\n",
        )
        .expect("write");

        project
            .install_skill_from_dir("listed", src.path(), sample_meta())
            .expect("install");

        let installed = project.load_installed().expect("load");
        assert!(installed.skills.contains_key("listed"));
    }

    #[test]
    fn tracking_file_contains_valid_json_after_install() {
        let (_dir, project) = temp_project();
        let src = tempfile::tempdir().expect("tempdir");
        fs::write(
            src.path().join("SKILL.md"),
            "---\nname: atomic-check\ndescription: Checks atomic\n---\n",
        )
        .expect("write");

        project
            .install_skill_from_dir("atomic-check", src.path(), sample_meta())
            .expect("install");

        let raw = fs::read(project.tracking_path()).expect("read tracking file");
        let parsed: InstalledSkills =
            serde_json::from_slice(&raw).expect("tracking file should be valid JSON");
        assert!(parsed.skills.contains_key("atomic-check"));

        assert!(
            !project.tracking_path().with_extension("tmp").exists(),
            ".tmp file should be gone after atomic rename"
        );
    }

    // -----------------------------------------------------------------------
    // install_skill_from_dir
    // -----------------------------------------------------------------------

    #[test]
    fn install_skill_from_dir_copies_skill_and_references() {
        let (_dir, project) = temp_project();
        let src = tempfile::tempdir().expect("tempdir");

        fs::write(
            src.path().join("SKILL.md"),
            "---\nname: with-refs\ndescription: Has references\n---\nSee `references/api.md`.\n",
        )
        .expect("write");
        fs::create_dir_all(src.path().join("references")).expect("mkdir");
        fs::write(
            src.path().join("references").join("api.md"),
            "# API Reference\nDetails here.",
        )
        .expect("write");

        project
            .install_skill_from_dir("with-refs", src.path(), sample_meta())
            .expect("install should succeed");

        let skill_md = project.skill_dir("with-refs").join("SKILL.md");
        let content = fs::read_to_string(&skill_md).expect("read");
        assert!(content.contains("See `references/api.md`."));

        let ref_file = project
            .skill_dir("with-refs")
            .join("references")
            .join("api.md");
        assert!(ref_file.exists(), "reference file should be copied");
        let ref_content = fs::read_to_string(&ref_file).expect("read");
        assert_eq!(ref_content, "# API Reference\nDetails here.");

        let installed = project.load_installed().expect("load");
        assert!(installed.skills.contains_key("with-refs"));

        // No temp dir should remain.
        let skills_dir = project.skills_dir();
        let leftover: Vec<_> = fs::read_dir(&skills_dir)
            .expect("read skills dir")
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().starts_with("_installing-"))
            .collect();
        assert!(leftover.is_empty(), "temp dir should be cleaned up");
    }

    #[test]
    fn install_skill_from_dir_rejects_duplicate() {
        let (_dir, project) = temp_project();
        let src = tempfile::tempdir().expect("tempdir");
        fs::write(
            src.path().join("SKILL.md"),
            "---\nname: dup\ndescription: Dup\n---\n",
        )
        .expect("write");

        project
            .install_skill_from_dir("dup", src.path(), sample_meta())
            .expect("first install");

        let err = project
            .install_skill_from_dir("dup", src.path(), sample_meta())
            .expect_err("second install should fail");
        assert!(err.to_string().contains("already installed"));
    }

    #[test]
    fn install_skill_from_dir_force_overwrites() {
        let (_dir, project) = temp_project();
        let src1 = tempfile::tempdir().expect("tempdir");
        let src2 = tempfile::tempdir().expect("tempdir");

        fs::write(
            src1.path().join("SKILL.md"),
            "---\nname: s\ndescription: v1\n---\nOriginal.\n",
        )
        .expect("write");
        fs::write(
            src2.path().join("SKILL.md"),
            "---\nname: s\ndescription: v2\n---\nUpdated.\n",
        )
        .expect("write");
        fs::create_dir_all(src2.path().join("references")).expect("mkdir");
        fs::write(src2.path().join("references").join("new.md"), "new ref").expect("write");

        project
            .install_skill_from_dir("s", src1.path(), sample_meta())
            .expect("first install");

        project
            .install_skill_from_dir_force("s", src2.path(), sample_meta())
            .expect("force install");

        let content = fs::read_to_string(project.skill_dir("s").join("SKILL.md")).expect("read");
        assert!(content.contains("Updated."));

        assert!(
            project
                .skill_dir("s")
                .join("references")
                .join("new.md")
                .exists()
        );
    }

    #[test]
    fn install_skill_from_dir_rejects_path_traversal() {
        let (_dir, project) = temp_project();
        let src = tempfile::tempdir().expect("tempdir");
        fs::write(
            src.path().join("SKILL.md"),
            "---\nname: evil\ndescription: Evil\n---\n",
        )
        .expect("write");

        let err = project
            .install_skill_from_dir("../escape", src.path(), sample_meta())
            .expect_err("should reject path traversal");
        let msg = err.to_string();
        assert!(
            msg.contains("invalid name"),
            "expected 'invalid name', got: {msg}"
        );
    }

    #[test]
    fn install_skill_from_dir_works_with_skill_only_no_references() {
        let (_dir, project) = temp_project();
        let src = tempfile::tempdir().expect("tempdir");
        fs::write(
            src.path().join("SKILL.md"),
            "---\nname: simple\ndescription: Simple\n---\nBody.\n",
        )
        .expect("write");

        project
            .install_skill_from_dir("simple", src.path(), sample_meta())
            .expect("install should succeed");

        let skill_md = project.skill_dir("simple").join("SKILL.md");
        assert!(skill_md.exists());
        assert!(!project.skill_dir("simple").join("references").exists());
    }

    #[test]
    fn install_skill_from_dir_force_rejects_path_traversal() {
        let (_dir, project) = temp_project();
        let src = tempfile::tempdir().expect("tempdir");
        fs::write(
            src.path().join("SKILL.md"),
            "---\nname: evil\ndescription: Evil\n---\n",
        )
        .expect("write");

        let err = project
            .install_skill_from_dir_force("../escape", src.path(), sample_meta())
            .expect_err("should reject path traversal");
        let msg = err.to_string();
        assert!(
            msg.contains("invalid name"),
            "expected 'invalid name', got: {msg}"
        );
    }

    #[test]
    fn install_skill_from_dir_force_removes_stale_files_from_old_version() {
        let (_dir, project) = temp_project();
        let src1 = tempfile::tempdir().expect("tempdir");
        let src2 = tempfile::tempdir().expect("tempdir");

        // v1: SKILL.md + references/old.md
        fs::write(
            src1.path().join("SKILL.md"),
            "---\nname: s\ndescription: v1\n---\n",
        )
        .expect("write");
        fs::create_dir_all(src1.path().join("references")).expect("mkdir");
        fs::write(src1.path().join("references").join("old.md"), "old ref").expect("write");

        project
            .install_skill_from_dir("s", src1.path(), sample_meta())
            .expect("first install");
        assert!(
            project
                .skill_dir("s")
                .join("references")
                .join("old.md")
                .exists()
        );

        // v2: SKILL.md only, no references/
        fs::write(
            src2.path().join("SKILL.md"),
            "---\nname: s\ndescription: v2\n---\n",
        )
        .expect("write");

        project
            .install_skill_from_dir_force("s", src2.path(), sample_meta())
            .expect("force install");

        // Old reference file should be gone — full directory replacement.
        assert!(
            !project.skill_dir("s").join("references").exists(),
            "stale references/ dir from v1 should be gone after force install"
        );
    }

    // -----------------------------------------------------------------------
    // copy_dir_recursive
    // -----------------------------------------------------------------------

    #[test]
    fn copy_dir_recursive_copies_nested_structure() {
        let src = tempfile::tempdir().expect("tempdir");
        let dest = tempfile::tempdir().expect("tempdir");
        let dest_path = dest.path().join("output");

        fs::write(src.path().join("SKILL.md"), "skill content").expect("write");
        fs::create_dir_all(src.path().join("references")).expect("mkdir");
        fs::write(
            src.path().join("references").join("guide.md"),
            "guide content",
        )
        .expect("write");

        copy_dir_recursive(src.path(), &dest_path).expect("copy should succeed");

        assert_eq!(
            fs::read_to_string(dest_path.join("SKILL.md")).expect("read"),
            "skill content"
        );
        assert_eq!(
            fs::read_to_string(dest_path.join("references").join("guide.md")).expect("read"),
            "guide content"
        );
    }

    #[test]
    fn copy_dir_recursive_handles_empty_directory() {
        let src = tempfile::tempdir().expect("tempdir");
        let dest = tempfile::tempdir().expect("tempdir");
        let dest_path = dest.path().join("output");

        fs::write(src.path().join("SKILL.md"), "just skill").expect("write");

        copy_dir_recursive(src.path(), &dest_path).expect("copy should succeed");

        assert_eq!(
            fs::read_to_string(dest_path.join("SKILL.md")).expect("read"),
            "just skill"
        );
    }

    #[test]
    fn copy_dir_recursive_errors_on_nonexistent_source() {
        let dest = tempfile::tempdir().expect("tempdir");
        let dest_path = dest.path().join("output");
        let fake_src = dest.path().join("does-not-exist");

        let err = copy_dir_recursive(&fake_src, &dest_path).expect_err("should fail");
        assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
    }

    #[cfg(unix)]
    #[test]
    fn copy_dir_recursive_skips_symlinks() {
        use std::os::unix::fs as unix_fs;

        let src = tempfile::tempdir().expect("tempdir");
        let dest = tempfile::tempdir().expect("tempdir");
        let dest_path = dest.path().join("output");

        fs::write(src.path().join("SKILL.md"), "skill content").expect("write");
        // Create a symlink that points to a sensitive file.
        unix_fs::symlink("/etc/passwd", src.path().join("evil-link")).expect("symlink");

        copy_dir_recursive(src.path(), &dest_path).expect("copy should succeed");

        // The regular file should be copied.
        assert!(dest_path.join("SKILL.md").exists());
        // The symlink should NOT be copied.
        assert!(
            !dest_path.join("evil-link").exists(),
            "symlinks should be skipped during copy"
        );
    }

    #[cfg(unix)]
    #[test]
    fn copy_dir_recursive_skips_hardlinks() {
        // A malicious LocalPath marketplace creates a hardlink inside the
        // skill source pointing at a sensitive file (here we use a
        // sentinel within the same temp tree to avoid touching real host
        // files, but the threat is `~/.ssh/id_rsa`-class). The copy must
        // skip the hardlink so the installed skill does not expose the
        // sensitive content.
        let src = tempfile::tempdir().expect("tempdir");
        let dest = tempfile::tempdir().expect("tempdir");
        let dest_path = dest.path().join("output");

        // Two regular files in the source.
        fs::write(src.path().join("SKILL.md"), "skill content").expect("write");

        // A "secret" file outside the skill dir.
        let secret_dir = tempfile::tempdir().expect("tempdir");
        let secret_path = secret_dir.path().join("secret.txt");
        fs::write(&secret_path, "TOP SECRET").expect("write secret");

        // Hardlink the secret into the skill dir as a benign-looking name.
        std::fs::hard_link(&secret_path, src.path().join("notes.md")).expect("hardlink");

        copy_dir_recursive(src.path(), &dest_path).expect("copy should succeed");

        // The regular file is copied as expected.
        assert!(dest_path.join("SKILL.md").exists());
        // The hardlink must NOT be copied — its content (the secret) must
        // never reach the install destination.
        assert!(
            !dest_path.join("notes.md").exists(),
            "hardlinked file must be skipped during copy"
        );
        // The original secret file is untouched.
        assert_eq!(
            fs::read_to_string(&secret_path).unwrap(),
            "TOP SECRET",
            "skipping must not delete or modify the source"
        );
    }

    #[test]
    fn install_skill_from_dir_serializes_concurrent_same_name_installs() {
        // Two threads racing to install the same skill name. Without the
        // file lock + existence-check-inside-lock, both could pass the
        // existence check and clobber each other's staging directories.
        let (_dir, project) = temp_project();
        let project = std::sync::Arc::new(project);

        let src = tempfile::tempdir().expect("tempdir");
        fs::write(
            src.path().join("SKILL.md"),
            "---\nname: racey\ndescription: Racey\n---\n",
        )
        .expect("write");
        let src_path = src.path().to_path_buf();

        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));

        let handles: Vec<_> = (0..2)
            .map(|_| {
                let project = std::sync::Arc::clone(&project);
                let barrier = std::sync::Arc::clone(&barrier);
                let src_path = src_path.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    project.install_skill_from_dir("racey", &src_path, sample_meta())
                })
            })
            .collect();

        let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();

        // Exactly one should succeed; the other should see AlreadyInstalled.
        let ok_count = results.iter().filter(|r| r.is_ok()).count();
        let already_count = results
            .iter()
            .filter(|r| {
                matches!(
                    r,
                    Err(crate::error::Error::Skill(
                        SkillError::AlreadyInstalled { .. }
                    ))
                )
            })
            .count();
        assert_eq!(ok_count, 1, "exactly one install should succeed");
        assert_eq!(already_count, 1, "the other should be AlreadyInstalled");

        // No leftover staging dirs from either attempt.
        let leftover: Vec<_> = fs::read_dir(project.skills_dir())
            .expect("read skills dir")
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().starts_with("_installing-"))
            .collect();
        assert!(
            leftover.is_empty(),
            "no staging dirs should remain after race: {leftover:?}"
        );

        // The skill should be installed and tracked exactly once.
        let installed = project.load_installed().expect("load");
        assert_eq!(installed.skills.len(), 1);
        assert!(installed.skills.contains_key("racey"));
    }

    // Deleted: `installed_skill_meta_loads_legacy_json_without_hash_fields` —
    // legacy JSON without `source_scan_root` now fails to deserialize
    // by design (install↔detect symmetry pass; no users → required
    // field). The inverse contract is pinned by the deserialize-rejection
    // tests added in Task 7 of the install-detect-symmetry plan.

    // Deleted: `installed_agent_meta_loads_legacy_json_without_hash_fields` —
    // legacy JSON without `source_path` now fails to deserialize by
    // design (install↔detect symmetry pass; no users → required field).
    // The inverse contract is pinned by the deserialize-rejection tests
    // added in Task 7 of the install-detect-symmetry plan.
    //
    // Deleted: `installed_agents_loads_legacy_json_without_native_companions` —
    // same reason. The native_companions-defaults-to-empty contract is
    // still useful (agents-only tracking files should still load), but
    // it requires source_path on the agent entries to be testable. The
    // round-trip test below covers the same surface with a current-shape
    // tracking entry.

    #[test]
    fn installed_native_companions_meta_round_trips_through_serde() {
        let meta = InstalledNativeCompanionsMeta {
            marketplace: mp("m"),
            plugin: pn("p"),
            version: Some("0.1.0".into()),
            installed_at: chrono::Utc::now(),
            files: vec![
                std::path::PathBuf::from("prompts/a.md"),
                std::path::PathBuf::from("prompts/b.md"),
            ],
            source_hash: "blake3:abc".into(),
            installed_hash: "blake3:abc".into(),
            source_scan_root: RelativePath::new("agents").expect("valid"),
        };
        let bytes = serde_json::to_vec(&meta).unwrap();
        let back: InstalledNativeCompanionsMeta = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(back.files.len(), 2);
        // PR #100 review I6: the round-trip used to assert only on
        // `files.len()`, which was satisfied long before
        // `source_scan_root` joined the schema. Locking the field
        // explicitly so a future regression that drops or re-types it
        // breaks here instead of silently round-tripping a
        // default/empty value.
        assert_eq!(back.source_scan_root.as_str(), "agents");
    }

    /// PR #100 review I1: `required_steering_scan_root` must surface a
    /// structural validation failure as `SteeringError::ScanRootInvalid`,
    /// not a synthetic `SourceReadFailed { source: io::Error }`. The
    /// real `ValidationError` propagates via `#[source]` so the
    /// `error_full_chain` projection used at FFI/log boundaries renders
    /// the precise cause instead of a fabricated I/O message.
    #[test]
    fn required_steering_scan_root_returns_scan_root_invalid_when_outside_plugin_dir() {
        let plugin_dir = std::path::PathBuf::from("/plugins/foo");
        let scan_root = std::path::PathBuf::from("/somewhere/else");

        let err = required_steering_scan_root(&scan_root, &plugin_dir)
            .expect_err("scan_root outside plugin_dir must fail");

        match err {
            crate::steering::SteeringError::ScanRootInvalid {
                ref path,
                plugin_dir: ref pd,
                ..
            } => {
                assert_eq!(path, &scan_root);
                assert_eq!(pd, &plugin_dir);
            }
            other => panic!("expected ScanRootInvalid, got {other:?}"),
        }
        // The variant carries the underlying ValidationError via
        // `#[source]` so chained renderers see "scan_root invalid → not
        // under base" instead of a fabricated InvalidInput io::Error.
        assert!(
            std::error::Error::source(&err).is_some(),
            "ScanRootInvalid must expose its ValidationError via Error::source"
        );
    }

    // ---------------------------------------------------------------------
    // Deserialize-rejection tests for the install↔detect symmetry pass.
    //
    // After the no-users assumption let us tighten source_path /
    // source_scan_root from Option<...> to required, legacy tracking
    // files written before this PR landed must fail to deserialize. The
    // contract is "intentionally invalid by design" — these tests pin
    // it so a future refactor that re-adds Option (re-introducing the
    // probe-fallback machinery) breaks here, not silently in production.
    // ---------------------------------------------------------------------

    #[test]
    fn load_installed_skills_rejects_legacy_entry_without_source_scan_root() {
        let (_dir, project) = temp_project();
        let kiro_dir = project.kiro_dir();
        std::fs::create_dir_all(&kiro_dir).expect("create .kiro");

        let legacy_json = br#"{
            "skills": {
                "alpha": {
                    "marketplace": "mp",
                    "plugin": "p",
                    "version": "1.0",
                    "installed_at": "2026-01-01T00:00:00Z"
                }
            }
        }"#;
        std::fs::write(kiro_dir.join("installed-skills.json"), legacy_json)
            .expect("write legacy tracking");

        let err = project
            .load_installed()
            .expect_err("legacy entry must fail to deserialize");
        let msg = err.to_string();
        assert!(
            msg.contains("source_scan_root") || msg.contains("missing field"),
            "error must mention the missing required field; got: {msg}"
        );
    }

    #[test]
    fn load_installed_steering_rejects_legacy_entry_without_source_scan_root() {
        let (_dir, project) = temp_project();
        let kiro_dir = project.kiro_dir();
        std::fs::create_dir_all(&kiro_dir).expect("create .kiro");

        let legacy_json = br#"{
            "files": {
                "guide.md": {
                    "marketplace": "mp",
                    "plugin": "p",
                    "version": "1.0",
                    "installed_at": "2026-01-01T00:00:00Z",
                    "source_hash": "blake3:0000",
                    "installed_hash": "blake3:0000"
                }
            }
        }"#;
        std::fs::write(kiro_dir.join("installed-steering.json"), legacy_json)
            .expect("write legacy tracking");

        let err = project
            .load_installed_steering()
            .expect_err("legacy entry must fail to deserialize");
        assert!(
            err.to_string().contains("source_scan_root"),
            "error must mention source_scan_root; got: {err}"
        );
    }

    #[test]
    fn load_installed_agents_rejects_legacy_entry_without_source_path() {
        let (_dir, project) = temp_project();
        let kiro_dir = project.kiro_dir();
        std::fs::create_dir_all(&kiro_dir).expect("create .kiro");

        let legacy_json = br#"{
            "agents": {
                "reviewer": {
                    "marketplace": "mp",
                    "plugin": "p",
                    "version": "1.0",
                    "installed_at": "2026-01-01T00:00:00Z",
                    "dialect": "native"
                }
            }
        }"#;
        std::fs::write(kiro_dir.join("installed-agents.json"), legacy_json)
            .expect("write legacy tracking");

        let err = project
            .load_installed_agents()
            .expect_err("legacy entry must fail to deserialize");
        let msg = err.to_string();
        assert!(
            msg.contains("source_path") || msg.contains("missing field"),
            "error must mention a missing required field; got: {msg}"
        );
    }

    /// PR #100 review I2: a tracking entry with `source_scan_root` but
    /// without `source_hash` / `installed_hash` was unreachable in
    /// practice (every install path that records `scan_root` also
    /// records hashes), but the deferred `Option<String>` field type
    /// kept the `legacy_fallback` escape hatch alive in detection.
    /// After I2 the fields are required `String` and a contradictory
    /// entry (`scan_root` present, hashes absent) fails to deserialize
    /// at the boundary.
    #[test]
    fn load_installed_skills_rejects_legacy_entry_without_source_hash() {
        let (_dir, project) = temp_project();
        let kiro_dir = project.kiro_dir();
        std::fs::create_dir_all(&kiro_dir).expect("create .kiro");

        let legacy_json = br#"{
            "skills": {
                "alpha": {
                    "marketplace": "mp",
                    "plugin": "p",
                    "version": "1.0",
                    "installed_at": "2026-01-01T00:00:00Z",
                    "source_scan_root": "skills"
                }
            }
        }"#;
        std::fs::write(kiro_dir.join("installed-skills.json"), legacy_json)
            .expect("write legacy tracking");

        let err = project
            .load_installed()
            .expect_err("legacy entry without source_hash must fail to deserialize");
        let msg = err.to_string();
        assert!(
            msg.contains("source_hash") || msg.contains("installed_hash"),
            "error must mention the missing required field; got: {msg}"
        );
    }

    /// PR #100 review I2 sibling for agents: a tracking entry that
    /// has `source_path` (the Stage-1+ symmetry-pass marker) but
    /// lacks `source_hash` / `installed_hash` is structurally
    /// contradictory and must be rejected at the deserialize
    /// boundary, not silently skipped at scan time.
    #[test]
    fn load_installed_agents_rejects_legacy_entry_without_source_hash() {
        let (_dir, project) = temp_project();
        let kiro_dir = project.kiro_dir();
        std::fs::create_dir_all(&kiro_dir).expect("create .kiro");

        let legacy_json = br#"{
            "agents": {
                "reviewer": {
                    "marketplace": "mp",
                    "plugin": "p",
                    "version": "1.0",
                    "installed_at": "2026-01-01T00:00:00Z",
                    "dialect": "native",
                    "source_path": "agents/reviewer.json"
                }
            }
        }"#;
        std::fs::write(kiro_dir.join("installed-agents.json"), legacy_json)
            .expect("write legacy tracking");

        let err = project
            .load_installed_agents()
            .expect_err("legacy agent entry without source_hash must fail to deserialize");
        let msg = err.to_string();
        assert!(
            msg.contains("source_hash") || msg.contains("installed_hash"),
            "error must mention the missing required field; got: {msg}"
        );
    }

    #[test]
    fn load_installed_native_companions_rejects_legacy_entry_without_source_scan_root() {
        let (_dir, project) = temp_project();
        let kiro_dir = project.kiro_dir();
        std::fs::create_dir_all(&kiro_dir).expect("create .kiro");

        let legacy_json = br#"{
            "agents": {},
            "native_companions": {
                "p": {
                    "marketplace": "mp",
                    "plugin": "p",
                    "version": "1.0",
                    "installed_at": "2026-01-01T00:00:00Z",
                    "files": ["prompts/reviewer.md"],
                    "source_hash": "blake3:0000",
                    "installed_hash": "blake3:0000"
                }
            }
        }"#;
        std::fs::write(kiro_dir.join("installed-agents.json"), legacy_json)
            .expect("write legacy tracking");

        let err = project
            .load_installed_agents()
            .expect_err("legacy companions entry must fail");
        assert!(
            err.to_string().contains("source_scan_root"),
            "error must mention source_scan_root; got: {err}"
        );
    }

    #[test]
    fn installed_agents_with_empty_native_companions_does_not_serialize_the_field() {
        // Regression guard: a legacy tracking file (no native_companions key)
        // must round-trip byte-identical when no companions exist. Without
        // skip_serializing_if = "HashMap::is_empty", the empty default would
        // serialize as `"native_companions": {}` and silently mutate the file.
        let installed = InstalledAgents {
            agents: std::collections::HashMap::new(),
            native_companions: std::collections::HashMap::new(),
        };

        let json = serde_json::to_string(&installed).unwrap();
        assert!(
            !json.contains("native_companions"),
            "empty native_companions must be omitted from serialized output, got: {json}"
        );
    }

    #[test]
    fn install_skill_from_dir_populates_source_and_installed_hashes() {
        let (tmp, project) = temp_project();

        // Create a tiny source skill directory.
        let skill_src = tmp.path().join("source");
        fs::create_dir_all(&skill_src).unwrap();
        fs::write(skill_src.join("SKILL.md"), b"# test skill\n\nbody").unwrap();

        let meta = InstalledSkillMeta {
            marketplace: mp("m"),
            plugin: pn("p"),
            version: Some("1.0.0".into()),
            installed_at: chrono::Utc::now(),
            source_hash: String::new(),
            installed_hash: String::new(),
            source_scan_root: RelativePath::new("skills").expect("valid"),
        };

        project
            .install_skill_from_dir("test", &skill_src, meta)
            .unwrap();

        let installed = project.load_installed().unwrap();
        let entry = installed.skills.get("test").expect("entry persisted");

        let src_hash = &entry.source_hash;
        let inst_hash = &entry.installed_hash;

        assert!(src_hash.starts_with("blake3:"));
        assert!(inst_hash.starts_with("blake3:"));
        // Source and installed contents are identical (we just copied), so the
        // hashes match.
        assert_eq!(src_hash, inst_hash);
    }

    #[test]
    fn install_agent_translated_populates_source_and_installed_hashes() {
        let tmp = tempfile::tempdir().unwrap();
        let project = KiroProject::new(tmp.path().to_path_buf());

        let source_md = write_agent(tmp.path(), "rev", "You are a reviewer.");
        let def = crate::agent::AgentDefinition {
            name: "rev".into(),
            description: None,
            prompt_body: "You are a reviewer.".into(),
            model: None,
            source_tools: vec![],
            mcp_servers: std::collections::BTreeMap::new(),
            dialect: crate::agent::AgentDialect::Claude,
        };
        let mapped: Vec<crate::agent::tools::MappedTool> = vec![];
        let mut meta = sample_agent_meta();
        meta.source_hash = String::new();
        meta.installed_hash = String::new();
        let plugin_name = meta.plugin.clone();

        project
            .install_agent(&def, &mapped, meta, Some(&source_md))
            .expect("install succeeds");

        let installed = project.load_installed_agents().unwrap();
        let entry = installed.agents.get("rev").expect("entry persisted");

        let src = &entry.source_hash;
        let inst = &entry.installed_hash;
        assert!(src.starts_with("blake3:"));
        assert!(inst.starts_with("blake3:"));
        // Translated path: source bytes (raw .md) differ from installed bytes
        // (emitted .json + prompt body), so the two hashes ARE different here.
        assert_ne!(src, inst);

        // Sanity: re-hashing the source file directly matches the recorded
        // source_hash.
        let recomputed_src = crate::hash::hash_artifact(
            source_md.parent().unwrap(),
            &[std::path::PathBuf::from(source_md.file_name().unwrap())],
        )
        .unwrap();
        assert_eq!(src, &recomputed_src);

        // Companion-entry synthesis: this plugin should now own
        // `prompts/rev.md` in the native_companions map.
        let companion = installed
            .native_companions
            .get(plugin_name.as_str())
            .expect("native_companions entry synthesized");
        assert!(
            companion
                .files
                .contains(&std::path::PathBuf::from("prompts/rev.md")),
            "prompt file must be tracked under native_companions: {:?}",
            companion.files
        );
        assert!(companion.source_hash.starts_with("blake3:"));
        assert_eq!(companion.source_hash, companion.installed_hash);
    }

    #[test]
    fn install_agent_translated_appends_to_existing_companion_entry() {
        // A plugin that installs TWO translated agents must end up with a
        // single native_companions entry listing BOTH prompt files.
        let tmp = tempfile::tempdir().unwrap();
        let project = KiroProject::new(tmp.path().to_path_buf());
        let plugin_name = sample_agent_meta().plugin.clone();

        for name in ["alpha", "beta"] {
            let source_md = write_agent(tmp.path(), name, "body");
            let def = crate::agent::AgentDefinition {
                name: name.into(),
                description: None,
                prompt_body: "body".into(),
                model: None,
                source_tools: vec![],
                mcp_servers: std::collections::BTreeMap::new(),
                dialect: crate::agent::AgentDialect::Claude,
            };
            let mut meta = sample_agent_meta();
            meta.source_hash = String::new();
            meta.installed_hash = String::new();
            project
                .install_agent(&def, &[], meta, Some(&source_md))
                .expect("install succeeds");
        }

        let installed = project.load_installed_agents().unwrap();
        let companion = installed
            .native_companions
            .get(plugin_name.as_str())
            .expect("entry exists");
        assert_eq!(companion.files.len(), 2);
        assert!(
            companion
                .files
                .contains(&std::path::PathBuf::from("prompts/alpha.md"))
        );
        assert!(
            companion
                .files
                .contains(&std::path::PathBuf::from("prompts/beta.md"))
        );
    }

    /// PR #100 review I3: when a plugin's manifest changes its
    /// translated-agent source path between installs, the recorded
    /// `source_scan_root` on the existing `native_companions` entry
    /// must refresh to the latest install's scan root — otherwise
    /// drift detection (once issue #99 lands and starts consulting
    /// the field) would look up the source under the stale root and
    /// emit false drift on every scan. Pre-fix the `or_insert_with`
    /// initializer set the value once and the post-insert refresh
    /// updated `marketplace`/`version`/`installed_at`/`files` but
    /// skipped `source_scan_root`, so the very first install's value
    /// stuck forever.
    #[test]
    fn install_agent_translated_refreshes_source_scan_root_on_subsequent_install() {
        let tmp = tempfile::tempdir().unwrap();
        let project = KiroProject::new(tmp.path().to_path_buf());

        // First install: source under `agents/`.
        let alpha_md = write_agent(tmp.path(), "alpha", "body");
        let alpha_def = crate::agent::AgentDefinition {
            name: "alpha".into(),
            description: None,
            prompt_body: "body".into(),
            model: None,
            source_tools: vec![],
            mcp_servers: std::collections::BTreeMap::new(),
            dialect: crate::agent::AgentDialect::Claude,
        };
        let mut meta_alpha = sample_agent_meta();
        meta_alpha.source_path = RelativePath::new("agents/alpha.md").expect("valid");
        meta_alpha.source_hash = String::new();
        meta_alpha.installed_hash = String::new();
        project
            .install_agent(&alpha_def, &[], meta_alpha, Some(&alpha_md))
            .expect("install alpha");

        let installed = project.load_installed_agents().unwrap();
        let plugin_key = sample_agent_meta().plugin.clone();
        assert_eq!(
            installed
                .native_companions
                .get(plugin_key.as_str())
                .expect("entry after first install")
                .source_scan_root
                .as_str(),
            "agents",
            "first install records its own scan root"
        );

        // Second install for the SAME plugin from a manifest that
        // moved its agents under `prompts/` — the recorded
        // source_scan_root must follow.
        let beta_md = write_agent(tmp.path(), "beta", "body2");
        let beta_def = crate::agent::AgentDefinition {
            name: "beta".into(),
            description: None,
            prompt_body: "body2".into(),
            model: None,
            source_tools: vec![],
            mcp_servers: std::collections::BTreeMap::new(),
            dialect: crate::agent::AgentDialect::Claude,
        };
        let mut meta_beta = sample_agent_meta();
        meta_beta.source_path = RelativePath::new("prompts/beta.md").expect("valid");
        meta_beta.source_hash = String::new();
        meta_beta.installed_hash = String::new();
        project
            .install_agent(&beta_def, &[], meta_beta, Some(&beta_md))
            .expect("install beta");

        let installed = project.load_installed_agents().unwrap();
        let companion = installed
            .native_companions
            .get(plugin_key.as_str())
            .expect("entry after second install");
        assert_eq!(
            companion.source_scan_root.as_str(),
            "prompts",
            "second install must refresh the recorded source_scan_root \
             (regression: pre-fix it stuck at the first install's value)"
        );
    }

    // -----------------------------------------------------------------------
    // install_native_agent
    // -----------------------------------------------------------------------

    use rstest::{fixture, rstest};

    /// Source bytes for a minimal valid native agent named `rev`. Reused
    /// across collision tests where the specific JSON content doesn't
    /// matter — only its hash and identity do.
    const REV_BODY: &[u8] = br#"{"name":"rev"}"#;

    /// Fully-baked test fixture: a tempdir, a project rooted at it, a
    /// staged-and-parsed `NativeAgentBundle` for `rev`, and the
    /// pre-computed `source_hash` over the staging dir. Owns the tempdir
    /// (kept alive for the test's lifetime).
    struct NativeRev {
        _dir: tempfile::TempDir,
        project: KiroProject,
        bundle: crate::agent::NativeAgentBundle,
        src_dir: std::path::PathBuf,
        src_json: std::path::PathBuf,
        source_hash: String,
    }

    impl NativeRev {
        /// Re-stage and re-parse the source JSON after the body changes.
        /// Used by the content-changed test (T12) to bump from v1 to v2
        /// without re-creating the tempdir or project.
        fn rewrite_source(&mut self, new_body: &[u8]) {
            fs::write(&self.src_json, new_body).expect("rewrite source");
            self.bundle = crate::agent::parse_native_kiro_agent_file(&self.src_json, &self.src_dir)
                .expect("re-parse bundle");
            self.source_hash =
                crate::hash::hash_artifact(&self.src_dir, &[std::path::PathBuf::from("rev.json")])
                    .expect("re-hash");
        }
    }

    /// Stage a source agent JSON in `<tmp>/source-agents/` and parse it
    /// into a `NativeAgentBundle` ready for install.
    fn stage_native_source(
        scratch: &Path,
        name: &str,
        body: &[u8],
    ) -> (
        crate::agent::NativeAgentBundle,
        std::path::PathBuf,
        std::path::PathBuf,
    ) {
        let src_dir = scratch.join("source-agents");
        fs::create_dir_all(&src_dir).expect("create source-agents");
        let src_json = src_dir.join(format!("{name}.json"));
        fs::write(&src_json, body).expect("write source");
        let bundle = crate::agent::parse_native_kiro_agent_file(&src_json, &src_dir)
            .expect("parse native agent");
        (bundle, src_dir, src_json)
    }

    #[fixture]
    fn native_rev() -> NativeRev {
        let dir = tempfile::tempdir().expect("tempdir");
        let project = KiroProject::new(dir.path().to_path_buf());
        let (bundle, src_dir, src_json) = stage_native_source(dir.path(), "rev", REV_BODY);
        let source_hash =
            crate::hash::hash_artifact(&src_dir, &[std::path::PathBuf::from("rev.json")])
                .expect("source hash");
        NativeRev {
            _dir: dir,
            project,
            bundle,
            src_dir,
            src_json,
            source_hash,
        }
    }

    /// Convenience: install `rev` from the fixture under `(marketplace,
    /// plugin)`. Wraps the same `install_native_agent` call every test
    /// makes, parameterised only by mode and identity.
    fn install_rev(
        f: &NativeRev,
        marketplace: &str,
        plugin: &str,
        mode: crate::service::InstallMode,
    ) -> Result<InstalledNativeAgentOutcome, AgentError> {
        // Wrap once at the helper boundary so the test bodies stay
        // string-literal-friendly (they pass `"m"`, `"plugin-a"` etc.).
        let marketplace = mp(marketplace);
        let plugin = pn(plugin);
        // Synthetic source_path; the install_native_agent function
        // only stores it on the tracking meta — these tests exercise
        // install behavior, not detection.
        let source_path = RelativePath::new("agents/rev.json").expect("valid");
        f.project.install_native_agent(&NativeAgentInstallInput {
            bundle: &f.bundle,
            marketplace: &marketplace,
            plugin: &plugin,
            version: None,
            source_hash: &f.source_hash,
            source_path: &source_path,
            mode,
        })
    }

    #[test]
    fn install_native_agent_writes_json_with_dialect_native_and_hashes() {
        // Happy-path test uses a richer body than the fixture's REV_BODY
        // so the assertions exercise version, marketplace, and plugin
        // fields together.
        let (dir, project) = temp_project();
        let scratch = dir.path();
        let (bundle, src_dir, _src_json) = stage_native_source(
            scratch,
            "rev",
            br#"{"name": "rev", "prompt": "You are a reviewer."}"#,
        );
        let source_hash =
            crate::hash::hash_artifact(&src_dir, &[std::path::PathBuf::from("rev.json")])
                .expect("source hash");

        let outcome = project
            .install_native_agent(&NativeAgentInstallInput {
                bundle: &bundle,
                marketplace: &mp("marketplace-x"),
                plugin: &pn("plugin-y"),
                version: Some("0.1.0"),
                source_hash: &source_hash,
                source_path: &RelativePath::new("agents/rev.json").expect("valid"),
                mode: crate::service::InstallMode::New,
            })
            .expect("install_native_agent must succeed");

        assert_eq!(outcome.name, "rev");
        assert!(outcome.json_path.ends_with("rev.json"));
        assert_eq!(outcome.kind, InstallOutcomeKind::Installed);
        assert_eq!(outcome.source_hash, source_hash);
        assert!(outcome.installed_hash.starts_with("blake3:"));
        assert!(outcome.json_path.exists());

        let tracking = project.load_installed_agents().expect("load tracking");
        let entry = tracking.agents.get("rev").expect("entry persisted");
        assert_eq!(entry.dialect, crate::agent::AgentDialect::Native);
        assert_eq!(entry.plugin, "plugin-y");
        assert_eq!(entry.marketplace, "marketplace-x");
        assert_eq!(entry.source_hash, source_hash);
        assert_eq!(entry.installed_hash, outcome.installed_hash);
    }

    #[rstest]
    fn install_native_agent_idempotent_when_source_hash_matches(native_rev: NativeRev) {
        let first = install_rev(&native_rev, "m", "p", crate::service::InstallMode::New)
            .expect("first install");
        assert_eq!(first.kind, InstallOutcomeKind::Installed);
        let first_installed_at = native_rev
            .project
            .load_installed_agents()
            .expect("load")
            .agents
            .get("rev")
            .expect("entry")
            .installed_at;

        // Reinstall with the same source_hash — must be a verified no-op.
        let second = install_rev(&native_rev, "m", "p", crate::service::InstallMode::New)
            .expect("second install");
        assert_eq!(second.kind, InstallOutcomeKind::Idempotent);
        // Idempotent path must NOT touch tracking — installed_at should
        // still reflect the first install, proving no write occurred.
        let second_installed_at = native_rev
            .project
            .load_installed_agents()
            .expect("load")
            .agents
            .get("rev")
            .expect("entry")
            .installed_at;
        assert_eq!(first_installed_at, second_installed_at);
    }

    #[rstest]
    fn install_native_agent_content_changed_requires_force(mut native_rev: NativeRev) {
        // v1 install seeds tracking.
        let h_v1 = native_rev.source_hash.clone();
        install_rev(&native_rev, "m", "p", crate::service::InstallMode::New)
            .expect("first install");

        // Bump source content. Fixture handles re-parse + re-hash.
        native_rev.rewrite_source(br#"{"name":"rev","v":2}"#);
        assert_ne!(h_v1, native_rev.source_hash);

        // Without --force: must fail with ContentChangedRequiresForce.
        let err = install_rev(&native_rev, "m", "p", crate::service::InstallMode::New)
            .expect_err("must refuse");
        match err {
            AgentError::ContentChangedRequiresForce { name } => {
                assert_eq!(name, "rev");
            }
            other => panic!("expected ContentChangedRequiresForce, got {other:?}"),
        }

        // With --force: succeeds, kind is ForceOverwrote, content updates.
        let outcome = install_rev(&native_rev, "m", "p", crate::service::InstallMode::Force)
            .expect("force install");
        assert_eq!(outcome.kind, InstallOutcomeKind::ForceOverwrote);
        assert_eq!(outcome.source_hash, native_rev.source_hash);
        let installed_bytes = fs::read(&outcome.json_path).expect("read installed");
        assert_eq!(installed_bytes, br#"{"name":"rev","v":2}"#);
    }

    #[rstest]
    fn install_native_agent_cross_plugin_name_clash_fails_loudly(native_rev: NativeRev) {
        // plugin-a installs first.
        install_rev(
            &native_rev,
            "m",
            "plugin-a",
            crate::service::InstallMode::New,
        )
        .expect("plugin-a install");

        // plugin-b tries to install the same agent name — must fail.
        let err = install_rev(
            &native_rev,
            "m",
            "plugin-b",
            crate::service::InstallMode::New,
        )
        .expect_err("must refuse");
        match err {
            AgentError::NameClashWithOtherPlugin { name, owner } => {
                assert_eq!(name, "rev");
                assert_eq!(owner, "plugin-a");
            }
            other => panic!("expected NameClashWithOtherPlugin, got {other:?}"),
        }

        // With --force: ownership transfers to plugin-b.
        let outcome = install_rev(
            &native_rev,
            "m",
            "plugin-b",
            crate::service::InstallMode::Force,
        )
        .expect("force transfer");
        assert_eq!(outcome.kind, InstallOutcomeKind::ForceOverwrote);

        let tracking = native_rev.project.load_installed_agents().expect("load");
        let entry = tracking.agents.get("rev").expect("entry");
        assert_eq!(entry.plugin, "plugin-b", "ownership must transfer");
    }

    #[rstest]
    fn install_native_agent_orphan_at_destination_fails_loudly(native_rev: NativeRev) {
        // Pre-create the destination with no tracking (orphan from a manual
        // copy or a prior crashed install).
        fs::create_dir_all(native_rev.project.kiro_dir().join("agents"))
            .expect("create agents dir");
        let orphan_path = native_rev
            .project
            .kiro_dir()
            .join("agents")
            .join("rev.json");
        fs::write(&orphan_path, b"orphan content").expect("write orphan");

        // Without --force: must fail with OrphanFileAtDestination.
        let err = install_rev(&native_rev, "m", "p", crate::service::InstallMode::New)
            .expect_err("must refuse");
        match err {
            AgentError::OrphanFileAtDestination { path } => {
                assert_eq!(path, orphan_path);
            }
            other => panic!("expected OrphanFileAtDestination, got {other:?}"),
        }

        // With --force: orphan is overwritten and ownership recorded.
        let outcome = install_rev(&native_rev, "m", "p", crate::service::InstallMode::Force)
            .expect("force install");
        assert_eq!(outcome.kind, InstallOutcomeKind::ForceOverwrote);

        let tracking = native_rev.project.load_installed_agents().expect("load");
        assert!(tracking.agents.contains_key("rev"));
        let installed_bytes = fs::read(&orphan_path).expect("read installed");
        assert_eq!(installed_bytes, REV_BODY);
    }

    #[test]
    fn install_native_agent_writes_raw_bytes_verbatim() {
        // Source contains non-canonical whitespace + field ordering.
        // The installed file must be byte-for-byte identical to the source
        // (per the design doc's "v1 preserves verbatim" promise).
        let (dir, project) = temp_project();
        let scratch = dir.path();
        let body = b"{\n  \"name\":   \"rev\",\n     \"prompt\":\"x\"\n}\n";
        let (bundle, src_dir, _src_json) = stage_native_source(scratch, "rev", body);
        let source_hash =
            crate::hash::hash_artifact(&src_dir, &[std::path::PathBuf::from("rev.json")])
                .expect("source hash");

        let outcome = project
            .install_native_agent(&NativeAgentInstallInput {
                bundle: &bundle,
                marketplace: &mp("m"),
                plugin: &pn("p"),
                version: None,
                source_hash: &source_hash,
                source_path: &RelativePath::new("agents/rev.json").expect("valid"),
                mode: crate::service::InstallMode::New,
            })
            .expect("install");

        let installed_bytes = fs::read(&outcome.json_path).expect("read installed");
        assert_eq!(installed_bytes.as_slice(), body.as_slice());

        // Closes pr-test-analyzer C5: native install writes bytes
        // verbatim, so installed_hash must equal source_hash exactly.
        // A future bug where staging accidentally normalizes / re-encodes
        // before the hash would only surface as silent hash drift.
        assert_eq!(
            outcome.installed_hash, source_hash,
            "native install must produce installed_hash == source_hash (verbatim copy invariant)"
        );
    }

    #[test]
    fn install_native_agent_rollback_restores_when_tracking_write_fails() {
        // Closes pr-test-analyzer C3 (native agent half).
        let (_dir, project) = temp_project();
        let scratch = tempfile::tempdir().unwrap();
        let body_v1 = br#"{"name":"rev","prompt":"v1"}"#;
        let (bundle_v1, src_dir, _) = stage_native_source(scratch.path(), "rev", body_v1);
        let h_v1 =
            crate::hash::hash_artifact(&src_dir, &[std::path::PathBuf::from("rev.json")]).unwrap();
        project
            .install_native_agent(&NativeAgentInstallInput {
                bundle: &bundle_v1,
                marketplace: &mp("m"),
                plugin: &pn("p"),
                version: None,
                source_hash: &h_v1,
                source_path: &RelativePath::new("agents/rev.json").expect("valid"),
                mode: crate::service::InstallMode::New,
            })
            .expect("v1 install");

        let dest = project.root.join(".kiro/agents/rev.json");
        let v1_bytes = fs::read(&dest).unwrap();
        assert_eq!(v1_bytes.as_slice(), body_v1.as_slice());

        // Poison the atomic_write tmp path (NOT the tracking path itself).
        // `cache::atomic_write` opens `<path>.with_extension("tmp")` first,
        // syncs, then renames into place. Pre-creating that tmp path as a
        // directory blocks the OpenOptions::create+truncate+open call but
        // leaves the real tracking file readable — so `load_installed_agents`
        // succeeds, the install proceeds through promote (acquiring backups),
        // and `write_agent_tracking` fails AFTER promotion. That's the
        // sequence the rollback path is designed to handle. (Replacing the
        // tracking file itself with a directory makes `load_installed_agents`
        // fail BEFORE promotion, never reaching the rollback code.)
        let tmp_blocker = project.root.join(".kiro/installed-agents.tmp");
        fs::create_dir_all(&tmp_blocker).unwrap();

        // Force-install v2 (stage_native_source overwrites the source).
        let body_v2 = br#"{"name":"rev","prompt":"v2"}"#;
        let (bundle_v2, _, _) = stage_native_source(scratch.path(), "rev", body_v2);
        let h_v2 =
            crate::hash::hash_artifact(&src_dir, &[std::path::PathBuf::from("rev.json")]).unwrap();
        let err = project
            .install_native_agent(&NativeAgentInstallInput {
                bundle: &bundle_v2,
                marketplace: &mp("m"),
                plugin: &pn("p"),
                version: None,
                source_hash: &h_v2,
                source_path: &RelativePath::new("agents/rev.json").expect("valid"),
                mode: crate::service::InstallMode::Force,
            })
            .expect_err("tracking write must fail");
        assert!(matches!(err, AgentError::InstallFailed { .. }));

        // V1 bytes must be restored (backup-then-swap rollback).
        assert_eq!(
            fs::read(&dest).unwrap(),
            v1_bytes,
            "v1 must be restored from backup after tracking-write failure"
        );
        // No leftover .kiro-bak.
        assert!(
            !project.root.join(".kiro/agents/rev.json.kiro-bak").exists(),
            "backup must be consumed by the restore"
        );
    }

    #[test]
    fn install_steering_file_rollback_restores_when_tracking_write_fails() {
        // Closes pr-test-analyzer C3 (steering half).
        let (_dir, project) = temp_project();
        let scratch = tempfile::tempdir().unwrap();
        let scan_root = scratch.path().join("src");
        fs::create_dir_all(&scan_root).unwrap();
        fs::write(scan_root.join("guide.md"), b"v1").unwrap();
        let h_v1 = crate::hash::hash_artifact(&scan_root, &[PathBuf::from("guide.md")]).unwrap();

        let discovered = crate::agent::DiscoveredNativeFile {
            source: scan_root.join("guide.md"),
            scan_root: scan_root.clone(),
        };
        let mp_name = mp("m");
        let pn_name = pn("p");
        project
            .install_steering_file(
                &discovered,
                &h_v1,
                crate::steering::SteeringInstallContext {
                    mode: crate::service::InstallMode::New,
                    marketplace: &mp_name,
                    plugin: &pn_name,
                    version: None,
                    plugin_dir: scratch.path(),
                },
            )
            .expect("v1 install");

        let dest = project.root.join(".kiro/steering/guide.md");
        assert_eq!(fs::read(&dest).unwrap(), b"v1");

        // Poison the atomic_write tmp path so write_steering_tracking
        // fails AFTER promote without breaking load_installed_steering.
        // See install_native_agent_rollback_restores_when_tracking_write_fails
        // for the full rationale.
        let tmp_blocker = project.root.join(".kiro/installed-steering.tmp");
        fs::create_dir_all(&tmp_blocker).unwrap();

        // Force-install v2.
        fs::write(scan_root.join("guide.md"), b"v2").unwrap();
        let h_v2 = crate::hash::hash_artifact(&scan_root, &[PathBuf::from("guide.md")]).unwrap();
        let err = project
            .install_steering_file(
                &discovered,
                &h_v2,
                crate::steering::SteeringInstallContext {
                    mode: crate::service::InstallMode::Force,
                    marketplace: &mp_name,
                    plugin: &pn_name,
                    version: None,
                    plugin_dir: scratch.path(),
                },
            )
            .expect_err("tracking write must fail");
        assert!(matches!(
            err,
            crate::steering::SteeringError::TrackingIoFailed { .. }
        ));

        // V1 bytes must be restored.
        assert_eq!(
            fs::read(&dest).unwrap(),
            b"v1",
            "v1 must be restored from backup after tracking-write failure"
        );
        assert!(
            !project
                .root
                .join(".kiro/steering/guide.md.kiro-bak")
                .exists(),
            "backup must be consumed by the restore"
        );
    }

    // -----------------------------------------------------------------------
    // install_native_companions
    // -----------------------------------------------------------------------

    /// Stage two companion files at `<scratch>/companions-src/prompts/{a,b}.md`
    /// with the given body bytes. Returns `(scan_root, rel_paths, source_hash)`.
    fn stage_companion_source(
        scratch: &Path,
        bodies: &[(&str, &[u8])],
    ) -> (PathBuf, Vec<PathBuf>, String) {
        let scan_root = scratch.join("companions-src");
        let prompts = scan_root.join("prompts");
        fs::create_dir_all(&prompts).expect("create prompts dir");
        let mut rel_paths = Vec::new();
        for (name, body) in bodies {
            let rel = PathBuf::from(format!("prompts/{name}"));
            fs::write(scan_root.join(&rel), body).expect("write companion source");
            rel_paths.push(rel);
        }
        let source_hash =
            crate::hash::hash_artifact(&scan_root, &rel_paths).expect("companion source hash");
        (scan_root, rel_paths, source_hash)
    }

    #[test]
    fn install_native_companions_copies_files_and_writes_tracking() {
        let (dir, project) = temp_project();
        let (scan_root, rel_paths, source_hash) =
            stage_companion_source(dir.path(), &[("a.md", b"prompt a"), ("b.md", b"prompt b")]);

        let mp_x = mp("marketplace-x");
        let pn_y = pn("plugin-y");
        let outcome = project
            .install_native_companions(&NativeCompanionsInput {
                scan_root: &scan_root,
                rel_paths: &rel_paths,
                marketplace: &mp_x,
                plugin: &pn_y,
                version: Some("0.1.0"),
                source_hash: &source_hash,
                mode: crate::service::InstallMode::New,
                plugin_dir: dir.path(),
            })
            .expect("install companions");

        assert_eq!(outcome.plugin, "plugin-y");
        assert_eq!(outcome.files.len(), 2);
        assert_eq!(outcome.kind, InstallOutcomeKind::Installed);
        assert_eq!(outcome.source_hash, source_hash);
        assert!(outcome.installed_hash.starts_with("blake3:"));

        // Files landed at the right destinations with original content.
        let dest_a = project.kiro_dir().join("agents/prompts/a.md");
        let dest_b = project.kiro_dir().join("agents/prompts/b.md");
        assert!(dest_a.exists(), "a.md must land at {}", dest_a.display());
        assert!(dest_b.exists(), "b.md must land at {}", dest_b.display());
        assert_eq!(fs::read(&dest_a).expect("read a"), b"prompt a");
        assert_eq!(fs::read(&dest_b).expect("read b"), b"prompt b");

        // Tracking entry records the bundle.
        let tracking = project.load_installed_agents().expect("load");
        let entry = tracking
            .native_companions
            .get("plugin-y")
            .expect("native_companions entry written");
        assert_eq!(entry.plugin, "plugin-y");
        assert_eq!(entry.marketplace, "marketplace-x");
        assert_eq!(entry.version.as_deref(), Some("0.1.0"));
        assert_eq!(entry.files.len(), 2);
        assert_eq!(entry.source_hash, source_hash);
        assert_eq!(entry.installed_hash, outcome.installed_hash);
    }

    #[test]
    fn install_native_companions_empty_files_is_idempotent_no_op() {
        // Empty rel_paths returns an idempotent outcome with no tracking
        // write — the bundle has nothing to install, and we shouldn't
        // create a tracking entry for an empty file set.
        let (_dir, project) = temp_project();
        let scan_root = std::path::PathBuf::from("/tmp/unused");

        let outcome = project
            .install_native_companions(&NativeCompanionsInput {
                scan_root: &scan_root,
                rel_paths: &[],
                marketplace: &mp("m"),
                plugin: &pn("p"),
                version: None,
                source_hash: "blake3:empty",
                mode: crate::service::InstallMode::New,
                plugin_dir: std::path::Path::new("/tmp"),
            })
            .expect("empty install");
        assert_eq!(outcome.kind, InstallOutcomeKind::Idempotent);
        assert!(outcome.files.is_empty());

        let tracking = project.load_installed_agents().expect("load");
        assert!(
            !tracking.native_companions.contains_key("p"),
            "empty bundle must NOT create a tracking entry"
        );
    }

    #[cfg(unix)]
    #[test]
    fn install_native_companions_refuses_hardlinked_source() {
        // A hardlinked companion source shares an inode with another
        // path that could be sensitive. Discovery's symlink/junction
        // filter doesn't catch hardlinks. Refuse at staging-time
        // before fs::copy.
        let (_dir, project) = temp_project();
        let scratch = tempfile::tempdir().unwrap();
        let scan_root = scratch.path().join("src");
        fs::create_dir_all(scan_root.join("prompts")).unwrap();

        // The hardlink target lives outside the scan_root subtree to
        // model the "exfil sensitive host file" threat. Both paths
        // point at the same inode.
        let outside = scratch.path().join("sensitive.md");
        fs::write(&outside, b"sensitive").unwrap();
        let linked = scan_root.join("prompts/a.md");
        fs::hard_link(&outside, &linked).expect("create hardlink");

        let rel_paths = vec![PathBuf::from("prompts/a.md")];
        let h = crate::hash::hash_artifact(&scan_root, &rel_paths).unwrap();

        let err = project
            .install_native_companions(&NativeCompanionsInput {
                scan_root: &scan_root,
                rel_paths: &rel_paths,
                marketplace: &mp("m"),
                plugin: &pn("p"),
                version: None,
                source_hash: &h,
                mode: crate::service::InstallMode::New,
                plugin_dir: scratch.path(),
            })
            .expect_err("hardlinked source must be refused");
        match err {
            AgentError::SourceHardlinked { path, nlink } => {
                assert_eq!(path, linked);
                assert!(nlink >= 2, "nlink must reflect the hardlink share");
            }
            other => panic!("expected AgentError::SourceHardlinked, got {other:?}"),
        }

        // Destination must remain untouched.
        assert!(
            !project.root.join(".kiro/agents/prompts/a.md").exists(),
            "destination must not exist after hardlink rejection"
        );
    }

    #[test]
    fn install_native_companions_force_shrink_preserves_files_when_tracking_write_fails() {
        // Atomicity regression test (code-reviewer #1 / silent-failure-hunter #2).
        // Pre-fix flow removed diffed prior files BEFORE write_agent_tracking,
        // so a tracking-write failure left the user with files removed AND
        // tracking still claiming them. Now diff captured pre-mutation,
        // removed only AFTER successful tracking write.
        let (_dir, project) = temp_project();

        // Stage a 2-file bundle for plugin P.
        let scratch = tempfile::tempdir().unwrap();
        let scan_root = scratch.path().join("src");
        fs::create_dir_all(scan_root.join("prompts")).unwrap();
        fs::write(scan_root.join("prompts/a.md"), b"a v1").unwrap();
        fs::write(scan_root.join("prompts/b.md"), b"b v1").unwrap();
        let rel_paths_v1 = vec![PathBuf::from("prompts/a.md"), PathBuf::from("prompts/b.md")];
        let h_v1 = crate::hash::hash_artifact(&scan_root, &rel_paths_v1).unwrap();

        project
            .install_native_companions(&NativeCompanionsInput {
                scan_root: &scan_root,
                rel_paths: &rel_paths_v1,
                marketplace: &mp("m"),
                plugin: &pn("p"),
                version: None,
                source_hash: &h_v1,
                plugin_dir: scratch.path(),
                mode: crate::service::InstallMode::New,
            })
            .expect("v1 install");

        let dest_a = project.root.join(".kiro/agents/prompts/a.md");
        let dest_b = project.root.join(".kiro/agents/prompts/b.md");
        assert!(dest_a.exists() && dest_b.exists());

        // Poison the atomic_write tmp path so write_agent_tracking
        // fails AFTER promote + diff-capture, exercising the
        // post-tracking-write rollback. Replacing the tracking file
        // itself with a directory would make load_installed_agents
        // fail BEFORE promote and never reach the diff/removal logic
        // this test is designed to exercise.
        let tmp_blocker = project.root.join(".kiro/installed-agents.tmp");
        fs::create_dir_all(&tmp_blocker).unwrap();

        // Bump a.md content + drop b.md from the new bundle. This is
        // the shrink case: prior tracking owned [a.md, b.md], new
        // bundle owns [a.md] only. Force install needed because
        // a.md content changed.
        fs::write(scan_root.join("prompts/a.md"), b"a v2").unwrap();
        let rel_paths_v2 = vec![PathBuf::from("prompts/a.md")];
        let h_v2 = crate::hash::hash_artifact(&scan_root, &rel_paths_v2).unwrap();

        let err = project
            .install_native_companions(&NativeCompanionsInput {
                scan_root: &scan_root,
                rel_paths: &rel_paths_v2,
                marketplace: &mp("m"),
                plugin: &pn("p"),
                version: None,
                source_hash: &h_v2,
                mode: crate::service::InstallMode::Force,
                plugin_dir: scratch.path(),
            })
            .expect_err("tracking write must fail");
        assert!(matches!(err, AgentError::InstallFailed { .. }));

        // Critical: b.md must still exist on disk. Pre-fix it would be
        // gone (removed before the failed tracking write).
        assert!(
            dest_b.exists(),
            "b.md must survive tracking-write failure on shrink — pre-fix \
             behaviour removed it before the write attempt"
        );
        // a.md must contain v1 content (rollback restored from backup).
        assert_eq!(fs::read(&dest_a).unwrap(), b"a v1");
    }

    #[test]
    fn install_native_companions_clean_install_rolls_back_placed_files_on_tracking_write_failure() {
        // Mirrors `install_native_agent_rollback_restores_when_tracking_write_fails`
        // for the companion path's clean-install branch (no backups, only
        // newly-placed files). The shrink test above covers `forced_overwrite
        // = true` with non-empty backups; this test covers `forced_overwrite
        // = false` with empty backups but non-empty `placed`.
        //
        // Pre-fix: if `rollback_companion_promotion(&placed, &backups)` ever
        // shipped a regression that skipped the placed-file removal when
        // `backups` was empty, a clean first-install with a tracking-write
        // failure would leave orphan files at the destination — the user
        // would then hit `OrphanFileAtDestination` on every subsequent
        // install attempt with no obvious recovery path.
        let (_dir, project) = temp_project();

        let scratch = tempfile::tempdir().unwrap();
        let scan_root = scratch.path().join("src");
        fs::create_dir_all(scan_root.join("prompts")).unwrap();
        fs::write(scan_root.join("prompts/a.md"), b"a v1").unwrap();
        fs::write(scan_root.join("prompts/b.md"), b"b v1").unwrap();
        let rel_paths = vec![PathBuf::from("prompts/a.md"), PathBuf::from("prompts/b.md")];
        let source_hash = crate::hash::hash_artifact(&scan_root, &rel_paths).unwrap();

        // Poison the atomic_write tmp path BEFORE the install so
        // write_agent_tracking fails on the very first attempt — the
        // companion promote already happened, but the tracking write is
        // about to fail. Replacing the tracking file itself with a directory
        // would make load_installed_agents fail before promote and never
        // reach the rollback path.
        let tmp_blocker = project.root.join(".kiro/installed-agents.tmp");
        fs::create_dir_all(&tmp_blocker).unwrap();

        let err = project
            .install_native_companions(&NativeCompanionsInput {
                scan_root: &scan_root,
                rel_paths: &rel_paths,
                marketplace: &mp("m"),
                plugin: &pn("p"),
                version: None,
                source_hash: &source_hash,
                mode: crate::service::InstallMode::New,
                plugin_dir: scratch.path(),
            })
            .expect_err("tracking write must fail with .tmp dir blocker in place");
        assert!(matches!(err, AgentError::InstallFailed { .. }));

        // Rollback contract: placed files must be removed so the user is
        // not left with orphan files blocking future install attempts.
        let dest_a = project.root.join(".kiro/agents/prompts/a.md");
        let dest_b = project.root.join(".kiro/agents/prompts/b.md");
        assert!(
            !dest_a.exists(),
            "clean-install rollback must remove placed file a.md after tracking-write failure"
        );
        assert!(
            !dest_b.exists(),
            "clean-install rollback must remove placed file b.md after tracking-write failure"
        );
    }

    /// Fixture: a tempdir, a project, a single-file companion bundle
    /// staged under `companions-src/prompts/a.md`, plus the precomputed
    /// `source_hash`. Reused across the three collision tests.
    struct CompanionBundle {
        /// Owns the tempdir lifetime AND exposes its path for tests that
        /// need to stage sibling source trees (e.g. cross-plugin transfer).
        scratch: tempfile::TempDir,
        project: KiroProject,
        scan_root: PathBuf,
        rel_paths: Vec<PathBuf>,
        source_hash: String,
    }

    impl CompanionBundle {
        /// Re-stage the source with new content and recompute the hash,
        /// preserving the same `rel_paths`. Used by the content-changed
        /// test to bump the body without rebuilding the whole fixture.
        fn rewrite_source(&mut self, body: &[u8]) {
            for rel in &self.rel_paths {
                fs::write(self.scan_root.join(rel), body).expect("rewrite source");
            }
            self.source_hash =
                crate::hash::hash_artifact(&self.scan_root, &self.rel_paths).expect("re-hash");
        }
    }

    #[fixture]
    fn companion_bundle() -> CompanionBundle {
        let dir = tempfile::tempdir().expect("tempdir");
        let project = KiroProject::new(dir.path().to_path_buf());
        let (scan_root, rel_paths, source_hash) =
            stage_companion_source(dir.path(), &[("a.md", b"prompt a")]);
        CompanionBundle {
            scratch: dir,
            project,
            scan_root,
            rel_paths,
            source_hash,
        }
    }

    /// Convenience: install the fixture's bundle under `(marketplace,
    /// plugin)`. Wraps the seven-arg `install_native_companions` call.
    fn install_companions(
        f: &CompanionBundle,
        marketplace: &str,
        plugin: &str,
        mode: crate::service::InstallMode,
    ) -> Result<InstalledNativeCompanionsOutcome, AgentError> {
        // Wrap once at the helper boundary; tests pass `"m"`, `"plugin-a"`
        // etc. and the wrap stays internal.
        let marketplace = mp(marketplace);
        let plugin = pn(plugin);
        f.project.install_native_companions(&NativeCompanionsInput {
            scan_root: &f.scan_root,
            rel_paths: &f.rel_paths,
            marketplace: &marketplace,
            plugin: &plugin,
            version: None,
            source_hash: &f.source_hash,
            mode,
            plugin_dir: f.scratch.path(),
        })
    }

    #[rstest]
    fn install_native_companions_idempotent_when_source_hash_matches(
        companion_bundle: CompanionBundle,
    ) {
        let first = install_companions(
            &companion_bundle,
            "m",
            "p",
            crate::service::InstallMode::New,
        )
        .expect("first");
        assert_eq!(first.kind, InstallOutcomeKind::Installed);

        let first_installed_at = companion_bundle
            .project
            .load_installed_agents()
            .expect("load")
            .native_companions
            .get("p")
            .expect("entry")
            .installed_at;

        let second = install_companions(
            &companion_bundle,
            "m",
            "p",
            crate::service::InstallMode::New,
        )
        .expect("second");
        assert_eq!(second.kind, InstallOutcomeKind::Idempotent);

        // Idempotent path must NOT touch tracking.
        let second_installed_at = companion_bundle
            .project
            .load_installed_agents()
            .expect("load")
            .native_companions
            .get("p")
            .expect("entry")
            .installed_at;
        assert_eq!(first_installed_at, second_installed_at);
    }

    #[rstest]
    fn install_native_companions_content_changed_requires_force(
        mut companion_bundle: CompanionBundle,
    ) {
        // v1 install seeds tracking.
        let h_v1 = companion_bundle.source_hash.clone();
        install_companions(
            &companion_bundle,
            "m",
            "p",
            crate::service::InstallMode::New,
        )
        .expect("first");

        // Bump source content.
        companion_bundle.rewrite_source(b"prompt v2");
        assert_ne!(h_v1, companion_bundle.source_hash);

        // Without --force: must fail with ContentChangedRequiresForce.
        let err = install_companions(
            &companion_bundle,
            "m",
            "p",
            crate::service::InstallMode::New,
        )
        .expect_err("must refuse");
        match err {
            AgentError::ContentChangedRequiresForce { name } => {
                assert!(
                    name.contains('p') && name.contains("companions"),
                    "ContentChangedRequiresForce name should reference plugin and \
                     'companions' to disambiguate from agent collisions; got: {name}"
                );
            }
            other => panic!("expected ContentChangedRequiresForce, got {other:?}"),
        }

        // With --force: succeeds, content updates, kind is ForceOverwrote.
        let outcome = install_companions(
            &companion_bundle,
            "m",
            "p",
            crate::service::InstallMode::Force,
        )
        .expect("force install");
        assert_eq!(outcome.kind, InstallOutcomeKind::ForceOverwrote);
        assert_eq!(outcome.source_hash, companion_bundle.source_hash);

        let dest_a = companion_bundle
            .project
            .kiro_dir()
            .join("agents/prompts/a.md");
        assert_eq!(fs::read(&dest_a).expect("read"), b"prompt v2");
    }

    #[rstest]
    fn install_native_companions_orphan_at_destination_fails_loudly(
        companion_bundle: CompanionBundle,
    ) {
        // Closes pr-test-analyzer C1: classify_companion_collision
        // raises OrphanFileAtDestination when a companion file exists
        // on disk with no plugin owning it. Mirrors install_native_agent's
        // orphan test for the companion path.
        let dest = companion_bundle
            .project
            .root
            .join(".kiro/agents/prompts/a.md");
        fs::create_dir_all(dest.parent().unwrap()).unwrap();
        fs::write(&dest, b"orphan").unwrap();

        let err = install_companions(
            &companion_bundle,
            "m",
            "p",
            crate::service::InstallMode::New,
        )
        .expect_err("orphan must fail without --force");
        assert!(matches!(err, AgentError::OrphanFileAtDestination { .. }));

        // --force overwrites the orphan and tracks ownership.
        let outcome = install_companions(
            &companion_bundle,
            "m",
            "p",
            crate::service::InstallMode::Force,
        )
        .expect("force install over orphan");
        assert_eq!(outcome.kind, InstallOutcomeKind::ForceOverwrote);
        assert_eq!(fs::read(&dest).unwrap(), b"prompt a");
    }

    #[rstest]
    fn install_native_companions_cross_plugin_overlap_fails_loudly(
        companion_bundle: CompanionBundle,
    ) {
        // plugin-a installs first; the dest path becomes plugin-a-owned.
        install_companions(
            &companion_bundle,
            "m",
            "plugin-a",
            crate::service::InstallMode::New,
        )
        .expect("plugin-a install");

        // plugin-b stages a different body at the SAME rel path. Without
        // --force, the path conflict must fail loudly with
        // PathOwnedByOtherPlugin.
        let scratch_b = companion_bundle.scratch.path().join("plugin-b-src");
        fs::create_dir_all(scratch_b.join("prompts")).expect("create");
        fs::write(scratch_b.join("prompts/a.md"), b"from-b").expect("write");
        let rel_paths_b = vec![PathBuf::from("prompts/a.md")];
        let h_b = crate::hash::hash_artifact(&scratch_b, &rel_paths_b).expect("hash b");

        let err = companion_bundle
            .project
            .install_native_companions(&NativeCompanionsInput {
                scan_root: &scratch_b,
                rel_paths: &rel_paths_b,
                marketplace: &mp("m"),
                plugin: &pn("plugin-b"),
                version: None,
                source_hash: &h_b,
                mode: crate::service::InstallMode::New,
                plugin_dir: companion_bundle.scratch.path(),
            })
            .expect_err("must refuse");
        match err {
            AgentError::PathOwnedByOtherPlugin { path, owner } => {
                assert!(path.ends_with("prompts/a.md"), "path: {}", path.display());
                assert_eq!(owner, "plugin-a");
            }
            other => panic!("expected PathOwnedByOtherPlugin, got {other:?}"),
        }

        // With --force: plugin-b takes ownership, plugin-a's tracking
        // entry loses the file (and is removed entirely since it had
        // only the one file).
        let outcome = companion_bundle
            .project
            .install_native_companions(&NativeCompanionsInput {
                scan_root: &scratch_b,
                rel_paths: &rel_paths_b,
                marketplace: &mp("m"),
                plugin: &pn("plugin-b"),
                version: None,
                source_hash: &h_b,
                mode: crate::service::InstallMode::Force,
                plugin_dir: companion_bundle.scratch.path(),
            })
            .expect("force transfer");
        assert_eq!(outcome.kind, InstallOutcomeKind::ForceOverwrote);

        let tracking = companion_bundle
            .project
            .load_installed_agents()
            .expect("load");
        assert!(
            !tracking.native_companions.contains_key("plugin-a"),
            "plugin-a's entry should be removed (its only file was transferred)"
        );
        assert!(
            tracking.native_companions.contains_key("plugin-b"),
            "plugin-b should now own the path"
        );

        let dest = companion_bundle
            .project
            .kiro_dir()
            .join("agents/prompts/a.md");
        assert_eq!(fs::read(&dest).expect("read installed"), b"from-b");
    }
}

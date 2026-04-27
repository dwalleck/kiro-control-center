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

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Metadata recorded for each installed skill.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledSkillMeta {
    /// Name of the marketplace the skill came from.
    pub marketplace: String,
    /// Name of the plugin that owns the skill.
    pub plugin: String,
    /// Optional version string from the plugin manifest.
    pub version: Option<String>,
    /// Timestamp when the skill was installed.
    pub installed_at: DateTime<Utc>,

    /// Tree-hash of the skill source as it existed in the marketplace at
    /// install time. `None` for entries written before Stage 1 of the
    /// native-kiro-import work landed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_hash: Option<String>,

    /// Tree-hash of the skill as it was copied into the project. `None`
    /// for entries written before Stage 1 landed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installed_hash: Option<String>,
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
    /// Name of the marketplace the agent came from.
    pub marketplace: String,
    /// Name of the plugin that owns the agent.
    pub plugin: String,
    /// Optional version string from the plugin manifest.
    pub version: Option<String>,
    /// Timestamp when the agent was installed.
    pub installed_at: DateTime<Utc>,
    /// Which source dialect the agent was parsed from. Persisted via the
    /// enum's serde rename so the wire format stays `"claude"` / `"copilot"`.
    pub dialect: AgentDialect,

    /// Tree-hash of the agent source as it existed in the marketplace at
    /// install time. `None` for entries written before Stage 1 of the
    /// native-kiro-import work landed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_hash: Option<String>,

    /// Tree-hash of the agent as it was copied into the project. `None`
    /// for entries written before Stage 1 landed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installed_hash: Option<String>,
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
    pub marketplace: String,
    pub plugin: String,
    pub version: Option<String>,
    pub installed_at: DateTime<Utc>,
    /// Relative paths under `.kiro/agents/` of every companion file owned
    /// by this plugin. Used for collision detection (cross-plugin path
    /// overlap) and for uninstall.
    pub files: Vec<PathBuf>,
    pub source_hash: String,
    pub installed_hash: String,
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
    pub marketplace: String,
    pub plugin: String,
    pub version: Option<String>,
    pub installed_at: DateTime<Utc>,
    pub source_hash: String,
    pub installed_hash: String,
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
/// [`crate::steering::InstalledSteeringOutcome`]).
enum CollisionDecision<T> {
    Idempotent(Box<T>),
    Proceed { forced_overwrite: bool },
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
    pub marketplace: &'a str,
    pub plugin: &'a str,
    pub version: Option<&'a str>,
    pub source_hash: &'a str,
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
    marketplace: &'a str,
    plugin: &'a str,
    version: Option<&'a str>,
    agents_root: &'a Path,
    prompt_rel: &'a Path,
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
    /// Deletes the skill directory and removes the entry from the tracking
    /// file.
    ///
    /// # Errors
    ///
    /// - [`SkillError::NotInstalled`] if the skill is not installed.
    /// - I/O or JSON serialisation errors.
    pub fn remove_skill(&self, name: &str) -> crate::error::Result<()> {
        validation::validate_name(name)?;

        crate::file_lock::with_file_lock(&self.tracking_path(), || -> crate::error::Result<()> {
            let dir = self.skill_dir(name);

            if !dir.exists() {
                return Err(SkillError::NotInstalled {
                    name: name.to_owned(),
                }
                .into());
            }

            // Update tracking BEFORE deleting the directory so a crash
            // between the two operations leaves the directory on disk
            // (harmless) rather than a phantom tracking entry (confusing).
            let mut installed = self.load_installed()?;
            let saved_meta = installed.skills.remove(name);
            self.write_tracking(&installed)?;

            if let Err(e) = fs::remove_dir_all(&dir) {
                // Directory delete failed after tracking was already updated.
                // Re-insert the entry so the tracking file stays consistent.
                warn!(
                    name,
                    error = %e,
                    "failed to delete skill directory after tracking update; \
                     restoring tracking entry"
                );
                if let Some(meta) = saved_meta {
                    installed.skills.insert(name.to_owned(), meta);
                    if let Err(restore_err) = self.write_tracking(&installed) {
                        warn!(
                            name,
                            error = %restore_err,
                            "failed to restore tracking entry — skill may be \
                             untracked on disk"
                        );
                    }
                } else {
                    debug!(
                        name,
                        "skill directory exists on disk but had no tracking \
                         entry; no restore needed"
                    );
                }
                return Err(e.into());
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
        // Refuse hardlinked sources before allocating the read. A hardlink
        // shares an inode with some other path that could be a sensitive
        // host file (`~/.ssh/id_rsa`); writing the inode's bytes into
        // `.kiro/steering/` would exfiltrate them. Discovery's
        // symlink/junction filter does not catch hardlinks (the share is
        // at the inode level, not the path). Windows hardlinks lack a
        // portable nlink accessor in std — platform.rs's reparse-point
        // check covers junctions, which is the analogous Windows risk.
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
        plugin: &str,
        source_hash: &str,
        dest: &Path,
        mode: crate::service::InstallMode,
    ) -> Result<
        CollisionDecision<crate::steering::InstalledSteeringOutcome>,
        crate::steering::SteeringError,
    > {
        match installed.files.get(rel_path) {
            Some(existing) if existing.plugin == plugin => {
                if existing.source_hash == source_hash {
                    return Ok(CollisionDecision::Idempotent(Box::new(
                        crate::steering::InstalledSteeringOutcome {
                            source: dest.to_path_buf(),
                            destination: dest.to_path_buf(),
                            kind: InstallOutcomeKind::Idempotent,
                            source_hash: source_hash.to_owned(),
                            installed_hash: existing.installed_hash.clone(),
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
                crate::steering::SteeringError::TrackingMalformed {
                    path: tracking_path,
                    source: json_err,
                }
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
                crate::steering::SteeringError::TrackingMalformed {
                    path: tracking_path.clone(),
                    source: json_err,
                }
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
            CollisionDecision::Idempotent(outcome) => return Ok(*outcome),
            CollisionDecision::Proceed { forced_overwrite } => forced_overwrite,
        };

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
            && existing.plugin != ctx.plugin
        {
            installed.files.remove(rel_path);
        }

        installed.files.insert(
            rel_path.to_path_buf(),
            InstalledSteeringMeta {
                marketplace: ctx.marketplace.to_owned(),
                plugin: ctx.plugin.to_owned(),
                version: ctx.version.map(str::to_owned),
                installed_at: chrono::Utc::now(),
                source_hash: source_hash.to_owned(),
                installed_hash: installed_hash.clone(),
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
    /// thin air); otherwise the blake3-prefixed hash. Lifted out of
    /// `install_agent_inner` to keep that function under the line cap.
    fn hash_translated_source(source_path: Option<&Path>) -> crate::error::Result<Option<String>> {
        let Some(p) = source_path else {
            return Ok(None);
        };
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
        Ok(Some(crate::hash::hash_artifact(
            parent,
            &[std::path::PathBuf::from(filename)],
        )?))
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
                meta.installed_hash = Some(installed_hash);

                // Capture plugin identity before moving meta into the map.
                let marketplace = meta.marketplace.clone();
                let plugin = meta.plugin.clone();
                let version = meta.version.clone();

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
        let companion_entry = installed
            .native_companions
            .entry(input.plugin.to_owned())
            .or_insert_with(|| InstalledNativeCompanionsMeta {
                marketplace: input.marketplace.to_owned(),
                plugin: input.plugin.to_owned(),
                version: input.version.map(str::to_owned),
                installed_at: chrono::Utc::now(),
                files: Vec::new(),
                source_hash: String::new(),
                installed_hash: String::new(),
            });
        // Refresh marketplace/version/timestamp on every install.
        input
            .marketplace
            .clone_into(&mut companion_entry.marketplace);
        companion_entry.version = input.version.map(str::to_owned);
        companion_entry.installed_at = chrono::Utc::now();
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
        plugin: &str,
        source_hash: &str,
        json_target: &Path,
        mode: crate::service::InstallMode,
    ) -> crate::error::Result<CollisionDecision<InstalledNativeAgentOutcome>> {
        match installed.agents.get(agent_name) {
            Some(existing) if existing.plugin == plugin => {
                if existing.source_hash.as_deref() == Some(source_hash) {
                    return Ok(CollisionDecision::Idempotent(Box::new(
                        InstalledNativeAgentOutcome {
                            name: agent_name.to_owned(),
                            json_path: json_target.to_path_buf(),
                            kind: InstallOutcomeKind::Idempotent,
                            source_hash: source_hash.to_owned(),
                            installed_hash: existing.installed_hash.clone().unwrap_or_default(),
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
                        owner: existing.plugin.clone(),
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
        bundle: &crate::agent::NativeAgentBundle,
        marketplace: &str,
        plugin: &str,
        version: Option<&str>,
        source_hash: &str,
        mode: crate::service::InstallMode,
    ) -> Result<InstalledNativeAgentOutcome, AgentError> {
        let json_target = self.agents_dir().join(format!("{}.json", &bundle.name));
        let agent_name = bundle.name.clone();
        let json_target_for_err = json_target.clone();

        let result: crate::error::Result<InstalledNativeAgentOutcome> =
            crate::file_lock::with_file_lock(&self.agent_tracking_path(), || {
                let mut installed = self.load_installed_agents()?;

                // Collision matrix — return early or set `forced_overwrite`.
                let forced_overwrite = match Self::classify_native_collision(
                    &installed,
                    &agent_name,
                    plugin,
                    source_hash,
                    &json_target,
                    mode,
                )? {
                    CollisionDecision::Idempotent(outcome) => return Ok(*outcome),
                    CollisionDecision::Proceed { forced_overwrite } => forced_overwrite,
                };

                let (staging, json_rel, installed_hash) =
                    self.stage_native_agent_file(&agent_name, &bundle.raw_bytes)?;

                let had_backup = self.promote_native_agent(
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
                        marketplace: marketplace.to_string(),
                        plugin: plugin.to_string(),
                        version: version.map(String::from),
                        installed_at: chrono::Utc::now(),
                        dialect: AgentDialect::Native,
                        source_hash: Some(source_hash.to_string()),
                        installed_hash: Some(installed_hash.clone()),
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
                    if had_backup {
                        let backup = json_target.with_extension("json.kiro-bak");
                        if let Err(restore_err) = fs::rename(&backup, &json_target) {
                            warn!(
                                backup = %backup.display(),
                                target = %json_target.display(),
                                error = %restore_err,
                                "failed to restore backup after tracking write failure — \
                                 user may need to rename .kiro-bak file manually"
                            );
                        }
                    }
                    return Err(e);
                }

                // Success — drop the backup file. Best-effort; an orphan
                // .kiro-bak left here is a curiosity, not a correctness issue.
                if had_backup {
                    let backup = json_target.with_extension("json.kiro-bak");
                    if let Err(e) = fs::remove_file(&backup)
                        && e.kind() != std::io::ErrorKind::NotFound
                    {
                        warn!(
                            path = %backup.display(),
                            error = %e,
                            "failed to remove install backup after success"
                        );
                    }
                }

                debug!(name = %agent_name, force = mode.is_force(), "native agent installed");

                Ok(InstalledNativeAgentOutcome {
                    name: agent_name,
                    json_path: json_target,
                    kind: if forced_overwrite {
                        InstallOutcomeKind::ForceOverwrote
                    } else {
                        InstallOutcomeKind::Installed
                    },
                    source_hash: source_hash.to_string(),
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
    /// is set. Returns `had_backup` so the caller can restore on tracking
    /// failure or drop the backup on success.
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
    ) -> crate::error::Result<bool> {
        let staging_json = staging.join(json_rel);

        fs::create_dir_all(self.agents_dir())?;

        // Backup phase — only when overwriting an existing file.
        let backup_target = json_target.with_extension("json.kiro-bak");
        let mut had_backup = false;
        if forced_overwrite && json_target.exists() {
            fs::rename(json_target, &backup_target)?;
            had_backup = true;
        }

        // Promote phase.
        if let Err(e) = fs::rename(&staging_json, json_target) {
            // Restore backup if we made one.
            if had_backup && let Err(restore_err) = fs::rename(&backup_target, json_target) {
                warn!(
                    backup = %backup_target.display(),
                    target = %json_target.display(),
                    error = %restore_err,
                    "failed to restore backup after rename failure"
                );
            }
            return Err(e.into());
        }
        Ok(had_backup)
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

        let (staging, installed_hash) =
            self.stage_native_companion_files(input.plugin, input.scan_root, input.rel_paths)?;

        let CompanionPromotion { placed, backups } = self.promote_native_companions(
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
            input.plugin.to_string(),
            InstalledNativeCompanionsMeta {
                marketplace: input.marketplace.to_string(),
                plugin: input.plugin.to_string(),
                version: input.version.map(String::from),
                installed_at: chrono::Utc::now(),
                files: input.rel_paths.to_vec(),
                source_hash: input.source_hash.to_string(),
                installed_hash: installed_hash.clone(),
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
        if let Some(existing) = installed.native_companions.get(input.plugin) {
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
                if other_plugin == input.plugin {
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
        plugin: &str,
        scan_root: &Path,
        rel_paths: &[PathBuf],
    ) -> crate::error::Result<(tempfile::TempDir, String)> {
        let staging = tempfile::Builder::new()
            .prefix(&format!("_installing-companions-{plugin}-"))
            .tempdir_in(self.kiro_dir())?;

        for rel in rel_paths {
            let src = scan_root.join(rel);
            // Refuse hardlinked sources before fs::copy. Same threat
            // model as stage_steering_file: a hardlink shares an inode
            // with another path that could be sensitive (`~/.ssh/id_rsa`).
            // Discovery's symlink/junction filter doesn't catch this.
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                let md = fs::symlink_metadata(&src)?;
                if md.is_file() && md.nlink() > 1 {
                    return Err(AgentError::InstallFailed {
                        path: src.clone(),
                        source: Box::new(crate::error::Error::Io(std::io::Error::other(format!(
                            "refusing hardlinked native companion at {} (nlink={})",
                            src.display(),
                            md.nlink()
                        )))),
                    }
                    .into());
                }
            }
            let dest = staging.path().join(rel);
            if let Some(parent) = dest.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(&src, &dest)?;
        }

        let installed_hash = match crate::hash::hash_artifact(staging.path(), rel_paths) {
            Ok(h) => h,
            Err(e) => {
                warn!(
                    plugin,
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
        &self,
        staging: &Path,
        rel_paths: &[PathBuf],
        agents_dir: &Path,
        forced_overwrite: bool,
    ) -> crate::error::Result<CompanionPromotion> {
        let _ = self;
        let mut placed: Vec<PathBuf> = Vec::with_capacity(rel_paths.len());
        let mut backups: Vec<(PathBuf, PathBuf)> = Vec::new();

        for rel in rel_paths {
            let src = staging.join(rel);
            let dest = agents_dir.join(rel);
            if let Some(parent) = dest.parent()
                && let Err(e) = fs::create_dir_all(parent)
            {
                Self::rollback_companion_promotion(&placed, &backups);
                return Err(e.into());
            }
            // Backup the existing destination if we'll overwrite it.
            if forced_overwrite && dest.exists() {
                let backup = Self::companion_backup_path(&dest);
                if let Err(e) = fs::rename(&dest, &backup) {
                    Self::rollback_companion_promotion(&placed, &backups);
                    return Err(e.into());
                }
                backups.push((dest.clone(), backup));
            }
            if let Err(e) = fs::rename(&src, &dest) {
                Self::rollback_companion_promotion(&placed, &backups);
                return Err(e.into());
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
    /// owns, since the original source bundle is no longer accessible.
    ///
    /// # Errors
    ///
    /// Returns the hash error if recomputing any modified plugin's
    /// hash fails. Caller is responsible for rolling back the file
    /// promotion since this happens AFTER promote.
    fn strip_transferred_paths_from_other_plugins(
        installed: &mut InstalledAgents,
        plugin: &str,
        rel_paths: &[PathBuf],
        agents_dir: &Path,
    ) -> crate::error::Result<()> {
        let new_set: std::collections::HashSet<&PathBuf> = rel_paths.iter().collect();
        let other_plugins: Vec<String> = installed
            .native_companions
            .keys()
            .filter(|p| p.as_str() != plugin)
            .cloned()
            .collect();
        let mut modified: Vec<String> = Vec::new();
        for p in other_plugins {
            if let Some(meta) = installed.native_companions.get_mut(&p) {
                let len_before = meta.files.len();
                meta.files.retain(|f| !new_set.contains(f));
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
        plugin: &str,
        rel_paths: &[PathBuf],
    ) -> Vec<PathBuf> {
        let Some(prior) = installed.native_companions.get(plugin) else {
            return Vec::new();
        };
        let new_set: std::collections::HashSet<&PathBuf> = rel_paths.iter().collect();
        prior
            .files
            .iter()
            .filter(|f| !new_set.contains(*f))
            .cloned()
            .collect()
    }

    /// Best-effort removal of prior-install companion files that have
    /// dropped out of the new tracking entry. Failures are logged but
    /// don't propagate — the file set is already canonical in
    /// tracking, and a stray on-disk file is strictly less harmful
    /// than rolling back a successful install.
    fn remove_companion_files_best_effort(rel_paths: &[PathBuf], agents_dir: &Path, plugin: &str) {
        for rel in rel_paths {
            let abs = agents_dir.join(rel);
            if let Err(e) = fs::remove_file(&abs)
                && e.kind() != std::io::ErrorKind::NotFound
            {
                warn!(
                    plugin,
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
            meta.source_hash = Some(source_hash);
            meta.installed_hash = Some(installed_hash);

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

    fn temp_project() -> (tempfile::TempDir, KiroProject) {
        let dir = tempfile::tempdir().expect("tempdir");
        let project = KiroProject::new(dir.path().to_path_buf());
        (dir, project)
    }

    fn sample_meta() -> InstalledSkillMeta {
        InstalledSkillMeta {
            marketplace: "test-market".into(),
            plugin: "test-plugin".into(),
            version: Some("1.0.0".into()),
            installed_at: Utc::now(),
            source_hash: None,
            installed_hash: None,
        }
    }

    #[test]
    fn installed_agent_meta_roundtrips_json() {
        let meta = InstalledAgentMeta {
            marketplace: "mp".into(),
            plugin: "pr-review-toolkit".into(),
            version: Some("1.2.3".into()),
            installed_at: Utc::now(),
            dialect: AgentDialect::Claude,
            source_hash: None,
            installed_hash: None,
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
            marketplace: "mp".into(),
            plugin: "p".into(),
            version: None,
            installed_at: Utc::now(),
            dialect: AgentDialect::Copilot,
            source_hash: None,
            installed_hash: None,
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
                marketplace: "m".into(),
                plugin: "p".into(),
                version: Some("0.1.0".into()),
                installed_at: chrono::Utc::now(),
                source_hash: "blake3:abc".into(),
                installed_hash: "blake3:abc".into(),
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
    fn load_installed_steering_round_trips_through_disk() {
        let (_dir, project) = temp_project();

        let mut to_save = InstalledSteering::default();
        to_save.files.insert(
            PathBuf::from("guide.md"),
            InstalledSteeringMeta {
                marketplace: "m".into(),
                plugin: "p".into(),
                version: None,
                installed_at: chrono::Utc::now(),
                source_hash: "blake3:abc".into(),
                installed_hash: "blake3:abc".into(),
            },
        );
        project.write_steering_tracking(&to_save).unwrap();

        let loaded = project.load_installed_steering().unwrap();
        assert_eq!(loaded.files.len(), 1);
        assert!(loaded.files.contains_key(std::path::Path::new("guide.md")));
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
        f.project.install_steering_file(
            &discovered,
            &f.source_hash,
            crate::steering::SteeringInstallContext {
                mode,
                marketplace: "m",
                plugin,
                version: None,
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

        let err = steering_file
            .project
            .install_steering_file(
                &discovered_b,
                &source_hash_b,
                crate::steering::SteeringInstallContext {
                    mode: crate::service::InstallMode::New,
                    marketplace: "m",
                    plugin: "plugin-b",
                    version: None,
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
                    marketplace: "m",
                    plugin: "plugin-b",
                    version: None,
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

        let err = steering_file
            .project
            .install_steering_file(
                &discovered,
                &source_hash,
                crate::steering::SteeringInstallContext {
                    mode: crate::service::InstallMode::New,
                    marketplace: "m",
                    plugin: "p",
                    version: None,
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

        let outcome = project
            .install_steering_file(
                &discovered,
                &source_hash,
                crate::steering::SteeringInstallContext {
                    mode: crate::service::InstallMode::New,
                    marketplace: "marketplace-x",
                    plugin: "plugin-y",
                    version: Some("0.1.0"),
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
            marketplace: "mp".into(),
            plugin: "p".into(),
            version: None,
            installed_at: Utc::now(),
            dialect: AgentDialect::Claude,
            source_hash: None,
            installed_hash: None,
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
        "plugin-a".clone_into(&mut meta_a.plugin);
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
        "plugin-b".clone_into(&mut meta_b.plugin);
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
        project
            .install_native_companions(&NativeCompanionsInput {
                scan_root: &scan_a,
                rel_paths: &rel_paths_a,
                marketplace: "m",
                plugin: "plugin-a",
                version: None,
                source_hash: &h_a,
                mode: crate::service::InstallMode::New,
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
        project
            .install_native_companions(&NativeCompanionsInput {
                scan_root: &scan_b,
                rel_paths: &rel_paths_b,
                marketplace: "m",
                plugin: "plugin-b",
                version: None,
                source_hash: &h_b,
                mode: crate::service::InstallMode::Force,
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

    #[test]
    fn installed_skill_meta_loads_legacy_json_without_hash_fields() {
        // Old tracking files (pre-Stage-1) lack source_hash / installed_hash.
        // The new schema must deserialize them with both fields = None.
        let legacy = br#"{
            "marketplace": "m",
            "plugin": "p",
            "version": "1.0.0",
            "installed_at": "2026-01-01T00:00:00Z"
        }"#;

        let meta: InstalledSkillMeta = serde_json::from_slice(legacy).unwrap();

        assert_eq!(meta.marketplace, "m");
        assert_eq!(meta.plugin, "p");
        assert!(meta.source_hash.is_none());
        assert!(meta.installed_hash.is_none());
    }

    #[test]
    fn installed_agent_meta_loads_legacy_json_without_hash_fields() {
        let legacy = br#"{
            "marketplace": "m",
            "plugin": "p",
            "version": "0.1.0",
            "installed_at": "2026-01-01T00:00:00Z",
            "dialect": "claude"
        }"#;

        let meta: InstalledAgentMeta = serde_json::from_slice(legacy).unwrap();

        assert_eq!(meta.dialect, AgentDialect::Claude);
        assert!(meta.source_hash.is_none());
        assert!(meta.installed_hash.is_none());
    }

    #[test]
    fn installed_agents_loads_legacy_json_without_native_companions() {
        // Old tracking files (pre-Stage-1) lack the native_companions map.
        // The new schema must deserialize them with native_companions = empty.
        let legacy = br#"{
            "agents": {
                "x": {
                    "marketplace": "m",
                    "plugin": "p",
                    "version": null,
                    "installed_at": "2026-01-01T00:00:00Z",
                    "dialect": "claude"
                }
            }
        }"#;

        let installed: InstalledAgents = serde_json::from_slice(legacy).unwrap();
        assert_eq!(installed.agents.len(), 1);
        assert!(installed.native_companions.is_empty());
    }

    #[test]
    fn installed_native_companions_meta_round_trips_through_serde() {
        let meta = InstalledNativeCompanionsMeta {
            marketplace: "m".into(),
            plugin: "p".into(),
            version: Some("0.1.0".into()),
            installed_at: chrono::Utc::now(),
            files: vec![
                std::path::PathBuf::from("prompts/a.md"),
                std::path::PathBuf::from("prompts/b.md"),
            ],
            source_hash: "blake3:abc".into(),
            installed_hash: "blake3:abc".into(),
        };
        let bytes = serde_json::to_vec(&meta).unwrap();
        let back: InstalledNativeCompanionsMeta = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(back.files.len(), 2);
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
            marketplace: "m".into(),
            plugin: "p".into(),
            version: Some("1.0.0".into()),
            installed_at: chrono::Utc::now(),
            source_hash: None,
            installed_hash: None,
        };

        project
            .install_skill_from_dir("test", &skill_src, meta)
            .unwrap();

        let installed = project.load_installed().unwrap();
        let entry = installed.skills.get("test").expect("entry persisted");

        let src_hash = entry.source_hash.as_ref().expect("source_hash populated");
        let inst_hash = entry
            .installed_hash
            .as_ref()
            .expect("installed_hash populated");

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
        meta.source_hash = None;
        meta.installed_hash = None;
        let plugin_name = meta.plugin.clone();

        project
            .install_agent(&def, &mapped, meta, Some(&source_md))
            .expect("install succeeds");

        let installed = project.load_installed_agents().unwrap();
        let entry = installed.agents.get("rev").expect("entry persisted");

        let src = entry.source_hash.as_ref().expect("source_hash set");
        let inst = entry.installed_hash.as_ref().expect("installed_hash set");
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
            .get(&plugin_name)
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
            meta.source_hash = None;
            meta.installed_hash = None;
            project
                .install_agent(&def, &[], meta, Some(&source_md))
                .expect("install succeeds");
        }

        let installed = project.load_installed_agents().unwrap();
        let companion = installed
            .native_companions
            .get(&plugin_name)
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
        f.project
            .install_native_agent(&f.bundle, marketplace, plugin, None, &f.source_hash, mode)
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
            .install_native_agent(
                &bundle,
                "marketplace-x",
                "plugin-y",
                Some("0.1.0"),
                &source_hash,
                crate::service::InstallMode::New,
            )
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
        assert_eq!(entry.source_hash.as_deref(), Some(source_hash.as_str()));
        assert_eq!(
            entry.installed_hash.as_deref(),
            Some(outcome.installed_hash.as_str())
        );
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
            .install_native_agent(
                &bundle,
                "m",
                "p",
                None,
                &source_hash,
                crate::service::InstallMode::New,
            )
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
            .install_native_agent(
                &bundle_v1,
                "m",
                "p",
                None,
                &h_v1,
                crate::service::InstallMode::New,
            )
            .expect("v1 install");

        let dest = project.root.join(".kiro/agents/rev.json");
        let v1_bytes = fs::read(&dest).unwrap();
        assert_eq!(v1_bytes.as_slice(), body_v1.as_slice());

        // Poison tracking so write fails: replace with a directory.
        let tracking_path = project.root.join(".kiro/installed-agents.json");
        fs::remove_file(&tracking_path).unwrap();
        fs::create_dir_all(&tracking_path).unwrap();

        // Force-install v2 (stage_native_source overwrites the source).
        let body_v2 = br#"{"name":"rev","prompt":"v2"}"#;
        let (bundle_v2, _, _) = stage_native_source(scratch.path(), "rev", body_v2);
        let h_v2 =
            crate::hash::hash_artifact(&src_dir, &[std::path::PathBuf::from("rev.json")]).unwrap();
        let err = project
            .install_native_agent(
                &bundle_v2,
                "m",
                "p",
                None,
                &h_v2,
                crate::service::InstallMode::Force,
            )
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
        project
            .install_steering_file(
                &discovered,
                &h_v1,
                crate::steering::SteeringInstallContext {
                    mode: crate::service::InstallMode::New,
                    marketplace: "m",
                    plugin: "p",
                    version: None,
                },
            )
            .expect("v1 install");

        let dest = project.root.join(".kiro/steering/guide.md");
        assert_eq!(fs::read(&dest).unwrap(), b"v1");

        // Poison the steering tracking path.
        let tracking_path = project.root.join(".kiro/installed-steering.json");
        fs::remove_file(&tracking_path).unwrap();
        fs::create_dir_all(&tracking_path).unwrap();

        // Force-install v2.
        fs::write(scan_root.join("guide.md"), b"v2").unwrap();
        let h_v2 = crate::hash::hash_artifact(&scan_root, &[PathBuf::from("guide.md")]).unwrap();
        let err = project
            .install_steering_file(
                &discovered,
                &h_v2,
                crate::steering::SteeringInstallContext {
                    mode: crate::service::InstallMode::Force,
                    marketplace: "m",
                    plugin: "p",
                    version: None,
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

        let outcome = project
            .install_native_companions(&NativeCompanionsInput {
                scan_root: &scan_root,
                rel_paths: &rel_paths,
                marketplace: "marketplace-x",
                plugin: "plugin-y",
                version: Some("0.1.0"),
                source_hash: &source_hash,
                mode: crate::service::InstallMode::New,
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
                marketplace: "m",
                plugin: "p",
                version: None,
                source_hash: "blake3:empty",
                mode: crate::service::InstallMode::New,
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

        // The hardlink target lives outside the plugin tree to model
        // the "exfil sensitive host file" threat. Both paths point at
        // the same inode.
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
                marketplace: "m",
                plugin: "p",
                version: None,
                source_hash: &h,
                mode: crate::service::InstallMode::New,
            })
            .expect_err("hardlinked source must be refused");
        let msg = err.to_string();
        assert!(
            msg.contains("hardlinked"),
            "expected hardlinked-rejection error, got: {msg}"
        );

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
                marketplace: "m",
                plugin: "p",
                version: None,
                source_hash: &h_v1,
                mode: crate::service::InstallMode::New,
            })
            .expect("v1 install");

        let dest_a = project.root.join(".kiro/agents/prompts/a.md");
        let dest_b = project.root.join(".kiro/agents/prompts/b.md");
        assert!(dest_a.exists() && dest_b.exists());

        // Poison tracking so write_agent_tracking fails: replace the
        // tracking file with a directory of the same name.
        let tracking_path = project.root.join(".kiro/installed-agents.json");
        fs::remove_file(&tracking_path).unwrap();
        fs::create_dir_all(&tracking_path).unwrap();

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
                marketplace: "m",
                plugin: "p",
                version: None,
                source_hash: &h_v2,
                mode: crate::service::InstallMode::Force,
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
        f.project.install_native_companions(&NativeCompanionsInput {
            scan_root: &f.scan_root,
            rel_paths: &f.rel_paths,
            marketplace,
            plugin,
            version: None,
            source_hash: &f.source_hash,
            mode,
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
                marketplace: "m",
                plugin: "plugin-b",
                version: None,
                source_hash: &h_b,
                mode: crate::service::InstallMode::New,
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
                marketplace: "m",
                plugin: "plugin-b",
                version: None,
                source_hash: &h_b,
                mode: crate::service::InstallMode::Force,
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

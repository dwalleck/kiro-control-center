//! Kiro project state management.
//!
//! Manages the `.kiro/skills/` directory layout and an
//! `installed-skills.json` tracking file that records which skills have been
//! installed, from which marketplace and plugin, and when.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicU64;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

use crate::agent::tools::MappedTool;
use crate::agent::{AgentDefinition, AgentDialect};
use crate::error::{AgentError, SkillError};
use crate::validation;

/// Process-local sequence used to disambiguate concurrent staging directories.
/// Combined with `process::id()` to guarantee unique paths even when two
/// threads in the same process race past the file lock.
static STAGING_COUNTER: AtomicU64 = AtomicU64::new(0);

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

// ---------------------------------------------------------------------------
// KiroProject
// ---------------------------------------------------------------------------

/// Name of the skill tracking file inside `.kiro/`.
const INSTALLED_SKILLS_FILE: &str = "installed-skills.json";

/// Name of the agent tracking file inside `.kiro/`.
const INSTALLED_AGENTS_FILE: &str = "installed-agents.json";

/// Best-effort removal of a staging directory, logging on failure instead
/// of propagating the error. Used in rollback paths where the caller is
/// already returning a more meaningful error and the staging dir is
/// unreachable user-facing state.
fn remove_staging_dir(staging: &Path) {
    if let Err(e) = fs::remove_dir_all(staging)
        && e.kind() != std::io::ErrorKind::NotFound
    {
        warn!(
            path = %staging.display(),
            error = %e,
            "failed to remove staging directory"
        );
    }
}

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
/// 7 immutable refs that the helper needs so the public-ish signature stays
/// at two parameters (the `&mut InstalledAgents` plus the bundle), avoiding
/// a `#[allow(clippy::too_many_arguments)]` waiver that would otherwise be
/// required.
struct CompanionInput<'a> {
    marketplace: &'a str,
    plugin: &'a str,
    version: Option<&'a str>,
    agents_root: &'a Path,
    prompt_rel: &'a Path,
    /// Final destination of the agent JSON; used by the rollback path on
    /// companion-hash failure to remove just-renamed files.
    json_target: &'a Path,
    /// Final destination of the agent prompt body; used by the rollback path
    /// on companion-hash failure.
    prompt_target: &'a Path,
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

    /// Generate a per-attempt staging directory path for an agent install.
    fn fresh_agent_staging_dir(&self, name: &str) -> PathBuf {
        use std::sync::atomic::Ordering;
        let pid = std::process::id();
        let seq = STAGING_COUNTER.fetch_add(1, Ordering::Relaxed);
        self.kiro_dir()
            .join(format!("_installing-agent-{name}-{pid}-{seq}"))
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
        let source_hash: Option<String> = source_path
            .map(|p| -> crate::error::Result<String> {
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
            })
            .transpose()?;

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

                // Sweep leftover staging dirs for THIS agent from prior crashed
                // attempts. Safe because we hold the lock — no other installer
                // of this agent is currently running.
                self.cleanup_leftover_agent_staging(&def.name)?;

                let (staging, json_rel, prompt_rel, installed_hash) =
                    self.stage_agent_files(&def.name, &json_bytes, def.prompt_body.as_bytes())?;

                let (json_target, prompt_target) =
                    self.promote_staged_agent(&def.name, &staging, &json_rel, &prompt_rel, force)?;

                // installed_hash was computed pre-destructive (against staging).
                let agents_root = self.agents_dir(); // needed for companion hash below

                meta.source_hash = source_hash;
                meta.installed_hash = Some(installed_hash);

                // Capture plugin identity before moving meta into the map.
                let marketplace = meta.marketplace.clone();
                let plugin = meta.plugin.clone();
                let version = meta.version.clone();

                installed.agents.insert(def.name.clone(), meta);

                Self::synthesize_companion_entry(
                    &mut installed,
                    &CompanionInput {
                        marketplace: &marketplace,
                        plugin: &plugin,
                        version: version.as_deref(),
                        agents_root: &agents_root,
                        prompt_rel: &prompt_rel,
                        json_target: &json_target,
                        prompt_target: &prompt_target,
                    },
                )?;

                if let Err(e) = self.write_agent_tracking(&installed) {
                    warn!(
                        name = %def.name,
                        error = %e,
                        "agent tracking update failed after rename; rolling back files"
                    );
                    if let Err(rb_err) = fs::remove_file(&json_target) {
                        warn!(
                            path = %json_target.display(),
                            error = %rb_err,
                            "failed to roll back agent JSON after tracking failure — \
                             agent is on disk but not tracked"
                        );
                    }
                    if let Err(rb_err) = fs::remove_file(&prompt_target) {
                        warn!(
                            path = %prompt_target.display(),
                            error = %rb_err,
                            "failed to roll back agent prompt after tracking failure"
                        );
                    }
                    return Err(e);
                }

                debug!(name = %def.name, force, "agent installed");
                Ok(())
            },
        )
    }

    /// Move staged agent files from `staging` into their final locations under
    /// `agents_root`. In force mode, existing targets are unlinked first. In
    /// non-force mode, any pre-existing target file (e.g. from a prior crash)
    /// causes an `AlreadyExists` error without touching `agents_root`.
    ///
    /// On any error, `staging` is cleaned up before returning. JSON is renamed
    /// first; if the prompt rename then fails, the JSON rename is rolled back so
    /// neither target is left half-populated.
    ///
    /// Returns `(json_target, prompt_target)` on success.
    fn promote_staged_agent(
        &self,
        name: &str,
        staging: &Path,
        json_rel: &Path,
        prompt_rel: &Path,
        force: bool,
    ) -> crate::error::Result<(PathBuf, PathBuf)> {
        let staging_json = staging.join(json_rel);
        let staging_prompt = staging.join(prompt_rel);

        fs::create_dir_all(self.agent_prompts_dir())?;

        let json_target = self.agents_dir().join(format!("{name}.json"));
        let prompt_target = self.agent_prompts_dir().join(format!("{name}.md"));

        if force {
            // Remove existing targets before rename. Required for Windows
            // (rename fails on existing dest) and makes the Unix path
            // explicit rather than relying on rename's replace-on-Unix
            // behaviour. Missing-file is fine.
            for p in [&json_target, &prompt_target] {
                if let Err(e) = fs::remove_file(p)
                    && e.kind() != std::io::ErrorKind::NotFound
                {
                    remove_staging_dir(staging);
                    return Err(e.into());
                }
            }
        } else if json_target.exists() || prompt_target.exists() {
            // Non-force install: a prior crash could leave orphaned files
            // on disk without a tracking entry. Refuse to silently clobber
            // — the user either manually cleans up or re-invokes with
            // `install_agent_force`.
            remove_staging_dir(staging);
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

        // Rename JSON first. If the prompt rename fails afterwards, roll
        // back the JSON rename so we never leave an agent with only half
        // its files on disk.
        if let Err(e) = fs::rename(&staging_json, &json_target) {
            remove_staging_dir(staging);
            return Err(e.into());
        }
        if let Err(e) = fs::rename(&staging_prompt, &prompt_target) {
            if let Err(rb_err) = fs::remove_file(&json_target) {
                warn!(
                    path = %json_target.display(),
                    error = %rb_err,
                    "failed to roll back agent JSON after prompt-rename failure"
                );
            }
            remove_staging_dir(staging);
            return Err(e.into());
        }

        // Staging directory should now be empty (or contain the empty prompts subdir).
        remove_staging_dir(staging);
        Ok((json_target, prompt_target))
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
    ) -> crate::error::Result<(PathBuf, PathBuf, PathBuf, String)> {
        let staging = self.fresh_agent_staging_dir(name);
        let json_rel = PathBuf::from(format!("{name}.json"));
        let prompt_rel = PathBuf::from(format!("prompts/{name}.md"));
        let staging_json = staging.join(&json_rel);
        let staging_prompt_dir = staging.join("prompts");
        let staging_prompt = staging.join(&prompt_rel);

        fs::create_dir_all(&staging_prompt_dir)?;
        if let Err(e) = fs::write(&staging_json, json_bytes)
            .and_then(|()| fs::write(&staging_prompt, prompt_bytes))
        {
            remove_staging_dir(&staging);
            return Err(e.into());
        }

        let installed_hash =
            match crate::hash::hash_artifact(&staging, &[json_rel.clone(), prompt_rel.clone()]) {
                Ok(h) => h,
                Err(e) => {
                    warn!(
                        name,
                        error = %e,
                        "installed_hash computation failed on staging; removing staging dir"
                    );
                    remove_staging_dir(&staging);
                    return Err(e.into());
                }
            };

        Ok((staging, json_rel, prompt_rel, installed_hash))
    }

    /// Synthesize/update the per-plugin `native_companions` tracking entry
    /// to register this agent's prompt file as plugin-owned. Called from
    /// the translated agent install path; the native install path
    /// will call this with its own companion bundle.
    ///
    /// Recomputes the per-plugin companion hash over the full union of
    /// prompt files for this plugin. On hash failure, rolls back the
    /// just-placed json/prompt files.
    ///
    /// # Residual risk (force mode)
    /// The companion hash runs post-rename because it must hash prompt files
    /// from prior installs that live at their real `agents_root` locations.
    /// A hash failure in force mode will try to remove the newly placed files,
    /// but the previously existing files were already unlinked. A full
    /// backup-then-swap atomic install is deferred to a follow-up PR.
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
            match crate::hash::hash_artifact(input.agents_root, &companion_files_snapshot) {
                Ok(h) => h,
                Err(e) => {
                    warn!(
                        plugin = input.plugin,
                        error = %e,
                        "companion hash computation failed; rolling back files"
                    );
                    if let Err(rb_err) = fs::remove_file(input.json_target) {
                        warn!(
                            path = %input.json_target.display(),
                            error = %rb_err,
                            "failed to roll back agent JSON after companion-hash failure"
                        );
                    }
                    if let Err(rb_err) = fs::remove_file(input.prompt_target) {
                        warn!(
                            path = %input.prompt_target.display(),
                            error = %rb_err,
                            "failed to roll back agent prompt after companion-hash failure"
                        );
                    }
                    return Err(e.into());
                }
            };
        companion_entry.source_hash = companion_hash.clone();
        companion_entry.installed_hash = companion_hash;
        Ok(())
    }

    /// Remove any `_installing-agent-<name>-*` staging directories left over
    /// from prior crashed installs. Caller must hold the agent tracking lock.
    ///
    /// Best-effort: per-entry iteration errors are logged via `warn!` and
    /// skipped rather than aborting the install. A transient filesystem
    /// glitch reading one entry under `.kiro/` should not prevent the
    /// install — the staging dir is unreachable user-facing state, and
    /// the subsequent `fresh_agent_staging_dir` uses a unique per-attempt
    /// path regardless of whether cleanup fully succeeded.
    fn cleanup_leftover_agent_staging(&self, name: &str) -> std::io::Result<()> {
        let prefix = format!("_installing-agent-{name}-");
        let kiro_dir = self.kiro_dir();
        let entries = match fs::read_dir(&kiro_dir) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e),
        };
        for entry in entries {
            let entry = match entry {
                Ok(e) => e,
                Err(e) => {
                    warn!(
                        dir = %kiro_dir.display(),
                        error = %e,
                        "failed to read entry during agent staging cleanup; skipping"
                    );
                    continue;
                }
            };
            let file_name = entry.file_name();
            let Some(name_str) = file_name.to_str() else {
                continue;
            };
            if !name_str.starts_with(&prefix) {
                continue;
            }
            let path = entry.path();
            debug!(
                path = %path.display(),
                "removing leftover agent staging directory from prior install"
            );
            if let Err(e) = fs::remove_dir_all(&path) {
                warn!(
                    path = %path.display(),
                    error = %e,
                    "failed to remove leftover agent staging directory"
                );
            }
        }
        Ok(())
    }

    // -- internal helpers --------------------------------------------------

    /// Copy a source skill directory and update tracking.
    ///
    /// The entire flow — existence check, staging copy, rename, and tracking
    /// update — runs under a single advisory lock on the tracking file so
    /// two concurrent installs of the same skill name cannot both pass the
    /// existence check and clobber each other's staging directory.
    ///
    /// Per-attempt staging directory naming (`_installing-<name>-<pid>-<seq>`)
    /// provides defense-in-depth against impossible races and ensures two
    /// threads in the same process always have distinct staging paths.
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

            // Sweep any leftover staging dirs for THIS skill from prior
            // crashed attempts. Safe because we hold the lock — no other
            // installer of this skill is currently running.
            self.cleanup_leftover_staging(name)?;

            let staging_dir = self.fresh_staging_dir(name);

            // Stage the copy into the temp directory.
            if let Err(e) = copy_dir_recursive(source_dir, &staging_dir) {
                if let Err(cleanup_err) = fs::remove_dir_all(&staging_dir) {
                    warn!(
                        path = %staging_dir.display(),
                        error = %cleanup_err,
                        "failed to clean up partial staging directory"
                    );
                }
                return Err(e.into());
            }

            // Compute installed_hash on the staged copy BEFORE the destructive
            // rename. Any hash failure here leaves the previous install (if
            // force mode) intact on disk — the rename hasn't happened yet.
            // Staging contains the same bytes that will land, so the hash value
            // is identical to what we'd compute post-rename. This is the
            // correct TOCTOU stance: `installed_hash` is the source of truth
            // for what the user has, computed over the bytes we're about to
            // commit to disk.
            let installed_hash = match crate::hash::hash_dir_tree(&staging_dir) {
                Ok(h) => h,
                Err(e) => {
                    warn!(
                        name,
                        error = %e,
                        "installed_hash computation failed on staging; removing staging dir"
                    );
                    if let Err(cleanup_err) = fs::remove_dir_all(&staging_dir) {
                        warn!(
                            path = %staging_dir.display(),
                            error = %cleanup_err,
                            "failed to clean up staging directory after hash failure"
                        );
                    }
                    return Err(e.into());
                }
            };

            // Only now do the destructive swap — hash is already in hand so
            // any failure from here is unrelated to the hash computation.
            if dir.exists() {
                debug!(name, "removing existing skill directory for force install");
                fs::remove_dir_all(&dir)?;
            }

            // Rename staging to final location.
            fs::rename(&staging_dir, &dir)?;
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

    /// Generate a per-attempt staging directory path for a skill install.
    ///
    /// Encoding the pid and a process-local atomic sequence guarantees two
    /// threads (or two processes) computing this for the same skill name get
    /// different paths.
    fn fresh_staging_dir(&self, name: &str) -> PathBuf {
        use std::sync::atomic::Ordering;
        let pid = std::process::id();
        let seq = STAGING_COUNTER.fetch_add(1, Ordering::Relaxed);
        self.skills_dir()
            .join(format!("_installing-{name}-{pid}-{seq}"))
    }

    /// Remove any staging directories left over for this skill from prior
    /// crashed attempts. Matches both the new `_installing-<name>-<pid>-<seq>`
    /// form and the legacy `_installing-<name>` form. Caller must hold the
    /// tracking-file lock.
    fn cleanup_leftover_staging(&self, name: &str) -> std::io::Result<()> {
        let exact = format!("_installing-{name}");
        let prefix = format!("_installing-{name}-");
        let skills_dir = self.skills_dir();

        let entries = match fs::read_dir(&skills_dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e),
        };

        for entry in entries {
            let entry = entry?;
            let file_name = entry.file_name();
            let Some(name_str) = file_name.to_str() else {
                continue;
            };
            if name_str != exact && !name_str.starts_with(&prefix) {
                continue;
            }
            let path = entry.path();
            debug!(
                path = %path.display(),
                "removing leftover staging directory from prior install"
            );
            if let Err(e) = fs::remove_dir_all(&path) {
                warn!(
                    path = %path.display(),
                    error = %e,
                    "failed to remove leftover staging directory"
                );
            }
        }
        Ok(())
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
    fn install_agent_sweeps_leftover_staging_from_prior_crash() {
        let (_dir, project) = temp_project();
        let src_tmp = tempfile::tempdir().unwrap();
        let src = write_agent(src_tmp.path(), "sweep", "---\nname: sweep\n---\nbody\n");
        let (def, mapped) = parse_and_map(&src);

        // Simulate a crashed prior attempt that left staging around.
        fs::create_dir_all(project.root.join(".kiro")).unwrap();
        let ghost = project.root.join(".kiro/_installing-agent-sweep-99999-0");
        fs::create_dir_all(ghost.join("prompts")).unwrap();
        fs::write(ghost.join("agent.json"), b"{}").unwrap();

        project
            .install_agent(&def, &mapped, sample_agent_meta(), None)
            .expect("install should succeed and sweep leftover");

        assert!(!ghost.exists(), "leftover staging should have been swept");
        assert!(project.root.join(".kiro/agents/sweep.json").exists());
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
    fn install_skill_from_dir_recovers_from_leftover_staging_dir() {
        let (_dir, project) = temp_project();

        // Simulate a previous crash that left a staging directory behind.
        let staging_dir = project.skills_dir().join("_installing-recovered");
        fs::create_dir_all(&staging_dir).expect("create staging dir");
        fs::write(staging_dir.join("SKILL.md"), "stale content").expect("write stale");

        // A fresh install of the same skill should clean up and succeed.
        let src = tempfile::tempdir().expect("tempdir");
        fs::write(
            src.path().join("SKILL.md"),
            "---\nname: recovered\ndescription: Fresh\n---\nFresh content.\n",
        )
        .expect("write");

        project
            .install_skill_from_dir("recovered", src.path(), sample_meta())
            .expect("install should succeed despite leftover staging dir");

        let content =
            fs::read_to_string(project.skill_dir("recovered").join("SKILL.md")).expect("read");
        assert!(content.contains("Fresh content."));
        assert!(!staging_dir.exists(), "staging dir should be cleaned up");
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
    fn fresh_staging_dir_returns_unique_paths_within_process() {
        let (_dir, project) = temp_project();
        let p1 = project.fresh_staging_dir("foo");
        let p2 = project.fresh_staging_dir("foo");
        assert_ne!(
            p1, p2,
            "two staging dirs for the same skill name must be distinct"
        );
    }

    #[test]
    fn cleanup_leftover_staging_handles_legacy_format() {
        let (_dir, project) = temp_project();
        fs::create_dir_all(project.skills_dir()).expect("mkdir");

        // Both formats: legacy bare and new pid-suffixed.
        let legacy = project.skills_dir().join("_installing-skillX");
        fs::create_dir_all(&legacy).expect("create legacy staging");
        let new_format = project.skills_dir().join("_installing-skillX-9999-42");
        fs::create_dir_all(&new_format).expect("create new staging");
        let unrelated = project.skills_dir().join("_installing-other-1-2");
        fs::create_dir_all(&unrelated).expect("create unrelated staging");

        project
            .cleanup_leftover_staging("skillX")
            .expect("cleanup should succeed");

        assert!(!legacy.exists(), "legacy staging dir should be removed");
        assert!(!new_format.exists(), "new staging dir should be removed");
        assert!(unrelated.exists(), "unrelated skill's staging is untouched");
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
}

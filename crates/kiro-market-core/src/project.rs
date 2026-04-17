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
}

/// The on-disk structure of `installed-agents.json`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InstalledAgents {
    /// Map from agent name to its installation metadata.
    pub agents: HashMap<String, InstalledAgentMeta>,
}

// ---------------------------------------------------------------------------
// KiroProject
// ---------------------------------------------------------------------------

/// Name of the skill tracking file inside `.kiro/`.
const INSTALLED_SKILLS_FILE: &str = "installed-skills.json";

/// Name of the agent tracking file inside `.kiro/`.
const INSTALLED_AGENTS_FILE: &str = "installed-agents.json";

/// Recursively copy a directory tree from `src` to `dest`.
///
/// Creates `dest` and all intermediate directories. Files are copied
/// preserving the relative directory structure. **Symlinks are skipped**
/// to prevent path traversal attacks where a malicious skill package
/// could include symlinks pointing to sensitive host files.
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
        self.write_skill_dir(name, source_dir, meta, false)
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
        self.write_skill_dir(name, source_dir, meta, true)
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

    /// Install a parsed agent: emit its Kiro JSON + externalized prompt
    /// markdown under `.kiro/agents/`, and record metadata in
    /// `installed-agents.json`.
    ///
    /// The caller is responsible for parsing the source file and mapping the
    /// tool list — the service layer does both upstream so warnings can be
    /// surfaced before the install lock is acquired. This method is purely
    /// the on-disk write step.
    ///
    /// File writes use a staging-and-rename pattern: prompt + JSON are
    /// written to `_installing-agent-<name>-<pid>-<seq>/` under `.kiro/`,
    /// renamed into place after the duplicate check, then tracking is
    /// written last. The whole flow runs under the agent tracking lock.
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
    ) -> crate::error::Result<()> {
        validation::validate_name(&def.name)?;

        // Build JSON outside the lock — this can't fail due to concurrency.
        let json = crate::agent::emit::build_kiro_json(def, mapped_tools)?;
        let json_bytes = serde_json::to_vec_pretty(&json)?;

        crate::file_lock::with_file_lock(
            &self.agent_tracking_path(),
            || -> crate::error::Result<()> {
                let mut installed = self.load_installed_agents()?;
                if installed.agents.contains_key(&def.name) {
                    return Err(AgentError::AlreadyInstalled {
                        name: def.name.clone(),
                    }
                    .into());
                }

                let staging = self.fresh_agent_staging_dir(&def.name);
                let staging_json = staging.join("agent.json");
                let staging_prompt_dir = staging.join("prompts");
                let staging_prompt = staging_prompt_dir.join(format!("{}.md", def.name));

                // Stage both files.
                fs::create_dir_all(&staging_prompt_dir)?;
                if let Err(e) = fs::write(&staging_json, &json_bytes)
                    .and_then(|()| fs::write(&staging_prompt, def.prompt_body.as_bytes()))
                {
                    if let Err(cleanup_err) = fs::remove_dir_all(&staging) {
                        warn!(
                            path = %staging.display(),
                            error = %cleanup_err,
                            "failed to clean up agent staging directory after write failure"
                        );
                    }
                    return Err(e.into());
                }

                // Ensure target directories exist.
                fs::create_dir_all(self.agent_prompts_dir())?;

                let json_target = self.agents_dir().join(format!("{}.json", def.name));
                let prompt_target = self.agent_prompts_dir().join(format!("{}.md", def.name));

                // Rename JSON first. If the prompt rename fails afterwards,
                // roll back the JSON rename so we never leave an agent with
                // only half its files on disk.
                if let Err(e) = fs::rename(&staging_json, &json_target) {
                    let _ = fs::remove_dir_all(&staging);
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
                    let _ = fs::remove_dir_all(&staging);
                    return Err(e.into());
                }

                // Staging directory itself should now be empty (or contain
                // the empty prompts subdir). Remove it.
                let _ = fs::remove_dir_all(&staging);

                // Tracking last. On failure, roll back both files.
                installed.agents.insert(def.name.clone(), meta);
                if let Err(e) = self.write_agent_tracking(&installed) {
                    warn!(
                        name = %def.name,
                        error = %e,
                        "agent tracking update failed after rename; rolling back files"
                    );
                    let _ = fs::remove_file(&json_target);
                    let _ = fs::remove_file(&prompt_target);
                    return Err(e);
                }

                debug!(name = %def.name, "agent installed");
                Ok(())
            },
        )
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
        meta: InstalledSkillMeta,
        force: bool,
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

            // For force installs, remove the old directory now that the new
            // content is safely staged.
            if dir.exists() {
                debug!(name, "removing existing skill directory for force install");
                fs::remove_dir_all(&dir)?;
            }

            // Rename staging to final location.
            fs::rename(&staging_dir, &dir)?;

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
            .install_agent(&def, &mapped, sample_agent_meta())
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
            .install_agent(&def, &mapped, sample_agent_meta())
            .unwrap();
        let err = project
            .install_agent(&def, &mapped, sample_agent_meta())
            .unwrap_err();
        assert!(matches!(
            err,
            crate::error::Error::Agent(AgentError::AlreadyInstalled { .. })
        ));
    }

    #[test]
    fn install_agent_updates_tracking() {
        let (_dir, project) = temp_project();
        let src_tmp = tempfile::tempdir().unwrap();
        let src = write_agent(src_tmp.path(), "a", "---\nname: a\n---\nbody\n");
        let (def, mapped) = parse_and_map(&src);

        project
            .install_agent(&def, &mapped, sample_agent_meta())
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
            .install_agent(&def, &mapped, sample_agent_meta())
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
            .install_agent(&def, &mapped, sample_agent_meta())
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
            .install_agent(&def, &mapped, sample_agent_meta())
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
}

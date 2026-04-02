//! Kiro project state management.
//!
//! Manages the `.kiro/skills/` directory layout and an
//! `installed-skills.json` tracking file that records which skills have been
//! installed, from which marketplace and plugin, and when.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::debug;

use crate::error::SkillError;
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
}

/// The on-disk structure of `installed-skills.json`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InstalledSkills {
    /// Map from skill name to its installation metadata.
    pub skills: HashMap<String, InstalledSkillMeta>,
}

// ---------------------------------------------------------------------------
// KiroProject
// ---------------------------------------------------------------------------

/// Name of the tracking file inside `.kiro/`.
const INSTALLED_SKILLS_FILE: &str = "installed-skills.json";

/// Name of the skill definition file inside each skill directory.
const SKILL_MD: &str = "SKILL.md";

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

    /// Alias for [`Self::load_installed`].
    ///
    /// # Errors
    ///
    /// Same as [`Self::load_installed`].
    pub fn list_installed(&self) -> crate::error::Result<InstalledSkills> {
        self.load_installed()
    }

    /// Install a skill into the project.
    ///
    /// Creates `.kiro/skills/<name>/SKILL.md` with the provided `content`
    /// and records the installation in the tracking file.
    ///
    /// # Errors
    ///
    /// - [`SkillError::AlreadyInstalled`] if a skill with this name already
    ///   exists.
    /// - I/O or JSON serialisation errors.
    pub fn install_skill(
        &self,
        name: &str,
        content: &str,
        meta: InstalledSkillMeta,
    ) -> crate::error::Result<()> {
        validation::validate_name(name)?;
        let dir = self.skill_dir(name);

        if dir.exists() {
            return Err(SkillError::AlreadyInstalled {
                name: name.to_owned(),
            }
            .into());
        }

        self.write_skill(name, content, meta)
    }

    /// Install a skill, overwriting any existing installation.
    ///
    /// # Errors
    ///
    /// I/O or JSON serialisation errors.
    pub fn install_skill_force(
        &self,
        name: &str,
        content: &str,
        meta: InstalledSkillMeta,
    ) -> crate::error::Result<()> {
        validation::validate_name(name)?;
        let dir = self.skill_dir(name);

        if dir.exists() {
            debug!(name, "removing existing skill directory for force install");
            fs::remove_dir_all(&dir)?;
        }

        self.write_skill(name, content, meta)
    }

    /// Remove an installed skill.
    ///
    /// Deletes the skill directory and removes the entry from the tracking
    /// file.
    ///
    /// # Errors
    ///
    /// - [`SkillError::SkillMdNotFound`] if the skill is not installed.
    /// - I/O or JSON serialisation errors.
    pub fn remove_skill(&self, name: &str) -> crate::error::Result<()> {
        validation::validate_name(name)?;
        let dir = self.skill_dir(name);

        if !dir.exists() {
            return Err(SkillError::SkillMdNotFound {
                path: dir.join(SKILL_MD),
            }
            .into());
        }

        fs::remove_dir_all(&dir)?;

        let mut installed = self.load_installed()?;
        installed.skills.remove(name);
        self.write_tracking(&installed)?;

        debug!(name, "skill removed");
        Ok(())
    }

    // -- internal helpers --------------------------------------------------

    /// Write SKILL.md and update tracking for a skill installation.
    fn write_skill(
        &self,
        name: &str,
        content: &str,
        meta: InstalledSkillMeta,
    ) -> crate::error::Result<()> {
        let dir = self.skill_dir(name);
        fs::create_dir_all(&dir)?;
        fs::write(dir.join(SKILL_MD), content)?;

        let mut installed = self.load_installed()?;
        installed.skills.insert(name.to_owned(), meta);
        self.write_tracking(&installed)?;

        debug!(name, "skill installed");
        Ok(())
    }

    /// Persist the tracking file to disk.
    fn write_tracking(&self, installed: &InstalledSkills) -> crate::error::Result<()> {
        let path = self.tracking_path();

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let json = serde_json::to_string_pretty(installed)?;
        fs::write(&path, json)?;
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
    fn install_skill_creates_directory_and_file() {
        let (_dir, project) = temp_project();
        let content = "---\nname: rust-check\ndescription: Rust checks\n---\nBody.\n";

        project
            .install_skill("rust-check", content, sample_meta())
            .expect("install should succeed");

        let skill_md = project.skill_dir("rust-check").join("SKILL.md");
        assert!(skill_md.exists(), "SKILL.md should exist");

        let written = fs::read_to_string(&skill_md).expect("read SKILL.md");
        assert_eq!(written, content);

        let installed = project.load_installed().expect("load");
        assert!(installed.skills.contains_key("rust-check"));
        assert_eq!(installed.skills["rust-check"].plugin, "test-plugin");
    }

    #[test]
    fn install_skill_rejects_duplicate_without_force() {
        let (_dir, project) = temp_project();
        let content = "---\nname: dup\ndescription: Dup\n---\n";

        project
            .install_skill("dup", content, sample_meta())
            .expect("first install should succeed");

        let err = project
            .install_skill("dup", content, sample_meta())
            .expect_err("second install should fail");

        let msg = err.to_string();
        assert!(
            msg.contains("already installed"),
            "expected 'already installed', got: {msg}"
        );
    }

    #[test]
    fn install_skill_force_overwrites_existing() {
        let (_dir, project) = temp_project();
        let original = "---\nname: skill\ndescription: v1\n---\nOriginal.\n";
        let updated = "---\nname: skill\ndescription: v2\n---\nUpdated.\n";

        project
            .install_skill("skill", original, sample_meta())
            .expect("first install");

        project
            .install_skill_force("skill", updated, sample_meta())
            .expect("force install should succeed");

        let written =
            fs::read_to_string(project.skill_dir("skill").join("SKILL.md")).expect("read");
        assert_eq!(written, updated);
    }

    #[test]
    fn remove_skill_deletes_directory_and_tracking() {
        let (_dir, project) = temp_project();
        let content = "---\nname: removable\ndescription: Goes away\n---\n";

        project
            .install_skill("removable", content, sample_meta())
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
            msg.contains("SKILL.md not found"),
            "expected 'SKILL.md not found', got: {msg}"
        );
    }

    #[test]
    fn load_installed_returns_default_when_no_file() {
        let (_dir, project) = temp_project();

        let installed = project.load_installed().expect("load should succeed");
        assert!(installed.skills.is_empty());
    }

    #[test]
    fn install_skill_rejects_path_traversal() {
        let (_dir, project) = temp_project();
        let content = "---\nname: evil\ndescription: Evil\n---\n";

        let err = project
            .install_skill("../escape", content, sample_meta())
            .expect_err("should reject path traversal");
        let msg = err.to_string();
        assert!(
            msg.contains("invalid name"),
            "expected 'invalid name', got: {msg}"
        );
    }

    #[test]
    fn install_skill_rejects_slash_in_name() {
        let (_dir, project) = temp_project();
        let content = "---\nname: evil\ndescription: Evil\n---\n";

        let err = project
            .install_skill("sub/dir", content, sample_meta())
            .expect_err("should reject path separator");
        let msg = err.to_string();
        assert!(
            msg.contains("path separator"),
            "expected 'path separator', got: {msg}"
        );
    }

    #[test]
    fn install_skill_force_rejects_path_traversal() {
        let (_dir, project) = temp_project();
        let content = "---\nname: evil\ndescription: Evil\n---\n";

        let err = project
            .install_skill_force("../escape", content, sample_meta())
            .expect_err("should reject path traversal");
        let msg = err.to_string();
        assert!(
            msg.contains("invalid name"),
            "expected 'invalid name', got: {msg}"
        );
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
    fn list_installed_delegates_to_load() {
        let (_dir, project) = temp_project();
        let content = "---\nname: listed\ndescription: Listed\n---\n";

        project
            .install_skill("listed", content, sample_meta())
            .expect("install");

        let installed = project.list_installed().expect("list");
        assert!(installed.skills.contains_key("listed"));
    }
}

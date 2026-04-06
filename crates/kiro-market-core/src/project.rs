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

/// Recursively copy a directory tree from `src` to `dest`.
///
/// Creates `dest` and all intermediate directories. Files are copied
/// preserving the relative directory structure. Symlinks are followed
/// (the target content is copied, not the link itself).
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
        // Use fs::metadata (follows symlinks) instead of entry.file_type()
        // (which does not follow symlinks on all platforms).
        let metadata = fs::metadata(entry.path()).map_err(|e| {
            std::io::Error::new(
                e.kind(),
                format!("failed to read metadata for {}: {e}", entry.path().display()),
            )
        })?;
        if metadata.is_dir() {
            copy_dir_recursive(&entry.path(), &target)?;
        } else {
            fs::copy(&entry.path(), &target).map_err(|e| {
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
    /// - [`SkillError::NotInstalled`] if the skill is not installed.
    /// - I/O or JSON serialisation errors.
    pub fn remove_skill(&self, name: &str) -> crate::error::Result<()> {
        validation::validate_name(name)?;
        let dir = self.skill_dir(name);

        if !dir.exists() {
            return Err(SkillError::NotInstalled {
                name: name.to_owned(),
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

    /// Install a skill by copying an entire source directory into the project.
    ///
    /// Recursively copies `source_dir` to `.kiro/skills/<name>/`, preserving
    /// companion files (e.g. `references/`) for Kiro's lazy loading. The copy
    /// is atomic: files are staged in a temp directory, then renamed into place
    /// so a crash cannot leave a partially installed skill.
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
        let dir = self.skill_dir(name);

        if dir.exists() {
            return Err(SkillError::AlreadyInstalled {
                name: name.to_owned(),
            }
            .into());
        }

        self.write_skill_dir(name, source_dir, meta)
    }

    /// Install a skill by copying a source directory, overwriting any existing installation.
    ///
    /// The copy is atomic: new content is staged in a temp directory first, then
    /// the old directory is removed and the temp is renamed into place.
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
        self.write_skill_dir(name, source_dir, meta)
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
        crate::cache::atomic_write(&dir.join(SKILL_MD), content.as_bytes())?;

        let mut installed = self.load_installed()?;
        installed.skills.insert(name.to_owned(), meta);
        self.write_tracking(&installed)?;

        debug!(name, "skill installed");
        Ok(())
    }

    /// Copy a source skill directory atomically and update tracking.
    ///
    /// Copies to a staging directory (`_installing-<name>`) first, then
    /// renames into the final location. For force installs, the old directory
    /// is removed after the staging copy succeeds but before the rename.
    fn write_skill_dir(
        &self,
        name: &str,
        source_dir: &Path,
        meta: InstalledSkillMeta,
    ) -> crate::error::Result<()> {
        let dir = self.skill_dir(name);
        let staging_dir = self.skills_dir().join(format!("_installing-{name}"));

        // Clean up any leftover staging dir from a previous crash.
        if staging_dir.exists() {
            debug!(
                path = %staging_dir.display(),
                "removing leftover staging directory"
            );
            fs::remove_dir_all(&staging_dir)?;
        }

        // Ensure the skills parent directory exists.
        fs::create_dir_all(self.skills_dir())?;

        // Stage the copy into the temp directory.
        copy_dir_recursive(source_dir, &staging_dir)?;

        // For force installs, remove the old directory now that the new
        // content is safely staged.
        if dir.exists() {
            debug!(name, "removing existing skill directory for force install");
            fs::remove_dir_all(&dir)?;
        }

        // Atomic rename from staging to final location.
        fs::rename(&staging_dir, &dir)?;

        let mut installed = self.load_installed()?;
        installed.skills.insert(name.to_owned(), meta);
        self.write_tracking(&installed)?;

        debug!(name, "skill installed from directory");
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
    fn load_installed_returns_installed_skills() {
        let (_dir, project) = temp_project();
        let content = "---\nname: listed\ndescription: Listed\n---\n";

        project
            .install_skill("listed", content, sample_meta())
            .expect("install");

        let installed = project.load_installed().expect("load");
        assert!(installed.skills.contains_key("listed"));
    }

    #[test]
    fn tracking_file_contains_valid_json_after_install() {
        let (_dir, project) = temp_project();
        let content = "---\nname: atomic-check\ndescription: Checks atomic\n---\n";

        project
            .install_skill("atomic-check", content, sample_meta())
            .expect("install");

        let raw = fs::read(project.tracking_path()).expect("read tracking file");
        let parsed: InstalledSkills =
            serde_json::from_slice(&raw).expect("tracking file should be valid JSON");
        assert!(parsed.skills.contains_key("atomic-check"));

        // The temp file should not remain.
        assert!(
            !project.tracking_path().with_extension("tmp").exists(),
            ".tmp file should be gone after atomic rename"
        );
    }

    #[test]
    fn skill_md_written_atomically_no_tmp_leftover() {
        let (_dir, project) = temp_project();
        let content = "---\nname: atomic-skill\ndescription: Atomic write\n---\nBody.\n";

        project
            .install_skill("atomic-skill", content, sample_meta())
            .expect("install");

        let skill_md = project.skill_dir("atomic-skill").join("SKILL.md");
        assert!(skill_md.exists(), "SKILL.md should exist");

        let written = fs::read_to_string(&skill_md).expect("read SKILL.md");
        assert_eq!(written, content, "content should match exactly");

        // The atomic write temp file should not remain.
        assert!(
            !skill_md.with_extension("tmp").exists(),
            "SKILL.md.tmp should be gone after atomic rename"
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
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with("_installing-")
            })
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

        let content =
            fs::read_to_string(project.skill_dir("s").join("SKILL.md")).expect("read");
        assert!(content.contains("Updated."));

        assert!(project
            .skill_dir("s")
            .join("references")
            .join("new.md")
            .exists());
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
}

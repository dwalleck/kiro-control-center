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
                format!("failed to read metadata for {}: {e}", entry.path().display()),
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
        self.write_skill_dir(name, source_dir, meta)
    }

    // -- internal helpers --------------------------------------------------

    /// Copy a source skill directory and update tracking.
    ///
    /// Copies to a staging directory (`_installing-<name>`) first, then
    /// renames into the final location so a crash during the copy phase
    /// cannot leave a partially installed skill directory. The tracking
    /// file is updated separately after the rename. For force installs,
    /// the old directory is removed after staging succeeds but before
    /// the rename.
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
        if let Err(e) = copy_dir_recursive(source_dir, &staging_dir) {
            // Clean up the partial staging directory.
            if let Err(cleanup_err) = fs::remove_dir_all(&staging_dir) {
                debug!(
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

        // Update the tracking file. If this fails, roll back the rename
        // to keep the filesystem and tracking file consistent.
        let tracking_result = self
            .load_installed()
            .and_then(|mut installed| {
                installed.skills.insert(name.to_owned(), meta);
                self.write_tracking(&installed)
            });

        if let Err(e) = tracking_result {
            debug!(
                name,
                error = %e,
                "tracking update failed after rename, rolling back"
            );
            if let Err(rollback_err) = fs::remove_dir_all(&dir) {
                debug!(
                    path = %dir.display(),
                    error = %rollback_err,
                    "failed to roll back skill directory after tracking failure"
                );
            }
            return Err(e);
        }

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
        assert!(project.skill_dir("s").join("references").join("old.md").exists());

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
}

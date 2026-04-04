//! Commands for managing installed skills.

use std::path::PathBuf;

use serde::Serialize;
use tracing::debug;

use kiro_market_core::project::KiroProject;

use crate::error::CommandError;

// ---------------------------------------------------------------------------
// Response types
// ---------------------------------------------------------------------------

/// Information about a single installed skill.
#[derive(Clone, Debug, Serialize, specta::Type)]
pub struct InstalledSkillInfo {
    pub name: String,
    pub marketplace: String,
    pub plugin: String,
    pub version: Option<String>,
    /// ISO 8601 timestamp of when the skill was installed.
    pub installed_at: String,
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// List all skills installed in a Kiro project, sorted by name.
#[tauri::command]
#[specta::specta]
pub async fn list_installed_skills(
    project_path: String,
) -> Result<Vec<InstalledSkillInfo>, CommandError> {
    let project = KiroProject::new(PathBuf::from(&project_path));
    let installed = project.load_installed().map_err(CommandError::from)?;

    let mut results: Vec<InstalledSkillInfo> = installed
        .skills
        .into_iter()
        .map(|(name, meta)| InstalledSkillInfo {
            name,
            marketplace: meta.marketplace,
            plugin: meta.plugin,
            version: meta.version,
            installed_at: meta.installed_at.to_rfc3339(),
        })
        .collect();

    results.sort_by(|a, b| a.name.cmp(&b.name));
    debug!(count = results.len(), "loaded installed skills");

    Ok(results)
}

/// Remove an installed skill from a Kiro project.
#[tauri::command]
#[specta::specta]
pub async fn remove_skill(name: String, project_path: String) -> Result<(), CommandError> {
    let project = KiroProject::new(PathBuf::from(&project_path));
    project.remove_skill(&name).map_err(CommandError::from)?;
    debug!(skill = %name, "skill removed via control center");

    Ok(())
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use kiro_market_core::project::{InstalledSkillMeta, KiroProject};

    use super::*;

    fn temp_project_with_skill(name: &str) -> (tempfile::TempDir, String) {
        let dir = tempfile::tempdir().expect("tempdir");
        let project = KiroProject::new(dir.path().to_path_buf());
        let meta = InstalledSkillMeta {
            marketplace: "test-market".into(),
            plugin: "test-plugin".into(),
            version: Some("1.0.0".into()),
            installed_at: Utc::now(),
        };
        project
            .install_skill(name, "# Test Skill\nBody content", meta)
            .expect("install_skill");
        let path = dir.path().to_str().expect("valid utf-8").to_owned();
        (dir, path)
    }

    #[tokio::test]
    async fn list_installed_skills_returns_sorted_list() {
        let dir = tempfile::tempdir().expect("tempdir");
        let project = KiroProject::new(dir.path().to_path_buf());
        let path = dir.path().to_str().expect("valid utf-8").to_owned();

        for name in &["zulu-skill", "alpha-skill", "mike-skill"] {
            let meta = InstalledSkillMeta {
                marketplace: "test-market".into(),
                plugin: "test-plugin".into(),
                version: Some("1.0.0".into()),
                installed_at: Utc::now(),
            };
            project
                .install_skill(name, "# Skill\nBody", meta)
                .expect("install_skill");
        }

        let result = list_installed_skills(path).await.expect("should succeed");

        assert_eq!(result.len(), 3);
        assert_eq!(result[0].name, "alpha-skill");
        assert_eq!(result[1].name, "mike-skill");
        assert_eq!(result[2].name, "zulu-skill");
        assert_eq!(result[0].marketplace, "test-market");
        assert_eq!(result[0].plugin, "test-plugin");
        assert!(result[0].version.as_deref() == Some("1.0.0"));
    }

    #[tokio::test]
    async fn list_installed_skills_empty_project_returns_empty_vec() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().to_str().expect("valid utf-8").to_owned();

        let result = list_installed_skills(path).await.expect("should succeed");

        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn remove_skill_removes_from_project() {
        let (_dir, path) = temp_project_with_skill("removable-skill");

        remove_skill("removable-skill".into(), path.clone())
            .await
            .expect("should succeed");

        let result = list_installed_skills(path).await.expect("should succeed");
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn remove_skill_nonexistent_returns_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().to_str().expect("valid utf-8").to_owned();

        let err = remove_skill("nonexistent".into(), path)
            .await
            .expect_err("should fail");

        assert!(
            err.message.contains("nonexistent"),
            "expected skill name in error: {}",
            err.message
        );
    }
}

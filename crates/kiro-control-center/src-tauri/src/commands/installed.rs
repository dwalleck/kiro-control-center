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

//! Settings management: scan roots, project discovery, and active project.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

use crate::error::CommandError;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Persisted application settings.
#[derive(Clone, Debug, Default, Serialize, Deserialize, specta::Type)]
pub struct Settings {
    /// Directories to scan for Kiro projects.
    #[serde(default)]
    pub scan_roots: Vec<String>,
    /// Last active project path (restored on launch).
    #[serde(default)]
    pub last_project: Option<String>,
}

/// A discovered Kiro project.
#[derive(Clone, Debug, Serialize, specta::Type)]
pub struct DiscoveredProject {
    /// Absolute path to the project root.
    pub path: String,
    /// Directory name (for display).
    pub name: String,
    /// Whether `.kiro/` exists.
    pub kiro_initialized: bool,
    /// Number of installed skills (0 during discovery, loaded on demand).
    pub skill_count: u32,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Path to the settings file.
///
/// Respects `KIRO_MARKET_CONFIG_DIR` env var for test isolation (the `dirs`
/// crate ignores `XDG_CONFIG_HOME` on macOS/Windows).
fn settings_path() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("KIRO_MARKET_CONFIG_DIR") {
        return Some(PathBuf::from(dir).join("settings.json"));
    }
    dirs::config_dir().map(|d| d.join("kiro-market").join("settings.json"))
}

/// Load settings from disk, returning defaults if the file doesn't exist.
fn load_settings() -> Settings {
    let Some(path) = settings_path() else {
        return Settings::default();
    };
    match fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Settings::default(),
        Err(e) => {
            warn!(path = %path.display(), error = %e, "failed to read settings");
            Settings::default()
        }
    }
}

/// Save settings to disk.
fn save_settings(settings: &Settings) -> Result<(), CommandError> {
    let path = settings_path().ok_or_else(|| {
        CommandError::new(
            "could not determine config directory",
            crate::error::ErrorType::IoError,
        )
    })?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| {
            CommandError::new(
                format!("failed to create config directory: {e}"),
                crate::error::ErrorType::IoError,
            )
        })?;
    }
    let json = serde_json::to_string_pretty(settings).map_err(|e| {
        CommandError::new(
            format!("failed to serialize settings: {e}"),
            crate::error::ErrorType::IoError,
        )
    })?;
    fs::write(&path, json).map_err(|e| {
        CommandError::new(
            format!("failed to write settings: {e}"),
            crate::error::ErrorType::IoError,
        )
    })?;
    debug!(path = %path.display(), "settings saved");
    Ok(())
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// Load application settings.
#[tauri::command]
#[specta::specta]
#[allow(clippy::unused_async)] // Tauri commands must be async
pub async fn get_settings() -> Result<Settings, CommandError> {
    Ok(load_settings())
}

/// Save the list of scan root directories.
#[tauri::command]
#[specta::specta]
#[allow(clippy::unused_async)] // Tauri commands must be async
pub async fn save_scan_roots(roots: Vec<String>) -> Result<(), CommandError> {
    let mut settings = load_settings();
    settings.scan_roots = roots;
    save_settings(&settings)
}

/// Discover Kiro projects by scanning configured root directories.
///
/// Scans each root 1-2 levels deep for directories containing `.kiro/`.
#[tauri::command]
#[specta::specta]
#[allow(clippy::unused_async)] // Tauri commands must be async
pub async fn discover_projects() -> Result<Vec<DiscoveredProject>, CommandError> {
    let settings = load_settings();
    let mut projects = Vec::new();

    for root in &settings.scan_roots {
        let root_path = PathBuf::from(shellexpand_tilde(root));
        if !root_path.is_dir() {
            warn!(root = %root, "scan root is not a directory, skipping");
            continue;
        }
        scan_for_projects(&root_path, 0, 2, &mut projects);
    }

    projects.sort_by(|a, b| a.name.cmp(&b.name));
    projects.dedup_by(|a, b| a.path == b.path);
    Ok(projects)
}

/// Set the active project and persist it.
#[tauri::command]
#[specta::specta]
pub async fn set_active_project(
    path: String,
) -> Result<crate::commands::browse::ProjectInfo, CommandError> {
    let project_path = PathBuf::from(&path);
    if !project_path.is_dir() {
        return Err(CommandError::new(
            format!("directory does not exist: {path}"),
            crate::error::ErrorType::NotFound,
        ));
    }

    // Persist as last_project.
    let mut settings = load_settings();
    settings.last_project = Some(path.clone());
    save_settings(&settings)?;

    // Return ProjectInfo (reuse existing command logic).
    crate::commands::browse::get_project_info(path).await
}

// ---------------------------------------------------------------------------
// Scanning helpers
// ---------------------------------------------------------------------------

/// Expand `~` to the home directory.
fn shellexpand_tilde(path: &str) -> String {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest).to_string_lossy().into_owned();
        }
    }
    path.to_owned()
}

/// Recursively scan for `.kiro` directories up to `max_depth` levels.
fn scan_for_projects(
    dir: &Path,
    current_depth: u32,
    max_depth: u32,
    results: &mut Vec<DiscoveredProject>,
) {
    if current_depth > max_depth {
        return;
    }

    // Check if this directory itself is a Kiro project.
    let kiro_dir = dir.join(".kiro");
    if kiro_dir.is_dir() {
        let name = dir.file_name().map_or_else(
            || dir.to_string_lossy().into_owned(),
            |n| n.to_string_lossy().into_owned(),
        );

        // Skip reading installed-skills.json during scan for performance.
        // skill_count is loaded on demand when the user selects a project.
        results.push(DiscoveredProject {
            path: dir.to_string_lossy().into_owned(),
            name,
            kiro_initialized: true,
            skill_count: 0,
        });
        // Don't recurse into .kiro projects.
        return;
    }

    // Recurse into subdirectories.
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries {
        let Ok(entry) = entry else { continue };
        let path = entry.path();
        if path.is_dir() {
            // Skip hidden directories and common non-project dirs.
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str.starts_with('.')
                || name_str == "node_modules"
                || name_str == "target"
                || name_str == ".git"
            {
                continue;
            }
            scan_for_projects(&path, current_depth + 1, max_depth, results);
        }
    }
}

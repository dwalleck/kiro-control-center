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

/// A discovered Kiro project found during directory scanning.
#[derive(Clone, Debug, Serialize, specta::Type)]
pub struct DiscoveredProject {
    /// Absolute path to the project root.
    pub path: String,
    /// Directory name (for display).
    pub name: String,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Resolve the platform config directory for kiro-market settings.
///
/// Returns `None` only when the OS has no config directory (rare).
fn default_config_dir() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("kiro-market"))
}

/// Distinct outcomes from attempting to load the settings file.
///
/// Distinguishing `Missing` from `Corrupt` is load-bearing: read-only
/// callers can treat either as "fall back to defaults", but write callers
/// MUST refuse to overwrite a corrupt file (otherwise saving any single
/// field silently destroys the rest of the user's settings).
enum LoadOutcome {
    Loaded(Settings),
    Missing,
    Corrupt(String),
}

/// Try to load settings, distinguishing missing/corrupt/loaded outcomes.
fn try_load_settings_from(config_dir: &Path) -> LoadOutcome {
    let path = config_dir.join("settings.json");
    match fs::read(&path) {
        Ok(bytes) => match serde_json::from_slice(&bytes) {
            Ok(settings) => LoadOutcome::Loaded(settings),
            Err(e) => {
                warn!(
                    path = %path.display(),
                    error = %e,
                    "settings file contains invalid JSON; refusing to overwrite from save paths"
                );
                LoadOutcome::Corrupt(e.to_string())
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => LoadOutcome::Missing,
        Err(e) => {
            warn!(path = %path.display(), error = %e, "failed to read settings");
            LoadOutcome::Corrupt(e.to_string())
        }
    }
}

/// Load settings for read-only callers (e.g. `get_settings`, `discover_projects`).
/// Falls back to defaults whether the file is missing or corrupt — display
/// paths must never fail the UI for this.
fn load_settings_from(config_dir: &Path) -> Settings {
    match try_load_settings_from(config_dir) {
        LoadOutcome::Loaded(s) => s,
        LoadOutcome::Missing | LoadOutcome::Corrupt(_) => Settings::default(),
    }
}

/// Load settings for callers that intend to write back. A corrupt file is
/// returned as an error so we never silently destroy a partially-recoverable
/// settings file by saving defaults+one-new-field over it.
fn load_settings_for_modification(config_dir: &Path) -> Result<Settings, CommandError> {
    match try_load_settings_from(config_dir) {
        LoadOutcome::Loaded(s) => Ok(s),
        LoadOutcome::Missing => Ok(Settings::default()),
        LoadOutcome::Corrupt(detail) => {
            let path = config_dir.join("settings.json");
            Err(CommandError::new(
                format!(
                    "settings file at {} contains invalid JSON and cannot be safely updated: {detail}. \
                     Back up or delete the file and try again.",
                    path.display()
                ),
                crate::error::ErrorType::ParseError,
            ))
        }
    }
}

/// Save settings to `config_dir/settings.json`.
fn save_settings_to(config_dir: &Path, settings: &Settings) -> Result<(), CommandError> {
    let path = config_dir.join("settings.json");
    fs::create_dir_all(config_dir).map_err(|e| {
        CommandError::new(
            format!("failed to create config directory: {e}"),
            crate::error::ErrorType::IoError,
        )
    })?;
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

/// Convenience: load from the default config directory (read-only paths).
fn load_settings() -> Settings {
    let Some(dir) = default_config_dir() else {
        return Settings::default();
    };
    load_settings_from(&dir)
}

/// Convenience: load from the default config directory for write-back paths.
fn load_settings_for_save() -> Result<Settings, CommandError> {
    let dir = default_config_dir().ok_or_else(|| {
        CommandError::new(
            "could not determine config directory",
            crate::error::ErrorType::IoError,
        )
    })?;
    load_settings_for_modification(&dir)
}

/// Convenience: save to the default config directory.
fn save_settings(settings: &Settings) -> Result<(), CommandError> {
    let dir = default_config_dir().ok_or_else(|| {
        CommandError::new(
            "could not determine config directory",
            crate::error::ErrorType::IoError,
        )
    })?;
    save_settings_to(&dir, settings)
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
    let mut settings = load_settings_for_save()?;
    settings.scan_roots = roots;
    save_settings(&settings)
}

/// Discover Kiro projects by scanning configured root directories.
///
/// Scans each root up to 2 levels deep for directories containing `.kiro/`.
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

    // Dedup by path (requires sorting by path first), then sort by name for display.
    projects.sort_by(|a, b| a.path.cmp(&b.path));
    projects.dedup_by(|a, b| a.path == b.path);
    projects.sort_by(|a, b| a.name.cmp(&b.name));
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

    // Persist as last_project. Use the strict loader so a corrupt settings
    // file is not silently overwritten with defaults+last_project.
    let mut settings = load_settings_for_save()?;
    settings.last_project = Some(path.clone());
    save_settings(&settings)?;

    // Return ProjectInfo (reuse existing command logic).
    crate::commands::browse::get_project_info(path).await
}

// ---------------------------------------------------------------------------
// Scanning helpers
// ---------------------------------------------------------------------------

/// Expand `~/` prefix to the home directory. A bare `~` is also expanded.
fn shellexpand_tilde(path: &str) -> String {
    if path == "~" {
        if let Some(home) = dirs::home_dir() {
            return home.to_string_lossy().into_owned();
        }
    }
    let rest = path.strip_prefix("~/").or_else(|| path.strip_prefix("~\\"));
    if let Some(rest) = rest {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest).to_string_lossy().into_owned();
        }
        warn!(path = %path, "could not expand '~' — HOME directory not available");
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

        // Discovery collects only path and name; full project details
        // are loaded when the user selects a project.
        results.push(DiscoveredProject {
            path: dir.to_string_lossy().into_owned(),
            name,
        });
        // Don't recurse into .kiro projects.
        return;
    }

    // Recurse into subdirectories.
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) => {
            debug!(dir = %dir.display(), error = %e, "could not read directory, skipping");
            return;
        }
    };
    for entry in entries {
        let Ok(entry) = entry else { continue };
        let path = entry.path();
        if path.is_dir() {
            // Skip hidden directories (covers .git, .cache, etc.) and common build dirs.
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str.starts_with('.') || name_str == "node_modules" || name_str == "target" {
                continue;
            }
            scan_for_projects(&path, current_depth + 1, max_depth, results);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_settings_returns_defaults_when_no_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let settings = load_settings_from(dir.path());
        assert!(settings.scan_roots.is_empty());
        assert!(settings.last_project.is_none());
    }

    #[test]
    fn save_and_load_settings_roundtrip() {
        let dir = tempfile::tempdir().expect("tempdir");
        let settings = Settings {
            scan_roots: vec!["~/repos".into(), "~/work".into()],
            last_project: Some("/home/user/project".into()),
        };
        save_settings_to(dir.path(), &settings).expect("save");

        let loaded = load_settings_from(dir.path());
        assert_eq!(loaded.scan_roots, vec!["~/repos", "~/work"]);
        assert_eq!(loaded.last_project.as_deref(), Some("/home/user/project"));
    }

    #[test]
    fn shellexpand_tilde_expands_home() {
        let expanded = shellexpand_tilde("~/repos");
        assert!(
            !expanded.starts_with('~'),
            "tilde should be expanded: {expanded}"
        );
        assert!(expanded.ends_with("repos"));
    }

    #[test]
    fn shellexpand_tilde_leaves_absolute_paths_alone() {
        let path = "/absolute/path";
        assert_eq!(shellexpand_tilde(path), path);
    }

    #[test]
    fn scan_for_projects_finds_kiro_dirs() {
        let dir = tempfile::tempdir().expect("tempdir");

        // Create two projects: one at depth 1, one at depth 2.
        let proj1 = dir.path().join("project-a");
        std::fs::create_dir_all(proj1.join(".kiro")).expect("create .kiro");

        let org = dir.path().join("org");
        let proj2 = org.join("project-b");
        std::fs::create_dir_all(proj2.join(".kiro")).expect("create .kiro");

        // Create a non-project directory (no .kiro).
        std::fs::create_dir_all(dir.path().join("not-a-project")).expect("create dir");

        let mut results = Vec::new();
        scan_for_projects(dir.path(), 0, 2, &mut results);

        assert_eq!(results.len(), 2, "should find 2 projects: {results:?}");

        let names: Vec<&str> = results.iter().map(|p| p.name.as_str()).collect();
        assert!(names.contains(&"project-a"), "missing project-a: {names:?}");
        assert!(names.contains(&"project-b"), "missing project-b: {names:?}");
    }

    #[test]
    fn scan_for_projects_respects_max_depth() {
        let dir = tempfile::tempdir().expect("tempdir");

        // Create project at depth 3 (should NOT be found with max_depth=2).
        let deep = dir.path().join("a").join("b").join("c");
        std::fs::create_dir_all(deep.join(".kiro")).expect("create .kiro");

        let mut results = Vec::new();
        scan_for_projects(dir.path(), 0, 2, &mut results);

        assert!(results.is_empty(), "should not find projects at depth 3");
    }

    #[test]
    fn scan_for_projects_skips_hidden_and_build_dirs() {
        let dir = tempfile::tempdir().expect("tempdir");

        // Create .kiro inside hidden, node_modules, and target dirs.
        for name in &[".hidden", "node_modules", "target"] {
            let d = dir.path().join(name).join("sneaky");
            std::fs::create_dir_all(d.join(".kiro")).expect("create .kiro");
        }

        let mut results = Vec::new();
        scan_for_projects(dir.path(), 0, 2, &mut results);

        assert!(
            results.is_empty(),
            "should skip hidden/build dirs: {results:?}"
        );
    }

    #[test]
    fn load_settings_returns_defaults_on_corrupt_json() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("settings.json");
        std::fs::write(&path, "not valid json {{{").expect("write garbage");

        let settings = load_settings_from(dir.path());
        assert!(
            settings.scan_roots.is_empty(),
            "corrupt file should fall back to defaults"
        );
        assert!(settings.last_project.is_none());
    }

    #[test]
    fn load_settings_for_modification_refuses_to_overwrite_corrupt_file() {
        // Corrupt JSON with recoverable content — saving defaults+new_field
        // would destroy the user's last_project line. The strict loader
        // must surface an error so the save path bails before write.
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("settings.json");
        std::fs::write(
            &path,
            // Looks like JSON but has a trailing comma — invalid.
            r#"{"scan_roots": ["~/repos"], "last_project": "/home/user/proj",}"#,
        )
        .expect("write");

        let err = load_settings_for_modification(dir.path())
            .expect_err("strict loader should refuse corrupt file");
        assert_eq!(err.error_type, crate::error::ErrorType::ParseError);
        assert!(
            err.message.contains("invalid JSON"),
            "expected hint about invalid JSON in error: {}",
            err.message
        );

        // Verify the file is unchanged after the failed load.
        let after = std::fs::read_to_string(&path).expect("read");
        assert!(
            after.contains("/home/user/proj"),
            "corrupt file must not be touched, got: {after}"
        );
    }

    #[test]
    fn load_settings_for_modification_returns_defaults_when_missing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let settings = load_settings_for_modification(dir.path()).expect("missing should be ok");
        assert!(settings.scan_roots.is_empty());
        assert!(settings.last_project.is_none());
    }

    #[test]
    fn save_scan_roots_preserves_last_project() {
        let dir = tempfile::tempdir().expect("tempdir");

        // Save settings with both fields populated.
        let settings = Settings {
            scan_roots: vec!["~/old-root".into()],
            last_project: Some("/home/user/my-project".into()),
        };
        save_settings_to(dir.path(), &settings).expect("save initial");

        // Now update only scan_roots (simulating save_scan_roots).
        let mut loaded = load_settings_from(dir.path());
        loaded.scan_roots = vec!["~/new-root".into()];
        save_settings_to(dir.path(), &loaded).expect("save updated roots");

        // Verify last_project survived the update.
        let final_settings = load_settings_from(dir.path());
        assert_eq!(final_settings.scan_roots, vec!["~/new-root"]);
        assert_eq!(
            final_settings.last_project.as_deref(),
            Some("/home/user/my-project"),
            "last_project should survive scan_roots update"
        );
    }

    #[test]
    fn shellexpand_tilde_expands_bare_tilde() {
        let expanded = shellexpand_tilde("~");
        assert!(
            !expanded.contains('~'),
            "bare ~ should be expanded: {expanded}"
        );
        assert!(!expanded.is_empty(), "expanded home should not be empty");
    }
}

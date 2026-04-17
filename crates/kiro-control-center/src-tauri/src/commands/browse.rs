//! Browse commands for marketplace/plugin/skill discovery.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;
use tracing::{debug, warn};

use kiro_market_core::cache::{CacheDir, MarketplaceSource};
use kiro_market_core::git::GixCliBackend;
use kiro_market_core::marketplace::{PluginEntry, PluginSource, StructuredSource};
use kiro_market_core::plugin::{discover_skill_dirs, PluginManifest};
use kiro_market_core::project::KiroProject;
use kiro_market_core::service::{InstallFilter, MarketplaceService};
use kiro_market_core::skill::parse_frontmatter;

use crate::error::{CommandError, ErrorType};

// ---------------------------------------------------------------------------
// Response types
// ---------------------------------------------------------------------------

/// Source type classification for marketplaces and plugins.
///
/// Serialized as snake_case strings. The TypeScript side receives a union
/// type like `"github" | "git" | "local" | "relative" | "git_subdir"`.
#[derive(Clone, Debug, Serialize, specta::Type)]
pub enum SourceType {
    #[serde(rename = "github")]
    GitHub,
    #[serde(rename = "git")]
    Git,
    #[serde(rename = "local")]
    Local,
    #[serde(rename = "relative")]
    Relative,
    #[serde(rename = "git-subdir")]
    GitSubdir,
}

/// Summary information about a registered marketplace.
#[derive(Clone, Debug, Serialize, specta::Type)]
pub struct MarketplaceInfo {
    pub name: String,
    pub source_type: SourceType,
    pub plugin_count: u32,
    /// If the marketplace manifest could not be read or parsed, this field
    /// carries the error message so the frontend can show a warning.
    pub load_error: Option<String>,
}

/// Summary information about a plugin within a marketplace.
#[derive(Clone, Debug, Serialize, specta::Type)]
pub struct PluginInfo {
    pub name: String,
    pub description: Option<String>,
    pub skill_count: u32,
    pub source_type: SourceType,
}

/// Information about a single skill, including installation status.
#[derive(Clone, Debug, Serialize, specta::Type)]
pub struct SkillInfo {
    pub name: String,
    pub description: String,
    pub plugin: String,
    pub marketplace: String,
    pub installed: bool,
}

/// Result of an install operation across multiple skills.
#[derive(Clone, Debug, Serialize, specta::Type)]
pub struct InstallResult {
    pub installed: Vec<String>,
    pub skipped: Vec<String>,
    pub failed: Vec<FailedSkill>,
}

/// A skill that failed to install, with the reason.
#[derive(Clone, Debug, Serialize, specta::Type)]
pub struct FailedSkill {
    pub name: String,
    pub error: String,
}

/// Summary information about a Kiro project directory.
#[derive(Clone, Debug, Serialize, specta::Type)]
pub struct ProjectInfo {
    pub path: String,
    pub kiro_initialized: bool,
    pub installed_skill_count: u32,
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// Construct a `MarketplaceService` for read-side handlers.
///
/// All Tauri commands here are read-only or install-only; the [`GitBackend`]
/// is unused on every code path, so the default `GixCliBackend` is fine.
fn make_service() -> Result<MarketplaceService, CommandError> {
    let cache = CacheDir::default_location().ok_or_else(|| {
        CommandError::new(
            "could not determine data directory; is $HOME set?",
            ErrorType::IoError,
        )
    })?;
    Ok(MarketplaceService::new(cache, GixCliBackend::default()))
}

/// List all registered marketplaces with plugin counts.
#[tauri::command]
#[specta::specta]
pub async fn list_marketplaces() -> Result<Vec<MarketplaceInfo>, CommandError> {
    let svc = make_service()?;
    let known = svc.list().map_err(CommandError::from)?;

    let mut results = Vec::with_capacity(known.len());
    for entry in &known {
        let source_type = marketplace_source_type(&entry.source);
        let (plugin_count, load_error) = match svc.list_plugin_entries(&entry.name) {
            Ok(entries) => (entries.len() as u32, None),
            Err(e) => (0, Some(e.to_string())),
        };
        results.push(MarketplaceInfo {
            name: entry.name.clone(),
            source_type,
            plugin_count,
            load_error,
        });
    }

    Ok(results)
}

/// List all plugins in a given marketplace.
#[tauri::command]
#[specta::specta]
pub async fn list_plugins(marketplace: String) -> Result<Vec<PluginInfo>, CommandError> {
    let svc = make_service()?;
    let marketplace_path = svc.marketplace_path(&marketplace);
    let plugin_entries = svc
        .list_plugin_entries(&marketplace)
        .map_err(CommandError::from)?;

    let mut results = Vec::with_capacity(plugin_entries.len());
    for plugin in &plugin_entries {
        let source_type = plugin_source_type(&plugin.source);
        let skill_count = count_plugin_skills(plugin, &marketplace_path);
        results.push(PluginInfo {
            name: plugin.name.clone(),
            description: plugin.description.clone(),
            skill_count: skill_count as u32,
            source_type,
        });
    }

    Ok(results)
}

/// List all available skills for a plugin, cross-referenced with installed state.
#[tauri::command]
#[specta::specta]
pub async fn list_available_skills(
    marketplace: String,
    plugin: String,
    project_path: String,
) -> Result<Vec<SkillInfo>, CommandError> {
    let svc = make_service()?;
    let marketplace_path = svc.marketplace_path(&marketplace);
    let plugin_entries = svc
        .list_plugin_entries(&marketplace)
        .map_err(CommandError::from)?;

    let plugin_entry = plugin_entries
        .iter()
        .find(|p| p.name == plugin)
        .ok_or_else(|| {
            CommandError::new(
                format!("plugin '{plugin}' not found in marketplace '{marketplace}'"),
                ErrorType::NotFound,
            )
        })?;

    let plugin_dir = resolve_local_plugin_dir(plugin_entry, &marketplace_path)?;
    let plugin_manifest = load_plugin_manifest(&plugin_dir)?;
    let skill_dirs = discover_skills_for_plugin(&plugin_dir, plugin_manifest.as_ref());

    let project = KiroProject::new(PathBuf::from(&project_path));
    let installed = project.load_installed().map_err(CommandError::from)?;

    let mut results = Vec::with_capacity(skill_dirs.len());
    for skill_dir in &skill_dirs {
        let skill_md_path = skill_dir.join("SKILL.md");
        let content = match fs::read_to_string(&skill_md_path) {
            Ok(c) => c,
            Err(e) => {
                warn!(
                    path = %skill_md_path.display(),
                    error = %e,
                    "failed to read SKILL.md, skipping"
                );
                continue;
            }
        };

        let (frontmatter, _body_offset) = match parse_frontmatter(&content) {
            Ok(result) => result,
            Err(e) => {
                warn!(
                    path = %skill_md_path.display(),
                    error = %e,
                    "failed to parse SKILL.md frontmatter, skipping"
                );
                continue;
            }
        };

        let is_installed = installed.skills.contains_key(&frontmatter.name);
        results.push(SkillInfo {
            name: frontmatter.name,
            description: frontmatter.description,
            plugin: plugin.clone(),
            marketplace: marketplace.clone(),
            installed: is_installed,
        });
    }

    Ok(results)
}

/// Install specific skills from a plugin into a Kiro project.
#[tauri::command]
#[specta::specta]
pub async fn install_skills(
    marketplace: String,
    plugin: String,
    skills: Vec<String>,
    force: bool,
    project_path: String,
) -> Result<InstallResult, CommandError> {
    let svc = make_service()?;
    let marketplace_path = svc.marketplace_path(&marketplace);
    let plugin_entries = svc
        .list_plugin_entries(&marketplace)
        .map_err(CommandError::from)?;

    let plugin_entry = plugin_entries
        .iter()
        .find(|p| p.name == plugin)
        .ok_or_else(|| {
            CommandError::new(
                format!("plugin '{plugin}' not found in marketplace '{marketplace}'"),
                ErrorType::NotFound,
            )
        })?;

    let plugin_dir = resolve_local_plugin_dir(plugin_entry, &marketplace_path)?;

    let plugin_manifest = load_plugin_manifest(&plugin_dir)?;
    let version = plugin_manifest.as_ref().and_then(|m| m.version.clone());
    let skill_dirs = discover_skills_for_plugin(&plugin_dir, plugin_manifest.as_ref());

    let project = KiroProject::new(PathBuf::from(&project_path));
    let svc_result = svc.install_skills(
        &project,
        &skill_dirs,
        &InstallFilter::Names(&skills),
        force,
        &marketplace,
        &plugin,
        version.as_deref(),
    );

    Ok(InstallResult {
        installed: svc_result.installed,
        skipped: svc_result.skipped,
        failed: svc_result
            .failed
            .into_iter()
            .map(|f| FailedSkill {
                name: f.name,
                error: f.error,
            })
            .collect(),
    })
}

/// Get summary information about a Kiro project directory.
#[tauri::command]
#[specta::specta]
pub async fn get_project_info(project_path: String) -> Result<ProjectInfo, CommandError> {
    let path = PathBuf::from(&project_path);
    let kiro_initialized = path.join(".kiro").exists();
    let project = KiroProject::new(path);
    let installed_skill_count = project
        .load_installed()
        .map(|i| i.skills.len() as u32)
        .map_err(|e| {
            warn!(path = %project_path, error = %e, "failed to load installed skills");
            CommandError::new(
                format!("failed to read installed skills: {e}"),
                ErrorType::IoError,
            )
        })?;

    Ok(ProjectInfo {
        path: project_path,
        kiro_initialized,
        installed_skill_count,
    })
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Map a `MarketplaceSource` to a `SourceType`.
fn marketplace_source_type(source: &MarketplaceSource) -> SourceType {
    match source {
        MarketplaceSource::GitHub { .. } => SourceType::GitHub,
        MarketplaceSource::GitUrl { .. } => SourceType::Git,
        MarketplaceSource::LocalPath { .. } => SourceType::Local,
    }
}

/// Map a `PluginSource` to a `SourceType`.
fn plugin_source_type(source: &PluginSource) -> SourceType {
    match source {
        PluginSource::RelativePath(_) => SourceType::Relative,
        PluginSource::Structured(StructuredSource::GitHub { .. }) => SourceType::GitHub,
        PluginSource::Structured(StructuredSource::GitUrl { .. }) => SourceType::Git,
        PluginSource::Structured(StructuredSource::GitSubdir { .. }) => SourceType::GitSubdir,
    }
}

/// Resolve the local directory for a plugin. Only supports `RelativePath` sources;
/// structured (remote) sources return an error since they require git cloning.
fn resolve_local_plugin_dir(
    entry: &PluginEntry,
    marketplace_path: &Path,
) -> Result<PathBuf, CommandError> {
    match &entry.source {
        PluginSource::RelativePath(rel) => {
            let resolved = marketplace_path.join(rel);
            if !resolved.exists() {
                return Err(CommandError::new(
                    format!("plugin directory does not exist: {}", resolved.display()),
                    ErrorType::NotFound,
                ));
            }
            Ok(resolved)
        }
        PluginSource::Structured(_) => Err(CommandError::new(
            format!(
                "plugin '{}' uses a remote source and is not available locally; \
                 use the CLI to clone it first",
                entry.name
            ),
            ErrorType::Validation,
        )),
    }
}

/// Discover skill directories within a plugin, using the provided manifest
/// (if any) to determine skill paths.  Falls back to
/// [`kiro_market_core::DEFAULT_SKILL_PATHS`] when the manifest is `None` or
/// its `skills` list is empty.
fn discover_skills_for_plugin(
    plugin_dir: &Path,
    manifest: Option<&PluginManifest>,
) -> Vec<PathBuf> {
    let skill_paths: Vec<&str> = if let Some(m) = manifest.filter(|m| !m.skills.is_empty()) {
        m.skills.iter().map(String::as_str).collect()
    } else {
        kiro_market_core::DEFAULT_SKILL_PATHS.to_vec()
    };

    discover_skill_dirs(plugin_dir, &skill_paths)
}

/// Load a `plugin.json` from the given directory.
///
/// Returns `Ok(None)` if the file is genuinely missing (not an error) and
/// `Err` if the file exists but could not be read or parsed (corruption /
/// permission issues).
fn load_plugin_manifest(plugin_dir: &Path) -> Result<Option<PluginManifest>, CommandError> {
    let manifest_path = plugin_dir.join("plugin.json");
    let bytes = match fs::read(&manifest_path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            debug!(
                path = %manifest_path.display(),
                "plugin.json not found, using defaults"
            );
            return Ok(None);
        }
        Err(e) => {
            warn!(
                path = %manifest_path.display(),
                error = %e,
                "failed to read plugin.json"
            );
            return Err(CommandError::new(
                format!(
                    "failed to read plugin.json at {}: {e}",
                    manifest_path.display()
                ),
                ErrorType::IoError,
            ));
        }
    };

    match PluginManifest::from_json(&bytes) {
        Ok(manifest) => {
            debug!(name = %manifest.name, "loaded plugin manifest");
            Ok(Some(manifest))
        }
        Err(e) => {
            warn!(
                path = %manifest_path.display(),
                error = %e,
                "plugin.json is malformed"
            );
            Err(CommandError::new(
                format!(
                    "plugin.json at {} is malformed: {e}",
                    manifest_path.display()
                ),
                ErrorType::ParseError,
            ))
        }
    }
}

/// Count skills within a plugin entry. Only counts for local (relative path)
/// plugins; remote plugins report 0.
///
/// A malformed `plugin.json` is logged at `warn` rather than collapsed into
/// "use defaults" so the listing count agrees with `list_available_skills`,
/// which surfaces the parse error to the user. A genuinely missing manifest
/// (the common case) falls back to default skill paths silently.
fn count_plugin_skills(entry: &PluginEntry, marketplace_path: &Path) -> usize {
    match &entry.source {
        PluginSource::RelativePath(rel) => {
            let plugin_dir = marketplace_path.join(rel);
            let manifest = match load_plugin_manifest(&plugin_dir) {
                Ok(opt) => opt,
                Err(e) => {
                    warn!(
                        plugin = %entry.name,
                        path = %plugin_dir.display(),
                        error = %e.message,
                        "could not load plugin.json for skill count; reporting 0"
                    );
                    return 0;
                }
            };
            discover_skills_for_plugin(&plugin_dir, manifest.as_ref()).len()
        }
        PluginSource::Structured(_) => 0,
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use kiro_market_core::cache::MarketplaceSource;
    use kiro_market_core::marketplace::{PluginEntry, PluginSource, StructuredSource};

    use super::*;

    // -----------------------------------------------------------------------
    // marketplace_source_type
    // -----------------------------------------------------------------------

    #[test]
    fn marketplace_source_type_github() {
        let source = MarketplaceSource::GitHub {
            repo: "owner/repo".into(),
        };
        assert!(matches!(
            marketplace_source_type(&source),
            SourceType::GitHub
        ));
    }

    #[test]
    fn marketplace_source_type_git_url() {
        let source = MarketplaceSource::GitUrl {
            url: "https://example.com/repo.git".into(),
        };
        assert!(matches!(marketplace_source_type(&source), SourceType::Git));
    }

    #[test]
    fn marketplace_source_type_local_path() {
        let source = MarketplaceSource::LocalPath {
            path: "/home/user/marketplace".into(),
        };
        assert!(matches!(
            marketplace_source_type(&source),
            SourceType::Local
        ));
    }

    // -----------------------------------------------------------------------
    // plugin_source_type
    // -----------------------------------------------------------------------

    #[test]
    fn plugin_source_type_relative_path() {
        let source = PluginSource::RelativePath("./plugins/dotnet".into());
        assert!(matches!(plugin_source_type(&source), SourceType::Relative));
    }

    #[test]
    fn plugin_source_type_github() {
        let source = PluginSource::Structured(StructuredSource::GitHub {
            repo: "owner/repo".into(),
            git_ref: None,
            sha: None,
        });
        assert!(matches!(plugin_source_type(&source), SourceType::GitHub));
    }

    #[test]
    fn plugin_source_type_git_url() {
        let source = PluginSource::Structured(StructuredSource::GitUrl {
            url: "https://example.com/repo.git".into(),
            git_ref: None,
            sha: None,
        });
        assert!(matches!(plugin_source_type(&source), SourceType::Git));
    }

    #[test]
    fn plugin_source_type_git_subdir() {
        let source = PluginSource::Structured(StructuredSource::GitSubdir {
            url: "https://example.com/repo.git".into(),
            path: "plugins/foo".into(),
            git_ref: None,
            sha: None,
        });
        assert!(matches!(plugin_source_type(&source), SourceType::GitSubdir));
    }

    // -----------------------------------------------------------------------
    // resolve_local_plugin_dir
    // -----------------------------------------------------------------------

    #[test]
    fn resolve_local_plugin_dir_relative_path_exists() {
        let tmp = tempdir().expect("failed to create tempdir");
        let plugin_dir = tmp.path().join("plugins").join("my-plugin");
        fs::create_dir_all(&plugin_dir).expect("failed to create plugin dir");

        let entry = PluginEntry {
            name: "my-plugin".into(),
            description: None,
            source: PluginSource::RelativePath("plugins/my-plugin".into()),
        };

        let result = resolve_local_plugin_dir(&entry, tmp.path());
        assert!(result.is_ok());
        assert_eq!(result.expect("should resolve"), plugin_dir);
    }

    #[test]
    fn resolve_local_plugin_dir_relative_path_not_found() {
        let tmp = tempdir().expect("failed to create tempdir");

        let entry = PluginEntry {
            name: "missing-plugin".into(),
            description: None,
            source: PluginSource::RelativePath("plugins/missing-plugin".into()),
        };

        let result = resolve_local_plugin_dir(&entry, tmp.path());
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.error_type, ErrorType::NotFound);
        assert!(
            err.message.contains("does not exist"),
            "expected 'does not exist' in message, got: {}",
            err.message
        );
    }

    #[test]
    fn resolve_local_plugin_dir_structured_returns_validation_error() {
        let tmp = tempdir().expect("failed to create tempdir");

        let entry = PluginEntry {
            name: "remote-plugin".into(),
            description: None,
            source: PluginSource::Structured(StructuredSource::GitHub {
                repo: "owner/repo".into(),
                git_ref: None,
                sha: None,
            }),
        };

        let result = resolve_local_plugin_dir(&entry, tmp.path());
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.error_type, ErrorType::Validation);
        assert!(
            err.message.contains("remote source"),
            "expected 'remote source' in message, got: {}",
            err.message
        );
    }
}

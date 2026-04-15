//! Browse commands for marketplace/plugin/skill discovery.

use std::fs;
use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::Serialize;
use tracing::{debug, warn};

use kiro_market_core::cache::{CacheDir, MarketplaceSource};
use kiro_market_core::error::{Error as CoreError, SkillError};
use kiro_market_core::marketplace::{Marketplace, PluginEntry, PluginSource, StructuredSource};
use kiro_market_core::plugin::{discover_skill_dirs, PluginManifest};
use kiro_market_core::project::{InstalledSkillMeta, KiroProject};
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

/// List all registered marketplaces with plugin counts.
#[tauri::command]
#[specta::specta]
pub async fn list_marketplaces() -> Result<Vec<MarketplaceInfo>, CommandError> {
    let cache = CacheDir::default_location().ok_or_else(|| {
        CommandError::new(
            "could not determine data directory; is $HOME set?",
            ErrorType::IoError,
        )
    })?;

    let known = cache
        .load_known_marketplaces()
        .map_err(CommandError::from)?;

    let mut results = Vec::with_capacity(known.len());
    for entry in &known {
        let source_type = marketplace_source_type(&entry.source);
        let (plugin_count, load_error) = match count_marketplace_plugins(&cache, &entry.name) {
            Ok(count) => (count as u32, None),
            Err(msg) => (0, Some(msg)),
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
    let cache = CacheDir::default_location().ok_or_else(|| {
        CommandError::new(
            "could not determine data directory; is $HOME set?",
            ErrorType::IoError,
        )
    })?;

    let marketplace_path = cache.marketplace_path(&marketplace);
    let manifest = load_marketplace_manifest(&marketplace_path, &marketplace)?;

    let mut results = Vec::with_capacity(manifest.plugins.len());
    for plugin in &manifest.plugins {
        let source_type = plugin_source_type(&plugin.source);
        let skill_count = count_plugin_skills(plugin, &marketplace_path);
        results.push(PluginInfo {
            name: plugin.name.clone(),
            description: plugin.description.clone(),
            skill_count: skill_count as u32,
            source_type,
        });
    }

    // Merge in discovered plugins that are not listed in the manifest.
    let discovered = kiro_market_core::plugin::discover_plugins(&marketplace_path, 3);
    for dp in &discovered {
        let dp_path = dp.relative_path_unix();
        let already_listed = manifest.plugins.iter().any(|p| {
            p.name == dp.name()
                || matches!(&p.source, PluginSource::RelativePath(rel) if {
                    let normalized = rel
                        .trim_start_matches("./")
                        .trim_start_matches(".\\")
                        .trim_end_matches(['/', '\\'])
                        .replace('\\', "/");
                    normalized == dp_path
                })
        });
        if !already_listed {
            let plugin_dir = marketplace_path.join(dp.relative_path());
            let manifest = load_plugin_manifest(&plugin_dir).ok().flatten();
            let skill_count =
                discover_skills_for_plugin(&plugin_dir, manifest.as_ref()).len();
            results.push(PluginInfo {
                name: dp.name().to_owned(),
                description: dp.description().map(String::from),
                skill_count: skill_count as u32,
                source_type: SourceType::Relative,
            });
        }
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
    let cache = CacheDir::default_location().ok_or_else(|| {
        CommandError::new(
            "could not determine data directory; is $HOME set?",
            ErrorType::IoError,
        )
    })?;

    let marketplace_path = cache.marketplace_path(&marketplace);
    let manifest = load_marketplace_manifest(&marketplace_path, &marketplace)?;

    let plugin_entry = manifest
        .plugins
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
    let cache = CacheDir::default_location().ok_or_else(|| {
        CommandError::new(
            "could not determine data directory; is $HOME set?",
            ErrorType::IoError,
        )
    })?;

    let marketplace_path = cache.marketplace_path(&marketplace);
    let manifest = load_marketplace_manifest(&marketplace_path, &marketplace)?;

    let plugin_entry = manifest
        .plugins
        .iter()
        .find(|p| p.name == plugin)
        .ok_or_else(|| {
            CommandError::new(
                format!("plugin '{plugin}' not found in marketplace '{marketplace}'"),
                ErrorType::NotFound,
            )
        })?;

    let plugin_dir = resolve_local_plugin_dir(plugin_entry, &marketplace_path)?;

    // Load the plugin manifest once and reuse for both skill discovery and
    // version extraction (fixes the previous double-read).
    let plugin_manifest = load_plugin_manifest(&plugin_dir)?;
    let version = plugin_manifest.as_ref().and_then(|m| m.version.clone());
    let skill_dirs = discover_skills_for_plugin(&plugin_dir, plugin_manifest.as_ref());

    let project = KiroProject::new(PathBuf::from(&project_path));

    let mut result = InstallResult {
        installed: Vec::new(),
        skipped: Vec::new(),
        failed: Vec::new(),
    };

    // Track which requested skill names were actually encountered so we can
    // report unmatched ones at the end.
    let mut processed_skills: std::collections::HashSet<String> =
        std::collections::HashSet::with_capacity(skills.len());

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
            Ok(r) => r,
            Err(e) => {
                warn!(
                    path = %skill_md_path.display(),
                    error = %e,
                    "failed to parse SKILL.md frontmatter, skipping"
                );
                continue;
            }
        };

        if !skills.contains(&frontmatter.name) {
            continue;
        }

        processed_skills.insert(frontmatter.name.clone());

        let meta = InstalledSkillMeta {
            marketplace: marketplace.clone(),
            plugin: plugin.clone(),
            version: version.clone(),
            installed_at: Utc::now(),
        };

        let install_outcome = if force {
            project.install_skill_from_dir_force(&frontmatter.name, skill_dir, meta)
        } else {
            project.install_skill_from_dir(&frontmatter.name, skill_dir, meta)
        };

        match install_outcome {
            Ok(()) => {
                debug!(skill = %frontmatter.name, "skill installed successfully");
                result.installed.push(frontmatter.name);
            }
            Err(CoreError::Skill(SkillError::AlreadyInstalled { .. })) => {
                debug!(skill = %frontmatter.name, "skill already installed, skipping");
                result.skipped.push(frontmatter.name);
            }
            Err(e) => {
                warn!(skill = %frontmatter.name, error = %e, "failed to install skill");
                result.failed.push(FailedSkill {
                    name: frontmatter.name,
                    error: e.to_string(),
                });
            }
        }
    }

    // Report any requested skills that were not found in this plugin.
    for skill_name in &skills {
        if !processed_skills.contains(skill_name) {
            warn!(skill = %skill_name, plugin = %plugin, "requested skill not found in plugin");
            result.failed.push(FailedSkill {
                name: skill_name.clone(),
                error: format!("skill '{skill_name}' not found in plugin '{plugin}'"),
            });
        }
    }

    Ok(result)
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

/// Count the number of plugins in a marketplace by reading its manifest.
///
/// Returns `Ok(count)` on success, or `Err(message)` if the manifest could
/// not be read or parsed.  The caller should set `plugin_count` to 0 and
/// surface the error via `MarketplaceInfo::load_error`.
fn count_marketplace_plugins(cache: &CacheDir, marketplace_name: &str) -> Result<usize, String> {
    let marketplace_path = cache.marketplace_path(marketplace_name);
    let manifest_count = count_manifest_plugins(&marketplace_path, marketplace_name);
    let discovered = kiro_market_core::plugin::discover_plugins(&marketplace_path, 3);

    match manifest_count {
        Ok(n) => Ok(n + count_unlisted_plugins(&marketplace_path, &discovered)),
        // No manifest — use discovered count directly.
        Err(_) if !discovered.is_empty() => Ok(discovered.len()),
        Err(e) => Err(e),
    }
}

/// Count plugins listed in the manifest only.
fn count_manifest_plugins(marketplace_path: &Path, marketplace_name: &str) -> Result<usize, String> {
    let manifest_path = marketplace_path.join(kiro_market_core::MARKETPLACE_MANIFEST_PATH);

    let bytes = match fs::read(&manifest_path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            debug!(
                marketplace = marketplace_name,
                "marketplace manifest not found"
            );
            return Err("no manifest".into());
        }
        Err(e) => {
            warn!(
                marketplace = marketplace_name,
                error = %e,
                "failed to read marketplace manifest"
            );
            return Err(format!("failed to read marketplace manifest: {e}"));
        }
    };

    match Marketplace::from_json(&bytes) {
        Ok(m) => Ok(m.plugins.len()),
        Err(e) => {
            warn!(
                marketplace = marketplace_name,
                error = %e,
                "failed to parse marketplace manifest"
            );
            Err(format!("failed to parse marketplace manifest: {e}"))
        }
    }
}

/// Count discovered plugins that are NOT already listed in the manifest.
fn count_unlisted_plugins(
    marketplace_path: &Path,
    discovered: &[kiro_market_core::plugin::DiscoveredPlugin],
) -> usize {
    let manifest_path = marketplace_path.join(kiro_market_core::MARKETPLACE_MANIFEST_PATH);
    let manifest = fs::read(&manifest_path)
        .ok()
        .and_then(|b| Marketplace::from_json(&b).ok());

    let Some(manifest) = manifest else {
        return discovered.len();
    };

    let listed_paths: Vec<String> = manifest
        .plugins
        .iter()
        .filter_map(|p| match &p.source {
            PluginSource::RelativePath(rel) => {
                Some(
                    rel.trim_start_matches("./")
                        .trim_start_matches(".\\")
                        .trim_end_matches(['/', '\\'])
                        .replace('\\', "/"),
                )
            }
            PluginSource::Structured(_) => None,
        })
        .collect();

    let listed_names: Vec<&str> = manifest.plugins.iter().map(|p| p.name.as_str()).collect();

    discovered
        .iter()
        .filter(|dp| {
            let dp_path = dp.relative_path_unix();
            !listed_paths.contains(&dp_path) && !listed_names.contains(&dp.name())
        })
        .count()
}

/// Load and parse a marketplace manifest, returning a `CommandError` on failure.
fn load_marketplace_manifest(
    marketplace_path: &Path,
    marketplace_name: &str,
) -> Result<Marketplace, CommandError> {
    let manifest_path = marketplace_path.join(kiro_market_core::MARKETPLACE_MANIFEST_PATH);

    let bytes = fs::read(&manifest_path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            CommandError::new(
                format!("marketplace '{marketplace_name}' not found or has no manifest"),
                ErrorType::NotFound,
            )
        } else {
            CommandError::new(
                format!(
                    "failed to read marketplace manifest at {}: {e}",
                    manifest_path.display()
                ),
                ErrorType::IoError,
            )
        }
    })?;

    Marketplace::from_json(&bytes).map_err(|e| {
        CommandError::new(
            format!("failed to parse marketplace manifest for '{marketplace_name}': {e}"),
            ErrorType::ParseError,
        )
    })
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
fn count_plugin_skills(entry: &PluginEntry, marketplace_path: &Path) -> usize {
    match &entry.source {
        PluginSource::RelativePath(rel) => {
            let plugin_dir = marketplace_path.join(rel);
            // Best-effort: use the manifest if available, fall back to defaults.
            let manifest = load_plugin_manifest(&plugin_dir).ok().flatten();
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

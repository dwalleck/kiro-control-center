//! Browse commands for marketplace/plugin/skill discovery.

use std::fs;
use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::Serialize;
use tracing::{debug, warn};

use kiro_market_core::cache::{CacheDir, MarketplaceSource};
use kiro_market_core::error::{Error as CoreError, SkillError};
use kiro_market_core::marketplace::{Marketplace, PluginEntry, PluginSource, StructuredSource};
use kiro_market_core::plugin::{PluginManifest, discover_skill_dirs};
use kiro_market_core::project::{InstalledSkillMeta, KiroProject};
use kiro_market_core::skill::{extract_relative_md_links, merge_skill, parse_frontmatter};

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

    let known = cache.load_known_marketplaces().map_err(CommandError::from)?;

    let mut results = Vec::with_capacity(known.len());
    for entry in &known {
        let source_type = marketplace_source_type(&entry.source);
        let plugin_count = count_marketplace_plugins(&cache, &entry.name);
        results.push(MarketplaceInfo {
            name: entry.name.clone(),
            source_type,
            plugin_count: plugin_count as u32,
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
    let skill_dirs = discover_skills_for_plugin(&plugin_dir);

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
    let skill_dirs = discover_skills_for_plugin(&plugin_dir);
    let plugin_manifest = load_plugin_manifest(&plugin_dir);
    let version = plugin_manifest.as_ref().and_then(|m| m.version.clone());

    let project = KiroProject::new(PathBuf::from(&project_path));

    let mut result = InstallResult {
        installed: Vec::new(),
        skipped: Vec::new(),
        failed: Vec::new(),
    };

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

        let (frontmatter, body_offset) = match parse_frontmatter(&content) {
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

        let merged_content = match prepare_merged_content(&content, body_offset, skill_dir) {
            Ok(c) => c,
            Err(e) => {
                result.failed.push(FailedSkill {
                    name: frontmatter.name,
                    error: e,
                });
                continue;
            }
        };

        let meta = InstalledSkillMeta {
            marketplace: marketplace.clone(),
            plugin: plugin.clone(),
            version: version.clone(),
            installed_at: Utc::now(),
        };

        let install_outcome = if force {
            project.install_skill_force(&frontmatter.name, &merged_content, meta)
        } else {
            project.install_skill(&frontmatter.name, &merged_content, meta)
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
/// Returns 0 if the manifest cannot be read or parsed, logging a warning.
fn count_marketplace_plugins(cache: &CacheDir, marketplace_name: &str) -> usize {
    let marketplace_path = cache.marketplace_path(marketplace_name);
    let manifest_path = marketplace_path.join(kiro_market_core::MARKETPLACE_MANIFEST_PATH);

    match fs::read(&manifest_path) {
        Ok(bytes) => match Marketplace::from_json(&bytes) {
            Ok(m) => m.plugins.len(),
            Err(e) => {
                warn!(
                    marketplace = marketplace_name,
                    error = %e,
                    "failed to parse marketplace manifest, reporting 0 plugins"
                );
                0
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            debug!(
                marketplace = marketplace_name,
                "marketplace manifest not found, reporting 0 plugins"
            );
            0
        }
        Err(e) => {
            warn!(
                marketplace = marketplace_name,
                error = %e,
                "failed to read marketplace manifest, reporting 0 plugins"
            );
            0
        }
    }
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
                    format!(
                        "plugin directory does not exist: {}",
                        resolved.display()
                    ),
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

/// Discover skill directories within a plugin, using its manifest or defaults.
fn discover_skills_for_plugin(plugin_dir: &Path) -> Vec<PathBuf> {
    let manifest = load_plugin_manifest(plugin_dir);
    let skill_paths: Vec<&str> =
        if let Some(m) = manifest.as_ref().filter(|m| !m.skills.is_empty()) {
            m.skills.iter().map(String::as_str).collect()
        } else {
            kiro_market_core::DEFAULT_SKILL_PATHS.to_vec()
        };

    discover_skill_dirs(plugin_dir, &skill_paths)
}

/// Load a `plugin.json` from the given directory, returning `None` if missing or malformed.
fn load_plugin_manifest(plugin_dir: &Path) -> Option<PluginManifest> {
    let manifest_path = plugin_dir.join("plugin.json");
    match fs::read(&manifest_path) {
        Ok(bytes) => match PluginManifest::from_json(&bytes) {
            Ok(manifest) => {
                debug!(name = %manifest.name, "loaded plugin manifest");
                Some(manifest)
            }
            Err(e) => {
                warn!(
                    path = %manifest_path.display(),
                    error = %e,
                    "plugin.json is malformed, falling back to defaults"
                );
                None
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            debug!(
                path = %manifest_path.display(),
                "plugin.json not found, using defaults"
            );
            None
        }
        Err(e) => {
            warn!(
                path = %manifest_path.display(),
                error = %e,
                "failed to read plugin.json, falling back to defaults"
            );
            None
        }
    }
}

/// Count skills within a plugin entry. Only counts for local (relative path)
/// plugins; remote plugins report 0.
fn count_plugin_skills(entry: &PluginEntry, marketplace_path: &Path) -> usize {
    match &entry.source {
        PluginSource::RelativePath(rel) => {
            let plugin_dir = marketplace_path.join(rel);
            discover_skills_for_plugin(&plugin_dir).len()
        }
        PluginSource::Structured(_) => 0,
    }
}

/// Read companion files, merge them into the skill content, and return the
/// merged result. Returns an error string on failure.
fn prepare_merged_content(
    skill_content: &str,
    body_offset: usize,
    skill_dir: &Path,
) -> Result<String, String> {
    let body = &skill_content[body_offset..];
    let relative_links = extract_relative_md_links(body);

    let mut companions: Vec<(String, String)> = Vec::new();
    for link in &relative_links {
        let companion_path = skill_dir.join(link);
        match fs::read_to_string(&companion_path) {
            Ok(content) => companions.push((link.clone(), content)),
            Err(e) => {
                return Err(format!(
                    "companion file '{}' referenced by SKILL.md could not be read: {e}",
                    companion_path.display()
                ));
            }
        }
    }

    let companion_refs: Vec<(&str, &str)> = companions
        .iter()
        .map(|(path, content)| (path.as_str(), content.as_str()))
        .collect();

    merge_skill(skill_content, &companion_refs)
        .map_err(|e| format!("failed to merge skill content: {e}"))
}

//! Browse commands for marketplace/plugin/skill discovery.

use std::path::PathBuf;

use serde::Serialize;
use tracing::warn;

use kiro_market_core::cache::{CacheDir, MarketplaceSource};
use kiro_market_core::error::error_full_chain;
use kiro_market_core::git::GixCliBackend;
use kiro_market_core::marketplace::{PluginSource, StructuredSource};
use kiro_market_core::project::{InstalledSkills, KiroProject};
use kiro_market_core::service::{
    BulkSkillsResult, InstallFilter, InstallMode, InstallSkillsResult, MarketplaceService,
    PluginSkillsResult, SkillCount,
};

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
    pub skill_count: SkillCount,
    pub source_type: SourceType,
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
            Ok(entries) => (saturate_to_u32(entries.len(), "plugin_count"), None),
            Err(e) => {
                let detail = error_full_chain(&e);
                warn!(
                    marketplace = %entry.name,
                    error = %detail,
                    "failed to list plugin entries for marketplace summary"
                );
                (0, Some(detail))
            }
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
        results.push(PluginInfo {
            name: plugin.name.clone(),
            description: plugin.description.clone(),
            skill_count: svc.count_skills_for_plugin(plugin, &marketplace_path),
            source_type,
        });
    }

    Ok(results)
}

/// List all available skills for a plugin, cross-referenced with installed state.
///
/// Returns a [`PluginSkillsResult`] carrying both the happy-path skill list
/// and any per-skill read failures (unreadable `SKILL.md`, malformed
/// frontmatter) as [`PluginSkillsResult::skipped_skills`]. Previously these
/// per-skill failures vanished into `warn!` logs, leaving the frontend to
/// wonder why the count shrank; surfacing them structurally lets the UI
/// show "N skills failed to load" with a drill-down.
#[tauri::command]
#[specta::specta]
pub async fn list_available_skills(
    marketplace: String,
    plugin: String,
    project_path: String,
) -> Result<PluginSkillsResult, CommandError> {
    let svc = make_service()?;
    let project = KiroProject::new(PathBuf::from(&project_path));
    let installed = load_installed_or_error(&project, &project_path)?;
    svc.list_skills_for_plugin(&marketplace, &plugin, &installed)
        .map_err(CommandError::from)
}

/// List all skills across every plugin in a marketplace, cross-referenced
/// with installed state.
///
/// Bulk alternative to calling [`list_available_skills`] per plugin when no
/// plugin filter is active. The returned [`BulkSkillsResult::skipped`]
/// carries plugin-level errors (missing directory, malformed manifest,
/// remote source) so the frontend can surface a partial-listing warning.
#[tauri::command]
#[specta::specta]
pub async fn list_all_skills_for_marketplace(
    marketplace: String,
    project_path: String,
) -> Result<BulkSkillsResult, CommandError> {
    let svc = make_service()?;
    let project = KiroProject::new(PathBuf::from(&project_path));
    let installed = load_installed_or_error(&project, &project_path)?;
    svc.list_all_skills(&marketplace, &installed)
        .map_err(CommandError::from)
}

/// Install specific skills from a plugin into a Kiro project.
///
/// Returns the core [`InstallSkillsResult`] directly rather than through a
/// Tauri-local wrapper — the previous wrapper was a field-by-field copy
/// and risked drifting away from the core shape (e.g. losing the
/// structured `FailedSkill::kind` and `skipped_skills` fields).
/// Using the core type keeps the wire format lockstep with what the
/// service emits.
#[tauri::command]
#[specta::specta]
pub async fn install_skills(
    marketplace: String,
    plugin: String,
    skills: Vec<String>,
    force: bool,
    project_path: String,
) -> Result<InstallSkillsResult, CommandError> {
    let svc = make_service()?;
    let ctx = svc
        .resolve_plugin_install_context(&marketplace, &plugin)
        .map_err(CommandError::from)?;
    let project = KiroProject::new(PathBuf::from(&project_path));
    Ok(svc.install_skills(
        &project,
        &ctx.skill_dirs,
        &InstallFilter::Names(&skills),
        InstallMode::from(force),
        &marketplace,
        &plugin,
        ctx.version.as_deref(),
    ))
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
        .map(|i| saturate_to_u32(i.skills.len(), "installed_skill_count"))
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

/// Load the project's installed-skills tracking file, wrapping I/O errors
/// with an actionable message that names the path and hints at the next
/// step. Without this extra framing the frontend sees bare `serde_json` or
/// filesystem error text that doesn't tell users where the problem lives.
fn load_installed_or_error(
    project: &KiroProject,
    project_path: &str,
) -> Result<InstalledSkills, CommandError> {
    project.load_installed().map_err(|e| {
        warn!(path = %project_path, error = %e, "failed to load installed skills");
        CommandError::new(
            format!(
                "failed to read installed skills for project at '{project_path}': {e}. \
                 Check that .kiro/installed.json exists and is readable."
            ),
            ErrorType::IoError,
        )
    })
}

/// Narrow a `usize` count into a `u32` for the serialized frontend
/// response, saturating at `u32::MAX` if the count overflows.
///
/// A count above `u32::MAX` means registry corruption or runaway
/// discovery, not a legitimate value — so the overflow arm logs
/// the original `usize` with the `field` name that overflowed, rather
/// than silently pegging the UI at `4294967295` with no trace.
fn saturate_to_u32(count: usize, field: &'static str) -> u32 {
    u32::try_from(count)
        .inspect_err(|_| {
            warn!(
                field,
                original = count,
                "count exceeds u32::MAX, saturating"
            );
        })
        .unwrap_or(u32::MAX)
}

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

#[cfg(test)]
mod tests {
    use kiro_market_core::cache::MarketplaceSource;
    use kiro_market_core::marketplace::{PluginSource, StructuredSource};

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
        let source = PluginSource::RelativePath(
            kiro_market_core::validation::RelativePath::new("./plugins/dotnet").unwrap(),
        );
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
            path: kiro_market_core::validation::RelativePath::new("plugins/foo").unwrap(),
            git_ref: None,
            sha: None,
        });
        assert!(matches!(plugin_source_type(&source), SourceType::GitSubdir));
    }

    // -----------------------------------------------------------------------
    // saturate_to_u32
    // -----------------------------------------------------------------------

    #[test]
    fn saturate_to_u32_passes_through_in_range_values() {
        assert_eq!(saturate_to_u32(0, "test"), 0);
        assert_eq!(saturate_to_u32(42, "test"), 42);
        assert_eq!(saturate_to_u32(u32::MAX as usize, "test"), u32::MAX);
    }

    /// On 32-bit targets `usize::MAX == u32::MAX` so `try_from` never fails
    /// and the overflow arm is unreachable. Gate the overflow test to 64-bit
    /// where the arm is actually exercised.
    #[cfg(target_pointer_width = "64")]
    #[test]
    fn saturate_to_u32_clamps_values_above_u32_max() {
        assert_eq!(saturate_to_u32((u32::MAX as usize) + 1, "test"), u32::MAX);
        assert_eq!(saturate_to_u32(usize::MAX, "test"), u32::MAX);
    }
}

//! `install` command: install a plugin or specific skill into a Kiro project.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use chrono::Utc;
use colored::Colorize;
use kiro_market_core::cache::CacheDir;
use kiro_market_core::git::GitProtocol;
use kiro_market_core::error::{Error as CoreError, SkillError};
use kiro_market_core::git;
use kiro_market_core::marketplace::{PluginEntry, PluginSource, StructuredSource};
use kiro_market_core::plugin::{PluginManifest, discover_skill_dirs};
use kiro_market_core::project::{InstalledSkillMeta, KiroProject};
use kiro_market_core::skill::{extract_relative_md_links, merge_skill, parse_frontmatter};
use tracing::{debug, warn};

use crate::cli;

/// Tracks installation results across skills.
struct InstallStats {
    installed: u32,
    skipped: u32,
    failed: u32,
}

/// Run the install command.
///
/// Resolves `plugin_ref` to a plugin, discovers skills, merges companions,
/// and installs into the current Kiro project.
pub fn run(plugin_ref: &str, skill_filter: Option<&str>, force: bool) -> Result<()> {
    let (plugin_name, marketplace_name) = cli::parse_plugin_ref(plugin_ref).with_context(|| {
        format!("invalid plugin reference '{plugin_ref}': expected plugin@marketplace")
    })?;

    let cache = CacheDir::default_location()
        .context("could not determine data directory; is $HOME set?")?;
    let marketplace_path = cache.marketplace_path(marketplace_name);
    if !marketplace_path.exists() {
        bail!(
            "marketplace '{}' not found. Run {} first.",
            marketplace_name,
            "kiro-market marketplace add".bold()
        );
    }

    // Look up the stored protocol preference for this marketplace.
    let protocol = match cache.load_known_marketplaces() {
        Ok(entries) => entries
            .into_iter()
            .find(|e| e.name == marketplace_name)
            .and_then(|e| e.protocol)
            .unwrap_or_default(),
        Err(e) => {
            warn!(
                marketplace = marketplace_name,
                error = %e,
                "failed to load marketplace registry; defaulting to HTTPS protocol"
            );
            GitProtocol::default()
        }
    };

    let plugin_entry =
        super::common::find_plugin_entry(&marketplace_path, plugin_name, marketplace_name)?;

    let plugin_dir = resolve_plugin_dir(
        &plugin_entry,
        &marketplace_path,
        &cache,
        marketplace_name,
        protocol,
    )
        .with_context(|| format!("failed to resolve plugin directory for '{plugin_name}'"))?;

    debug!(plugin_dir = %plugin_dir.display(), "resolved plugin directory");

    let plugin_manifest = load_plugin_manifest(&plugin_dir);
    let skill_dirs = discover_plugin_skills(&plugin_dir, plugin_manifest.as_ref());

    if skill_dirs.is_empty() {
        bail!("no skills found in plugin '{plugin_name}'");
    }

    let cwd = std::env::current_dir().context("failed to determine current directory")?;
    let project = KiroProject::new(cwd);
    let version = plugin_manifest.as_ref().and_then(|m| m.version.clone());

    let stats = install_skills(
        &skill_dirs,
        skill_filter,
        force,
        &project,
        marketplace_name,
        plugin_name,
        version.as_ref(),
    );

    print_summary(plugin_ref, &stats);

    if stats.installed == 0 && stats.skipped == 0 {
        bail!("no skills were installed from '{plugin_ref}'");
    }

    Ok(())
}

/// Discover skill directories from a plugin, using its manifest or defaults.
fn discover_plugin_skills(
    plugin_dir: &Path,
    plugin_manifest: Option<&PluginManifest>,
) -> Vec<PathBuf> {
    let skill_paths: Vec<&str> =
        if let Some(manifest) = plugin_manifest.filter(|m| !m.skills.is_empty()) {
            manifest.skills.iter().map(String::as_str).collect()
        } else {
            kiro_market_core::DEFAULT_SKILL_PATHS.to_vec()
        };

    discover_skill_dirs(plugin_dir, &skill_paths)
}

/// Install each discovered skill into the project.
fn install_skills(
    skill_dirs: &[PathBuf],
    skill_filter: Option<&str>,
    force: bool,
    project: &KiroProject,
    marketplace_name: &str,
    plugin_name: &str,
    version: Option<&String>,
) -> InstallStats {
    let mut stats = InstallStats {
        installed: 0,
        skipped: 0,
        failed: 0,
    };

    for skill_dir in skill_dirs {
        match process_skill(
            skill_dir,
            skill_filter,
            force,
            project,
            marketplace_name,
            plugin_name,
            version,
        ) {
            SkillResult::Installed => stats.installed += 1,
            SkillResult::Skipped => stats.skipped += 1,
            SkillResult::Failed => stats.failed += 1,
            SkillResult::Filtered => {}
        }
    }

    stats
}

/// Outcome of processing a single skill directory.
enum SkillResult {
    Installed,
    Skipped,
    Failed,
    Filtered,
}

/// Process and install a single skill from its directory.
fn process_skill(
    skill_dir: &Path,
    skill_filter: Option<&str>,
    force: bool,
    project: &KiroProject,
    marketplace_name: &str,
    plugin_name: &str,
    version: Option<&String>,
) -> SkillResult {
    let skill_md_path = skill_dir.join("SKILL.md");
    let skill_content = match fs::read_to_string(&skill_md_path) {
        Ok(content) => content,
        Err(e) => {
            eprintln!(
                "  {} Failed to read {}: {e}",
                "✗".red().bold(),
                skill_md_path.display()
            );
            return SkillResult::Failed;
        }
    };

    let (frontmatter, body_offset) = match parse_frontmatter(&skill_content) {
        Ok(result) => result,
        Err(e) => {
            eprintln!(
                "  {} Failed to parse SKILL.md in {}: {e}",
                "✗".red().bold(),
                skill_dir.display()
            );
            return SkillResult::Failed;
        }
    };

    // Apply skill filter if provided.
    if skill_filter.is_some_and(|filter| frontmatter.name != filter) {
        debug!(
            skill = %frontmatter.name,
            "skipping skill (does not match filter)"
        );
        return SkillResult::Filtered;
    }

    // Extract relative md links and read companion files.
    let body = &skill_content[body_offset..];
    let relative_links = extract_relative_md_links(body);

    let companions: Vec<(String, String)> = relative_links
        .iter()
        .filter_map(|link| {
            let companion_path = skill_dir.join(link);
            match fs::read_to_string(&companion_path) {
                Ok(content) => Some((link.clone(), content)),
                Err(e) => {
                    debug!(link, error = %e, "companion file not found, skipping");
                    None
                }
            }
        })
        .collect();

    let companion_refs: Vec<(&str, &str)> = companions
        .iter()
        .map(|(path, content)| (path.as_str(), content.as_str()))
        .collect();

    let Ok(merged_content) = merge_skill(&skill_content, &companion_refs) else {
        eprintln!(
            "  {} Failed to merge skill '{}'",
            "✗".red().bold(),
            frontmatter.name
        );
        return SkillResult::Failed;
    };

    let meta = InstalledSkillMeta {
        marketplace: marketplace_name.to_owned(),
        plugin: plugin_name.to_owned(),
        version: version.cloned(),
        installed_at: Utc::now(),
    };

    let install_result = if force {
        project.install_skill_force(&frontmatter.name, &merged_content, meta)
    } else {
        project.install_skill(&frontmatter.name, &merged_content, meta)
    };

    match install_result {
        Ok(()) => {
            println!(
                "  {} Installed skill '{}'",
                "✓".green().bold(),
                frontmatter.name.bold()
            );
            SkillResult::Installed
        }
        Err(CoreError::Skill(SkillError::AlreadyInstalled { .. })) => {
            println!(
                "  {} Skill '{}' already installed (use --force to overwrite)",
                "·".yellow().bold(),
                frontmatter.name.bold()
            );
            SkillResult::Skipped
        }
        Err(e) => {
            eprintln!(
                "  {} Failed to install skill '{}': {e}",
                "✗".red().bold(),
                frontmatter.name
            );
            SkillResult::Failed
        }
    }
}

/// Print the installation summary.
fn print_summary(plugin_ref: &str, stats: &InstallStats) {
    println!();
    if stats.installed > 0 {
        println!(
            "{} Installed {} skill{} from {}",
            "✓".green().bold(),
            stats.installed,
            if stats.installed == 1 { "" } else { "s" },
            plugin_ref.bold()
        );
    }
    if stats.skipped > 0 {
        println!(
            "{} Skipped {} already-installed skill{}",
            "·".yellow().bold(),
            stats.skipped,
            if stats.skipped == 1 { "" } else { "s" }
        );
    }
    if stats.failed > 0 {
        println!(
            "{} {} skill{} failed",
            "✗".red().bold(),
            stats.failed,
            if stats.failed == 1 { "" } else { "s" }
        );
    }
}

/// Resolve the on-disk directory for a plugin based on its source.
fn resolve_plugin_dir(
    entry: &PluginEntry,
    marketplace_path: &Path,
    cache: &CacheDir,
    marketplace_name: &str,
    protocol: GitProtocol,
) -> Result<PathBuf> {
    match &entry.source {
        PluginSource::RelativePath(rel) => {
            let resolved = marketplace_path.join(rel);
            if !resolved.exists() {
                bail!("plugin directory does not exist: {}", resolved.display());
            }
            Ok(resolved)
        }
        PluginSource::Structured(structured) => {
            resolve_structured_source(structured, cache, marketplace_name, &entry.name, protocol)
        }
    }
}

/// Clone a structured source into the cache plugins directory and return the path.
fn resolve_structured_source(
    source: &StructuredSource,
    cache: &CacheDir,
    marketplace_name: &str,
    plugin_name: &str,
    protocol: GitProtocol,
) -> Result<PathBuf> {
    cache
        .ensure_dirs()
        .context("failed to create cache directories")?;

    let dest = cache.plugin_path(marketplace_name, plugin_name);

    // Extract the varying parts from each source variant.
    let (url, subdir, git_ref, sha, label) = match source {
        StructuredSource::GitHub { repo, git_ref, sha } => (
            git::github_repo_to_url(repo, protocol),
            None,
            git_ref.as_deref(),
            sha.as_deref(),
            repo.as_str(),
        ),
        StructuredSource::GitUrl { url, git_ref, sha } => (
            url.clone(),
            None,
            git_ref.as_deref(),
            sha.as_deref(),
            url.as_str(),
        ),
        StructuredSource::GitSubdir {
            url,
            path,
            git_ref,
            sha,
        } => (
            url.clone(),
            Some(path.as_str()),
            git_ref.as_deref(),
            sha.as_deref(),
            url.as_str(),
        ),
    };

    // If already cloned, reuse the existing directory.
    if dest.exists() {
        debug!(dest = %dest.display(), "plugin already cached, reusing");
        return Ok(match subdir {
            Some(path) => dest.join(path),
            None => dest,
        });
    }

    debug!(url = %url, dest = %dest.display(), "cloning plugin");
    print!("  Cloning {label}...");
    git::clone_repo(&url, &dest, git_ref)
        .with_context(|| format!("failed to clone plugin from '{label}'"))?;
    println!(" done");

    if let Some(expected) = sha {
        git::verify_sha(&dest, expected)
            .with_context(|| format!("SHA verification failed for '{label}'"))?;
    }

    Ok(match subdir {
        Some(path) => dest.join(path),
        None => dest,
    })
}

/// Load a `plugin.json` from the given directory, returning `None` if missing.
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

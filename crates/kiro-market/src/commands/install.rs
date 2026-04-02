//! `install` command: install a plugin or specific skill into a Kiro project.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use chrono::Utc;
use colored::Colorize;
use kiro_market_core::cache::CacheDir;
use kiro_market_core::git;
use kiro_market_core::marketplace::{Marketplace, PluginEntry, PluginSource, StructuredSource};
use kiro_market_core::plugin::{PluginManifest, discover_skill_dirs};
use kiro_market_core::project::{InstalledSkillMeta, KiroProject};
use kiro_market_core::skill::{extract_relative_md_links, merge_skill, parse_frontmatter};
use tracing::debug;

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

    let plugin_entry = find_plugin_entry(&marketplace_path, plugin_name, marketplace_name)?;

    let plugin_dir = resolve_plugin_dir(&plugin_entry, &marketplace_path, &cache, marketplace_name)
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

/// Read the marketplace manifest and find the matching plugin entry.
fn find_plugin_entry(
    marketplace_path: &Path,
    plugin_name: &str,
    marketplace_name: &str,
) -> Result<PluginEntry> {
    let manifest_path = marketplace_path.join(kiro_market_core::MARKETPLACE_MANIFEST_PATH);
    let manifest_bytes = fs::read(&manifest_path).with_context(|| {
        format!(
            "failed to read marketplace manifest at {}",
            manifest_path.display()
        )
    })?;
    let manifest =
        Marketplace::from_json(&manifest_bytes).context("failed to parse marketplace manifest")?;

    manifest
        .plugins
        .into_iter()
        .find(|p| p.name == plugin_name)
        .with_context(|| {
            format!("plugin '{plugin_name}' not found in marketplace '{marketplace_name}'")
        })
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
    let Ok(skill_content) = fs::read_to_string(&skill_md_path) else {
        eprintln!(
            "  {} Failed to read {}",
            "✗".red().bold(),
            skill_md_path.display()
        );
        return SkillResult::Failed;
    };

    let Ok((frontmatter, body_offset)) = parse_frontmatter(&skill_content) else {
        eprintln!(
            "  {} Failed to parse SKILL.md in {}",
            "✗".red().bold(),
            skill_dir.display()
        );
        return SkillResult::Failed;
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
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("already installed") {
                println!(
                    "  {} Skill '{}' already installed (use --force to overwrite)",
                    "·".yellow().bold(),
                    frontmatter.name.bold()
                );
                SkillResult::Skipped
            } else {
                eprintln!(
                    "  {} Failed to install skill '{}': {e}",
                    "✗".red().bold(),
                    frontmatter.name
                );
                SkillResult::Failed
            }
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
            resolve_structured_source(structured, cache, marketplace_name, &entry.name)
        }
    }
}

/// Clone a structured source into the cache plugins directory and return the path.
fn resolve_structured_source(
    source: &StructuredSource,
    cache: &CacheDir,
    marketplace_name: &str,
    plugin_name: &str,
) -> Result<PathBuf> {
    cache
        .ensure_dirs()
        .context("failed to create cache directories")?;

    let dest = cache.plugin_path(marketplace_name, plugin_name);

    // If already cloned, reuse the existing directory.
    if dest.exists() {
        debug!(dest = %dest.display(), "plugin already cached, reusing");
        return match source {
            StructuredSource::GitSubdir { path, .. } => Ok(dest.join(path)),
            _ => Ok(dest),
        };
    }

    match source {
        StructuredSource::GitHub {
            repo, git_ref, sha, ..
        } => {
            let url = git::github_repo_to_url(repo);
            debug!(url = %url, dest = %dest.display(), "cloning GitHub plugin");
            print!("  Cloning {repo}...");
            let repo_handle = git::clone_repo(&url, &dest, git_ref.as_deref())
                .with_context(|| format!("failed to clone plugin from GitHub repo '{repo}'"))?;
            println!(" done");
            if let Some(expected) = sha {
                git::verify_sha(&repo_handle, expected)
                    .with_context(|| format!("SHA verification failed for '{repo}'"))?;
            }
            Ok(dest)
        }
        StructuredSource::GitUrl {
            url, git_ref, sha, ..
        } => {
            debug!(url = %url, dest = %dest.display(), "cloning git plugin");
            print!("  Cloning {url}...");
            let repo_handle = git::clone_repo(url, &dest, git_ref.as_deref())
                .with_context(|| format!("failed to clone plugin from '{url}'"))?;
            println!(" done");
            if let Some(expected) = sha {
                git::verify_sha(&repo_handle, expected)
                    .with_context(|| format!("SHA verification failed for '{url}'"))?;
            }
            Ok(dest)
        }
        StructuredSource::GitSubdir {
            url,
            path,
            git_ref,
            sha,
            ..
        } => {
            debug!(url = %url, path, dest = %dest.display(), "cloning git-subdir plugin");
            print!("  Cloning {url}...");
            let repo_handle = git::clone_repo(url, &dest, git_ref.as_deref())
                .with_context(|| format!("failed to clone plugin repo '{url}'"))?;
            println!(" done");
            if let Some(expected) = sha {
                git::verify_sha(&repo_handle, expected)
                    .with_context(|| format!("SHA verification failed for '{url}'"))?;
            }
            Ok(dest.join(path))
        }
    }
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
                debug!(
                    path = %manifest_path.display(),
                    error = %e,
                    "failed to parse plugin.json, using defaults"
                );
                None
            }
        },
        Err(e) => {
            debug!(
                path = %manifest_path.display(),
                error = %e,
                "plugin.json not found, using defaults"
            );
            None
        }
    }
}

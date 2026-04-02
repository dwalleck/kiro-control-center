//! `info` command: show detailed information about a plugin.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use colored::Colorize;
use kiro_market_core::cache::CacheDir;
use kiro_market_core::marketplace::{Marketplace, PluginEntry, PluginSource, StructuredSource};
use kiro_market_core::plugin::discover_skill_dirs;
use kiro_market_core::skill::parse_frontmatter;
use tracing::debug;

use crate::cli;

/// Path within a marketplace clone where the manifest lives.
const MARKETPLACE_MANIFEST: &str = ".claude-plugin/marketplace.json";

/// Default skill scan paths when a plugin has no explicit skill list.
const DEFAULT_SKILL_PATHS: &[&str] = &["./skills/"];

/// Run the info command.
///
/// Parses the plugin reference, finds the plugin in its marketplace, and
/// prints its details. For relative-path plugins, also lists discovered skills.
pub fn run(plugin_ref: &str) -> Result<()> {
    let (plugin_name, marketplace_name) = cli::parse_plugin_ref(plugin_ref).with_context(|| {
        format!("invalid plugin reference '{plugin_ref}': expected plugin@marketplace")
    })?;

    if plugin_name.is_empty() || marketplace_name.is_empty() {
        bail!(
            "invalid plugin reference '{plugin_ref}': both plugin and marketplace names are required"
        );
    }

    let cache = CacheDir::default_location();
    let marketplace_path = cache.marketplace_path(marketplace_name);
    if !marketplace_path.exists() {
        bail!(
            "marketplace '{}' not found. Run {} first.",
            marketplace_name,
            "kiro-market marketplace add".bold()
        );
    }

    let plugin_entry = find_plugin_entry(&marketplace_path, plugin_name, marketplace_name)?;

    print_plugin_details(&plugin_entry, marketplace_name);
    print_source_info(&plugin_entry.source);

    if let PluginSource::RelativePath(rel) = &plugin_entry.source {
        print_skills(&marketplace_path.join(rel));
    }

    Ok(())
}

/// Read the marketplace manifest and find the matching plugin entry.
fn find_plugin_entry(
    marketplace_path: &Path,
    plugin_name: &str,
    marketplace_name: &str,
) -> Result<PluginEntry> {
    let manifest_path = marketplace_path.join(MARKETPLACE_MANIFEST);
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

/// Print the basic plugin details header.
fn print_plugin_details(plugin_entry: &PluginEntry, marketplace_name: &str) {
    println!("{}", "Plugin Information".bold().underline());
    println!();
    println!("  {:<14} {}", "Name:".bold(), plugin_entry.name);
    println!("  {:<14} {}", "Marketplace:".bold(), marketplace_name);

    if let Some(desc) = &plugin_entry.description {
        println!("  {:<14} {}", "Description:".bold(), desc);
    }
}

/// Print the source information for a plugin.
fn print_source_info(source: &PluginSource) {
    match source {
        PluginSource::RelativePath(rel) => {
            println!("  {:<14} {} (local)", "Source:".bold(), rel);
        }
        PluginSource::Structured(structured) => {
            let source_desc = format_structured_source(structured);
            println!("  {:<14} {}", "Source:".bold(), source_desc);
        }
    }
}

/// Format a structured source into a human-readable string.
fn format_structured_source(source: &StructuredSource) -> String {
    match source {
        StructuredSource::GitHub { repo, git_ref, .. } => {
            let ref_info = git_ref
                .as_deref()
                .map_or(String::new(), |r| format!(" (ref: {r})"));
            format!("github:{repo}{ref_info}")
        }
        StructuredSource::GitUrl { url, git_ref, .. } => {
            let ref_info = git_ref
                .as_deref()
                .map_or(String::new(), |r| format!(" (ref: {r})"));
            format!("{url}{ref_info}")
        }
        StructuredSource::GitSubdir {
            url, path, git_ref, ..
        } => {
            let ref_info = git_ref
                .as_deref()
                .map_or(String::new(), |r| format!(" (ref: {r})"));
            format!("{url} [{path}]{ref_info}")
        }
    }
}

/// Discover and print skills for a relative-path plugin.
fn print_skills(plugin_dir: &Path) {
    if !plugin_dir.exists() {
        debug!(
            path = %plugin_dir.display(),
            "plugin directory does not exist, cannot list skills"
        );
        return;
    }

    let skill_paths = load_skill_paths(plugin_dir);
    let skill_path_refs: Vec<&str> = skill_paths.iter().map(String::as_str).collect();
    let skill_dirs = discover_skill_dirs(plugin_dir, &skill_path_refs);

    if skill_dirs.is_empty() {
        return;
    }

    println!();
    println!("  {}", "Skills:".bold());

    for skill_dir in &skill_dirs {
        let skill_md_path = skill_dir.join("SKILL.md");
        let Ok(content) = fs::read_to_string(&skill_md_path) else {
            continue;
        };

        let Ok((frontmatter, _)) = parse_frontmatter(&content) else {
            continue;
        };

        println!(
            "    {} - {}",
            frontmatter.name.green(),
            frontmatter.description
        );
    }
}

/// Load skill paths from a plugin's `plugin.json`, falling back to defaults.
fn load_skill_paths(plugin_dir: &Path) -> Vec<String> {
    let manifest_path = plugin_dir.join("plugin.json");
    match fs::read(&manifest_path) {
        Ok(bytes) => match kiro_market_core::plugin::PluginManifest::from_json(&bytes) {
            Ok(manifest) if !manifest.skills.is_empty() => manifest.skills,
            _ => DEFAULT_SKILL_PATHS.iter().map(|&s| s.to_owned()).collect(),
        },
        Err(_) => DEFAULT_SKILL_PATHS.iter().map(|&s| s.to_owned()).collect(),
    }
}

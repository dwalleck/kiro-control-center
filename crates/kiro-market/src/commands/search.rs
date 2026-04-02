//! `search` command: search plugins across registered marketplaces.

use std::fs;

use anyhow::{Context, Result};
use colored::Colorize;
use kiro_market_core::cache::CacheDir;
use kiro_market_core::marketplace::{Marketplace, PluginSource};
use kiro_market_core::plugin::discover_skill_dirs;
use kiro_market_core::skill::parse_frontmatter;
use tracing::debug;

/// Run the search command.
///
/// Iterates all known marketplaces, discovers skills from relative-path plugins,
/// and matches the query against skill names and descriptions (case-insensitive).
pub fn run(query: &str) -> Result<()> {
    let cache = CacheDir::default_location()
        .context("could not determine data directory; is $HOME set?")?;
    let entries = cache
        .load_known_marketplaces()
        .context("failed to load known marketplaces")?;

    if entries.is_empty() {
        println!(
            "No marketplaces registered. Use {} to add one.",
            "kiro-market marketplace add".bold()
        );
        return Ok(());
    }

    let query_lower = query.to_lowercase();
    let mut match_count = 0u32;

    for entry in &entries {
        let marketplace_path = cache.marketplace_path(&entry.name);
        let manifest_path = marketplace_path.join(kiro_market_core::MARKETPLACE_MANIFEST_PATH);

        let manifest_bytes = match fs::read(&manifest_path) {
            Ok(bytes) => bytes,
            Err(e) => {
                debug!(
                    marketplace = %entry.name,
                    error = %e,
                    "failed to read marketplace manifest, skipping"
                );
                continue;
            }
        };

        let manifest = match Marketplace::from_json(&manifest_bytes) {
            Ok(m) => m,
            Err(e) => {
                debug!(
                    marketplace = %entry.name,
                    error = %e,
                    "failed to parse marketplace manifest, skipping"
                );
                continue;
            }
        };

        for plugin in &manifest.plugins {
            // Only search relative-path plugins that are locally available.
            let PluginSource::RelativePath(rel) = &plugin.source else {
                debug!(
                    plugin = %plugin.name,
                    "skipping non-local plugin source for search"
                );
                continue;
            };

            let plugin_dir = marketplace_path.join(rel);
            if !plugin_dir.exists() {
                debug!(
                    path = %plugin_dir.display(),
                    "plugin directory does not exist, skipping"
                );
                continue;
            }

            // Load plugin.json to get skill paths, or use defaults.
            let skill_paths = load_skill_paths(&plugin_dir);
            let skill_path_refs: Vec<&str> = skill_paths.iter().map(String::as_str).collect();
            let skill_dirs = discover_skill_dirs(&plugin_dir, &skill_path_refs);

            for skill_dir in &skill_dirs {
                let skill_md_path = skill_dir.join("SKILL.md");
                let Ok(content) = fs::read_to_string(&skill_md_path) else {
                    continue;
                };

                let Ok((frontmatter, _)) = parse_frontmatter(&content) else {
                    continue;
                };

                let name_lower = frontmatter.name.to_lowercase();
                let desc_lower = frontmatter.description.to_lowercase();

                if name_lower.contains(&query_lower) || desc_lower.contains(&query_lower) {
                    if match_count == 0 {
                        println!("{}", "Search results:".bold());
                        println!();
                    }

                    println!(
                        "  {} ({}@{})",
                        frontmatter.name.green().bold(),
                        plugin.name,
                        entry.name
                    );
                    println!("    {}", frontmatter.description);
                    println!();

                    match_count += 1;
                }
            }
        }
    }

    if match_count == 0 {
        println!("No skills found matching '{query}'.");
    } else {
        println!(
            "Found {} skill{} matching '{}'.",
            match_count,
            if match_count == 1 { "" } else { "s" },
            query
        );
    }

    Ok(())
}

/// Load skill paths from a plugin's `plugin.json`, falling back to defaults.
fn load_skill_paths(plugin_dir: &std::path::Path) -> Vec<String> {
    let manifest_path = plugin_dir.join("plugin.json");
    match fs::read(&manifest_path) {
        Ok(bytes) => match kiro_market_core::plugin::PluginManifest::from_json(&bytes) {
            Ok(manifest) if !manifest.skills.is_empty() => manifest.skills,
            _ => kiro_market_core::DEFAULT_SKILL_PATHS
                .iter()
                .map(|&s| s.to_owned())
                .collect(),
        },
        Err(_) => kiro_market_core::DEFAULT_SKILL_PATHS
            .iter()
            .map(|&s| s.to_owned())
            .collect(),
    }
}

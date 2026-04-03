//! `search` command: search plugins across registered marketplaces.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use colored::Colorize;
use kiro_market_core::cache::CacheDir;
use kiro_market_core::marketplace::{Marketplace, PluginSource};
use kiro_market_core::plugin::discover_skill_dirs;
use kiro_market_core::skill::{SkillFrontmatter, parse_frontmatter};
use tracing::debug;

/// Run the search command.
///
/// Iterates all known marketplaces, discovers skills from relative-path plugins,
/// and matches the query against skill names and descriptions (case-insensitive).
pub fn run(query: Option<&str>) -> Result<()> {
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

    let query_lower = query.map(str::to_lowercase);
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
            match_count += search_plugin(
                &marketplace_path,
                plugin,
                &entry.name,
                query_lower.as_deref(),
                match_count,
            );
        }
    }

    if match_count == 0 {
        match query {
            Some(q) => println!("No skills found matching '{q}'."),
            None => println!("No skills found in any registered marketplace."),
        }
    } else {
        let label = match query {
            Some(q) => format!("matching '{q}'"),
            None => "available".to_owned(),
        };
        println!(
            "Found {} skill{} {label}.",
            match_count,
            if match_count == 1 { "" } else { "s" },
        );
    }

    Ok(())
}

/// Search a single plugin's skills and print matches. Returns the number of matches found.
/// When `query_lower` is `None`, all skills match.
fn search_plugin(
    marketplace_path: &Path,
    plugin: &kiro_market_core::marketplace::PluginEntry,
    marketplace_name: &str,
    query_lower: Option<&str>,
    prior_matches: u32,
) -> u32 {
    let PluginSource::RelativePath(rel) = &plugin.source else {
        debug!(plugin = %plugin.name, "skipping non-local plugin source for search");
        return 0;
    };

    let plugin_dir = marketplace_path.join(rel);
    if !plugin_dir.exists() {
        debug!(path = %plugin_dir.display(), "plugin directory does not exist, skipping");
        return 0;
    }

    let skill_paths = super::common::load_skill_paths(&plugin_dir);
    let skill_path_refs: Vec<&str> = skill_paths.iter().map(String::as_str).collect();
    let skill_dirs = discover_skill_dirs(&plugin_dir, &skill_path_refs);

    let mut matches = 0u32;

    for skill_dir in &skill_dirs {
        if let Some(fm) = read_skill_frontmatter(skill_dir) {
            let name_lower = fm.name.to_lowercase();
            let desc_lower = fm.description.to_lowercase();

            let matches_query = match query_lower {
                Some(q) => name_lower.contains(q) || desc_lower.contains(q),
                None => true,
            };
            if matches_query {
                if prior_matches + matches == 0 {
                    let header = if query_lower.is_some() {
                        "Search results:"
                    } else {
                        "Available skills:"
                    };
                    println!("{}", header.bold());
                    println!();
                }

                println!(
                    "  {} ({}@{})",
                    fm.name.green().bold(),
                    plugin.name,
                    marketplace_name
                );
                println!("    {}", fm.description);
                println!();

                matches += 1;
            }
        }
    }

    matches
}

/// Read and parse SKILL.md frontmatter, logging errors at debug level.
fn read_skill_frontmatter(skill_dir: &Path) -> Option<SkillFrontmatter> {
    let skill_md_path = skill_dir.join("SKILL.md");
    let content = match fs::read_to_string(&skill_md_path) {
        Ok(c) => c,
        Err(e) => {
            debug!(path = %skill_md_path.display(), error = %e, "failed to read SKILL.md, skipping");
            return None;
        }
    };

    match parse_frontmatter(&content) {
        Ok((fm, _)) => Some(fm),
        Err(e) => {
            debug!(path = %skill_md_path.display(), error = %e, "failed to parse SKILL.md frontmatter, skipping");
            None
        }
    }
}

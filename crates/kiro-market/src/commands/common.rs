//! Shared helpers used by multiple command modules.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use kiro_market_core::marketplace::{Marketplace, PluginEntry};
use kiro_market_core::plugin::PluginManifest;
use tracing::{debug, warn};

/// Read the marketplace manifest and find the matching plugin entry.
pub fn find_plugin_entry(
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

/// Load skill paths from a plugin's `plugin.json`, falling back to defaults.
///
/// Distinguishes missing files (expected) from read errors and parse errors
/// (warned about) so that malformed manifests don't silently use wrong defaults.
pub fn load_skill_paths(plugin_dir: &Path) -> Vec<String> {
    let manifest_path = plugin_dir.join("plugin.json");
    match fs::read(&manifest_path) {
        Ok(bytes) => match PluginManifest::from_json(&bytes) {
            Ok(manifest) if !manifest.skills.is_empty() => manifest.skills,
            Ok(_) => default_skill_paths(),
            Err(e) => {
                warn!(
                    path = %manifest_path.display(),
                    error = %e,
                    "plugin.json is malformed, falling back to defaults"
                );
                default_skill_paths()
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            debug!(
                path = %manifest_path.display(),
                "plugin.json not found, using defaults"
            );
            default_skill_paths()
        }
        Err(e) => {
            warn!(
                path = %manifest_path.display(),
                error = %e,
                "failed to read plugin.json, falling back to defaults"
            );
            default_skill_paths()
        }
    }
}

fn default_skill_paths() -> Vec<String> {
    kiro_market_core::DEFAULT_SKILL_PATHS
        .iter()
        .map(|&s| s.to_owned())
        .collect()
}

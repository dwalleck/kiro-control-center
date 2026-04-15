//! Shared helpers used by multiple command modules.

use std::fs;
use std::path::Path;

use anyhow::Result;
use kiro_market_core::marketplace::{Marketplace, PluginEntry};
use kiro_market_core::plugin::PluginManifest;
use tracing::{debug, warn};

/// Read the marketplace manifest and find the matching plugin entry.
///
/// Falls back to a depth-limited scan for `plugin.json` files when
/// `marketplace.json` is absent, unreadable, malformed, or does not list
/// the requested plugin.
pub fn find_plugin_entry(
    marketplace_path: &Path,
    plugin_name: &str,
    marketplace_name: &str,
) -> Result<PluginEntry> {
    let manifest_path = marketplace_path.join(kiro_market_core::MARKETPLACE_MANIFEST_PATH);

    match fs::read(&manifest_path) {
        Ok(bytes) => match Marketplace::from_json(&bytes) {
            Ok(manifest) => {
                if let Some(entry) = manifest.plugins.into_iter().find(|p| p.name == plugin_name) {
                    return Ok(entry);
                }
                // Plugin not in manifest — fall through to scan.
            }
            Err(e) => {
                warn!(
                    path = %manifest_path.display(),
                    error = %e,
                    "marketplace.json is malformed, falling back to plugin scan"
                );
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            debug!(
                path = %manifest_path.display(),
                "no marketplace.json found, falling back to plugin scan"
            );
        }
        Err(e) => {
            warn!(
                path = %manifest_path.display(),
                error = %e,
                "failed to read marketplace.json, falling back to plugin scan"
            );
            // Still fall through to scan -- the manifest may be optional.
            // But propagate permission errors instead of silently scanning.
            if e.kind() == std::io::ErrorKind::PermissionDenied {
                anyhow::bail!("permission denied reading {}: {e}", manifest_path.display());
            }
        }
    }

    // Fall back to scanning for plugin.json.
    let discovered = kiro_market_core::plugin::discover_plugins(marketplace_path, 3);
    if let Some(dp) = discovered.into_iter().find(|dp| dp.name() == plugin_name) {
        return Ok(PluginEntry {
            name: dp.name().to_owned(),
            description: dp.description().map(String::from),
            source: kiro_market_core::marketplace::PluginSource::RelativePath(
                dp.as_relative_path_string(),
            ),
        });
    }

    anyhow::bail!("plugin '{plugin_name}' not found in marketplace '{marketplace_name}'")
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

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    /// Helper: create a marketplace repo layout with a manifest listing one plugin.
    fn create_marketplace_with_manifest(root: &std::path::Path, plugin_name: &str) {
        let mp_dir = root.join(".claude-plugin");
        fs::create_dir_all(&mp_dir).expect("create .claude-plugin");
        fs::write(
            mp_dir.join("marketplace.json"),
            format!(
                r#"{{"name":"test-market","owner":{{"name":"Test"}},"plugins":[{{"name":"{plugin_name}","description":"Listed plugin","source":"./plugins/{plugin_name}"}}]}}"#
            ),
        )
        .expect("write marketplace.json");

        let plugin_dir = root.join(format!("plugins/{plugin_name}"));
        fs::create_dir_all(&plugin_dir).expect("create plugin dir");
        fs::write(
            plugin_dir.join("plugin.json"),
            format!(
                r#"{{"name":"{plugin_name}","description":"Listed plugin","skills":["./skills/"]}}"#
            ),
        )
        .expect("write plugin.json");
    }

    /// Helper: create a plugin directory with plugin.json but no marketplace.json.
    fn create_plugin_without_manifest(root: &std::path::Path, plugin_name: &str) {
        let plugin_dir = root.join(format!("plugins/{plugin_name}"));
        fs::create_dir_all(&plugin_dir).expect("create plugin dir");
        fs::write(
            plugin_dir.join("plugin.json"),
            format!(
                r#"{{"name":"{plugin_name}","description":"Discovered plugin","skills":["./skills/"]}}"#
            ),
        )
        .expect("write plugin.json");
    }

    #[test]
    fn find_plugin_entry_from_manifest() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        create_marketplace_with_manifest(root, "listed");

        let entry =
            find_plugin_entry(root, "listed", "test-market").expect("should find listed plugin");

        assert_eq!(entry.name, "listed");
        assert_eq!(entry.description.as_deref(), Some("Listed plugin"));
    }

    #[test]
    fn find_plugin_entry_falls_back_to_scan_when_no_manifest() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        create_plugin_without_manifest(root, "discovered");

        let entry = find_plugin_entry(root, "discovered", "test-market")
            .expect("should find via scan fallback");

        assert_eq!(entry.name, "discovered");
        assert_eq!(entry.description.as_deref(), Some("Discovered plugin"));
        assert!(
            matches!(
                &entry.source,
                kiro_market_core::marketplace::PluginSource::RelativePath(p) if p.contains("discovered")
            ),
            "source should be a RelativePath: {:?}",
            entry.source
        );
    }

    #[test]
    fn find_plugin_entry_falls_back_to_scan_when_plugin_unlisted() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        // Create a manifest that lists "listed" but NOT "unlisted".
        create_marketplace_with_manifest(root, "listed");
        create_plugin_without_manifest(root, "unlisted");

        let entry = find_plugin_entry(root, "unlisted", "test-market")
            .expect("should find unlisted via scan");

        assert_eq!(entry.name, "unlisted");
    }

    #[test]
    fn find_plugin_entry_errors_when_plugin_not_found_anywhere() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        // Empty directory — no manifest, no plugins.
        fs::create_dir_all(root).expect("create root");

        let err = find_plugin_entry(root, "nonexistent", "test-market").expect_err("should fail");

        assert!(
            err.to_string().contains("not found"),
            "expected 'not found' in error: {err}"
        );
    }
}

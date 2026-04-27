//! Types representing a plugin manifest (`plugin.json`).
//!
//! Each plugin directory in a marketplace contains a `plugin.json` that
//! declares the plugin name, version, description, and the list of skill
//! subdirectories it ships.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use tracing::{debug, warn};

/// A plugin manifest as found in `plugin.json`.
#[derive(Debug, Clone, Deserialize)]
pub struct PluginManifest {
    pub name: String,
    pub version: Option<String>,
    pub description: Option<String>,
    #[serde(default)]
    pub skills: Vec<String>,
    /// Optional list of directories (relative to the plugin root) to scan
    /// for agent markdown files. Empty means "use the default scan paths"
    /// ([`crate::DEFAULT_AGENT_PATHS`]).
    #[serde(default)]
    pub agents: Vec<String>,
    /// Authoring format for this plugin. See [`PluginFormat`].
    #[serde(default)]
    pub format: Option<PluginFormat>,

    /// Optional list of directories (relative to the plugin root) to scan
    /// for steering markdown files. Empty means "use the default scan
    /// paths" ([`crate::DEFAULT_STEERING_PATHS`]).
    #[serde(default)]
    pub steering: Vec<String>,
}

/// The plugin's native authoring format. Drives dispatch in
/// `MarketplaceService::install_plugin_agents`: `KiroCli` skips
/// parse-and-translate and validates-and-copies native JSON agents.
/// Absent means the plugin uses Claude / Copilot markdown agents that
/// require translation (the existing default flow).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum PluginFormat {
    KiroCli,
}

impl PluginManifest {
    /// Deserialise a `PluginManifest` from a JSON byte slice.
    ///
    /// # Errors
    ///
    /// Returns a [`serde_json::Error`] if the input is not valid JSON or does
    /// not match the expected schema.
    pub fn from_json(json: &[u8]) -> Result<Self, serde_json::Error> {
        serde_json::from_slice(json)
    }
}

/// Name of the plugin manifest file.
const PLUGIN_JSON: &str = "plugin.json";

/// Directories to skip during recursive plugin scanning.
const SKIP_DIRS: &[&str] = &["node_modules", "target", "__pycache__", ".venv", "vendor"];

/// A plugin discovered by scanning a repository for `plugin.json` files.
#[derive(Debug, Clone)]
pub struct DiscoveredPlugin {
    name: String,
    description: Option<String>,
    relative_path: PathBuf,
}

impl DiscoveredPlugin {
    /// The plugin name from its `plugin.json`.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The plugin description from its `plugin.json`.
    #[must_use]
    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    /// Path to the plugin directory, relative to the repo root.
    #[must_use]
    pub fn relative_path(&self) -> &Path {
        &self.relative_path
    }

    /// The relative path as a `./`-prefixed string with forward slashes,
    /// matching the `PluginSource::RelativePath` convention used in
    /// `marketplace.json`. Uses forward slashes on all platforms.
    #[must_use]
    pub fn as_relative_path_string(&self) -> String {
        let unix_path = self.relative_path_unix();
        format!("./{unix_path}")
    }

    /// The relative path with forward slashes, suitable for cross-platform
    /// comparison against manifest paths.
    #[must_use]
    pub fn relative_path_unix(&self) -> String {
        self.relative_path
            .components()
            .map(|c| c.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/")
    }
}

/// Default maximum directory depth for [`discover_plugins`] scans.
///
/// Production callers use this value; tests may pass explicit depths to pin
/// behaviour. A depth of 3 accommodates typical catalog layouts like
/// `root/plugins/<name>/plugin.json` with one level of nesting to spare.
pub const DEFAULT_DISCOVERY_MAX_DEPTH: usize = 3;

/// Scan a repository for `plugin.json` files up to `max_depth` levels deep.
///
/// Skips hidden directories (starting with `.`) and common noise directories
/// (`node_modules`, `target`, etc.). Returns a list of discovered plugins with
/// their names, descriptions, and relative paths.
///
/// Per-file failures (malformed JSON, invalid names, unreadable subdirs deep in
/// the tree) are logged at `warn`/`debug` and skipped. An I/O error on the
/// **repo root itself** is propagated as `Err` so callers can distinguish
/// "no plugins exist" from "couldn't read the repo" — masking these as the
/// same condition leads to misleading "no plugins found" errors when the
/// real cause is a permission denial.
///
/// # Errors
///
/// Returns the underlying `io::Error` if `repo_root` cannot be read.
pub fn discover_plugins(
    repo_root: &Path,
    max_depth: usize,
) -> std::io::Result<Vec<DiscoveredPlugin>> {
    let mut results = Vec::new();
    scan_root(repo_root, max_depth, &mut results)?;
    Ok(results)
}

/// Read the repo root and dispatch into recursive scanning. The root read is
/// the only filesystem access whose failure is propagated as `Err`.
fn scan_root(
    repo_root: &Path,
    max_depth: usize,
    results: &mut Vec<DiscoveredPlugin>,
) -> std::io::Result<()> {
    // Surface the read attempt so a permission denial on the repo root is
    // not silently misreported downstream as "no plugins found".
    let entries = fs::read_dir(repo_root).map_err(|e| {
        std::io::Error::new(
            e.kind(),
            format!("failed to read repo root {}: {e}", repo_root.display()),
        )
    })?;

    if max_depth == 0 {
        // Caller asked for depth 0; only the root is in scope, and the root
        // itself is not treated as a plugin (matches existing semantics).
        let _ = entries;
        return Ok(());
    }

    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                warn!(error = %e, "failed to read directory entry at repo root, skipping");
                continue;
            }
        };
        let entry_path = entry.path();
        if !entry_path.is_dir() {
            continue;
        }
        let Some(dir_name) = entry.file_name().to_str().map(str::to_owned) else {
            debug!(path = %entry_path.display(), "skipping directory with non-UTF-8 name");
            continue;
        };
        if dir_name.starts_with('.') || SKIP_DIRS.contains(&dir_name.as_str()) {
            continue;
        }
        scan_for_plugins(repo_root, &entry_path, 1, max_depth, results);
    }
    Ok(())
}

/// Try to read and validate a `plugin.json` at the given directory.
/// Returns `Some(DiscoveredPlugin)` if successful, `None` if the file
/// doesn't exist, is malformed, or has an invalid name.
fn try_read_plugin(dir: &Path, repo_root: &Path) -> Option<DiscoveredPlugin> {
    let candidate = dir.join(PLUGIN_JSON);
    if !candidate.is_file() {
        return None;
    }

    let bytes = match fs::read(&candidate) {
        Ok(b) => b,
        Err(e) => {
            warn!(
                path = %candidate.display(),
                error = %e,
                "failed to read plugin.json, skipping"
            );
            return None;
        }
    };

    let manifest = match PluginManifest::from_json(&bytes) {
        Ok(m) => m,
        Err(e) => {
            warn!(
                path = %candidate.display(),
                error = %e,
                "skipping malformed plugin.json"
            );
            return None;
        }
    };

    if let Err(e) = crate::validation::validate_name(&manifest.name) {
        warn!(
            path = %candidate.display(),
            name = %manifest.name,
            error = %e,
            "skipping plugin with invalid name"
        );
        return None;
    }

    let relative_path = dir.strip_prefix(repo_root).unwrap_or(dir).to_path_buf();
    debug!(
        name = %manifest.name,
        path = %relative_path.display(),
        "discovered plugin"
    );
    Some(DiscoveredPlugin {
        name: manifest.name,
        description: manifest.description,
        relative_path,
    })
}

fn scan_for_plugins(
    repo_root: &Path,
    dir: &Path,
    current_depth: usize,
    max_depth: usize,
    results: &mut Vec<DiscoveredPlugin>,
) {
    if current_depth > max_depth {
        return;
    }

    // Check for plugin.json in this directory (skip the root itself).
    if current_depth > 0 && dir.join(PLUGIN_JSON).is_file() {
        if let Some(plugin) = try_read_plugin(dir, repo_root) {
            results.push(plugin);
        }
        // Don't recurse into a plugin directory — it won't contain nested plugins.
        return;
    }

    // Recurse into subdirectories.
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) => {
            if current_depth == 0 {
                warn!(
                    path = %dir.display(),
                    error = %e,
                    "failed to read repo root during plugin scan"
                );
            } else {
                debug!(
                    path = %dir.display(),
                    error = %e,
                    "failed to read directory during plugin scan, skipping"
                );
            }
            return;
        }
    };

    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                warn!(error = %e, "failed to read directory entry, skipping");
                continue;
            }
        };

        let entry_path = entry.path();
        if !entry_path.is_dir() {
            continue;
        }

        // Skip hidden and noise directories.
        let Some(dir_name) = entry.file_name().to_str().map(str::to_owned) else {
            debug!(
                path = %entry_path.display(),
                "skipping directory with non-UTF-8 name"
            );
            continue;
        };

        if dir_name.starts_with('.') || SKIP_DIRS.contains(&dir_name.as_str()) {
            continue;
        }

        scan_for_plugins(
            repo_root,
            &entry_path,
            current_depth + 1,
            max_depth,
            results,
        );
    }
}

/// Name of the skill definition file.
const SKILL_MD: &str = "SKILL.md";

/// Discover skill directories within a plugin root given a list of paths.
///
/// Each entry in `skill_paths` is interpreted relative to `plugin_root`:
///
/// - If it ends with `/`, it is treated as a directory to scan: every
///   immediate subdirectory that contains a `SKILL.md` is included.
/// - Otherwise it is treated as a specific directory; it is included only
///   if it contains a `SKILL.md`.
///
/// The returned paths are sorted for deterministic ordering.
#[must_use]
pub fn discover_skill_dirs(plugin_root: &Path, skill_paths: &[&str]) -> Vec<PathBuf> {
    let mut dirs = Vec::new();

    for &path_str in skill_paths {
        if let Err(e) = crate::validation::validate_relative_path(path_str) {
            warn!(
                path = path_str,
                error = %e,
                "skipping skill path that fails validation"
            );
            continue;
        }

        let candidate = plugin_root.join(path_str);

        if path_str.ends_with('/') || path_str.ends_with('\\') {
            // Scan subdirectories for those containing SKILL.md.
            match fs::read_dir(&candidate) {
                Ok(entries) => {
                    for entry in entries {
                        let entry = match entry {
                            Ok(e) => e,
                            Err(e) => {
                                warn!(
                                    path = %candidate.display(),
                                    error = %e,
                                    "failed to read directory entry, skipping"
                                );
                                continue;
                            }
                        };
                        let entry_path = entry.path();
                        if entry_path.is_dir() && entry_path.join(SKILL_MD).exists() {
                            dirs.push(entry_path);
                        }
                    }
                }
                Err(e) => {
                    debug!(
                        path = %candidate.display(),
                        error = %e,
                        "failed to read skill scan directory"
                    );
                }
            }
        } else if candidate.is_dir() && candidate.join(SKILL_MD).exists() {
            dirs.push(candidate);
        } else {
            debug!(
                path = %candidate.display(),
                "skill path does not contain SKILL.md, skipping"
            );
        }
    }

    dirs.sort();
    dirs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_with_skill_paths() {
        let json = br#"{
            "name": "dotnet",
            "version": "1.0.0",
            "description": ".NET development skills",
            "skills": ["skills/csharp", "skills/fsharp"]
        }"#;

        let m = PluginManifest::from_json(json).expect("should parse");
        assert_eq!(m.name, "dotnet");
        assert_eq!(m.version.as_deref(), Some("1.0.0"));
        assert_eq!(m.description.as_deref(), Some(".NET development skills"));
        assert_eq!(m.skills, vec!["skills/csharp", "skills/fsharp"]);
    }

    #[test]
    fn parse_without_skills_defaults_to_empty() {
        let json = br#"{
            "name": "minimal-plugin"
        }"#;

        let m = PluginManifest::from_json(json).expect("should parse");
        assert_eq!(m.name, "minimal-plugin");
        assert!(m.version.is_none());
        assert!(m.description.is_none());
        assert!(m.skills.is_empty());
        assert!(m.agents.is_empty());
    }

    #[test]
    fn parse_with_explicit_agents_list() {
        let json = br#"{
            "name": "agent-plugin",
            "skills": ["./skills/"],
            "agents": ["./agents/"]
        }"#;
        let m = PluginManifest::from_json(json).expect("should parse");
        assert_eq!(m.agents, vec!["./agents/"]);
    }

    #[test]
    fn parse_without_agents_defaults_to_empty() {
        let json = br#"{ "name": "p" }"#;
        let m = PluginManifest::from_json(json).expect("should parse");
        assert!(m.agents.is_empty());
    }

    #[test]
    fn manifest_parses_steering_paths() {
        let json = br#"{"name": "p", "steering": ["./guidance/", "./extras/"]}"#;
        let manifest = PluginManifest::from_json(json).expect("should parse");
        assert_eq!(manifest.steering, vec!["./guidance/", "./extras/"]);
    }

    #[test]
    fn manifest_steering_absent_is_empty_vec() {
        let json = br#"{"name": "p"}"#;
        let manifest = PluginManifest::from_json(json).expect("should parse");
        assert!(manifest.steering.is_empty());
    }

    #[test]
    fn parse_with_explicit_skill_list() {
        let json = br#"{
            "name": "multi-skill",
            "version": "2.1.0",
            "skills": ["skills/alpha", "skills/beta", "skills/gamma"]
        }"#;

        let m = PluginManifest::from_json(json).expect("should parse");
        assert_eq!(m.skills.len(), 3);
        assert_eq!(m.skills[0], "skills/alpha");
        assert_eq!(m.skills[1], "skills/beta");
        assert_eq!(m.skills[2], "skills/gamma");
    }

    // -----------------------------------------------------------------------
    // discover_skill_dirs
    // -----------------------------------------------------------------------

    /// Create a minimal SKILL.md in the given directory.
    fn create_skill_md(dir: &Path) {
        fs::create_dir_all(dir).expect("create_dir_all");
        fs::write(
            dir.join("SKILL.md"),
            "---\nname: test\ndescription: test\n---\n",
        )
        .expect("write SKILL.md");
    }

    #[test]
    fn discover_skills_from_directory_path() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();

        create_skill_md(&root.join("skills/tunit"));
        create_skill_md(&root.join("skills/efcore"));

        // A directory without SKILL.md should be ignored.
        fs::create_dir_all(root.join("skills/empty")).expect("mkdir");

        let dirs = discover_skill_dirs(root, &["./skills/"]);

        assert_eq!(dirs.len(), 2);
        // Results should be sorted, so efcore comes before tunit.
        assert!(
            dirs[0].ends_with("efcore"),
            "first should be efcore, got {:?}",
            dirs[0]
        );
        assert!(
            dirs[1].ends_with("tunit"),
            "second should be tunit, got {:?}",
            dirs[1]
        );
    }

    #[test]
    fn discover_skills_from_explicit_paths() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();

        create_skill_md(&root.join("skills/tunit"));
        create_skill_md(&root.join("skills/efcore"));

        // Only discover one specific skill.
        let dirs = discover_skill_dirs(root, &["./skills/tunit"]);

        assert_eq!(dirs.len(), 1);
        assert!(
            dirs[0].ends_with("tunit"),
            "should find tunit, got {:?}",
            dirs[0]
        );
    }

    #[test]
    fn discover_skills_skips_missing_skill_md() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();

        // Directory exists but has no SKILL.md.
        fs::create_dir_all(root.join("skills/no-skill")).expect("mkdir");

        let dirs = discover_skill_dirs(root, &["./skills/no-skill"]);
        assert!(dirs.is_empty(), "should skip directories without SKILL.md");
    }

    #[test]
    fn discover_skills_from_nonexistent_directory() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();

        let dirs = discover_skill_dirs(root, &["./nonexistent/"]);
        assert!(dirs.is_empty(), "should return empty for missing directory");
    }

    #[test]
    fn discover_skills_rejects_path_traversal() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();

        // Create a valid skill alongside the traversal attempt.
        create_skill_md(&root.join("skills/legit"));

        // The traversal path should be skipped; the valid one should still be found.
        let dirs = discover_skill_dirs(root, &["../../etc/passwd", "./skills/legit"]);

        assert_eq!(dirs.len(), 1, "traversal path should be skipped");
        assert!(
            dirs[0].ends_with("legit"),
            "only the valid skill should be returned, got {:?}",
            dirs[0]
        );
    }

    #[test]
    fn discover_skills_rejects_absolute_path() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();

        create_skill_md(&root.join("skills/safe"));

        let dirs = discover_skill_dirs(root, &["/etc/passwd", "./skills/safe"]);

        assert_eq!(dirs.len(), 1, "absolute path should be skipped");
        assert!(
            dirs[0].ends_with("safe"),
            "only the valid skill should be returned, got {:?}",
            dirs[0]
        );
    }

    // -----------------------------------------------------------------------
    // discover_plugins
    // -----------------------------------------------------------------------

    /// Helper: create a minimal plugin.json in the given directory.
    fn create_plugin_json(dir: &Path, name: &str, description: Option<&str>) {
        fs::create_dir_all(dir).expect("create_dir_all");
        let desc = description
            .map(|d| format!(r#","description":"{d}""#))
            .unwrap_or_default();
        fs::write(
            dir.join("plugin.json"),
            format!(r#"{{"name":"{name}"{desc},"skills":["./skills/"]}}"#),
        )
        .expect("write plugin.json");
    }

    #[test]
    fn discover_plugins_finds_plugin_json_at_depth_1() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();

        create_plugin_json(&root.join("my-plugin"), "my-plugin", Some("A plugin"));

        let discovered = discover_plugins(root, 3).expect("discover should succeed");
        assert_eq!(discovered.len(), 1);
        assert_eq!(discovered[0].name(), "my-plugin");
        assert_eq!(discovered[0].description(), Some("A plugin"));
    }

    #[test]
    fn discover_plugins_finds_plugin_json_at_depth_2() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();

        create_plugin_json(
            &root.join("plugins/dotnet-experimental"),
            "dotnet-experimental",
            Some("Experimental"),
        );

        let discovered = discover_plugins(root, 3).expect("discover should succeed");
        assert_eq!(discovered.len(), 1);
        assert_eq!(discovered[0].name(), "dotnet-experimental");
    }

    #[test]
    fn discover_plugins_finds_multiple_plugins() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();

        create_plugin_json(&root.join("plugins/alpha"), "alpha", None);
        create_plugin_json(&root.join("plugins/beta"), "beta", None);

        let mut discovered = discover_plugins(root, 3).expect("discover should succeed");
        discovered.sort_by(|a, b| a.name().cmp(b.name()));
        assert_eq!(discovered.len(), 2);
        assert_eq!(discovered[0].name(), "alpha");
        assert_eq!(discovered[1].name(), "beta");
    }

    #[test]
    fn discover_plugins_respects_depth_limit() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();

        create_plugin_json(&root.join("a/b/c/deep-plugin"), "deep-plugin", None);

        let discovered = discover_plugins(root, 3).expect("discover should succeed");
        assert!(discovered.is_empty(), "should not find plugin at depth 4");
    }

    #[test]
    fn discover_plugins_skips_hidden_directories() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();

        create_plugin_json(&root.join(".git/hooks"), "git-hooks", None);
        create_plugin_json(&root.join(".claude-plugin"), "claude-internal", None);
        create_plugin_json(&root.join("plugins/visible"), "visible", None);

        let discovered = discover_plugins(root, 3).expect("discover should succeed");
        assert_eq!(discovered.len(), 1);
        assert_eq!(discovered[0].name(), "visible");
    }

    #[test]
    fn discover_plugins_skips_noise_directories() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();

        create_plugin_json(&root.join("node_modules/some-pkg"), "npm-thing", None);
        create_plugin_json(&root.join("target/debug"), "build-artifact", None);
        create_plugin_json(&root.join("plugins/real"), "real", None);

        let discovered = discover_plugins(root, 3).expect("discover should succeed");
        assert_eq!(discovered.len(), 1);
        assert_eq!(discovered[0].name(), "real");
    }

    #[test]
    fn discover_plugins_skips_malformed_plugin_json() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();

        let bad_dir = root.join("plugins/bad");
        fs::create_dir_all(&bad_dir).expect("mkdir");
        fs::write(bad_dir.join("plugin.json"), "not json").expect("write");

        create_plugin_json(&root.join("plugins/good"), "good", None);

        let discovered = discover_plugins(root, 3).expect("discover should succeed");
        assert_eq!(discovered.len(), 1);
        assert_eq!(discovered[0].name(), "good");
    }

    #[test]
    fn discover_plugins_returns_empty_for_no_plugins() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let discovered = discover_plugins(tmp.path(), 3).expect("discover should succeed");
        assert!(discovered.is_empty());
    }

    #[test]
    fn discover_plugins_includes_relative_path() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();

        create_plugin_json(&root.join("plugins/my-plugin"), "my-plugin", None);

        let discovered = discover_plugins(root, 3).expect("discover should succeed");
        assert_eq!(discovered.len(), 1);
        assert_eq!(
            discovered[0].relative_path(),
            Path::new("plugins/my-plugin")
        );
    }

    #[test]
    fn discover_plugins_finds_plugin_at_exact_max_depth() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();

        // Depth 3 exactly — should be found with max_depth 3.
        create_plugin_json(&root.join("a/b/at-limit"), "at-limit", None);

        let discovered = discover_plugins(root, 3).expect("discover should succeed");
        assert_eq!(
            discovered.len(),
            1,
            "should find plugin at exactly max_depth"
        );
        assert_eq!(discovered[0].name(), "at-limit");
    }

    #[test]
    fn discover_plugins_relative_path_string_has_dot_slash_prefix() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();

        create_plugin_json(&root.join("plugins/my-plugin"), "my-plugin", None);

        let discovered = discover_plugins(root, 3).expect("discover should succeed");
        assert_eq!(discovered.len(), 1);
        assert_eq!(
            discovered[0].as_relative_path_string(),
            "./plugins/my-plugin"
        );
    }

    #[test]
    fn discover_plugins_max_depth_zero_returns_empty() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();

        create_plugin_json(&root.join("my-plugin"), "my-plugin", None);

        let discovered = discover_plugins(root, 0).expect("discover should succeed");
        assert!(
            discovered.is_empty(),
            "max_depth 0 should not find plugins at depth 1"
        );
    }

    #[test]
    fn parse_missing_name_returns_error() {
        let json = br#"{
            "version": "1.0.0",
            "description": "no name field"
        }"#;

        assert!(
            PluginManifest::from_json(json).is_err(),
            "missing `name` field should produce an error"
        );
    }

    #[test]
    fn manifest_parses_format_kiro_cli() {
        let json = br#"{"name": "p", "format": "kiro-cli"}"#;
        let manifest = PluginManifest::from_json(json).expect("should parse");
        assert_eq!(manifest.format, Some(PluginFormat::KiroCli));
    }

    #[test]
    fn manifest_format_absent_is_none() {
        let json = br#"{"name": "p"}"#;
        let manifest = PluginManifest::from_json(json).expect("should parse");
        assert!(manifest.format.is_none());
    }

    #[test]
    fn manifest_unknown_format_value_fails_loudly() {
        let json = br#"{"name": "p", "format": "kiro-ide"}"#;
        let err = PluginManifest::from_json(json).expect_err("unknown variant should fail");
        let msg = err.to_string();
        assert!(
            msg.contains("kiro-ide") || msg.contains("unknown variant"),
            "error must mention the unknown variant; got: {msg}"
        );
    }

    #[test]
    fn discover_plugins_skips_plugin_with_invalid_name() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();

        // Plugin with path traversal in name — should be skipped.
        let bad_dir = root.join("plugins/bad");
        fs::create_dir_all(&bad_dir).expect("mkdir");
        fs::write(
            bad_dir.join("plugin.json"),
            r#"{"name":"../escape","skills":["./skills/"]}"#,
        )
        .expect("write");

        create_plugin_json(&root.join("plugins/good"), "good", None);

        let discovered = discover_plugins(root, 3).expect("discover should succeed");
        assert_eq!(discovered.len(), 1);
        assert_eq!(discovered[0].name(), "good");
    }
}

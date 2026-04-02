//! Types representing a plugin manifest (`plugin.json`).
//!
//! Each plugin directory in a marketplace contains a `plugin.json` that
//! declares the plugin name, version, description, and the list of skill
//! subdirectories it ships.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use tracing::debug;

/// A plugin manifest as found in `plugin.json`.
#[derive(Debug, Clone, Deserialize)]
pub struct PluginManifest {
    pub name: String,
    pub version: Option<String>,
    pub description: Option<String>,
    #[serde(default)]
    pub skills: Vec<String>,
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
        let candidate = plugin_root.join(path_str);

        if path_str.ends_with('/') {
            // Scan subdirectories for those containing SKILL.md.
            match fs::read_dir(&candidate) {
                Ok(entries) => {
                    for entry in entries.filter_map(Result::ok) {
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
}

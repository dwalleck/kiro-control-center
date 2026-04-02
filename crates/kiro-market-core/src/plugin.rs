//! Types representing a plugin manifest (`plugin.json`).
//!
//! Each plugin directory in a marketplace contains a `plugin.json` that
//! declares the plugin name, version, description, and the list of skill
//! subdirectories it ships.

use serde::Deserialize;

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
}

//! Types representing the Claude Code `marketplace.json` format.
//!
//! A marketplace file describes a collection of plugins published by an owner.
//! Each plugin entry may specify its source as either a bare relative path string
//! or a structured object with provider-specific fields.

use std::fmt;

use serde::de::value::MapAccessDeserializer;
use serde::de::{self, MapAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};

use crate::validation::RelativePath;

/// Top-level marketplace manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Marketplace {
    pub name: String,
    pub owner: Owner,
    pub plugins: Vec<PluginEntry>,
}

/// The owner / publisher of a marketplace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Owner {
    pub name: String,
    pub url: Option<String>,
}

/// A single plugin listed in the marketplace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginEntry {
    pub name: String,
    pub description: Option<String>,
    pub source: PluginSource,
}

/// How the plugin source is specified in JSON.
///
/// The field is *either* a bare string (interpreted as a relative path to a local
/// directory) or a tagged object describing a remote source.
///
/// Because the JSON representation has no surrounding tag, we use
/// `#[serde(untagged)]` and rely on variant ordering — `Structured` is tried
/// first (it expects an object), and `RelativePath` (a plain string) acts as the
/// fallback.
#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum PluginSource {
    /// A structured source descriptor (GitHub, URL, git-subdir).
    Structured(StructuredSource),
    /// A bare relative path like `"./plugins/dotnet"`. Holding a
    /// [`RelativePath`] is a static guarantee the string passed validation.
    RelativePath(RelativePath),
}

impl<'de> Deserialize<'de> for PluginSource {
    // Hand-written to preserve specific validation errors. `#[serde(untagged)]`
    // swallows inner failures and emits a generic "did not match any variant"
    // message; our Visitor reports the exact reason a path was rejected.
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct PluginSourceVisitor;

        impl<'de> Visitor<'de> for PluginSourceVisitor {
            type Value = PluginSource;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a relative-path string or a structured source object")
            }

            fn visit_str<E>(self, s: &str) -> Result<PluginSource, E>
            where
                E: de::Error,
            {
                RelativePath::new(s)
                    .map(PluginSource::RelativePath)
                    .map_err(de::Error::custom)
            }

            fn visit_string<E>(self, s: String) -> Result<PluginSource, E>
            where
                E: de::Error,
            {
                RelativePath::new(s)
                    .map(PluginSource::RelativePath)
                    .map_err(de::Error::custom)
            }

            fn visit_map<A>(self, map: A) -> Result<PluginSource, A::Error>
            where
                A: MapAccess<'de>,
            {
                StructuredSource::deserialize(MapAccessDeserializer::new(map))
                    .map(PluginSource::Structured)
            }
        }

        deserializer.deserialize_any(PluginSourceVisitor)
    }
}

/// Provider-specific structured source descriptor, internally tagged on `"source"`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "source")]
pub enum StructuredSource {
    /// A GitHub repository.
    #[serde(rename = "github")]
    GitHub {
        repo: String,
        #[serde(rename = "ref")]
        git_ref: Option<String>,
        sha: Option<String>,
    },
    /// A plain Git URL.
    #[serde(rename = "url")]
    GitUrl {
        url: String,
        #[serde(rename = "ref")]
        git_ref: Option<String>,
        sha: Option<String>,
    },
    /// A subdirectory within a Git repository.
    #[serde(rename = "git-subdir")]
    GitSubdir {
        url: String,
        /// The subdir is typed as [`RelativePath`] so traversal cannot even
        /// exist in-memory — `RelativePath::new` is the only way in.
        path: RelativePath,
        #[serde(rename = "ref")]
        git_ref: Option<String>,
        sha: Option<String>,
    },
}

impl Marketplace {
    /// Deserialise a `Marketplace` from a JSON byte slice.
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
    use rstest::rstest;

    use super::*;

    #[test]
    fn parse_marketplace_with_relative_sources() {
        let json = br#"{
            "name": "dotnet-agent-skills",
            "owner": { "name": "Microsoft", "url": "https://github.com/microsoft" },
            "plugins": [
                {
                    "name": "dotnet",
                    "description": "General .NET development skills",
                    "source": "./plugins/dotnet"
                },
                {
                    "name": "aspnet",
                    "description": "ASP.NET skills",
                    "source": "./plugins/aspnet"
                }
            ]
        }"#;

        let m = Marketplace::from_json(json).expect("should parse");
        assert_eq!(m.name, "dotnet-agent-skills");
        assert_eq!(m.owner.name, "Microsoft");
        assert_eq!(m.plugins.len(), 2);

        assert!(
            matches!(&m.plugins[0].source, PluginSource::RelativePath(p) if p == "./plugins/dotnet")
        );
        assert!(
            matches!(&m.plugins[1].source, PluginSource::RelativePath(p) if p == "./plugins/aspnet")
        );
    }

    #[test]
    fn parse_marketplace_with_github_source() {
        let json = br#"{
            "name": "community-skills",
            "owner": { "name": "alice" },
            "plugins": [
                {
                    "name": "rust-skills",
                    "source": {
                        "source": "github",
                        "repo": "alice/rust-skills",
                        "ref": "main",
                        "sha": "abc123"
                    }
                }
            ]
        }"#;

        let m = Marketplace::from_json(json).expect("should parse");
        assert_eq!(m.plugins.len(), 1);

        match &m.plugins[0].source {
            PluginSource::Structured(StructuredSource::GitHub { repo, git_ref, sha }) => {
                assert_eq!(repo, "alice/rust-skills");
                assert_eq!(git_ref.as_deref(), Some("main"));
                assert_eq!(sha.as_deref(), Some("abc123"));
            }
            other => panic!("expected GitHub source, got {other:?}"),
        }
    }

    #[test]
    fn parse_marketplace_with_git_subdir_source() {
        let json = br#"{
            "name": "mono-repo-skills",
            "owner": { "name": "corp" },
            "plugins": [
                {
                    "name": "backend",
                    "source": {
                        "source": "git-subdir",
                        "url": "https://github.com/corp/mono.git",
                        "path": "skills/backend",
                        "ref": "v2"
                    }
                }
            ]
        }"#;

        let m = Marketplace::from_json(json).expect("should parse");

        match &m.plugins[0].source {
            PluginSource::Structured(StructuredSource::GitSubdir {
                url,
                path,
                git_ref,
                sha,
            }) => {
                assert_eq!(url, "https://github.com/corp/mono.git");
                assert_eq!(path, "skills/backend");
                assert_eq!(git_ref.as_deref(), Some("v2"));
                assert!(sha.is_none());
            }
            other => panic!("expected GitSubdir source, got {other:?}"),
        }
    }

    #[rstest]
    #[case::missing_name(br#"{ "owner": { "name": "x" }, "plugins": [] }"#, "name")]
    #[case::missing_owner(br#"{ "name": "x", "plugins": [] }"#, "owner")]
    #[case::missing_plugins(br#"{ "name": "x", "owner": { "name": "x" } }"#, "plugins")]
    fn reject_missing_required_fields(#[case] json: &[u8], #[case] field: &str) {
        let err = Marketplace::from_json(json).expect_err("should fail");
        let msg = err.to_string();
        assert!(
            msg.contains(field),
            "error should mention `{field}`, got: {msg}"
        );
    }

    #[test]
    fn parse_marketplace_with_git_url_source() {
        let json = br#"{
            "name": "url-skills",
            "owner": { "name": "bob" },
            "plugins": [
                {
                    "name": "remote-plugin",
                    "source": {
                        "source": "url",
                        "url": "https://example.com/repo.git",
                        "ref": "v1.0",
                        "sha": "deadbeef"
                    }
                }
            ]
        }"#;

        let m = Marketplace::from_json(json).expect("should parse");
        assert_eq!(m.plugins.len(), 1);

        match &m.plugins[0].source {
            PluginSource::Structured(StructuredSource::GitUrl { url, git_ref, sha }) => {
                assert_eq!(url, "https://example.com/repo.git");
                assert_eq!(git_ref.as_deref(), Some("v1.0"));
                assert_eq!(sha.as_deref(), Some("deadbeef"));
            }
            other => panic!("expected GitUrl source, got {other:?}"),
        }
    }

    #[rstest]
    #[case::relative_path_traversal(
        br#"{
            "name": "evil",
            "owner": { "name": "mallory" },
            "plugins": [
                { "name": "p", "source": "../../../etc" }
            ]
        }"#
    )]
    #[case::relative_path_absolute(
        br#"{
            "name": "evil",
            "owner": { "name": "mallory" },
            "plugins": [
                { "name": "p", "source": "/etc/passwd" }
            ]
        }"#
    )]
    #[case::git_subdir_path_traversal(
        br#"{
            "name": "evil",
            "owner": { "name": "mallory" },
            "plugins": [{
                "name": "p",
                "source": {
                    "source": "git-subdir",
                    "url": "https://example.com/r.git",
                    "path": "../../etc"
                }
            }]
        }"#
    )]
    #[case::git_subdir_path_absolute(
        br#"{
            "name": "evil",
            "owner": { "name": "mallory" },
            "plugins": [{
                "name": "p",
                "source": {
                    "source": "git-subdir",
                    "url": "https://example.com/r.git",
                    "path": "/etc/passwd"
                }
            }]
        }"#
    )]
    fn reject_unsafe_paths(#[case] json: &[u8]) {
        let err = Marketplace::from_json(json).expect_err("should reject unsafe path");
        let msg = err.to_string();
        assert!(
            msg.contains("..") || msg.contains("absolute"),
            "error should mention the reason, got: {msg}"
        );
    }

    #[test]
    fn parse_marketplace_optional_fields_default_to_none() {
        let json = br#"{
            "name": "minimal",
            "owner": { "name": "anon" },
            "plugins": [
                {
                    "name": "bare",
                    "source": {
                        "source": "github",
                        "repo": "anon/bare"
                    }
                }
            ]
        }"#;

        let m = Marketplace::from_json(json).expect("should parse");
        assert!(m.owner.url.is_none(), "owner.url should be None");
        assert!(
            m.plugins[0].description.is_none(),
            "description should be None"
        );

        match &m.plugins[0].source {
            PluginSource::Structured(StructuredSource::GitHub { git_ref, sha, .. }) => {
                assert!(git_ref.is_none(), "git_ref should be None");
                assert!(sha.is_none(), "sha should be None");
            }
            other => panic!("expected GitHub source, got {other:?}"),
        }
    }
}

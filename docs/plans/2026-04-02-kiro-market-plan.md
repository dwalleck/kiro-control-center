# kiro-market Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Build a Rust CLI tool that discovers and installs Claude Code marketplace skills into Kiro CLI projects.

**Architecture:** Git-centric fetcher. Clones marketplace repos, parses `marketplace.json`/`plugin.json`, extracts SKILL.md files, merges multi-file skills, and copies to `.kiro/skills/`. Workspace with `kiro-market-core` (library, thiserror) and `kiro-market` (binary, anyhow+clap).

**Tech Stack:** Rust 2024 edition, clap (derive), git2, serde/serde_json/serde_yaml, pulldown-cmark, colored, thiserror+anyhow, tracing, rstest+tempfile

**Reference repositories for conventions:** `../rivets` (CLI patterns, error types, clap usage, integration tests), `../cyril` (workspace layout, pulldown-cmark usage)

**Reference marketplace repos for test fixtures:** `../skills` (Microsoft dotnet-agent-skills marketplace), `../dotnet-skills` (community marketplace with multi-file skills)

---

### Task 1: Workspace Scaffolding

Set up the Cargo workspace with both crates, all dependencies, and lints.

**Files:**
- Create: `Cargo.toml` (workspace root)
- Create: `crates/kiro-market-core/Cargo.toml`
- Create: `crates/kiro-market-core/src/lib.rs`
- Create: `crates/kiro-market/Cargo.toml`
- Create: `crates/kiro-market/src/main.rs`
- Create: `.gitignore`

**Step 1: Create workspace root Cargo.toml**

```toml
[workspace]
resolver = "2"
members = ["crates/kiro-market-core", "crates/kiro-market"]

[workspace.package]
version = "0.1.0"
edition = "2024"
rust-version = "1.85.0"
license = "MIT"
repository = "https://github.com/dwalleck/kiro-marketplace-cli"

[workspace.lints.rust]
unsafe_code = "forbid"

[workspace.lints.clippy]
all = "warn"
pedantic = "warn"

[workspace.dependencies]
# Serialization
serde = { version = "1", features = ["derive"] }
serde_json = "1"
serde_yaml = "0.9"

# CLI
clap = { version = "4", features = ["derive"] }

# Git
git2 = "0.20"

# Markdown
pulldown-cmark = { version = "0.13", default-features = false }

# Console
colored = "3"

# Error handling
thiserror = "2"
anyhow = "1"

# Logging
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }

# File paths
dirs = "6"

# Date/time
chrono = { version = "0.4", features = ["serde"] }

# Testing
rstest = "0.26"
tempfile = "3"

# Internal
kiro-market-core = { version = "0.1.0", path = "crates/kiro-market-core" }
```

**Step 2: Create kiro-market-core/Cargo.toml**

```toml
[package]
name = "kiro-market-core"
version.workspace = true
edition.workspace = true
license.workspace = true
description = "Core library for kiro-market: Claude Code marketplace skill installer for Kiro CLI"

[lints]
workspace = true

[dependencies]
serde = { workspace = true }
serde_json = { workspace = true }
serde_yaml = { workspace = true }
git2 = { workspace = true }
pulldown-cmark = { workspace = true }
thiserror = { workspace = true }
tracing = { workspace = true }
dirs = { workspace = true }
chrono = { workspace = true }

[dev-dependencies]
rstest = { workspace = true }
tempfile = { workspace = true }
```

**Step 3: Create kiro-market/Cargo.toml**

```toml
[package]
name = "kiro-market"
version.workspace = true
edition.workspace = true
license.workspace = true
description = "CLI tool to install Claude Code marketplace skills into Kiro CLI projects"

[lints]
workspace = true

[dependencies]
kiro-market-core = { workspace = true }
clap = { workspace = true }
anyhow = { workspace = true }
colored = { workspace = true }
tracing = { workspace = true }
tracing-subscriber = { workspace = true }

[dev-dependencies]
rstest = { workspace = true }
tempfile = { workspace = true }
```

**Step 4: Create minimal lib.rs and main.rs**

`crates/kiro-market-core/src/lib.rs`:
```rust
//! Core library for kiro-market.
//!
//! Provides types and logic for discovering and installing Claude Code
//! marketplace skills into Kiro CLI projects.
```

`crates/kiro-market/src/main.rs`:
```rust
//! kiro-market CLI binary.

use anyhow::Result;

fn main() -> Result<()> {
    println!("kiro-market");
    Ok(())
}
```

**Step 5: Create .gitignore**

```
/target
```

**Step 6: Build to verify**

Run: `cargo build`
Expected: Compiles successfully with no errors.

**Step 7: Commit**

```bash
git add Cargo.toml crates/ .gitignore
git commit -m "feat: scaffold workspace with kiro-market and kiro-market-core crates"
```

---

### Task 2: Core Data Types with Serde Deserialization

Define the marketplace, plugin, and skill types that map to the Claude Code JSON format. Test with real fixture data from the Microsoft dotnet-agent-skills marketplace.

**Files:**
- Create: `crates/kiro-market-core/src/marketplace.rs`
- Create: `crates/kiro-market-core/src/plugin.rs`
- Create: `crates/kiro-market-core/src/skill.rs`
- Modify: `crates/kiro-market-core/src/lib.rs`

**Step 1: Write tests for marketplace.json deserialization**

Create `crates/kiro-market-core/src/marketplace.rs` with types and tests:

```rust
//! Marketplace catalog types and parsing.
//!
//! Represents the Claude Code `.claude-plugin/marketplace.json` format.

use serde::Deserialize;

/// A marketplace catalog listing available plugins.
#[derive(Debug, Clone, Deserialize)]
pub struct Marketplace {
    pub name: String,
    pub owner: Owner,
    pub plugins: Vec<PluginEntry>,
}

/// Marketplace owner/maintainer info.
#[derive(Debug, Clone, Deserialize)]
pub struct Owner {
    pub name: String,
    pub email: Option<String>,
}

/// A plugin listed in a marketplace catalog.
#[derive(Debug, Clone, Deserialize)]
pub struct PluginEntry {
    pub name: String,
    pub source: PluginSource,
    pub description: Option<String>,
    pub version: Option<String>,
}

/// Where to fetch a plugin from.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum PluginSource {
    /// Relative path within the marketplace repo (starts with "./")
    RelativePath(String),
    /// Structured source (GitHub, git URL, git subdirectory)
    Structured(StructuredSource),
}

/// A structured plugin source with explicit type.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "source")]
pub enum StructuredSource {
    /// GitHub repository source
    #[serde(rename = "github")]
    GitHub {
        repo: String,
        #[serde(rename = "ref")]
        git_ref: Option<String>,
        sha: Option<String>,
    },
    /// Git URL source
    #[serde(rename = "url")]
    GitUrl {
        url: String,
        #[serde(rename = "ref")]
        git_ref: Option<String>,
        sha: Option<String>,
    },
    /// Subdirectory within a git repo
    #[serde(rename = "git-subdir")]
    GitSubdir {
        url: String,
        path: String,
        #[serde(rename = "ref")]
        git_ref: Option<String>,
        sha: Option<String>,
    },
}

impl Marketplace {
    /// Parse a marketplace from JSON bytes.
    pub fn from_json(json: &[u8]) -> Result<Self, serde_json::Error> {
        serde_json::from_slice(json)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[test]
    fn parse_marketplace_with_relative_sources() {
        let json = r#"{
            "name": "dotnet-agent-skills",
            "owner": { "name": ".NET Team at Microsoft" },
            "plugins": [
                {
                    "name": "dotnet",
                    "source": "./plugins/dotnet",
                    "description": "Core .NET skills"
                },
                {
                    "name": "dotnet-data",
                    "source": "./plugins/dotnet-data",
                    "description": "Data access skills"
                }
            ]
        }"#;

        let m = Marketplace::from_json(json.as_bytes()).unwrap();
        assert_eq!(m.name, "dotnet-agent-skills");
        assert_eq!(m.owner.name, ".NET Team at Microsoft");
        assert_eq!(m.plugins.len(), 2);
        assert!(matches!(&m.plugins[0].source, PluginSource::RelativePath(p) if p == "./plugins/dotnet"));
    }

    #[test]
    fn parse_marketplace_with_github_source() {
        let json = r#"{
            "name": "my-marketplace",
            "owner": { "name": "Test", "email": "test@example.com" },
            "plugins": [
                {
                    "name": "my-plugin",
                    "source": { "source": "github", "repo": "owner/repo", "ref": "v2.0.0" }
                }
            ]
        }"#;

        let m = Marketplace::from_json(json.as_bytes()).unwrap();
        assert!(matches!(
            &m.plugins[0].source,
            PluginSource::Structured(StructuredSource::GitHub { repo, git_ref, .. })
            if repo == "owner/repo" && git_ref.as_deref() == Some("v2.0.0")
        ));
    }

    #[test]
    fn parse_marketplace_with_git_subdir_source() {
        let json = r#"{
            "name": "mono-marketplace",
            "owner": { "name": "Test" },
            "plugins": [
                {
                    "name": "my-plugin",
                    "source": {
                        "source": "git-subdir",
                        "url": "https://github.com/org/monorepo.git",
                        "path": "tools/plugin"
                    }
                }
            ]
        }"#;

        let m = Marketplace::from_json(json.as_bytes()).unwrap();
        assert!(matches!(
            &m.plugins[0].source,
            PluginSource::Structured(StructuredSource::GitSubdir { url, path, .. })
            if url.contains("monorepo") && path == "tools/plugin"
        ));
    }

    #[rstest]
    #[case::missing_name(r#"{"owner":{"name":"x"},"plugins":[]}"#)]
    #[case::missing_owner(r#"{"name":"x","plugins":[]}"#)]
    #[case::missing_plugins(r#"{"name":"x","owner":{"name":"x"}}"#)]
    fn parse_marketplace_rejects_missing_required_fields(#[case] json: &str) {
        assert!(Marketplace::from_json(json.as_bytes()).is_err());
    }
}
```

**Step 2: Write tests for plugin.json deserialization**

Create `crates/kiro-market-core/src/plugin.rs`:

```rust
//! Plugin manifest types and parsing.
//!
//! Represents the Claude Code `.claude-plugin/plugin.json` or per-plugin `plugin.json` format.

use serde::Deserialize;

/// A plugin manifest describing a plugin's contents.
#[derive(Debug, Clone, Deserialize)]
pub struct PluginManifest {
    pub name: String,
    pub version: Option<String>,
    pub description: Option<String>,
    /// Skill paths, e.g. ["./skills/"] or ["./skills/foo", "./skills/bar"]
    #[serde(default)]
    pub skills: Vec<String>,
}

impl PluginManifest {
    /// Parse a plugin manifest from JSON bytes.
    pub fn from_json(json: &[u8]) -> Result<Self, serde_json::Error> {
        serde_json::from_slice(json)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_plugin_manifest_with_skill_paths() {
        let json = r#"{
            "name": "dotnet",
            "version": "0.1.0",
            "description": "Common everyday C#/.NET coding skills.",
            "skills": ["./skills/"]
        }"#;

        let p = PluginManifest::from_json(json.as_bytes()).unwrap();
        assert_eq!(p.name, "dotnet");
        assert_eq!(p.version.as_deref(), Some("0.1.0"));
        assert_eq!(p.skills, vec!["./skills/"]);
    }

    #[test]
    fn parse_plugin_manifest_without_skills() {
        let json = r#"{ "name": "hooks-only", "version": "1.0.0" }"#;

        let p = PluginManifest::from_json(json.as_bytes()).unwrap();
        assert!(p.skills.is_empty());
    }

    #[test]
    fn parse_plugin_manifest_with_explicit_skill_list() {
        let json = r#"{
            "name": "dotnet-skills",
            "version": "1.3.0",
            "skills": [
                "./skills/akka-best-practices",
                "./skills/tunit"
            ]
        }"#;

        let p = PluginManifest::from_json(json.as_bytes()).unwrap();
        assert_eq!(p.skills.len(), 2);
        assert_eq!(p.skills[1], "./skills/tunit");
    }
}
```

**Step 3: Write skill frontmatter parsing types**

Create `crates/kiro-market-core/src/skill.rs`:

```rust
//! Skill types, SKILL.md parsing, and multi-file merging.

use serde::Deserialize;

/// YAML frontmatter from a SKILL.md file.
#[derive(Debug, Clone, Deserialize)]
pub struct SkillFrontmatter {
    pub name: String,
    pub description: String,
    /// Claude Code-specific field — not used by Kiro but preserved for context.
    #[serde(default)]
    pub invocable: Option<bool>,
}

/// A parsed skill ready for installation.
#[derive(Debug, Clone)]
pub struct Skill {
    pub name: String,
    pub description: String,
    /// The full SKILL.md content (frontmatter + body, with companions merged).
    pub content: String,
}

/// Parse YAML frontmatter from a SKILL.md file.
///
/// Expects the file to start with `---\n`, followed by YAML, followed by `---\n`.
/// Returns the parsed frontmatter and the byte offset where the body begins.
pub fn parse_frontmatter(content: &str) -> Result<(SkillFrontmatter, usize), ParseError> {
    let content = content.trim_start();
    if !content.starts_with("---") {
        return Err(ParseError::MissingFrontmatter);
    }

    let after_first_fence = &content[3..];
    let after_first_fence = after_first_fence.strip_prefix('\n').unwrap_or(after_first_fence);

    let closing = after_first_fence
        .find("\n---")
        .ok_or(ParseError::UnclosedFrontmatter)?;

    let yaml = &after_first_fence[..closing];
    let body_start = content.len() - after_first_fence.len() + closing + 4; // +4 for "\n---"

    let frontmatter: SkillFrontmatter =
        serde_yaml::from_str(yaml).map_err(|e| ParseError::InvalidYaml(e.to_string()))?;

    Ok((frontmatter, body_start))
}

/// Errors that can occur when parsing a SKILL.md file.
#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("SKILL.md is missing YAML frontmatter (expected to start with ---)")]
    MissingFrontmatter,
    #[error("SKILL.md frontmatter is not closed (missing closing ---)")]
    UnclosedFrontmatter,
    #[error("invalid YAML in frontmatter: {0}")]
    InvalidYaml(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid_frontmatter() {
        let content = "---\nname: tunit\ndescription: Write TUnit tests\n---\n\n# TUnit\n\nBody here.";
        let (fm, offset) = parse_frontmatter(content).unwrap();
        assert_eq!(fm.name, "tunit");
        assert_eq!(fm.description, "Write TUnit tests");
        assert!(content[offset..].contains("# TUnit"));
    }

    #[test]
    fn parse_frontmatter_with_invocable_field() {
        let content = "---\nname: test\ndescription: desc\ninvocable: false\n---\nBody";
        let (fm, _) = parse_frontmatter(content).unwrap();
        assert_eq!(fm.invocable, Some(false));
    }

    #[test]
    fn parse_frontmatter_missing_fence() {
        let content = "# No frontmatter\n\nJust a body.";
        assert!(matches!(
            parse_frontmatter(content),
            Err(ParseError::MissingFrontmatter)
        ));
    }

    #[test]
    fn parse_frontmatter_unclosed() {
        let content = "---\nname: broken\n\nNo closing fence.";
        assert!(matches!(
            parse_frontmatter(content),
            Err(ParseError::UnclosedFrontmatter)
        ));
    }

    #[test]
    fn parse_frontmatter_invalid_yaml() {
        let content = "---\n[invalid yaml\n---\nBody";
        assert!(matches!(
            parse_frontmatter(content),
            Err(ParseError::InvalidYaml(_))
        ));
    }
}
```

**Step 4: Wire up lib.rs**

Update `crates/kiro-market-core/src/lib.rs`:

```rust
//! Core library for kiro-market.
//!
//! Provides types and logic for discovering and installing Claude Code
//! marketplace skills into Kiro CLI projects.

pub mod marketplace;
pub mod plugin;
pub mod skill;
```

**Step 5: Run tests**

Run: `cargo test -p kiro-market-core`
Expected: All tests pass.

**Step 6: Commit**

```bash
git add crates/kiro-market-core/src/
git commit -m "feat: add core data types for marketplace, plugin, and skill parsing"
```

---

### Task 3: Error Types

Define the domain error types in a dedicated module, following the rivets pattern of grouped error enums with `#[non_exhaustive]`.

**Files:**
- Create: `crates/kiro-market-core/src/error.rs`
- Modify: `crates/kiro-market-core/src/lib.rs`

**Step 1: Write error types with tests**

Create `crates/kiro-market-core/src/error.rs`:

```rust
//! Error types for kiro-market-core operations.

use std::io;
use thiserror::Error;

/// Errors from marketplace operations (registry, parsing).
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum MarketplaceError {
    #[error("marketplace '{name}' not found")]
    NotFound { name: String },

    #[error("marketplace '{name}' is already registered")]
    AlreadyRegistered { name: String },

    #[error("invalid marketplace.json: {reason}")]
    InvalidManifest { reason: String },

    #[error("marketplace.json not found at expected path")]
    ManifestNotFound,
}

/// Errors from plugin operations.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum PluginError {
    #[error("plugin '{plugin}' not found in marketplace '{marketplace}'")]
    NotFound { plugin: String, marketplace: String },

    #[error("invalid plugin.json: {reason}")]
    InvalidManifest { reason: String },

    #[error("plugin.json not found at expected path")]
    ManifestNotFound,

    #[error("plugin '{name}' has no skills")]
    NoSkills { name: String },
}

/// Errors from skill installation and merging.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum SkillError {
    #[error("skill '{name}' is already installed (use --force to overwrite)")]
    AlreadyInstalled { name: String },

    #[error("SKILL.md not found in skill directory")]
    SkillMdNotFound,

    #[error("failed to merge companion file '{path}' for skill '{skill}': {reason}")]
    MergeFailed {
        skill: String,
        path: String,
        reason: String,
    },
}

/// Errors from git operations.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum GitError {
    #[error("git clone failed for '{url}'")]
    CloneFailed {
        url: String,
        #[source]
        source: git2::Error,
    },

    #[error("git pull failed for '{path}'")]
    PullFailed {
        path: String,
        #[source]
        source: git2::Error,
    },

    #[error("failed to open git repository at '{path}'")]
    OpenFailed {
        path: String,
        #[source]
        source: git2::Error,
    },
}

/// Top-level error type for kiro-market-core.
#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    Marketplace(#[from] MarketplaceError),

    #[error(transparent)]
    Plugin(#[from] PluginError),

    #[error(transparent)]
    Skill(#[from] SkillError),

    #[error(transparent)]
    Git(#[from] GitError),

    #[error("IO error: {0}")]
    Io(#[from] io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

/// Convenience result type for kiro-market-core.
pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;
    use std::error::Error as StdError;

    #[rstest]
    #[case::not_found(
        MarketplaceError::NotFound { name: "my-market".to_string() },
        "marketplace 'my-market' not found"
    )]
    #[case::already_registered(
        MarketplaceError::AlreadyRegistered { name: "my-market".to_string() },
        "marketplace 'my-market' is already registered"
    )]
    #[case::invalid_manifest(
        MarketplaceError::InvalidManifest { reason: "missing plugins field".to_string() },
        "invalid marketplace.json: missing plugins field"
    )]
    fn marketplace_error_display(#[case] error: MarketplaceError, #[case] expected: &str) {
        assert_eq!(error.to_string(), expected);
    }

    #[rstest]
    #[case::not_found(
        PluginError::NotFound { plugin: "dotnet".to_string(), marketplace: "ms".to_string() },
        "plugin 'dotnet' not found in marketplace 'ms'"
    )]
    #[case::no_skills(
        PluginError::NoSkills { name: "hooks-only".to_string() },
        "plugin 'hooks-only' has no skills"
    )]
    fn plugin_error_display(#[case] error: PluginError, #[case] expected: &str) {
        assert_eq!(error.to_string(), expected);
    }

    #[test]
    fn skill_already_installed_display() {
        let error = SkillError::AlreadyInstalled {
            name: "tunit".to_string(),
        };
        assert_eq!(
            error.to_string(),
            "skill 'tunit' is already installed (use --force to overwrite)"
        );
    }

    #[test]
    fn marketplace_error_converts_to_top_level() {
        let err = MarketplaceError::NotFound {
            name: "x".to_string(),
        };
        let top: Error = err.into();
        assert!(matches!(top, Error::Marketplace(MarketplaceError::NotFound { .. })));
    }

    #[test]
    fn git_clone_error_has_source() {
        let git_err = git2::Error::from_str("connection refused");
        let err = GitError::CloneFailed {
            url: "https://example.com".to_string(),
            source: git_err,
        };
        assert!(err.source().is_some());
    }

    #[test]
    fn io_error_converts_to_top_level() {
        let io_err = io::Error::new(io::ErrorKind::NotFound, "file missing");
        let top: Error = io_err.into();
        assert!(matches!(top, Error::Io(_)));
    }
}
```

**Step 2: Add module to lib.rs**

Add `pub mod error;` to `crates/kiro-market-core/src/lib.rs`.

**Step 3: Run tests**

Run: `cargo test -p kiro-market-core`
Expected: All tests pass.

**Step 4: Commit**

```bash
git add crates/kiro-market-core/src/error.rs crates/kiro-market-core/src/lib.rs
git commit -m "feat: add domain error types with thiserror"
```

---

### Task 4: Skill Merging Logic

Implement the companion file detection and merge flow using pulldown-cmark to find relative `.md` links.

**Files:**
- Modify: `crates/kiro-market-core/src/skill.rs`

**Step 1: Write failing tests for link extraction**

Add to `skill.rs` tests:

```rust
#[test]
fn extract_relative_md_links() {
    let body = r#"See [type mapping](references/type-mapping.md) for details.
Also check [diagnostics](references/diagnostics.md).
Ignore [external](https://example.com/docs.md) links.
Ignore [non-md](references/data.json) files."#;

    let links = extract_relative_md_links(body);
    assert_eq!(links.len(), 2);
    assert_eq!(links[0], "references/type-mapping.md");
    assert_eq!(links[1], "references/diagnostics.md");
}

#[test]
fn extract_no_links_from_plain_text() {
    let body = "No links here, just text.";
    let links = extract_relative_md_links(body);
    assert!(links.is_empty());
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test -p kiro-market-core -- extract_relative`
Expected: FAIL — `extract_relative_md_links` not defined.

**Step 3: Implement link extraction**

Add to `skill.rs`:

```rust
use pulldown_cmark::{Event, Parser, Tag, TagEnd};

/// Extract relative .md file links from markdown body text.
///
/// Returns paths like `references/type-mapping.md`. Ignores absolute URLs
/// and non-.md links.
pub fn extract_relative_md_links(markdown: &str) -> Vec<String> {
    let parser = Parser::new(markdown);
    let mut links = Vec::new();

    for event in parser {
        if let Event::Start(Tag::Link { dest_url, .. }) = event {
            let url = dest_url.as_ref();
            if url.ends_with(".md")
                && !url.starts_with("http://")
                && !url.starts_with("https://")
                && !url.starts_with('/')
            {
                links.push(url.to_string());
            }
        }
    }

    links
}
```

**Step 4: Run tests to verify they pass**

Run: `cargo test -p kiro-market-core -- extract_relative`
Expected: PASS.

**Step 5: Write failing tests for skill merging**

Add to `skill.rs` tests:

```rust
#[test]
fn merge_skill_with_companions() {
    let skill_md = "---\nname: pinvoke\ndescription: P/Invoke guide\n---\n\n# P/Invoke\n\nSee [type mapping](references/type-mapping.md) for types.\n";
    let companions = vec![
        ("references/type-mapping.md", "# Type Mapping\n\nint -> int\n"),
    ];

    let merged = merge_skill(skill_md, &companions).unwrap();
    assert!(merged.contains("# P/Invoke"));
    assert!(merged.contains("<!-- Merged from references/type-mapping.md -->"));
    assert!(merged.contains("# Type Mapping"));
}

#[test]
fn merge_skill_without_companions() {
    let skill_md = "---\nname: simple\ndescription: Simple skill\n---\n\n# Simple\n\nNo companions.";
    let merged = merge_skill(skill_md, &[]).unwrap();
    assert_eq!(merged, skill_md);
}
```

**Step 6: Run tests to verify they fail**

Run: `cargo test -p kiro-market-core -- merge_skill`
Expected: FAIL — `merge_skill` not defined.

**Step 7: Implement merge_skill**

Add to `skill.rs`:

```rust
/// Merge a SKILL.md with its companion .md files.
///
/// For each relative .md link found in the body, if a matching companion
/// is provided, its content is appended to the end of the file with a
/// separator comment. Links in the body are rewritten to point to the
/// appended section's heading.
///
/// Returns the merged SKILL.md content.
pub fn merge_skill(
    skill_content: &str,
    companions: &[(&str, &str)],
) -> std::result::Result<String, ParseError> {
    if companions.is_empty() {
        return Ok(skill_content.to_string());
    }

    let (_, body_start) = parse_frontmatter(skill_content)?;
    let body = &skill_content[body_start..];

    let referenced_links = extract_relative_md_links(body);

    // Build a map of path -> content for quick lookup
    let companion_map: std::collections::HashMap<&str, &str> =
        companions.iter().copied().collect();

    let mut result = skill_content.to_string();

    for link_path in &referenced_links {
        if let Some(content) = companion_map.get(link_path.as_str()) {
            result.push_str(&format!(
                "\n\n---\n<!-- Merged from {link_path} -->\n{content}"
            ));
        }
    }

    Ok(result)
}
```

**Step 8: Run all skill tests**

Run: `cargo test -p kiro-market-core -- skill`
Expected: All tests pass.

**Step 9: Commit**

```bash
git add crates/kiro-market-core/src/skill.rs
git commit -m "feat: add skill merging logic for multi-file Claude Code skills"
```

---

### Task 5: Cache and Path Management

Handle the local cache directory structure and the known marketplaces registry file.

**Files:**
- Create: `crates/kiro-market-core/src/cache.rs`
- Modify: `crates/kiro-market-core/src/lib.rs`

**Step 1: Write failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn cache_dir_creates_structure() {
        let tmp = TempDir::new().unwrap();
        let cache = CacheDir::with_root(tmp.path().to_path_buf());
        cache.ensure_dirs().unwrap();

        assert!(cache.marketplaces_dir().exists());
        assert!(cache.plugins_dir().exists());
    }

    #[test]
    fn known_marketplaces_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let cache = CacheDir::with_root(tmp.path().to_path_buf());
        cache.ensure_dirs().unwrap();

        let entry = KnownMarketplace {
            name: "dotnet-agent-skills".to_string(),
            source: MarketplaceSource::GitHub {
                repo: "dotnet/skills".to_string(),
            },
            added_at: chrono::Utc::now(),
        };

        cache.add_known_marketplace(&entry).unwrap();
        let loaded = cache.load_known_marketplaces().unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].name, "dotnet-agent-skills");
    }

    #[test]
    fn known_marketplaces_rejects_duplicate() {
        let tmp = TempDir::new().unwrap();
        let cache = CacheDir::with_root(tmp.path().to_path_buf());
        cache.ensure_dirs().unwrap();

        let entry = KnownMarketplace {
            name: "my-market".to_string(),
            source: MarketplaceSource::LocalPath {
                path: "/tmp/test".to_string(),
            },
            added_at: chrono::Utc::now(),
        };

        cache.add_known_marketplace(&entry).unwrap();
        let result = cache.add_known_marketplace(&entry);
        assert!(result.is_err());
    }
}
```

**Step 2: Run to verify failure**

Run: `cargo test -p kiro-market-core -- cache`
Expected: FAIL.

**Step 3: Implement cache module**

Create `crates/kiro-market-core/src/cache.rs`:

```rust
//! Local cache directory and marketplace registry management.
//!
//! Cache lives at `~/.local/share/kiro-market/` (XDG data dir).

use crate::error::MarketplaceError;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// Where a marketplace can be fetched from.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum MarketplaceSource {
    #[serde(rename = "github")]
    GitHub { repo: String },
    #[serde(rename = "git_url")]
    GitUrl { url: String },
    #[serde(rename = "local")]
    LocalPath { path: String },
}

/// A registered marketplace entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnownMarketplace {
    pub name: String,
    pub source: MarketplaceSource,
    pub added_at: DateTime<Utc>,
}

/// Manages the local cache directory structure.
pub struct CacheDir {
    root: PathBuf,
}

impl CacheDir {
    /// Create a CacheDir using the default XDG data directory.
    pub fn default_location() -> Option<Self> {
        dirs::data_dir().map(|d| Self {
            root: d.join("kiro-market"),
        })
    }

    /// Create a CacheDir at a specific root (for testing).
    pub fn with_root(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn marketplaces_dir(&self) -> PathBuf {
        self.root.join("marketplaces")
    }

    pub fn plugins_dir(&self) -> PathBuf {
        self.root.join("plugins")
    }

    fn known_marketplaces_path(&self) -> PathBuf {
        self.root.join("known_marketplaces.json")
    }

    /// Marketplace clone directory for a given marketplace name.
    pub fn marketplace_path(&self, name: &str) -> PathBuf {
        self.marketplaces_dir().join(name)
    }

    /// Plugin clone directory for a given marketplace + plugin.
    pub fn plugin_path(&self, marketplace: &str, plugin: &str) -> PathBuf {
        self.plugins_dir().join(marketplace).join(plugin)
    }

    /// Ensure all cache directories exist.
    pub fn ensure_dirs(&self) -> std::io::Result<()> {
        fs::create_dir_all(self.marketplaces_dir())?;
        fs::create_dir_all(self.plugins_dir())?;
        Ok(())
    }

    /// Load the list of known marketplaces from disk.
    pub fn load_known_marketplaces(&self) -> crate::error::Result<Vec<KnownMarketplace>> {
        let path = self.known_marketplaces_path();
        if !path.exists() {
            return Ok(Vec::new());
        }
        let data = fs::read_to_string(&path)?;
        let entries: Vec<KnownMarketplace> = serde_json::from_str(&data)?;
        Ok(entries)
    }

    /// Add a marketplace to the known list. Errors if already registered.
    pub fn add_known_marketplace(
        &self,
        entry: &KnownMarketplace,
    ) -> crate::error::Result<()> {
        let mut entries = self.load_known_marketplaces()?;

        if entries.iter().any(|e| e.name == entry.name) {
            return Err(MarketplaceError::AlreadyRegistered {
                name: entry.name.clone(),
            }
            .into());
        }

        entries.push(entry.clone());
        let json = serde_json::to_string_pretty(&entries)?;
        fs::write(self.known_marketplaces_path(), json)?;
        Ok(())
    }

    /// Remove a marketplace from the known list. Errors if not found.
    pub fn remove_known_marketplace(&self, name: &str) -> crate::error::Result<()> {
        let mut entries = self.load_known_marketplaces()?;
        let original_len = entries.len();
        entries.retain(|e| e.name != name);

        if entries.len() == original_len {
            return Err(MarketplaceError::NotFound {
                name: name.to_string(),
            }
            .into());
        }

        let json = serde_json::to_string_pretty(&entries)?;
        fs::write(self.known_marketplaces_path(), json)?;
        Ok(())
    }
}
```

**Step 4: Add to lib.rs, run tests**

Add `pub mod cache;` to lib.rs.

Run: `cargo test -p kiro-market-core -- cache`
Expected: All pass.

**Step 5: Commit**

```bash
git add crates/kiro-market-core/src/cache.rs crates/kiro-market-core/src/lib.rs
git commit -m "feat: add cache directory management and marketplace registry"
```

---

### Task 6: Git Operations

Wrap git2 for cloning and pulling marketplace/plugin repos.

**Files:**
- Create: `crates/kiro-market-core/src/git.rs`
- Modify: `crates/kiro-market-core/src/lib.rs`

**Step 1: Write tests using tempfile repos**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn clone_local_repo() {
        // Create a bare repo to clone from
        let origin = TempDir::new().unwrap();
        let repo = git2::Repository::init(origin.path()).unwrap();

        // Add a file and commit
        let mut index = repo.index().unwrap();
        let file_path = origin.path().join("test.txt");
        std::fs::write(&file_path, "hello").unwrap();
        index.add_path(std::path::Path::new("test.txt")).unwrap();
        index.write().unwrap();
        let tree_id = index.write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        let sig = git2::Signature::now("test", "test@test.com").unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[]).unwrap();

        // Clone it
        let dest = TempDir::new().unwrap();
        let result = clone_repo(
            origin.path().to_str().unwrap(),
            dest.path(),
            None,
        );
        assert!(result.is_ok());
        assert!(dest.path().join("test.txt").exists());
    }
}
```

**Step 2: Implement git operations**

Create `crates/kiro-market-core/src/git.rs`:

```rust
//! Git operations for cloning and updating marketplace/plugin repos.

use crate::error::GitError;
use std::path::Path;
use tracing::info;

/// Clone a git repository to a local path.
pub fn clone_repo(
    url: &str,
    dest: &Path,
    git_ref: Option<&str>,
) -> Result<git2::Repository, GitError> {
    info!(url, ?dest, "Cloning repository");

    let repo = git2::Repository::clone(url, dest).map_err(|e| GitError::CloneFailed {
        url: url.to_string(),
        source: e,
    })?;

    if let Some(refname) = git_ref {
        checkout_ref(&repo, refname).map_err(|e| GitError::CloneFailed {
            url: url.to_string(),
            source: e,
        })?;
    }

    Ok(repo)
}

/// Pull (fast-forward) an existing repository.
pub fn pull_repo(path: &Path) -> Result<(), GitError> {
    let path_str = path.display().to_string();
    info!(path = %path_str, "Pulling repository");

    let repo =
        git2::Repository::open(path).map_err(|e| GitError::OpenFailed {
            path: path_str.clone(),
            source: e,
        })?;

    let mut remote = repo.find_remote("origin").map_err(|e| GitError::PullFailed {
        path: path_str.clone(),
        source: e,
    })?;

    let head = repo.head().map_err(|e| GitError::PullFailed {
        path: path_str.clone(),
        source: e,
    })?;

    let branch_name = head
        .shorthand()
        .unwrap_or("main");

    remote
        .fetch(&[branch_name], None, None)
        .map_err(|e| GitError::PullFailed {
            path: path_str.clone(),
            source: e,
        })?;

    // Fast-forward to FETCH_HEAD
    let fetch_head = repo
        .find_reference("FETCH_HEAD")
        .map_err(|e| GitError::PullFailed {
            path: path_str.clone(),
            source: e,
        })?;

    let fetch_commit = repo
        .reference_to_annotated_commit(&fetch_head)
        .map_err(|e| GitError::PullFailed {
            path: path_str.clone(),
            source: e,
        })?;

    let (analysis, _) = repo.merge_analysis(&[&fetch_commit]).map_err(|e| {
        GitError::PullFailed {
            path: path_str.clone(),
            source: e,
        }
    })?;

    if analysis.is_fast_forward() {
        let mut reference = repo.head().map_err(|e| GitError::PullFailed {
            path: path_str.clone(),
            source: e,
        })?;

        reference
            .set_target(fetch_commit.id(), "kiro-market: fast-forward pull")
            .map_err(|e| GitError::PullFailed {
                path: path_str.clone(),
                source: e,
            })?;

        repo.checkout_head(Some(git2::build::CheckoutBuilder::default().force()))
            .map_err(|e| GitError::PullFailed {
                path: path_str.clone(),
                source: e,
            })?;
    }

    Ok(())
}

/// Convert a GitHub `owner/repo` shorthand to a full HTTPS URL.
pub fn github_repo_to_url(repo: &str) -> String {
    format!("https://github.com/{repo}.git")
}

fn checkout_ref(repo: &git2::Repository, refname: &str) -> Result<(), git2::Error> {
    let (object, reference) = repo.revparse_ext(refname)?;
    repo.checkout_tree(&object, None)?;

    match reference {
        Some(r) => {
            let name = r.name().unwrap_or(refname);
            repo.set_head(name)?;
        }
        None => repo.set_head_detached(object.id())?,
    }

    Ok(())
}
```

**Step 3: Run tests**

Run: `cargo test -p kiro-market-core -- git`
Expected: PASS.

**Step 4: Commit**

```bash
git add crates/kiro-market-core/src/git.rs crates/kiro-market-core/src/lib.rs
git commit -m "feat: add git clone and pull operations via git2"
```

---

### Task 7: Project State Management

Handle the `.kiro/installed-skills.json` tracking file and `.kiro/skills/` directory for a Kiro project.

**Files:**
- Create: `crates/kiro-market-core/src/project.rs`
- Modify: `crates/kiro-market-core/src/lib.rs`

**Step 1: Write failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn install_skill_creates_directory_and_file() {
        let tmp = TempDir::new().unwrap();
        let project = KiroProject::new(tmp.path().to_path_buf());

        let skill = InstalledSkillMeta {
            marketplace: "test-market".to_string(),
            plugin: "test-plugin".to_string(),
            version: Some("1.0.0".to_string()),
            installed_at: chrono::Utc::now(),
        };

        project
            .install_skill("my-skill", "---\nname: my-skill\n---\n# Content", &skill)
            .unwrap();

        assert!(tmp.path().join(".kiro/skills/my-skill/SKILL.md").exists());

        let installed = project.load_installed().unwrap();
        assert!(installed.skills.contains_key("my-skill"));
    }

    #[test]
    fn install_skill_rejects_duplicate_without_force() {
        let tmp = TempDir::new().unwrap();
        let project = KiroProject::new(tmp.path().to_path_buf());

        let skill = InstalledSkillMeta {
            marketplace: "m".to_string(),
            plugin: "p".to_string(),
            version: None,
            installed_at: chrono::Utc::now(),
        };

        project.install_skill("dup", "content", &skill).unwrap();
        let result = project.install_skill("dup", "content", &skill);
        assert!(result.is_err());
    }

    #[test]
    fn remove_skill_deletes_directory_and_tracking() {
        let tmp = TempDir::new().unwrap();
        let project = KiroProject::new(tmp.path().to_path_buf());

        let skill = InstalledSkillMeta {
            marketplace: "m".to_string(),
            plugin: "p".to_string(),
            version: None,
            installed_at: chrono::Utc::now(),
        };

        project.install_skill("removable", "content", &skill).unwrap();
        project.remove_skill("removable").unwrap();

        assert!(!tmp.path().join(".kiro/skills/removable").exists());
        let installed = project.load_installed().unwrap();
        assert!(!installed.skills.contains_key("removable"));
    }
}
```

**Step 2: Implement project module**

Create `crates/kiro-market-core/src/project.rs`:

```rust
//! Kiro project interaction: .kiro/skills/ and installed-skills.json.

use crate::error::SkillError;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

/// Metadata about an installed skill, stored in installed-skills.json.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledSkillMeta {
    pub marketplace: String,
    pub plugin: String,
    pub version: Option<String>,
    pub installed_at: DateTime<Utc>,
}

/// The installed-skills.json tracking file.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InstalledSkills {
    pub skills: HashMap<String, InstalledSkillMeta>,
}

/// Represents a Kiro project directory.
pub struct KiroProject {
    root: PathBuf,
}

impl KiroProject {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    fn kiro_dir(&self) -> PathBuf {
        self.root.join(".kiro")
    }

    fn skills_dir(&self) -> PathBuf {
        self.kiro_dir().join("skills")
    }

    fn installed_path(&self) -> PathBuf {
        self.kiro_dir().join("installed-skills.json")
    }

    /// Load the installed skills tracking file.
    pub fn load_installed(&self) -> crate::error::Result<InstalledSkills> {
        let path = self.installed_path();
        if !path.exists() {
            return Ok(InstalledSkills::default());
        }
        let data = fs::read_to_string(&path)?;
        let installed: InstalledSkills = serde_json::from_str(&data)?;
        Ok(installed)
    }

    fn save_installed(&self, installed: &InstalledSkills) -> crate::error::Result<()> {
        fs::create_dir_all(self.kiro_dir())?;
        let json = serde_json::to_string_pretty(installed)?;
        fs::write(self.installed_path(), json)?;
        Ok(())
    }

    /// Install a skill into .kiro/skills/<name>/SKILL.md.
    pub fn install_skill(
        &self,
        name: &str,
        content: &str,
        meta: &InstalledSkillMeta,
    ) -> crate::error::Result<()> {
        let skill_dir = self.skills_dir().join(name);

        if skill_dir.exists() {
            return Err(SkillError::AlreadyInstalled {
                name: name.to_string(),
            }
            .into());
        }

        fs::create_dir_all(&skill_dir)?;
        fs::write(skill_dir.join("SKILL.md"), content)?;

        let mut installed = self.load_installed()?;
        installed.skills.insert(name.to_string(), meta.clone());
        self.save_installed(&installed)?;

        Ok(())
    }

    /// Force-install a skill (overwrite if exists).
    pub fn install_skill_force(
        &self,
        name: &str,
        content: &str,
        meta: &InstalledSkillMeta,
    ) -> crate::error::Result<()> {
        let skill_dir = self.skills_dir().join(name);

        if skill_dir.exists() {
            fs::remove_dir_all(&skill_dir)?;
        }

        fs::create_dir_all(&skill_dir)?;
        fs::write(skill_dir.join("SKILL.md"), content)?;

        let mut installed = self.load_installed()?;
        installed.skills.insert(name.to_string(), meta.clone());
        self.save_installed(&installed)?;

        Ok(())
    }

    /// Remove an installed skill.
    pub fn remove_skill(&self, name: &str) -> crate::error::Result<()> {
        let skill_dir = self.skills_dir().join(name);

        if skill_dir.exists() {
            fs::remove_dir_all(&skill_dir)?;
        }

        let mut installed = self.load_installed()?;
        installed.skills.remove(name);
        self.save_installed(&installed)?;

        Ok(())
    }

    /// List all installed skills.
    pub fn list_installed(&self) -> crate::error::Result<InstalledSkills> {
        self.load_installed()
    }
}
```

**Step 3: Run tests**

Run: `cargo test -p kiro-market-core -- project`
Expected: PASS.

**Step 4: Commit**

```bash
git add crates/kiro-market-core/src/project.rs crates/kiro-market-core/src/lib.rs
git commit -m "feat: add Kiro project state management for skill installation"
```

---

### Task 8: Plugin Discovery — Find Skills on Disk

Given a plugin directory, parse `plugin.json` and discover all SKILL.md files within the declared skill paths.

**Files:**
- Modify: `crates/kiro-market-core/src/plugin.rs`

**Step 1: Write failing tests using tempfile**

Add to `plugin.rs`:

```rust
#[cfg(test)]
mod discovery_tests {
    use super::*;
    use tempfile::TempDir;
    use std::fs;

    #[test]
    fn discover_skills_from_directory_path() {
        let tmp = TempDir::new().unwrap();
        let skills_dir = tmp.path().join("skills");
        fs::create_dir_all(skills_dir.join("tunit")).unwrap();
        fs::write(
            skills_dir.join("tunit/SKILL.md"),
            "---\nname: tunit\ndescription: TUnit tests\n---\n# TUnit",
        ).unwrap();
        fs::create_dir_all(skills_dir.join("efcore")).unwrap();
        fs::write(
            skills_dir.join("efcore/SKILL.md"),
            "---\nname: efcore\ndescription: EF Core patterns\n---\n# EF Core",
        ).unwrap();

        let paths = discover_skill_dirs(tmp.path(), &["./skills/"]);
        assert_eq!(paths.len(), 2);
    }

    #[test]
    fn discover_skills_from_explicit_paths() {
        let tmp = TempDir::new().unwrap();
        let skills_dir = tmp.path().join("skills");
        fs::create_dir_all(skills_dir.join("tunit")).unwrap();
        fs::write(
            skills_dir.join("tunit/SKILL.md"),
            "---\nname: tunit\ndescription: desc\n---\n# T",
        ).unwrap();
        fs::create_dir_all(skills_dir.join("efcore")).unwrap();
        fs::write(
            skills_dir.join("efcore/SKILL.md"),
            "---\nname: efcore\ndescription: desc\n---\n# E",
        ).unwrap();

        // Only discover tunit, not efcore
        let paths = discover_skill_dirs(tmp.path(), &["./skills/tunit"]);
        assert_eq!(paths.len(), 1);
        assert!(paths[0].ends_with("tunit"));
    }
}
```

**Step 2: Implement discovery**

Add to `plugin.rs`:

```rust
use std::path::{Path, PathBuf};

/// Discover skill directories within a plugin based on declared skill paths.
///
/// Each skill path can be:
/// - A directory ending in `/` — scan all subdirectories containing SKILL.md
/// - A specific directory — use directly if it contains SKILL.md
pub fn discover_skill_dirs(plugin_root: &Path, skill_paths: &[&str]) -> Vec<PathBuf> {
    let mut results = Vec::new();

    for raw_path in skill_paths {
        let clean = raw_path.strip_prefix("./").unwrap_or(raw_path);
        let full = plugin_root.join(clean);

        if raw_path.ends_with('/') {
            // Directory — scan subdirectories for SKILL.md
            if let Ok(entries) = std::fs::read_dir(&full) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_dir() && path.join("SKILL.md").exists() {
                        results.push(path);
                    }
                }
            }
        } else if full.is_dir() && full.join("SKILL.md").exists() {
            results.push(full);
        }
    }

    results.sort();
    results
}
```

**Step 3: Run tests**

Run: `cargo test -p kiro-market-core -- discovery`
Expected: PASS.

**Step 4: Commit**

```bash
git add crates/kiro-market-core/src/plugin.rs
git commit -m "feat: add skill directory discovery from plugin manifest paths"
```

---

### Task 9: CLI Wiring — Clap Commands

Set up the clap command structure and wire it to placeholder handlers. This gives us the CLI skeleton to fill in with real logic.

**Files:**
- Create: `crates/kiro-market/src/cli.rs`
- Create: `crates/kiro-market/src/commands/mod.rs`
- Create: `crates/kiro-market/src/commands/marketplace.rs`
- Create: `crates/kiro-market/src/commands/install.rs`
- Create: `crates/kiro-market/src/commands/search.rs`
- Create: `crates/kiro-market/src/commands/list.rs`
- Create: `crates/kiro-market/src/commands/remove.rs`
- Create: `crates/kiro-market/src/commands/info.rs`
- Modify: `crates/kiro-market/src/main.rs`

**Step 1: Create CLI definitions**

`crates/kiro-market/src/cli.rs`:

```rust
//! CLI argument definitions using clap derive.

use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(
    name = "kiro-market",
    about = "Install Claude Code marketplace skills into Kiro CLI projects",
    version
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,

    /// Increase logging verbosity
    #[arg(short, long, action = clap::ArgAction::Count)]
    pub verbose: u8,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Manage marketplace registrations
    Marketplace {
        #[command(subcommand)]
        action: MarketplaceAction,
    },
    /// Search for skills across all marketplaces
    Search {
        /// Search query (matches skill names and descriptions)
        query: String,
    },
    /// Install skills from a plugin
    Install {
        /// Plugin to install (format: plugin@marketplace)
        plugin_ref: String,
        /// Install only a specific skill from the plugin
        #[arg(long)]
        skill: Option<String>,
        /// Overwrite existing skills
        #[arg(long)]
        force: bool,
    },
    /// List installed skills in the current project
    List,
    /// Update installed skills
    Update {
        /// Specific plugin to update (format: plugin@marketplace). Updates all if omitted.
        plugin_ref: Option<String>,
    },
    /// Remove an installed skill
    Remove {
        /// Name of the skill to remove
        skill_name: String,
    },
    /// Show details about a plugin
    Info {
        /// Plugin to inspect (format: plugin@marketplace)
        plugin_ref: String,
    },
}

#[derive(Subcommand, Debug)]
pub enum MarketplaceAction {
    /// Register a new marketplace
    Add {
        /// Marketplace source (GitHub owner/repo, git URL, or local path)
        source: String,
    },
    /// List registered marketplaces
    List,
    /// Update marketplace catalogs
    Update {
        /// Specific marketplace to update. Updates all if omitted.
        name: Option<String>,
    },
    /// Unregister a marketplace
    Remove {
        /// Name of the marketplace to remove
        name: String,
    },
}

/// Parse a "plugin@marketplace" reference.
pub fn parse_plugin_ref(plugin_ref: &str) -> Option<(&str, &str)> {
    plugin_ref.split_once('@')
}
```

**Step 2: Create command stubs**

`crates/kiro-market/src/commands/mod.rs`:
```rust
pub mod info;
pub mod install;
pub mod list;
pub mod marketplace;
pub mod remove;
pub mod search;
```

Each command file (e.g., `marketplace.rs`) starts as a stub:

```rust
use anyhow::Result;
use crate::cli::MarketplaceAction;

pub fn run(action: &MarketplaceAction) -> Result<()> {
    match action {
        MarketplaceAction::Add { source } => {
            println!("Adding marketplace: {source}");
            Ok(())
        }
        MarketplaceAction::List => {
            println!("Listing marketplaces");
            Ok(())
        }
        MarketplaceAction::Update { name } => {
            println!("Updating: {}", name.as_deref().unwrap_or("all"));
            Ok(())
        }
        MarketplaceAction::Remove { name } => {
            println!("Removing marketplace: {name}");
            Ok(())
        }
    }
}
```

Repeat similar stubs for `install.rs`, `search.rs`, `list.rs`, `remove.rs`, `info.rs`.

**Step 3: Wire main.rs**

```rust
//! kiro-market CLI binary.

mod cli;
mod commands;

use anyhow::Result;
use clap::Parser;
use cli::{Cli, Command};
use tracing_subscriber::EnvFilter;

fn main() -> Result<()> {
    let cli = Cli::parse();

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("kiro_market=info")),
        )
        .with_target(false)
        .init();

    match &cli.command {
        Command::Marketplace { action } => commands::marketplace::run(action),
        Command::Search { query } => commands::search::run(query),
        Command::Install { plugin_ref, skill, force } => {
            commands::install::run(plugin_ref, skill.as_deref(), *force)
        }
        Command::List => commands::list::run(),
        Command::Update { plugin_ref } => commands::install::run_update(plugin_ref.as_deref()),
        Command::Remove { skill_name } => commands::remove::run(skill_name),
        Command::Info { plugin_ref } => commands::info::run(plugin_ref),
    }
}
```

**Step 4: Build and test help output**

Run: `cargo build -p kiro-market && cargo run -p kiro-market -- --help`
Expected: Shows help with all subcommands.

Run: `cargo run -p kiro-market -- marketplace --help`
Expected: Shows marketplace subcommands.

**Step 5: Commit**

```bash
git add crates/kiro-market/src/
git commit -m "feat: add CLI skeleton with clap subcommands"
```

---

### Task 10: Implement Marketplace Commands

Wire the `marketplace add/list/update/remove` commands to real logic using the cache and git modules.

**Files:**
- Modify: `crates/kiro-market/src/commands/marketplace.rs`

**Step 1: Implement marketplace add**

This is the most complex command. It needs to:
1. Parse the source (detect if GitHub shorthand, git URL, or local path)
2. Clone the marketplace repo
3. Parse `marketplace.json` to get the marketplace name
4. Register in `known_marketplaces.json`

```rust
use anyhow::{Context, Result};
use colored::Colorize;
use kiro_market_core::{
    cache::{CacheDir, KnownMarketplace, MarketplaceSource},
    git,
    marketplace::Marketplace,
};
use crate::cli::MarketplaceAction;

pub fn run(action: &MarketplaceAction) -> Result<()> {
    match action {
        MarketplaceAction::Add { source } => add(source),
        MarketplaceAction::List => list(),
        MarketplaceAction::Update { name } => update(name.as_deref()),
        MarketplaceAction::Remove { name } => remove(name),
    }
}

fn add(source: &str) -> Result<()> {
    let cache = CacheDir::default_location()
        .context("Could not determine data directory")?;
    cache.ensure_dirs()?;

    let marketplace_source = parse_source(source);

    // Clone or link the marketplace
    let temp_name = source.replace('/', "-");
    let clone_path = cache.marketplaces_dir().join(&temp_name);

    match &marketplace_source {
        MarketplaceSource::GitHub { repo } => {
            let url = git::github_repo_to_url(repo);
            println!("{} marketplace from {}...", "Cloning".green().bold(), repo);
            git::clone_repo(&url, &clone_path, None)?;
        }
        MarketplaceSource::GitUrl { url } => {
            println!("{} marketplace from {}...", "Cloning".green().bold(), url);
            git::clone_repo(url, &clone_path, None)?;
        }
        MarketplaceSource::LocalPath { path } => {
            // For local paths, create a symlink instead of cloning
            let src = std::path::Path::new(path).canonicalize()
                .context("Local marketplace path does not exist")?;
            std::os::unix::fs::symlink(&src, &clone_path)
                .context("Failed to symlink local marketplace")?;
        }
    }

    // Parse marketplace.json to get the real name
    let manifest_path = clone_path.join(".claude-plugin/marketplace.json");
    let manifest_data = std::fs::read(&manifest_path)
        .context("marketplace.json not found — is this a Claude Code marketplace?")?;
    let marketplace = Marketplace::from_json(&manifest_data)
        .context("Failed to parse marketplace.json")?;

    let real_name = marketplace.name.clone();

    // Rename clone dir to the real name if different
    let final_path = cache.marketplace_path(&real_name);
    if clone_path != final_path {
        if final_path.exists() {
            std::fs::remove_dir_all(&final_path)?;
        }
        std::fs::rename(&clone_path, &final_path)?;
    }

    // Register in known_marketplaces.json
    let entry = KnownMarketplace {
        name: real_name.clone(),
        source: marketplace_source,
        added_at: chrono::Utc::now(),
    };
    cache.add_known_marketplace(&entry)?;

    println!(
        "{} marketplace '{}' ({} plugins)",
        "Added".green().bold(),
        real_name,
        marketplace.plugins.len()
    );

    Ok(())
}

fn list() -> Result<()> {
    let cache = CacheDir::default_location()
        .context("Could not determine data directory")?;
    let entries = cache.load_known_marketplaces()?;

    if entries.is_empty() {
        println!("No marketplaces registered. Use 'kiro-market marketplace add' to register one.");
        return Ok(());
    }

    for entry in &entries {
        let source_str = match &entry.source {
            MarketplaceSource::GitHub { repo } => format!("github:{repo}"),
            MarketplaceSource::GitUrl { url } => url.clone(),
            MarketplaceSource::LocalPath { path } => format!("local:{path}"),
        };
        println!("  {} ({})", entry.name.bold(), source_str);
    }

    Ok(())
}

fn update(name: Option<&str>) -> Result<()> {
    let cache = CacheDir::default_location()
        .context("Could not determine data directory")?;
    let entries = cache.load_known_marketplaces()?;

    let to_update: Vec<&KnownMarketplace> = match name {
        Some(n) => entries.iter().filter(|e| e.name == n).collect(),
        None => entries.iter().collect(),
    };

    if to_update.is_empty() {
        println!("No marketplaces to update.");
        return Ok(());
    }

    for entry in to_update {
        let path = cache.marketplace_path(&entry.name);
        if path.is_symlink() {
            println!("  {} {} (local, skip)", "✓".green(), entry.name);
            continue;
        }
        print!("  Updating {}...", entry.name);
        match git::pull_repo(&path) {
            Ok(()) => println!(" {}", "✓".green()),
            Err(e) => println!(" {} ({})", "✗".red(), e),
        }
    }

    Ok(())
}

fn remove(name: &str) -> Result<()> {
    let cache = CacheDir::default_location()
        .context("Could not determine data directory")?;

    cache.remove_known_marketplace(name)?;

    // Clean up cloned data
    let path = cache.marketplace_path(name);
    if path.exists() || path.is_symlink() {
        if path.is_symlink() {
            std::fs::remove_file(&path)?;
        } else {
            std::fs::remove_dir_all(&path)?;
        }
    }

    println!("{} marketplace '{name}'", "Removed".green().bold());
    Ok(())
}

/// Detect the marketplace source type from user input.
fn parse_source(source: &str) -> MarketplaceSource {
    if source.starts_with("http://")
        || source.starts_with("https://")
        || source.starts_with("git@")
    {
        MarketplaceSource::GitUrl {
            url: source.to_string(),
        }
    } else if source.starts_with('/')
        || source.starts_with("./")
        || source.starts_with("../")
        || source.starts_with('~')
    {
        MarketplaceSource::LocalPath {
            path: source.to_string(),
        }
    } else {
        // Assume GitHub owner/repo shorthand
        MarketplaceSource::GitHub {
            repo: source.to_string(),
        }
    }
}
```

**Step 2: Build and test manually**

Run: `cargo build -p kiro-market`
Expected: Compiles.

Run: `cargo run -p kiro-market -- marketplace add ../skills`
Expected: Clones/links and registers the local marketplace.

Run: `cargo run -p kiro-market -- marketplace list`
Expected: Shows the registered marketplace.

**Step 3: Commit**

```bash
git add crates/kiro-market/src/commands/marketplace.rs
git commit -m "feat: implement marketplace add/list/update/remove commands"
```

---

### Task 11: Implement Install Command

The core feature — install skills from a plugin into the current project.

**Files:**
- Modify: `crates/kiro-market/src/commands/install.rs`

**Step 1: Implement install logic**

```rust
use anyhow::{bail, Context, Result};
use colored::Colorize;
use kiro_market_core::{
    cache::CacheDir,
    marketplace::Marketplace,
    plugin::{discover_skill_dirs, PluginManifest},
    project::{InstalledSkillMeta, KiroProject},
    skill::{extract_relative_md_links, merge_skill, parse_frontmatter},
};
use crate::cli::parse_plugin_ref;
use std::fs;

pub fn run(plugin_ref: &str, skill_filter: Option<&str>, force: bool) -> Result<()> {
    let (plugin_name, marketplace_name) = parse_plugin_ref(plugin_ref)
        .context("Invalid format. Use: plugin@marketplace")?;

    let cache = CacheDir::default_location()
        .context("Could not determine data directory")?;

    // Find the marketplace
    let market_path = cache.marketplace_path(marketplace_name);
    if !market_path.exists() {
        bail!(
            "Marketplace '{marketplace_name}' not found. Run: kiro-market marketplace add <source>"
        );
    }

    // Parse marketplace.json to find the plugin
    let manifest_data = fs::read(market_path.join(".claude-plugin/marketplace.json"))?;
    let marketplace = Marketplace::from_json(&manifest_data)?;

    let plugin_entry = marketplace
        .plugins
        .iter()
        .find(|p| p.name == plugin_name)
        .with_context(|| {
            format!("Plugin '{plugin_name}' not found in marketplace '{marketplace_name}'")
        })?;

    // Resolve the plugin directory
    let plugin_dir = resolve_plugin_dir(&cache, marketplace_name, plugin_entry, &market_path)?;

    // Parse plugin.json
    let plugin_json_path = plugin_dir.join("plugin.json");
    let plugin_manifest = if plugin_json_path.exists() {
        let data = fs::read(&plugin_json_path)?;
        PluginManifest::from_json(&data)?
    } else {
        // Fallback: assume skills/ directory
        PluginManifest {
            name: plugin_name.to_string(),
            version: plugin_entry.version.clone(),
            description: plugin_entry.description.clone(),
            skills: vec!["./skills/".to_string()],
        }
    };

    // Discover skill directories
    let skill_paths: Vec<&str> = plugin_manifest.skills.iter().map(String::as_str).collect();
    let skill_dirs = discover_skill_dirs(&plugin_dir, &skill_paths);

    if skill_dirs.is_empty() {
        bail!("No skills found in plugin '{plugin_name}'");
    }

    let project = KiroProject::new(std::env::current_dir()?);
    let mut installed_count = 0;

    for skill_dir in &skill_dirs {
        let skill_md_path = skill_dir.join("SKILL.md");
        if !skill_md_path.exists() {
            // Also check lowercase
            let alt = skill_dir.join("skill.md");
            if !alt.exists() {
                continue;
            }
        }

        let skill_content = fs::read_to_string(&skill_md_path)
            .or_else(|_| fs::read_to_string(skill_dir.join("skill.md")))?;

        let (frontmatter, _) = parse_frontmatter(&skill_content)
            .context(format!("Failed to parse {}", skill_md_path.display()))?;

        // Apply skill filter if specified
        if let Some(filter) = skill_filter {
            if frontmatter.name != filter {
                continue;
            }
        }

        // Find and merge companion files
        let body_links = extract_relative_md_links(&skill_content);
        let mut companions = Vec::new();
        for link in &body_links {
            let companion_path = skill_dir.join(link);
            if companion_path.exists() {
                let content = fs::read_to_string(&companion_path)?;
                companions.push((link.as_str(), content));
            }
        }

        let companion_refs: Vec<(&str, &str)> = companions
            .iter()
            .map(|(path, content)| (path.as_str(), content.as_str()))
            .collect();

        let merged = merge_skill(&skill_content, &companion_refs)
            .context(format!("Failed to merge skill '{}'", frontmatter.name))?;

        let meta = InstalledSkillMeta {
            marketplace: marketplace_name.to_string(),
            plugin: plugin_name.to_string(),
            version: plugin_manifest.version.clone(),
            installed_at: chrono::Utc::now(),
        };

        let install_result = if force {
            project.install_skill_force(&frontmatter.name, &merged, &meta)
        } else {
            project.install_skill(&frontmatter.name, &merged, &meta)
        };

        match install_result {
            Ok(()) => {
                let companion_note = if !companion_refs.is_empty() {
                    format!(" (merged {} companion files)", companion_refs.len())
                } else {
                    String::new()
                };
                println!(
                    "  {} {}{}",
                    "✓".green(),
                    frontmatter.name,
                    companion_note
                );
                installed_count += 1;
            }
            Err(e) => {
                println!("  {} {} — {}", "✗".red(), frontmatter.name, e);
            }
        }
    }

    if installed_count > 0 {
        println!(
            "\n{} {} skill(s) from {}",
            "Installed".green().bold(),
            installed_count,
            plugin_ref
        );
    } else if skill_filter.is_some() {
        bail!("Skill '{}' not found in plugin '{plugin_name}'", skill_filter.unwrap());
    }

    Ok(())
}

pub fn run_update(plugin_ref: Option<&str>) -> Result<()> {
    // For now, update = remove + reinstall
    println!("Update is not yet implemented. Use 'remove' + 'install --force' for now.");
    Ok(())
}

fn resolve_plugin_dir(
    cache: &CacheDir,
    marketplace_name: &str,
    plugin_entry: &kiro_market_core::marketplace::PluginEntry,
    market_path: &std::path::Path,
) -> Result<std::path::PathBuf> {
    use kiro_market_core::marketplace::PluginSource;

    match &plugin_entry.source {
        PluginSource::RelativePath(rel) => {
            let clean = rel.strip_prefix("./").unwrap_or(rel);
            let path = market_path.join(clean);
            if !path.exists() {
                bail!(
                    "Plugin directory '{}' not found in marketplace",
                    path.display()
                );
            }
            Ok(path)
        }
        PluginSource::Structured(structured) => {
            use kiro_market_core::marketplace::StructuredSource;
            let plugin_cache = cache.plugin_path(marketplace_name, &plugin_entry.name);
            if plugin_cache.exists() {
                return Ok(plugin_cache);
            }

            match structured {
                StructuredSource::GitHub { repo, git_ref, .. } => {
                    let url = kiro_market_core::git::github_repo_to_url(repo);
                    println!("  Fetching plugin from {}...", repo);
                    kiro_market_core::git::clone_repo(
                        &url,
                        &plugin_cache,
                        git_ref.as_deref(),
                    )?;
                }
                StructuredSource::GitUrl { url, git_ref, .. } => {
                    println!("  Fetching plugin from {}...", url);
                    kiro_market_core::git::clone_repo(
                        url,
                        &plugin_cache,
                        git_ref.as_deref(),
                    )?;
                }
                StructuredSource::GitSubdir { url, path, git_ref, .. } => {
                    // Clone full repo then point at subdirectory
                    let repo_cache = cache.plugin_path(marketplace_name, &format!("{}-repo", plugin_entry.name));
                    if !repo_cache.exists() {
                        println!("  Fetching plugin from {}...", url);
                        kiro_market_core::git::clone_repo(
                            url,
                            &repo_cache,
                            git_ref.as_deref(),
                        )?;
                    }
                    return Ok(repo_cache.join(path));
                }
            }

            Ok(plugin_cache)
        }
    }
}
```

**Step 2: Build and manually test with the local Microsoft marketplace**

Run: `cargo build -p kiro-market`

Run: `cargo run -p kiro-market -- install dotnet@dotnet-agent-skills --skill csharp-scripts`
Expected: Installs csharp-scripts to `.kiro/skills/csharp-scripts/SKILL.md`.

**Step 3: Commit**

```bash
git add crates/kiro-market/src/commands/install.rs
git commit -m "feat: implement install command with skill merging"
```

---

### Task 12: Implement Search, List, Remove, Info Commands

Fill in the remaining CLI command stubs.

**Files:**
- Modify: `crates/kiro-market/src/commands/search.rs`
- Modify: `crates/kiro-market/src/commands/list.rs`
- Modify: `crates/kiro-market/src/commands/remove.rs`
- Modify: `crates/kiro-market/src/commands/info.rs`

**Step 1: Implement search**

`search.rs` — iterates all marketplaces, loads each marketplace.json, loads each plugin.json, scans SKILL.md frontmatter for matches:

```rust
use anyhow::{Context, Result};
use colored::Colorize;
use kiro_market_core::{
    cache::CacheDir,
    marketplace::Marketplace,
    plugin::{discover_skill_dirs, PluginManifest},
    skill::parse_frontmatter,
};
use std::fs;

pub fn run(query: &str) -> Result<()> {
    let cache = CacheDir::default_location()
        .context("Could not determine data directory")?;
    let entries = cache.load_known_marketplaces()?;

    if entries.is_empty() {
        println!("No marketplaces registered. Use 'kiro-market marketplace add' to register one.");
        return Ok(());
    }

    let query_lower = query.to_lowercase();
    let mut found = 0;

    for entry in &entries {
        let market_path = cache.marketplace_path(&entry.name);
        let manifest_path = market_path.join(".claude-plugin/marketplace.json");
        let Ok(data) = fs::read(&manifest_path) else { continue };
        let Ok(marketplace) = Marketplace::from_json(&data) else { continue };

        for plugin in &marketplace.plugins {
            let plugin_dir = match &plugin.source {
                kiro_market_core::marketplace::PluginSource::RelativePath(rel) => {
                    let clean = rel.strip_prefix("./").unwrap_or(rel);
                    market_path.join(clean)
                }
                _ => continue, // skip external sources for search (not cloned yet)
            };

            let plugin_json_path = plugin_dir.join("plugin.json");
            let skill_paths = if plugin_json_path.exists() {
                let data = fs::read(&plugin_json_path).unwrap_or_default();
                PluginManifest::from_json(&data)
                    .map(|m| m.skills)
                    .unwrap_or_default()
            } else {
                vec!["./skills/".to_string()]
            };

            let path_refs: Vec<&str> = skill_paths.iter().map(String::as_str).collect();
            let skill_dirs = discover_skill_dirs(&plugin_dir, &path_refs);

            for skill_dir in skill_dirs {
                let skill_md = skill_dir.join("SKILL.md");
                let Ok(content) = fs::read_to_string(&skill_md) else { continue };
                let Ok((fm, _)) = parse_frontmatter(&content) else { continue };

                if fm.name.to_lowercase().contains(&query_lower)
                    || fm.description.to_lowercase().contains(&query_lower)
                {
                    if found == 0 {
                        println!();
                    }
                    println!(
                        "  {} ({}@{})",
                        fm.name.bold(),
                        plugin.name,
                        entry.name
                    );
                    println!("    {}", fm.description);
                    println!();
                    found += 1;
                }
            }
        }
    }

    if found == 0 {
        println!("No skills matching '{query}' found.");
    } else {
        println!("Found {found} skill(s). Install with: kiro-market install <plugin>@<marketplace>");
    }

    Ok(())
}
```

**Step 2: Implement list**

`list.rs`:

```rust
use anyhow::{Context, Result};
use colored::Colorize;
use kiro_market_core::project::KiroProject;

pub fn run() -> Result<()> {
    let project = KiroProject::new(std::env::current_dir()?);
    let installed = project.load_installed()?;

    if installed.skills.is_empty() {
        println!("No skills installed in this project.");
        return Ok(());
    }

    println!("{} installed skill(s):\n", installed.skills.len());
    for (name, meta) in &installed.skills {
        let version = meta.version.as_deref().unwrap_or("unknown");
        println!(
            "  {} ({}@{} v{})",
            name.bold(),
            meta.plugin,
            meta.marketplace,
            version
        );
    }

    Ok(())
}
```

**Step 3: Implement remove**

`remove.rs`:

```rust
use anyhow::{Context, Result};
use colored::Colorize;
use kiro_market_core::project::KiroProject;

pub fn run(skill_name: &str) -> Result<()> {
    let project = KiroProject::new(std::env::current_dir()?);
    project.remove_skill(skill_name)?;
    println!("{} skill '{skill_name}'", "Removed".green().bold());
    Ok(())
}
```

**Step 4: Implement info**

`info.rs`:

```rust
use anyhow::{bail, Context, Result};
use colored::Colorize;
use kiro_market_core::{
    cache::CacheDir,
    marketplace::Marketplace,
    plugin::{discover_skill_dirs, PluginManifest},
    skill::parse_frontmatter,
};
use crate::cli::parse_plugin_ref;
use std::fs;

pub fn run(plugin_ref: &str) -> Result<()> {
    let (plugin_name, marketplace_name) = parse_plugin_ref(plugin_ref)
        .context("Invalid format. Use: plugin@marketplace")?;

    let cache = CacheDir::default_location()
        .context("Could not determine data directory")?;

    let market_path = cache.marketplace_path(marketplace_name);
    if !market_path.exists() {
        bail!("Marketplace '{marketplace_name}' not found.");
    }

    let manifest_data = fs::read(market_path.join(".claude-plugin/marketplace.json"))?;
    let marketplace = Marketplace::from_json(&manifest_data)?;

    let plugin_entry = marketplace
        .plugins
        .iter()
        .find(|p| p.name == plugin_name)
        .with_context(|| format!("Plugin '{plugin_name}' not found"))?;

    println!("\n{} {}", "Plugin:".bold(), plugin_name);
    if let Some(desc) = &plugin_entry.description {
        println!("{} {}", "Description:".bold(), desc);
    }
    if let Some(ver) = &plugin_entry.version {
        println!("{} {}", "Version:".bold(), ver);
    }
    println!("{} {}", "Marketplace:".bold(), marketplace_name);

    // Try to list skills
    let plugin_dir = match &plugin_entry.source {
        kiro_market_core::marketplace::PluginSource::RelativePath(rel) => {
            let clean = rel.strip_prefix("./").unwrap_or(rel);
            market_path.join(clean)
        }
        _ => {
            println!("\nSkills: (external source — install to discover)");
            return Ok(());
        }
    };

    let plugin_json_path = plugin_dir.join("plugin.json");
    let skill_paths = if plugin_json_path.exists() {
        let data = fs::read(&plugin_json_path)?;
        PluginManifest::from_json(&data)?.skills
    } else {
        vec!["./skills/".to_string()]
    };

    let path_refs: Vec<&str> = skill_paths.iter().map(String::as_str).collect();
    let skill_dirs = discover_skill_dirs(&plugin_dir, &path_refs);

    println!("\n{} ({} found)", "Skills:".bold(), skill_dirs.len());
    for dir in &skill_dirs {
        let skill_md = dir.join("SKILL.md");
        if let Ok(content) = fs::read_to_string(&skill_md) {
            if let Ok((fm, _)) = parse_frontmatter(&content) {
                println!("  {} — {}", fm.name.green(), fm.description);
            }
        }
    }

    Ok(())
}
```

**Step 5: Build and test all commands**

Run: `cargo build -p kiro-market`
Expected: Compiles.

**Step 6: Commit**

```bash
git add crates/kiro-market/src/commands/
git commit -m "feat: implement search, list, remove, and info commands"
```

---

### Task 13: Integration Tests

Write integration tests that exercise the full CLI using the binary, following the rivets pattern with `CARGO_BIN_EXE_*`.

**Files:**
- Create: `crates/kiro-market/tests/common/mod.rs`
- Create: `crates/kiro-market/tests/marketplace_tests.rs`
- Create: `crates/kiro-market/tests/install_tests.rs`

**Step 1: Create test helpers**

`crates/kiro-market/tests/common/mod.rs`:

```rust
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

pub fn get_binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_kiro-market"))
}

pub fn run_in_dir(dir: &Path, args: &[&str]) -> Output {
    Command::new(get_binary())
        .args(args)
        .current_dir(dir)
        .env("XDG_DATA_HOME", dir.join(".data"))
        .output()
        .expect("Failed to execute kiro-market")
}

pub fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).to_string()
}

pub fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).to_string()
}
```

**Step 2: Write marketplace integration tests**

`crates/kiro-market/tests/marketplace_tests.rs`:

```rust
mod common;

use common::{run_in_dir, stdout};
use tempfile::TempDir;

#[test]
fn marketplace_list_empty() {
    let tmp = TempDir::new().unwrap();
    let output = run_in_dir(tmp.path(), &["marketplace", "list"]);
    assert!(output.status.success());
    assert!(stdout(&output).contains("No marketplaces registered"));
}
```

**Step 3: Run integration tests**

Run: `cargo test -p kiro-market -- --test marketplace_tests`
Expected: PASS.

**Step 4: Commit**

```bash
git add crates/kiro-market/tests/
git commit -m "test: add integration test scaffolding and marketplace list test"
```

---

### Task 14: Final Polish — README and CLAUDE.md

**Files:**
- Create: `CLAUDE.md` (project-level instructions for AI assistants)

**Step 1: Create CLAUDE.md**

```markdown
# kiro-market — Developer Guide

## Build
```bash
cargo build
```

## Test
```bash
cargo test                          # all tests
cargo test -p kiro-market-core      # core library tests
cargo test -p kiro-market           # CLI + integration tests
```

## Project Structure
- `crates/kiro-market-core/` — library crate (types, parsing, git, cache, project)
- `crates/kiro-market/` — binary crate (CLI, commands)

## Code Style
- Edition 2024, rust-version 1.85.0
- `thiserror` for typed errors in kiro-market-core
- `anyhow` for error propagation in kiro-market binary
- `rstest` for parameterized tests, `tempfile` for test fixtures
- `clippy::all` and `clippy::pedantic` enabled as warnings
```

**Step 2: Commit**

```bash
git add CLAUDE.md
git commit -m "docs: add CLAUDE.md with build and test instructions"
```

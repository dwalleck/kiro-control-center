# Agent Import Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Import Claude-style and Copilot-style markdown agents from plugins into Kiro projects as JSON config files plus externalized prompt files, with best-effort tool-name mapping.

**Architecture:** A new `agent` module in `kiro-market-core` parses two source dialects (Claude `.md` and Copilot `.agent.md`) into a shared `AgentDefinition`, then emits a Kiro-compatible `{name}.json` next to a `prompts/{name}.md` file. The JSON's `prompt` field uses Kiro's `file://` URI scheme (`file://./prompts/{name}.md`) so the `.md` remains the live source of truth. Installs are tracked in a new `installed-agents.json`, parallel to the existing `installed-skills.json`. `MarketplaceService::add()` discovers both skills and agents during install and surfaces warnings (dropped tools, unmapped models) in a structured `InstallWarning` list that the CLI renders.

**Tech Stack:** Rust edition 2024, `serde` / `serde_json` / `serde_yaml`, `thiserror`, `tracing`, `rstest`, `tempfile`

**Non-goals (deferred):**
- Support for awesome-copilot repos that are not Claude Code plugins (no `plugin.json`).
- Interactive tool-mapping prompts at install time.
- Converting agent `hooks` frontmatter (Claude agents rarely declare these).

---

### Task 1: Add `AgentError` variants and `Error::Agent` wrapper

**Files:**
- Modify: `crates/kiro-market-core/src/error.rs` (add `AgentError` enum between `SkillError` and `GitError`; add `Agent(#[from] AgentError)` variant to `Error`; extend the display rstest)

**Step 1: Write the failing tests**

Add to the `mod tests` in `error.rs`:

```rust
#[rstest]
#[case::agent_already_installed(
    AgentError::AlreadyInstalled { name: "reviewer".into() },
    "agent `reviewer` is already installed"
)]
#[case::agent_not_installed(
    AgentError::NotInstalled { name: "missing".into() },
    "agent `missing` is not installed"
)]
#[case::agent_parse_failed(
    AgentError::ParseFailed { path: PathBuf::from("a.md"), reason: "bad yaml".into() },
    "failed to parse agent at a.md: bad yaml"
)]
#[case::agent_missing_name(
    AgentError::MissingName { path: PathBuf::from("a.md") },
    "agent at a.md is missing required `name` field"
)]
fn agent_error_display(#[case] err: AgentError, #[case] expected: &str) {
    assert_eq!(err.to_string(), expected);
}

#[test]
fn from_agent_error() {
    let inner = AgentError::NotInstalled { name: "x".into() };
    let err: Error = inner.into();
    assert!(matches!(err, Error::Agent(_)));
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test -p kiro-market-core -- agent_error`
Expected: compile error — `AgentError` does not exist.

**Step 3: Add `AgentError` and `Error::Agent`**

Insert between `SkillError` and `GitError` sections:

```rust
// ---------------------------------------------------------------------------
// Agent errors
// ---------------------------------------------------------------------------

/// Errors related to agent operations.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum AgentError {
    /// The agent is already installed in the target project.
    #[error("agent `{name}` is already installed")]
    AlreadyInstalled { name: String },

    /// The agent is not installed in the target project.
    #[error("agent `{name}` is not installed")]
    NotInstalled { name: String },

    /// The source markdown could not be parsed.
    #[error("failed to parse agent at {path}: {reason}")]
    ParseFailed { path: PathBuf, reason: String },

    /// Frontmatter lacks a required `name` field.
    #[error("agent at {path} is missing required `name` field")]
    MissingName { path: PathBuf },
}
```

Add the `From` variant to `Error`:

```rust
    #[error(transparent)]
    Agent(#[from] AgentError),
```

**Step 4: Run tests to verify they pass**

Run: `cargo test -p kiro-market-core -- agent_error`
Expected: PASS (4 cases).

**Step 5: Commit**

```bash
git add crates/kiro-market-core/src/error.rs
git commit -m "feat: add AgentError variants for agent install domain"
```

---

### Task 2: Create `AgentDefinition` common type

**Files:**
- Create: `crates/kiro-market-core/src/agent/mod.rs`
- Create: `crates/kiro-market-core/src/agent/types.rs`
- Modify: `crates/kiro-market-core/src/lib.rs` (add `pub mod agent;`)

**Step 1: Write the failing test**

Create `crates/kiro-market-core/src/agent/types.rs`:

```rust
//! Dialect-agnostic representation of an agent after parsing.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Which source dialect the agent came from. Used for applying
/// dialect-specific tool-mapping rules and for warnings.
///
/// Serializes to `"claude"` / `"copilot"` so it can live directly in the
/// installed-agents tracking file without a string sidecar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentDialect {
    Claude,
    Copilot,
}

/// Agent definition normalized across Claude and Copilot source formats.
///
/// This is what both parsers produce and what the emitter consumes.
#[derive(Debug, Clone)]
pub struct AgentDefinition {
    pub name: String,
    pub description: Option<String>,
    pub prompt_body: String,
    pub model: Option<String>,
    /// Raw tool identifiers from the source frontmatter (pre-mapping).
    pub source_tools: Vec<String>,
    /// MCP server entries as captured from Copilot `mcp-servers:` frontmatter.
    /// Serialized opaquely and passed through to Kiro's `mcpServers` field.
    pub mcp_servers: BTreeMap<String, serde_json::Value>,
    pub dialect: AgentDialect,
}
```

Add `pub mod types;` and `pub use types::{AgentDefinition, AgentDialect};` to `crates/kiro-market-core/src/agent/mod.rs`.

Add `pub mod agent;` to `crates/kiro-market-core/src/lib.rs` (alphabetically, after `cache`).

Add a smoke test at the bottom of `agent/mod.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn agent_definition_constructs_with_minimum_fields() {
        let def = AgentDefinition {
            name: "reviewer".into(),
            description: None,
            prompt_body: "You are a reviewer.".into(),
            model: None,
            source_tools: vec![],
            mcp_servers: BTreeMap::new(),
            dialect: AgentDialect::Claude,
        };
        assert_eq!(def.name, "reviewer");
    }
}
```

**Step 2: Run test to verify it compiles and passes**

Run: `cargo test -p kiro-market-core --lib agent::`
Expected: PASS.

**Step 3: Commit**

```bash
git add crates/kiro-market-core/src/agent/ crates/kiro-market-core/src/lib.rs
git commit -m "feat: add AgentDefinition common type for agent parsing"
```

---

### Task 3: Parse Claude-style agent frontmatter

**Files:**
- Create: `crates/kiro-market-core/src/agent/frontmatter.rs` (shared `split_frontmatter` helper, dialect-agnostic)
- Create: `crates/kiro-market-core/src/agent/parse_claude.rs`
- Modify: `crates/kiro-market-core/src/agent/mod.rs` (add `mod frontmatter; mod parse_claude; pub use parse_claude::parse_claude_agent;`)

**Why a shared `frontmatter` module:** Both Claude and Copilot parsers need identical YAML-fence splitting. Putting the helper in its own module avoids the layering hack of one parser reaching into the other's `pub(super)` shim (which the original draft of Task 4 introduced).

**Step 1: Write the failing tests**

In `agent/parse_claude.rs` add a `mod tests` section:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "---\nname: code-reviewer\ndescription: Reviews code\nmodel: opus\ncolor: green\n---\n\nYou are a code reviewer.\n";

    #[test]
    fn parse_extracts_name_description_model() {
        let def = parse_claude_agent(SAMPLE).expect("parse");
        assert_eq!(def.name, "code-reviewer");
        assert_eq!(def.description.as_deref(), Some("Reviews code"));
        assert_eq!(def.model.as_deref(), Some("opus"));
    }

    #[test]
    fn parse_extracts_body_trimming_fence_newline() {
        let def = parse_claude_agent(SAMPLE).expect("parse");
        assert!(def.prompt_body.starts_with("You are a code reviewer."));
    }

    #[test]
    fn parse_drops_color_field_silently() {
        // color has no Kiro equivalent and should not appear in source_tools or as model.
        let def = parse_claude_agent(SAMPLE).expect("parse");
        assert!(def.source_tools.is_empty());
    }

    #[test]
    fn parse_model_inherit_becomes_none() {
        let src = "---\nname: a\nmodel: inherit\n---\nbody\n";
        let def = parse_claude_agent(src).expect("parse");
        assert!(def.model.is_none(), "model: inherit should be normalized to None");
    }

    #[test]
    fn parse_missing_name_errors() {
        let src = "---\ndescription: x\n---\nbody\n";
        let err = parse_claude_agent(src).unwrap_err();
        assert!(err.to_string().contains("name"));
    }

    #[test]
    fn parse_tools_frontmatter_captured_in_source_tools() {
        let src = "---\nname: a\ntools: [Read, Write, Bash]\n---\nbody\n";
        let def = parse_claude_agent(src).expect("parse");
        assert_eq!(def.source_tools, vec!["Read", "Write", "Bash"]);
    }

    #[test]
    fn parse_invalid_yaml_errors() {
        let src = "---\nname: [unclosed\n---\nbody\n";
        assert!(parse_claude_agent(src).is_err());
    }
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test -p kiro-market-core --lib agent::parse_claude`
Expected: compile error — `parse_claude_agent` does not exist.

**Step 3: Implement the shared splitter and the Claude parser**

Create `crates/kiro-market-core/src/agent/frontmatter.rs`:

```rust
//! Dialect-agnostic YAML frontmatter splitter.
//!
//! Both Claude (`*.md`) and Copilot (`*.agent.md`) agents use the same
//! `---`-fenced frontmatter convention, so the fence-handling logic lives
//! in one place rather than being duplicated (or worse, reached into via
//! a `pub(super)` shim) between the two parsers.

/// Split `---`-fenced YAML frontmatter from the body. Returns `(yaml, body)`.
///
/// # Errors
///
/// Returns a human-readable error if the opening or closing fence is missing.
pub(super) fn split_frontmatter(content: &str) -> Result<(&str, &str), String> {
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        return Err("missing opening `---` frontmatter fence".into());
    }
    let after_open = trimmed[3..]
        .strip_prefix('\n')
        .or_else(|| trimmed[3..].strip_prefix("\r\n"))
        .unwrap_or(&trimmed[3..]);
    let Some(close_pos) = after_open.find("\n---") else {
        return Err("unclosed frontmatter: missing closing `---` fence".into());
    };
    let yaml = &after_open[..close_pos];
    let body_start = &after_open[close_pos + 4..];
    let body = body_start
        .strip_prefix('\n')
        .or_else(|| body_start.strip_prefix("\r\n"))
        .unwrap_or(body_start);
    Ok((yaml, body))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_simple_frontmatter() {
        let (yaml, body) = split_frontmatter("---\nname: a\n---\nbody\n").unwrap();
        assert_eq!(yaml, "name: a");
        assert_eq!(body, "body\n");
    }

    #[test]
    fn missing_open_fence_errors() {
        assert!(split_frontmatter("body\n").is_err());
    }

    #[test]
    fn missing_close_fence_errors() {
        assert!(split_frontmatter("---\nname: a\nbody\n").is_err());
    }
}
```

Create `crates/kiro-market-core/src/agent/parse_claude.rs`:

```rust
//! Parse Claude-style agent markdown files.
//!
//! Claude agents have YAML frontmatter with: `name` (required),
//! `description`, `model` (`opus`/`sonnet`/`inherit`), `color` (dropped),
//! and optional `tools` (a list of PascalCase tool names).

use serde::Deserialize;
use std::collections::BTreeMap;

use super::frontmatter::split_frontmatter;
use super::types::{AgentDefinition, AgentDialect};

#[derive(Debug, Deserialize)]
struct ClaudeFrontmatter {
    name: Option<String>,
    description: Option<String>,
    model: Option<String>,
    #[serde(default)]
    tools: Vec<String>,
    // `color` is intentionally unmodeled — not in Kiro schema, silently dropped.
}

/// Parse a Claude-style `.md` agent file into an `AgentDefinition`.
///
/// # Errors
///
/// Returns a string describing the parse failure. The caller wraps this
/// into `AgentError::ParseFailed` or `AgentError::MissingName` with the
/// source path attached.
pub fn parse_claude_agent(content: &str) -> Result<AgentDefinition, String> {
    let (yaml_block, body) = split_frontmatter(content)?;
    let fm: ClaudeFrontmatter = serde_yaml::from_str(yaml_block)
        .map_err(|e| format!("invalid YAML: {e}"))?;

    let name = fm.name.ok_or_else(|| "missing `name` field".to_string())?;
    // Normalize `model: inherit` (Claude's "use parent model" sentinel) to None
    // so the Kiro emitter omits the field and defers to the CLI default.
    let model = fm.model.filter(|m| m != "inherit");

    Ok(AgentDefinition {
        name,
        description: fm.description,
        prompt_body: body.to_string(),
        model,
        source_tools: fm.tools,
        mcp_servers: BTreeMap::new(),
        dialect: AgentDialect::Claude,
    })
}
```

Wire up in `agent/mod.rs`:

```rust
mod frontmatter;
mod parse_claude;
pub use parse_claude::parse_claude_agent;
```

**Step 4: Run tests to verify they pass**

Run: `cargo test -p kiro-market-core --lib agent::parse_claude`
Expected: PASS (7 cases).

**Step 5: Commit**

```bash
git add crates/kiro-market-core/src/agent/
git commit -m "feat: parse Claude-style agent markdown frontmatter"
```

---

### Task 4: Parse Copilot-style agent frontmatter

**Files:**
- Create: `crates/kiro-market-core/src/agent/parse_copilot.rs`
- Modify: `crates/kiro-market-core/src/agent/mod.rs` (add `mod parse_copilot; pub use parse_copilot::parse_copilot_agent;`)

**Step 1: Write the failing tests**

In `agent/parse_copilot.rs` add `mod tests`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    const TERRAFORM: &str = r#"---
name: Terraform Agent
description: "Terraform specialist"
tools: ['read', 'edit', 'search', 'shell', 'terraform/*']
mcp-servers:
  terraform:
    type: 'local'
    command: 'docker'
    args: ['run', '-i', 'hashicorp/terraform-mcp-server:latest']
    tools: ["*"]
---

Body text.
"#;

    #[test]
    fn parse_extracts_name_and_body() {
        let def = parse_copilot_agent(TERRAFORM).expect("parse");
        assert_eq!(def.name, "Terraform Agent");
        assert!(def.prompt_body.starts_with("Body text."));
    }

    #[test]
    fn parse_captures_tools_list_verbatim() {
        let def = parse_copilot_agent(TERRAFORM).expect("parse");
        assert_eq!(
            def.source_tools,
            vec!["read", "edit", "search", "shell", "terraform/*"]
        );
    }

    #[test]
    fn parse_captures_mcp_servers_as_opaque_json() {
        let def = parse_copilot_agent(TERRAFORM).expect("parse");
        assert_eq!(def.mcp_servers.len(), 1);
        let tf = def.mcp_servers.get("terraform").expect("terraform entry");
        // `type: 'local'` is preserved opaquely; normalization happens at emit time.
        assert_eq!(tf["type"], "local");
        assert_eq!(tf["command"], "docker");
    }

    #[test]
    fn parse_drops_display_model_name() {
        // Copilot model values are display names, not IDs. Per design decision,
        // we drop them and let Kiro use the default model.
        let src = "---\nname: a\nmodel: Claude Sonnet 4\n---\nbody\n";
        let def = parse_copilot_agent(src).expect("parse");
        assert!(def.model.is_none(), "display-name model should be dropped");
    }

    #[test]
    fn parse_missing_name_errors() {
        let src = "---\ndescription: x\n---\nbody\n";
        assert!(parse_copilot_agent(src).is_err());
    }
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test -p kiro-market-core --lib agent::parse_copilot`
Expected: compile error.

**Step 3: Implement the parser**

Create `crates/kiro-market-core/src/agent/parse_copilot.rs`:

```rust
//! Parse Copilot-style `*.agent.md` files.
//!
//! Copilot frontmatter differs from Claude's in notable ways:
//! - `model` holds display names (`Claude Sonnet 4`), not model IDs.
//!   We drop it — the user can edit the emitted JSON to set a real ID.
//! - `mcp-servers` is a nested map of server configs (kebab-case key).
//!   We capture it opaquely as `serde_json::Value` and translate at emit time.
//! - `tools` uses mixed conventions (bare names, `namespace/tool`,
//!   `server/*` wildcards). We keep raw strings; mapping lives elsewhere.

use serde::Deserialize;
use std::collections::BTreeMap;

use super::frontmatter::split_frontmatter;
use super::types::{AgentDefinition, AgentDialect};

#[derive(Debug, Deserialize)]
struct CopilotFrontmatter {
    name: Option<String>,
    description: Option<String>,
    #[serde(default)]
    tools: Vec<String>,
    #[serde(rename = "mcp-servers", default)]
    mcp_servers: BTreeMap<String, serde_json::Value>,
    // `model` intentionally not modeled — Copilot uses display names (e.g.
    // "Claude Sonnet 4") which cannot be mapped to Kiro model IDs.
}

/// Parse a Copilot-style `.agent.md` file into an `AgentDefinition`.
pub fn parse_copilot_agent(content: &str) -> Result<AgentDefinition, String> {
    let (yaml, body) = split_frontmatter(content)?;
    let fm: CopilotFrontmatter = serde_yaml::from_str(yaml)
        .map_err(|e| format!("invalid YAML: {e}"))?;

    let name = fm.name.ok_or_else(|| "missing `name` field".to_string())?;

    Ok(AgentDefinition {
        name,
        description: fm.description,
        prompt_body: body.to_string(),
        model: None,
        source_tools: fm.tools,
        mcp_servers: fm.mcp_servers,
        dialect: AgentDialect::Copilot,
    })
}
```

Wire up in `agent/mod.rs`:

```rust
mod parse_copilot;
pub use parse_copilot::parse_copilot_agent;
```

**Step 4: Run tests to verify they pass**

Run: `cargo test -p kiro-market-core --lib agent::parse_copilot`
Expected: PASS (5 cases).

**Step 5: Commit**

```bash
git add crates/kiro-market-core/src/agent/
git commit -m "feat: parse Copilot-style agent markdown frontmatter"
```

---

### Task 5: Dialect detection by filename

**Files:**
- Create: `crates/kiro-market-core/src/agent/parse.rs`
- Modify: `crates/kiro-market-core/src/agent/mod.rs` (add `mod parse; pub use parse::parse_agent_file;`)

**Step 1: Write the failing tests**

In `agent/parse.rs` add `mod tests`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn detects_copilot_by_agent_md_suffix() {
        assert_eq!(
            detect_dialect(Path::new("foo.agent.md")),
            AgentDialect::Copilot
        );
        assert_eq!(
            detect_dialect(Path::new("/a/b/c.agent.md")),
            AgentDialect::Copilot
        );
    }

    #[test]
    fn detects_claude_for_plain_md() {
        assert_eq!(
            detect_dialect(Path::new("reviewer.md")),
            AgentDialect::Claude
        );
    }

    #[test]
    fn parse_agent_file_dispatches_by_dialect() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("sample.agent.md");
        std::fs::write(
            &path,
            "---\nname: sample\n---\nbody\n",
        ).unwrap();
        let def = parse_agent_file(&path).expect("parse");
        assert_eq!(def.dialect, AgentDialect::Copilot);
        assert_eq!(def.name, "sample");
    }
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test -p kiro-market-core --lib agent::parse::`
Expected: compile error.

**Step 3: Implement dispatch**

Create `crates/kiro-market-core/src/agent/parse.rs`:

```rust
//! Dialect detection and top-level parser dispatch.

use std::fs;
use std::path::{Path, PathBuf};

use crate::error::AgentError;

use super::types::{AgentDefinition, AgentDialect};
use super::{parse_claude_agent, parse_copilot_agent};

/// Detect the source dialect from a filename.
///
/// Filenames ending in `.agent.md` are treated as Copilot; everything else
/// as Claude. The `.agent.md` double-extension is the Copilot community
/// convention (see `awesome-copilot/agents/`).
pub fn detect_dialect(path: &Path) -> AgentDialect {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    if name.ends_with(".agent.md") {
        AgentDialect::Copilot
    } else {
        AgentDialect::Claude
    }
}

/// Read and parse an agent file, dispatching to the correct dialect parser.
pub fn parse_agent_file(path: &Path) -> Result<AgentDefinition, AgentError> {
    let content = fs::read_to_string(path).map_err(|e| AgentError::ParseFailed {
        path: path.to_path_buf(),
        reason: format!("read failed: {e}"),
    })?;
    let dialect = detect_dialect(path);
    let result = match dialect {
        AgentDialect::Claude => parse_claude_agent(&content),
        AgentDialect::Copilot => parse_copilot_agent(&content),
    };
    result.map_err(|reason| {
        if reason.contains("missing `name`") {
            AgentError::MissingName { path: path.to_path_buf() }
        } else {
            AgentError::ParseFailed { path: path.to_path_buf(), reason }
        }
    })
}
```

Wire up in `agent/mod.rs`:

```rust
mod parse;
pub use parse::{detect_dialect, parse_agent_file};
```

**Step 4: Run tests to verify they pass**

Run: `cargo test -p kiro-market-core --lib agent::parse::`
Expected: PASS (3 cases).

**Step 5: Commit**

```bash
git add crates/kiro-market-core/src/agent/
git commit -m "feat: detect Claude vs Copilot agent dialect from filename"
```

---

### Task 6: Tool-name mapping (Claude dialect)

**Files:**
- Create: `crates/kiro-market-core/src/agent/tools.rs`
- Modify: `crates/kiro-market-core/src/agent/mod.rs` (add `pub mod tools;`)

**Step 0: Verify the Kiro tool-name table**

Before writing the table, confirm the actual Kiro CLI tool names. The table below is provisional. Check by running:

```bash
# In a Kiro install:
kiro --help            # or the equivalent introspection command
# Or grep the Kiro source / agent-schema.json examples for tool name strings.
```

If the canonical names differ (e.g. Kiro uses `fs_read` instead of `read`), update the table in Step 3 before continuing. This is the single point where a wrong assumption silently corrupts every emitted agent — verify once, hard-code with confidence.

**Step 1: Write the failing tests**

In `agent/tools.rs` add `mod tests`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    #[case("Read", Some("read"))]
    #[case("Write", Some("write"))]
    #[case("Edit", Some("write"))]
    #[case("Bash", Some("shell"))]
    #[case("Grep", Some("grep"))]
    #[case("Glob", Some("glob"))]
    #[case("WebFetch", Some("web_fetch"))]
    #[case("WebSearch", Some("web_search"))]
    #[case("TodoWrite", Some("todo"))]
    #[case("Task", Some("subagent"))]
    #[case("NotebookEdit", None)]
    #[case("Skill", None)]
    #[case("Unknown", None)]
    fn claude_tool_maps_to_kiro(#[case] input: &str, #[case] expected: Option<&str>) {
        assert_eq!(map_claude_tool(input), expected.map(String::from));
    }

    #[test]
    fn map_claude_tools_returns_native_mapped_tools() {
        let (mapped, unmapped) = map_claude_tools(&[
            "Read".into(),
            "NotebookEdit".into(),
            "Skill".into(),
        ]);
        assert_eq!(mapped, vec![MappedTool::Native("read".into())]);
        assert_eq!(
            unmapped,
            vec![
                UnmappedTool { source: "NotebookEdit".into(), reason: UnmappedReason::NoKiroEquivalent },
                UnmappedTool { source: "Skill".into(), reason: UnmappedReason::NoKiroEquivalent },
            ]
        );
    }

    #[test]
    fn map_claude_tools_dedupes_write_from_edit_and_write() {
        let (mapped, _) = map_claude_tools(&["Edit".into(), "Write".into()]);
        assert_eq!(mapped, vec![MappedTool::Native("write".into())]);
    }
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test -p kiro-market-core --lib agent::tools::`
Expected: compile error.

**Step 3: Implement the structured types and the Claude tool map**

Create `crates/kiro-market-core/src/agent/tools.rs`:

```rust
//! Source-to-Kiro tool name mapping.
//!
//! Tools land in two different fields of the emitted agent JSON:
//! - native tool names (`read`, `shell`, etc.) → `allowedTools`
//! - MCP server references (`@server`, `@server/tool`) → `tools`
//!
//! The mapper returns a typed `MappedTool` so the emitter can route each
//! result to the correct field without re-parsing strings. Unmapped source
//! tools are returned structurally (not as pre-rendered messages) so callers
//! can re-render them as `InstallWarning` variants without string surgery.

/// A single source tool that has been successfully mapped to a Kiro identifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MappedTool {
    /// Native Kiro tool. Routed to `allowedTools` in the emitted JSON.
    Native(String),
    /// MCP server reference (`@server` or `@server/tool`). Routed to `tools`.
    McpRef(String),
}

/// A source tool that could not be mapped to any Kiro identifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnmappedTool {
    pub source: String,
    pub reason: UnmappedReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub enum UnmappedReason {
    /// Claude PascalCase name with no Kiro equivalent (e.g. `NotebookEdit`).
    NoKiroEquivalent,
    /// Copilot bare name (e.g. `codebase`, `findTestFiles`) — internal Copilot
    /// concept with no reliable Kiro mapping.
    BareCopilotName,
}

/// Look up the Kiro tool name for a Claude-style PascalCase tool name.
///
/// Returns `None` for tools with no Kiro equivalent. The caller is expected
/// to surface a warning for `None` results so the user knows the restriction
/// will not carry over.
#[must_use]
pub fn map_claude_tool(name: &str) -> Option<String> {
    // NOTE: Names verified in Step 0. Update this table if Kiro names diverge.
    let mapped = match name {
        "Read" => "read",
        "Write" | "Edit" => "write",
        "Bash" => "shell",
        "Grep" => "grep",
        "Glob" => "glob",
        "WebFetch" => "web_fetch",
        "WebSearch" => "web_search",
        "TodoWrite" => "todo",
        "Task" => "subagent",
        _ => return None,
    };
    Some(mapped.to_string())
}

/// Map a list of Claude tool names, returning the deduped Kiro list and a
/// vector of structured records for tools that had no mapping.
#[must_use]
pub fn map_claude_tools(source: &[String]) -> (Vec<MappedTool>, Vec<UnmappedTool>) {
    let mut mapped: Vec<MappedTool> = Vec::new();
    let mut unmapped: Vec<UnmappedTool> = Vec::new();
    for tool in source {
        match map_claude_tool(tool) {
            Some(kiro) => {
                let entry = MappedTool::Native(kiro);
                if !mapped.contains(&entry) {
                    mapped.push(entry);
                }
            }
            None => unmapped.push(UnmappedTool {
                source: tool.clone(),
                reason: UnmappedReason::NoKiroEquivalent,
            }),
        }
    }
    (mapped, unmapped)
}
```

Wire up in `agent/mod.rs`:

```rust
pub mod tools;
```

**Step 4: Run tests to verify they pass**

Run: `cargo test -p kiro-market-core --lib agent::tools::`
Expected: PASS.

**Step 5: Commit**

```bash
git add crates/kiro-market-core/src/agent/
git commit -m "feat: map Claude PascalCase tool names to Kiro tool names"
```

---

### Task 7: Tool-name mapping (Copilot dialect)

**Files:**
- Modify: `crates/kiro-market-core/src/agent/tools.rs`

**Step 1: Write the failing tests**

Add to `mod tests`:

```rust
#[test]
fn copilot_mcp_wildcard_maps_to_kiro_server_ref() {
    let (mapped, unmapped) = map_copilot_tools(&[
        "terraform/*".into(),
        "playwright/click".into(),
    ]);
    assert_eq!(
        mapped,
        vec![
            MappedTool::McpRef("@terraform".into()),
            MappedTool::McpRef("@playwright/click".into()),
        ]
    );
    assert!(unmapped.is_empty());
}

#[test]
fn copilot_bare_names_drop_with_structured_reason() {
    let (mapped, unmapped) = map_copilot_tools(&[
        "codebase".into(),
        "findTestFiles".into(),
        "problems".into(),
    ]);
    assert!(mapped.is_empty());
    assert_eq!(unmapped.len(), 3);
    assert!(unmapped.iter().all(|u| u.reason == UnmappedReason::BareCopilotName));
    assert_eq!(unmapped[0].source, "codebase");
}

#[test]
fn copilot_mixed_list_preserves_mcp_refs_drops_bare() {
    let (mapped, unmapped) = map_copilot_tools(&[
        "edit/editFiles".into(),
        "terraform/*".into(),
        "codebase".into(),
    ]);
    // edit/editFiles looks like MCP syntax but `edit` is not a known MCP server
    // in this agent; we pass it through as @edit/editFiles regardless.
    // The user can clean up in the emitted JSON.
    assert!(mapped.contains(&MappedTool::McpRef("@edit/editFiles".into())));
    assert!(mapped.contains(&MappedTool::McpRef("@terraform".into())));
    assert_eq!(unmapped.len(), 1);
    assert_eq!(unmapped[0].source, "codebase");
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test -p kiro-market-core --lib agent::tools::copilot`
Expected: compile error.

**Step 3: Implement the Copilot tool map**

Add to `agent/tools.rs`:

```rust
/// Map a list of Copilot source tool names to Kiro identifiers.
///
/// Copilot tools use mixed conventions:
/// - `{server}/*`     → `MappedTool::McpRef("@{server}")`        (whole-server access)
/// - `{server}/{tool}` → `MappedTool::McpRef("@{server}/{tool}")` (specific MCP tool)
/// - bare names (`codebase`, `findTestFiles`) → unmapped (BareCopilotName)
///
/// The bare-name drop is intentional per the design discussion: Copilot's
/// bare names are internal GitHub Copilot concepts with no reliable Kiro
/// equivalent. Users see the source tool list in the install output and
/// can restrict the emitted agent manually if desired.
#[must_use]
pub fn map_copilot_tools(source: &[String]) -> (Vec<MappedTool>, Vec<UnmappedTool>) {
    let mut mapped: Vec<MappedTool> = Vec::new();
    let mut unmapped: Vec<UnmappedTool> = Vec::new();
    for tool in source {
        if let Some((server, rest)) = tool.split_once('/') {
            let kiro = if rest == "*" {
                format!("@{server}")
            } else {
                format!("@{server}/{rest}")
            };
            let entry = MappedTool::McpRef(kiro);
            if !mapped.contains(&entry) {
                mapped.push(entry);
            }
        } else {
            unmapped.push(UnmappedTool {
                source: tool.clone(),
                reason: UnmappedReason::BareCopilotName,
            });
        }
    }
    (mapped, unmapped)
}
```

**Step 4: Run tests to verify they pass**

Run: `cargo test -p kiro-market-core --lib agent::tools`
Expected: PASS.

**Step 5: Commit**

```bash
git add crates/kiro-market-core/src/agent/tools.rs
git commit -m "feat: map Copilot MCP tool refs to Kiro; drop bare names with warnings"
```

---

### Task 8: Emit Kiro agent JSON + externalized prompt

**Files:**
- Create: `crates/kiro-market-core/src/agent/emit.rs`
- Modify: `crates/kiro-market-core/src/agent/mod.rs` (add `pub mod emit;`)

**Step 0: Verify the Kiro agent schema**

Read `agent-schema.json` at the repo root before writing the emitter. Confirm:
- `tools` field holds MCP refs (`@server`, `@server/tool`) per the schema description.
- `allowedTools` field holds explicit allowlist entries with `uniqueItems: true`.
- `mcpServers.<name>` follows the `CustomToolConfig` shape — find the `$defs/CustomToolConfig` block and note the allowed values for `type` (e.g. `stdio`, `http`, etc.). The `local → stdio` rewrite below is a placeholder; if the schema accepts `local` directly or uses different vocabulary, adjust.

If the schema disagrees with assumptions in the test bodies below, change the tests first, then code to them.

**Step 1: Write the failing tests**

In `agent/emit.rs` add `mod tests`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::tools::MappedTool;
    use crate::agent::types::{AgentDefinition, AgentDialect};
    use std::collections::BTreeMap;

    fn sample_claude_def() -> AgentDefinition {
        AgentDefinition {
            name: "reviewer".into(),
            description: Some("Reviews code".into()),
            prompt_body: "You are a reviewer.\n".into(),
            model: Some("opus".into()),
            source_tools: vec!["Read".into(), "Bash".into()],
            mcp_servers: BTreeMap::new(),
            dialect: AgentDialect::Claude,
        }
    }

    #[test]
    fn emit_sets_prompt_to_file_uri_relative_to_config() {
        let out = build_kiro_json(&sample_claude_def(), &[]).unwrap();
        assert_eq!(out["name"], "reviewer");
        assert_eq!(out["prompt"], "file://./prompts/reviewer.md");
    }

    #[test]
    fn emit_includes_description_and_model_when_present() {
        let out = build_kiro_json(&sample_claude_def(), &[]).unwrap();
        assert_eq!(out["description"], "Reviews code");
        assert_eq!(out["model"], "opus");
    }

    #[test]
    fn emit_omits_model_when_none() {
        let mut def = sample_claude_def();
        def.model = None;
        let out = build_kiro_json(&def, &[]).unwrap();
        assert!(out.get("model").is_none());
    }

    #[test]
    fn emit_routes_native_tools_into_allowed_tools() {
        let mapped = vec![
            MappedTool::Native("read".into()),
            MappedTool::Native("shell".into()),
        ];
        let out = build_kiro_json(&sample_claude_def(), &mapped).unwrap();
        let allowed = out["allowedTools"].as_array().unwrap();
        assert_eq!(allowed, &vec![
            serde_json::Value::String("read".into()),
            serde_json::Value::String("shell".into()),
        ]);
        // Native names must NOT leak into `tools` (which is for MCP refs).
        assert!(out.get("tools").is_none(),
            "no MCP refs in this list — `tools` field must be omitted");
    }

    #[test]
    fn emit_routes_mcp_refs_into_tools_field() {
        let mapped = vec![
            MappedTool::McpRef("@terraform".into()),
            MappedTool::McpRef("@playwright/click".into()),
        ];
        let out = build_kiro_json(&sample_claude_def(), &mapped).unwrap();
        let tools = out["tools"].as_array().unwrap();
        assert_eq!(tools[0], "@terraform");
        assert_eq!(tools[1], "@playwright/click");
        assert!(out.get("allowedTools").is_none(),
            "no native names — `allowedTools` field must be omitted");
    }

    #[test]
    fn emit_routes_mixed_lists_to_both_fields() {
        let mapped = vec![
            MappedTool::Native("read".into()),
            MappedTool::McpRef("@terraform".into()),
        ];
        let out = build_kiro_json(&sample_claude_def(), &mapped).unwrap();
        assert_eq!(out["allowedTools"][0], "read");
        assert_eq!(out["tools"][0], "@terraform");
    }

    #[test]
    fn emit_omits_tool_arrays_when_empty() {
        let out = build_kiro_json(&sample_claude_def(), &[]).unwrap();
        assert!(out.get("allowedTools").is_none());
        assert!(out.get("tools").is_none(),
            "empty tool list must omit both arrays so Kiro inherits full parent toolset");
    }

    #[test]
    fn emit_normalizes_mcp_server_type_local_to_stdio() {
        // PRE-CONDITION: confirmed in Step 0 that Kiro's CustomToolConfig
        // accepts `stdio`. If the schema names this differently, update both
        // the assertion and `normalize_mcp_server`.
        let mut def = sample_claude_def();
        def.mcp_servers.insert(
            "terraform".into(),
            serde_json::json!({
                "type": "local",
                "command": "docker",
                "args": ["run", "-i"],
                "tools": ["*"]
            }),
        );
        let out = build_kiro_json(&def, &[]).unwrap();
        let tf = &out["mcpServers"]["terraform"];
        assert_eq!(tf["type"], "stdio");
        // Inner `tools: ["*"]` allowlist is stripped — Kiro has no equivalent
        // and @{server} already covers "all tools".
        assert!(tf.get("tools").is_none());
    }
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test -p kiro-market-core --lib agent::emit`
Expected: compile error.

**Step 3: Implement emission**

Create `crates/kiro-market-core/src/agent/emit.rs`:

```rust
//! Emit Kiro agent JSON from an `AgentDefinition`.
//!
//! The emitted JSON matches the schema at `agent-schema.json`. The `prompt`
//! field uses Kiro's `file://` URI to reference the externalized prompt
//! markdown, so the `.md` remains the source of truth after install.
//!
//! NOTE on field ordering: workspace `serde_json` does not enable the
//! `preserve_order` feature, so `serde_json::Map` is BTreeMap-backed and
//! emits keys alphabetically in the on-disk JSON. Tests index by key so
//! they're unaffected, but reviewers reading the file should not expect
//! insertion order.

use serde_json::{Map, Value};

use super::tools::MappedTool;
use super::types::AgentDefinition;

/// Build the Kiro-compatible JSON for an agent, given the parsed definition
/// and the already-mapped tool list.
///
/// `mapped_tools` is the output of `tools::map_claude_tools` or
/// `tools::map_copilot_tools`. Native entries land in `allowedTools`; MCP
/// refs land in `tools` per the Kiro agent schema.
pub fn build_kiro_json(
    def: &AgentDefinition,
    mapped_tools: &[MappedTool],
) -> serde_json::Result<Value> {
    let mut obj = Map::new();
    obj.insert("name".into(), Value::String(def.name.clone()));
    if let Some(desc) = &def.description {
        obj.insert("description".into(), Value::String(desc.clone()));
    }
    obj.insert(
        "prompt".into(),
        Value::String(format!("file://./prompts/{}.md", def.name)),
    );
    if let Some(model) = &def.model {
        obj.insert("model".into(), Value::String(model.clone()));
    }

    let mut allowed: Vec<Value> = Vec::new();
    let mut tools: Vec<Value> = Vec::new();
    for entry in mapped_tools {
        match entry {
            MappedTool::Native(s) => allowed.push(Value::String(s.clone())),
            MappedTool::McpRef(s) => tools.push(Value::String(s.clone())),
        }
    }
    if !allowed.is_empty() {
        obj.insert("allowedTools".into(), Value::Array(allowed));
    }
    if !tools.is_empty() {
        obj.insert("tools".into(), Value::Array(tools));
    }

    if !def.mcp_servers.is_empty() {
        let mut servers = Map::new();
        for (name, raw) in &def.mcp_servers {
            servers.insert(name.clone(), normalize_mcp_server(raw));
        }
        obj.insert("mcpServers".into(), Value::Object(servers));
    }
    Ok(Value::Object(obj))
}

/// Normalize a Copilot MCP server entry to Kiro's `CustomToolConfig` shape.
///
/// Rules (verified against `agent-schema.json` `$defs/CustomToolConfig` in Step 0):
/// - `type: "local"` → `type: "stdio"`
/// - inner `tools` allowlist → dropped (Kiro has no per-server allowlist;
///   the outer `tools` array already handles whole-server access via `@name`).
fn normalize_mcp_server(raw: &Value) -> Value {
    let Some(obj) = raw.as_object() else {
        return raw.clone();
    };
    let mut out = Map::new();
    for (k, v) in obj {
        if k == "tools" {
            continue;
        }
        if k == "type" && v.as_str() == Some("local") {
            out.insert("type".into(), Value::String("stdio".into()));
            continue;
        }
        out.insert(k.clone(), v.clone());
    }
    Value::Object(out)
}
```

Wire up in `agent/mod.rs`:

```rust
pub mod emit;
```

**Step 4: Run tests to verify they pass**

Run: `cargo test -p kiro-market-core --lib agent::emit`
Expected: PASS.

**Step 5: Commit**

```bash
git add crates/kiro-market-core/src/agent/
git commit -m "feat: emit Kiro agent JSON with file:// prompt reference"
```

---

### Task 9: Add `InstalledAgentMeta` tracking structure

**Files:**
- Modify: `crates/kiro-market-core/src/project.rs` (add structs near `InstalledSkillMeta`, add `INSTALLED_AGENTS_FILE` constant)

**Step 1: Write the failing tests**

Append to the existing `mod tests` in `project.rs`:

```rust
#[test]
fn installed_agent_meta_roundtrips_json() {
    let meta = InstalledAgentMeta {
        marketplace: "mp".into(),
        plugin: "pr-review-toolkit".into(),
        version: Some("1.2.3".into()),
        installed_at: Utc::now(),
        dialect: AgentDialect::Claude,
    };
    let json = serde_json::to_string(&meta).unwrap();
    let back: InstalledAgentMeta = serde_json::from_str(&json).unwrap();
    assert_eq!(back.plugin, "pr-review-toolkit");
    assert_eq!(back.dialect, AgentDialect::Claude);
    // Spot-check the wire format: dialect serializes lowercase.
    assert!(json.contains("\"dialect\":\"claude\""));
}

#[test]
fn installed_agents_default_is_empty() {
    let ia = InstalledAgents::default();
    assert!(ia.agents.is_empty());
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test -p kiro-market-core --lib project::tests::installed_agent`
Expected: compile error.

**Step 3: Add the types**

In `project.rs`, near the existing `InstalledSkillMeta` / `InstalledSkills`, insert:

```rust
use crate::agent::AgentDialect;

/// Metadata recorded for each installed agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledAgentMeta {
    pub marketplace: String,
    pub plugin: String,
    pub version: Option<String>,
    pub installed_at: DateTime<Utc>,
    /// Which source dialect the agent was parsed from. Persisted via the
    /// enum's serde rename, not as a free-form string, to avoid drift.
    pub dialect: AgentDialect,
}

/// The on-disk structure of `installed-agents.json`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InstalledAgents {
    pub agents: HashMap<String, InstalledAgentMeta>,
}

/// Name of the agent tracking file inside `.kiro/`.
const INSTALLED_AGENTS_FILE: &str = "installed-agents.json";
```

**Step 4: Run tests to verify they pass**

Run: `cargo test -p kiro-market-core --lib project::tests::installed_agent`
Expected: PASS (2 tests).

**Step 5: Commit**

```bash
git add crates/kiro-market-core/src/project.rs
git commit -m "feat: add InstalledAgentMeta tracking type for agents"
```

---

### Task 10: `KiroProject::install_agent` method

**Why this signature changed from the original draft.** The original Task 10 took `(name, source_file, meta)` and re-parsed the file inside `install_agent_from_file`. But Task 14's service already parses the file to compute warnings — so the service ended up parsing once for warnings and the project method parsed again for installation. Pass the parsed `AgentDefinition` + mapped tools straight in. Each file is parsed exactly once.

**Why atomic staging.** `KiroProject::write_skill_dir` (`project.rs:296`) carefully stages skill files into a temp dir under the `.kiro/` root and renames into place so a crash can never leave a half-installed skill on disk. This task mirrors that contract for agents: prompt + JSON go to a per-attempt staging directory, then renames swap them into place atomically; tracking is written last and inside the same lock.

**Files:**
- Modify: `crates/kiro-market-core/src/project.rs` (add `install_agent` method, internal staging helper, `load_installed_agents`)

**Step 1: Write the failing tests**

Add to `mod tests` in `project.rs`:

```rust
fn write_agent(name: &str, body: &str) -> (tempfile::TempDir, std::path::PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let p = tmp.path().join(format!("{name}.md"));
    std::fs::write(&p, body).unwrap();
    (tmp, p)
}

fn parse_and_map(source: &Path) -> (AgentDefinition, Vec<MappedTool>) {
    let def = crate::agent::parse_agent_file(source).expect("parse");
    let (mapped, _unmapped) = match def.dialect {
        AgentDialect::Claude => crate::agent::tools::map_claude_tools(&def.source_tools),
        AgentDialect::Copilot => crate::agent::tools::map_copilot_tools(&def.source_tools),
    };
    (def, mapped)
}

#[test]
fn install_agent_writes_json_and_prompt() {
    let project = KiroProject::init_in_temp().unwrap();
    let (_tmp, src) = write_agent(
        "reviewer",
        "---\nname: reviewer\ndescription: Reviews\n---\nYou are a reviewer.\n",
    );
    let (def, mapped) = parse_and_map(&src);

    project
        .install_agent(&def, &mapped, sample_agent_meta())
        .expect("install");

    let json_path = project.root().join(".kiro/agents/reviewer.json");
    let prompt_path = project.root().join(".kiro/agents/prompts/reviewer.md");
    assert!(json_path.exists(), "JSON written");
    assert!(prompt_path.exists(), "prompt markdown written");

    let json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&json_path).unwrap()).unwrap();
    assert_eq!(json["name"], "reviewer");
    assert_eq!(json["prompt"], "file://./prompts/reviewer.md");

    let prompt = std::fs::read_to_string(&prompt_path).unwrap();
    assert!(prompt.starts_with("You are a reviewer."),
        "prompt body written without frontmatter, got: {prompt:?}");
}

#[test]
fn install_agent_rejects_duplicate() {
    let project = KiroProject::init_in_temp().unwrap();
    let (_tmp, src) = write_agent("a", "---\nname: a\n---\nbody\n");
    let (def, mapped) = parse_and_map(&src);

    project.install_agent(&def, &mapped, sample_agent_meta()).unwrap();
    let err = project
        .install_agent(&def, &mapped, sample_agent_meta())
        .unwrap_err();
    assert!(matches!(err, Error::Agent(AgentError::AlreadyInstalled { .. })));
}

#[test]
fn install_agent_updates_tracking() {
    let project = KiroProject::init_in_temp().unwrap();
    let (_tmp, src) = write_agent("a", "---\nname: a\n---\nbody\n");
    let (def, mapped) = parse_and_map(&src);
    project.install_agent(&def, &mapped, sample_agent_meta()).unwrap();

    let tracking_path = project.root().join(".kiro/installed-agents.json");
    let tracking: InstalledAgents =
        serde_json::from_str(&std::fs::read_to_string(tracking_path).unwrap()).unwrap();
    assert!(tracking.agents.contains_key("a"));
    assert_eq!(tracking.agents["a"].dialect, AgentDialect::Claude);
}

#[test]
fn install_agent_rejects_unsafe_name() {
    let project = KiroProject::init_in_temp().unwrap();
    let (_tmp, src) = write_agent("x", "---\nname: x\n---\nbody\n");
    let (mut def, mapped) = parse_and_map(&src);
    // Force an unsafe name into the parsed def to exercise validation.
    def.name = "../escape".into();
    let err = project.install_agent(&def, &mapped, sample_agent_meta()).unwrap_err();
    assert!(matches!(err, Error::Validation(_)));
}

fn sample_agent_meta() -> InstalledAgentMeta {
    InstalledAgentMeta {
        marketplace: "mp".into(),
        plugin: "p".into(),
        version: None,
        installed_at: Utc::now(),
        dialect: AgentDialect::Claude,
    }
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test -p kiro-market-core --lib project::tests::install_agent`
Expected: compile error.

**Step 3: Implement the method (atomic, mirroring `write_skill_dir`)**

In `project.rs`, add after the existing skill install methods:

```rust
/// Install a parsed agent: emit its Kiro JSON + externalized prompt
/// markdown under `.kiro/agents/`, and record metadata in
/// `installed-agents.json`.
///
/// The caller is responsible for parsing the source file and mapping the
/// tool list (the service layer does both upstream so warnings can be
/// surfaced before the install lock is acquired). This method is purely
/// the on-disk write step.
///
/// File writes use the same staging-and-rename pattern as
/// [`Self::install_skill_from_dir`] — prompt + JSON are written to
/// `_installing-agents-<name>-<pid>-<seq>/` under `.kiro/`, then renamed
/// into `.kiro/agents/{name}.json` and `.kiro/agents/prompts/{name}.md`
/// after the duplicate check, then tracking is written last. The whole
/// flow runs under the existing tracking lock.
///
/// # Errors
///
/// - [`AgentError::AlreadyInstalled`] if an agent with this name already exists.
/// - Validation errors for unsafe names.
/// - I/O errors.
pub fn install_agent(
    &self,
    def: &AgentDefinition,
    mapped_tools: &[MappedTool],
    meta: InstalledAgentMeta,
) -> crate::error::Result<()> {
    validation::validate_name(&def.name)?;
    let json = crate::agent::emit::build_kiro_json(def, mapped_tools)?;
    let json_bytes = serde_json::to_vec_pretty(&json)?;

    let tracking_path = self.kiro_dir().join(INSTALLED_AGENTS_FILE);
    crate::file_lock::with_file_lock(&tracking_path, || -> crate::error::Result<()> {
        let mut tracking = self.load_installed_agents()?;
        if tracking.agents.contains_key(&def.name) {
            return Err(AgentError::AlreadyInstalled {
                name: def.name.clone(),
            }
            .into());
        }

        // Stage under .kiro/ so rename is on the same filesystem as the target.
        let staging = self
            .kiro_dir()
            .join(format!(
                "_installing-agent-{name}-{pid}-{seq}",
                name = def.name,
                pid = std::process::id(),
                seq = staging_seq(),
            ));
        fs::create_dir_all(staging.join("prompts"))?;
        fs::write(staging.join(format!("{name}.json", name = def.name)), &json_bytes)?;
        fs::write(
            staging.join("prompts").join(format!("{name}.md", name = def.name)),
            def.prompt_body.as_bytes(),
        )?;

        let agents_root = self.kiro_dir().join("agents");
        let prompts_root = agents_root.join("prompts");
        fs::create_dir_all(&prompts_root)?;

        // Atomic rename for both files; if either fails we clean up the staging dir
        // and propagate.
        let json_target = agents_root.join(format!("{name}.json", name = def.name));
        let prompt_target = prompts_root.join(format!("{name}.md", name = def.name));
        let rename_result = fs::rename(
            staging.join(format!("{name}.json", name = def.name)),
            &json_target,
        )
        .and_then(|()| {
            fs::rename(
                staging.join("prompts").join(format!("{name}.md", name = def.name)),
                &prompt_target,
            )
        });
        // Best-effort cleanup of the staging directory regardless of outcome.
        let _ = fs::remove_dir_all(&staging);
        rename_result?;

        tracking.agents.insert(def.name.clone(), meta);
        let tracking_bytes = serde_json::to_vec_pretty(&tracking)?;
        fs::write(&tracking_path, tracking_bytes)?;
        Ok(())
    })
}

fn load_installed_agents(&self) -> crate::error::Result<InstalledAgents> {
    let path = self.kiro_dir().join(INSTALLED_AGENTS_FILE);
    if !path.exists() {
        return Ok(InstalledAgents::default());
    }
    let bytes = fs::read(path)?;
    Ok(serde_json::from_slice(&bytes)?)
}
```

`staging_seq()` should mirror whatever the skill flow uses (e.g. an `AtomicUsize::fetch_add`). If no helper exists, add one local to this file or reuse the skill version. The exact source of monotonicity matters only for distinct staging-dir names within the same process — the lock already serializes cross-process attempts.

**Step 4: Run tests to verify they pass**

Run: `cargo test -p kiro-market-core --lib project::tests::install_agent`
Expected: PASS (4 tests).

**Step 5: Commit**

```bash
git add crates/kiro-market-core/src/project.rs
git commit -m "feat: install parsed agent into .kiro/agents/ atomically"
```

---

### Task 11: Extend `PluginManifest` with `agents` field

**Files:**
- Modify: `crates/kiro-market-core/src/plugin.rs` (add `agents: Vec<String>` field to `PluginManifest`; add `DEFAULT_AGENT_PATHS` constant to `lib.rs`)
- Modify: `crates/kiro-market-core/src/lib.rs`

**Step 1: Write the failing tests**

Add to `mod tests` in `plugin.rs`:

```rust
#[test]
fn plugin_manifest_parses_agents_field() {
    let json = br#"{
        "name": "p",
        "skills": ["./skills/"],
        "agents": ["./agents/"]
    }"#;
    let m = PluginManifest::from_json(json).unwrap();
    assert_eq!(m.agents, vec!["./agents/"]);
}

#[test]
fn plugin_manifest_defaults_agents_to_empty() {
    let json = br#"{"name": "p"}"#;
    let m = PluginManifest::from_json(json).unwrap();
    assert!(m.agents.is_empty());
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test -p kiro-market-core --lib plugin::tests::plugin_manifest_parses_agents`
Expected: compile error — `agents` field does not exist.

**Step 3: Add the field**

In `plugin.rs`:

```rust
#[derive(Debug, Clone, Deserialize)]
pub struct PluginManifest {
    pub name: String,
    pub version: Option<String>,
    pub description: Option<String>,
    #[serde(default)]
    pub skills: Vec<String>,
    #[serde(default)]
    pub agents: Vec<String>,
}
```

In `lib.rs`, add below `DEFAULT_SKILL_PATHS`:

```rust
/// Default agent scan paths when a plugin has no `plugin.json` or its
/// `agents` list is empty.
pub const DEFAULT_AGENT_PATHS: &[&str] = &["./agents/"];
```

**Step 4: Run tests to verify they pass**

Run: `cargo test -p kiro-market-core --lib plugin::tests::plugin_manifest`
Expected: PASS.

**Step 5: Commit**

```bash
git add crates/kiro-market-core/src/plugin.rs crates/kiro-market-core/src/lib.rs
git commit -m "feat: extend PluginManifest with agents field and default paths"
```

---

### Task 12: `discover_agents_in_plugin()` scan helper

**Files:**
- Create: `crates/kiro-market-core/src/agent/discover.rs`
- Modify: `crates/kiro-market-core/src/agent/mod.rs` (add `pub mod discover;`)

**Step 1: Write the failing tests**

In `agent/discover.rs` add `mod tests`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn discover_finds_both_md_and_agent_md() {
        let tmp = tempdir().unwrap();
        let agents = tmp.path().join("agents");
        std::fs::create_dir_all(&agents).unwrap();
        std::fs::write(agents.join("claude.md"), "---\nname: c\n---\n").unwrap();
        std::fs::write(agents.join("copilot.agent.md"), "---\nname: o\n---\n").unwrap();
        std::fs::write(agents.join("README.txt"), "ignored").unwrap();

        let found = discover_agents_in_dirs(tmp.path(), &["./agents/".to_string()]);
        let names: Vec<_> = found.iter().map(|p| p.file_name().unwrap().to_string_lossy().to_string()).collect();
        assert!(names.contains(&"claude.md".to_string()));
        assert!(names.contains(&"copilot.agent.md".to_string()));
        assert!(!names.contains(&"README.txt".to_string()));
    }

    #[test]
    fn discover_returns_empty_when_directory_missing() {
        let tmp = tempdir().unwrap();
        let found = discover_agents_in_dirs(tmp.path(), &["./nope/".to_string()]);
        assert!(found.is_empty());
    }

    #[test]
    fn discover_excludes_readme_and_contributing() {
        let tmp = tempdir().unwrap();
        let agents = tmp.path().join("agents");
        std::fs::create_dir_all(&agents).unwrap();
        std::fs::write(agents.join("README.md"), "# README").unwrap();
        std::fs::write(agents.join("CONTRIBUTING.md"), "# Contrib").unwrap();
        std::fs::write(agents.join("real.md"), "---\nname: r\n---\n").unwrap();

        let found = discover_agents_in_dirs(tmp.path(), &["./agents/".to_string()]);
        let names: Vec<_> = found.iter().map(|p| p.file_name().unwrap().to_string_lossy().to_string()).collect();
        assert_eq!(names, vec!["real.md"]);
    }

    #[test]
    fn discover_does_not_recurse_into_subdirectories() {
        // Prevents accidentally picking up nested `agents/prompts/*.md` or
        // backup directories.
        let tmp = tempdir().unwrap();
        let agents = tmp.path().join("agents");
        let nested = agents.join("archived");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(agents.join("top.md"), "---\nname: t\n---\n").unwrap();
        std::fs::write(nested.join("deep.md"), "---\nname: d\n---\n").unwrap();

        let found = discover_agents_in_dirs(tmp.path(), &["./agents/".to_string()]);
        let names: Vec<_> = found.iter().map(|p| p.file_name().unwrap().to_string_lossy().to_string()).collect();
        assert_eq!(names, vec!["top.md"]);
    }
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test -p kiro-market-core --lib agent::discover`
Expected: compile error.

**Step 3: Implement discovery**

Create `crates/kiro-market-core/src/agent/discover.rs`:

```rust
//! Scan a plugin directory for agent markdown files.

use std::fs;
use std::path::{Path, PathBuf};

/// Find agent markdown files inside `plugin_dir` according to `scan_paths`.
///
/// `scan_paths` are relative to `plugin_dir`. Files ending in `.md` or
/// `.agent.md` are included; the caller uses `detect_dialect` at parse time
/// to route to the right parser. Scans are non-recursive: only direct
/// children of each scan directory are considered. This avoids grabbing
/// nested `prompts/*.md` or editor backup files.
///
/// `README.md` and `CONTRIBUTING.md` are excluded by name so plugins can keep
/// docs in their `agents/` directory without producing parse-failure warnings.
/// Other non-agent `.md` files (e.g. notes a plugin author drops in) will
/// still be picked up, parsed, and surfaced as `AgentParseFailed` warnings —
/// noisy but accurate. Service-layer rule: when the parse error reason is
/// "missing opening `---` frontmatter fence", demote to `tracing::debug!`
/// instead of pushing an `InstallWarning`. See Task 14.
#[must_use]
pub fn discover_agents_in_dirs(plugin_dir: &Path, scan_paths: &[String]) -> Vec<PathBuf> {
    const EXCLUDED: &[&str] = &["README.md", "CONTRIBUTING.md", "CHANGELOG.md"];
    let mut out = Vec::new();
    for rel in scan_paths {
        let dir = plugin_dir.join(rel.trim_start_matches("./"));
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if EXCLUDED.contains(&name) {
                continue;
            }
            if name.ends_with(".md") {
                out.push(path);
            }
        }
    }
    out
}
```

Wire up in `agent/mod.rs`:

```rust
pub mod discover;
```

**Step 4: Run tests to verify they pass**

Run: `cargo test -p kiro-market-core --lib agent::discover`
Expected: PASS.

**Step 5: Commit**

```bash
git add crates/kiro-market-core/src/agent/
git commit -m "feat: scan plugin directory for agent markdown files"
```

---

### Task 13: `InstallWarning` structured warning type

**Files:**
- Modify: `crates/kiro-market-core/src/service.rs` (add `InstallWarning` enum + the collection type in install results)

**Step 1: Write the failing tests**

Add to the existing service test module (look for `mod tests` at the end):

```rust
#[test]
fn install_warning_unmapped_tool_renders_cleanly() {
    let w = InstallWarning::UnmappedTool {
        agent: "reviewer".into(),
        tool: "NotebookEdit".into(),
    };
    let s = w.to_string();
    assert!(s.contains("reviewer"));
    assert!(s.contains("NotebookEdit"));
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test -p kiro-market-core --lib service::tests::install_warning`
Expected: compile error.

**Step 3: Add `InstallWarning`**

In `service.rs`, near the top-level result types:

```rust
use crate::agent::tools::UnmappedReason;

/// Non-fatal issue produced during install. Surfaced in install results
/// so the CLI / Tauri frontend can render them without blocking the install.
#[derive(Clone, Debug, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub enum InstallWarning {
    /// A source-declared tool had no Kiro equivalent and was dropped.
    /// The emitted agent will inherit the full parent toolset for that slot.
    UnmappedTool {
        agent: String,
        tool: String,
        reason: UnmappedReason,
    },
    /// An agent file could not be parsed; it was skipped.
    AgentParseFailed { path: PathBuf, reason: String },
}

impl std::fmt::Display for InstallWarning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InstallWarning::UnmappedTool { agent, tool, reason } => {
                let why = match reason {
                    UnmappedReason::NoKiroEquivalent => "no Kiro equivalent",
                    UnmappedReason::BareCopilotName => "Copilot bare name; not portable",
                };
                write!(f, "agent `{agent}`: tool `{tool}` dropped ({why})")
            }
            InstallWarning::AgentParseFailed { path, reason } => write!(
                f,
                "skipped agent at {}: {reason}",
                path.display()
            ),
        }
    }
}
```

`UnmappedReason` will need `Serialize` + `specta::Type` (under the feature). Add those derives in Task 6 if they aren't already there — easier to revisit Task 6 than to add a stringly conversion shim here.

**Step 4: Run tests to verify they pass**

Run: `cargo test -p kiro-market-core --lib service::tests::install_warning`
Expected: PASS.

**Step 5: Commit**

```bash
git add crates/kiro-market-core/src/service.rs
git commit -m "feat: add InstallWarning type for non-fatal install issues"
```

---

### Task 14: `MarketplaceService::install_plugin_agents`

**Files:**
- Modify: `crates/kiro-market-core/src/service.rs` (add a method that iterates discovered agent files, parses, maps tools, installs each, and returns `(installed_count, warnings)`)

**Step 1: Write the failing integration test**

Add to `service::tests`:

```rust
#[test]
fn install_plugin_agents_emits_json_and_warnings_per_file() {
    let tmp = tempfile::tempdir().unwrap();
    let plugin_dir = tmp.path().join("plugin-x");
    let agents_dir = plugin_dir.join("agents");
    std::fs::create_dir_all(&agents_dir).unwrap();

    // One Claude agent with a tool that maps cleanly.
    std::fs::write(
        agents_dir.join("reviewer.md"),
        "---\nname: reviewer\ndescription: Reviews\ntools: [Read, NotebookEdit]\n---\nYou are a reviewer.\n",
    ).unwrap();

    // One Copilot agent with a bare unmapped tool.
    std::fs::write(
        agents_dir.join("tester.agent.md"),
        "---\nname: tester\ntools: ['codebase', 'terraform/*']\n---\nBody.\n",
    ).unwrap();

    let project = KiroProject::init_in_temp().unwrap();
    // Match the constructor used by existing service tests in this file
    // (see e.g. `MarketplaceService::new(cache, MockGitBackend::default())`
    // around service.rs:649). Reuse whatever mock backend the nearest test
    // already constructs — this method does not invoke git.
    let svc = MarketplaceService::new(test_cache(), MockGitBackend::default());

    let (count, warnings) = svc
        .install_plugin_agents(
            &project,
            &plugin_dir,
            &["./agents/".to_string()],
            /*marketplace*/ "mp",
            /*plugin*/ "plugin-x",
            /*version*/ None,
        )
        .expect("install");

    assert_eq!(count, 2, "both agents installed");
    assert!(project.root().join(".kiro/agents/reviewer.json").exists());
    assert!(project.root().join(".kiro/agents/tester.json").exists());
    assert!(project.root().join(".kiro/agents/prompts/reviewer.md").exists());

    // Warnings should be structured: NotebookEdit → NoKiroEquivalent,
    // codebase → BareCopilotName. Match by variant, not by string.
    use crate::agent::tools::UnmappedReason;
    let unmapped: Vec<_> = warnings
        .iter()
        .filter_map(|w| match w {
            InstallWarning::UnmappedTool { tool, reason, .. } => Some((tool.as_str(), *reason)),
            _ => None,
        })
        .collect();
    assert!(unmapped.contains(&("NotebookEdit", UnmappedReason::NoKiroEquivalent)),
        "expected NotebookEdit unmapped: {unmapped:?}");
    assert!(unmapped.contains(&("codebase", UnmappedReason::BareCopilotName)),
        "expected codebase unmapped: {unmapped:?}");
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p kiro-market-core --lib service::tests::install_plugin_agents`
Expected: compile error — method does not exist.

**Step 3: Implement the method**

In `service.rs`, add:

```rust
impl MarketplaceService {
    /// Discover, parse, and install all agents from a plugin directory.
    ///
    /// Returns `(installed_count, warnings)`. Parse failures on individual
    /// agents are captured as warnings and do not abort the whole install.
    /// Each file is parsed exactly once — the parsed `AgentDefinition` is
    /// passed straight into `project.install_agent`, so the project layer
    /// never re-reads the source.
    ///
    /// # Errors
    ///
    /// Returns an error only for fatal problems (e.g. filesystem errors
    /// writing the tracking file). Per-file parse failures become warnings.
    pub fn install_plugin_agents(
        &self,
        project: &crate::project::KiroProject,
        plugin_dir: &Path,
        scan_paths: &[String],
        marketplace: &str,
        plugin: &str,
        version: Option<&str>,
    ) -> crate::error::Result<(usize, Vec<InstallWarning>)> {
        let files = crate::agent::discover::discover_agents_in_dirs(plugin_dir, scan_paths);
        let mut installed = 0_usize;
        let mut warnings: Vec<InstallWarning> = Vec::new();

        for path in files {
            let def = match crate::agent::parse_agent_file(&path) {
                Ok(d) => d,
                Err(e) => {
                    let reason = e.to_string();
                    // Demote "no frontmatter at all" — these are usually
                    // human-readable docs that share an `agents/` directory
                    // with real agent files. See discover.rs comment.
                    if reason.contains("missing opening `---` frontmatter fence") {
                        tracing::debug!(path = %path.display(), "skipping non-agent markdown");
                    } else {
                        warnings.push(InstallWarning::AgentParseFailed {
                            path: path.clone(),
                            reason,
                        });
                    }
                    continue;
                }
            };
            let (mapped, unmapped) = match def.dialect {
                crate::agent::AgentDialect::Claude => {
                    crate::agent::tools::map_claude_tools(&def.source_tools)
                }
                crate::agent::AgentDialect::Copilot => {
                    crate::agent::tools::map_copilot_tools(&def.source_tools)
                }
            };
            for u in unmapped {
                warnings.push(InstallWarning::UnmappedTool {
                    agent: def.name.clone(),
                    tool: u.source,
                    reason: u.reason,
                });
            }

            let meta = crate::project::InstalledAgentMeta {
                marketplace: marketplace.to_string(),
                plugin: plugin.to_string(),
                version: version.map(String::from),
                installed_at: chrono::Utc::now(),
                dialect: def.dialect,
            };
            project.install_agent(&def, &mapped, meta)?;
            installed += 1;
        }

        Ok((installed, warnings))
    }
}
```

**Step 4: Run test to verify it passes**

Run: `cargo test -p kiro-market-core --lib service::tests::install_plugin_agents`
Expected: PASS.

**Step 5: Commit**

```bash
git add crates/kiro-market-core/src/service.rs
git commit -m "feat: install all agents from a plugin directory with warnings"
```

---

### Task 15: Wire agent install into plugin install flow

**Files:**
- Modify: `crates/kiro-market-core/src/service.rs` (the existing `install_plugin` / equivalent call site — find where skills are installed per-plugin and add an agent pass right after)

**Step 1: Find the skill install call site**

Run: `rg 'install_skill_from_dir|install_plugin_skills' crates/kiro-market-core/src/service.rs`

Note which function owns that call and what it returns. The agent install pass should run **after** all skills install for a given plugin, append warnings to any existing warning list, and include the installed agent count in the result.

**Step 2: Write the failing test**

Add a test that exercises the full plugin-install flow with a plugin that ships both skills and agents, asserting that:
- The skills land in `.kiro/skills/`.
- The agents land in `.kiro/agents/`.
- Warnings from agent installation bubble up to the plugin install result.

(Copy the shape of the existing plugin-install happy-path test as a starting point.)

**Step 3: Run test to verify it fails**

Expected: agents are not installed.

**Step 4: Add the agent pass**

Where skills are installed, after the skill loop:

```rust
let scan_paths = if plugin_manifest.agents.is_empty() {
    crate::DEFAULT_AGENT_PATHS.iter().map(|s| (*s).to_string()).collect()
} else {
    plugin_manifest.agents.clone()
};
let (agent_count, mut agent_warnings) = self.install_plugin_agents(
    project,
    plugin_dir,
    &scan_paths,
    marketplace_name,
    &plugin_manifest.name,
    plugin_manifest.version.as_deref(),
)?;
// Append to whatever warning list the existing skill flow accumulated.
warnings.append(&mut agent_warnings);
```

Extend the return type to include `agent_count` if it doesn't already. If the existing result struct only carries skill info, add a new `agents_installed: usize` field.

**Step 5: Run tests to verify**

Run: `cargo test -p kiro-market-core`
Expected: all existing tests plus the new one pass.

**Step 6: Commit**

```bash
git add crates/kiro-market-core/src/service.rs
git commit -m "feat: install agents as part of plugin install flow"
```

---

### Task 16: Render agent install results in the CLI

**Files:**
- Modify: `crates/kiro-market/src/commands/install.rs` (print agent counts and warnings next to skill counts)

**Step 1: Read the current install output**

Run: `cat crates/kiro-market/src/commands/install.rs | head -80`

Note the existing output format for skills — whatever pattern is used (plain `println!`, a table, colored labels) should be mirrored for agents.

**Step 2: Write an integration test**

Add a test to `crates/kiro-market/tests/` (or extend an existing install CLI test) that runs the install command against a fixture plugin with one agent and asserts that the stdout mentions:
- The agent name.
- The warning about any dropped tool.

Use `assert_cmd` and `predicates` if those are already in the test dev-dependencies; otherwise match the existing CLI test style.

**Step 3: Run test to verify it fails**

Expected: agents are installed but not printed.

**Step 4: Add CLI rendering**

In `install.rs`, after the existing skill summary:

```rust
if result.agents_installed > 0 {
    println!(
        "{} {} agent{} installed",
        "+".green(),
        result.agents_installed,
        if result.agents_installed == 1 { "" } else { "s" },
    );
}
for warning in &result.warnings {
    eprintln!("{} {warning}", "warning:".yellow());
}
```

Adjust field names (`result.agents_installed`, `result.warnings`) to match what Task 15 actually surfaced.

**Step 4a: Re-install messaging**

There's no `install_agent_force` in this plan (deferred to follow-ups). When a user re-runs `install` against a plugin whose agents are already on disk, the command will fail with `AgentError::AlreadyInstalled` per agent. Make the CLI surface a single actionable line on that error rather than a raw error dump:

```rust
Err(Error::Agent(AgentError::AlreadyInstalled { name })) => {
    eprintln!(
        "{} agent `{name}` already installed; remove it manually \
         (`rm .kiro/agents/{name}.json .kiro/agents/prompts/{name}.md`) \
         and edit `.kiro/installed-agents.json` to re-install",
        "error:".red(),
    );
    std::process::exit(2);
}
```

This is a stop-gap until the follow-up adds proper `update`/`remove`/`force` commands.

**Step 5: Run tests to verify**

Run: `cargo test -p kiro-market`
Expected: PASS.

Also run a smoke install against a real plugin that ships agents to eyeball the output:

```bash
cargo run -- add <marketplace-url>
cargo run -- install <plugin-name>
```

**Step 6: Commit**

```bash
git add crates/kiro-market/src/commands/install.rs
git commit -m "feat: render installed agents and warnings in CLI install output"
```

---

### Task 17: Full-workspace verification

**Step 1: Run the complete test suite**

Run: `cargo test --workspace`
Expected: all tests PASS.

**Step 2: Run clippy**

Run: `cargo clippy --workspace -- -D warnings`
Expected: clean.

**Step 3: Manual smoke test**

In a scratch Kiro project:

```bash
cargo run -- add https://github.com/anthropics/claude-plugins-official
cargo run -- install pr-review-toolkit
ls .kiro/agents/
cat .kiro/agents/code-reviewer.json
cat .kiro/agents/prompts/code-reviewer.md
```

Verify:
- Each pr-review-toolkit agent has a matching `.json` + `prompts/*.md` pair.
- The `.json` files validate against `agent-schema.json`.
- The `prompt` field is `file://./prompts/{name}.md`.
- Warnings were printed for any source tools that could not be mapped.

**Step 4: Final commit (if any docs updated)**

If README or CLAUDE.md needs a note about agent support, update and commit:

```bash
git add README.md CLAUDE.md
git commit -m "docs: document agent import support"
```

---

## Post-implementation follow-ups (not part of this plan)

- Add `remove_agent` / `update_agent` / `install_agent_force` service methods paralleling the skill versions, then drop the manual-removal stop-gap from Task 16 Step 4a.
- Extend `list` command output to show installed agents.
- Validate every emitted JSON against `agent-schema.json` in tests (use `jsonschema` crate against the schema file in repo root). This is the most reliable defense against silent schema drift.
- Support awesome-copilot-style repos that are not Claude Code plugins (discovery at repo root instead of per-plugin).
- Honor `hooks:` frontmatter from source agents (currently dropped).
- Consider enabling the `preserve_order` feature on `serde_json` if reviewers complain about alphabetical key order in the emitted agent JSON.

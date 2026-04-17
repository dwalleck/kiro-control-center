//! Dialect-agnostic representation of an agent after parsing.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Which source dialect the agent came from. Used for applying
/// dialect-specific tool-mapping rules and for warnings.
///
/// Serializes to `"claude"` / `"copilot"` so it can live directly in the
/// installed-agents tracking file without a string sidecar.
///
/// Marked `#[non_exhaustive]` so adding a future dialect is not a breaking
/// change for external consumers, and so the tracking file's Deserialize
/// can be tolerantly extended later.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum AgentDialect {
    Claude,
    Copilot,
}

/// Agent definition normalized across Claude and Copilot source formats.
///
/// Constructed only by the parsers in this module. Fields are `pub` for
/// emitter and install-layer access, but callers should not mutate `name`
/// after construction without re-running name validation — the parsers
/// enforce path-safe naming up front so downstream fs operations can rely
/// on it.
#[derive(Debug, Clone)]
pub struct AgentDefinition {
    /// Short identifier used as the filename stem and Kiro agent `name` key.
    pub name: String,
    /// Optional human-readable blurb. Not shown to the model.
    pub description: Option<String>,
    /// Markdown body (everything after the closing YAML fence).
    pub prompt_body: String,
    /// Optional model ID (Claude only). Dropped for Copilot since its
    /// `model:` values are display names, not IDs.
    pub model: Option<String>,
    /// Raw tool identifiers from the source frontmatter (pre-mapping).
    pub source_tools: Vec<String>,
    /// MCP server entries as captured from Copilot `mcp-servers:` frontmatter.
    /// Serialized opaquely and passed through to Kiro's `mcpServers` field.
    pub mcp_servers: BTreeMap<String, serde_json::Value>,
    /// Which source dialect produced this definition.
    pub dialect: AgentDialect,
}

/// Structured reason a source agent file could not be parsed.
///
/// Replaces the pre-rendered `reason: String` that earlier versions carried
/// on `AgentError` and `InstallWarning`. Callers switch on variants
/// (e.g. to demote `MissingFrontmatter` to debug logs for README-style
/// files) rather than substring-matching on error text — which would
/// silently break the moment a message is reworded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[non_exhaustive]
pub enum ParseFailure {
    /// No opening `---` fence. Usually a README or other prose file
    /// accidentally scanned from the agents directory. Service layer
    /// demotes this to a debug log.
    MissingFrontmatter,
    /// Opening fence present but no closing fence — a broken file that
    /// the user probably wants to hear about.
    UnclosedFrontmatter,
    /// YAML parser rejected the frontmatter block.
    InvalidYaml(String),
    /// Frontmatter parsed but lacks the required `name` key.
    MissingName,
    /// Frontmatter `name` failed validation (unsafe for use as a filename).
    /// Carries the validator's human-readable reason.
    InvalidName(String),
    /// File read failed (permission denied, not found during racy delete,
    /// etc.). Carries the rendered I/O error message.
    IoError(String),
}

impl std::fmt::Display for ParseFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParseFailure::MissingFrontmatter => {
                f.write_str("missing opening `---` frontmatter fence")
            }
            ParseFailure::UnclosedFrontmatter => {
                f.write_str("unclosed frontmatter: missing closing `---` fence")
            }
            ParseFailure::InvalidYaml(msg) => write!(f, "invalid YAML: {msg}"),
            ParseFailure::MissingName => f.write_str("missing required `name` field"),
            ParseFailure::InvalidName(reason) => write!(f, "invalid `name` value: {reason}"),
            ParseFailure::IoError(msg) => write!(f, "read failed: {msg}"),
        }
    }
}

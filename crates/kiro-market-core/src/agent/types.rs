//! Dialect-agnostic representation of an agent after parsing.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

// ---------------------------------------------------------------------------
// MCP server config (typed)
// ---------------------------------------------------------------------------

/// Strongly-typed MCP server descriptor. Replaces an earlier
/// `serde_json::Value` "string bag" so:
///
/// 1. The downstream installer can reason about whether an agent will
///    spawn a subprocess (`Stdio`) vs. open a network connection
///    (`Http` / `Sse`) — the install gate uses this to decide whether
///    to require `--accept-mcp`.
/// 2. Malformed entries fail at the parse boundary instead of being
///    silently passed through to the on-disk JSON, where they would
///    only break when Kiro tried to execute them.
/// 3. The emitter no longer needs an opaque `serde_json::Value`-walking
///    normalize step — variants encode the wire format directly.
///
/// Wire format mirrors Kiro's `mcpServers` schema (the destination), with
/// `serde(alias = "local")` so Copilot's `type: local` keeps deserialising
/// (Copilot uses `local` for "spawn a subprocess"; Kiro calls the same
/// thing `stdio`, and we normalise to the latter on the way in).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum McpServerConfig {
    /// Subprocess transport: `command` is exec'd with `args`. Anything
    /// here can run arbitrary code on the host, which is why install
    /// requires `--accept-mcp` to write a Stdio-bearing agent into the
    /// project unattended.
    #[serde(alias = "local")]
    Stdio {
        command: String,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default)]
        env: BTreeMap<String, String>,
    },
    /// HTTP transport. Less risky than Stdio (no command execution) but
    /// the install gate still flags it because the URL points at a
    /// third party that the user may not have seen.
    Http {
        url: String,
        #[serde(default)]
        headers: BTreeMap<String, String>,
    },
    /// Server-Sent Events transport. Same risk profile as `Http` for
    /// install-time review.
    Sse { url: String },
}

impl McpServerConfig {
    /// Whether this entry would spawn a subprocess on the user's host.
    /// The install gate treats Stdio as the most-sensitive class.
    #[must_use]
    pub fn is_stdio(&self) -> bool {
        matches!(self, Self::Stdio { .. })
    }

    /// Short human-readable label for the transport, used in install-time
    /// warnings ("agent X brings 2 stdio servers, 1 http server").
    #[must_use]
    pub fn transport_label(&self) -> &'static str {
        match self {
            Self::Stdio { .. } => "stdio",
            Self::Http { .. } => "http",
            Self::Sse { .. } => "sse",
        }
    }
}

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
    /// Plugin authored in Kiro's native JSON format. Installed via
    /// validate-and-copy (no parse-and-translate).
    Native,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_dialect_native_serializes_to_native() {
        let json = serde_json::to_string(&AgentDialect::Native).expect("serialize");
        assert_eq!(json, "\"native\"");
        let round: AgentDialect = serde_json::from_str("\"native\"").expect("deserialize");
        assert_eq!(round, AgentDialect::Native);
    }
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
    /// Typed via [`McpServerConfig`] so the installer can gate execution
    /// risk (Stdio servers run subprocesses; Http/Sse open network
    /// connections) and so malformed entries fail at parse time.
    pub mcp_servers: BTreeMap<String, McpServerConfig>,
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

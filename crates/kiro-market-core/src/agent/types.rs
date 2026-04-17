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

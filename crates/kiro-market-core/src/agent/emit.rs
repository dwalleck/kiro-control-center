//! Emit Kiro agent JSON from an [`AgentDefinition`].
//!
//! The emitted JSON matches the schema at `agent-schema.json`. The `prompt`
//! field uses Kiro's `file://` URI to reference the externalized prompt
//! markdown, so the `.md` remains the source of truth after install.
//!
//! Routing: native Kiro tool names go into `allowedTools` (explicit
//! allowlist); MCP server refs go into `tools` (the field documented as
//! carrying `@server` / `@server/tool` entries per `agent-schema.json`).
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
/// `mapped_tools` is the output of [`super::tools::map_claude_tools`] or
/// [`super::tools::map_copilot_tools`]. Native entries land in `allowedTools`;
/// MCP refs land in `tools` per the Kiro agent schema.
///
/// # Errors
///
/// Currently infallible in practice (all inputs are already validated
/// upstream) but returns `serde_json::Result` so future extensions can
/// surface serialization failures.
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
        Value::String(format!(
            "file://./prompts/{}.md",
            percent_encode_path_segment(&def.name)
        )),
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

/// Percent-encode characters that are invalid in a URI path segment.
///
/// `validate_name` already forbids `/`, `\`, `..`, `.`, and empty names,
/// so the only characters that can appear here and break RFC 3986 are
/// whitespace and other non-unreserved printables (Copilot agents like
/// "Terraform Agent" contain spaces). Unreserved per RFC 3986: ASCII
/// alphanumeric, `-`, `_`, `.`, `~`. Everything else is percent-encoded.
fn percent_encode_path_segment(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for byte in s.bytes() {
        let unreserved = byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~');
        if unreserved {
            out.push(byte as char);
        } else {
            use std::fmt::Write as _;
            let _ = write!(out, "%{byte:02X}");
        }
    }
    out
}

/// Normalize a Copilot MCP server entry toward Kiro's `CustomToolConfig` shape.
///
/// Rules:
/// - `type: "local"` → `type: "stdio"` (Copilot's `local` means "spawn
///   this as a subprocess"; Kiro's equivalent vocabulary is `stdio`).
/// - inner `tools` allowlist → dropped. Kiro has no per-server allowlist
///   field on the server config itself; the outer `tools` array already
///   handles whole-server access via `@name` references.
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
    fn emit_percent_encodes_spaces_in_prompt_uri() {
        // Copilot agents like "Terraform Agent" have spaces in the name.
        // The JSON `name` field keeps the space; the file:// URI must
        // percent-encode it per RFC 3986.
        let mut def = sample_claude_def();
        def.name = "Terraform Agent".into();
        let out = build_kiro_json(&def, &[]).unwrap();
        assert_eq!(out["name"], "Terraform Agent");
        assert_eq!(out["prompt"], "file://./prompts/Terraform%20Agent.md");
    }

    #[test]
    fn emit_preserves_unreserved_chars_in_prompt_uri() {
        let mut def = sample_claude_def();
        def.name = "pr-review_toolkit.v2~beta".into();
        let out = build_kiro_json(&def, &[]).unwrap();
        // Unreserved chars per RFC 3986: alphanumeric and -_.~ — no encoding.
        assert_eq!(
            out["prompt"],
            "file://./prompts/pr-review_toolkit.v2~beta.md"
        );
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
        assert_eq!(
            allowed,
            &vec![
                serde_json::Value::String("read".into()),
                serde_json::Value::String("shell".into()),
            ]
        );
        // Native names must NOT leak into `tools` (which is for MCP refs).
        assert!(
            out.get("tools").is_none(),
            "no MCP refs in this list — `tools` field must be omitted"
        );
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
        assert!(
            out.get("allowedTools").is_none(),
            "no native names — `allowedTools` field must be omitted"
        );
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
        assert!(
            out.get("tools").is_none(),
            "empty tool list must omit both arrays so Kiro inherits full parent toolset"
        );
    }

    #[test]
    fn emit_normalizes_mcp_server_type_local_to_stdio() {
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

    #[test]
    fn emit_preserves_non_local_mcp_server_type() {
        let mut def = sample_claude_def();
        def.mcp_servers.insert(
            "hosted".into(),
            serde_json::json!({
                "type": "http",
                "url": "https://example.com/mcp",
            }),
        );
        let out = build_kiro_json(&def, &[]).unwrap();
        assert_eq!(out["mcpServers"]["hosted"]["type"], "http");
        assert_eq!(
            out["mcpServers"]["hosted"]["url"],
            "https://example.com/mcp"
        );
    }
}

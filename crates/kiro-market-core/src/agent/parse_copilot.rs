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
use super::types::{AgentDefinition, AgentDialect, ParseFailure};

#[derive(Debug, Deserialize)]
struct CopilotFrontmatter {
    name: Option<String>,
    description: Option<String>,
    #[serde(default)]
    tools: Vec<String>,
    #[serde(rename = "mcp-servers", default)]
    mcp_servers: BTreeMap<String, serde_json::Value>,
    // `model` accepted and ignored: Copilot uses display names ("Claude
    // Sonnet 4") with no reliable mapping to Kiro model IDs. We take it
    // as Option<String> rather than `#[serde(deny_unknown_fields)]` so
    // real-world Copilot files continue to parse.
    #[allow(dead_code)]
    model: Option<String>,
}

/// Parse a Copilot-style `.agent.md` file into an `AgentDefinition`.
///
/// # Errors
///
/// Returns a [`ParseFailure`] variant describing which stage of parsing
/// failed. The caller (`parse::parse_agent_file`) attaches the source
/// path and lifts into `AgentError::ParseFailed`.
pub fn parse_copilot_agent(content: &str) -> Result<AgentDefinition, ParseFailure> {
    let (yaml, body) = split_frontmatter(content)?;
    let fm: CopilotFrontmatter =
        serde_yaml::from_str(yaml).map_err(|e| ParseFailure::InvalidYaml(e.to_string()))?;

    let name = fm.name.ok_or(ParseFailure::MissingName)?;

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

    #[test]
    fn parse_dialect_set_to_copilot() {
        let def = parse_copilot_agent(TERRAFORM).expect("parse");
        assert_eq!(def.dialect, AgentDialect::Copilot);
    }
}

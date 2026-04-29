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
use super::types::{AgentDefinition, AgentDialect, McpServerConfig, ParseFailure};

#[derive(Debug, Deserialize)]
struct CopilotFrontmatter {
    name: Option<String>,
    description: Option<String>,
    #[serde(default)]
    tools: Vec<String>,
    /// MCP server map. Captured at parse-time as
    /// [`super::types::McpServerConfig`] so:
    ///
    /// - typos in the discriminator (`type: lokal` instead of `local`)
    ///   fail here instead of breaking the agent at runtime;
    /// - the installer can decide opt-in policy (`--accept-mcp`) based
    ///   on transport variant rather than peeking inside an opaque
    ///   JSON value.
    ///
    /// Copilot allows an inner `tools: ["*"]` allowlist on each server
    /// entry; that field is silently ignored on the deserializer side
    /// (no field captures it) because Kiro has no equivalent.
    #[serde(rename = "mcp-servers", default)]
    mcp_servers: BTreeMap<String, McpServerConfig>,
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
        serde_yaml_ng::from_str(yaml).map_err(|e| ParseFailure::InvalidYaml {
            reason: e.to_string(),
        })?;

    let name = fm.name.ok_or(ParseFailure::MissingName)?;
    // Validate the name at parse time so downstream fs operations (and the
    // file:// URI in the emitted JSON) can trust it without re-checking.
    crate::validation::validate_name(&name).map_err(|e| match e {
        // See parse_claude.rs for rationale on the explicit two-arm form
        // — same CLAUDE.md classifier-exhaustiveness discipline.
        crate::error::ValidationError::InvalidName { reason, .. }
        | crate::error::ValidationError::InvalidRelativePath { reason, .. } => {
            ParseFailure::InvalidName { reason }
        }
    })?;

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
    fn parse_captures_mcp_servers_as_typed_stdio() {
        // The Copilot fixture's `type: 'local'` is accepted via
        // `serde(alias = "local")` on the typed Stdio variant. Anything
        // that doesn't fit the typed schema fails the parse, so we know
        // by construction here that `command` is a string and `args` is
        // a Vec<String> — no `serde_json::Value` indexing.
        use crate::agent::types::McpServerConfig;
        let def = parse_copilot_agent(TERRAFORM).expect("parse");
        assert_eq!(def.mcp_servers.len(), 1);
        let tf = def.mcp_servers.get("terraform").expect("terraform entry");
        match tf {
            McpServerConfig::Stdio { command, args, .. } => {
                assert_eq!(command, "docker");
                assert_eq!(
                    args,
                    &vec![
                        "run".to_string(),
                        "-i".to_string(),
                        "hashicorp/terraform-mcp-server:latest".to_string()
                    ]
                );
            }
            other => panic!("expected Stdio variant, got {other:?}"),
        }
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

    #[test]
    fn parse_rejects_unknown_mcp_server_type_at_parse_time() {
        // Whole point of the typed McpServerConfig schema is that
        // typos in the discriminator fail HERE, not later when Kiro
        // tries to spawn the server. Without this test, the contract
        // claim in agent/types.rs is unverified — a regression that
        // accepts any `serde_json::Value` would pass silently.
        let src = "---\n\
                   name: bad\n\
                   description: x\n\
                   mcp-servers:\n  \
                     server1:\n    \
                       type: 'lokal'\n    \
                       command: docker\n\
                   ---\n\
                   body\n";
        let err = parse_copilot_agent(src).expect_err("unknown discriminator must be rejected");
        assert!(
            matches!(err, ParseFailure::InvalidYaml { .. }),
            "expected InvalidYaml for unknown MCP type, got {err:?}"
        );
    }

    #[test]
    fn parse_rejects_stdio_mcp_server_missing_command_field() {
        // Stdio variant requires `command`. A missing field must fail
        // at parse, not at install — proves the typed schema actually
        // enforces required fields per variant.
        let src = "---\n\
                   name: bad\n\
                   mcp-servers:\n  \
                     server1:\n    \
                       type: stdio\n\
                   ---\n\
                   body\n";
        let err = parse_copilot_agent(src).expect_err("missing command must be rejected");
        assert!(
            matches!(err, ParseFailure::InvalidYaml { .. }),
            "expected InvalidYaml for missing command, got {err:?}"
        );
    }

    #[test]
    fn parse_rejects_http_mcp_server_missing_url_field() {
        let src = "---\n\
                   name: bad\n\
                   mcp-servers:\n  \
                     server1:\n    \
                       type: http\n\
                   ---\n\
                   body\n";
        let err = parse_copilot_agent(src).expect_err("missing url must be rejected");
        assert!(matches!(err, ParseFailure::InvalidYaml { .. }));
    }
}

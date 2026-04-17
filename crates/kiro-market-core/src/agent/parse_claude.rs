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
    let fm: ClaudeFrontmatter =
        serde_yaml::from_str(yaml_block).map_err(|e| format!("invalid YAML: {e}"))?;

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
        assert!(
            def.model.is_none(),
            "model: inherit should be normalized to None"
        );
    }

    #[test]
    fn parse_missing_name_errors() {
        let src = "---\ndescription: x\n---\nbody\n";
        let err = parse_claude_agent(src).unwrap_err();
        assert!(err.contains("name"));
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

    #[test]
    fn parse_dialect_set_to_claude() {
        let def = parse_claude_agent(SAMPLE).expect("parse");
        assert_eq!(def.dialect, AgentDialect::Claude);
    }
}

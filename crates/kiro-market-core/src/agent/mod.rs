//! Agent import: parse Claude- and Copilot-style agent markdown files,
//! map their tools to Kiro identifiers, and emit Kiro agent JSON.

pub mod discover;
pub mod emit;
mod frontmatter;
mod parse;
mod parse_claude;
mod parse_copilot;
pub mod tools;
pub mod types;

pub use parse::{detect_dialect, parse_agent_file};
pub use parse_claude::parse_claude_agent;
pub use parse_copilot::parse_copilot_agent;
pub use types::{AgentDefinition, AgentDialect};

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

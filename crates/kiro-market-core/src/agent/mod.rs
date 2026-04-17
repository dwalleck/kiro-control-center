//! Agent import: parse Claude- and Copilot-style agent markdown files,
//! map their tools to Kiro identifiers, and emit Kiro agent JSON.

mod frontmatter;
mod parse_claude;
pub mod types;

pub use parse_claude::parse_claude_agent;
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

//! Source-to-Kiro tool name mapping.
//!
//! Tools land in two different fields of the emitted agent JSON:
//! - native tool names (`read`, `shell`, etc.) → `allowedTools`
//! - MCP server references (`@server`, `@server/tool`) → `tools`
//!
//! The mapper returns a typed [`MappedTool`] so the emitter can route each
//! result to the correct field without re-parsing strings. Unmapped source
//! tools are returned structurally (not as pre-rendered messages) so callers
//! can re-render them as `InstallWarning` variants without string surgery.
//!
//! Kiro tool names are verified against
//! <https://kiro.dev/docs/cli/reference/built-in-tools/>.

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
        let (mapped, unmapped) =
            map_claude_tools(&["Read".into(), "NotebookEdit".into(), "Skill".into()]);
        assert_eq!(mapped, vec![MappedTool::Native("read".into())]);
        assert_eq!(
            unmapped,
            vec![
                UnmappedTool {
                    source: "NotebookEdit".into(),
                    reason: UnmappedReason::NoKiroEquivalent
                },
                UnmappedTool {
                    source: "Skill".into(),
                    reason: UnmappedReason::NoKiroEquivalent
                },
            ]
        );
    }

    #[test]
    fn map_claude_tools_dedupes_write_from_edit_and_write() {
        let (mapped, _) = map_claude_tools(&["Edit".into(), "Write".into()]);
        assert_eq!(mapped, vec![MappedTool::Native("write".into())]);
    }

    #[test]
    fn map_claude_tools_preserves_input_order() {
        let (mapped, _) = map_claude_tools(&["Bash".into(), "Read".into()]);
        assert_eq!(
            mapped,
            vec![
                MappedTool::Native("shell".into()),
                MappedTool::Native("read".into()),
            ]
        );
    }
}

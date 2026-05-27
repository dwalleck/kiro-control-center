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
//! <https://kiro.dev/docs/cli/reference/built-in-tools/>
//! (retrieved 2026-04-16). Update this comment with a new retrieval date
//! whenever the table below is re-validated.

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
    /// Claude `PascalCase` name with no Kiro equivalent (e.g. `NotebookEdit`).
    NoKiroEquivalent,
    /// Copilot bare name (e.g. `codebase`, `findTestFiles`) — internal Copilot
    /// concept with no reliable Kiro mapping.
    BareCopilotName,
}

/// Look up the Kiro tool name for a Claude-style `PascalCase` tool name.
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

/// Copilot bare tool alias → Kiro native tool name(s). Each group's first
/// element is the canonical Copilot alias; alternates (`shell` vs. `bash`
/// vs. `powershell`) all map to the same Kiro tools. Comparison is
/// case-insensitive (`Read` and `read` both resolve), but alternates are
/// listed exactly as they appear in the Copilot docs so a future audit
/// against the upstream table is straightforward.
///
/// Internal Copilot concepts with no reliable Kiro equivalent
/// (`codebase`, `findTestFiles`, `usages`, `problems`, `testFailure`,
/// `runCommands`, `runTasks`, `editFiles`, …) are intentionally absent
/// — `map_copilot_tools` reports them as [`UnmappedReason::BareCopilotName`]
/// so the user sees them in the install output and can choose to restrict
/// the emitted agent manually.
///
/// Reference: <https://docs.github.com/en/copilot/reference/custom-agents-configuration#tool-aliases>
/// (retrieved 2026-05-15).
const COPILOT_BARE_TOOL_GROUPS: &[(&[&str], &[&str])] = &[
    (&["execute", "shell", "bash", "powershell"], &["shell"]),
    (&["read", "notebookread"], &["read"]),
    (&["edit", "multiedit", "write", "notebookedit"], &["write"]),
    (&["search", "grep", "glob"], &["grep", "glob"]),
    (&["agent", "custom-agent", "task"], &["subagent"]),
    (
        &["web", "websearch", "webfetch"],
        &["web_fetch", "web_search"],
    ),
    (&["todo", "todowrite"], &["todo"]),
];

/// Look up the Kiro tool name(s) for a Copilot-style bare tool alias.
///
/// Returns `None` for names with no Kiro equivalent. See
/// [`COPILOT_BARE_TOOL_GROUPS`] for the full alias table and the
/// intentional-unmapped policy.
#[must_use]
fn map_copilot_bare_tool(name: &str) -> Option<&'static [&'static str]> {
    COPILOT_BARE_TOOL_GROUPS
        .iter()
        .find(|(aliases, _)| aliases.iter().any(|a| a.eq_ignore_ascii_case(name)))
        .map(|(_, kiro)| *kiro)
}

/// Map a list of Copilot source tool names to Kiro identifiers.
///
/// Copilot tools use mixed conventions:
/// - `{server}/*` → [`MappedTool::McpRef("@{server}")`] (whole-server access)
/// - `{server}/{tool}` → [`MappedTool::McpRef("@{server}/{tool}")`] (specific MCP tool)
/// - known bare aliases (`read`, `edit`, `search`, `shell`, …) → native Kiro tools
/// - unknown bare names (`codebase`, `findTestFiles`, …) → unmapped ([`UnmappedReason::BareCopilotName`])
///
/// Unknown bare names are dropped *intentionally*: they represent internal
/// Copilot concepts (IDE problem-pane, test-runner integrations) with no
/// reliable Kiro equivalent. Users see them in the install output and can
/// restrict the emitted agent manually if desired.
///
/// Reference: <https://docs.github.com/en/copilot/reference/custom-agents-configuration#tool-aliases>
/// (retrieved 2026-05-15).
#[must_use]
pub fn map_copilot_tools(source: &[String]) -> (Vec<MappedTool>, Vec<UnmappedTool>) {
    let mut mapped: Vec<MappedTool> = Vec::new();
    let mut unmapped: Vec<UnmappedTool> = Vec::new();
    for tool in source {
        if let Some((server, rest)) = tool.split_once('/') {
            let kiro = if rest == "*" {
                format!("@{server}")
            } else {
                format!("@{server}/{rest}")
            };
            let entry = MappedTool::McpRef(kiro);
            if !mapped.contains(&entry) {
                mapped.push(entry);
            }
        } else if let Some(kiro_names) = map_copilot_bare_tool(tool) {
            for &kiro in kiro_names {
                let already_present = mapped
                    .iter()
                    .any(|m| matches!(m, MappedTool::Native(s) if s == kiro));
                if !already_present {
                    mapped.push(MappedTool::Native(kiro.to_string()));
                }
            }
        } else {
            unmapped.push(UnmappedTool {
                source: tool.clone(),
                reason: UnmappedReason::BareCopilotName,
            });
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

    #[test]
    fn copilot_mcp_wildcard_maps_to_kiro_server_ref() {
        let (mapped, unmapped) =
            map_copilot_tools(&["terraform/*".into(), "playwright/click".into()]);
        assert_eq!(
            mapped,
            vec![
                MappedTool::McpRef("@terraform".into()),
                MappedTool::McpRef("@playwright/click".into()),
            ]
        );
        assert!(unmapped.is_empty());
    }

    #[test]
    fn copilot_bare_names_drop_with_structured_reason() {
        let (mapped, unmapped) =
            map_copilot_tools(&["codebase".into(), "findTestFiles".into(), "problems".into()]);
        assert!(mapped.is_empty());
        assert_eq!(unmapped.len(), 3);
        assert!(
            unmapped
                .iter()
                .all(|u| u.reason == UnmappedReason::BareCopilotName)
        );
        assert_eq!(unmapped[0].source, "codebase");
    }

    #[test]
    fn copilot_mixed_list_preserves_mcp_refs_drops_bare() {
        let (mapped, unmapped) = map_copilot_tools(&[
            "edit/editFiles".into(),
            "terraform/*".into(),
            "codebase".into(),
        ]);
        assert!(mapped.contains(&MappedTool::McpRef("@edit/editFiles".into())));
        assert!(mapped.contains(&MappedTool::McpRef("@terraform".into())));
        assert_eq!(unmapped.len(), 1);
        assert_eq!(unmapped[0].source, "codebase");
    }

    #[test]
    fn claude_dedupes_edit_and_write_across_mixed_input() {
        // Input order: Edit, Read, Write, Edit — should dedupe to write+read.
        let (mapped, _) =
            map_claude_tools(&["Edit".into(), "Read".into(), "Write".into(), "Edit".into()]);
        assert_eq!(
            mapped,
            vec![
                MappedTool::Native("write".into()),
                MappedTool::Native("read".into()),
            ]
        );
    }

    #[test]
    fn copilot_dedupes_mixed_refs_and_wildcards() {
        let (mapped, _) = map_copilot_tools(&[
            "terraform/*".into(),
            "playwright/click".into(),
            "terraform/*".into(),
            "playwright/click".into(),
        ]);
        assert_eq!(
            mapped,
            vec![
                MappedTool::McpRef("@terraform".into()),
                MappedTool::McpRef("@playwright/click".into()),
            ]
        );
    }

    #[test]
    fn copilot_dedupes_repeated_refs() {
        let (mapped, _) = map_copilot_tools(&["terraform/*".into(), "terraform/*".into()]);
        assert_eq!(mapped, vec![MappedTool::McpRef("@terraform".into())]);
    }

    #[test]
    fn copilot_bare_aliases_map_to_native_tools() {
        let (mapped, unmapped) = map_copilot_tools(&[
            "read".into(),
            "edit".into(),
            "shell".into(),
            "search".into(),
        ]);
        assert!(unmapped.is_empty());
        assert_eq!(
            mapped,
            vec![
                MappedTool::Native("read".into()),
                MappedTool::Native("write".into()),
                MappedTool::Native("shell".into()),
                MappedTool::Native("grep".into()),
                MappedTool::Native("glob".into()),
            ]
        );
    }

    #[test]
    fn copilot_bare_aliases_case_insensitive() {
        let (mapped, _) = map_copilot_tools(&["Read".into(), "SHELL".into()]);
        assert!(mapped.contains(&MappedTool::Native("read".into())));
        assert!(mapped.contains(&MappedTool::Native("shell".into())));
    }

    #[test]
    fn copilot_mixed_bare_mcp_and_unknown() {
        let (mapped, unmapped) =
            map_copilot_tools(&["read".into(), "terraform/*".into(), "codebase".into()]);
        assert_eq!(
            mapped,
            vec![
                MappedTool::Native("read".into()),
                MappedTool::McpRef("@terraform".into()),
            ]
        );
        assert_eq!(unmapped.len(), 1);
        assert_eq!(unmapped[0].source, "codebase");
    }

    #[test]
    fn copilot_bare_web_alias_expands_to_two_native_tools() {
        let (mapped, unmapped) = map_copilot_tools(&["web".into()]);
        assert!(unmapped.is_empty());
        assert_eq!(
            mapped,
            vec![
                MappedTool::Native("web_fetch".into()),
                MappedTool::Native("web_search".into()),
            ]
        );
    }

    #[test]
    fn copilot_bare_agent_alias_maps_to_subagent() {
        let (mapped, unmapped) =
            map_copilot_tools(&["agent".into(), "custom-agent".into(), "task".into()]);
        assert!(unmapped.is_empty());
        assert_eq!(mapped, vec![MappedTool::Native("subagent".into())]);
    }

    #[test]
    fn copilot_bare_todo_alias_maps_to_todo() {
        let (mapped, unmapped) = map_copilot_tools(&["todo".into(), "TodoWrite".into()]);
        assert!(unmapped.is_empty());
        assert_eq!(mapped, vec![MappedTool::Native("todo".into())]);
    }

    #[test]
    fn copilot_cross_alias_dedupes_to_same_native() {
        // `shell`, `bash`, `execute`, and `powershell` all map to Kiro's
        // `shell`; the result must contain a single entry.
        let (mapped, _) = map_copilot_tools(&[
            "shell".into(),
            "bash".into(),
            "execute".into(),
            "powershell".into(),
        ]);
        assert_eq!(mapped, vec![MappedTool::Native("shell".into())]);
    }

    #[test]
    fn copilot_bare_search_and_grep_dedupe_via_grep_glob_pair() {
        // `search`, `grep`, and `glob` all map to the (grep, glob) pair.
        // Mixed input must still produce exactly that pair without dupes.
        let (mapped, _) = map_copilot_tools(&["search".into(), "grep".into(), "glob".into()]);
        assert_eq!(
            mapped,
            vec![
                MappedTool::Native("grep".into()),
                MappedTool::Native("glob".into()),
            ]
        );
    }
}

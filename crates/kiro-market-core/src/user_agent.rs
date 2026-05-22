//! User-authored agent surface for the kiro-control-center
//! "Workflows > Agents" view.
//!
//! Distinct from the marketplace-install path
//! ([`crate::agent::parse_native`]): this module carries the
//! list/save/delete/duplicate wire shapes the UI consumes. The list
//! payload is computed in [`crate::project::KiroProject::list_user_agents`]
//! via untyped JSON (`serde_json::Value`) — not the strict native-agent
//! parser whose symlink/hardlink/byte-cap checks are appropriate for
//! install-time copy-in, not display-time projection. See design claim
//! C2 in `.agents-view/design-slice-1.md`.

use serde::Serialize;

/// One row of the Agents list-page payload. Serialized as the response
/// of the `list_user_agents` Tauri command. See spec behavior B1.
#[derive(Clone, Debug, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[non_exhaustive]
pub struct UserAgentRow {
    /// Agent identity. Sourced from the JSON file's `name` field when
    /// present, else falls back to the filename stem (spec decision
    /// #14). Save path enforces these always match; list path is
    /// tolerant of pre-existing drift.
    pub name: String,
    /// Human-only label; not shown to the model.
    pub description: Option<String>,
    /// Model ID override; `None` means "use Kiro's default."
    pub model: Option<String>,
    /// Number of entries in the JSON's `tools` array.
    pub tools_count: usize,
    /// Number of entries in the JSON's `mcpServers` object.
    pub mcp_count: usize,
    /// Number of entries in the JSON's `resources` array.
    pub resources_count: usize,
    /// Sum of array lengths across the JSON's `hooks` object values.
    pub hooks_count: usize,
    /// Marketplace lineage badge data. `Some` iff the agent's name is
    /// a key in `installed-agents.json#/agents`.
    pub lineage: Option<UserAgentLineage>,
}

/// Marketplace lineage projected from
/// [`crate::project::InstalledAgentMeta`] for display.
#[derive(Clone, Debug, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[non_exhaustive]
pub struct UserAgentLineage {
    pub marketplace: String,
    pub plugin: String,
    pub version: Option<String>,
}

impl UserAgentRow {
    /// Construct a row for an untracked (user-authored) agent.
    /// Tests and the list builder use this to keep field discipline.
    #[must_use]
    pub fn user_authored(
        name: String,
        description: Option<String>,
        model: Option<String>,
        tools_count: usize,
        mcp_count: usize,
        resources_count: usize,
        hooks_count: usize,
    ) -> Self {
        Self {
            name,
            description,
            model,
            tools_count,
            mcp_count,
            resources_count,
            hooks_count,
            lineage: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// S2 stress fixture: round-trip a fully-populated row through serde.
    /// The Unicode name + counts at `usize` boundary values are designed
    /// to fail if a future contributor "tightens" the types to `u32` or
    /// to a non-Unicode-safe string. The Option fields are exercised in
    /// both `Some` and `None` shapes (lineage `Some`; description/model/
    /// version each `None`) so the field-presence matrix gets coverage.
    #[test]
    fn user_agent_row_serializes_to_expected_wire_shape() {
        let row = UserAgentRow {
            name: "agent-with-üñîçødé".to_string(),
            description: None,
            model: None,
            tools_count: usize::MAX,
            mcp_count: 0,
            resources_count: 1,
            hooks_count: 42,
            lineage: Some(UserAgentLineage {
                marketplace: "m".to_string(),
                plugin: "p".to_string(),
                version: Some("0.0.0-pre".to_string()),
            }),
        };

        let value: serde_json::Value = serde_json::to_value(&row).expect("serialize to value");
        let obj = value.as_object().expect("row is a JSON object");

        // Field set: every documented key is present.
        for key in [
            "name",
            "description",
            "model",
            "tools_count",
            "mcp_count",
            "resources_count",
            "hooks_count",
            "lineage",
        ] {
            assert!(obj.contains_key(key), "wire shape missing key: {key}");
        }
        // No extra keys (catches a future contributor accidentally adding
        // a public field without updating bindings.ts consumers).
        assert_eq!(obj.len(), 8, "wire shape has unexpected extra fields");

        // Unicode name survives.
        assert_eq!(obj["name"], serde_json::json!("agent-with-üñîçødé"));
        // Option<String>::None serializes as JSON null.
        assert!(obj["description"].is_null());
        assert!(obj["model"].is_null());
        // Counts round-trip at usize::MAX (catches a downgrade to u32).
        assert_eq!(obj["tools_count"], serde_json::json!(usize::MAX));
        // Nested lineage object shape.
        let lin = obj["lineage"].as_object().expect("lineage is object");
        assert_eq!(lin["marketplace"], serde_json::json!("m"));
        assert_eq!(lin["plugin"], serde_json::json!("p"));
        assert_eq!(lin["version"], serde_json::json!("0.0.0-pre"));
    }

    #[test]
    fn user_agent_row_with_none_lineage_serializes_null() {
        let row = UserAgentRow::user_authored(
            "plain".to_string(),
            Some("desc".to_string()),
            Some("claude-sonnet-4-6".to_string()),
            2,
            0,
            0,
            0,
        );
        let value: serde_json::Value = serde_json::to_value(&row).expect("serialize to value");
        assert!(value["lineage"].is_null());
        assert_eq!(value["description"], serde_json::json!("desc"));
    }
}

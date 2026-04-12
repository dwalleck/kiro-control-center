//! Kiro CLI settings registry types and definitions.

use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

// ---------------------------------------------------------------------------
// Setting category
// ---------------------------------------------------------------------------

/// Top-level category for a Kiro CLI setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum SettingCategory {
    Telemetry,
    Chat,
    Knowledge,
    KeyBindings,
    Features,
    Api,
    Mcp,
    Environment,
}

impl SettingCategory {
    /// Human-readable display name for this category.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Telemetry => "Telemetry",
            Self::Chat => "Chat",
            Self::Knowledge => "Knowledge",
            Self::KeyBindings => "Key Bindings",
            Self::Features => "Features",
            Self::Api => "API",
            Self::Mcp => "MCP",
            Self::Environment => "Environment",
        }
    }
}

// ---------------------------------------------------------------------------
// Setting value type (internal)
// ---------------------------------------------------------------------------

/// Describes what kind of value a setting holds.
#[derive(Debug, Clone)]
pub enum SettingType {
    Bool,
    String,
    Number,
    Char,
    StringArray,
    Enum(Vec<&'static str>),
}

impl SettingType {
    /// Returns the wire-format type name used in [`SettingEntry::value_type`].
    fn type_name(&self) -> &'static str {
        match self {
            Self::Bool => "bool",
            Self::String => "string",
            Self::Number => "number",
            Self::Char => "char",
            Self::StringArray => "string_array",
            Self::Enum(_) => "enum",
        }
    }

    /// Returns enum options if this is an `Enum` variant, otherwise `None`.
    fn enum_options(&self) -> Option<Vec<String>> {
        match self {
            Self::Enum(opts) => Some(opts.iter().map(|s| (*s).to_owned()).collect()),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Setting definition (internal registry entry)
// ---------------------------------------------------------------------------

/// Internal definition of a single Kiro CLI setting.
pub struct SettingDef {
    /// Dotted JSON key path (e.g. `"chat.defaultModel"`).
    pub key: &'static str,
    /// Short human-readable label.
    pub label: &'static str,
    /// Longer description shown in the settings UI.
    pub description: &'static str,
    /// Logical grouping category.
    pub category: SettingCategory,
    /// What kind of value this setting holds.
    pub value_type: SettingType,
    /// Default value as a JSON value.
    pub default: JsonValue,
}

// ---------------------------------------------------------------------------
// Setting entry (serialisable, frontend-facing)
// ---------------------------------------------------------------------------

/// A fully-resolved setting entry suitable for serialisation to a frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct SettingEntry {
    /// Dotted JSON key path.
    pub key: String,
    /// Short human-readable label.
    pub label: String,
    /// Longer description.
    pub description: String,
    /// Machine-readable category identifier.
    pub category: SettingCategory,
    /// Human-readable category label.
    pub category_label: String,
    /// Wire-format type name (`"bool"`, `"string"`, `"enum"`, …).
    pub value_type: String,
    /// For `Enum` settings: the allowed values.  `None` for all other types.
    pub enum_options: Option<Vec<String>>,
    /// Default value as a JSON value.
    pub default_value: JsonValue,
    /// Current value as stored in the user's settings file.
    pub current_value: JsonValue,
}

// ---------------------------------------------------------------------------
// Registry
// ---------------------------------------------------------------------------

/// Returns the complete list of known Kiro CLI settings definitions.
///
/// The returned `Vec` is ordered: categories are grouped and within each
/// category settings appear in a logical sequence.
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn registry() -> Vec<SettingDef> {
    vec![
        // ----------------------------------------------------------------
        // Telemetry
        // ----------------------------------------------------------------
        SettingDef {
            key: "telemetry.enabled",
            label: "Enable Telemetry",
            description: "Send anonymous usage data to help improve Kiro.",
            category: SettingCategory::Telemetry,
            value_type: SettingType::Bool,
            default: JsonValue::Bool(true),
        },
        SettingDef {
            key: "telemetry.shareCodeSnippets",
            label: "Share Code Snippets",
            description: "Allow telemetry to include short code snippets for context.",
            category: SettingCategory::Telemetry,
            value_type: SettingType::Bool,
            default: JsonValue::Bool(false),
        },
        // ----------------------------------------------------------------
        // Chat
        // ----------------------------------------------------------------
        SettingDef {
            key: "chat.defaultModel",
            label: "Default Model",
            description: "The AI model used by default for chat sessions.",
            category: SettingCategory::Chat,
            value_type: SettingType::Enum(vec![
                "claude-opus-4-5",
                "claude-sonnet-4-5",
                "claude-haiku-4-5",
                "claude-opus-4",
                "claude-sonnet-4",
                "claude-haiku-3",
            ]),
            default: JsonValue::String("claude-sonnet-4-5".to_owned()),
        },
        SettingDef {
            key: "chat.temperature",
            label: "Temperature",
            description: "Sampling temperature for model responses (0.0\u{2013}1.0).",
            category: SettingCategory::Chat,
            value_type: SettingType::Number,
            default: serde_json::json!(0.7),
        },
        SettingDef {
            key: "chat.maxTokens",
            label: "Max Tokens",
            description: "Maximum number of tokens the model may generate per response.",
            category: SettingCategory::Chat,
            value_type: SettingType::Number,
            default: serde_json::json!(8096),
        },
        SettingDef {
            key: "chat.streamResponses",
            label: "Stream Responses",
            description: "Stream model output token-by-token as it is generated.",
            category: SettingCategory::Chat,
            value_type: SettingType::Bool,
            default: JsonValue::Bool(true),
        },
        SettingDef {
            key: "chat.autoSave",
            label: "Auto-Save Conversations",
            description: "Automatically persist chat sessions to disk.",
            category: SettingCategory::Chat,
            value_type: SettingType::Bool,
            default: JsonValue::Bool(true),
        },
        SettingDef {
            key: "chat.contextWindow",
            label: "Context Window",
            description: "Number of previous messages included in each request.",
            category: SettingCategory::Chat,
            value_type: SettingType::Number,
            default: serde_json::json!(20),
        },
        // ----------------------------------------------------------------
        // Knowledge
        // ----------------------------------------------------------------
        SettingDef {
            key: "knowledge.indexOnStartup",
            label: "Index on Startup",
            description: "Re-index the project knowledge base when Kiro starts.",
            category: SettingCategory::Knowledge,
            value_type: SettingType::Bool,
            default: JsonValue::Bool(true),
        },
        SettingDef {
            key: "knowledge.excludePatterns",
            label: "Exclude Patterns",
            description: "Glob patterns for files and directories excluded from indexing.",
            category: SettingCategory::Knowledge,
            value_type: SettingType::StringArray,
            default: serde_json::json!(["node_modules", ".git", "target", "dist"]),
        },
        SettingDef {
            key: "knowledge.maxFileSize",
            label: "Max File Size (KB)",
            description: "Files larger than this limit (in kilobytes) are skipped during indexing.",
            category: SettingCategory::Knowledge,
            value_type: SettingType::Number,
            default: serde_json::json!(512),
        },
        SettingDef {
            key: "knowledge.chunkSize",
            label: "Chunk Size",
            description: "Token chunk size used when splitting documents for embedding.",
            category: SettingCategory::Knowledge,
            value_type: SettingType::Number,
            default: serde_json::json!(1000),
        },
        SettingDef {
            key: "knowledge.embeddingModel",
            label: "Embedding Model",
            description: "Model used to generate document embeddings for semantic search.",
            category: SettingCategory::Knowledge,
            value_type: SettingType::Enum(vec![
                "amazon.titan-embed-text-v2:0",
                "cohere.embed-english-v3",
                "cohere.embed-multilingual-v3",
            ]),
            default: JsonValue::String("amazon.titan-embed-text-v2:0".to_owned()),
        },
        // ----------------------------------------------------------------
        // Key Bindings
        // ----------------------------------------------------------------
        SettingDef {
            key: "keyBindings.submitChat",
            label: "Submit Chat",
            description: "Key combination to submit the current chat message.",
            category: SettingCategory::KeyBindings,
            value_type: SettingType::String,
            default: JsonValue::String("Enter".to_owned()),
        },
        SettingDef {
            key: "keyBindings.newLine",
            label: "New Line",
            description: "Key combination to insert a newline without submitting.",
            category: SettingCategory::KeyBindings,
            value_type: SettingType::String,
            default: JsonValue::String("Shift+Enter".to_owned()),
        },
        SettingDef {
            key: "keyBindings.clearChat",
            label: "Clear Chat",
            description: "Key combination to clear the current chat session.",
            category: SettingCategory::KeyBindings,
            value_type: SettingType::String,
            default: JsonValue::String("Ctrl+L".to_owned()),
        },
        SettingDef {
            key: "keyBindings.focusChat",
            label: "Focus Chat Input",
            description: "Key combination to move focus to the chat input box.",
            category: SettingCategory::KeyBindings,
            value_type: SettingType::String,
            default: JsonValue::String("Ctrl+I".to_owned()),
        },
        SettingDef {
            key: "keyBindings.toggleSidebar",
            label: "Toggle Sidebar",
            description: "Key combination to show or hide the sidebar.",
            category: SettingCategory::KeyBindings,
            value_type: SettingType::String,
            default: JsonValue::String("Ctrl+B".to_owned()),
        },
        // ----------------------------------------------------------------
        // Features
        // ----------------------------------------------------------------
        SettingDef {
            key: "features.agentMode",
            label: "Agent Mode",
            description: "Enable autonomous agent mode for multi-step task execution.",
            category: SettingCategory::Features,
            value_type: SettingType::Bool,
            default: JsonValue::Bool(true),
        },
        SettingDef {
            key: "features.codeActions",
            label: "Code Actions",
            description: "Show inline AI-powered code actions inside the editor.",
            category: SettingCategory::Features,
            value_type: SettingType::Bool,
            default: JsonValue::Bool(true),
        },
        SettingDef {
            key: "features.inlineCompletions",
            label: "Inline Completions",
            description: "Provide ghost-text completions as you type.",
            category: SettingCategory::Features,
            value_type: SettingType::Bool,
            default: JsonValue::Bool(true),
        },
        SettingDef {
            key: "features.diagnosticsIntegration",
            label: "Diagnostics Integration",
            description: "Surface compiler/linter diagnostics inside the AI chat.",
            category: SettingCategory::Features,
            value_type: SettingType::Bool,
            default: JsonValue::Bool(true),
        },
        SettingDef {
            key: "features.experimentalTools",
            label: "Experimental Tools",
            description: "Enable tools that are still in beta and may change.",
            category: SettingCategory::Features,
            value_type: SettingType::Bool,
            default: JsonValue::Bool(false),
        },
        // ----------------------------------------------------------------
        // API
        // ----------------------------------------------------------------
        SettingDef {
            key: "api.provider",
            label: "API Provider",
            description: "Which API backend Kiro uses to call the AI model.",
            category: SettingCategory::Api,
            value_type: SettingType::Enum(vec!["anthropic", "bedrock", "vertex"]),
            default: JsonValue::String("anthropic".to_owned()),
        },
        SettingDef {
            key: "api.region",
            label: "AWS Region",
            description: "AWS region for Bedrock or Vertex API calls.",
            category: SettingCategory::Api,
            value_type: SettingType::String,
            default: JsonValue::String("us-east-1".to_owned()),
        },
        SettingDef {
            key: "api.timeout",
            label: "Request Timeout (s)",
            description: "Number of seconds before an API request times out.",
            category: SettingCategory::Api,
            value_type: SettingType::Number,
            default: serde_json::json!(60),
        },
        SettingDef {
            key: "api.retries",
            label: "Max Retries",
            description: "How many times to retry a failed API request.",
            category: SettingCategory::Api,
            value_type: SettingType::Number,
            default: serde_json::json!(3),
        },
        SettingDef {
            key: "api.proxyUrl",
            label: "Proxy URL",
            description: "HTTP/HTTPS proxy URL for outbound API requests.",
            category: SettingCategory::Api,
            value_type: SettingType::String,
            default: JsonValue::String(String::new()),
        },
        // ----------------------------------------------------------------
        // MCP
        // ----------------------------------------------------------------
        SettingDef {
            key: "mcp.enabled",
            label: "Enable MCP",
            description: "Enable the Model Context Protocol server integration.",
            category: SettingCategory::Mcp,
            value_type: SettingType::Bool,
            default: JsonValue::Bool(true),
        },
        SettingDef {
            key: "mcp.autoStart",
            label: "Auto-Start Servers",
            description: "Automatically start configured MCP servers when Kiro launches.",
            category: SettingCategory::Mcp,
            value_type: SettingType::Bool,
            default: JsonValue::Bool(true),
        },
        SettingDef {
            key: "mcp.serverTimeout",
            label: "Server Timeout (s)",
            description: "Seconds to wait for an MCP server to become ready.",
            category: SettingCategory::Mcp,
            value_type: SettingType::Number,
            default: serde_json::json!(30),
        },
        SettingDef {
            key: "mcp.allowedHosts",
            label: "Allowed Hosts",
            description: "Hostnames or IP addresses that MCP servers may connect to.",
            category: SettingCategory::Mcp,
            value_type: SettingType::StringArray,
            default: serde_json::json!([]),
        },
        SettingDef {
            key: "mcp.logLevel",
            label: "Log Level",
            description: "Verbosity of MCP server logs.",
            category: SettingCategory::Mcp,
            value_type: SettingType::Enum(vec!["error", "warn", "info", "debug", "trace"]),
            default: JsonValue::String("info".to_owned()),
        },
        // ----------------------------------------------------------------
        // Environment
        // ----------------------------------------------------------------
        SettingDef {
            key: "environment.shell",
            label: "Shell",
            description: "Shell executable used when running terminal commands.",
            category: SettingCategory::Environment,
            value_type: SettingType::String,
            default: JsonValue::String(String::new()),
        },
        SettingDef {
            key: "environment.workingDirectory",
            label: "Working Directory",
            description: "Default working directory for new chat sessions.",
            category: SettingCategory::Environment,
            value_type: SettingType::String,
            default: JsonValue::String(String::new()),
        },
        SettingDef {
            key: "environment.extraEnvVars",
            label: "Extra Environment Variables",
            description: "Additional environment variables injected into agent sub-processes.",
            category: SettingCategory::Environment,
            value_type: SettingType::StringArray,
            default: serde_json::json!([]),
        },
        SettingDef {
            key: "environment.theme",
            label: "Theme",
            description: "Color theme for the Kiro UI.",
            category: SettingCategory::Environment,
            value_type: SettingType::Enum(vec!["system", "light", "dark"]),
            default: JsonValue::String("system".to_owned()),
        },
        SettingDef {
            key: "environment.locale",
            label: "Locale",
            description: "BCP 47 locale tag used for formatting dates and numbers.",
            category: SettingCategory::Environment,
            value_type: SettingType::String,
            default: JsonValue::String("en-US".to_owned()),
        },
        SettingDef {
            key: "environment.fontSize",
            label: "Font Size",
            description: "Base font size (in pixels) for the Kiro UI.",
            category: SettingCategory::Environment,
            value_type: SettingType::Number,
            default: serde_json::json!(14),
        },
    ]
}

// ---------------------------------------------------------------------------
// JSON path helpers
// ---------------------------------------------------------------------------

/// Traverse a nested JSON object following a dotted key path.
///
/// Returns `None` if any segment of the path is absent or if an intermediate
/// value is not an object.
///
/// # Examples
/// ```
/// # use serde_json::json;
/// # use kiro_market_core::kiro_settings::get_nested;
/// let v = json!({"chat": {"defaultModel": "claude-sonnet-4-5"}});
/// assert_eq!(get_nested(&v, "chat.defaultModel"), Some(&json!("claude-sonnet-4-5")));
/// assert_eq!(get_nested(&v, "chat.missing"), None);
/// ```
#[must_use]
pub fn get_nested<'a>(value: &'a JsonValue, path: &str) -> Option<&'a JsonValue> {
    let mut current = value;
    for segment in path.split('.') {
        current = current.as_object()?.get(segment)?;
    }
    Some(current)
}

/// Write a value at a dotted key path, creating intermediate objects as needed.
///
/// If any intermediate node already exists but is not an object it is replaced
/// with an empty object before descending.
///
/// # Panics
///
/// Panics if `path` is an empty string (i.e. contains no `.`-separated segments).
pub fn set_nested(value: &mut JsonValue, path: &str, val: JsonValue) {
    let segments: Vec<&str> = path.split('.').collect();
    let (last, parents) = segments.split_last().expect("path must not be empty");

    let mut current = value;
    for &segment in parents {
        if !current.is_object() {
            *current = serde_json::json!({});
        }
        let obj = current.as_object_mut().expect("ensured above");
        if !obj.contains_key(segment) {
            obj.insert(segment.to_owned(), serde_json::json!({}));
        }
        current = obj.get_mut(segment).expect("just inserted");
    }

    if !current.is_object() {
        *current = serde_json::json!({});
    }
    current
        .as_object_mut()
        .expect("ensured above")
        .insert((*last).to_owned(), val);
}

/// Remove the value at a dotted key path, cleaning up empty parent objects.
///
/// If the path does not exist this is a no-op.
pub fn remove_nested(value: &mut JsonValue, path: &str) {
    let segments: Vec<&str> = path.split('.').collect();
    remove_nested_impl(value, &segments);
}

/// Recursive implementation for [`remove_nested`].
///
/// Returns `true` if the caller should remove the current object from its
/// parent (i.e., the object became empty after the removal).
fn remove_nested_impl(value: &mut JsonValue, segments: &[&str]) -> bool {
    let Some((&first, rest)) = segments.split_first() else {
        return false;
    };

    let Some(obj) = value.as_object_mut() else {
        return false;
    };

    if rest.is_empty() {
        obj.remove(first);
    } else if let Some(child) = obj.get_mut(first) {
        let should_remove = remove_nested_impl(child, rest);
        if should_remove {
            obj.remove(first);
        }
    }

    obj.is_empty()
}

/// Resolve all registry settings against a loaded JSON config, returning a
/// [`SettingEntry`] for each definition.
///
/// Settings absent from `json` fall back to their `default` values.
#[must_use]
pub fn resolve_settings(json: &JsonValue) -> Vec<SettingEntry> {
    registry()
        .into_iter()
        .map(|def| {
            let current_value = get_nested(json, def.key)
                .cloned()
                .unwrap_or_else(|| def.default.clone());

            SettingEntry {
                key: def.key.to_owned(),
                label: def.label.to_owned(),
                description: def.description.to_owned(),
                category: def.category,
                category_label: def.category.label().to_owned(),
                value_type: def.value_type.type_name().to_owned(),
                enum_options: def.value_type.enum_options(),
                default_value: def.default,
                current_value,
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};

    use super::*;

    #[test]
    fn registry_keys_are_unique() {
        let reg = registry();
        let mut seen = HashSet::new();
        for def in &reg {
            assert!(seen.insert(def.key), "duplicate registry key: {}", def.key);
        }
    }

    #[test]
    fn registry_has_all_categories() {
        let all_categories = [
            SettingCategory::Telemetry,
            SettingCategory::Chat,
            SettingCategory::Knowledge,
            SettingCategory::KeyBindings,
            SettingCategory::Features,
            SettingCategory::Api,
            SettingCategory::Mcp,
            SettingCategory::Environment,
        ];

        let present: HashSet<SettingCategory> = registry().iter().map(|d| d.category).collect();

        for cat in all_categories {
            assert!(
                present.contains(&cat),
                "no settings found for category {:?}",
                cat
            );
        }
    }

    #[test]
    fn setting_category_labels_are_nonempty() {
        let all_categories = [
            SettingCategory::Telemetry,
            SettingCategory::Chat,
            SettingCategory::Knowledge,
            SettingCategory::KeyBindings,
            SettingCategory::Features,
            SettingCategory::Api,
            SettingCategory::Mcp,
            SettingCategory::Environment,
        ];

        for cat in all_categories {
            assert!(
                !cat.label().is_empty(),
                "empty label for category {:?}",
                cat
            );
        }
    }

    #[test]
    fn registry_defaults_match_value_type() {
        for def in registry() {
            match &def.value_type {
                SettingType::Bool => {
                    assert!(
                        def.default.is_boolean(),
                        "key '{}' is Bool but default is not boolean: {}",
                        def.key,
                        def.default
                    );
                }
                SettingType::Number => {
                    assert!(
                        def.default.is_number(),
                        "key '{}' is Number but default is not a number: {}",
                        def.key,
                        def.default
                    );
                }
                SettingType::String | SettingType::Char => {
                    assert!(
                        def.default.is_string(),
                        "key '{}' is String/Char but default is not a string: {}",
                        def.key,
                        def.default
                    );
                }
                SettingType::StringArray => {
                    assert!(
                        def.default.is_array(),
                        "key '{}' is StringArray but default is not an array: {}",
                        def.key,
                        def.default
                    );
                }
                SettingType::Enum(opts) => {
                    let default_str = def.default.as_str().unwrap_or_else(|| {
                        panic!(
                            "key '{}' is Enum but default is not a string: {}",
                            def.key, def.default
                        )
                    });
                    assert!(
                        opts.contains(&default_str),
                        "key '{}' default '{}' is not in enum options {:?}",
                        def.key,
                        default_str,
                        opts
                    );
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // Task 2 — JSON path helpers
    // -----------------------------------------------------------------------

    #[test]
    fn get_nested_finds_top_level_key() {
        let v = serde_json::json!({"foo": 42});
        assert_eq!(get_nested(&v, "foo"), Some(&serde_json::json!(42)));
    }

    #[test]
    fn get_nested_finds_dotted_path() {
        let v = serde_json::json!({"chat": {"defaultModel": "claude-sonnet-4-5"}});
        assert_eq!(
            get_nested(&v, "chat.defaultModel"),
            Some(&serde_json::json!("claude-sonnet-4-5"))
        );
    }

    #[test]
    fn get_nested_returns_none_for_missing() {
        let v = serde_json::json!({"chat": {}});
        assert_eq!(get_nested(&v, "chat.defaultModel"), None);
        assert_eq!(get_nested(&v, "nonexistent"), None);
    }

    #[test]
    fn set_nested_creates_intermediate_objects() {
        let mut v = serde_json::json!({});
        set_nested(
            &mut v,
            "chat.defaultModel",
            serde_json::json!("claude-opus-4"),
        );
        assert_eq!(
            get_nested(&v, "chat.defaultModel"),
            Some(&serde_json::json!("claude-opus-4"))
        );
    }

    #[test]
    fn set_nested_overwrites_existing() {
        let mut v = serde_json::json!({"chat": {"defaultModel": "old"}});
        set_nested(&mut v, "chat.defaultModel", serde_json::json!("new"));
        assert_eq!(
            get_nested(&v, "chat.defaultModel"),
            Some(&serde_json::json!("new"))
        );
    }

    #[test]
    fn remove_nested_deletes_key() {
        let mut v =
            serde_json::json!({"chat": {"defaultModel": "claude-sonnet-4-5", "temperature": 0.7}});
        remove_nested(&mut v, "chat.defaultModel");
        assert_eq!(get_nested(&v, "chat.defaultModel"), None);
        // sibling should still exist
        assert_eq!(
            get_nested(&v, "chat.temperature"),
            Some(&serde_json::json!(0.7))
        );
    }

    #[test]
    fn remove_nested_cleans_empty_parents() {
        let mut v = serde_json::json!({"chat": {"defaultModel": "claude-sonnet-4-5"}});
        remove_nested(&mut v, "chat.defaultModel");
        // chat object should have been pruned since it became empty
        assert_eq!(get_nested(&v, "chat"), None);
    }

    #[test]
    fn remove_nested_noop_for_missing() {
        let mut v = serde_json::json!({"chat": {}});
        // should not panic
        remove_nested(&mut v, "chat.doesNotExist");
        remove_nested(&mut v, "completely.missing.path");
    }

    #[test]
    fn resolve_settings_uses_defaults_when_no_file() {
        let empty = serde_json::json!({});
        let entries = resolve_settings(&empty);

        assert!(!entries.is_empty());

        for entry in &entries {
            assert_eq!(
                entry.current_value, entry.default_value,
                "key '{}' should use default when config is empty",
                entry.key
            );
        }
    }

    #[test]
    fn resolve_settings_picks_up_user_values() {
        let config = serde_json::json!({
            "chat": {
                "defaultModel": "claude-opus-4"
            }
        });
        let entries = resolve_settings(&config);

        let model_entry = entries
            .iter()
            .find(|e| e.key == "chat.defaultModel")
            .expect("chat.defaultModel must be in registry");

        assert_eq!(
            model_entry.current_value,
            serde_json::json!("claude-opus-4")
        );
    }

    #[test]
    fn resolve_settings_includes_category_label() {
        let entries = resolve_settings(&serde_json::json!({}));
        let mut labels_by_category: HashMap<String, String> = HashMap::new();
        for entry in &entries {
            labels_by_category
                .entry(format!("{:?}", entry.category))
                .or_insert_with(|| entry.category_label.clone());
        }
        for label in labels_by_category.values() {
            assert!(!label.is_empty(), "category_label must not be empty");
        }
    }
}

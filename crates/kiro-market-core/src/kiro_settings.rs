//! Kiro CLI settings registry, JSON path helpers, and file I/O.

use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use tracing::{debug, warn};

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
            Self::Telemetry => "Telemetry & Privacy",
            Self::Chat => "Chat Interface",
            Self::Knowledge => "Knowledge Base",
            Self::KeyBindings => "Key Bindings",
            Self::Features => "Feature Toggles",
            Self::Api => "API & Service",
            Self::Mcp => "MCP",
            Self::Environment => "Environment Variables",
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

    /// Returns enum options if this is an `Enum` variant, otherwise an empty vec.
    fn enum_options(&self) -> Vec<std::string::String> {
        match self {
            Self::Enum(opts) => opts.iter().map(|s| (*s).to_owned()).collect(),
            _ => vec![],
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
    /// Default value as a JSON value. `None` when the default is unknown.
    pub default: Option<JsonValue>,
}

// ---------------------------------------------------------------------------
// Setting entry (serialisable, frontend-facing)
// ---------------------------------------------------------------------------

/// A fully-resolved setting entry suitable for serialisation to a frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct SettingEntry {
    /// Dotted JSON key path.
    pub key: std::string::String,
    /// Short human-readable label.
    pub label: std::string::String,
    /// Longer description.
    pub description: std::string::String,
    /// Machine-readable category identifier (serialized `snake_case` string).
    pub category: std::string::String,
    /// Human-readable category label.
    pub category_label: std::string::String,
    /// Wire-format type name (`"bool"`, `"string"`, `"enum"`, …).
    pub value_type: std::string::String,
    /// For `Enum` settings: the allowed values. Empty vec for all other types.
    pub enum_options: Vec<std::string::String>,
    /// Default value as a JSON value. `None` when no default is known.
    pub default_value: Option<JsonValue>,
    /// Current value from the user's settings file. `None` means key absent (using default).
    pub current_value: Option<JsonValue>,
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
        // Telemetry & Privacy
        // ----------------------------------------------------------------
        SettingDef {
            key: "telemetry.enabled",
            label: "Enable Telemetry",
            description: "Enable or disable telemetry collection",
            category: SettingCategory::Telemetry,
            value_type: SettingType::Bool,
            default: Some(JsonValue::Bool(true)),
        },
        SettingDef {
            key: "telemetryClientId",
            label: "Telemetry Client ID",
            description: "Client identifier for telemetry",
            category: SettingCategory::Telemetry,
            value_type: SettingType::String,
            default: None,
        },
        // ----------------------------------------------------------------
        // Chat Interface
        // ----------------------------------------------------------------
        SettingDef {
            key: "chat.defaultModel",
            label: "Default Model",
            description: "Default AI model for conversations",
            category: SettingCategory::Chat,
            value_type: SettingType::String,
            default: None,
        },
        SettingDef {
            key: "chat.defaultAgent",
            label: "Default Agent",
            description: "Default agent configuration",
            category: SettingCategory::Chat,
            value_type: SettingType::String,
            default: None,
        },
        SettingDef {
            key: "chat.diffTool",
            label: "Diff Tool",
            description: "External diff tool for viewing code changes",
            category: SettingCategory::Chat,
            value_type: SettingType::String,
            default: None,
        },
        SettingDef {
            key: "chat.greeting.enabled",
            label: "Show Greeting",
            description: "Show greeting message on chat start",
            category: SettingCategory::Chat,
            value_type: SettingType::Bool,
            default: Some(JsonValue::Bool(true)),
        },
        SettingDef {
            key: "chat.editMode",
            label: "Edit Mode",
            description: "Enable edit mode for chat interface",
            category: SettingCategory::Chat,
            value_type: SettingType::Bool,
            default: None,
        },
        SettingDef {
            key: "chat.enableNotifications",
            label: "Enable Notifications",
            description: "Enable desktop notifications",
            category: SettingCategory::Chat,
            value_type: SettingType::Bool,
            default: None,
        },
        SettingDef {
            key: "chat.disableMarkdownRendering",
            label: "Disable Markdown Rendering",
            description: "Disable markdown formatting in chat",
            category: SettingCategory::Chat,
            value_type: SettingType::Bool,
            default: Some(JsonValue::Bool(false)),
        },
        SettingDef {
            key: "chat.disableAutoCompaction",
            label: "Disable Auto Compaction",
            description: "Disable automatic conversation summarization",
            category: SettingCategory::Chat,
            value_type: SettingType::Bool,
            default: Some(JsonValue::Bool(false)),
        },
        SettingDef {
            key: "chat.enablePromptHints",
            label: "Enable Prompt Hints",
            description: "Show startup hints with tips and shortcuts",
            category: SettingCategory::Chat,
            value_type: SettingType::Bool,
            default: Some(JsonValue::Bool(true)),
        },
        SettingDef {
            key: "chat.enableHistoryHints",
            label: "Enable History Hints",
            description: "Show conversation history hints",
            category: SettingCategory::Chat,
            value_type: SettingType::Bool,
            default: None,
        },
        SettingDef {
            key: "chat.uiMode",
            label: "UI Mode",
            description: "UI variant to use",
            category: SettingCategory::Chat,
            value_type: SettingType::String,
            default: None,
        },
        SettingDef {
            key: "chat.enableContextUsageIndicator",
            label: "Enable Context Usage Indicator",
            description: "Show context usage percentage in prompt",
            category: SettingCategory::Chat,
            value_type: SettingType::Bool,
            default: None,
        },
        SettingDef {
            key: "compaction.excludeMessages",
            label: "Compaction Exclude Messages",
            description: "Minimum message pairs to retain during compaction",
            category: SettingCategory::Chat,
            value_type: SettingType::Number,
            default: None,
        },
        SettingDef {
            key: "compaction.excludeContextWindowPercent",
            label: "Compaction Exclude Context Window Percent",
            description: "Minimum percentage of context window to retain during compaction",
            category: SettingCategory::Chat,
            value_type: SettingType::Number,
            default: None,
        },
        // ----------------------------------------------------------------
        // Knowledge Base
        // ----------------------------------------------------------------
        SettingDef {
            key: "chat.enableKnowledge",
            label: "Enable Knowledge",
            description: "Enable knowledge base functionality",
            category: SettingCategory::Knowledge,
            value_type: SettingType::Bool,
            default: None,
        },
        SettingDef {
            key: "knowledge.defaultIncludePatterns",
            label: "Default Include Patterns",
            description: "Default file patterns to include in knowledge indexing",
            category: SettingCategory::Knowledge,
            value_type: SettingType::StringArray,
            default: None,
        },
        SettingDef {
            key: "knowledge.defaultExcludePatterns",
            label: "Default Exclude Patterns",
            description: "Default file patterns to exclude from knowledge indexing",
            category: SettingCategory::Knowledge,
            value_type: SettingType::StringArray,
            default: None,
        },
        SettingDef {
            key: "knowledge.maxFiles",
            label: "Max Files",
            description: "Maximum number of files for knowledge indexing",
            category: SettingCategory::Knowledge,
            value_type: SettingType::Number,
            default: None,
        },
        SettingDef {
            key: "knowledge.chunkSize",
            label: "Chunk Size",
            description: "Text chunk size for knowledge processing",
            category: SettingCategory::Knowledge,
            value_type: SettingType::Number,
            default: None,
        },
        SettingDef {
            key: "knowledge.chunkOverlap",
            label: "Chunk Overlap",
            description: "Overlap between text chunks in knowledge processing",
            category: SettingCategory::Knowledge,
            value_type: SettingType::Number,
            default: None,
        },
        SettingDef {
            key: "knowledge.indexType",
            label: "Index Type",
            description: "Type of knowledge index to use",
            category: SettingCategory::Knowledge,
            value_type: SettingType::String,
            default: None,
        },
        // ----------------------------------------------------------------
        // Key Bindings
        // ----------------------------------------------------------------
        SettingDef {
            key: "chat.skimCommandKey",
            label: "Skim Command Key",
            description: "Key for fuzzy search command",
            category: SettingCategory::KeyBindings,
            value_type: SettingType::Char,
            default: None,
        },
        SettingDef {
            key: "chat.autocompletionKey",
            label: "Autocompletion Key",
            description: "Key for autocompletion hint acceptance",
            category: SettingCategory::KeyBindings,
            value_type: SettingType::Char,
            default: None,
        },
        SettingDef {
            key: "chat.tangentModeKey",
            label: "Tangent Mode Key",
            description: "Key for tangent mode toggle",
            category: SettingCategory::KeyBindings,
            value_type: SettingType::Char,
            default: None,
        },
        SettingDef {
            key: "chat.delegateModeKey",
            label: "Delegate Mode Key",
            description: "Key for delegate command",
            category: SettingCategory::KeyBindings,
            value_type: SettingType::Char,
            default: None,
        },
        // ----------------------------------------------------------------
        // Feature Toggles
        // ----------------------------------------------------------------
        SettingDef {
            key: "chat.enableThinking",
            label: "Enable Thinking",
            description: "Enable thinking tool for complex reasoning",
            category: SettingCategory::Features,
            value_type: SettingType::Bool,
            default: None,
        },
        SettingDef {
            key: "chat.enableTangentMode",
            label: "Enable Tangent Mode",
            description: "Enable tangent mode feature",
            category: SettingCategory::Features,
            value_type: SettingType::Bool,
            default: None,
        },
        SettingDef {
            key: "introspect.tangentMode",
            label: "Introspect Tangent Mode",
            description: "Auto-enter tangent mode for introspect",
            category: SettingCategory::Features,
            value_type: SettingType::Bool,
            default: None,
        },
        SettingDef {
            key: "chat.enableTodoList",
            label: "Enable Todo List",
            description: "Enable todo list feature",
            category: SettingCategory::Features,
            value_type: SettingType::Bool,
            default: None,
        },
        SettingDef {
            key: "chat.enableCheckpoint",
            label: "Enable Checkpoint",
            description: "Enable checkpoint feature",
            category: SettingCategory::Features,
            value_type: SettingType::Bool,
            default: None,
        },
        SettingDef {
            key: "chat.enableDelegate",
            label: "Enable Delegate",
            description: "Enable delegate tool",
            category: SettingCategory::Features,
            value_type: SettingType::Bool,
            default: None,
        },
        // ----------------------------------------------------------------
        // API & Service
        // ----------------------------------------------------------------
        SettingDef {
            key: "api.timeout",
            label: "API Timeout",
            description: "API request timeout in seconds",
            category: SettingCategory::Api,
            value_type: SettingType::Number,
            default: None,
        },
        // ----------------------------------------------------------------
        // MCP
        // ----------------------------------------------------------------
        SettingDef {
            key: "mcp.initTimeout",
            label: "MCP Init Timeout",
            description: "MCP server initialization timeout in milliseconds",
            category: SettingCategory::Mcp,
            value_type: SettingType::Number,
            default: None,
        },
        SettingDef {
            key: "mcp.noInteractiveTimeout",
            label: "MCP Non-Interactive Timeout",
            description: "Non-interactive MCP timeout in milliseconds",
            category: SettingCategory::Mcp,
            value_type: SettingType::Number,
            default: None,
        },
        SettingDef {
            key: "mcp.loadedBefore",
            label: "MCP Loaded Before",
            description: "Track previously loaded MCP servers",
            category: SettingCategory::Mcp,
            value_type: SettingType::Bool,
            default: None,
        },
        // ----------------------------------------------------------------
        // Environment Variables
        // ----------------------------------------------------------------
        SettingDef {
            key: "KIRO_LOG_NO_COLOR",
            label: "Disable Log Color",
            description: "Set to disable colored log output",
            category: SettingCategory::Environment,
            value_type: SettingType::Bool,
            default: Some(JsonValue::Bool(false)),
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
/// `current_value` is `None` when the key is absent from `json` (meaning the
/// setting is at its default). `default_value` is `None` when no default is
/// known for that setting.
#[must_use]
pub fn resolve_settings(json: &JsonValue) -> Vec<SettingEntry> {
    registry()
        .into_iter()
        .map(|def| {
            let current_value = get_nested(json, def.key).cloned();

            let category_str = serde_json::to_value(def.category)
                .ok()
                .and_then(|v| v.as_str().map(std::borrow::ToOwned::to_owned))
                .unwrap_or_default();

            SettingEntry {
                key: def.key.to_owned(),
                label: def.label.to_owned(),
                description: def.description.to_owned(),
                category: category_str,
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
// Kiro settings file I/O
// ---------------------------------------------------------------------------

/// Path to the CLI settings file relative to the Kiro home directory.
const SETTINGS_FILE: &str = "settings/cli.json";

/// Load `settings/cli.json` from the given Kiro home directory.
///
/// Returns an empty JSON object (`{}`) if the file does not exist or if its
/// contents cannot be parsed as JSON.
#[must_use]
pub fn load_kiro_settings_from(kiro_dir: &Path) -> JsonValue {
    let path = kiro_dir.join(SETTINGS_FILE);
    debug!(path = %path.display(), "loading Kiro settings");

    match std::fs::read_to_string(&path) {
        Ok(contents) => match serde_json::from_str::<JsonValue>(&contents) {
            Ok(json) => json,
            Err(e) => {
                warn!(
                    path = %path.display(),
                    error = %e,
                    "settings file contains invalid JSON, using empty config"
                );
                serde_json::json!({})
            }
        },
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            debug!(path = %path.display(), "settings file not found, using empty config");
            serde_json::json!({})
        }
        Err(e) => {
            warn!(
                path = %path.display(),
                error = %e,
                "could not read settings file, using empty config"
            );
            serde_json::json!({})
        }
    }
}

/// Save a JSON value to `settings/cli.json` inside the given Kiro home directory.
///
/// Creates the `settings/` subdirectory if it does not already exist.
///
/// # Errors
///
/// Returns an [`io::Error`] if directory creation or file write fails.
pub fn save_kiro_settings_to(kiro_dir: &Path, json: &JsonValue) -> io::Result<()> {
    let path = kiro_dir.join(SETTINGS_FILE);
    debug!(path = %path.display(), "saving Kiro settings");

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let contents = serde_json::to_string_pretty(json)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

    std::fs::write(&path, contents)?;
    Ok(())
}

/// Resolve the default Kiro home directory (`~/.kiro`).
///
/// Returns `None` if the home directory cannot be determined.
#[must_use]
pub fn default_kiro_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".kiro"))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};

    use tempfile::TempDir;

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
            // Settings with no known default skip type validation.
            let Some(ref default) = def.default else {
                continue;
            };

            match &def.value_type {
                SettingType::Bool => {
                    assert!(
                        default.is_boolean(),
                        "key '{}' is Bool but default is not boolean: {}",
                        def.key,
                        default
                    );
                }
                SettingType::Number => {
                    assert!(
                        default.is_number(),
                        "key '{}' is Number but default is not a number: {}",
                        def.key,
                        default
                    );
                }
                SettingType::String | SettingType::Char => {
                    assert!(
                        default.is_string(),
                        "key '{}' is String/Char but default is not a string: {}",
                        def.key,
                        default
                    );
                }
                SettingType::StringArray => {
                    assert!(
                        default.is_array(),
                        "key '{}' is StringArray but default is not an array: {}",
                        def.key,
                        default
                    );
                }
                SettingType::Enum(opts) => {
                    let default_str = default.as_str().unwrap_or_else(|| {
                        panic!(
                            "key '{}' is Enum but default is not a string: {}",
                            def.key, default
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
            assert!(
                entry.current_value.is_none(),
                "key '{}' current_value should be None when config is empty",
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
            Some(serde_json::json!("claude-opus-4"))
        );
    }

    #[test]
    fn resolve_settings_includes_category_label() {
        let entries = resolve_settings(&serde_json::json!({}));

        let has_telemetry_privacy = entries
            .iter()
            .any(|e| e.category_label == "Telemetry & Privacy");

        assert!(
            has_telemetry_privacy,
            "expected 'Telemetry & Privacy' category label to appear in resolved entries"
        );

        let mut labels_by_category: HashMap<String, String> = HashMap::new();
        for entry in &entries {
            labels_by_category
                .entry(entry.category.clone())
                .or_insert_with(|| entry.category_label.clone());
        }
        for label in labels_by_category.values() {
            assert!(!label.is_empty(), "category_label must not be empty");
        }
    }

    // -----------------------------------------------------------------------
    // Task 3 — Kiro settings file I/O
    // -----------------------------------------------------------------------

    #[test]
    fn load_kiro_settings_returns_empty_when_no_file() {
        let dir = TempDir::new().unwrap();
        let result = load_kiro_settings_from(dir.path());
        assert_eq!(result, serde_json::json!({}));
    }

    #[test]
    fn save_and_load_kiro_settings_roundtrip() {
        let dir = TempDir::new().unwrap();
        let settings = serde_json::json!({
            "chat": {
                "defaultModel": "claude-opus-4",
                "temperature": 0.5
            }
        });

        save_kiro_settings_to(dir.path(), &settings).expect("save should succeed");

        let loaded = load_kiro_settings_from(dir.path());
        assert_eq!(loaded, settings);
    }

    #[test]
    fn save_kiro_settings_creates_parent_dirs() {
        let dir = TempDir::new().unwrap();
        // The settings subdirectory does not exist yet.
        let nested = dir.path().join("does_not_exist_yet");
        let settings = serde_json::json!({"key": "value"});

        save_kiro_settings_to(&nested, &settings).expect("save should create parent dirs");

        let loaded = load_kiro_settings_from(&nested);
        assert_eq!(loaded, settings);
    }

    #[test]
    fn load_kiro_settings_returns_empty_on_corrupt_json() {
        let dir = TempDir::new().unwrap();
        let settings_dir = dir.path().join("settings");
        std::fs::create_dir_all(&settings_dir).unwrap();
        std::fs::write(settings_dir.join("cli.json"), b"{ this is not valid json }").unwrap();

        let result = load_kiro_settings_from(dir.path());
        assert_eq!(result, serde_json::json!({}));
    }

    #[test]
    fn save_kiro_settings_preserves_unknown_keys() {
        let dir = TempDir::new().unwrap();
        let settings = serde_json::json!({
            "chat": {"defaultModel": "claude-sonnet-4-5"},
            "unknownFutureKey": {"nestedValue": true}
        });

        save_kiro_settings_to(dir.path(), &settings).unwrap();
        let loaded = load_kiro_settings_from(dir.path());

        assert_eq!(
            loaded["unknownFutureKey"]["nestedValue"],
            serde_json::json!(true)
        );
        assert_eq!(
            loaded["chat"]["defaultModel"],
            serde_json::json!("claude-sonnet-4-5")
        );
    }
}

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
    /// Returns the wire-format type name used in error messages.
    #[must_use]
    pub fn type_name(&self) -> &'static str {
        match self {
            Self::Bool => "bool",
            Self::String => "string",
            Self::Number => "number",
            Self::Char => "char",
            Self::StringArray => "string_array",
            Self::Enum(_) => "enum",
        }
    }

    /// Check if a JSON value is compatible with this setting type.
    #[must_use]
    pub fn is_compatible_value(&self, value: &JsonValue) -> bool {
        match self {
            Self::Bool => value.is_boolean(),
            Self::String => value.is_string(),
            Self::Char => value.as_str().is_some_and(|s| s.chars().count() == 1),
            Self::Number => value.is_number(),
            Self::StringArray => value
                .as_array()
                .is_some_and(|arr| arr.iter().all(JsonValue::is_string)),
            Self::Enum(opts) => value.as_str().is_some_and(|s| opts.contains(&s)),
        }
    }
}

// ---------------------------------------------------------------------------
// Setting value info (frontend-facing discriminated union)
// ---------------------------------------------------------------------------

/// Describes the value type and any type-specific metadata for a setting entry.
///
/// Serialized as an internally-tagged enum so the frontend can use a
/// discriminated union: `entry.value_type.kind === "bool"`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SettingValueInfo {
    Bool,
    String,
    Number,
    Char,
    StringArray,
    Enum { options: Vec<std::string::String> },
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
    /// Typed category identifier.
    pub category: SettingCategory,
    /// Human-readable category label.
    pub category_label: std::string::String,
    /// Value type and type-specific metadata (discriminated union on the frontend).
    pub value_type: SettingValueInfo,
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
/// The returned slice is ordered: categories are grouped and within each
/// category settings appear in a logical sequence. The registry is initialized
/// once and reused on subsequent calls.
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn registry() -> &'static [SettingDef] {
    use std::sync::OnceLock;
    static REGISTRY: OnceLock<Vec<SettingDef>> = OnceLock::new();
    REGISTRY.get_or_init(|| {
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
            // Key lives under the chat namespace in the upstream CLI but
            // logically belongs to knowledge configuration.
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
    })
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
/// The internal `expect` calls are defensive assertions that cannot be reached
/// in practice: `str::split('.')` always produces at least one element, so
/// `split_last` never returns `None`, and the object-mutation invariants are
/// upheld by the preceding `if !current.is_object()` guards.
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
        .iter()
        .map(|def| {
            let current_value = get_nested(json, def.key).cloned();

            let value_type = match &def.value_type {
                SettingType::Bool => SettingValueInfo::Bool,
                SettingType::String => SettingValueInfo::String,
                SettingType::Number => SettingValueInfo::Number,
                SettingType::Char => SettingValueInfo::Char,
                SettingType::StringArray => SettingValueInfo::StringArray,
                SettingType::Enum(opts) => SettingValueInfo::Enum {
                    options: opts.iter().map(|&s| s.to_owned()).collect(),
                },
            };

            SettingEntry {
                key: def.key.to_owned(),
                label: def.label.to_owned(),
                description: def.description.to_owned(),
                category: def.category,
                category_label: def.category.label().to_owned(),
                value_type,
                default_value: def.default.clone(),
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

/// Errors from loading the Kiro CLI settings file.
#[derive(Debug, thiserror::Error)]
pub enum LoadSettingsError {
    /// The file does not exist — use empty defaults.
    #[error("settings file not found")]
    NotFound,
    /// An I/O error occurred reading the file.
    #[error("failed to read settings file: {0}")]
    Io(#[from] std::io::Error),
    /// The file exists but contains invalid JSON.
    #[error("settings file contains invalid JSON: {0}")]
    InvalidJson(#[from] serde_json::Error),
}

/// Load `settings/cli.json` from the given Kiro home directory.
///
/// Returns `Err(LoadSettingsError::NotFound)` if the file does not exist,
/// `Err(LoadSettingsError::InvalidJson)` if the file contains invalid JSON,
/// or `Err(LoadSettingsError::Io)` for other I/O errors.
///
/// # Errors
///
/// Returns a [`LoadSettingsError`] if the file cannot be read or parsed.
pub fn load_kiro_settings_from(kiro_dir: &Path) -> Result<JsonValue, LoadSettingsError> {
    let path = kiro_dir.join(SETTINGS_FILE);
    debug!(path = %path.display(), "loading Kiro settings");

    match std::fs::read_to_string(&path) {
        Ok(contents) => match serde_json::from_str::<JsonValue>(&contents) {
            Ok(json) => Ok(json),
            Err(e) => {
                warn!(
                    path = %path.display(),
                    error = %e,
                    "settings file contains invalid JSON"
                );
                Err(LoadSettingsError::InvalidJson(e))
            }
        },
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            debug!(path = %path.display(), "settings file not found");
            Err(LoadSettingsError::NotFound)
        }
        Err(e) => {
            warn!(
                path = %path.display(),
                error = %e,
                "could not read settings file"
            );
            Err(LoadSettingsError::Io(e))
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

    crate::cache::atomic_write(&path, contents.as_bytes())?;
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

    use rstest::rstest;
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn registry_keys_are_unique() {
        let reg = registry();
        let mut seen = HashSet::new();
        for def in reg {
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
    // JSON path helpers
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
    fn get_nested_finds_three_segment_path() {
        let json: JsonValue = serde_json::json!({"chat": {"greeting": {"enabled": true}}});
        assert_eq!(
            get_nested(&json, "chat.greeting.enabled"),
            Some(&JsonValue::Bool(true))
        );
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
    fn set_nested_creates_three_segment_path() {
        let mut json: JsonValue = serde_json::json!({});
        set_nested(&mut json, "chat.greeting.enabled", JsonValue::Bool(true));
        assert_eq!(
            json,
            serde_json::json!({"chat": {"greeting": {"enabled": true}}})
        );
    }

    #[test]
    fn set_nested_replaces_non_object_intermediate() {
        let mut json: JsonValue = serde_json::json!({"chat": "not-an-object"});
        set_nested(
            &mut json,
            "chat.defaultModel",
            JsonValue::String("opus".into()),
        );
        assert_eq!(
            get_nested(&json, "chat.defaultModel"),
            Some(&JsonValue::String("opus".into()))
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
    fn remove_nested_cleans_all_empty_ancestors() {
        let mut json: JsonValue = serde_json::json!({"chat": {"greeting": {"enabled": true}}});
        remove_nested(&mut json, "chat.greeting.enabled");
        assert_eq!(json, serde_json::json!({}));
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

        let mut labels_by_category: HashMap<SettingCategory, String> = HashMap::new();
        for entry in &entries {
            labels_by_category
                .entry(entry.category)
                .or_insert_with(|| entry.category_label.clone());
        }
        for label in labels_by_category.values() {
            assert!(!label.is_empty(), "category_label must not be empty");
        }
    }

    // -----------------------------------------------------------------------
    // Kiro settings file I/O
    // -----------------------------------------------------------------------

    #[test]
    fn load_kiro_settings_returns_empty_when_no_file() {
        let dir = TempDir::new().unwrap();
        let result = load_kiro_settings_from(dir.path());
        assert!(matches!(result, Err(LoadSettingsError::NotFound)));
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

        let loaded = load_kiro_settings_from(dir.path()).unwrap();
        assert_eq!(loaded, settings);
    }

    #[test]
    fn save_kiro_settings_creates_parent_dirs() {
        let dir = TempDir::new().unwrap();
        // The settings subdirectory does not exist yet.
        let nested = dir.path().join("does_not_exist_yet");
        let settings = serde_json::json!({"key": "value"});

        save_kiro_settings_to(&nested, &settings).expect("save should create parent dirs");

        let loaded = load_kiro_settings_from(&nested).unwrap();
        assert_eq!(loaded, settings);
    }

    #[test]
    fn load_kiro_settings_returns_empty_on_corrupt_json() {
        let dir = TempDir::new().unwrap();
        let settings_dir = dir.path().join("settings");
        std::fs::create_dir_all(&settings_dir).unwrap();
        std::fs::write(settings_dir.join("cli.json"), b"{ this is not valid json }").unwrap();

        let result = load_kiro_settings_from(dir.path());
        assert!(matches!(result, Err(LoadSettingsError::InvalidJson(_))));
    }

    #[test]
    fn save_kiro_settings_preserves_unknown_keys() {
        let dir = TempDir::new().unwrap();
        let settings = serde_json::json!({
            "chat": {"defaultModel": "claude-sonnet-4-5"},
            "unknownFutureKey": {"nestedValue": true}
        });

        save_kiro_settings_to(dir.path(), &settings).unwrap();
        let loaded = load_kiro_settings_from(dir.path()).unwrap();

        assert_eq!(
            loaded["unknownFutureKey"]["nestedValue"],
            serde_json::json!(true)
        );
        assert_eq!(
            loaded["chat"]["defaultModel"],
            serde_json::json!("claude-sonnet-4-5")
        );
    }

    #[rstest]
    #[case::bool_accepts_bool(SettingType::Bool, serde_json::json!(true), true)]
    #[case::bool_rejects_string(SettingType::Bool, serde_json::json!("yes"), false)]
    #[case::number_accepts_number(SettingType::Number, serde_json::json!(42), true)]
    #[case::number_rejects_bool(SettingType::Number, serde_json::json!(true), false)]
    #[case::string_accepts_string(SettingType::String, serde_json::json!("hello"), true)]
    #[case::string_rejects_number(SettingType::String, serde_json::json!(42), false)]
    #[case::char_accepts_single_char(SettingType::Char, serde_json::json!("x"), true)]
    #[case::char_rejects_multi_char(SettingType::Char, serde_json::json!("abc"), false)]
    #[case::char_rejects_empty(SettingType::Char, serde_json::json!(""), false)]
    #[case::string_array_accepts_array(SettingType::StringArray, serde_json::json!(["a", "b"]), true)]
    #[case::string_array_rejects_int_array(SettingType::StringArray, serde_json::json!([1, 2]), false)]
    #[case::string_array_accepts_empty(SettingType::StringArray, serde_json::json!([]), true)]
    #[case::enum_accepts_valid(SettingType::Enum(vec!["a", "b"]), serde_json::json!("a"), true)]
    #[case::enum_rejects_invalid(SettingType::Enum(vec!["a", "b"]), serde_json::json!("c"), false)]
    #[case::string_array_rejects_mixed(SettingType::StringArray, serde_json::json!([1, "a"]), false)]
    #[case::enum_rejects_when_options_empty(SettingType::Enum(vec![]), serde_json::json!("any"), false)]
    fn is_compatible_value_validates_types(
        #[case] setting_type: SettingType,
        #[case] value: JsonValue,
        #[case] expected: bool,
    ) {
        assert_eq!(
            setting_type.is_compatible_value(&value),
            expected,
            "is_compatible_value({setting_type:?}, {value}) should be {expected}"
        );
    }

    #[test]
    fn resolve_settings_populates_value_type_and_default() {
        let empty = serde_json::json!({});
        let entries = resolve_settings(&empty);

        let telemetry = entries
            .iter()
            .find(|e| e.key == "telemetry.enabled")
            .expect("telemetry.enabled must be in registry");

        assert!(
            matches!(telemetry.value_type, SettingValueInfo::Bool),
            "expected Bool value_type, got {:?}",
            telemetry.value_type
        );
        assert_eq!(
            telemetry.default_value,
            Some(serde_json::json!(true)),
            "telemetry.enabled should default to true"
        );
        assert!(
            telemetry.current_value.is_none(),
            "current_value should be None when resolved against empty JSON"
        );
    }

    #[test]
    fn set_nested_preserves_sibling_keys() {
        let mut json = serde_json::json!({
            "chat": {
                "defaultModel": "claude-sonnet-4-5",
                "temperature": 0.7
            },
            "mcp": {
                "initTimeout": 5000
            }
        });

        set_nested(
            &mut json,
            "chat.defaultModel",
            serde_json::json!("claude-opus-4"),
        );

        assert_eq!(
            get_nested(&json, "chat.defaultModel"),
            Some(&serde_json::json!("claude-opus-4")),
            "chat.defaultModel should be updated"
        );
        assert_eq!(
            get_nested(&json, "chat.temperature"),
            Some(&serde_json::json!(0.7)),
            "chat.temperature should be unchanged"
        );
        assert_eq!(
            get_nested(&json, "mcp.initTimeout"),
            Some(&serde_json::json!(5000)),
            "mcp.initTimeout should be unchanged"
        );
    }
}

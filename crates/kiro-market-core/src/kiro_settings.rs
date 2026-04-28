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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
// Setting value (typed subset of JSON the frontend can produce)
// ---------------------------------------------------------------------------

/// Every shape a Kiro CLI setting can hold — the frontend-safe subset of
/// `serde_json::Value`. Declared as an untagged enum so the wire format is
/// identical to raw JSON primitives, while giving specta a precise
/// TypeScript union to emit. `Integer` precedes `Float` in the variant order
/// so serde's untagged dispatch prefers `i64` for whole-number JSON literals
/// and only falls through to `f64` for fractional inputs — preserving the
/// integer/float distinction on round-trip through `settings.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(untagged)]
pub enum SettingValue {
    Bool(bool),
    Integer(i64),
    Float(f64),
    String(std::string::String),
    StringArray(Vec<std::string::String>),
}

impl From<SettingValue> for JsonValue {
    fn from(v: SettingValue) -> Self {
        match v {
            SettingValue::Bool(b) => Self::Bool(b),
            SettingValue::Integer(i) => Self::from(i),
            SettingValue::Float(f) => {
                serde_json::Number::from_f64(f).map_or(Self::Null, Self::Number)
            }
            SettingValue::String(s) => Self::String(s),
            SettingValue::StringArray(a) => Self::Array(a.into_iter().map(Self::String).collect()),
        }
    }
}

impl TryFrom<&JsonValue> for SettingValue {
    type Error = UnsupportedValueShape;

    fn try_from(v: &JsonValue) -> Result<Self, Self::Error> {
        match v {
            JsonValue::Bool(b) => Ok(Self::Bool(*b)),
            JsonValue::Number(n) => n
                .as_i64()
                .map(Self::Integer)
                .or_else(|| n.as_f64().map(Self::Float))
                .ok_or(UnsupportedValueShape),
            JsonValue::String(s) => Ok(Self::String(s.clone())),
            JsonValue::Array(arr) => arr
                .iter()
                .map(|item| {
                    item.as_str()
                        .map(str::to_owned)
                        .ok_or(UnsupportedValueShape)
                })
                .collect::<Result<Vec<_>, _>>()
                .map(Self::StringArray),
            JsonValue::Null | JsonValue::Object(_) => Err(UnsupportedValueShape),
        }
    }
}

/// Returned when a stored JSON value does not fit any [`SettingValue`] variant
/// (e.g. a nested object, or an array containing non-string elements). The
/// setting is exposed to the frontend as "no current value" rather than
/// surfacing a partial or misleading shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnsupportedValueShape;

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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
    /// Default value. `None` when no default is known.
    ///
    /// Stored internally as a `serde_json::Value` to preserve round-trip
    /// fidelity with `settings.json`. Emitted to specta as `SettingValue` so
    /// the frontend sees a precise TypeScript union instead of an opaque
    /// recursive JSON type.
    #[cfg_attr(feature = "specta", specta(type = Option<SettingValue>))]
    pub default_value: Option<JsonValue>,
    /// Current value from the user's settings file. `None` means key absent (using default).
    #[cfg_attr(feature = "specta", specta(type = Option<SettingValue>))]
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
/// with an empty object before descending. The replacement is logged at
/// `warn` so that unexpectedly destroying a leaf value is not silent — a
/// user with `{"chat": "broken"}` calling `set_nested("chat.model", v)`
/// loses the `"broken"` string, and the warning gives them a trail.
///
/// Empty paths and paths with empty segments (`""`, `"."`, `".key"`,
/// `"key."`, `"a..b"`, `".."`) are rejected at the top of the function
/// with a `tracing::error!` and the JSON value is left unchanged. Use
/// [`is_well_formed_dotted_path`] to pre-validate if you need a
/// boolean check rather than the side-effecting skip.
///
/// # Trust contract
///
/// `set_nested` does **not** look up the key in [`registry()`] or check
/// the value's type. Callers handling untrusted input MUST validate
/// against the registry first — use [`apply_registered_setting`] for
/// the combined "validate against registry, then write" flow. The
/// command-layer wrapper in `crates/kiro-control-center/src-tauri`
/// already does both validations; this raw helper exists only because
/// some callers (test fixtures, future migrations) need to write
/// arbitrary keys without registry coupling.
pub fn set_nested(value: &mut JsonValue, path: &str, val: JsonValue) {
    // Reject malformed dotted paths (empty, leading/trailing/repeated dot)
    // before walking. Prior versions silently produced shapes like
    // `{"": {"key": v}}` for `".key"` — invisible in the UI and a real
    // foot-gun for any future migration that calls `set_nested` directly.
    // The registry-validated wrapper `apply_registered_setting` never
    // reaches this branch, so this is defense-in-depth for `pub` callers.
    if !is_well_formed_dotted_path(path) {
        tracing::error!(
            path = %path,
            "set_nested: malformed dotted path (empty / leading dot / trailing dot / repeated dot); write skipped"
        );
        return;
    }

    // rsplit_once distinguishes "no dot" (e.g. "key") from "trailing dot"
    // (e.g. ".key"). With no dot, parents is empty and the whole path is
    // the leaf key; otherwise the prefix splits on '.' to walk segments.
    let (parents, last): (Vec<&str>, &str) = match path.rsplit_once('.') {
        Some((rest, last)) => (rest.split('.').collect(), last),
        None => (Vec::new(), path),
    };

    let mut current = value;
    let mut traversed = String::new();
    for segment in parents {
        ensure_object(current, path, &traversed, "intermediate");
        let JsonValue::Object(obj) = current else {
            // `ensure_object` unconditionally produces a `JsonValue::Object`
            // — reaching this branch means that contract was broken. Surface
            // it via `tracing::error!` so a future regression in
            // `ensure_object` is observable, not a silent no-op write.
            tracing::error!(
                path = %path,
                at = %traversed,
                "set_nested: ensure_object did not yield an Object on intermediate; write skipped"
            );
            return;
        };
        let entry = obj
            .entry(segment.to_owned())
            .or_insert_with(|| JsonValue::Object(serde_json::Map::new()));
        if !traversed.is_empty() {
            traversed.push('.');
        }
        traversed.push_str(segment);
        current = entry;
    }

    ensure_object(current, path, &traversed, "leaf-parent");
    let JsonValue::Object(obj) = current else {
        tracing::error!(
            path = %path,
            at = %traversed,
            "set_nested: ensure_object did not yield an Object at leaf-parent; write skipped"
        );
        return;
    };
    obj.insert(last.to_owned(), val);
}

/// A dotted path like `"chat.defaultModel"` is well-formed iff every
/// `.`-delimited segment is non-empty. Rejects `""`, `"."`, `".key"`,
/// `"key."`, `"a..b"`, and `".."` — the previously-silent corruption
/// shapes flagged by the silent-failure hunter on PR #73.
fn is_well_formed_dotted_path(path: &str) -> bool {
    !path.is_empty() && path.split('.').all(|s| !s.is_empty())
}

/// Replace a non-Object `JsonValue` with an empty object, logging the
/// kind that was discarded. Idempotent: if `value` is already an Object,
/// this is a no-op.
///
/// `position` is `"intermediate"` (mid-walk) or `"leaf-parent"` (final
/// `ensure_object` before the leaf insert). The previous shape collapsed
/// both into the same log message; preserving the distinction lets log
/// triage tell apart a destroyed mid-path object from a destroyed leaf
/// parent. The `at` field uses `%` Display formatting so log parsers
/// don't see the field rendered with surrounding quotes.
fn ensure_object(value: &mut JsonValue, path: &str, at: &str, position: &'static str) {
    if matches!(value, JsonValue::Object(_)) {
        return;
    }
    let at_display = if at.is_empty() { "<root>" } else { at };
    tracing::warn!(
        path = %path,
        at = %at_display,
        position = position,
        replaced_kind = json_kind(value),
        "set_nested overwrote non-object {position} with an empty object"
    );
    *value = JsonValue::Object(serde_json::Map::new());
}

/// Reasons an [`apply_registered_setting`] call can reject the input.
///
/// Marked `#[non_exhaustive]` so future variants (e.g. `ReadOnly`,
/// `Deprecated`, `OutOfRange`) are additive without breaking matches in
/// downstream crates. Pattern matches in any caller MUST include a
/// catch-all `_ =>` arm.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ApplySettingError {
    /// The dotted key is not present in [`registry()`]. A typo or a key
    /// from a future schema; refusing to write avoids polluting the
    /// settings file with junk that the UI would never show.
    #[error("unknown setting key: {key}")]
    UnknownKey { key: String },

    /// The value's JSON type does not match the registered
    /// [`SettingType`] for the key. Catches `chat.defaultModel = true`
    /// (string slot, bool value) before it reaches disk.
    #[error("invalid value for `{key}`: expected {expected}, got {actual}")]
    TypeMismatch {
        key: String,
        expected: &'static str,
        actual: &'static str,
    },
}

/// Write a value at a registered Kiro CLI setting key, validating both
/// the key and the value type against [`registry()`].
///
/// Use this from any caller that handles externally-supplied input —
/// CLI flags, Tauri command arguments, future plugin/IPC messages.
/// `set_nested` alone has no registry coupling and would happily write
/// `weird.unknown.path = 12` into the user's settings file, which the
/// UI then can't render. This wrapper combines the previously inline
/// `validate_key` + type-compatibility check + `set_nested` triple so
/// every call site gets the same defense-in-depth.
///
/// # Errors
///
/// - [`ApplySettingError::UnknownKey`] if `key` is not in the registry.
/// - [`ApplySettingError::TypeMismatch`] if `val` is not a valid value
///   for the registered [`SettingType`].
pub fn apply_registered_setting(
    json: &mut JsonValue,
    key: &str,
    val: JsonValue,
) -> Result<(), ApplySettingError> {
    let def =
        registry()
            .iter()
            .find(|d| d.key == key)
            .ok_or_else(|| ApplySettingError::UnknownKey {
                key: key.to_owned(),
            })?;

    if !def.value_type.is_compatible_value(&val) {
        return Err(ApplySettingError::TypeMismatch {
            key: key.to_owned(),
            expected: def.value_type.type_name(),
            actual: json_kind(&val),
        });
    }

    set_nested(json, key, val);
    Ok(())
}

/// Human-readable label for a JSON value's variant, used in `set_nested` warnings.
fn json_kind(v: &JsonValue) -> &'static str {
    match v {
        JsonValue::Null => "null",
        JsonValue::Bool(_) => "bool",
        JsonValue::Number(_) => "number",
        JsonValue::String(_) => "string",
        JsonValue::Array(_) => "array",
        JsonValue::Object(_) => "object",
    }
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

/// Resolve a single registry setting by key against a loaded JSON config,
/// returning `None` if the key is not registered.
///
/// Equivalent to `resolve_settings(json).into_iter().find(|e| e.key == key)`
/// but avoids materializing the full entry list. Use this from callers that
/// have a key in hand (e.g. the Tauri `set_kiro_setting` handler that needs
/// to return the just-updated entry); use [`resolve_settings`] from callers
/// that need every entry.
#[must_use]
pub fn resolve_setting_for_key(json: &JsonValue, key: &str) -> Option<SettingEntry> {
    let def = registry().iter().find(|d| d.key == key)?;
    let current_value = get_nested(json, def.key).cloned();
    Some(SettingEntry {
        key: def.key.to_owned(),
        label: def.label.to_owned(),
        description: def.description.to_owned(),
        category: def.category,
        category_label: def.category.label().to_owned(),
        value_type: setting_type_to_value_info(&def.value_type),
        default_value: def.default.clone(),
        current_value,
    })
}

/// Project a `SettingType` (the registry's compact representation) into a
/// `SettingValueInfo` (the wire-format representation the frontend
/// receives). Shared between [`resolve_setting_for_key`] and
/// [`resolve_settings`] so a future `SettingType` variant only needs the
/// arm added in one place — three independent reviewers (gemini-code-assist,
/// two Kiro re-review passes) flagged the prior duplication.
fn setting_type_to_value_info(t: &SettingType) -> SettingValueInfo {
    match t {
        SettingType::Bool => SettingValueInfo::Bool,
        SettingType::String => SettingValueInfo::String,
        SettingType::Number => SettingValueInfo::Number,
        SettingType::Char => SettingValueInfo::Char,
        SettingType::StringArray => SettingValueInfo::StringArray,
        SettingType::Enum(opts) => SettingValueInfo::Enum {
            options: opts.iter().map(|&s| s.to_owned()).collect(),
        },
    }
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
        .map(|def| SettingEntry {
            key: def.key.to_owned(),
            label: def.label.to_owned(),
            description: def.description.to_owned(),
            category: def.category,
            category_label: def.category.label().to_owned(),
            value_type: setting_type_to_value_info(&def.value_type),
            default_value: def.default.clone(),
            current_value: get_nested(json, def.key).cloned(),
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Kiro settings file I/O
// ---------------------------------------------------------------------------

/// Path to the CLI settings file relative to the Kiro home directory.
const SETTINGS_FILE: &str = "settings/cli.json";

/// Errors from loading the Kiro CLI settings file.
///
/// `#[non_exhaustive]` so future variants (file corruption mid-write,
/// permission-denied with diagnostics, etc.) are additive without
/// breaking matches in downstream crates. Pattern matches outside
/// `kiro-market-core` MUST include a catch-all `_ =>` arm.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
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

/// Path to `settings/cli.json` inside the given Kiro home directory.
///
/// Exposed so callers (e.g. Tauri commands using
/// [`crate::file_lock::with_file_lock`]) can lock the same file the
/// load/save functions touch, providing cross-process serialisation.
#[must_use]
pub fn kiro_settings_path(kiro_dir: &Path) -> PathBuf {
    kiro_dir.join(SETTINGS_FILE)
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

    /// `apply_registered_setting` calls `set_nested(json, key, val)` and
    /// unconditionally returns `Ok(())` afterward. `set_nested` silently
    /// skips writes for malformed dotted paths (with `tracing::error!`).
    /// If a future registry maintainer ships a malformed key (e.g.
    /// `chat..defaultModel`, `KIRO_LOG_NO_COLOR.`), the user would see
    /// "save succeeded" but no value would persist — the kind of silent
    /// failure the no-unwrap gate exists to prevent.
    ///
    /// This test pins the implicit contract: every registered key must
    /// be a well-formed dotted path that `set_nested` will actually act
    /// on.
    #[test]
    fn every_registry_key_is_well_formed_dotted_path() {
        for def in registry() {
            assert!(
                is_well_formed_dotted_path(def.key),
                "registry key {:?} is not a well-formed dotted path; \
                 apply_registered_setting would silently skip writes for it",
                def.key,
            );
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
                "no settings found for category {cat:?}"
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
            assert!(!cat.label().is_empty(), "empty label for category {cat:?}");
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

    /// Adversarial inputs that the previous `set_nested` body silently
    /// corrupted into garbage JSON shapes (e.g. `".key"` →
    /// `{"": {"key": val}}`). Validation now skips the write and emits a
    /// `tracing::error!` so the failure is observable instead of writing
    /// the user's settings into a key they cannot see in the UI.
    #[rstest]
    #[case::empty("")]
    #[case::single_dot(".")]
    #[case::leading_dot(".key")]
    #[case::trailing_dot("key.")]
    #[case::double_dot("a..b")]
    #[case::only_dots("..")]
    fn set_nested_rejects_malformed_paths(#[case] path: &str) {
        let mut v = serde_json::json!({});
        set_nested(&mut v, path, JsonValue::from(1));
        assert_eq!(
            v,
            serde_json::json!({}),
            "malformed path {path:?} must not produce any write; got {v}"
        );
    }

    #[test]
    fn set_nested_writes_single_segment_path() {
        // No-dot path should write at the root (the rsplit_once None
        // branch). Lock the contract so a future regression doesn't
        // accidentally start treating "topLevel" as malformed.
        let mut v = serde_json::json!({});
        set_nested(&mut v, "topLevel", JsonValue::from(42));
        assert_eq!(v, serde_json::json!({"topLevel": 42}));
    }

    // -----------------------------------------------------------------------
    // apply_registered_setting
    // -----------------------------------------------------------------------

    #[test]
    fn apply_registered_setting_writes_known_key() {
        // chat.defaultModel is a known registered string setting.
        let mut json = serde_json::json!({});
        apply_registered_setting(
            &mut json,
            "chat.defaultModel",
            JsonValue::from("claude-sonnet-4-6"),
        )
        .expect("known string key should accept a string value");
        assert_eq!(
            get_nested(&json, "chat.defaultModel"),
            Some(&JsonValue::from("claude-sonnet-4-6"))
        );
    }

    #[test]
    fn apply_registered_setting_rejects_unknown_key() {
        // A path that is not in the registry must be refused so junk
        // settings cannot accumulate. Critical for the Tauri command
        // boundary which receives keys from the renderer process.
        let mut json = serde_json::json!({});
        let err =
            apply_registered_setting(&mut json, "totally.unknown.setting", JsonValue::from(true))
                .expect_err("unknown key must be rejected");
        assert!(
            matches!(err, ApplySettingError::UnknownKey { ref key }
                if key == "totally.unknown.setting"),
            "expected UnknownKey, got {err:?}"
        );
        // The write must NOT have happened.
        assert_eq!(get_nested(&json, "totally.unknown.setting"), None);
    }

    #[test]
    fn apply_registered_setting_rejects_type_mismatch() {
        // chat.defaultModel is a string slot — a bool is the wrong type.
        let mut json = serde_json::json!({});
        let err = apply_registered_setting(&mut json, "chat.defaultModel", JsonValue::from(true))
            .expect_err("bool into string slot must be rejected");
        assert!(
            matches!(err, ApplySettingError::TypeMismatch { ref key, .. }
                if key == "chat.defaultModel"),
            "expected TypeMismatch, got {err:?}"
        );
        // The write must NOT have happened.
        assert_eq!(get_nested(&json, "chat.defaultModel"), None);
    }

    #[test]
    fn apply_registered_setting_accepts_compatible_bool() {
        // telemetry.enabled is a bool slot; passing a bool must succeed.
        let mut json = serde_json::json!({});
        apply_registered_setting(&mut json, "telemetry.enabled", JsonValue::from(false))
            .expect("bool into bool slot");
        assert_eq!(
            get_nested(&json, "telemetry.enabled"),
            Some(&JsonValue::from(false))
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

    // -----------------------------------------------------------------------
    // resolve_setting_for_key
    // -----------------------------------------------------------------------

    #[test]
    fn resolve_setting_for_key_returns_some_for_known_key_with_current_value() {
        let json = serde_json::json!({"chat": {"defaultModel": "claude-opus-4"}});
        let entry =
            resolve_setting_for_key(&json, "chat.defaultModel").expect("known key must resolve");
        assert_eq!(entry.key, "chat.defaultModel");
        assert_eq!(
            entry.current_value,
            Some(serde_json::json!("claude-opus-4"))
        );
        assert!(matches!(entry.value_type, SettingValueInfo::String));
    }

    #[test]
    fn resolve_setting_for_key_returns_some_with_none_current_value_for_absent_key() {
        let entry = resolve_setting_for_key(&serde_json::json!({}), "telemetry.enabled")
            .expect("registered key must resolve regardless of file content");
        assert_eq!(entry.key, "telemetry.enabled");
        assert!(
            entry.current_value.is_none(),
            "current_value should be None when the key is absent from JSON"
        );
    }

    #[test]
    fn resolve_setting_for_key_returns_none_for_unknown_key() {
        assert!(resolve_setting_for_key(&serde_json::json!({}), "totally.unknown.key").is_none());
    }

    #[test]
    fn resolve_setting_for_key_returns_none_for_empty_key() {
        assert!(resolve_setting_for_key(&serde_json::json!({}), "").is_none());
    }

    /// Cross-check: every key in `registry()` produces a `SettingEntry`
    /// that is byte-identical between `resolve_settings` (which builds the
    /// full list) and `resolve_setting_for_key` (which builds one). Pins
    /// the duplicated per-entry projection block so a future addition to
    /// `SettingType` updates both arms or fails this test.
    #[test]
    fn resolve_setting_for_key_agrees_with_resolve_settings_for_every_registry_key() {
        let json = serde_json::json!({"chat": {"defaultModel": "claude-opus-4"}});
        let full = resolve_settings(&json);
        for entry in &full {
            let single = resolve_setting_for_key(&json, &entry.key)
                .unwrap_or_else(|| panic!("registered key {} must resolve", entry.key));
            assert_eq!(
                &single, entry,
                "single-key resolver disagreed with full-list resolver for {}",
                entry.key
            );
        }
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

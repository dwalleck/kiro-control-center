# Kiro CLI Settings View — Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add a settings management view to the Kiro Control Center Tauri app for viewing and editing Kiro CLI settings at `~/.kiro/settings/cli.json`, with sidebar category navigation, search, type-appropriate editors, and inline descriptions/defaults.

**Architecture:** A static settings registry in `kiro-market-core` defines all ~40 Kiro CLI settings with metadata (key, label, description, category, type, default). Three Tauri commands (get/set/reset) resolve the registry against the JSON file. Four new Svelte 5 components render the sidebar + panel layout with search and auto-persist editing.

**Tech Stack:** Rust (kiro-market-core + Tauri commands), Svelte 5 ($state/$derived/$props), Tailwind CSS (existing kiro-* theme tokens), tauri-specta (TypeScript binding generation), serde_json (unstructured JSON manipulation).

---

### Task 1: Settings Registry Types

Add the static settings registry types to kiro-market-core. These define the metadata for every Kiro CLI setting.

**Files:**
- Create: `crates/kiro-market-core/src/kiro_settings.rs`
- Modify: `crates/kiro-market-core/src/lib.rs`

**Step 1: Write tests for the registry**

Create `crates/kiro-market-core/src/kiro_settings.rs` with test-first approach. Start with the types and a test that the registry is well-formed:

```rust
//! Kiro CLI settings registry.
//!
//! Defines metadata for all known Kiro CLI settings, enabling the Control
//! Center to render a guided settings editor without hardcoding UI knowledge
//! of each setting.

use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Category grouping for settings in the sidebar.
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
    /// Human-readable label for display.
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

/// Value type of a setting, determines the editor widget in the UI.
#[derive(Debug, Clone, PartialEq)]
pub enum SettingType {
    Bool,
    String,
    Number,
    Char,
    StringArray,
    Enum(Vec<&'static str>),
}

/// Definition of a single Kiro CLI setting.
#[derive(Debug, Clone)]
pub struct SettingDef {
    /// Dotted key path in cli.json (e.g. "chat.defaultModel").
    pub key: &'static str,
    /// Short human-readable label.
    pub label: &'static str,
    /// Description shown below the label.
    pub description: &'static str,
    /// Category for sidebar grouping.
    pub category: SettingCategory,
    /// Value type (determines editor widget).
    pub value_type: SettingType,
    /// Default value (None means no default known).
    pub default: Option<JsonValue>,
}

/// A resolved setting entry with current value, ready for the frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct SettingEntry {
    pub key: String,
    pub label: String,
    pub description: String,
    pub category: String,
    pub category_label: String,
    /// One of: "bool", "string", "number", "char", "string_array", "enum".
    pub value_type: String,
    /// Populated only for enum settings.
    pub enum_options: Vec<String>,
    pub default_value: Option<JsonValue>,
    /// None means using the default (key absent from cli.json).
    pub current_value: Option<JsonValue>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_keys_are_unique() {
        let registry = registry();
        let mut keys: Vec<&str> = registry.iter().map(|d| d.key).collect();
        let len_before = keys.len();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), len_before, "duplicate keys in registry");
    }

    #[test]
    fn registry_has_all_categories() {
        let registry = registry();
        let categories: std::collections::HashSet<SettingCategory> =
            registry.iter().map(|d| d.category).collect();
        // Every category enum variant should have at least one setting.
        assert!(categories.contains(&SettingCategory::Telemetry));
        assert!(categories.contains(&SettingCategory::Chat));
        assert!(categories.contains(&SettingCategory::Knowledge));
        assert!(categories.contains(&SettingCategory::KeyBindings));
        assert!(categories.contains(&SettingCategory::Features));
        assert!(categories.contains(&SettingCategory::Api));
        assert!(categories.contains(&SettingCategory::Mcp));
        assert!(categories.contains(&SettingCategory::Environment));
    }

    #[test]
    fn setting_category_labels_are_nonempty() {
        let categories = [
            SettingCategory::Telemetry,
            SettingCategory::Chat,
            SettingCategory::Knowledge,
            SettingCategory::KeyBindings,
            SettingCategory::Features,
            SettingCategory::Api,
            SettingCategory::Mcp,
            SettingCategory::Environment,
        ];
        for cat in categories {
            assert!(!cat.label().is_empty(), "{cat:?} has empty label");
        }
    }

    #[test]
    fn registry_defaults_match_value_type() {
        let registry = registry();
        for def in &registry {
            if let Some(ref default) = def.default {
                match &def.value_type {
                    SettingType::Bool => assert!(
                        default.is_boolean(),
                        "{}: default should be bool, got {default}",
                        def.key
                    ),
                    SettingType::Number => assert!(
                        default.is_number(),
                        "{}: default should be number, got {default}",
                        def.key
                    ),
                    SettingType::String | SettingType::Char | SettingType::Enum(_) => assert!(
                        default.is_string(),
                        "{}: default should be string, got {default}",
                        def.key
                    ),
                    SettingType::StringArray => assert!(
                        default.is_array(),
                        "{}: default should be array, got {default}",
                        def.key
                    ),
                }
            }
        }
    }
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p kiro-market-core kiro_settings`
Expected: FAIL — `registry()` function doesn't exist yet.

**Step 3: Add the registry function and all setting definitions**

Add below the types in the same file:

```rust
/// The complete registry of known Kiro CLI settings.
///
/// This is the single source of truth for what appears in the settings UI.
/// Add new settings here when Kiro CLI introduces them.
#[must_use]
pub fn registry() -> Vec<SettingDef> {
    vec![
        // -- Telemetry & Privacy --
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
        // -- Chat Interface --
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
            label: "Show Prompt Hints",
            description: "Show startup hints with tips and shortcuts",
            category: SettingCategory::Chat,
            value_type: SettingType::Bool,
            default: Some(JsonValue::Bool(true)),
        },
        SettingDef {
            key: "chat.enableHistoryHints",
            label: "Show History Hints",
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
            label: "Context Usage Indicator",
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
            label: "Compaction Exclude Context %",
            description: "Minimum percentage of context window to retain during compaction",
            category: SettingCategory::Chat,
            value_type: SettingType::Number,
            default: None,
        },
        // -- Knowledge Base --
        SettingDef {
            key: "chat.enableKnowledge",
            label: "Enable Knowledge Base",
            description: "Enable knowledge base functionality",
            category: SettingCategory::Knowledge,
            value_type: SettingType::Bool,
            default: None,
        },
        SettingDef {
            key: "knowledge.defaultIncludePatterns",
            label: "Include Patterns",
            description: "Default file patterns to include in knowledge indexing",
            category: SettingCategory::Knowledge,
            value_type: SettingType::StringArray,
            default: None,
        },
        SettingDef {
            key: "knowledge.defaultExcludePatterns",
            label: "Exclude Patterns",
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
        // -- Key Bindings --
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
        // -- Feature Toggles --
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
        // -- API & Service --
        SettingDef {
            key: "api.timeout",
            label: "API Timeout",
            description: "API request timeout in seconds",
            category: SettingCategory::Api,
            value_type: SettingType::Number,
            default: None,
        },
        // -- MCP --
        SettingDef {
            key: "mcp.initTimeout",
            label: "Init Timeout",
            description: "MCP server initialization timeout in milliseconds",
            category: SettingCategory::Mcp,
            value_type: SettingType::Number,
            default: None,
        },
        SettingDef {
            key: "mcp.noInteractiveTimeout",
            label: "Non-Interactive Timeout",
            description: "Non-interactive MCP timeout in milliseconds",
            category: SettingCategory::Mcp,
            value_type: SettingType::Number,
            default: None,
        },
        SettingDef {
            key: "mcp.loadedBefore",
            label: "Loaded Before",
            description: "Track previously loaded MCP servers",
            category: SettingCategory::Mcp,
            value_type: SettingType::Bool,
            default: None,
        },
        // -- Environment Variables --
        SettingDef {
            key: "KIRO_LOG_NO_COLOR",
            label: "Disable Log Colors",
            description: "Set to disable colored log output",
            category: SettingCategory::Environment,
            value_type: SettingType::Bool,
            default: Some(JsonValue::Bool(false)),
        },
    ]
}
```

**Step 4: Register module in lib.rs**

Add to `crates/kiro-market-core/src/lib.rs`:

```rust
pub mod kiro_settings;
```

**Step 5: Run tests to verify they pass**

Run: `cargo test -p kiro-market-core kiro_settings`
Expected: PASS — all 4 tests green.

**Step 6: Commit**

```bash
git add crates/kiro-market-core/src/kiro_settings.rs crates/kiro-market-core/src/lib.rs
git commit -m "feat(core): add Kiro CLI settings registry types and definitions"
```

---

### Task 2: JSON Path Helpers and Settings Resolution

Add functions to traverse nested JSON by dotted key paths, and to resolve the registry against a `cli.json` file.

**Files:**
- Modify: `crates/kiro-market-core/src/kiro_settings.rs`

**Step 1: Write tests for JSON path helpers**

Add to the `tests` module:

```rust
#[test]
fn get_nested_finds_top_level_key() {
    let json: JsonValue = serde_json::json!({"foo": "bar"});
    assert_eq!(get_nested(&json, "foo"), Some(&JsonValue::String("bar".into())));
}

#[test]
fn get_nested_finds_dotted_path() {
    let json: JsonValue = serde_json::json!({"chat": {"defaultModel": "sonnet"}});
    assert_eq!(
        get_nested(&json, "chat.defaultModel"),
        Some(&JsonValue::String("sonnet".into()))
    );
}

#[test]
fn get_nested_returns_none_for_missing() {
    let json: JsonValue = serde_json::json!({"chat": {}});
    assert_eq!(get_nested(&json, "chat.defaultModel"), None);
}

#[test]
fn set_nested_creates_intermediate_objects() {
    let mut json: JsonValue = serde_json::json!({});
    set_nested(&mut json, "chat.defaultModel", JsonValue::String("sonnet".into()));
    assert_eq!(json, serde_json::json!({"chat": {"defaultModel": "sonnet"}}));
}

#[test]
fn set_nested_overwrites_existing() {
    let mut json: JsonValue = serde_json::json!({"chat": {"defaultModel": "old"}});
    set_nested(&mut json, "chat.defaultModel", JsonValue::String("new".into()));
    assert_eq!(json, serde_json::json!({"chat": {"defaultModel": "new"}}));
}

#[test]
fn remove_nested_deletes_key() {
    let mut json: JsonValue = serde_json::json!({"chat": {"defaultModel": "sonnet", "other": true}});
    remove_nested(&mut json, "chat.defaultModel");
    assert_eq!(json, serde_json::json!({"chat": {"other": true}}));
}

#[test]
fn remove_nested_cleans_empty_parents() {
    let mut json: JsonValue = serde_json::json!({"chat": {"defaultModel": "sonnet"}});
    remove_nested(&mut json, "chat.defaultModel");
    assert_eq!(json, serde_json::json!({}));
}

#[test]
fn remove_nested_noop_for_missing() {
    let mut json: JsonValue = serde_json::json!({"foo": "bar"});
    remove_nested(&mut json, "chat.defaultModel");
    assert_eq!(json, serde_json::json!({"foo": "bar"}));
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test -p kiro-market-core kiro_settings`
Expected: FAIL — `get_nested`, `set_nested`, `remove_nested` don't exist.

**Step 3: Implement the JSON path helpers**

Add above the `registry()` function:

```rust
// ---------------------------------------------------------------------------
// JSON path helpers
// ---------------------------------------------------------------------------

/// Get a value from nested JSON by dotted path (e.g. "chat.defaultModel").
#[must_use]
pub fn get_nested<'a>(value: &'a JsonValue, path: &str) -> Option<&'a JsonValue> {
    let mut current = value;
    for segment in path.split('.') {
        current = current.get(segment)?;
    }
    Some(current)
}

/// Set a value in nested JSON by dotted path, creating intermediate objects.
pub fn set_nested(value: &mut JsonValue, path: &str, val: JsonValue) {
    let segments: Vec<&str> = path.split('.').collect();
    let mut current = value;

    // Navigate/create intermediate objects.
    for &segment in &segments[..segments.len() - 1] {
        if !current.get(segment).is_some_and(JsonValue::is_object) {
            current[segment] = JsonValue::Object(serde_json::Map::new());
        }
        current = current.get_mut(segment).expect("just created");
    }

    if let Some(last) = segments.last() {
        current[*last] = val;
    }
}

/// Remove a key from nested JSON by dotted path.
///
/// Cleans up empty parent objects after removal.
pub fn remove_nested(value: &mut JsonValue, path: &str) {
    let segments: Vec<&str> = path.split('.').collect();
    remove_nested_recursive(value, &segments);
}

fn remove_nested_recursive(value: &mut JsonValue, segments: &[&str]) -> bool {
    let Some((&first, rest)) = segments.split_first() else {
        return false;
    };

    let Some(obj) = value.as_object_mut() else {
        return false;
    };

    if rest.is_empty() {
        // Leaf segment — remove the key.
        obj.remove(first);
    } else if let Some(child) = obj.get_mut(first) {
        // Recurse, then clean up if child became an empty object.
        remove_nested_recursive(child, rest);
        if child.as_object().is_some_and(serde_json::Map::is_empty) {
            obj.remove(first);
        }
    }

    obj.is_empty()
}
```

**Step 4: Run tests to verify they pass**

Run: `cargo test -p kiro-market-core kiro_settings`
Expected: PASS.

**Step 5: Write tests for resolve_settings**

Add to the `tests` module:

```rust
#[test]
fn resolve_settings_uses_defaults_when_no_file() {
    let json = serde_json::json!({});
    let entries = resolve_settings(&json);
    assert!(!entries.is_empty());
    // All current_value should be None.
    for entry in &entries {
        assert!(entry.current_value.is_none(), "{} should have no value", entry.key);
    }
}

#[test]
fn resolve_settings_picks_up_user_values() {
    let json = serde_json::json!({
        "chat": {"defaultModel": "opus"}
    });
    let entries = resolve_settings(&json);
    let model = entries.iter().find(|e| e.key == "chat.defaultModel").expect("should exist");
    assert_eq!(model.current_value, Some(JsonValue::String("opus".into())));
}

#[test]
fn resolve_settings_includes_category_label() {
    let json = serde_json::json!({});
    let entries = resolve_settings(&json);
    let telemetry = entries.iter().find(|e| e.key == "telemetry.enabled").expect("should exist");
    assert_eq!(telemetry.category_label, "Telemetry & Privacy");
}
```

**Step 6: Implement resolve_settings**

```rust
/// Resolve the registry against a loaded JSON value, producing entries
/// with current values filled in.
#[must_use]
pub fn resolve_settings(json: &JsonValue) -> Vec<SettingEntry> {
    registry()
        .into_iter()
        .map(|def| {
            let current_value = get_nested(json, def.key).cloned();
            let value_type_str = match &def.value_type {
                SettingType::Bool => "bool",
                SettingType::String => "string",
                SettingType::Number => "number",
                SettingType::Char => "char",
                SettingType::StringArray => "string_array",
                SettingType::Enum(_) => "enum",
            };
            let enum_options = match &def.value_type {
                SettingType::Enum(opts) => opts.iter().map(|&s| s.to_owned()).collect(),
                _ => Vec::new(),
            };

            SettingEntry {
                key: def.key.to_owned(),
                label: def.label.to_owned(),
                description: def.description.to_owned(),
                category: serde_json::to_value(def.category)
                    .ok()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default(),
                category_label: def.category.label().to_owned(),
                value_type: value_type_str.to_owned(),
                enum_options,
                default_value: def.default,
                current_value,
            }
        })
        .collect()
}
```

**Step 7: Run tests**

Run: `cargo test -p kiro-market-core kiro_settings`
Expected: PASS.

**Step 8: Commit**

```bash
git add crates/kiro-market-core/src/kiro_settings.rs
git commit -m "feat(core): add JSON path helpers and settings resolution"
```

---

### Task 3: Kiro Settings File I/O

Add functions to load and save `~/.kiro/settings/cli.json` as unstructured JSON.

**Files:**
- Modify: `crates/kiro-market-core/src/kiro_settings.rs`

**Step 1: Write tests for file I/O**

```rust
#[test]
fn load_kiro_settings_returns_empty_when_no_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let json = load_kiro_settings_from(dir.path());
    assert_eq!(json, serde_json::json!({}));
}

#[test]
fn save_and_load_kiro_settings_roundtrip() {
    let dir = tempfile::tempdir().expect("tempdir");
    let json = serde_json::json!({"chat": {"defaultModel": "opus"}});
    save_kiro_settings_to(dir.path(), &json).expect("save");
    let loaded = load_kiro_settings_from(dir.path());
    assert_eq!(loaded, json);
}

#[test]
fn save_kiro_settings_creates_parent_dirs() {
    let dir = tempfile::tempdir().expect("tempdir");
    let nested = dir.path().join("deep").join("path");
    let json = serde_json::json!({"foo": "bar"});
    save_kiro_settings_to(&nested, &json).expect("save");
    let loaded = load_kiro_settings_from(&nested);
    assert_eq!(loaded, json);
}

#[test]
fn load_kiro_settings_returns_empty_on_corrupt_json() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(dir.path().join("settings")).expect("mkdir");
    std::fs::write(dir.path().join("settings").join("cli.json"), "{{not json}}")
        .expect("write");
    let json = load_kiro_settings_from(dir.path());
    assert_eq!(json, serde_json::json!({}));
}

#[test]
fn save_kiro_settings_preserves_unknown_keys() {
    let dir = tempfile::tempdir().expect("tempdir");
    let original = serde_json::json!({"unknown_future_setting": true, "chat": {"defaultModel": "opus"}});
    save_kiro_settings_to(dir.path(), &original).expect("save");

    // Modify one key.
    let mut json = load_kiro_settings_from(dir.path());
    set_nested(&mut json, "chat.defaultModel", JsonValue::String("sonnet".into()));
    save_kiro_settings_to(dir.path(), &json).expect("save");

    // Verify unknown key survived.
    let loaded = load_kiro_settings_from(dir.path());
    assert_eq!(loaded["unknown_future_setting"], JsonValue::Bool(true));
    assert_eq!(loaded["chat"]["defaultModel"], JsonValue::String("sonnet".into()));
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test -p kiro-market-core kiro_settings`
Expected: FAIL — `load_kiro_settings_from`, `save_kiro_settings_to` don't exist.

**Step 3: Implement I/O functions**

Add to the file:

```rust
use std::fs;
use std::path::Path;

use tracing::{debug, warn};

// ---------------------------------------------------------------------------
// File I/O
// ---------------------------------------------------------------------------

const KIRO_SETTINGS_PATH: &str = "settings/cli.json";

/// Load `settings/cli.json` from the given Kiro config directory.
///
/// Returns an empty JSON object if the file is missing or corrupt.
#[must_use]
pub fn load_kiro_settings_from(kiro_dir: &Path) -> JsonValue {
    let path = kiro_dir.join(KIRO_SETTINGS_PATH);
    match fs::read(&path) {
        Ok(bytes) => match serde_json::from_slice(&bytes) {
            Ok(v) => v,
            Err(e) => {
                warn!(path = %path.display(), error = %e, "cli.json is corrupt, returning empty");
                JsonValue::Object(serde_json::Map::new())
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            debug!(path = %path.display(), "cli.json not found, returning empty");
            JsonValue::Object(serde_json::Map::new())
        }
        Err(e) => {
            warn!(path = %path.display(), error = %e, "failed to read cli.json");
            JsonValue::Object(serde_json::Map::new())
        }
    }
}

/// Save JSON to `settings/cli.json` under the given Kiro config directory.
///
/// Creates parent directories as needed.
///
/// # Errors
///
/// Returns an I/O error if directory creation or file write fails.
pub fn save_kiro_settings_to(kiro_dir: &Path, json: &JsonValue) -> std::io::Result<()> {
    let path = kiro_dir.join(KIRO_SETTINGS_PATH);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let formatted = serde_json::to_string_pretty(json)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    fs::write(&path, formatted)?;
    debug!(path = %path.display(), "cli.json saved");
    Ok(())
}

/// Resolve the default Kiro config directory (`~/.kiro`).
#[must_use]
pub fn default_kiro_dir() -> Option<std::path::PathBuf> {
    dirs::home_dir().map(|h| h.join(".kiro"))
}
```

**Step 4: Run tests**

Run: `cargo test -p kiro-market-core kiro_settings`
Expected: PASS.

**Step 5: Run clippy**

Run: `cargo clippy -p kiro-market-core -- -D warnings`
Expected: PASS.

**Step 6: Commit**

```bash
git add crates/kiro-market-core/src/kiro_settings.rs
git commit -m "feat(core): add Kiro CLI settings file I/O"
```

---

### Task 4: Tauri Commands for Settings

Add three new Tauri commands that bridge the core settings logic to the frontend.

**Files:**
- Create: `crates/kiro-control-center/src-tauri/src/commands/kiro_settings.rs`
- Modify: `crates/kiro-control-center/src-tauri/src/commands/mod.rs`
- Modify: `crates/kiro-control-center/src-tauri/src/lib.rs`

**Step 1: Create the commands module**

Create `crates/kiro-control-center/src-tauri/src/commands/kiro_settings.rs`:

```rust
//! Kiro CLI settings management: get, set, and reset individual settings.

use kiro_market_core::kiro_settings::{
    self, SettingEntry,
    default_kiro_dir, load_kiro_settings_from, save_kiro_settings_to,
    get_nested, set_nested, remove_nested, resolve_settings, registry,
};
use serde_json::Value as JsonValue;

use crate::error::{CommandError, ErrorType};

/// Helper: resolve the Kiro config directory or return an error.
fn kiro_dir() -> Result<std::path::PathBuf, CommandError> {
    default_kiro_dir().ok_or_else(|| {
        CommandError::new(
            "could not determine home directory",
            ErrorType::IoError,
        )
    })
}

/// Load all Kiro CLI settings, resolved against the registry.
#[tauri::command]
#[specta::specta]
#[allow(clippy::unused_async)]
pub async fn get_kiro_settings() -> Result<Vec<SettingEntry>, CommandError> {
    let dir = kiro_dir()?;
    let json = load_kiro_settings_from(&dir);
    Ok(resolve_settings(&json))
}

/// Update a single Kiro CLI setting by dotted key path.
#[tauri::command]
#[specta::specta]
#[allow(clippy::unused_async)]
pub async fn set_kiro_setting(
    key: String,
    value: JsonValue,
) -> Result<SettingEntry, CommandError> {
    // Validate the key exists in the registry.
    let reg = registry();
    let def = reg.iter().find(|d| d.key == key).ok_or_else(|| {
        CommandError::new(
            format!("unknown setting: {key}"),
            ErrorType::Validation,
        )
    })?;

    let dir = kiro_dir()?;
    let mut json = load_kiro_settings_from(&dir);
    set_nested(&mut json, &key, value);
    save_kiro_settings_to(&dir, &json).map_err(|e| {
        CommandError::new(
            format!("failed to save settings: {e}"),
            ErrorType::IoError,
        )
    })?;

    // Return the updated entry.
    let current_value = get_nested(&json, &key).cloned();
    Ok(SettingEntry {
        key: def.key.to_owned(),
        label: def.label.to_owned(),
        description: def.description.to_owned(),
        category: serde_json::to_value(def.category)
            .ok()
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_default(),
        category_label: def.category.label().to_owned(),
        value_type: match &def.value_type {
            kiro_settings::SettingType::Bool => "bool",
            kiro_settings::SettingType::String => "string",
            kiro_settings::SettingType::Number => "number",
            kiro_settings::SettingType::Char => "char",
            kiro_settings::SettingType::StringArray => "string_array",
            kiro_settings::SettingType::Enum(_) => "enum",
        }.to_owned(),
        enum_options: match &def.value_type {
            kiro_settings::SettingType::Enum(opts) => opts.iter().map(|&s| s.to_owned()).collect(),
            _ => Vec::new(),
        },
        default_value: def.default.clone(),
        current_value,
    })
}

/// Reset a Kiro CLI setting to its default (remove from cli.json).
#[tauri::command]
#[specta::specta]
#[allow(clippy::unused_async)]
pub async fn reset_kiro_setting(key: String) -> Result<(), CommandError> {
    // Validate the key exists.
    let reg = registry();
    if !reg.iter().any(|d| d.key == key) {
        return Err(CommandError::new(
            format!("unknown setting: {key}"),
            ErrorType::Validation,
        ));
    }

    let dir = kiro_dir()?;
    let mut json = load_kiro_settings_from(&dir);
    remove_nested(&mut json, &key);
    save_kiro_settings_to(&dir, &json).map_err(|e| {
        CommandError::new(
            format!("failed to save settings: {e}"),
            ErrorType::IoError,
        )
    })?;
    Ok(())
}
```

**Step 2: Register the module**

Add to `crates/kiro-control-center/src-tauri/src/commands/mod.rs`:

```rust
pub mod kiro_settings;
```

**Step 3: Register commands in lib.rs**

Add to the `collect_commands!` macro in `crates/kiro-control-center/src-tauri/src/lib.rs`:

```rust
commands::kiro_settings::get_kiro_settings,
commands::kiro_settings::set_kiro_setting,
commands::kiro_settings::reset_kiro_setting,
```

**Step 4: Build to verify compilation and generate bindings**

Run: `cargo build -p kiro-control-center`
Expected: PASS.

**Step 5: Regenerate TypeScript bindings**

Run: `cargo test -p kiro-control-center generate_types -- --exact --ignored`
Expected: `bindings.ts` regenerated with `getKiroSettings`, `setKiroSetting`, `resetKiroSetting`.

**Step 6: Verify the new types appear in bindings.ts**

Run: `grep -n "kiro_setting\|KiroSetting\|SettingEntry" crates/kiro-control-center/src/lib/bindings.ts`
Expected: Shows the new command functions and `SettingEntry` type.

**Step 7: Commit**

```bash
git add crates/kiro-control-center/src-tauri/src/commands/kiro_settings.rs \
        crates/kiro-control-center/src-tauri/src/commands/mod.rs \
        crates/kiro-control-center/src-tauri/src/lib.rs \
        crates/kiro-control-center/src/lib/bindings.ts
git commit -m "feat(tauri): add get/set/reset Kiro CLI settings commands"
```

---

### Task 5: SettingControl Component

The leaf component that renders a single setting row with a type-appropriate editor.

**Files:**
- Create: `crates/kiro-control-center/src/lib/components/SettingControl.svelte`

**Step 1: Create the component**

Create `crates/kiro-control-center/src/lib/components/SettingControl.svelte`:

```svelte
<script lang="ts">
  import { commands } from "$lib/bindings";
  import type { SettingEntry } from "$lib/bindings";

  let { entry, onUpdate }: {
    entry: SettingEntry;
    onUpdate: (updated: SettingEntry) => void;
  } = $props();

  let saving = $state(false);
  let error: string | null = $state(null);

  // Chips input state for string_array type.
  let chipInput = $state("");

  let isModified = $derived(entry.current_value !== null && entry.current_value !== undefined);

  let displayValue = $derived(entry.current_value ?? entry.default_value);

  async function setValue(value: unknown) {
    saving = true;
    error = null;
    const result = await commands.setKiroSetting(entry.key, value as any);
    if (result.status === "ok") {
      onUpdate(result.data);
    } else {
      error = result.error.message;
    }
    saving = false;
  }

  async function resetValue() {
    saving = true;
    error = null;
    const result = await commands.resetKiroSetting(entry.key);
    if (result.status === "ok") {
      // Reset current_value to null (use default).
      onUpdate({ ...entry, current_value: null });
    } else {
      error = result.error.message;
    }
    saving = false;
  }

  function handleBoolToggle() {
    const current = displayValue as boolean | null;
    setValue(!current);
  }

  function handleStringChange(e: Event) {
    const target = e.target as HTMLInputElement;
    setValue(target.value);
  }

  function handleNumberChange(e: Event) {
    const target = e.target as HTMLInputElement;
    const num = Number(target.value);
    if (!Number.isNaN(num)) {
      setValue(num);
    }
  }

  function handleCharChange(e: Event) {
    const target = e.target as HTMLInputElement;
    // Take only the last character typed.
    const char = target.value.slice(-1);
    target.value = char;
    if (char) {
      setValue(char);
    }
  }

  function handleEnumChange(e: Event) {
    const target = e.target as HTMLSelectElement;
    setValue(target.value);
  }

  function addChip() {
    const val = chipInput.trim();
    if (!val) return;
    const current = (displayValue as string[] | null) ?? [];
    if (!current.includes(val)) {
      setValue([...current, val]);
    }
    chipInput = "";
  }

  function removeChip(chip: string) {
    const current = (displayValue as string[] | null) ?? [];
    setValue(current.filter((c: string) => c !== chip));
  }

  function handleChipKeydown(e: KeyboardEvent) {
    if (e.key === "Enter") {
      e.preventDefault();
      addChip();
    }
  }
</script>

<div class="flex items-start justify-between gap-4 py-3 px-4 rounded-lg hover:bg-kiro-overlay/50 transition-colors">
  <div class="flex-1 min-w-0">
    <div class="flex items-center gap-2">
      <span class="text-sm font-medium text-kiro-text">{entry.label}</span>
      {#if isModified}
        <span class="inline-flex items-center px-1.5 py-0.5 text-[10px] font-medium rounded bg-kiro-accent-900/20 text-kiro-accent-400">
          Modified
        </span>
      {/if}
    </div>
    <p class="mt-0.5 text-xs text-kiro-subtle">{entry.description}</p>
    <p class="mt-0.5 text-[10px] text-kiro-subtle/60 font-mono">{entry.key}</p>
    {#if error}
      <p class="mt-1 text-xs text-kiro-error">{error}</p>
    {/if}
  </div>

  <div class="flex items-center gap-2 flex-shrink-0">
    {#if entry.value_type === "bool"}
      <button
        class="relative inline-flex h-5 w-9 items-center rounded-full transition-colors
          {displayValue ? 'bg-kiro-accent-500' : 'bg-kiro-muted'}"
        onclick={handleBoolToggle}
        disabled={saving}
        aria-label="Toggle {entry.label}"
      >
        <span
          class="inline-block h-3.5 w-3.5 rounded-full bg-white transition-transform
            {displayValue ? 'translate-x-4' : 'translate-x-0.5'}"
        />
      </button>

    {:else if entry.value_type === "string"}
      <input
        type="text"
        value={displayValue ?? ""}
        onchange={handleStringChange}
        disabled={saving}
        placeholder={entry.default_value?.toString() ?? ""}
        class="w-48 px-2 py-1 text-sm rounded border border-kiro-muted bg-kiro-overlay text-kiro-text placeholder-kiro-subtle/50 focus:outline-none focus:ring-1 focus:ring-kiro-accent-500"
      />

    {:else if entry.value_type === "number"}
      <input
        type="number"
        value={displayValue ?? ""}
        onchange={handleNumberChange}
        disabled={saving}
        placeholder={entry.default_value?.toString() ?? ""}
        class="w-28 px-2 py-1 text-sm rounded border border-kiro-muted bg-kiro-overlay text-kiro-text placeholder-kiro-subtle/50 focus:outline-none focus:ring-1 focus:ring-kiro-accent-500"
      />

    {:else if entry.value_type === "char"}
      <input
        type="text"
        value={displayValue ?? ""}
        oninput={handleCharChange}
        disabled={saving}
        maxlength={1}
        class="w-12 px-2 py-1 text-sm text-center rounded border border-kiro-muted bg-kiro-overlay text-kiro-text focus:outline-none focus:ring-1 focus:ring-kiro-accent-500 font-mono"
      />

    {:else if entry.value_type === "enum"}
      <select
        value={displayValue ?? ""}
        onchange={handleEnumChange}
        disabled={saving}
        class="w-48 px-2 py-1 text-sm rounded border border-kiro-muted bg-kiro-overlay text-kiro-text focus:outline-none focus:ring-1 focus:ring-kiro-accent-500"
      >
        <option value="">Default</option>
        {#each entry.enum_options as opt (opt)}
          <option value={opt}>{opt}</option>
        {/each}
      </select>

    {:else if entry.value_type === "string_array"}
      <div class="w-64">
        <div class="flex flex-wrap gap-1 mb-1">
          {#each (displayValue as string[] ?? []) as chip (chip)}
            <span class="inline-flex items-center gap-1 px-2 py-0.5 text-xs rounded-full bg-kiro-muted text-kiro-text-secondary">
              {chip}
              <button
                class="text-kiro-subtle hover:text-kiro-error"
                onclick={() => removeChip(chip)}
                aria-label="Remove {chip}"
              >x</button>
            </span>
          {/each}
        </div>
        <div class="flex gap-1">
          <input
            type="text"
            bind:value={chipInput}
            onkeydown={handleChipKeydown}
            disabled={saving}
            placeholder="Add pattern..."
            class="flex-1 px-2 py-1 text-xs rounded border border-kiro-muted bg-kiro-overlay text-kiro-text placeholder-kiro-subtle/50 focus:outline-none focus:ring-1 focus:ring-kiro-accent-500"
          />
          <button
            class="px-2 py-1 text-xs rounded border border-kiro-muted text-kiro-text-secondary hover:bg-kiro-overlay"
            onclick={addChip}
            disabled={saving || !chipInput.trim()}
          >+</button>
        </div>
      </div>
    {/if}

    {#if isModified}
      <button
        class="text-xs text-kiro-subtle hover:text-kiro-accent-400 transition-colors"
        onclick={resetValue}
        disabled={saving}
        title="Reset to default"
      >
        <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15" />
        </svg>
      </button>
    {/if}
  </div>
</div>
```

**Step 2: Commit**

```bash
git add crates/kiro-control-center/src/lib/components/SettingControl.svelte
git commit -m "feat(ui): add SettingControl component with type-appropriate editors"
```

---

### Task 6: CategoryList Component

The sidebar listing all setting categories with counts and active state.

**Files:**
- Create: `crates/kiro-control-center/src/lib/components/CategoryList.svelte`

**Step 1: Create the component**

Create `crates/kiro-control-center/src/lib/components/CategoryList.svelte`:

```svelte
<script lang="ts">
  let { categories, activeCategory, onSelect, matchCounts }: {
    categories: { key: string; label: string; count: number }[];
    activeCategory: string;
    onSelect: (key: string) => void;
    matchCounts: Record<string, number> | null;
  } = $props();
</script>

<nav class="w-48 flex-shrink-0 border-r border-kiro-muted bg-kiro-surface overflow-y-auto p-3">
  <h3 class="text-xs font-semibold text-kiro-subtle uppercase tracking-wider mb-2">
    Categories
  </h3>
  {#each categories as cat (cat.key)}
    {@const isActive = activeCategory === cat.key}
    {@const searchCount = matchCounts?.[cat.key]}
    {@const dimmed = matchCounts !== null && (searchCount === undefined || searchCount === 0)}
    <button
      class="w-full text-left px-3 py-2 text-sm rounded-md transition-colors duration-100 mb-0.5
        {isActive
          ? 'bg-kiro-muted text-kiro-text font-medium'
          : dimmed
            ? 'text-kiro-subtle/40'
            : 'text-kiro-text-secondary hover:bg-kiro-overlay'}"
      onclick={() => onSelect(cat.key)}
      disabled={dimmed}
    >
      <span class="flex items-center justify-between">
        <span class="truncate">{cat.label}</span>
        <span class="text-xs {isActive ? 'text-kiro-accent-400' : 'text-kiro-subtle'}">
          {matchCounts !== null ? (searchCount ?? 0) : cat.count}
        </span>
      </span>
    </button>
  {/each}
</nav>
```

**Step 2: Commit**

```bash
git add crates/kiro-control-center/src/lib/components/CategoryList.svelte
git commit -m "feat(ui): add CategoryList sidebar component"
```

---

### Task 7: SettingsPanel Component

The main content area that renders a list of `SettingControl` components.

**Files:**
- Create: `crates/kiro-control-center/src/lib/components/SettingsPanel.svelte`

**Step 1: Create the component**

Create `crates/kiro-control-center/src/lib/components/SettingsPanel.svelte`:

```svelte
<script lang="ts">
  import type { SettingEntry } from "$lib/bindings";
  import SettingControl from "./SettingControl.svelte";

  let { entries, showCategoryHeaders, onUpdate }: {
    entries: SettingEntry[];
    showCategoryHeaders: boolean;
    onUpdate: (updated: SettingEntry) => void;
  } = $props();

  // Group entries by category_label for rendering headers.
  let grouped = $derived.by(() => {
    if (!showCategoryHeaders) {
      return [{ label: "", entries }];
    }
    const groups: { label: string; entries: SettingEntry[] }[] = [];
    let currentLabel = "";
    for (const entry of entries) {
      if (entry.category_label !== currentLabel) {
        currentLabel = entry.category_label;
        groups.push({ label: currentLabel, entries: [] });
      }
      groups[groups.length - 1].entries.push(entry);
    }
    return groups;
  });
</script>

<div class="flex-1 overflow-y-auto px-2 py-3">
  {#if entries.length === 0}
    <div class="flex items-center justify-center h-full text-kiro-subtle">
      <p class="text-sm">No settings match the search</p>
    </div>
  {:else}
    {#each grouped as group (group.label)}
      {#if group.label}
        <h3 class="text-xs font-semibold text-kiro-subtle uppercase tracking-wider px-4 pt-4 pb-2">
          {group.label}
        </h3>
      {/if}
      <div class="space-y-1">
        {#each group.entries as entry (entry.key)}
          <SettingControl {entry} {onUpdate} />
        {/each}
      </div>
    {/each}
  {/if}
</div>
```

**Step 2: Commit**

```bash
git add crates/kiro-control-center/src/lib/components/SettingsPanel.svelte
git commit -m "feat(ui): add SettingsPanel component"
```

---

### Task 8: SettingsView Component

The top-level settings layout that owns all state: loading, search, category navigation.

**Files:**
- Create: `crates/kiro-control-center/src/lib/components/SettingsView.svelte`

**Step 1: Create the component**

Create `crates/kiro-control-center/src/lib/components/SettingsView.svelte`:

```svelte
<script lang="ts">
  import { onMount } from "svelte";
  import { commands } from "$lib/bindings";
  import type { SettingEntry } from "$lib/bindings";
  import CategoryList from "./CategoryList.svelte";
  import SettingsPanel from "./SettingsPanel.svelte";

  let { onClose }: { onClose: () => void } = $props();

  let allEntries: SettingEntry[] = $state([]);
  let loading = $state(true);
  let loadError: string | null = $state(null);
  let searchQuery = $state("");
  let activeCategory = $state("");

  // Build categories from loaded entries.
  let categories = $derived.by(() => {
    const seen = new Map<string, { key: string; label: string; count: number }>();
    for (const e of allEntries) {
      const existing = seen.get(e.category);
      if (existing) {
        existing.count++;
      } else {
        seen.set(e.category, { key: e.category, label: e.category_label, count: 1 });
      }
    }
    return Array.from(seen.values());
  });

  // Filter by search query.
  let filteredEntries = $derived.by(() => {
    if (!searchQuery.trim()) return allEntries;
    const q = searchQuery.toLowerCase();
    return allEntries.filter(
      (e) =>
        e.label.toLowerCase().includes(q) ||
        e.description.toLowerCase().includes(q) ||
        e.key.toLowerCase().includes(q)
    );
  });

  // During search, compute per-category match counts.
  let matchCounts: Record<string, number> | null = $derived.by(() => {
    if (!searchQuery.trim()) return null;
    const counts: Record<string, number> = {};
    for (const e of filteredEntries) {
      counts[e.category] = (counts[e.category] ?? 0) + 1;
    }
    return counts;
  });

  // Entries to display in the panel.
  let displayEntries = $derived.by(() => {
    if (searchQuery.trim()) {
      // During search, show all matches grouped by category.
      return filteredEntries;
    }
    // Otherwise, show only the active category.
    return allEntries.filter((e) => e.category === activeCategory);
  });

  let showCategoryHeaders = $derived(!!searchQuery.trim());

  function handleUpdate(updated: SettingEntry) {
    allEntries = allEntries.map((e) => (e.key === updated.key ? updated : e));
  }

  onMount(async () => {
    const result = await commands.getKiroSettings();
    if (result.status === "ok") {
      allEntries = result.data;
      if (result.data.length > 0) {
        activeCategory = result.data[0].category;
      }
    } else {
      loadError = result.error.message;
    }
    loading = false;
  });
</script>

<div class="flex flex-col h-full bg-kiro-base text-kiro-text">
  <!-- Top bar: back + search -->
  <div class="flex items-center gap-4 px-4 py-3 border-b border-kiro-muted bg-kiro-surface">
    <button
      class="text-sm text-kiro-accent-400 hover:text-kiro-accent-300 transition-colors flex items-center gap-1"
      onclick={onClose}
    >
      <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M15 19l-7-7 7-7" />
      </svg>
      Back
    </button>
    <input
      type="text"
      placeholder="Search settings..."
      bind:value={searchQuery}
      class="flex-1 px-3 py-1.5 text-sm rounded-md border border-kiro-muted bg-kiro-overlay text-kiro-text placeholder-kiro-subtle focus:outline-none focus:ring-2 focus:ring-kiro-accent-500 focus:border-transparent"
    />
  </div>

  {#if loading}
    <div class="flex items-center justify-center flex-1">
      <p class="text-sm text-kiro-subtle">Loading settings...</p>
    </div>
  {:else if loadError}
    <div class="flex items-center justify-center flex-1 p-4">
      <div class="max-w-md text-center">
        <p class="text-sm text-kiro-error mb-2">Failed to load settings</p>
        <p class="text-xs text-kiro-subtle">{loadError}</p>
      </div>
    </div>
  {:else}
    <div class="flex flex-1 overflow-hidden">
      <CategoryList
        {categories}
        {activeCategory}
        {matchCounts}
        onSelect={(key) => (activeCategory = key)}
      />
      <SettingsPanel
        entries={displayEntries}
        {showCategoryHeaders}
        onUpdate={handleUpdate}
      />
    </div>
  {/if}
</div>
```

**Step 2: Commit**

```bash
git add crates/kiro-control-center/src/lib/components/SettingsView.svelte
git commit -m "feat(ui): add SettingsView layout with sidebar and search"
```

---

### Task 9: Wire Settings into the Main Page

Add the gear icon to the header and the `showSettings` toggle to `+page.svelte`.

**Files:**
- Modify: `crates/kiro-control-center/src/routes/+page.svelte`

**Step 1: Add import and state**

Add `SettingsView` import alongside the other imports:

```svelte
import SettingsView from "$lib/components/SettingsView.svelte";
```

Add state variable:

```svelte
let showSettings = $state(false);
```

**Step 2: Add gear icon to the header**

In the `<header>` element, add a gear button between the title and `ProjectDropdown`:

```svelte
<header class="flex items-center justify-between px-6 py-3 bg-kiro-surface border-b border-kiro-muted shadow-sm">
  <h1 class="text-lg font-semibold">Kiro Control Center</h1>
  <div class="flex items-center gap-3">
    <button
      class="text-kiro-subtle hover:text-kiro-text-secondary transition-colors"
      onclick={() => (showSettings = true)}
      title="Kiro CLI Settings"
    >
      <svg class="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M10.325 4.317c.426-1.756 2.924-1.756 3.35 0a1.724 1.724 0 002.573 1.066c1.543-.94 3.31.826 2.37 2.37a1.724 1.724 0 001.066 2.573c1.756.426 1.756 2.924 0 3.35a1.724 1.724 0 00-1.066 2.573c.94 1.543-.826 3.31-2.37 2.37a1.724 1.724 0 00-2.573 1.066c-.426 1.756-2.924 1.756-3.35 0a1.724 1.724 0 00-2.573-1.066c-1.543.94-3.31-.826-2.37-2.37a1.724 1.724 0 00-1.066-2.573c-1.756-.426-1.756-2.924 0-3.35a1.724 1.724 0 001.066-2.573c-.94-1.543.826-3.31 2.37-2.37.996.608 2.296.07 2.572-1.065z" />
        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M15 12a3 3 0 11-6 0 3 3 0 016 0z" />
      </svg>
    </button>
    <ProjectDropdown onManageRoots={() => (showManageRoots = true)} />
  </div>
</header>
```

**Step 3: Add conditional rendering**

When `showSettings` is true, replace the tab bar + main content with `SettingsView`. Wrap the existing `{#if store.projectPath}` block so the settings view takes over the full area below the header:

The project-selected block should become:

```svelte
{:else if store.projectPath}
  <div class="flex flex-col h-screen bg-kiro-base text-kiro-text">
    <header class="flex items-center justify-between px-6 py-3 bg-kiro-surface border-b border-kiro-muted shadow-sm">
      <h1 class="text-lg font-semibold">Kiro Control Center</h1>
      <div class="flex items-center gap-3">
        <button
          class="text-kiro-subtle hover:text-kiro-text-secondary transition-colors"
          onclick={() => (showSettings = true)}
          title="Kiro CLI Settings"
        >
          <svg class="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M10.325 4.317c.426-1.756 2.924-1.756 3.35 0a1.724 1.724 0 002.573 1.066c1.543-.94 3.31.826 2.37 2.37a1.724 1.724 0 001.066 2.573c1.756.426 1.756 2.924 0 3.35a1.724 1.724 0 00-1.066 2.573c.94 1.543-.826 3.31-2.37 2.37a1.724 1.724 0 00-2.573 1.066c-.426 1.756-2.924 1.756-3.35 0a1.724 1.724 0 00-2.573-1.066c-1.543.94-3.31-.826-2.37-2.37a1.724 1.724 0 00-1.066-2.573c-1.756-.426-1.756-2.924 0-3.35a1.724 1.724 0 001.066-2.573c-.94-1.543.826-3.31 2.37-2.37.996.608 2.296.07 2.572-1.065z" />
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M15 12a3 3 0 11-6 0 3 3 0 016 0z" />
          </svg>
        </button>
        <ProjectDropdown onManageRoots={() => (showManageRoots = true)} />
      </div>
    </header>

    {#if showSettings}
      <SettingsView onClose={() => (showSettings = false)} />
    {:else}
      <TabBar {tabs} {activeTab} onTabChange={(tab) => (activeTab = tab)} />

      <main class="flex-1 overflow-hidden">
        {#if activeTab === "Browse"}
          <BrowseTab projectPath={store.projectPath} />
        {:else if activeTab === "Installed"}
          <InstalledTab projectPath={store.projectPath} />
        {:else if activeTab === "Marketplaces"}
          <MarketplacesTab />
        {/if}
      </main>
    {/if}
  </div>
```

**Step 4: Build frontend**

Run: `cd crates/kiro-control-center && npm run check`
Expected: No type errors.

**Step 5: Commit**

```bash
git add crates/kiro-control-center/src/routes/+page.svelte
git commit -m "feat(ui): wire settings view into main page with gear icon"
```

---

### Task 10: Manual Testing and Polish

Start the dev server and verify the full flow works.

**Step 1: Start the app**

Run: `cd crates/kiro-control-center && cargo tauri dev`

**Step 2: Test checklist**

- [ ] Gear icon visible in header
- [ ] Clicking gear shows settings view (replaces tabs)
- [ ] "Back" button returns to marketplace tabs
- [ ] Categories listed in sidebar with correct counts
- [ ] Clicking categories shows correct settings
- [ ] Search filters across all categories
- [ ] Search highlights matching categories, dims others
- [ ] Boolean toggles persist immediately
- [ ] String inputs persist on blur/change
- [ ] Number inputs persist on change
- [ ] "Modified" badge appears on changed settings
- [ ] Reset button removes the value and clears "Modified" badge
- [ ] Settings survive app restart (check `~/.kiro/settings/cli.json`)
- [ ] App handles missing cli.json gracefully (all defaults)

**Step 3: Fix any issues found**

Apply fixes as needed.

**Step 4: Final commit**

```bash
git add -A
git commit -m "fix(ui): polish settings view from manual testing"
```

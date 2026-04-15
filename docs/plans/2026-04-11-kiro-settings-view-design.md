# Kiro CLI Settings View — Design

## Overview

Add a settings management view to the Kiro Control Center (Tauri desktop app)
for viewing and editing Kiro CLI settings stored at `~/.kiro/settings/cli.json`.
The view uses a sidebar category navigation pattern with search, type-appropriate
editors, and inline descriptions/defaults.

## Entry Point & Navigation

A gear icon in the header bar (left of the project dropdown) opens the settings
view. The view replaces the tab bar and main content area — it is a full-screen
mode, not a 4th tab. This reinforces that settings are user-global, not
project-scoped. A "Back to Marketplace" link returns to the previous tab.

```
┌──────────────────────────────────────────────────────┐
│ Kiro Control Center                ⚙️  [Project ▼]  │
├──────────────────────────────────────────────────────┤
│ ← Back to Marketplace    🔍 Search settings...       │
├───────────────┬──────────────────────────────────────┤
│  Telemetry    │  Chat Interface                      │
│  Chat      ◀──│                                      │
│  Knowledge    │  Default Model              [sonnet] │
│  Key Bindings │    AI model for new conversations    │
│  Features     │                                      │
│  API          │  Enable Notifications           ⬜   │
│  MCP          │    Desktop notifications             │
│  Env Vars     │                                      │
└───────────────┴──────────────────────────────────────┘
```

## Settings Data Model

### Static Registry (Rust)

Settings are defined as a static registry in `kiro-market-core`. Each entry
carries everything the UI needs:

```rust
pub struct SettingDef {
    pub key: &'static str,           // "chat.defaultModel"
    pub label: &'static str,         // "Default Model"
    pub description: &'static str,   // "AI model for new conversations"
    pub category: SettingCategory,   // SettingCategory::Chat
    pub value_type: SettingType,     // SettingType::String
    pub default: Option<JsonValue>,  // Some("sonnet")
}

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

pub enum SettingType {
    Bool,
    String,
    Number,
    Char,
    StringArray,
    Enum(Vec<&'static str>),
}
```

### File Format

`~/.kiro/settings/cli.json` is unstructured JSON — a nested map. The registry
gives it structure for the UI without imposing a rigid Rust struct. Dotted keys
like `chat.defaultModel` map to `{"chat": {"defaultModel": ...}}`.

## Rust Backend (Tauri Commands)

Three new Tauri commands:

### `get_kiro_settings`

Returns the full settings state. The backend loads `cli.json`, walks the
registry, and returns a `Vec<SettingEntry>` with current values resolved:

```rust
pub struct SettingEntry {
    pub key: String,
    pub label: String,
    pub description: String,
    pub category: String,
    pub value_type: String,        // "bool", "string", "number", "char", "string_array", "enum"
    pub enum_options: Vec<String>,
    pub default_value: Option<JsonValue>,
    pub current_value: Option<JsonValue>,
}
```

The frontend groups by `category` for rendering — no dotted-path JSON
traversal needed on the Svelte side.

### `set_kiro_setting(key, value)`

Loads `cli.json`, sets the value at the dotted path, writes back. Returns the
updated `SettingEntry`.

### `reset_kiro_setting(key)`

Removes a key from `cli.json` so Kiro CLI falls back to its built-in default.

## Svelte Frontend Components

### SettingsView.svelte

Top-level layout. Owns sidebar + main panel split and search state. Calls
`get_kiro_settings` on mount, groups results by category. Manages
`activeCategory` and `searchQuery`. During search, shows filtered results
across all categories with category headers.

### CategoryList.svelte

The sidebar. Vertical list of category buttons with count badges. Active
category highlighted. During search, categories with no matches are dimmed.

### SettingsPanel.svelte

Main content area. Renders `SettingControl` components for the active category
or search results. Each setting shows label, description, editor widget, a
"modified" indicator if value differs from default, and a "reset to default"
action.

### SettingControl.svelte

Single setting row. Switches on `value_type` for the appropriate editor:

| Type           | Widget                              |
|----------------|-------------------------------------|
| `bool`         | Toggle switch                       |
| `string`       | Text input                          |
| `number`       | Number input with increment/decrement |
| `char`         | Single-character input              |
| `string_array` | Chip list with add/remove           |
| `enum`         | Dropdown select                     |

Changes auto-persist immediately (no save button). Optimistic UI update with
error toast on failure.

### Integration

A `showSettings` boolean in `+page.svelte` (like existing `showManageRoots`).
The gear icon toggles it. When true, `SettingsView` replaces the tab bar and
main content.

## Error Handling

- **File missing:** Return all settings with `current_value: None`. First write
  creates the file and parent directories.
- **Malformed JSON:** Show error banner. Do not silently overwrite.
- **Unknown keys:** Ignore in the UI but preserve on write. Never drop
  unrecognized keys.
- **Concurrent edits:** No file watching. Re-read on each settings view open.
  Last-write-wins on conflict.
- **Registry staleness:** Compiled into the binary. Unknown-to-registry keys in
  the file are invisible but preserved.

## Settings Inventory (from kiro.dev/docs/cli/reference/settings/)

### Telemetry & Privacy (2)
- `telemetry.enabled` (bool) — Enable/disable telemetry collection
- `telemetryClientId` (string) — Client identifier for telemetry

### Chat Interface (12)
- `chat.defaultModel` (string) — Default AI model for conversations
- `chat.defaultAgent` (string) — Default agent configuration
- `chat.diffTool` (string) — External diff tool for viewing code changes
- `chat.greeting.enabled` (bool) — Show greeting message on chat start
- `chat.editMode` (bool) — Enable edit mode for chat interface
- `chat.enableNotifications` (bool) — Enable desktop notifications
- `chat.disableMarkdownRendering` (bool) — Disable markdown formatting
- `chat.disableAutoCompaction` (bool) — Disable automatic conversation summarization
- `chat.enablePromptHints` (bool, default: true) — Show startup hints
- `chat.enableHistoryHints` (bool) — Show conversation history hints
- `chat.uiMode` (string) — UI variant to use
- `chat.enableContextUsageIndicator` (bool) — Show context usage percentage
- `compaction.excludeMessages` (number) — Min message pairs to retain during compaction
- `compaction.excludeContextWindowPercent` (number) — Min % of context window to retain

### Knowledge Base (7)
- `chat.enableKnowledge` (bool) — Enable knowledge base functionality
- `knowledge.defaultIncludePatterns` (string_array) — Default file patterns to include
- `knowledge.defaultExcludePatterns` (string_array) — Default file patterns to exclude
- `knowledge.maxFiles` (number) — Maximum files for indexing
- `knowledge.chunkSize` (number) — Text chunk size for processing
- `knowledge.chunkOverlap` (number) — Overlap between text chunks
- `knowledge.indexType` (string) — Type of knowledge index

### Key Bindings (4)
- `chat.skimCommandKey` (char) — Key for fuzzy search command
- `chat.autocompletionKey` (char) — Key for autocompletion hint acceptance
- `chat.tangentModeKey` (char) — Key for tangent mode toggle
- `chat.delegateModeKey` (char) — Key for delegate command

### Feature Toggles (5)
- `chat.enableThinking` (bool) — Enable thinking tool for complex reasoning
- `chat.enableTangentMode` (bool) — Enable tangent mode feature
- `introspect.tangentMode` (bool) — Auto-enter tangent mode for introspect
- `chat.enableTodoList` (bool) — Enable todo list feature
- `chat.enableCheckpoint` (bool) — Enable checkpoint feature
- `chat.enableDelegate` (bool) — Enable delegate tool

### API & Service (1)
- `api.timeout` (number) — API request timeout in seconds

### MCP (3)
- `mcp.initTimeout` (number) — MCP server initialization timeout
- `mcp.noInteractiveTimeout` (number) — Non-interactive MCP timeout
- `mcp.loadedBefore` (bool) — Track previously loaded MCP servers

### Environment Variables (1)
- `KIRO_LOG_NO_COLOR` (bool-like) — Set to 1 to disable colored log output

# Interfaces

<!-- tags: api, ipc, cli, interfaces -->

## Tauri IPC Commands

All commands registered in `src-tauri/src/lib.rs` via `tauri-specta`. TypeScript bindings auto-generated to `src/lib/bindings.ts`. Commands return `Result<T, CommandError>`.

### Browse (`commands/browse.rs`)

| Command | Key Parameters | Returns |
|---|---|---|
| `list_marketplaces` | — | `Vec<MarketplaceInfo>` |
| `list_plugins` | `marketplace: String` | `Vec<PluginInfo>` |
| `list_available_skills` | `marketplace, plugin: String` | `PluginSkillsResult` |
| `list_all_skills_for_marketplace` | `marketplace: String` | `BulkSkillsResult` |
| `list_plugin_catalog_for_marketplace` | `marketplace, project_path: String` | `PluginCatalogResponseView` |
| `install_skills` | `marketplace, plugin, project_path: String; skill_names: Vec<String>; force: bool` | `InstallSkillsResult` |
| `get_project_info` | `project_path: String` | `ProjectInfo` |

### Installed (`commands/installed.rs`)

| Command | Key Parameters | Returns |
|---|---|---|
| `list_installed_skills` | `project_path: String` | `Vec<InstalledSkillInfo>` |
| `remove_skill` | `project_path, skill_name: String` | `()` |

### Plugins (`commands/plugins.rs`)

| Command | Key Parameters | Returns |
|---|---|---|
| `install_plugin` | `marketplace, plugin, project_path: String; force, accept_mcp: bool` | `InstallPluginResult` |
| `list_installed_plugins` | `project_path: String` | `InstalledPluginsView` |
| `remove_plugin` | `marketplace, plugin, project_path: String` | `RemovePluginResult` |
| `detect_plugin_updates` | `project_path: String` | `DetectUpdatesResult` |

### Agents (`commands/agents.rs`)

| Command | Key Parameters | Returns |
|---|---|---|
| `install_plugin_agents` | `marketplace, plugin, project_path: String; force, accept_mcp: bool; names: Option<Vec<String>>` | `InstallAgentsResult` |
| `install_agents` | `marketplace, plugin, project_path: String; agent_names: Vec<String>; force, accept_mcp: bool` | `InstallAgentsResult` |
| `remove_agent` | `project_path, agent_name: String` | `()` |

### Steering (`commands/steering.rs`)

| Command | Key Parameters | Returns |
|---|---|---|
| `install_plugin_steering` | `marketplace, plugin, project_path: String; force: bool; names: Option<Vec<String>>` | `InstallSteeringResult` |
| `install_steering_files` | `marketplace, plugin, project_path: String; file_names: Vec<String>; force: bool` | `InstallSteeringResult` |
| `remove_steering_file` | `project_path, rel: String` | `()` |

### Marketplaces (`commands/marketplaces.rs`)

| Command | Key Parameters | Returns |
|---|---|---|
| `add_marketplace` | `source: String; allow_insecure_http: bool` | `MarketplaceAddResult` |
| `remove_marketplace` | `name: String` | `()` |
| `update_marketplace` | `name: Option<String>` | `UpdateResult` |

### Settings (`commands/settings.rs`, `commands/kiro_settings.rs`)

| Command | Key Parameters | Returns |
|---|---|---|
| `get_settings` | — | `Settings` |
| `save_scan_roots` | `roots: Vec<String>` | `()` |
| `discover_projects` | `roots: Vec<String>` | `Vec<DiscoveredProject>` |
| `set_active_project` | `project_path: String` | `()` |
| `get_kiro_settings` | `project_path: String` | `Vec<SettingEntry>` |
| `set_kiro_setting` | `project_path, key: String; value: serde_json::Value` | `()` |
| `reset_kiro_setting` | `project_path, key: String` | `()` |

---

## CLI Commands (`kiro-market`)

Plugin reference format: `plugin@marketplace` (split on first `@`).

| Command | Notable Flags | Description |
|---|---|---|
| `marketplace add <source>` | `--protocol ssh\|https`, `--allow-insecure-http` | Add marketplace |
| `marketplace list` | — | List registered marketplaces |
| `marketplace update [name]` | — | Pull updates (all or named) |
| `marketplace remove <name>` | — | Remove marketplace |
| `search [query]` | — | Search skills by name/description |
| `install <plugin@marketplace>` | `--skill <name>`, `--force`, `--accept-mcp` | Install skills and agents |
| `info <plugin@marketplace>` | — | Show plugin details |
| `list` | — | List installed skills in current project |
| `remove <skill-name>` | — | Remove installed skill |
| `cache prune` | `--dry-run` | Remove orphaned clones and stale staging dirs |

---

## MarketplaceService Rust API

`crates/kiro-market-core/src/service/mod.rs`. Generic over `G: GitBackend`.

```rust
impl<G: GitBackend> MarketplaceService<G> {
    pub fn new(cache: CacheDir, git: G) -> Self
    pub fn add(&self, source: &str, opts: MarketplaceAddOptions) -> Result<MarketplaceAddResult>
    pub fn remove(&self, name: &MarketplaceName) -> Result<()>
    pub fn update(&self, name: Option<&MarketplaceName>) -> Result<UpdateResult>
    pub fn list(&self) -> Result<Vec<PluginBasicInfo>>
    pub fn list_plugin_entries(&self, marketplace: &MarketplaceName) -> Result<Vec<PluginEntry>>
    pub fn install_plugin(&self, ctx: AgentInstallContext) -> Result<InstallPluginResult>
    pub fn install_skills(&self, ...) -> Result<InstallSkillsResult>
    pub fn install_plugin_agents(&self, ...) -> Result<InstallAgentsResult>
    pub fn install_plugin_steering(&self, ...) -> Result<InstallSteeringResult>
    pub fn detect_plugin_updates(&self, project: &KiroProject) -> Result<DetectUpdatesResult>
    pub fn list_plugin_catalog(&self, marketplace: &MarketplaceName, project: &KiroProject) -> Result<PluginCatalogView>
}
```

## GitBackend Trait

```rust
pub trait GitBackend {
    fn clone_repo(&self, url: &str, dest: &Path, opts: CloneOptions) -> Result<(), GitError>;
    fn pull_repo(&self, path: &Path) -> Result<(), GitError>;
    fn verify_sha(&self, path: &Path, expected: &str) -> Result<(), GitError>;
}
```

Production implementation: `GixCliBackend` (gix primary, CLI fallback). Test implementations in `service::test_support` (feature `test-support`).

---

## Error Wire Format

`CommandError` serializes to JSON for the frontend:

```json
{
  "type": "validation" | "not_found" | "parse_error" | "internal" | "unknown",
  "message": "human-readable string"
}
```

`ErrorType` mapping: `ValidationError` → `validation`, `serde_json::Error` → `parse_error`, core `Error` variants → mapped by kind, everything else → `internal`.

---

## Shared Command Helpers (`commands/mod.rs`)

- `validate_kiro_project_path(path)` — canonicalizes, rejects non-existent, non-dir, symlinked, or missing `.kiro/` subdir paths
- `reject_empty_names(names, prefix)` — returns validation error if any name is empty/whitespace
- `make_service()` — constructs `MarketplaceService<GixCliBackend>` with default cache location

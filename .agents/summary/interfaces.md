# Interfaces

## Tauri IPC Commands

All commands are registered in `src-tauri/src/lib.rs` via `tauri-specta`. TypeScript bindings are auto-generated in `src/lib/bindings.ts`.

### Browse Commands

| Command | Parameters | Returns | Purpose |
|---------|-----------|---------|---------|
| `list_marketplaces` | — | `MarketplaceInfo[]` | List registered marketplaces with metadata |
| `list_plugins` | `marketplace: string` | `PluginInfo[]` | List plugins in a marketplace |
| `list_available_skills` | `marketplace: string, plugin: string` | `PluginSkillsResult` | List skills for a specific plugin |
| `list_all_skills_for_marketplace` | `marketplace: string` | `BulkSkillsResult` | List all skills across all plugins in a marketplace |
| `install_skills` | `marketplace, plugin, skills?, force?, accept_mcp?` | `InstallResult` | Install skills/agents from a plugin |
| `get_project_info` | — | `ProjectInfo` | Get active project path and status |

### Installed Commands

| Command | Parameters | Returns | Purpose |
|---------|-----------|---------|---------|
| `list_installed_skills` | — | `InstalledSkillInfo[]` | List skills installed in active project |
| `remove_skill` | `name: string` | — | Remove an installed skill |

### Marketplace Commands

| Command | Parameters | Returns | Purpose |
|---------|-----------|---------|---------|
| `add_marketplace` | `source: string` | `MarketplaceAddResult` | Register a new marketplace |
| `remove_marketplace` | `name: string` | — | Unregister a marketplace |
| `update_marketplace` | `name?: string` | `UpdateResult` | Pull latest from remote |

### Settings Commands

| Command | Parameters | Returns | Purpose |
|---------|-----------|---------|---------|
| `get_settings` | — | `Settings` | Load app settings |
| `save_scan_roots` | `roots: string[]` | — | Save project scan root paths |
| `discover_projects` | — | `DiscoveredProject[]` | Scan for Kiro projects |
| `set_active_project` | `path: string` | — | Set the active project |

### Kiro Settings Commands

| Command | Parameters | Returns | Purpose |
|---------|-----------|---------|---------|
| `get_kiro_settings` | — | `ResolvedSettings` | Load `.kiro/settings.json` with defaults |
| `set_kiro_setting` | `key: string, value: JsonValue` | — | Set a setting value |
| `reset_kiro_setting` | `key: string` | — | Remove a setting (revert to default) |

---

## CLI Interface (kiro-market)

```
kiro-market [OPTIONS] <COMMAND>

Commands:
  marketplace   Manage marketplace sources
    add <source> [--protocol https|ssh] [--allow-insecure-http]
    list
    update [name]
    remove <name>
  search [query]
  install <plugin@marketplace> [--skill <name>] [--force] [--accept-mcp]
  list
  update [plugin_ref]
  remove <skill-name>
  info <plugin@marketplace>
  cache
    prune [--dry-run]

Options:
  -v, -vv, -vvv    Increase verbosity
  --version        Show version
  --help           Show help
```

**Plugin reference format:** `plugin@marketplace` (split on first `@`)

---

## Core Library Public API

### MarketplaceService<G: GitBackend>

```rust
// Marketplace lifecycle
fn add(opts: MarketplaceAddOptions) -> Result<MarketplaceAddResult>
fn remove(name: &str) -> Result<()>
fn update(name: Option<&str>) -> Result<UpdateResult>
fn list() -> Result<Vec<KnownMarketplace>>

// Plugin operations
fn list_plugin_entries(marketplace: &str) -> Result<Vec<PluginEntry>>
fn marketplace_path(name: &str) -> PathBuf

// Skill browsing
fn list_skills_for_plugin(marketplace, plugin, installed) -> Result<PluginSkillsResult>
fn list_all_skills(marketplace, installed) -> BulkSkillsResult
fn count_skills_for_plugin(marketplace, plugin) -> SkillCount

// Installation
fn install_skills(context, project, filter, mode) -> InstallSkillsResult
fn install_plugin_agents(context, project, accept_mcp, mode) -> InstallAgentsResult
fn install_plugin_steering(context, project, mode) -> InstallSteeringResult
```

### KiroProject

```rust
fn new(kiro_dir: PathBuf) -> Self
fn install_skill_from_dir(name, source_dir, version?, marketplace?, plugin?) -> Result<()>
fn install_skill_from_dir_force(name, source_dir, ...) -> Result<()>
fn remove_skill(name: &str) -> Result<()>
fn load_installed() -> Result<InstalledSkills>
fn install_agent(definition, marketplace, plugin, mode) -> Result<()>
fn install_native_agent(source, name, marketplace, plugin, mode) -> Result<InstalledNativeAgentOutcome>
fn install_steering_file(source, name, marketplace, plugin, mode) -> Result<()>
```

### CacheDir

```rust
fn default_location() -> PathBuf  // ~/.cache/kiro-market/
fn detect(source: &str) -> MarketplaceSource
fn add_known_marketplace(name, source, protocol?) -> Result<()>
fn remove_known_marketplace(name: &str) -> Result<()>
fn load_known_marketplaces() -> Result<Vec<KnownMarketplace>>
fn prune_orphans(mode: PruneMode) -> Result<PruneReport>
```

### GitBackend Trait

```rust
trait GitBackend {
    fn clone_repo(url: &str, dest: &Path, options: &CloneOptions) -> Result<String>;
    fn pull_repo(path: &Path) -> Result<String>;
    fn verify_sha(path: &Path, expected: &str) -> Result<()>;
}
```

---

## File Format Interfaces

### marketplace.json (in marketplace repos)

```json
{
  "plugins": [
    {
      "name": "plugin-name",
      "description": "Optional description",
      "path": "./relative/path",
      "source": { "github": "owner/repo" }  // or git_url, or relative path
    }
  ]
}
```

### plugin.json (in plugin directories)

```json
{
  "name": "plugin-name",
  "description": "Optional",
  "skills": ["./skills/"],
  "agents": ["./agents/"],
  "steering": ["./steering/"],
  "format": "kiro-cli"  // optional, triggers native agent handling
}
```

### .kiro/settings.json

Typed key-value store with dotted paths (e.g., `"editor.tabSize"`). Schema defined in `kiro_settings.rs` registry.

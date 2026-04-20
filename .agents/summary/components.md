# Components

## Core Library (`kiro-market-core`)

The shared library containing all business logic. Both CLI and desktop app depend on this.

### MarketplaceService (`service.rs`)

The primary orchestrator for marketplace lifecycle operations.

**Responsibilities:**
- Add/remove/update/list marketplaces
- Clone remote repos or link/copy local paths
- Discover plugins within marketplaces
- Install skills and agents into projects
- Manage plugin registries

**Key types:**
- `MarketplaceService` — stateful service holding `CacheDir` and `GitBackend`
- `MarketplaceAddResult` — result of adding a marketplace (name, plugins, storage type)
- `InstallSkillsResult` / `InstallAgentsResult` — outcome of install operations
- `InstallFilter` — `All` or `Names(&[String])` for selective install
- `InstallMode` — `Normal` or `Force` (overwrite existing)
- `InsecureHttpPolicy` — controls whether `http://` sources are allowed

### CacheDir (`cache.rs`)

Manages the on-disk cache at `~/.cache/kiro-market/` (or platform equivalent).

**Responsibilities:**
- Directory structure: `marketplaces/`, `plugins/`, `registries/`
- Known marketplace registry (JSON file tracking registered sources)
- Plugin registries (per-marketplace JSON listing discovered plugins)
- Source detection (GitHub shorthand, git URLs, local paths, file URLs)
- Orphan pruning (remove unregistered marketplace dirs, stale plugin locks, pending staging)
- Local path resolution with security restrictions

**Key types:**
- `CacheDir` — root handle with path accessors
- `KnownMarketplace` — registered marketplace entry (name, source, protocol)
- `MarketplaceSource` — enum: `GitUrl(String)` | `LocalPath(PathBuf)`
- `PruneReport` / `PruneMode` — orphan cleanup results and dry-run support

### KiroProject (`project.rs`)

Manages the `.kiro/` directory within a user's project.

**Responsibilities:**
- Skill installation (copy skill dir → `.kiro/skills/<name>/`)
- Agent installation (write JSON config + prompt file → `.kiro/agents/`)
- Tracking installed skills/agents in JSON manifests
- Removal of installed skills
- Staging directory management with crash recovery
- Concurrent access serialization via file locks

**Key types:**
- `KiroProject` — project root handle
- `InstalledSkills` / `InstalledSkillMeta` — skill tracking
- `InstalledAgents` / `InstalledAgentMeta` — agent tracking

### Git Operations (`git.rs`)

Dual-backend git clone/pull with error translation.

**Responsibilities:**
- Clone repos (gix primary, CLI fallback)
- Pull updates on existing clones
- SHA verification for pinned refs
- GitHub shorthand → URL conversion (HTTPS/SSH)
- Auth error detection and user-friendly messages

**Key types:**
- `GitProtocol` — `Https` | `Ssh`
- `CloneOptions` — ref, protocol, insecure HTTP policy
- `GitRef` — validated git reference (rejects empty, dash-prefixed)
- `GixCliBackend` — production implementation of `GitBackend` trait

### Plugin Discovery (`plugin.rs`)

Discovers plugins and skills within marketplace repositories.

**Responsibilities:**
- Scan directories for `plugin.json` manifests
- Discover skills within plugin directories
- Parse plugin manifests (name, description, skill paths, agent paths)
- Respect depth limits during scanning

**Key types:**
- `PluginManifest` — parsed `plugin.json`
- `DiscoveredPlugin` — plugin found during scan (name, path, manifest)

### Skill Parsing (`skill.rs`)

Parses `SKILL.md` files with YAML frontmatter.

**Key types:**
- `SkillFrontmatter` — name, description, invocable flag
- `ParseError` — frontmatter parsing failures

### Agent System (`agent/`)

Multi-dialect agent parsing and emission.

**Submodules:**
- `parse.rs` — dialect detection and dispatch
- `parse_claude.rs` — Claude agent format (YAML frontmatter in `.md`)
- `parse_copilot.rs` — Copilot agent format (`.agent.md`)
- `discover.rs` — scan directories for agent files
- `tools.rs` — tool mapping between Claude/Copilot and Kiro formats
- `emit.rs` — generate Kiro agent JSON + prompt files
- `types.rs` — `AgentDefinition`, `AgentDialect`, `McpServerConfig`

### Validation (`validation.rs`)

Security-focused input validation.

**Key functions:**
- `validate_name()` — safe filesystem names
- `validate_relative_path()` — safe relative paths
- `RelativePath` newtype — validated at construction

### Kiro Settings (`kiro_settings.rs`)

Manages `.kiro/settings.json` with a typed registry of known settings.

**Responsibilities:**
- Load/save settings with unknown-key preservation
- Typed setting definitions with categories, defaults, and value types
- Nested JSON path get/set/remove operations

### Platform Abstraction (`platform.rs`)

OS-specific filesystem operations.

- Unix: symlinks
- Windows: NTFS junctions with copy fallback
- `LinkResult` — `Linked` | `Copied`

### File Locking (`file_lock.rs`)

Cross-process file locking using `fs4`.

- `with_file_lock()` — acquire lock, run closure, release
- Creates parent directories automatically
- Propagates closure errors, releases lock on panic

### RAII Guard (`raii.rs`)

- `DirCleanupGuard` — removes directory on drop unless `defuse()`d

---

## CLI (`kiro-market`)

Thin command-line interface using clap derive.

### Command Modules

| Module | Commands |
|--------|----------|
| `marketplace.rs` | add, list, update, remove |
| `search.rs` | search skills across marketplaces |
| `install.rs` | install plugin skills/agents |
| `list.rs` | list installed skills |
| `remove.rs` | remove installed skill |
| `info.rs` | show plugin details |
| `update.rs` | update installed plugins |
| `cache.rs` | cache prune |
| `common.rs` | shared helpers (find plugin entry, load skill paths) |

---

## Desktop App (`kiro-control-center`)

### Rust Backend (Tauri Commands)

| Module | Tauri Commands |
|--------|---------------|
| `browse.rs` | `list_marketplaces`, `list_plugins`, `list_available_skills`, `list_all_skills_for_marketplace`, `install_skills`, `get_project_info` |
| `installed.rs` | `list_installed_skills`, `remove_skill` |
| `marketplaces.rs` | `add_marketplace`, `remove_marketplace`, `update_marketplace` |
| `settings.rs` | `get_settings`, `save_scan_roots`, `discover_projects`, `set_active_project` |
| `kiro_settings.rs` | `get_kiro_settings`, `set_kiro_setting`, `reset_kiro_setting` |

### Svelte Frontend

| Component | Purpose |
|-----------|---------|
| `+page.svelte` | Root page with tab navigation and settings |
| `NavRail.svelte` | Left navigation rail (Browse, Installed, Marketplaces) |
| `BrowseTab.svelte` | Browse marketplaces, view skills, bulk install |
| `InstalledTab.svelte` | View/remove installed skills |
| `MarketplacesTab.svelte` | Add/update/remove marketplace sources |
| `ProjectPicker.svelte` | Initial project selection |
| `ProjectDropdown.svelte` | Switch between discovered projects |
| `ScanRootsPanel.svelte` | Manage scan root directories |
| `SettingsView.svelte` | Kiro settings editor |
| `SettingControl.svelte` | Individual setting input control |
| `SkillCard.svelte` | Skill display card with install checkbox |
| `SettingsPanel.svelte` | Settings panel wrapper |

### State Management

`project.svelte.ts` — Svelte 5 `$state` module pattern:
- Exports a const `store` object with reactive properties
- Actions mutate store properties through the deep state proxy
- Handles initialization, project selection, scan root management

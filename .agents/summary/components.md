# Components

## Core Library (kiro-market-core)

### service.rs — MarketplaceService

The primary orchestrator. Manages the full lifecycle of marketplaces and plugin installation.

**Responsibilities:**
- Add/remove/update/list marketplaces
- Clone or link marketplace repositories
- Install/remove skills and agents into projects
- Resolve plugin directories (local vs remote)
- Build and persist plugin registries

**Key types:** `MarketplaceService<G: GitBackend>`, `MarketplaceAddResult`, `UpdateResult`, `InstallFilter`, `InsecureHttpPolicy`

### service/browse.rs — Skill Browsing

Enumerates skills across marketplaces for display in UI/CLI.

**Responsibilities:**
- List all skills across all marketplaces
- List skills for a specific plugin
- Count skills per plugin (with error handling for broken manifests)
- Resolve plugin install context (skill paths, agent paths, steering paths)
- Track which skills are already installed

**Key types:** `BulkSkillsResult`, `SkillInfo`, `SkippedPlugin`, `SkippedSkill`, `PluginInstallContext`, `SkillCount`

### cache.rs — CacheDir

Manages the `~/.cache/kiro-market/` directory structure.

**Responsibilities:**
- Marketplace source detection (GitHub shorthand, git URL, local path, file URL)
- Known marketplace registry (add/remove/list)
- Plugin registry persistence per marketplace
- Cache pruning (orphaned dirs, stale staging, stale lock files)
- Local path resolution with security restrictions

**Key types:** `CacheDir`, `MarketplaceSource`, `KnownMarketplace`, `PruneReport`, `PruneMode`

### project.rs — KiroProject

Manages the `.kiro/` directory within a user's project.

**Responsibilities:**
- Install/remove skills (with file locking for concurrency)
- Install translated agents (JSON config + prompt file)
- Install native Kiro agents (verbatim copy with hash tracking)
- Install native companion files
- Install steering files
- Track installed state in JSON manifests
- RAII rollback on partial failures

**Key types:** `KiroProject`, `InstalledSkills`, `InstalledSkillMeta`, `InstalledAgents`, `InstalledAgentMeta`, `InstalledSteering`

### git.rs — Git Operations

Dual-backend git with gix (primary) and CLI (fallback).

**Responsibilities:**
- Clone repositories (with optional ref checkout)
- Pull updates on existing clones
- SHA verification (prefix matching)
- Auth error translation to user-friendly messages
- GitHub shorthand → URL conversion

**Key types:** `GixCliBackend`, `GitBackend` trait, `CloneOptions`, `GitRef`, `GitProtocol`

### agent/ — Agent Module

Parses, transforms, and emits agent definitions across dialects.

| Submodule | Purpose |
|-----------|---------|
| `discover.rs` | Find agent files in directories (`.md`, `.agent.md`, native `.json`) |
| `parse.rs` | Dialect detection and dispatch |
| `parse_claude.rs` | Parse Claude agent format (YAML frontmatter + markdown body) |
| `parse_copilot.rs` | Parse Copilot agent format (`.agent.md`) |
| `parse_native.rs` | Parse native Kiro agent JSON |
| `emit.rs` | Generate Kiro agent JSON + prompt file from any dialect |
| `tools.rs` | Map Claude/Copilot tool names to Kiro equivalents |
| `frontmatter.rs` | YAML frontmatter extraction |
| `types.rs` | Shared types (`AgentDefinition`, `AgentDialect`, `McpServerConfig`) |

### plugin.rs — Plugin Discovery

Scans marketplace directories for plugins and their skills.

**Responsibilities:**
- Discover `plugin.json` manifests at configurable depth
- Parse plugin manifests (name, description, skill paths, agent paths, steering paths, format)
- Discover skill directories within a plugin
- Handle missing manifests gracefully (fall back to directory scanning)

### skill.rs — Skill Parsing

Parses `SKILL.md` files with YAML frontmatter.

**Responsibilities:**
- Extract frontmatter (name, description, invocable flag)
- Validate skill names against security rules
- Handle CRLF line endings

### validation.rs — Input Validation

Security-critical validation for all user-supplied inputs.

**Responsibilities:**
- Name validation (rejects traversal, reserved names, control chars, NUL bytes)
- Relative path validation (rejects absolute, `..`, backslash, NUL)
- `RelativePath` newtype with construction-time enforcement
- Custom serde deserializer for `RelativePath`

### Other Core Modules

| Module | Purpose |
|--------|---------|
| `error.rs` | Structured error hierarchy with `thiserror` |
| `file_lock.rs` | Cross-process file locking via `fs4` |
| `hash.rs` | BLAKE3 content hashing for change detection |
| `kiro_settings.rs` | `.kiro/settings.json` typed registry with categories |
| `marketplace.rs` | Marketplace manifest parsing (`marketplace.json`) |
| `platform.rs` | OS abstraction (symlinks, junctions, copy fallback) |
| `raii.rs` | `DirCleanupGuard` (RAII temp dir removal) |
| `steering/` | Steering file discovery and installation |

---

## CLI (kiro-market)

Thin `clap` wrapper. Each subcommand is a module in `commands/`:

| Module | Command | Delegates To |
|--------|---------|-------------|
| `marketplace.rs` | `marketplace add/list/update/remove` | `MarketplaceService` |
| `install.rs` | `install <plugin@marketplace>` | `MarketplaceService` + `KiroProject` |
| `search.rs` | `search [query]` | Skill frontmatter scanning |
| `list.rs` | `list` | `KiroProject::load_installed()` |
| `remove.rs` | `remove <skill>` | `KiroProject::remove_skill()` |
| `info.rs` | `info <plugin@marketplace>` | Plugin manifest + skill enumeration |
| `cache.rs` | `cache prune` | `CacheDir::prune_orphans()` |
| `update.rs` | `update [plugin]` | `MarketplaceService::update()` |
| `common.rs` | Shared helpers | Plugin entry resolution |

---

## Desktop App (kiro-control-center)

### Rust Backend (src-tauri/)

| Module | Purpose |
|--------|---------|
| `lib.rs` | Tauri command registration via `tauri-specta` |
| `commands/browse.rs` | Marketplace/plugin/skill listing + install |
| `commands/installed.rs` | Installed skill listing + removal |
| `commands/marketplaces.rs` | Marketplace add/remove/update |
| `commands/settings.rs` | App settings, project discovery, scan roots |
| `commands/kiro_settings.rs` | `.kiro/settings.json` read/write with mutex |
| `error.rs` | `CommandError` wrapper for frontend-friendly errors |

### Svelte Frontend (src/)

| Component | Purpose |
|-----------|---------|
| `BrowseTab.svelte` | Marketplace/plugin browser with skill cards and bulk install |
| `InstalledTab.svelte` | Installed skills list with bulk remove |
| `MarketplacesTab.svelte` | Marketplace source management |
| `SettingsView.svelte` | Kiro settings editor (categorized) |
| `SettingsPanel.svelte` | App settings panel |
| `SettingControl.svelte` | Individual setting input control |
| `ScanRootsPanel.svelte` | Project scan root configuration |
| `ProjectPicker.svelte` | Project selection UI |
| `ProjectDropdown.svelte` | Project dropdown selector |
| `NavRail.svelte` | Tab navigation rail |
| `SkillCard.svelte` | Individual skill display card |

**State management:** Svelte 5 `$state` module pattern in `lib/stores/project.svelte.ts`

---

## Dev Tooling (xtask)

| Function | Purpose |
|----------|---------|
| `hook_block_cargo_lock` | Pre-tool-use hook: blocks direct Cargo.lock edits |
| `hook_post_edit` | Post-tool-use hook: runs rustfmt + clippy on edited files |
| `derive_package` | Maps file paths to their owning crate for targeted linting |

# Components

<!-- tags: components, modules, responsibilities -->

## kiro-market-core Modules

| Module | File | Responsibility |
|---|---|---|
| `service` | `service/mod.rs` + `service/browse.rs` | Primary orchestrator. `MarketplaceService` owns add/remove/update/install/detect-updates. `browse.rs` handles skill/agent/steering enumeration and plugin catalog assembly. |
| `cache` | `cache.rs` | `CacheDir`: manages `~/.cache/kiro-market/`. Marketplace registration, plugin registry read/write, orphan pruning, source detection (`MarketplaceSource`). |
| `project` | `project.rs` | `KiroProject`: all `.kiro/` directory operations. Install/remove skills, agents, steering, native companions. Tracking file read/write with `fs4` locking. |
| `git` | `git.rs` | Dual-backend git: `gix` primary, `git` CLI fallback. Clone, pull, SHA verification. Auth error translation to user-friendly messages. `GixCliBackend` is the production implementation. |
| `agent` | `agent/` | Agent parsing (Claude `.md`, Copilot `.agent.md`, native Kiro JSON), tool mapping, Kiro JSON+prompt emission. Sub-modules: `parse`, `parse_claude`, `parse_copilot`, `parse_native`, `discover`, `emit`, `tools`, `types`, `frontmatter`. |
| `steering` | `steering/` | Steering file discovery (`discover.rs`) and install/remove types (`types.rs`). |
| `plugin` | `plugin.rs` | Plugin discovery (scan dirs for `plugin.json`). `PluginManifest` parsing. `DiscoveredPlugin` and `DiscoveredSkill` types. |
| `skill` | `skill.rs` | `SKILL.md` YAML frontmatter parsing. `SkillFrontmatter` type. |
| `validation` | `validation.rs` | `validate_name()`, `validate_relative_path()`. Newtypes: `PluginName`, `MarketplaceName`, `AgentName`, `RelativePath`. |
| `kiro_settings` | `kiro_settings.rs` | `.kiro/settings.json` typed registry. Setting definitions, type validation, nested JSON get/set/remove. |
| `hash` | `hash.rs` | BLAKE3 content hashing. `BlakeHash` newtype (validated hex string with `blake3:` prefix). `hash_artifact()` for files/dirs. |
| `platform` | `platform.rs` | OS abstraction for local marketplace linking: symlinks (Unix), NTFS junctions (Windows), recursive copy fallback. |
| `file_lock` | `file_lock.rs` | `with_file_lock()`: cross-process file locking via `fs4`. Creates parent dirs, runs closure under lock. |
| `error` | `error.rs` | Structured error hierarchy: `Error`, `ValidationError`, `GitError`, `PluginError`, `MarketplaceError`, `AgentError`, `SkillError`. `remediation_hint()` for actionable CLI messages. |
| `raii` | `raii.rs` | `DirCleanupGuard`: RAII temp dir removal. `.defuse()` on success, auto-remove on drop/failure. `.retarget()` to move cleanup focus. |
| `marketplace` | `marketplace.rs` | `Marketplace` manifest type. `PluginSource` (GitHub shorthand, git URL, git subdir, relative path). |

## kiro-market CLI Modules

| Module | File | Responsibility |
|---|---|---|
| `cli` | `cli.rs` | Clap derive definitions: `Cli`, `Command`, `MarketplaceAction`, `CacheAction`. `parse_plugin_ref()` splits `plugin@marketplace` on first `@`. |
| `main` | `main.rs` | Entry point. Initializes tracing, dispatches to command modules. |
| `commands/marketplace` | `commands/marketplace.rs` | `add`, `list`, `update`, `remove` subcommands. |
| `commands/install` | `commands/install.rs` | `install` subcommand. Fetches plugin dir, runs skill/agent/steering installs, prints outcomes. |
| `commands/search` | `commands/search.rs` | `search` subcommand. Reads skill frontmatter, filters by query. |
| `commands/list` | `commands/list.rs` | `list` subcommand. Lists installed skills from project tracking. |
| `commands/remove` | `commands/remove.rs` | `remove` subcommand. |
| `commands/info` | `commands/info.rs` | `info` subcommand. Prints plugin details and skill list. |
| `commands/cache` | `commands/cache.rs` | `cache prune` subcommand. |
| `commands/common` | `commands/common.rs` | `find_plugin_entry()`: resolves plugin from manifest or scan fallback. |
| `commands/update` | `commands/update.rs` | `update` subcommand (marketplace update). |

## kiro-control-center Tauri Backend

| Module | File | Responsibility |
|---|---|---|
| `lib` | `src-tauri/src/lib.rs` | Registers all Tauri commands via `tauri-specta`. Exports `bindings.ts` in debug builds. |
| `commands/browse` | `commands/browse.rs` | Browse commands + view types: `PluginCatalogResponseView`, `PluginCatalogEntryView`, `SourceType`, `ProjectInfo`, `PluginInfo`, `MarketplaceInfo`. |
| `commands/installed` | `commands/installed.rs` | `list_installed_skills`, `remove_skill`. `InstalledSkillInfo` view type. |
| `commands/plugins` | `commands/plugins.rs` | `install_plugin`, `list_installed_plugins`, `remove_plugin`, `detect_plugin_updates`. |
| `commands/agents` | `commands/agents.rs` | `install_plugin_agents`, `install_agents`, `remove_agent`. |
| `commands/steering` | `commands/steering.rs` | `install_plugin_steering`, `install_steering_files`, `remove_steering_file`. |
| `commands/marketplaces` | `commands/marketplaces.rs` | `add_marketplace`, `remove_marketplace`, `update_marketplace`. |
| `commands/settings` | `commands/settings.rs` | `get_settings`, `save_scan_roots`, `discover_projects`, `set_active_project`. `Settings`, `DiscoveredProject` types. |
| `commands/kiro_settings` | `commands/kiro_settings.rs` | `get_kiro_settings`, `set_kiro_setting`, `reset_kiro_setting`. |
| `commands/mod` | `commands/mod.rs` | `validate_kiro_project_path()`, `reject_empty_names()`, `make_service()` shared helpers. |
| `error` | `src-tauri/src/error.rs` | `CommandError` / `ErrorType` — maps Rust errors to typed JSON for the frontend. |

## Svelte Frontend Components

| Component | File | Responsibility |
|---|---|---|
| `BrowseTab` | `components/BrowseTab.svelte` | Main browse UI. Marketplace/plugin sidebar, skill cards with checkboxes, bulk install, `CustomizeDrawer` integration. |
| `InstalledTab` | `components/InstalledTab.svelte` | Lists installed skills/agents/steering. Bulk remove. |
| `MarketplacesTab` | `components/MarketplacesTab.svelte` | Add/update/remove marketplaces. Shows plugin count and source type. |
| `CustomizeDrawer` | `components/CustomizeDrawer.svelte` | Per-plugin drawer for granular skill/agent/steering selection and apply-diff. |
| `PluginCard` | `components/PluginCard.svelte` | Plugin summary card with skill/agent/steering counts and install state. |
| `SkillCard` | `components/SkillCard.svelte` | Individual skill card with description and install badge. |
| `NavRail` | `components/NavRail.svelte` | Tab navigation rail. |
| `BannerStack` | `components/BannerStack.svelte` | Stacked update/warning banners. |
| `ProjectDropdown` | `components/ProjectDropdown.svelte` | Active project selector. |
| `ProjectPicker` | `components/ProjectPicker.svelte` | Project picker dialog. |
| `ScanRootsPanel` | `components/ScanRootsPanel.svelte` | Manage project scan roots. |
| `SettingsView` | `components/SettingsView.svelte` | Kiro settings viewer/editor. |
| `SettingsPanel` | `components/SettingsPanel.svelte` | Settings category panel. |
| `SettingControl` | `components/SettingControl.svelte` | Individual setting control (bool/string/etc). |

## Frontend Stores and Utilities

| Module | File | Responsibility |
|---|---|---|
| `project.svelte.ts` | `stores/project.svelte.ts` | Active project state, scan roots, project discovery. Svelte 5 `$state` module pattern. |
| `plugin-updates.svelte.ts` | `stores/plugin-updates.svelte.ts` | Plugin update check state. `PluginUpdatesStore`. |
| `plugin-update-banners.svelte.ts` | `stores/plugin-update-banners.svelte.ts` | Banner display logic for update notifications. |
| `plugin-updates.ts` | `stores/plugin-updates.ts` | Pure functions: `groupFailures`, `kindLabel`, `remediationClass`, `hintFor`. |
| `plugin-actions.ts` | `lib/plugin-actions.ts` | `runPluginInstall`, `runPluginRemove` — orchestrate multi-step IPC calls. |
| `format.ts` | `lib/format.ts` | Result formatting for install/remove outcomes, warnings, failures. |
| `drawer-diff.ts` | `lib/drawer-diff.ts` | `deriveDiff`, `deriveSectionState` — compute what changed in the customize drawer. |
| `error-source.ts` | `lib/error-source.ts` | Error source chain extraction from `CommandError`. |
| `keys.ts` | `lib/keys.ts` | Keyboard shortcut constants. |
| `bindings.ts` | `lib/bindings.ts` | **Auto-generated.** TypeScript types and `invoke()` wrappers for all Tauri commands. Do not edit. |

## xtask

| Module | File | Responsibility |
|---|---|---|
| `main.rs` | `xtask/src/main.rs` | Dispatch: `hook-block-cargo-lock`, `hook-post-edit`, `plan-lint`. Git status parsing, frontend path detection, rustfmt/clippy runners. |
| `plan_lint.rs` | `xtask/src/plan_lint.rs` | Static analysis gates: `no_panic`, `no_unwrap`, `non_exhaustive`, `no_frontend_deps`, `ffi_enum_tag`. Allowlist support per gate. |

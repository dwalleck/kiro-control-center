# Project Discovery & Selection — Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Let users discover, select, and switch between Kiro projects in the control center via configured root directories and an OS folder picker.

**Architecture:** A `Settings` config file persists scan root directories and the last-active project. On launch, the app scans roots 1-2 levels deep for `.kiro` directories. A landing screen appears on first launch; a header dropdown allows switching once a project is active. Four new Tauri commands handle settings, discovery, and project activation. The frontend uses a shared Svelte store for `projectPath` so both the page and layout can access it.

**Tech Stack:** Rust (Tauri 2, serde_json, dirs), Svelte 5 ($state/$props/$effect), Tailwind CSS, tauri-plugin-dialog (OS folder picker)

---

### Task 1: Add tauri-plugin-dialog dependency

**Files:**
- Modify: `crates/kiro-control-center/src-tauri/Cargo.toml`
- Modify: `crates/kiro-control-center/src-tauri/tauri.conf.json`
- Modify: `crates/kiro-control-center/src-tauri/src/lib.rs`
- Modify: `crates/kiro-control-center/package.json`

**Step 1: Add Rust dependency**

In `crates/kiro-control-center/src-tauri/Cargo.toml`, add to `[dependencies]`:

```toml
tauri-plugin-dialog = "2"
```

**Step 2: Add JS dependency**

```bash
cd crates/kiro-control-center
npm install @tauri-apps/plugin-dialog
```

**Step 3: Register the plugin in `lib.rs`**

In the `tauri::Builder::default()` chain, add the dialog plugin:

```rust
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(builder.invoke_handler())
```

**Step 4: Add dialog permission in capabilities file**

The file `src-tauri/capabilities/default.json` already exists with `core:default` and `opener:default`. Add `"dialog:default"` to the permissions array:

```json
{
  "$schema": "../gen/schemas/desktop-schema.json",
  "identifier": "default",
  "description": "Capability for the main window",
  "windows": ["main"],
  "permissions": [
    "core:default",
    "opener:default",
    "dialog:default"
  ]
}
```

**Note:** This is a Tauri 2 capabilities file, NOT `tauri.conf.json`. Tauri 2 uses `src-tauri/capabilities/*.json` for permission management. Without this, the dialog plugin will fail at runtime with a permission denied error.

**Step 5: Verify it compiles**

Run: `cargo check -p kiro-control-center`

**Step 6: Commit**

```
feat: add tauri-plugin-dialog for OS folder picker
```

---

### Task 2: Create Settings type and persistence

**Files:**
- Create: `crates/kiro-control-center/src-tauri/src/commands/settings.rs`
- Modify: `crates/kiro-control-center/src-tauri/src/commands/mod.rs`

**Step 1: Create the settings module**

Create `crates/kiro-control-center/src-tauri/src/commands/settings.rs`:

```rust
//! Settings management: scan roots, project discovery, and active project.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

use kiro_market_core::project::KiroProject;

use crate::error::CommandError;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Persisted application settings.
#[derive(Clone, Debug, Default, Serialize, Deserialize, specta::Type)]
pub struct Settings {
    /// Directories to scan for Kiro projects.
    #[serde(default)]
    pub scan_roots: Vec<String>,
    /// Last active project path (restored on launch).
    #[serde(default)]
    pub last_project: Option<String>,
}

/// A discovered Kiro project.
#[derive(Clone, Debug, Serialize, specta::Type)]
pub struct DiscoveredProject {
    /// Absolute path to the project root.
    pub path: String,
    /// Directory name (for display).
    pub name: String,
    /// Whether `.kiro/` exists.
    pub kiro_initialized: bool,
    /// Number of installed skills.
    pub skill_count: u32,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Path to the settings file.
///
/// Respects `KIRO_MARKET_CONFIG_DIR` env var for test isolation (the `dirs`
/// crate ignores `XDG_CONFIG_HOME` on macOS/Windows).
fn settings_path() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("KIRO_MARKET_CONFIG_DIR") {
        return Some(PathBuf::from(dir).join("settings.json"));
    }
    dirs::config_dir().map(|d| d.join("kiro-market").join("settings.json"))
}

/// Load settings from disk, returning defaults if the file doesn't exist.
fn load_settings() -> Settings {
    let Some(path) = settings_path() else {
        return Settings::default();
    };
    match fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Settings::default(),
        Err(e) => {
            warn!(path = %path.display(), error = %e, "failed to read settings");
            Settings::default()
        }
    }
}

/// Save settings to disk.
fn save_settings(settings: &Settings) -> Result<(), CommandError> {
    let path = settings_path().ok_or_else(|| {
        CommandError::new(
            "could not determine config directory",
            crate::error::ErrorType::IoError,
        )
    })?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| {
            CommandError::new(
                format!("failed to create config directory: {e}"),
                crate::error::ErrorType::IoError,
            )
        })?;
    }
    let json = serde_json::to_string_pretty(settings).map_err(|e| {
        CommandError::new(
            format!("failed to serialize settings: {e}"),
            crate::error::ErrorType::IoError,
        )
    })?;
    fs::write(&path, json).map_err(|e| {
        CommandError::new(
            format!("failed to write settings: {e}"),
            crate::error::ErrorType::IoError,
        )
    })?;
    debug!(path = %path.display(), "settings saved");
    Ok(())
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// Load application settings.
#[tauri::command]
#[specta::specta]
pub async fn get_settings() -> Result<Settings, CommandError> {
    Ok(load_settings())
}

/// Save the list of scan root directories.
#[tauri::command]
#[specta::specta]
pub async fn save_scan_roots(roots: Vec<String>) -> Result<(), CommandError> {
    let mut settings = load_settings();
    settings.scan_roots = roots;
    save_settings(&settings)
}

/// Discover Kiro projects by scanning configured root directories.
///
/// Scans each root 1-2 levels deep for directories containing `.kiro/`.
#[tauri::command]
#[specta::specta]
pub async fn discover_projects() -> Result<Vec<DiscoveredProject>, CommandError> {
    let settings = load_settings();
    let mut projects = Vec::new();

    for root in &settings.scan_roots {
        let root_path = PathBuf::from(shellexpand_tilde(root));
        if !root_path.is_dir() {
            warn!(root = %root, "scan root is not a directory, skipping");
            continue;
        }
        scan_for_projects(&root_path, 0, 2, &mut projects);
    }

    projects.sort_by(|a, b| a.name.cmp(&b.name));
    projects.dedup_by(|a, b| a.path == b.path);
    Ok(projects)
}

/// Set the active project and persist it.
#[tauri::command]
#[specta::specta]
pub async fn set_active_project(
    path: String,
) -> Result<crate::commands::browse::ProjectInfo, CommandError> {
    let project_path = PathBuf::from(&path);
    if !project_path.is_dir() {
        return Err(CommandError::new(
            format!("directory does not exist: {path}"),
            crate::error::ErrorType::NotFound,
        ));
    }

    // Persist as last_project.
    let mut settings = load_settings();
    settings.last_project = Some(path.clone());
    save_settings(&settings)?;

    // Return ProjectInfo (reuse existing command logic).
    crate::commands::browse::get_project_info(path).await
}

// ---------------------------------------------------------------------------
// Scanning helpers
// ---------------------------------------------------------------------------

/// Expand `~` to the home directory.
fn shellexpand_tilde(path: &str) -> String {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest).to_string_lossy().into_owned();
        }
    }
    path.to_owned()
}

/// Recursively scan for `.kiro` directories up to `max_depth` levels.
fn scan_for_projects(
    dir: &Path,
    current_depth: u32,
    max_depth: u32,
    results: &mut Vec<DiscoveredProject>,
) {
    if current_depth > max_depth {
        return;
    }

    // Check if this directory itself is a Kiro project.
    let kiro_dir = dir.join(".kiro");
    if kiro_dir.is_dir() {
        let name = dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| dir.to_string_lossy().into_owned());

        // Skip reading installed-skills.json during scan for performance.
        // With 50+ projects, reading a JSON file per project would be slow.
        // skill_count is loaded on demand when the user selects a project.
        results.push(DiscoveredProject {
            path: dir.to_string_lossy().into_owned(),
            name,
            kiro_initialized: true,
            skill_count: 0,
        });
        // Don't recurse into .kiro projects (they won't contain sub-projects).
        return;
    }

    // Recurse into subdirectories.
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries {
        let Ok(entry) = entry else { continue };
        let path = entry.path();
        if path.is_dir() {
            // Skip hidden directories and common non-project dirs.
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str.starts_with('.')
                || name_str == "node_modules"
                || name_str == "target"
                || name_str == ".git"
            {
                continue;
            }
            scan_for_projects(&path, current_depth + 1, max_depth, results);
        }
    }
}
```

**Step 2: Register the module**

In `commands/mod.rs`, add:

```rust
pub mod settings;
```

**Step 3: Run tests and clippy**

Run: `cargo check -p kiro-control-center`
Run: `cargo clippy --workspace -- -D warnings`

**Step 4: Commit**

```
feat: add settings persistence and project discovery commands

Four new Tauri commands: get_settings, save_scan_roots,
discover_projects, set_active_project. Settings persisted to
~/.config/kiro-market/settings.json. Project scanning searches
1-2 levels deep in configured root directories.
```

---

### Task 3: Register new commands and regenerate bindings

**Files:**
- Modify: `crates/kiro-control-center/src-tauri/src/lib.rs`

**Step 1: Add commands to the builder**

In `lib.rs`, add the four new commands to `collect_commands!`:

```rust
fn create_builder() -> Builder<tauri::Wry> {
    Builder::<tauri::Wry>::new().commands(collect_commands![
        // existing commands...
        commands::browse::list_marketplaces,
        commands::browse::list_plugins,
        commands::browse::list_available_skills,
        commands::browse::install_skills,
        commands::browse::get_project_info,
        commands::installed::list_installed_skills,
        commands::installed::remove_skill,
        commands::marketplaces::add_marketplace,
        commands::marketplaces::remove_marketplace,
        commands::marketplaces::update_marketplace,
        // new commands
        commands::settings::get_settings,
        commands::settings::save_scan_roots,
        commands::settings::discover_projects,
        commands::settings::set_active_project,
    ])
}
```

**Step 2: Regenerate TypeScript bindings**

```bash
cargo test -p kiro-control-center generate_types -- --exact --ignored
```

This updates `src/lib/bindings.ts` with the new command types.

**Step 3: Verify bindings exist**

Check that `bindings.ts` now has `getSettings`, `saveScanRoots`, `discoverProjects`, `setActiveProject`.

**Step 4: Commit**

```
feat: register settings commands and regenerate TypeScript bindings
```

---

### Task 4: Add unit tests for settings commands

**Files:**
- Modify: `crates/kiro-control-center/src-tauri/src/commands/settings.rs`

**Step 1: Add tests**

Add a `#[cfg(test)] mod tests` at the bottom of `settings.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// Set KIRO_MARKET_CONFIG_DIR to a temp directory for isolated tests.
    fn with_temp_config<F: FnOnce()>(dir: &tempfile::TempDir, f: F) {
        // Note: env var mutation is not thread-safe, but test runner
        // runs these sequentially by default.
        unsafe { std::env::set_var("KIRO_MARKET_CONFIG_DIR", dir.path()); }
        f();
        unsafe { std::env::remove_var("KIRO_MARKET_CONFIG_DIR"); }
    }

    #[test]
    fn load_settings_returns_defaults_when_no_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        with_temp_config(&dir, || {
            let settings = load_settings();
            assert!(settings.scan_roots.is_empty());
            assert!(settings.last_project.is_none());
        });
    }

    #[test]
    fn save_and_load_settings_roundtrip() {
        let dir = tempfile::tempdir().expect("tempdir");
        with_temp_config(&dir, || {
            let mut settings = Settings::default();
            settings.scan_roots = vec!["~/repos".into(), "~/work".into()];
            settings.last_project = Some("/home/user/project".into());
            save_settings(&settings).expect("save");

            let loaded = load_settings();
            assert_eq!(loaded.scan_roots, vec!["~/repos", "~/work"]);
            assert_eq!(loaded.last_project.as_deref(), Some("/home/user/project"));
        });
    }

    #[test]
    fn shellexpand_tilde_expands_home() {
        let expanded = shellexpand_tilde("~/repos");
        assert!(!expanded.starts_with('~'), "tilde should be expanded: {expanded}");
        assert!(expanded.ends_with("repos"));
    }

    #[test]
    fn shellexpand_tilde_leaves_absolute_paths_alone() {
        let path = "/absolute/path";
        assert_eq!(shellexpand_tilde(path), path);
    }

    #[test]
    fn scan_for_projects_finds_kiro_dirs() {
        let dir = tempfile::tempdir().expect("tempdir");

        // Create two projects: one at depth 1, one at depth 2.
        let proj1 = dir.path().join("project-a");
        std::fs::create_dir_all(proj1.join(".kiro")).expect("create .kiro");

        let org = dir.path().join("org");
        let proj2 = org.join("project-b");
        std::fs::create_dir_all(proj2.join(".kiro")).expect("create .kiro");

        // Create a non-project directory (no .kiro).
        std::fs::create_dir_all(dir.path().join("not-a-project")).expect("create dir");

        let mut results = Vec::new();
        scan_for_projects(dir.path(), 0, 2, &mut results);

        assert_eq!(results.len(), 2, "should find 2 projects: {results:?}");

        let names: Vec<&str> = results.iter().map(|p| p.name.as_str()).collect();
        assert!(names.contains(&"project-a"), "missing project-a: {names:?}");
        assert!(names.contains(&"project-b"), "missing project-b: {names:?}");
    }

    #[test]
    fn scan_for_projects_respects_max_depth() {
        let dir = tempfile::tempdir().expect("tempdir");

        // Create project at depth 3 (should NOT be found with max_depth=2).
        let deep = dir.path().join("a").join("b").join("c");
        std::fs::create_dir_all(deep.join(".kiro")).expect("create .kiro");

        let mut results = Vec::new();
        scan_for_projects(dir.path(), 0, 2, &mut results);

        assert!(results.is_empty(), "should not find projects at depth 3");
    }

    #[test]
    fn scan_for_projects_skips_hidden_and_build_dirs() {
        let dir = tempfile::tempdir().expect("tempdir");

        // Create .kiro inside hidden, node_modules, and target dirs.
        for name in &[".hidden", "node_modules", "target"] {
            let d = dir.path().join(name).join("sneaky");
            std::fs::create_dir_all(d.join(".kiro")).expect("create .kiro");
        }

        let mut results = Vec::new();
        scan_for_projects(dir.path(), 0, 2, &mut results);

        assert!(results.is_empty(), "should skip hidden/build dirs: {results:?}");
    }
}
```

**Step 2: Run tests**

Run: `cargo test -p kiro-control-center settings`
Expected: All 5 tests pass.

**Step 3: Commit**

```
test: add unit tests for settings commands and project scanning
```

---

### Task 5: Create shared project store (Svelte)

**Files:**
- Create: `crates/kiro-control-center/src/lib/stores/project.svelte.ts`
- Create: `crates/kiro-control-center/src/lib/stores/` (directory)

**Step 1: Create the store**

Create `src/lib/stores/project.svelte.ts`:

```typescript
import { commands } from "$lib/bindings";
import type { ProjectInfo, Settings, DiscoveredProject } from "$lib/bindings";

// ---------------------------------------------------------------------------
// Reactive state (exported as $state object — Svelte 5 Pattern A)
//
// Components read properties directly: `store.projectPath`, `store.loading`.
// Property access on the $state proxy is reactive — no getters needed.
// See: https://svelte.dev/docs/svelte/$state#Exporting-state
// ---------------------------------------------------------------------------

export const store = $state({
  projectPath: null as string | null,
  projectInfo: null as ProjectInfo | null,
  projectError: null as string | null,
  settings: { scan_roots: [], last_project: null } as Settings,
  discoveredProjects: [] as DiscoveredProject[],
  loading: true,
});

// ---------------------------------------------------------------------------
// Actions (mutate the store object's properties)
// ---------------------------------------------------------------------------

export async function initialize() {
  store.loading = true;
  store.projectError = null;

  // Load settings.
  const settingsResult = await commands.getSettings();
  if (settingsResult.status === "ok") {
    store.settings = settingsResult.data;
  }

  // Discover projects.
  await refreshProjects();

  // Restore last project if it still exists on disk.
  if (store.settings.last_project) {
    const found = store.discoveredProjects.find(
      (p) => p.path === store.settings.last_project
    );
    if (found) {
      await selectProject(store.settings.last_project);
    }
  }

  store.loading = false;
}

export async function selectProject(path: string) {
  store.projectError = null;
  const result = await commands.setActiveProject(path);
  if (result.status === "ok") {
    store.projectPath = result.data.path;
    store.projectInfo = result.data;
  } else {
    store.projectError = result.error.message;
  }
}

export async function refreshProjects() {
  const result = await commands.discoverProjects();
  if (result.status === "ok") {
    store.discoveredProjects = result.data;
  }
}

export async function addScanRoot(root: string) {
  const roots = [...store.settings.scan_roots, root];
  const result = await commands.saveScanRoots(roots);
  if (result.status === "ok") {
    store.settings = { ...store.settings, scan_roots: roots };
    await refreshProjects();
  }
}

export async function removeScanRoot(root: string) {
  const roots = store.settings.scan_roots.filter((r: string) => r !== root);
  const result = await commands.saveScanRoots(roots);
  if (result.status === "ok") {
    store.settings = { ...store.settings, scan_roots: roots };
    await refreshProjects();
  }
}

export function clearProject() {
  store.projectPath = null;
  store.projectInfo = null;
}
```

**Reactivity note:** Components import `store` and read properties directly in templates:
```svelte
<script>
  import { store, selectProject } from "$lib/stores/project.svelte";
</script>

<!-- These are reactive — property access on the $state proxy is tracked -->
{#if store.projectPath}
  <p>Active: {store.projectPath}</p>
{:else}
  <p>No project selected</p>
{/if}
```

Do NOT capture store properties in local `let` variables (they become snapshots). Use `$derived` if a local reactive binding is needed:
```svelte
<script>
  let hasProject = $derived(store.projectPath !== null);
</script>
```

**Step 2: Commit**

```
feat: add shared Svelte store for project state management
```

---

### Task 6: Create ProjectPicker (landing screen) component

**Files:**
- Create: `crates/kiro-control-center/src/lib/components/ProjectPicker.svelte`

This is the full-page landing screen shown when no project is active.

**Step 1: Create the component**

Create `src/lib/components/ProjectPicker.svelte`:

```svelte
<script lang="ts">
  import { open } from "@tauri-apps/plugin-dialog";
  import { store, selectProject, addScanRoot, refreshProjects } from "$lib/stores/project.svelte";

  async function handleAddDirectory() {
    const selected = await open({ directory: true, title: "Select a directory to scan for projects" });
    if (selected === null) return; // User cancelled
    await addScanRoot(selected);
  }

  async function handleOpenOther() {
    const selected = await open({ directory: true, title: "Select a Kiro project" });
    if (selected === null) return; // User cancelled
    await selectProject(selected);
  }
</script>

<div class="flex items-center justify-center h-full bg-gray-100 dark:bg-gray-950">
  <div class="max-w-2xl w-full mx-auto p-8">
    <h1 class="text-2xl font-bold text-gray-900 dark:text-gray-100 mb-2">Kiro Control Center</h1>
    <p class="text-gray-500 dark:text-gray-400 mb-8">Select a project to manage its skills.</p>

    {#if store.discoveredProjects.length > 0}
      <div class="space-y-2 mb-6">
        {#each store.discoveredProjects as project (project.path)}
          <button
            class="w-full text-left px-4 py-3 rounded-lg border border-gray-200 dark:border-gray-700 bg-white dark:bg-gray-900 hover:border-blue-400 dark:hover:border-blue-500 transition-colors"
            onclick={() => selectProject(project.path)}
          >
            <div class="font-medium text-gray-900 dark:text-gray-100">{project.name}</div>
            <div class="text-sm text-gray-500 dark:text-gray-400 truncate">{project.path}</div>
          </button>
        {/each}
      </div>
    {:else if store.settings.scan_roots.length > 0}
      <p class="text-gray-500 dark:text-gray-400 mb-6">No projects found in your configured directories.</p>
    {/if}

    <div class="flex gap-3">
      <button
        class="px-4 py-2 rounded-lg bg-blue-600 text-white hover:bg-blue-700 transition-colors text-sm font-medium"
        onclick={handleAddDirectory}
      >
        Add Directory to Scan
      </button>
      <button
        class="px-4 py-2 rounded-lg border border-gray-300 dark:border-gray-600 text-gray-700 dark:text-gray-300 hover:bg-gray-50 dark:hover:bg-gray-800 transition-colors text-sm font-medium"
        onclick={handleOpenOther}
      >
        Open Other...
      </button>
    </div>

    {#if store.settings.scan_roots.length > 0}
      <div class="mt-8 text-xs text-gray-400 dark:text-gray-500">
        Scanning: {store.settings.scan_roots.join(", ")}
      </div>
    {/if}
  </div>
</div>
```

**Key design decisions:**
- Projects shown as a list of clickable rows (not cards or grid — simpler, works at any count)
- "Add Directory" is primary (blue), "Open Other" is secondary (outlined)
- Dialog `open()` returns `null` on cancel — guarded with early return
- Discovered projects read from `store.discoveredProjects` (reactive via $state proxy)
- Scan roots shown as a small footer for context

**Step 2: Commit**

```
feat: add ProjectPicker landing screen component
```

---

### Task 7: Create ProjectDropdown (header switcher) component

**Files:**
- Create: `crates/kiro-control-center/src/lib/components/ProjectDropdown.svelte`

This is the dropdown in the header for quick project switching.

**Step 1: Create the component**

Create `src/lib/components/ProjectDropdown.svelte`:

```svelte
<script lang="ts">
  import { open } from "@tauri-apps/plugin-dialog";
  import { store, selectProject, clearProject } from "$lib/stores/project.svelte";

  let { onManageRoots }: { onManageRoots: () => void } = $props();

  let isOpen = $state(false);

  function toggle() {
    isOpen = !isOpen;
  }

  function close() {
    isOpen = false;
  }

  async function handleSelectProject(path: string) {
    close();
    await selectProject(path);
  }

  async function handleOpenOther() {
    close();
    const selected = await open({ directory: true, title: "Select a Kiro project" });
    if (selected === null) return;
    await selectProject(selected);
  }

  function handleManageRoots() {
    close();
    onManageRoots();
  }
</script>

<!-- Click-outside handler -->
{#if isOpen}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="fixed inset-0 z-40" onclick={close} onkeydown={() => {}}></div>
{/if}

<div class="relative">
  <button
    class="flex items-center gap-2 text-sm text-gray-700 dark:text-gray-300 hover:text-gray-900 dark:hover:text-gray-100 transition-colors"
    onclick={toggle}
  >
    <span class="truncate max-w-xs font-medium">
      {store.projectInfo?.path ?? "No project"}
    </span>
    <svg class="w-4 h-4 opacity-50" fill="none" stroke="currentColor" viewBox="0 0 24 24">
      <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M19 9l-7 7-7-7" />
    </svg>
  </button>

  {#if isOpen}
    <div class="absolute right-0 top-full mt-1 w-80 bg-white dark:bg-gray-800 rounded-lg shadow-lg border border-gray-200 dark:border-gray-700 z-50 overflow-hidden">
      {#if store.discoveredProjects.length > 0}
        <div class="max-h-64 overflow-y-auto py-1">
          {#each store.discoveredProjects as project (project.path)}
            <button
              class="w-full text-left px-4 py-2 hover:bg-gray-100 dark:hover:bg-gray-700 transition-colors
                {project.path === store.projectPath ? 'bg-blue-50 dark:bg-blue-900/20' : ''}"
              onclick={() => handleSelectProject(project.path)}
            >
              <div class="text-sm font-medium text-gray-900 dark:text-gray-100">{project.name}</div>
              <div class="text-xs text-gray-500 dark:text-gray-400 truncate">{project.path}</div>
            </button>
          {/each}
        </div>
      {/if}

      <div class="border-t border-gray-200 dark:border-gray-700 py-1">
        <button
          class="w-full text-left px-4 py-2 text-sm text-gray-600 dark:text-gray-400 hover:bg-gray-100 dark:hover:bg-gray-700"
          onclick={handleOpenOther}
        >
          Open Other...
        </button>
        <button
          class="w-full text-left px-4 py-2 text-sm text-gray-600 dark:text-gray-400 hover:bg-gray-100 dark:hover:bg-gray-700"
          onclick={handleManageRoots}
        >
          Manage Directories...
        </button>
      </div>
    </div>
  {/if}
</div>
```

**Key design decisions:**
- Click-outside uses a full-screen invisible overlay (`fixed inset-0 z-40`)
- Active project highlighted with blue background
- `onManageRoots` callback passed as `$props` (Svelte 5 pattern, not events)
- Dropdown max-height with scroll for many projects
- Dialog `open()` cancel guarded with null check

**Step 2: Commit**

```
feat: add ProjectDropdown header component for project switching
```

---

### Task 8: Create ScanRootsPanel component

**Files:**
- Create: `crates/kiro-control-center/src/lib/components/ScanRootsPanel.svelte`

A small panel/modal for managing scan root directories.

**Step 1: Create the component**

Create `src/lib/components/ScanRootsPanel.svelte`:

```svelte
<script lang="ts">
  import { open } from "@tauri-apps/plugin-dialog";
  import { store, addScanRoot, removeScanRoot } from "$lib/stores/project.svelte";

  let { onClose }: { onClose: () => void } = $props();

  async function handleAddRoot() {
    const selected = await open({ directory: true, title: "Select a directory to scan" });
    if (selected === null) return;
    await addScanRoot(selected);
  }
</script>

<!-- Modal backdrop -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="fixed inset-0 z-50 flex items-center justify-center bg-black/50" onclick={onClose} onkeydown={() => {}}>
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    class="bg-white dark:bg-gray-800 rounded-lg shadow-xl w-full max-w-md mx-4 overflow-hidden"
    onclick={(e) => e.stopPropagation()}
    onkeydown={() => {}}
  >
    <div class="flex items-center justify-between px-4 py-3 border-b border-gray-200 dark:border-gray-700">
      <h2 class="text-sm font-semibold text-gray-900 dark:text-gray-100">Scan Directories</h2>
      <button
        class="text-gray-400 hover:text-gray-600 dark:hover:text-gray-300"
        onclick={onClose}
      >
        <svg class="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12" />
        </svg>
      </button>
    </div>

    <div class="p-4">
      {#if store.settings.scan_roots.length > 0}
        <ul class="space-y-2 mb-4">
          {#each store.settings.scan_roots as root (root)}
            <li class="flex items-center justify-between px-3 py-2 bg-gray-50 dark:bg-gray-700/50 rounded text-sm">
              <span class="truncate text-gray-700 dark:text-gray-300">{root}</span>
              <button
                class="ml-2 text-gray-400 hover:text-red-500 flex-shrink-0"
                onclick={() => removeScanRoot(root)}
              >
                <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12" />
                </svg>
              </button>
            </li>
          {/each}
        </ul>
      {:else}
        <p class="text-sm text-gray-500 dark:text-gray-400 mb-4">
          No directories configured. Add a directory to discover Kiro projects.
        </p>
      {/if}

      <button
        class="w-full px-4 py-2 rounded-lg border border-dashed border-gray-300 dark:border-gray-600 text-sm text-gray-600 dark:text-gray-400 hover:border-blue-400 hover:text-blue-500 transition-colors"
        onclick={handleAddRoot}
      >
        + Add Directory
      </button>
    </div>
  </div>
</div>
```

**Key design decisions:**
- Modal with backdrop (not inline panel) — keeps the main view uncluttered
- `onClose` callback via `$props` — parent controls visibility
- Backdrop click closes modal (`onclick={onClose}` + `stopPropagation` on inner div)
- Each root has an X button for removal
- "Add Directory" uses dashed border style to indicate an action area
- Dialog `open()` cancel guarded with null check

**Step 2: Commit**

```
feat: add ScanRootsPanel modal for managing scan directories
```

---

### Task 9: Wire everything together in +page.svelte

**Files:**
- Modify: `crates/kiro-control-center/src/routes/+page.svelte`

This is where it all comes together. The page conditionally shows the landing screen or the tabs.

**Step 1: Rewrite +page.svelte**

Replace the entire file with:

```svelte
<script lang="ts">
  import { store, initialize } from "$lib/stores/project.svelte";
  import TabBar from "$lib/components/TabBar.svelte";
  import BrowseTab from "$lib/components/BrowseTab.svelte";
  import InstalledTab from "$lib/components/InstalledTab.svelte";
  import MarketplacesTab from "$lib/components/MarketplacesTab.svelte";
  import ProjectPicker from "$lib/components/ProjectPicker.svelte";
  import ProjectDropdown from "$lib/components/ProjectDropdown.svelte";
  import ScanRootsPanel from "$lib/components/ScanRootsPanel.svelte";

  const tabs = ["Browse", "Installed", "Marketplaces"];
  let activeTab: string = $state("Browse");
  let showManageRoots = $state(false);

  // Initialize on mount — loads settings, discovers projects, restores last project.
  $effect(() => {
    initialize();
  });
</script>

{#if store.loading}
  <!-- Loading state -->
  <div class="flex items-center justify-center h-screen bg-gray-100 dark:bg-gray-950">
    <p class="text-gray-500 dark:text-gray-400">Loading...</p>
  </div>
{:else if store.projectPath}
  <!-- Active project — normal app experience -->
  <div class="flex flex-col h-screen bg-gray-100 dark:bg-gray-950 text-gray-900 dark:text-gray-100">
    <header class="flex items-center justify-between px-6 py-3 bg-white dark:bg-gray-900 border-b border-gray-200 dark:border-gray-700 shadow-sm">
      <h1 class="text-lg font-semibold">Kiro Control Center</h1>
      <ProjectDropdown onManageRoots={() => (showManageRoots = true)} />
    </header>

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
  </div>
{:else}
  <!-- No active project — show picker -->
  <ProjectPicker />
{/if}

{#if showManageRoots}
  <ScanRootsPanel onClose={() => (showManageRoots = false)} />
{/if}
```

**Key changes from the old +page.svelte:**
- `projectPath = "."` hardcode removed — state comes from the store
- `loadProjectInfo()` removed — `initialize()` handles everything
- Conditional rendering: loading → active project → picker
- Header uses `ProjectDropdown` instead of static path display
- `ScanRootsPanel` shown as a modal overlay when triggered from the dropdown
- `store.projectPath` passed directly to tab components (reactive via $state proxy)
- `store.projectPath` is `string | null` — the `{:else if store.projectPath}` block guarantees it's non-null when passed to tabs

**Step 2: Test manually**

```bash
cd crates/kiro-control-center
cargo tauri dev
```

Verify:
- First launch shows the landing screen
- Adding a root directory discovers projects
- Clicking a project loads the normal tab view
- The header dropdown allows switching
- Restarting the app restores the last project

**Step 3: Commit**

```
feat: wire project discovery into main page

Landing screen on first launch, header dropdown for switching.
Project state managed via shared Svelte store.
```

---

### Task 10: Final verification

**Step 1: Run all backend tests**

```bash
cargo test --workspace
cargo clippy --workspace -- -D warnings
```

**Step 2: Run frontend checks**

```bash
cd crates/kiro-control-center
npm run check
```

**Step 3: Verify the full flow manually**

```bash
cargo tauri dev
```

- [ ] First launch → landing screen appears
- [ ] "Add Directory" → OS folder picker works
- [ ] Projects discovered and listed
- [ ] Click project → tabs load with correct project
- [ ] Header dropdown → shows projects, can switch
- [ ] "Open Other..." → folder picker, loads one-off project
- [ ] Close and reopen app → restores last project
- [ ] Remove all roots → landing screen reappears

**Step 4: Commit any remaining fixes**

```
chore: final verification and cleanup for project discovery
```

---

## Platform Notes

**UNVERIFIED ASSUMPTION:** `dirs::config_dir()` returns:
- Linux: `$XDG_CONFIG_HOME` or `~/.config`
- macOS: `~/Library/Application Support`
- Windows: `{FOLDERID_RoamingAppData}`

The settings file path differs per platform. This should work but is not tested on macOS/Windows in CI. The `KIRO_MARKET_DATA_DIR` pattern from the cache module could be applied here if needed for test isolation.

**UNVERIFIED ASSUMPTION:** `tauri-plugin-dialog`'s folder picker works on all three platforms. This is a well-maintained Tauri plugin but we don't have automated E2E tests for it.

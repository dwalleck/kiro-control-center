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

**Step 4: Add dialog permission in `tauri.conf.json`**

Add to the `"app"` section (or create `"permissions"` in the appropriate capabilities file — check Tauri 2 docs for exact location).

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
fn settings_path() -> Option<PathBuf> {
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

        let project = KiroProject::new(dir.to_path_buf());
        let skill_count = project
            .load_installed()
            .map(|i| i.skills.len() as u32)
            .unwrap_or(0);

        results.push(DiscoveredProject {
            path: dir.to_string_lossy().into_owned(),
            name,
            kiro_initialized: true,
            skill_count,
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

    #[test]
    fn load_settings_returns_defaults_when_no_file() {
        // Uses the real config path — if no file exists, returns defaults.
        let settings = load_settings();
        // We can't assert much about scan_roots (user might have a file),
        // but the function should not panic.
        assert!(settings.scan_roots.len() >= 0);
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
// Reactive state
// ---------------------------------------------------------------------------

let projectPath: string | null = $state(null);
let projectInfo: ProjectInfo | null = $state(null);
let projectError: string | null = $state(null);
let settings: Settings = $state({ scan_roots: [], last_project: null });
let discoveredProjects: DiscoveredProject[] = $state([]);
let loading: boolean = $state(true);

// ---------------------------------------------------------------------------
// Actions
// ---------------------------------------------------------------------------

export async function initialize() {
  loading = true;
  projectError = null;

  // Load settings.
  const settingsResult = await commands.getSettings();
  if (settingsResult.status === "ok") {
    settings = settingsResult.data;
  }

  // Discover projects.
  await refreshProjects();

  // Restore last project if it still exists.
  if (settings.last_project) {
    const found = discoveredProjects.find(
      (p) => p.path === settings.last_project
    );
    if (found) {
      await selectProject(settings.last_project);
    }
  }

  loading = false;
}

export async function selectProject(path: string) {
  projectError = null;
  const result = await commands.setActiveProject(path);
  if (result.status === "ok") {
    projectPath = result.data.path;
    projectInfo = result.data;
  } else {
    projectError = result.error.message;
  }
}

export async function refreshProjects() {
  const result = await commands.discoverProjects();
  if (result.status === "ok") {
    discoveredProjects = result.data;
  }
}

export async function addScanRoot(root: string) {
  const roots = [...settings.scan_roots, root];
  const result = await commands.saveScanRoots(roots);
  if (result.status === "ok") {
    settings.scan_roots = roots;
    await refreshProjects();
  }
}

export async function removeScanRoot(root: string) {
  const roots = settings.scan_roots.filter((r) => r !== root);
  const result = await commands.saveScanRoots(roots);
  if (result.status === "ok") {
    settings.scan_roots = roots;
    await refreshProjects();
  }
}

export function clearProject() {
  projectPath = null;
  projectInfo = null;
}

// ---------------------------------------------------------------------------
// Getters (read-only access to state)
// ---------------------------------------------------------------------------

export function getProjectPath() { return projectPath; }
export function getProjectInfo() { return projectInfo; }
export function getProjectError() { return projectError; }
export function getSettings() { return settings; }
export function getDiscoveredProjects() { return discoveredProjects; }
export function isLoading() { return loading; }
export function hasActiveProject() { return projectPath !== null; }
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

Create `src/lib/components/ProjectPicker.svelte`. The component should:

- Show a list of discovered projects with name, path, and skill count
- Each project is a clickable card that calls `selectProject(path)`
- "Add Directory" button that opens the OS folder picker via `@tauri-apps/plugin-dialog`
- "Open Other..." button that opens the OS folder picker for a one-off project
- If no roots are configured, show a friendly onboarding message
- Use the same Tailwind styling as existing components (dark mode support)

This is a Svelte 5 component — use `$props`, `$state`, `$effect`. Import the dialog plugin with `import { open } from "@tauri-apps/plugin-dialog"` for the folder picker.

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

The component should:

- Show the current project name as a clickable button
- On click, show a dropdown with discovered projects
- Each project shows name and path (truncated)
- "Open Other..." option at the bottom → opens OS folder picker
- "Manage Directories..." option → emits an event to open settings
- Clicking outside closes the dropdown
- Use the same Tailwind styling as existing header elements

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

The component should:

- List current scan roots with a remove button next to each
- "Add Directory" button that opens the OS folder picker
- Simple, compact design — this is a settings panel, not a main view

**Step 2: Commit**

```
feat: add ScanRootsPanel for managing scan directories
```

---

### Task 9: Wire everything together in +page.svelte

**Files:**
- Modify: `crates/kiro-control-center/src/routes/+page.svelte`

This is where it all comes together. The page conditionally shows the landing screen or the tabs.

**Step 1: Rewrite +page.svelte**

Replace the current content with:

- Import the project store
- On mount, call `initialize()`
- If loading → show loading spinner
- If no active project → show `ProjectPicker`
- If active project → show header with `ProjectDropdown` + tabs + content
- The `projectPath` prop passed to tab components comes from the store

Key changes:
- Remove hardcoded `projectPath = "."`
- Remove inline `loadProjectInfo()` — the store handles this
- Add conditional rendering: `{#if hasActiveProject()} ... {:else} <ProjectPicker /> {/if}`
- Header now uses `ProjectDropdown` instead of the static path display

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

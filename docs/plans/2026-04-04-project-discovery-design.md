# Project Discovery & Selection — Design

**Date:** 2026-04-04
**Goal:** Allow users to discover, select, and switch between Kiro projects in the control center instead of being locked to the directory the app was launched from.

---

## Current State

The control center hardcodes `projectPath = "."` on startup. There is no UI to change it, no persistence, and no project discovery. Every Tauri command takes `project_path: String` as a parameter — the backend is fully stateless.

## Design Decisions

- **One project at a time** — IDE-style workspace model, not a multi-project dashboard
- **Configured roots + file picker** — User adds root directories (e.g. `~/repos`), app scans for `.kiro` projects. File picker available for one-off projects outside configured roots.
- **Roots persist, discovery is fresh** — Root directories saved to config. Project list re-scanned on each launch so new projects appear and deleted ones disappear. Last-active project also persisted.
- **Landing screen + header dropdown** — Full-page picker on first launch or when no project is selected. Header dropdown for quick switching during normal use.

---

## 1. Data Model

### Config file

Location: `~/.config/kiro-market/settings.json` (via `dirs::config_dir()`)

```json
{
  "scan_roots": ["~/repos", "~/work"],
  "last_project": "/home/user/repos/my-app"
}
```

- `scan_roots`: directories to scan for `.kiro` projects (persisted across launches)
- `last_project`: path of the most recently selected project (persisted)

### Discovered project

```rust
pub struct DiscoveredProject {
    pub path: String,
    pub name: String,           // directory name
    pub kiro_initialized: bool, // .kiro/ exists
    pub skill_count: u32,       // number of installed skills
}
```

### Scan depth

Only check immediate children and grandchildren of each root (depth 1-2). `~/repos/my-app/.kiro` and `~/repos/org/my-app/.kiro` are found. Deeper nesting is not scanned.

---

## 2. Backend Commands

Four new Tauri commands in `commands/settings.rs`:

| Command | Parameters | Returns | Purpose |
|---------|-----------|---------|---------|
| `get_settings()` | none | `Settings` | Load config file (defaults if missing) |
| `save_scan_roots(roots)` | `Vec<String>` | `()` | Persist root directories |
| `discover_projects()` | none | `Vec<DiscoveredProject>` | Scan roots for `.kiro` projects |
| `set_active_project(path)` | `String` | `ProjectInfo` | Validate, persist, and activate a project |

Existing commands (`list_installed_skills`, `install_skills`, etc.) stay unchanged — they still take `project_path: String` per-request. The frontend passes the currently active project path.

---

## 3. UI Flow

### First launch (no config)

1. `get_settings()` → empty defaults
2. Landing screen: "No projects found. Add a directory to scan."
3. User clicks "Add Directory" → OS folder picker
4. `save_scan_roots(roots)` → `discover_projects()`
5. Landing screen populates with project list
6. User clicks a project → `set_active_project(path)` → normal tabs appear

### Subsequent launches

1. `get_settings()` → saved roots + last project
2. If `last_project` exists on disk → go straight to tabs with that project active
3. If `last_project` no longer exists → show landing screen

### Switching projects (header dropdown)

1. Click project name in header → dropdown opens
2. Shows discovered projects sorted by name
3. "Open Other..." → OS file picker
4. "Manage Directories..." → settings panel for scan roots
5. Pick a project → `set_active_project(path)` → all tabs reload

### Landing screen vs dropdown

Both show the same `Vec<DiscoveredProject>` data. The landing screen is the full-page version for when there's no active project.

---

## 4. What We're NOT Building

- No project creation — discovers existing `.kiro` projects only
- No multi-project view — one active project at a time
- No root `.kiro` configuration management (parked for later)
- No deep scanning — max 2 levels, no recursive walks
- No file watching — refresh on launch and root changes only
- No project metadata caching — reads `.kiro/installed-skills.json` on discovery

---

## 5. Existing Code Impact

### Backend (minimal changes)
- New `commands/settings.rs` module with 4 commands
- New `Settings` type (config file serialization)
- New project scanning logic in `kiro-market-core` or inline in commands
- Register new commands in `lib.rs`
- No changes to existing commands

### Frontend (moderate changes)
- New `ProjectPicker.svelte` component (landing screen + used in dropdown)
- New `ProjectDropdown.svelte` component (header project switcher)
- New `SettingsPanel.svelte` component (manage scan roots)
- Modify `+page.svelte` to conditionally show landing screen vs tabs
- Modify `+layout.svelte` header to show project dropdown
- `projectPath` state moves to a shared store (accessible from layout + page)

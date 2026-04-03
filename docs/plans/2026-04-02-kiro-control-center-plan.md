# Kiro Control Center (kcc) Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Build a Tauri v2 + Svelte 5 desktop app that provides a tabbed GUI for browsing, installing, and managing Claude Code marketplace skills in Kiro projects.

**Architecture:** New `kiro-control-center` crate in the workspace. Tauri commands call `kiro-market-core` directly (no subprocess). `tauri-specta` generates typed TypeScript bindings. SvelteKit frontend with `adapter-static` in SPA mode. Project scoped to the working directory at launch.

**Tech Stack:** Tauri v2, Svelte 5, SvelteKit, TypeScript, tauri-specta, Tailwind CSS

**Reference project:** `/home/dwalleck/repos/mental-health-bar-rs/` — follow the same patterns for command structure, error handling, specta setup, and SvelteKit configuration.

---

## Task 1: Scaffold the Tauri + SvelteKit project

**Files:**
- Create: `crates/kiro-control-center/` (entire Tauri scaffold)
- Modify: `Cargo.toml` (workspace members)

**Step 1: Add crate to workspace**

In the root `Cargo.toml`, add the new crate to workspace members:

```toml
members = ["crates/kiro-market-core", "crates/kiro-market", "crates/kiro-control-center/src-tauri"]
```

**Step 2: Scaffold Tauri project**

From `crates/kiro-control-center/`:

```bash
npm create tauri-app@latest . -- --template sveltekit-ts --manager npm
```

If the scaffold tool doesn't support `.` as destination, create in a temp name and move. The result should match:

```
crates/kiro-control-center/
  src-tauri/
    src/main.rs
    Cargo.toml
    tauri.conf.json
    build.rs
    icons/
  src/
    routes/+page.svelte
    app.html
  package.json
  svelte.config.js
  vite.config.js (or .ts)
  tsconfig.json
```

**Step 3: Configure tauri.conf.json**

```json
{
  "$schema": "https://schema.tauri.app/config/2",
  "productName": "Kiro Control Center",
  "version": "0.1.0",
  "identifier": "com.dwalleck.kiro-control-center",
  "build": {
    "beforeDevCommand": "npm run dev",
    "devUrl": "http://localhost:1420",
    "beforeBuildCommand": "npm run build",
    "frontendDist": "../build"
  },
  "app": {
    "windows": [
      {
        "title": "Kiro Control Center",
        "width": 960,
        "height": 700
      }
    ],
    "security": {
      "csp": null
    }
  },
  "bundle": {
    "active": true,
    "targets": "all",
    "icon": [
      "icons/32x32.png",
      "icons/128x128.png",
      "icons/128x128@2x.png",
      "icons/icon.icns",
      "icons/icon.ico"
    ]
  }
}
```

**Step 4: Configure svelte.config.js for SPA mode**

```js
import adapter from "@sveltejs/adapter-static";
import { vitePreprocess } from "@sveltejs/vite-plugin-svelte";

/** @type {import('@sveltejs/kit').Config} */
const config = {
  preprocess: vitePreprocess(),
  kit: {
    adapter: adapter({
      fallback: "index.html",
    }),
  },
};

export default config;
```

**Step 5: Configure vite.config.js for Tauri**

Use the same config as mental-health-bar-rs — fixed port 1420, ignore `src-tauri/`, HMR support.

**Step 6: Verify scaffold builds**

```bash
cd crates/kiro-control-center
npm install
cd src-tauri && cargo check
cd .. && npm run build
```

**Step 7: Commit**

```bash
git add crates/kiro-control-center/ Cargo.toml
git commit -m "feat(kcc): scaffold Tauri v2 + SvelteKit project"
```

---

## Task 2: Configure Cargo.toml with dependencies and specta

**Files:**
- Modify: `crates/kiro-control-center/src-tauri/Cargo.toml`

**Step 1: Set up Cargo.toml**

```toml
[package]
name = "kiro-control-center"
version = "0.1.0"
edition = "2024"
description = "Kiro Control Center — GUI for managing Claude Code marketplace skills"

[[bin]]
name = "kcc"

[lib]
name = "kcc_lib"
crate-type = ["staticlib", "cdylib", "rlib"]

[build-dependencies]
tauri-build = { version = "2", features = [] }
tauri-specta = { version = "2.0.0-rc.20", features = ["typescript"] }

[dependencies]
kiro-market-core = { path = "../../kiro-market-core" }
tauri = { version = "2", features = [] }
tauri-plugin-opener = "2"
tauri-specta = { version = "2.0.0-rc.20", features = ["typescript"] }
specta = { version = "2.0.0-rc.20" }
specta-typescript = "0.0.7"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
chrono = { version = "0.4", features = ["serde"] }
thiserror = "2"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
tokio = { version = "1", features = ["full"] }

[profile.dev]
opt-level = 0
debug = 1
incremental = true

[profile.dev.package."*"]
opt-level = 0
debug = 0
```

**Step 2: Verify it compiles**

```bash
cd crates/kiro-control-center/src-tauri && cargo check
```

**Step 3: Commit**

```bash
git add crates/kiro-control-center/src-tauri/Cargo.toml
git commit -m "feat(kcc): configure Cargo dependencies with kiro-market-core and specta"
```

---

## Task 3: Implement error types and Tauri command infrastructure

**Files:**
- Create: `crates/kiro-control-center/src-tauri/src/error.rs`
- Modify: `crates/kiro-control-center/src-tauri/src/lib.rs`
- Create: `crates/kiro-control-center/src-tauri/src/commands/mod.rs`

**Step 1: Create error.rs**

Follow mental-health-bar-rs pattern. Map `kiro_market_core::error::Error` variants to `CommandError`:

```rust
use serde::Serialize;

#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum ErrorType {
    NotFound,
    AlreadyExists,
    Validation,
    GitError,
    IoError,
    ParseError,
    Unknown,
}

#[derive(Debug, Clone, Serialize, specta::Type)]
pub struct CommandError {
    pub message: String,
    pub error_type: ErrorType,
}

impl CommandError {
    pub fn new(message: impl Into<String>, error_type: ErrorType) -> Self {
        Self {
            message: message.into(),
            error_type,
        }
    }
}

impl From<kiro_market_core::error::Error> for CommandError {
    fn from(err: kiro_market_core::error::Error) -> Self {
        use kiro_market_core::error::Error;
        let error_type = match &err {
            Error::Marketplace(e) => match e {
                kiro_market_core::error::MarketplaceError::NotFound { .. } => ErrorType::NotFound,
                kiro_market_core::error::MarketplaceError::AlreadyRegistered { .. } => ErrorType::AlreadyExists,
                _ => ErrorType::ParseError,
            },
            Error::Skill(e) => match e {
                kiro_market_core::error::SkillError::AlreadyInstalled { .. } => ErrorType::AlreadyExists,
                kiro_market_core::error::SkillError::NotInstalled { .. } => ErrorType::NotFound,
                _ => ErrorType::NotFound,
            },
            Error::Validation(_) => ErrorType::Validation,
            Error::Git(_) => ErrorType::GitError,
            Error::Io(_) => ErrorType::IoError,
            Error::Json(_) => ErrorType::ParseError,
            _ => ErrorType::Unknown,
        };
        CommandError::new(err.to_string(), error_type)
    }
}
```

**Step 2: Create commands/mod.rs**

```rust
pub mod browse;
pub mod installed;
pub mod marketplaces;
```

**Step 3: Set up lib.rs with specta builder**

Follow mental-health-bar-rs lib.rs structure. Register commands with `collect_commands!`, export bindings in debug mode, set up Tauri builder:

```rust
use tauri_specta::{collect_commands, Builder};

mod commands;
mod error;

pub fn run() {
    let builder = Builder::<tauri::Wry>::new()
        .commands(collect_commands![
            // Commands will be added in later tasks
        ]);

    #[cfg(debug_assertions)]
    builder
        .export(
            specta_typescript::Typescript::default(),
            "../src/lib/bindings.ts",
        )
        .expect("Failed to export typescript bindings");

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(builder.invoke_handler())
        .setup(move |app| {
            builder.mount_events(app);
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
```

**Step 4: Update main.rs**

```rust
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    tracing::info!("Starting Kiro Control Center");
    kcc_lib::run();
}
```

**Step 5: Verify it compiles**

```bash
cd crates/kiro-control-center/src-tauri && cargo check
```

**Step 6: Commit**

```bash
git add crates/kiro-control-center/src-tauri/src/
git commit -m "feat(kcc): add error types, command modules, and specta binding setup"
```

---

## Task 4: Implement browse commands

**Files:**
- Create: `crates/kiro-control-center/src-tauri/src/commands/browse.rs`
- Modify: `crates/kiro-control-center/src-tauri/src/lib.rs` (register commands)

**Step 1: Define response types and implement commands**

Create `browse.rs` with these commands:
- `list_marketplaces()` — loads known marketplaces from cache, reads each manifest to count plugins
- `list_plugins(marketplace)` — reads marketplace manifest, returns plugins with skill counts
- `list_available_skills(marketplace, plugin)` — resolves plugin dir, discovers skills, reads frontmatter, cross-references with installed skills in project
- `install_skills(marketplace, plugin, skills, force, project_path)` — installs selected skills

Each command follows the pattern:
```rust
#[tauri::command]
#[specta::specta]
pub async fn list_marketplaces() -> Result<Vec<MarketplaceInfo>, CommandError> {
    let cache = CacheDir::default_location()
        .ok_or_else(|| CommandError::new("could not determine data directory", ErrorType::IoError))?;
    // ... call kiro_market_core functions, map errors with ?/into()
}
```

All response types derive `Serialize` and `specta::Type`.

The `install_skills` command needs `project_path: String` passed from the frontend (resolved at app startup from cwd).

**Step 2: Register in lib.rs**

Add all browse commands to the `collect_commands!` macro.

**Step 3: Verify it compiles and bindings generate**

```bash
cd crates/kiro-control-center/src-tauri && cargo check
```

**Step 4: Commit**

```bash
git commit -am "feat(kcc): implement browse commands (list marketplaces, plugins, skills, install)"
```

---

## Task 5: Implement installed and marketplace commands

**Files:**
- Create: `crates/kiro-control-center/src-tauri/src/commands/installed.rs`
- Create: `crates/kiro-control-center/src-tauri/src/commands/marketplaces.rs`
- Modify: `crates/kiro-control-center/src-tauri/src/lib.rs` (register commands)

**Step 1: Implement installed.rs**

Two commands:
- `list_installed_skills(project_path)` — creates `KiroProject`, calls `load_installed()`, maps to response type with name, marketplace, plugin, version, installed_at
- `remove_skill(name, project_path)` — calls `project.remove_skill()`

**Step 2: Implement marketplaces.rs**

Three commands:
- `add_marketplace(source)` — mirrors the CLI `marketplace add` logic: detect source, clone/symlink, read manifest, validate name, rename, register. Returns `MarketplaceAddResult` with name and plugin list.
- `remove_marketplace(name)` — calls `cache.remove_known_marketplace()` + removes directory
- `update_marketplace(name)` — calls `git::pull_repo()` for each target

**Step 3: Add get_project_info utility command**

Either in `browse.rs` or a new `util.rs`:
```rust
#[tauri::command]
#[specta::specta]
pub async fn get_project_info(project_path: String) -> Result<ProjectInfo, CommandError> {
    let project = KiroProject::new(PathBuf::from(&project_path));
    let installed = project.load_installed().map_err(CommandError::from)?;
    Ok(ProjectInfo {
        path: project_path,
        kiro_initialized: PathBuf::from(&project_path).join(".kiro").exists(),
        installed_skill_count: installed.skills.len(),
    })
}
```

**Step 4: Register all commands in lib.rs**

Update `collect_commands!` with all commands from browse, installed, marketplaces.

**Step 5: Run `cargo check` and verify bindings.ts generates**

```bash
cd crates/kiro-control-center/src-tauri && cargo test  # triggers binding generation
ls ../src/lib/bindings.ts
```

**Step 6: Commit**

```bash
git commit -am "feat(kcc): implement installed, marketplace, and utility commands"
```

---

## Task 6: Install frontend dependencies and configure Tailwind

**Files:**
- Modify: `crates/kiro-control-center/package.json`
- Create/Modify: Tailwind config files

**Step 1: Install dependencies**

```bash
cd crates/kiro-control-center
npm install @tauri-apps/api@^2
npm install -D @tauri-apps/cli@^2 tailwindcss @tailwindcss/postcss postcss
```

**Step 2: Configure Tailwind**

Add Tailwind's PostCSS plugin. Create `src/app.css` with Tailwind imports:

```css
@import 'tailwindcss';
```

Import it in `src/routes/+layout.svelte`:

```svelte
<script>
  import '../app.css';
  let { children } = $props();
</script>

{@render children()}
```

**Step 3: Verify dev server starts**

```bash
npm run dev
```

**Step 4: Commit**

```bash
git commit -am "feat(kcc): configure Tailwind CSS and frontend dependencies"
```

---

## Task 7: Build the tab layout shell

**Files:**
- Create: `crates/kiro-control-center/src/lib/components/TabBar.svelte`
- Modify: `crates/kiro-control-center/src/routes/+layout.svelte`
- Modify: `crates/kiro-control-center/src/routes/+page.svelte`

**Step 1: Create TabBar component**

A horizontal tab bar with three tabs: Browse, Installed, Marketplaces. Uses Svelte 5 runes (`$state`, `$props`). Highlights the active tab.

**Step 2: Wire up +layout.svelte**

Import app.css, render TabBar at top, render page content below.

**Step 3: Set up +page.svelte with conditional tab rendering**

Use a `$state` variable for the active tab. Conditionally render the tab content components (stub components for now — just the tab name as a heading).

**Step 4: Verify it renders in `npm run tauri dev`**

```bash
cd crates/kiro-control-center && npm run tauri dev
```

Should see a window with three clickable tabs and placeholder content.

**Step 5: Commit**

```bash
git commit -am "feat(kcc): build tab layout shell with Browse/Installed/Marketplaces tabs"
```

---

## Task 8: Build the Browse tab

**Files:**
- Create: `crates/kiro-control-center/src/lib/components/BrowseTab.svelte`
- Create: `crates/kiro-control-center/src/lib/components/SkillCard.svelte`

**Step 1: Build BrowseTab**

Layout:
- Left sidebar: marketplace list → click to expand plugins → click plugin to show skills
- Main area: grid of SkillCard components with checkbox selection
- Bottom bar: "Install N selected" button
- Top: search/filter input

Data flow:
1. On mount, call `commands.listMarketplaces()` to populate sidebar
2. When marketplace selected, call `commands.listPlugins(marketplace)`
3. When plugin selected, call `commands.listAvailableSkills(marketplace, plugin)`
4. Skills render as SkillCard components
5. Checkbox state tracked in a `Set<string>` ($state)
6. Install button calls `commands.installSkills()` with selected skill names

**Step 2: Build SkillCard**

A card component showing:
- Checkbox (bound to parent's selection set)
- Skill name (bold)
- Description
- "Installed" badge if `skill.installed` is true
- Disabled checkbox if already installed (unless force mode)

**Step 3: Verify with `npm run tauri dev`**

Browse tab should load marketplaces, let you drill into plugins/skills, select, and install.

**Step 4: Commit**

```bash
git commit -am "feat(kcc): implement Browse tab with skill selection and installation"
```

---

## Task 9: Build the Installed tab

**Files:**
- Create: `crates/kiro-control-center/src/lib/components/InstalledTab.svelte`

**Step 1: Build InstalledTab**

Layout:
- Table/list of installed skills sorted by name
- Columns: skill name, plugin@marketplace, version, installed date
- Checkbox selection for bulk remove
- "Remove N selected" button at bottom
- Search/filter input at top

Data flow:
1. On mount, call `commands.listInstalledSkills(projectPath)`
2. Render list
3. Remove button calls `commands.removeSkill()` for each selected skill, then refreshes

**Step 2: Verify with `npm run tauri dev`**

Installed tab should show skills and support removal.

**Step 3: Commit**

```bash
git commit -am "feat(kcc): implement Installed tab with skill removal"
```

---

## Task 10: Build the Marketplaces tab

**Files:**
- Create: `crates/kiro-control-center/src/lib/components/MarketplacesTab.svelte`

**Step 1: Build MarketplacesTab**

Layout:
- "Add marketplace" input + button at top
- List of registered marketplaces
- Each row: name, source type badge (github/git/local), plugin count
- Per-row: Update button, Remove button
- Status feedback (loading spinners, success/error toasts)

Data flow:
1. On mount, call `commands.listMarketplaces()`
2. Add: call `commands.addMarketplace(source)`, refresh list, show added plugins
3. Update: call `commands.updateMarketplace(name)`, show result
4. Remove: call `commands.removeMarketplace(name)`, refresh list

**Step 2: Verify with `npm run tauri dev`**

Marketplaces tab should support add/remove/update with feedback.

**Step 3: Commit**

```bash
git commit -am "feat(kcc): implement Marketplaces tab with add/remove/update"
```

---

## Task 11: Polish, error handling, and project path resolution

**Files:**
- Modify: `crates/kiro-control-center/src/routes/+page.svelte`
- Modify: various components

**Step 1: Resolve project path at startup**

On app mount, call `get_project_info()` with the current working directory. Display project path in a header/footer bar. Show a warning if `.kiro/` doesn't exist yet (skills can still be installed — the directory will be created).

**Step 2: Add error handling to all command calls**

Each `invoke` call should catch errors and display them inline or as a toast. Use the `error_type` field from `CommandError` to show appropriate messages:
- `not_found`: "Marketplace not registered" etc.
- `git_error`: "Failed to clone — check your network/SSH keys"
- `validation`: "Invalid name" etc.

**Step 3: Add loading states**

Show spinners/skeletons while commands are in flight. Disable buttons during operations.

**Step 4: Final visual polish**

- Consistent spacing, typography
- System theme (prefers-color-scheme for dark/light)
- Responsive layout within the window

**Step 5: Verify full flow**

```bash
cd /path/to/kiro/project && /path/to/kcc
```

Test: add marketplace → browse plugins → select skills → install → check Installed tab → remove skill → check Marketplaces tab.

**Step 6: Commit**

```bash
git commit -am "feat(kcc): polish error handling, loading states, and project path display"
```

---

## Task 12: Build release and update README

**Files:**
- Modify: `README.md`

**Step 1: Build release**

```bash
cd crates/kiro-control-center && npm run tauri build
```

The binary will be at `src-tauri/target/release/kcc`.

**Step 2: Update README with app section**

Add a section about the Tauri app:

```markdown
## Desktop App (Kiro Control Center)

For a visual interface, use the Tauri desktop app:

\`\`\`bash
cd /path/to/your/kiro-project
kcc
\`\`\`

Build from source:

\`\`\`bash
cd crates/kiro-control-center
npm install
npm run tauri build
cp src-tauri/target/release/kcc ~/.local/bin/
\`\`\`
```

**Step 3: Commit**

```bash
git commit -am "docs: add Kiro Control Center to README"
```

# Kiro Control Center

A desktop app and CLI for browsing, installing, and managing [Claude Code marketplace](https://docs.anthropic.com/en/docs/claude-code/skills#marketplace) skills in [Kiro](https://kiro.dev) projects.

Claude Code skills are distributed as `SKILL.md` files in marketplace repositories. Kiro uses a similar skill format but expects files at `.kiro/skills/<name>/SKILL.md`. Kiro Control Center bridges the gap: it clones marketplace repos, discovers plugins and skills, merges multi-file skills into single files, and installs them into your project.

## Desktop App (kcc)

The desktop app provides a tabbed GUI for the full skill management workflow.

### Installation

```bash
git clone https://github.com/dwalleck/kiro-control-center.git
cd kiro-control-center/crates/kiro-control-center
npm install
npx tauri build
cp ../../target/release/kcc ~/.local/bin/
```

### Usage

Launch from within a Kiro project directory:

```bash
cd /path/to/your/kiro-project
kcc
```

### Tabs

**Browse** -- Explore marketplaces and plugins in a sidebar, view skill cards with descriptions, select with checkboxes, and bulk install. Skills already installed are marked with a badge.

**Installed** -- View all installed skills with plugin, marketplace, version, and install date. Select and bulk remove.

**Marketplaces** -- Add new marketplace sources (GitHub `owner/repo`, git URL, or local path), update from remote, or remove. Shows plugin count and source type for each marketplace.

## CLI (kiro-market)

For scripting and automation, a full CLI is also available.

### Installation

```bash
cargo install --path crates/kiro-market
```

### Quick Start

```bash
# 1. Register a marketplace
kiro-market marketplace add microsoft/dotnet-agent-skills

# 2. Browse available skills (no query = list all)
kiro-market search

# 3. Install a plugin's skills into your Kiro project
cd /path/to/your/kiro-project
kiro-market install dotnet@dotnet-agent-skills

# 4. See what's installed
kiro-market list
```

### Commands

| Command | Description |
|---------|-------------|
| `marketplace add <source>` | Add a marketplace (GitHub shorthand, git URL, or local path) |
| `marketplace list` | List registered marketplaces |
| `marketplace update [name]` | Update marketplace clones from remote |
| `marketplace remove <name>` | Remove a registered marketplace |
| `search [query]` | Search skills by name/description (lists all if no query) |
| `install <plugin@marketplace>` | Install skills (`--skill <name>` for one, `--force` to overwrite) |
| `info <plugin@marketplace>` | Show plugin details and available skills |
| `list` | List installed skills in the current project |
| `remove <skill-name>` | Remove an installed skill |

The plugin reference format is `plugin@marketplace` (e.g., `dotnet@dotnet-agent-skills`).

## How It Works

1. **Marketplaces** are Git repositories containing a `.claude-plugin/marketplace.json` manifest that lists plugins.
2. **Plugins** are directories within a marketplace (or separate repos). Each plugin contains one or more skills, optionally described in a `plugin.json`.
3. **Skills** are directories containing a `SKILL.md` with YAML frontmatter (`name`, `description`) and Markdown content. Multi-file skills with companion `.md` references are merged into a single file.
4. **Installation** writes to `.kiro/skills/<name>/SKILL.md` and tracks metadata in `.kiro/installed-skills.json`.

### Project layout after installation

```
your-project/
  .kiro/
    installed-skills.json    # Tracks what's installed and where it came from
    skills/
      efcore/
        SKILL.md             # Merged skill content
      tunit/
        SKILL.md
```

## Requirements

- Rust 1.85.0+
- Node.js 20+ (for building the desktop app)
- Git (for cloning marketplace repositories)
- SSH agent or git credential helpers (for private repos)

## Development

```bash
cargo build                          # Build all crates
cargo test                           # All tests (147 across workspace)
cargo clippy --workspace -- -D warnings  # Lint
cargo test -p kiro-market-core       # Core library tests only
cargo test -p kiro-control-center    # Desktop app tests only

# Frontend development
cd crates/kiro-control-center
npm run dev                          # Vite dev server (port 1420)
npx tauri dev                        # Launch app in dev mode

# Regenerate TypeScript bindings after changing Tauri commands
cargo test -p kiro-control-center --lib -- --ignored generate_types
```

## Project Structure

```
crates/
  kiro-market-core/       # Shared library: types, parsing, git, cache, project state
  kiro-market/            # CLI binary (kiro-market)
  kiro-control-center/    # Tauri desktop app (kcc)
    src-tauri/            #   Rust backend: Tauri commands calling kiro-market-core
    src/                  #   Svelte 5 frontend with typed bindings via tauri-specta
```

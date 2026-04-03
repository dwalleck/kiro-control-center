# kiro-market

A CLI tool that installs [Claude Code marketplace](https://docs.anthropic.com/en/docs/claude-code/skills#marketplace) skills into [Kiro CLI](https://kiro.dev) projects.

Claude Code skills are distributed as `SKILL.md` files in marketplace repositories. Kiro uses a similar skill format but expects files at `.kiro/skills/<name>/SKILL.md`. This tool bridges the gap: it clones marketplace repos, discovers plugins and skills, merges multi-file skills into single files (Kiro doesn't support deferred companion loading), and installs them into your project.

## Requirements

- Rust 1.85.0+ (edition 2024)
- Git (for cloning marketplace repositories)
- SSH agent or git credential helpers configured (for private repos)

## Installation

```bash
# Clone and build
git clone https://github.com/dwalleck/kiro-marketplace-cli.git
cd kiro-marketplace-cli
cargo install --path crates/kiro-market

# Verify
kiro-market --version
```

## Quick Start

```bash
# 1. Register a marketplace (GitHub shorthand, git URL, or local path)
kiro-market marketplace add microsoft/dotnet-agent-skills

# 2. Search for skills
kiro-market search "entity framework"

# 3. Install a plugin's skills into your Kiro project
cd /path/to/your/kiro-project
kiro-market install dotnet@dotnet-agent-skills

# 4. See what's installed
kiro-market list
```

## Commands

### `marketplace` -- Manage marketplace sources

```bash
# Add from GitHub shorthand
kiro-market marketplace add microsoft/dotnet-agent-skills

# Add from a git URL
kiro-market marketplace add https://github.com/org/private-skills.git

# Add from a local directory (symlinked, always up to date)
kiro-market marketplace add ~/repos/my-skills

# List registered marketplaces
kiro-market marketplace list

# Update all marketplace clones from remote
kiro-market marketplace update

# Update a specific marketplace
kiro-market marketplace update dotnet-agent-skills

# Remove a marketplace
kiro-market marketplace remove dotnet-agent-skills
```

### `search` -- Find skills across marketplaces

Searches skill names and descriptions (case-insensitive) across all registered marketplaces:

```bash
kiro-market search rust
kiro-market search "code review"
```

### `install` -- Install skills into a Kiro project

Run this from within a Kiro project directory (one that has or will have a `.kiro/` folder):

```bash
# Install all skills from a plugin
kiro-market install dotnet@dotnet-agent-skills

# Install a specific skill by name
kiro-market install dotnet@dotnet-agent-skills --skill efcore

# Force overwrite if already installed
kiro-market install dotnet@dotnet-agent-skills --force
```

The plugin reference format is `plugin@marketplace`.

Installed skills are written to `.kiro/skills/<skill-name>/SKILL.md` and tracked in `.kiro/installed-skills.json`.

### `info` -- Show plugin details

```bash
kiro-market info dotnet@dotnet-agent-skills
```

### `list` -- Show installed skills

```bash
kiro-market list
```

### `remove` -- Remove an installed skill

```bash
kiro-market remove efcore
```

### `update` -- Update installed plugins

In-place update is not yet supported. To update a skill:

```bash
kiro-market remove <skill-name>
kiro-market install <plugin@marketplace> --force
```

## How It Works

1. **Marketplaces** are Git repositories containing a `.claude-plugin/marketplace.json` manifest that lists plugins.
2. **Plugins** are directories within a marketplace (or separate repos). Each plugin contains one or more skills, optionally described in a `plugin.json`.
3. **Skills** are directories containing a `SKILL.md` with YAML frontmatter (`name`, `description`) and Markdown content. Skills may reference companion `.md` files.
4. **Installation** clones the marketplace, resolves the plugin source, discovers skills, merges any companion files into the main `SKILL.md`, and writes the result to `.kiro/skills/`.

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

## Desktop App (Kiro Control Center)

For a visual interface with tabs for browsing, installing, and managing skills:

```bash
cd /path/to/your/kiro-project
kcc
```

The app provides three tabs:
- **Browse** -- Explore marketplaces, drill into plugins, select skills with checkboxes, and bulk install
- **Installed** -- View installed skills, select and remove
- **Marketplaces** -- Add, update, and remove marketplace sources

### Building from source

```bash
cd crates/kiro-control-center
npm install
npx tauri build
cp ../../target/release/kcc ~/.local/bin/
```

## Verbosity

```bash
kiro-market -v install ...     # Debug logging
kiro-market -vv install ...    # Trace logging
RUST_LOG=debug kiro-market ... # Or use RUST_LOG directly
```

## Development

```bash
cargo build                          # Build
cargo test                           # All tests
cargo test -p kiro-market-core       # Core library tests only
cargo clippy --workspace -- -D warnings  # Lint
```

## Project Structure

```
crates/
  kiro-market-core/       # Library: types, parsing, git, cache, project state
  kiro-market/            # Binary: CLI commands
  kiro-control-center/    # Tauri desktop app (kcc)
```

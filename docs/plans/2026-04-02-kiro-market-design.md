# kiro-market Design Document

**Date:** 2026-04-02
**Status:** Approved

## Problem

Kiro CLI has skills (SKILL.md files in `.kiro/skills/`) but no marketplace or plugin
distribution mechanism. Installing skills requires manually copying files from plugin
repositories into projects. Claude Code has a full plugin marketplace ecosystem with
`marketplace.json` catalogs — we want to leverage that ecosystem for Kiro.

## Solution

A standalone Rust CLI tool (`kiro-market`) that reads Claude Code `marketplace.json`
files, discovers plugins and their skills, and installs them into Kiro projects at
`.kiro/skills/`.

## Scope

- **In scope:** Skills (SKILL.md files) from Claude Code marketplaces
- **Out of scope:** Agents, MCP servers, hooks, LSP servers, steering files, npm sources
- **Best-effort:** Multi-file skill merging (companion .md files appended into single SKILL.md)

## Architecture: Git-Centric Fetcher

Marketplaces are git repositories. The tool clones/pulls marketplace repos, parses
`marketplace.json` to discover plugins, then clones plugin sources to extract skills.

### Input: Claude Code Marketplace Format (read-only)

```
marketplace-repo/
  .claude-plugin/
    marketplace.json    # { name, owner, plugins: [{ name, source, description }] }
  plugins/
    dotnet/
      plugin.json       # { name, version, description, skills: ["./skills/"] }
      skills/
        csharp-scripts/
          SKILL.md      # YAML frontmatter (name, description) + markdown body
          references/   # optional companion .md files
```

### Output: Kiro Project Skills

```
my-project/
  .kiro/
    skills/
      csharp-scripts/
        SKILL.md        # merged single file (companions appended)
    installed-skills.json
```

### Local State

**Cache** (`~/.local/share/kiro-market/`):
- `marketplaces/<name>/` — cloned marketplace repos
- `plugins/<marketplace>/<plugin>/` — cloned plugin repos (when source is external)
- `known_marketplaces.json` — list of registered marketplaces with their sources

**Per-project** (`.kiro/installed-skills.json`):
```json
{
  "skills": {
    "csharp-scripts": {
      "marketplace": "dotnet-agent-skills",
      "plugin": "dotnet",
      "version": "0.1.0",
      "installed_at": "2026-04-02T..."
    }
  }
}
```

## CLI Commands

| Command | Description |
|---------|-------------|
| `marketplace add <source>` | Register a marketplace (GitHub `owner/repo`, git URL, or local path) |
| `marketplace list` | List registered marketplaces |
| `marketplace update [name]` | Pull latest for one or all marketplaces |
| `marketplace remove <name>` | Unregister a marketplace |
| `search <query>` | Search skills across all marketplaces by name/description |
| `install <plugin>@<marketplace> [--skill <name>]` | Install all skills from a plugin, or a specific skill |
| `list` | List installed skills in the current project |
| `update [plugin@marketplace]` | Update installed skills from their sources |
| `remove <skill-name>` | Remove an installed skill from `.kiro/skills/` |
| `info <plugin>@<marketplace>` | Show plugin details and list its skills |

### Typical Workflow

```bash
# Register Microsoft's .NET marketplace
kiro-market marketplace add dotnet/skills

# Search across all marketplaces
kiro-market search "efcore"

# Install a plugin's skills
kiro-market install dotnet-data@dotnet-agent-skills

# Install a single skill
kiro-market install dotnet@dotnet-agent-skills --skill csharp-scripts

# List installed skills
kiro-market list

# Update when marketplace publishes new versions
kiro-market update
```

## Project Structure

```
kiro-marketplace-cli/
  Cargo.toml                    # workspace root
  crates/
    kiro-market/                # binary crate (CLI entry point)
      src/
        main.rs
        cli.rs                  # clap command definitions
        commands/
          marketplace.rs        # add, list, update, remove
          install.rs            # install, update skills
          search.rs             # search across marketplaces
          list.rs               # list installed skills
          remove.rs             # remove skills
    kiro-market-core/           # library crate (all logic)
      src/
        lib.rs
        marketplace.rs          # marketplace.json parsing & registry
        plugin.rs               # plugin.json parsing & skill discovery
        skill.rs                # SKILL.md parsing, merging, installation
        git.rs                  # git clone, pull, sparse checkout
        cache.rs                # ~/.local/share/kiro-market/ management
        project.rs              # .kiro/skills/ and installed-skills.json
        error.rs                # thiserror error types
```

## Key Types

### Marketplace (marketplace.rs)

```rust
struct Marketplace {
    name: String,
    owner: Owner,
    plugins: Vec<PluginEntry>,
}

struct PluginEntry {
    name: String,
    source: PluginSource,
    description: Option<String>,
    version: Option<String>,
}

enum PluginSource {
    RelativePath(String),
    GitHub { repo: String, r#ref: Option<String>, sha: Option<String> },
    GitUrl { url: String, r#ref: Option<String>, sha: Option<String> },
    GitSubdir { url: String, path: String, r#ref: Option<String>, sha: Option<String> },
}
```

### Plugin (plugin.rs)

```rust
struct PluginManifest {
    name: String,
    version: Option<String>,
    description: Option<String>,
    skills: Vec<String>,
}
```

### Skill (skill.rs)

```rust
struct Skill {
    name: String,
    description: String,
    content: String,
    companion_files: Vec<CompanionFile>,
}

struct CompanionFile {
    relative_path: String,
    content: String,
}
```

## Error Types

```rust
#[derive(Debug, thiserror::Error)]
enum MarketplaceError {
    #[error("marketplace '{name}' not found")]
    NotFound { name: String },
    #[error("marketplace '{name}' already registered")]
    AlreadyRegistered { name: String },
    #[error("invalid marketplace.json: {reason}")]
    InvalidManifest { reason: String },
}

#[derive(Debug, thiserror::Error)]
enum PluginError {
    #[error("plugin '{plugin}' not found in marketplace '{marketplace}'")]
    NotFound { plugin: String, marketplace: String },
    #[error("invalid plugin.json: {reason}")]
    InvalidManifest { reason: String },
}

#[derive(Debug, thiserror::Error)]
enum SkillError {
    #[error("skill '{name}' already installed (use --force to overwrite)")]
    AlreadyInstalled { name: String },
    #[error("failed to merge companion files for skill '{name}': {reason}")]
    MergeFailed { name: String, reason: String },
}

#[derive(Debug, thiserror::Error)]
enum GitError {
    #[error("git clone failed for {url}: {source}")]
    CloneFailed { url: String, source: git2::Error },
    #[error("git pull failed for {path}: {source}")]
    PullFailed { path: String, source: git2::Error },
}
```

## Skill Merge Flow

For multi-file Claude Code skills (SKILL.md + companion .md files), since Kiro
only supports a single SKILL.md:

1. Read SKILL.md
2. Parse YAML frontmatter (name, description)
3. Scan markdown body for relative .md links using pulldown-cmark
4. For each referenced companion file:
   a. Read the companion file content
   b. Append to end of SKILL.md with separator:
      `\n\n---\n<!-- Merged from {relative_path} -->\n`
   c. Rewrite the original link to point to a heading anchor
5. Write merged SKILL.md to `.kiro/skills/<name>/SKILL.md`
6. Record in `installed-skills.json`

## Crate Dependencies

| Concern | Crate | Notes |
|---------|-------|-------|
| CLI framework | `clap` (derive) | Subcommand support, shell completions |
| Git operations | `git2` | libgit2 bindings for clone/pull |
| JSON | `serde` + `serde_json` | Marketplace and plugin manifest parsing |
| YAML frontmatter | `serde_yaml` | SKILL.md frontmatter parsing |
| Markdown parsing | `pulldown-cmark` | Find relative .md links for merging |
| File paths | `dirs` | XDG-compliant path resolution |
| Console output | `colored` | Colored terminal output |
| Error handling | `thiserror` (lib) + `anyhow` (bin) | Typed errors in core, ergonomic propagation in CLI |
| Logging | `tracing` + `tracing-subscriber` | Structured logging |
| Testing | `rstest` + `tempfile` | Test fixtures and temp directories |

## Supported Plugin Sources

| Source | Format | Implementation |
|--------|--------|----------------|
| Relative path | `"./plugins/dotnet"` | Resolve within cloned marketplace repo |
| GitHub | `{ "source": "github", "repo": "owner/repo" }` | Clone via git2 |
| Git URL | `{ "source": "url", "url": "https://..." }` | Clone via git2 |
| Git subdirectory | `{ "source": "git-subdir", "url": "...", "path": "..." }` | Sparse clone via git2 |
| npm | Not supported | Out of scope — skills-only focus |

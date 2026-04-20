# Interfaces

## CLI Interface

The CLI (`kiro-market`) exposes commands via clap derive:

```
kiro-market [OPTIONS] <COMMAND>

Commands:
  marketplace   Manage marketplace sources (add, list, update, remove)
  search        Search plugins across all registered marketplaces
  install       Install a plugin (or a specific skill from a plugin)
  list          List all installed skills in the current project
  update        Update installed plugins (or a specific one)
  remove        Remove an installed skill from the current project
  info          Show detailed information about a plugin
  cache         Inspect or clean up the on-disk cache

Options:
  -v, --verbose   Increase verbosity (-v, -vv, -vvv)
```

### Plugin Reference Format

`plugin@marketplace` — e.g., `dotnet@dotnet-agent-skills`

Parsed by `parse_plugin_ref()` which splits on the first `@`.

## Tauri IPC Interface

The desktop app exposes Rust functions to the frontend via `tauri-specta`. TypeScript bindings are auto-generated at `src/lib/bindings.ts`.

### Browse Commands

```typescript
listMarketplaces(): Promise<MarketplaceInfo[]>
listPlugins(marketplace: string): Promise<PluginInfo[]>
listAvailableSkills(marketplace: string, plugin: string): Promise<SkillInfo[]>
listAllSkillsForMarketplace(marketplace: string): Promise<BulkSkillsResult>
installSkills(marketplace: string, plugin: string, skills: string[], force: boolean): Promise<InstallResult>
getProjectInfo(): Promise<ProjectInfo>
```

### Installed Commands

```typescript
listInstalledSkills(): Promise<InstalledSkillInfo[]>
removeSkill(name: string): Promise<void>
```

### Marketplace Commands

```typescript
addMarketplace(source: string): Promise<MarketplaceInfo>
removeMarketplace(name: string): Promise<void>
updateMarketplace(name: string): Promise<void>
```

### Settings Commands

```typescript
getSettings(): Promise<Settings>
saveScanRoots(roots: string[]): Promise<void>
discoverProjects(): Promise<DiscoveredProject[]>
setActiveProject(path: string): Promise<ProjectInfo>
getKiroSettings(): Promise<SettingEntry[]>
setKiroSetting(key: string, value: JsonValue): Promise<SettingEntry>
resetKiroSetting(key: string): Promise<SettingEntry>
```

## Core Library Public API

### MarketplaceService

```rust
impl MarketplaceService {
    pub fn new(cache: CacheDir, git: impl GitBackend) -> Self;
    pub fn add(source: &str, options: MarketplaceAddOptions) -> Result<MarketplaceAddResult>;
    pub fn remove(name: &str) -> Result<()>;
    pub fn update(name: Option<&str>) -> Result<UpdateResult>;
    pub fn list() -> Result<Vec<KnownMarketplace>>;
    pub fn marketplace_path(name: &str) -> PathBuf;
    pub fn list_plugin_entries(marketplace: &str) -> Result<Vec<PluginEntry>>;
    pub fn install_skills(project: &KiroProject, ...) -> Result<InstallSkillsResult>;
    pub fn install_plugin_agents(project: &KiroProject, ...) -> Result<InstallAgentsResult>;
}
```

### KiroProject

```rust
impl KiroProject {
    pub fn new(root: &Path) -> Self;
    pub fn install_skill_from_dir(name: &str, source: &Path) -> Result<()>;
    pub fn install_skill_from_dir_force(name: &str, source: &Path) -> Result<()>;
    pub fn remove_skill(name: &str) -> Result<()>;
    pub fn load_installed() -> Result<InstalledSkills>;
    pub fn install_agent(def: &AgentDefinition, meta: InstalledAgentMeta) -> Result<()>;
    pub fn install_agent_force(def: &AgentDefinition, meta: InstalledAgentMeta) -> Result<()>;
    pub fn load_installed_agents() -> Result<InstalledAgents>;
}
```

### GitBackend Trait

```rust
pub trait GitBackend {
    fn clone_repo(url: &str, dest: &Path, options: &CloneOptions) -> Result<()>;
    fn pull_repo(path: &Path) -> Result<()>;
    fn verify_sha(path: &Path, expected: &str) -> Result<()>;
}
```

## File System Interfaces

### Marketplace Manifest

Located at `.claude-plugin/marketplace.json` within a marketplace repo:

```json
{
  "name": "marketplace-name",
  "description": "Optional description",
  "plugins": [
    {
      "name": "plugin-name",
      "description": "Optional",
      "source": "relative/path"
    }
  ]
}
```

### Plugin Manifest

Located at `plugin.json` within a plugin directory:

```json
{
  "name": "plugin-name",
  "description": "Optional",
  "skills": ["./skills/"],
  "agents": ["./agents/"]
}
```

### Skill File

Located at `SKILL.md` with YAML frontmatter:

```markdown
---
name: skill-name
description: What this skill does
invocable: true
---

Skill content in markdown...
```

### Agent File (Claude Format)

```markdown
---
name: agent-name
description: Optional
model: optional-model
tools:
  - Read
  - Edit
---

Agent prompt content...
```

### Agent File (Copilot Format)

`*.agent.md` with YAML frontmatter including `tools` list and optional `mcpServers` configuration.

### Installed Skills Tracking

`.kiro/installed-skills.json`:

```json
{
  "skills": {
    "skill-name": {
      "marketplace": "marketplace-name",
      "plugin": "plugin-name",
      "installed_at": "2024-01-01T00:00:00Z"
    }
  }
}
```

### Installed Agents Tracking

`.kiro/installed-agents.json`:

```json
{
  "agents": {
    "agent-name": {
      "marketplace": "marketplace-name",
      "plugin": "plugin-name",
      "dialect": "claude",
      "installed_at": "2024-01-01T00:00:00Z"
    }
  }
}
```

### Cache Structure

```
~/.cache/kiro-market/
├── known_marketplaces.json
├── marketplaces/
│   └── <name>/              # Cloned/linked marketplace repos
├── plugins/
│   └── <marketplace>/<plugin>/  # Resolved plugin directories
└── registries/
    └── <marketplace>.json   # Persisted plugin registry
```

### Desktop App Settings

`~/.config/kiro-control-center/settings.json`:

```json
{
  "scan_roots": ["/path/to/projects"],
  "last_project": "/path/to/last/project"
}
```

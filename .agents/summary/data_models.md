# Data Models

## Core Domain Types

### Marketplace Layer

```mermaid
classDiagram
    class Marketplace {
        +String name
        +Option~String~ description
        +Vec~PluginEntry~ plugins
        +from_json(value) Result~Marketplace~
    }

    class PluginEntry {
        +String name
        +Option~String~ description
        +PluginSource source
    }

    class PluginSource {
        <<enum>>
        RelativePath(String)
        Structured(StructuredSource)
    }

    class StructuredSource {
        +String url
        +Option~String~ subdir
        +Option~String~ git_ref
        +Option~String~ sha
    }

    Marketplace --> PluginEntry
    PluginEntry --> PluginSource
    PluginSource --> StructuredSource
```

### Cache Layer

```mermaid
classDiagram
    class CacheDir {
        +PathBuf root
        +default_location() PathBuf
        +with_root(path) Self
        +marketplace_path(name) PathBuf
        +plugin_path(marketplace, plugin) PathBuf
        +ensure_dirs() Result
        +prune_orphans(mode) Result~PruneReport~
    }

    class KnownMarketplace {
        +String name
        +MarketplaceSource source
        +GitProtocol protocol
    }

    class MarketplaceSource {
        <<enum>>
        GitUrl(String)
        LocalPath(PathBuf)
    }

    class PruneReport {
        +Vec~PathBuf~ removed
        +Vec~PruneFailure~ failed
    }

    class PruneMode {
        <<enum>>
        DryRun
        Execute
    }

    CacheDir --> KnownMarketplace
    KnownMarketplace --> MarketplaceSource
    CacheDir --> PruneReport
```

### Project Layer

```mermaid
classDiagram
    class KiroProject {
        +PathBuf root
        +new(root) Self
        +kiro_dir() PathBuf
        +skills_dir() PathBuf
        +agents_dir() PathBuf
    }

    class InstalledSkills {
        +HashMap~String, InstalledSkillMeta~ skills
    }

    class InstalledSkillMeta {
        +String marketplace
        +String plugin
        +DateTime installed_at
    }

    class InstalledAgents {
        +HashMap~String, InstalledAgentMeta~ agents
    }

    class InstalledAgentMeta {
        +String marketplace
        +String plugin
        +AgentDialect dialect
        +DateTime installed_at
    }

    KiroProject --> InstalledSkills
    KiroProject --> InstalledAgents
    InstalledSkills --> InstalledSkillMeta
    InstalledAgents --> InstalledAgentMeta
```

### Plugin & Skill Layer

```mermaid
classDiagram
    class PluginManifest {
        +String name
        +Option~String~ description
        +Vec~String~ skills
        +Vec~String~ agents
        +from_json(value) Result
    }

    class DiscoveredPlugin {
        +String name
        +PathBuf path
        +Option~PluginManifest~ manifest
        +relative_path() String
    }

    class SkillFrontmatter {
        +String name
        +Option~String~ description
        +Option~bool~ invocable
        +parse_frontmatter(content) Result
    }

    DiscoveredPlugin --> PluginManifest
```

### Agent Layer

```mermaid
classDiagram
    class AgentDefinition {
        +String name
        +Option~String~ description
        +Option~String~ model
        +AgentDialect dialect
        +Vec~MappedTool~ tools
        +Vec~UnmappedTool~ unmapped_tools
        +Vec~McpServerConfig~ mcp_servers
        +String body
    }

    class AgentDialect {
        <<enum>>
        Claude
        Copilot
    }

    class McpServerConfig {
        +String name
        +McpTransport transport
        +is_stdio() bool
    }

    class MappedTool {
        +String source_name
        +String kiro_name
    }

    class UnmappedTool {
        +String name
        +UnmappedReason reason
    }

    AgentDefinition --> AgentDialect
    AgentDefinition --> McpServerConfig
    AgentDefinition --> MappedTool
    AgentDefinition --> UnmappedTool
```

### Service Result Types

```mermaid
classDiagram
    class MarketplaceAddResult {
        +String name
        +Vec~PluginBasicInfo~ plugins
        +MarketplaceStorage storage
    }

    class MarketplaceStorage {
        <<enum>>
        Cloned
        Linked
        Copied
    }

    class InstallSkillsResult {
        +Vec~String~ installed
        +Vec~String~ skipped
        +Vec~FailedSkill~ failed
    }

    class InstallAgentsResult {
        +Vec~String~ installed
        +Vec~String~ skipped
        +Vec~FailedAgent~ failed
        +Vec~InstallWarning~ warnings
    }

    class UpdateResult {
        +Vec~String~ updated
        +Vec~FailedUpdate~ failed
        +Vec~String~ skipped
    }

    MarketplaceAddResult --> MarketplaceStorage
```

### Error Types

```mermaid
classDiagram
    class Error {
        <<enum>>
        Marketplace(MarketplaceError)
        Plugin(PluginError)
        Skill(SkillError)
        Agent(AgentError)
        Git(GitError)
        Validation(ValidationError)
        Io(io::Error)
        Json(serde_json::Error)
    }

    class GitError {
        <<enum>>
        CloneFailed
        PullFailed
        OpenFailed
        ShaMismatch
        NotFound
        CommandFailed
    }

    class ValidationError {
        +String message
    }

    Error --> GitError
    Error --> ValidationError
```

### Settings Types

```mermaid
classDiagram
    class SettingDef {
        +String key
        +SettingCategory category
        +SettingType value_type
        +SettingValue default
        +String description
    }

    class SettingCategory {
        <<enum>>
        General
        Security
        Display
    }

    class SettingType {
        <<enum>>
        Bool
        String
        Number
    }

    class SettingEntry {
        +String key
        +String category
        +String category_label
        +SettingType value_type
        +SettingValue value
        +SettingValue default
        +String description
    }

    SettingDef --> SettingCategory
    SettingDef --> SettingType
```

## Tauri Frontend Types

### TypeScript Interfaces (auto-generated via specta)

Key types exposed to the frontend:

- `MarketplaceInfo` — marketplace name, source type, plugin count
- `PluginInfo` — plugin name, description, source type
- `SkillInfo` — skill name, description, installed status
- `InstalledSkillInfo` — name, marketplace, plugin, version, install date
- `ProjectInfo` — project path, installed skill/agent counts
- `Settings` — scan roots, last project
- `DiscoveredProject` — path, has `.kiro/` directory
- `SettingEntry` — key, category, type, value, default, description
- `CommandError` — error type enum + message string
- `SourceType` — `github` | `git_url` | `local_path` | `git_subdir`

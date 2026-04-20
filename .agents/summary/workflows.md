# Workflows

## Marketplace Registration

```mermaid
sequenceDiagram
    participant User
    participant Frontend as CLI/GUI
    participant Service as MarketplaceService
    participant Cache as CacheDir
    participant Git as GitBackend

    User->>Frontend: add source
    Frontend->>Service: add(source, options)
    Service->>Cache: detect(source)
    alt GitHub shorthand or Git URL
        Service->>Git: clone_repo(url, staging_dir)
        Git-->>Service: clone complete
        Service->>Cache: register_known_marketplace(name, source)
    else Local path
        Service->>Cache: resolve_local_path_restricted(path)
        Service->>Cache: create_local_link(path, marketplace_dir)
        Note over Cache: symlink (Unix) or junction (Windows)
    end
    Service->>Service: discover plugins (manifest + scan)
    Service->>Cache: write_plugin_registry(entries)
    Service-->>Frontend: MarketplaceAddResult
    Frontend-->>User: show plugins found
```

## Skill Installation

```mermaid
sequenceDiagram
    participant User
    participant Frontend as CLI/GUI
    participant Service as MarketplaceService
    participant Project as KiroProject
    participant FS as File System

    User->>Frontend: install plugin@marketplace
    Frontend->>Service: install_skills(project, marketplace, plugin, filter)
    Service->>Service: resolve plugin directory
    Service->>Service: discover skills in plugin
    loop Each skill
        Service->>Project: install_skill_from_dir(name, source_dir)
        Project->>Project: validate name (path traversal check)
        Project->>Project: acquire file lock
        Project->>Project: check for duplicates
        Project->>FS: create staging dir
        Project->>FS: copy_dir_recursive(source → staging)
        Note over FS: Skips symlinks and hardlinks
        Project->>FS: rename staging → .kiro/skills/<name>/
        Project->>Project: update installed-skills.json
        Project->>Project: release file lock
    end
    Service-->>Frontend: InstallSkillsResult
    Frontend-->>User: show outcome
```

## Agent Installation

```mermaid
sequenceDiagram
    participant Service as MarketplaceService
    participant Project as KiroProject
    participant Parser as Agent Parser
    participant Emitter as Agent Emitter

    Service->>Service: discover agent files in plugin
    loop Each agent file
        Service->>Parser: parse_agent_file(path)
        Parser->>Parser: detect_dialect (Claude vs Copilot)
        Parser-->>Service: AgentDefinition
        Service->>Service: map tools (Claude/Copilot → Kiro)
        alt Has MCP servers AND --accept-mcp not set
            Service->>Service: skip with warning
        else
            Service->>Emitter: build_kiro_json(definition)
            Emitter-->>Service: JSON config + prompt content
            Service->>Project: install_agent(definition, meta)
            Project->>Project: write .kiro/agents/<name>.json
            Project->>Project: write .kiro/agents/<name>.prompt.md
            Project->>Project: update installed-agents.json
        end
    end
```

## Marketplace Update

```mermaid
sequenceDiagram
    participant User
    participant Service as MarketplaceService
    participant Cache as CacheDir
    participant Git as GitBackend

    User->>Service: update(name?)
    Service->>Cache: load_known_marketplaces()
    loop Each marketplace (or specific one)
        alt Source is GitUrl
            Service->>Git: pull_repo(marketplace_path)
            alt Pull succeeds
                Service->>Service: regenerate_plugin_registry()
                Service->>Service: add to updated list
            else Pull fails
                Service->>Service: add to failed list
            end
        else Source is LocalPath
            Service->>Service: add to skipped list
            Note over Service: Local paths are live-linked
        end
    end
    Service-->>User: UpdateResult
```

## Cache Pruning

```mermaid
flowchart TD
    A[Start prune_orphans] --> B[Load known marketplaces]
    B --> C[Scan marketplaces/ directory]
    C --> D{Dir registered?}
    D -->|No| E[Remove orphaned dir]
    D -->|Yes| F[Keep]
    E --> G[Scan registries/ directory]
    F --> G
    G --> H{Registry has matching marketplace?}
    H -->|No| I[Remove orphaned registry JSON]
    H -->|Yes| J[Keep]
    I --> K[Scan for _pending_* dirs]
    J --> K
    K --> L[Remove stale staging dirs]
    L --> M[Scan plugin lock files]
    M --> N{Plugin dir exists?}
    N -->|No| O[Remove stale lock file]
    N -->|Yes| P[Keep lock]
    O --> Q[Return PruneReport]
    P --> Q
```

## Desktop App Initialization

```mermaid
sequenceDiagram
    participant App as Tauri App
    participant Store as project.svelte.ts
    participant Backend as Rust Commands
    participant FS as File System

    App->>Store: initialize()
    Store->>Backend: getSettings()
    Backend->>FS: load settings.json
    FS-->>Backend: settings data
    Backend-->>Store: Settings
    Store->>Backend: discoverProjects()
    Backend->>FS: scan_for_projects(scan_roots)
    Note over FS: Walk dirs looking for .kiro/
    FS-->>Backend: discovered projects
    Backend-->>Store: DiscoveredProject[]
    alt last_project exists in discovered
        Store->>Backend: setActiveProject(path)
        Backend->>FS: verify .kiro/ exists
        Backend-->>Store: ProjectInfo
    end
    Store->>App: loading = false
```

## Git Clone (Dual Backend)

```mermaid
flowchart TD
    A[clone_repo called] --> B[Try gix backend]
    B --> C{gix success?}
    C -->|Yes| D[Return Ok]
    C -->|No| E[Try CLI backend]
    E --> F{CLI success?}
    F -->|Yes| D
    F -->|No| G[Combine both errors]
    G --> H[translate_git_error]
    H --> I[Return combined error with auth hints]

    D --> J{checkout_ref needed?}
    J -->|Yes| K[Checkout specified ref]
    J -->|No| L[Done]
    K --> L
```

## CI Pipeline

```mermaid
flowchart LR
    subgraph "Parallel Jobs"
        FMT[Format Check]
        LINT[Clippy Lint]
        TEST[Test 3 OS]
        FE[Frontend Check]
        CLI_BUILD[Build CLI 3 OS]
        DENY[cargo-deny]
        CURL[assert-curl-tls]
        COV[Coverage]
        COMMIT[Commitlint]
    end

    FE --> TAURI[Build Tauri 3 OS]

    subgraph "Gate"
        ALL[ci-success]
    end

    FMT --> ALL
    LINT --> ALL
    TEST --> ALL
    FE --> ALL
    CLI_BUILD --> ALL
    TAURI --> ALL
    DENY --> ALL
    CURL --> ALL
    COV --> ALL
    COMMIT --> ALL
```

## Development Workflow (Claude Hooks)

```mermaid
flowchart TD
    A[Developer edits .rs file] --> B{PreToolUse: Write/Edit}
    B --> C{Is Cargo.lock?}
    C -->|Yes| D[BLOCK edit]
    C -->|No| E[Allow edit]
    E --> F{PostToolUse: Write/Edit}
    F --> G[rustfmt on file]
    G --> H[clippy on package]
    H --> I{Clippy issues?}
    I -->|Yes| J[Report to Claude]
    I -->|No| K[Done]
```

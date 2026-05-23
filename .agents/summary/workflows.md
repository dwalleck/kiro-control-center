# Workflows

<!-- tags: workflows, processes, sequences -->

## 1. Add Marketplace

```mermaid
sequenceDiagram
    participant User
    participant CLI/UI
    participant Service as MarketplaceService
    participant Cache as CacheDir
    participant Git

    User->>CLI/UI: marketplace add <source>
    CLI/UI->>Service: add(source, opts)
    Service->>Cache: detect source type (GitHub/git URL/local)
    alt local path
        Service->>Cache: create_local_link (symlink/junction/copy)
    else remote
        Service->>Git: clone_repo(url, dest, opts)
        Git-->>Service: cloned
    end
    Service->>Cache: register_known_marketplace_unlocked
    Service->>Service: build_registry_entries (scan + manifest merge)
    Service->>Cache: write_plugin_registry
    Service-->>CLI/UI: MarketplaceAddResult { plugins }
```

**Key details:**
- Source detection in `cache.rs::detect()` — handles GitHub shorthand (`owner/repo`), HTTPS/SSH URLs, local paths, file URLs
- `http://` rejected unless `allow_insecure_http: true`
- Duplicate marketplace names return an error
- Plugin registry is written atomically

---

## 2. Plugin Install (Full)

```mermaid
sequenceDiagram
    participant User
    participant CLI/UI
    participant Service as MarketplaceService
    participant Browse as service/browse.rs
    participant Project as KiroProject
    participant FS

    User->>CLI/UI: install plugin@marketplace
    CLI/UI->>Service: install_plugin(ctx)
    Service->>Browse: resolve_plugin_install_context
    Browse->>FS: read plugin.json (if present)
    Browse-->>Service: PluginInstallContext { skill_dirs, agent_paths, steering_paths, format, version }
    Service->>Service: install_skills(...)
    loop each skill dir
        Service->>FS: read SKILL.md frontmatter
        Service->>Project: install_skill_from_dir (hash check → stage → promote)
    end
    Service->>Service: install_plugin_agents(...)
    loop each agent file
        Service->>FS: parse agent (Claude/Copilot/Native dispatch)
        Service->>Project: install_agent (stage → promote → write tracking)
    end
    Service->>Service: install_plugin_steering(...)
    loop each steering file
        Service->>Project: install_steering_file (hash check → stage → promote)
    end
    Service-->>CLI/UI: InstallPluginResult { skills, agents, steering }
```

**Key details:**
- `InstallMode::Force` overwrites existing; default mode rejects duplicates
- BLAKE3 hashes compared: if source hash matches installed hash, install is skipped (idempotent)
- MCP agents blocked unless `accept_mcp: true`
- `DirCleanupGuard` ensures staging dirs are removed on failure
- Tracking files updated atomically under `fs4` lock

---

## 3. Skill Install (Detail)

```mermaid
flowchart TD
    A[install_skill_from_dir] --> B{Force mode?}
    B -->|No| C{Already installed?}
    C -->|Yes, same hash| D[Skip — idempotent]
    C -->|Yes, different hash| E[Error: ContentChangedRequiresForce]
    C -->|No| F[Stage files to tempdir]
    B -->|Yes| F
    F --> G[copy_dir_recursive — skips symlinks/hardlinks]
    G --> H[Compute BLAKE3 hashes]
    H --> I[Write tracking entry under fs4 lock]
    I --> J[Move staged files to .kiro/skills/name/]
    J --> K[defuse DirCleanupGuard]
```

---

## 4. Agent Conversion (Translated Format)

```mermaid
flowchart TD
    A[Agent file discovered] --> B{Detect dialect}
    B -->|.agent.md suffix| C[parse_copilot_agent]
    B -->|plain .md| D[parse_claude_agent]
    B -->|.json| E[parse_native_kiro_agent_file]
    C --> F[Extract name, tools, MCP servers]
    D --> F
    E --> G[NativeAgentBundle — write verbatim]
    F --> H{Has MCP servers?}
    H -->|Yes, accept_mcp=false| I[Warning — skip agent]
    H -->|Yes, accept_mcp=true| J[map tools to Kiro format]
    H -->|No| J
    J --> K[build_kiro_json — emit JSON + prompt URI]
    K --> L[Write .kiro/agents/name.json]
    K --> M[Write .kiro/agents/prompts/name.md]
    L --> N[Update installed-agents.json under lock]
    M --> N
```

**Tool mapping:**
- Claude tools → `allowedTools` (Kiro native tool names)
- Copilot MCP refs → `tools` (MCP server references)
- Copilot bare names → dropped with `UnmappedReason`
- Deduplication applied (e.g., `Edit` + `Write` → `Write` only)

---

## 5. Update Detection

```mermaid
sequenceDiagram
    participant UI
    participant Service as MarketplaceService
    participant Project as KiroProject
    participant FS

    UI->>Service: detect_plugin_updates(project)
    Service->>Project: installed_plugins() — load all tracking files
    loop each installed plugin
        Service->>FS: load plugin.json (current version)
        Service->>Service: scan_plugin_for_content_drift
        alt version bumped
            Service-->>UI: PluginUpdateInfo { signal: VersionBumped }
        else content hash changed (no version bump)
            Service-->>UI: PluginUpdateInfo { signal: ContentDrift }
        else up to date
            Note over Service: skip
        end
    end
    Service-->>UI: DetectUpdatesResult { updates, failures }
```

**Key details:**
- Version comparison: string equality (not semver)
- Content drift: re-hashes source files and compares to stored `source_hash`
- Structured sources (git subdir, git URL) return a failure (cannot check locally)
- Partial load warnings (corrupt tracking entries) are surfaced alongside results

---

## 6. Browse / Plugin Catalog

```mermaid
sequenceDiagram
    participant UI
    participant Cmd as commands/browse.rs
    participant Service as MarketplaceService
    participant Browse as service/browse.rs
    participant Project as KiroProject

    UI->>Cmd: list_plugin_catalog_for_marketplace(marketplace, project_path)
    Cmd->>Service: list_plugin_catalog(marketplace, project)
    Service->>Browse: list_plugin_entries (from registry)
    loop each plugin
        Browse->>Browse: count_skills_for_plugin
        Browse->>Browse: list_skills_with_manifest
        Browse->>Browse: list_agents_with_manifest
        Browse->>Browse: list_steering_with_manifest
        Browse->>Project: check installed tracking (mark installed=true)
        Browse-->>Service: PluginCatalogEntry
    end
    Service-->>Cmd: PluginCatalogView
    Cmd-->>UI: PluginCatalogResponseView (with SourceType enrichment)
```

**Key details:**
- `SkippedItem` captures plugins/skills that could not be enumerated (with reason)
- `SkillCount` can be `Known(n)`, `Remote` (structured source), or `ManifestFailed`
- Installed flags are set by cross-referencing tracking files

---

## 7. Marketplace Update

```mermaid
sequenceDiagram
    participant User
    participant Service as MarketplaceService
    participant Cache as CacheDir
    participant Git

    User->>Service: update(name=None)
    Service->>Cache: load_known_marketplaces
    loop each marketplace (or named one)
        Service->>Git: pull_repo(marketplace_path)
        alt pull succeeds
            Service->>Service: regenerate_plugin_registry
            Service->>Cache: write_plugin_registry
        else pull fails
            Service-->>User: FailedUpdate { name, error }
        end
    end
    Service-->>User: UpdateResult { updated, failed }
```

---

## 8. Remove Plugin

```mermaid
flowchart TD
    A[remove_plugin marketplace+plugin] --> B[Load all tracking files under lock]
    B --> C[Remove matching skills from .kiro/skills/]
    C --> D[Remove matching agents from .kiro/agents/]
    D --> E[Remove matching steering from .kiro/steering/]
    E --> F[Remove native companions entry]
    F --> G[Write updated tracking files]
    G --> H[RemovePluginResult — counts removed + failures]
```

Partial failures are collected (not fatal): files that fail to unlink are recorded in `failures` but the rest of the removal proceeds.

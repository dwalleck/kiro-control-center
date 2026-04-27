# Workflows

## Add Marketplace

```mermaid
sequenceDiagram
    participant User
    participant Frontend as CLI/Desktop
    participant Service as MarketplaceService
    participant Cache as CacheDir
    participant Git as GitBackend

    User->>Frontend: add "owner/repo"
    Frontend->>Service: add(opts)
    Service->>Cache: detect(source)
    Cache-->>Service: MarketplaceSource::GitHubShorthand
    Service->>Service: Check InsecureHttpPolicy
    Service->>Cache: add_known_marketplace(name, source)
    Service->>Git: clone_repo(url, dest, options)
    Git-->>Service: sha
    Service->>Service: Try read marketplace.json
    alt manifest exists
        Service->>Service: Parse plugin entries from manifest
    else no manifest
        Service->>Service: Scan for plugin.json at depth 1-3
    end
    Service->>Cache: write_plugin_registry(entries)
    Service-->>Frontend: MarketplaceAddResult
    Frontend-->>User: "Added with N plugins"
```

**Key behaviors:**
- Local paths use symlinks/junctions instead of cloning
- Duplicate marketplace names are rejected
- `http://` URLs rejected unless `InsecureHttpPolicy::Allow`
- Failed clones trigger `DirCleanupGuard` to remove partial state
- Plugin registry is persisted for fast subsequent lookups

---

## Install Skills

```mermaid
sequenceDiagram
    participant User
    participant Frontend as CLI/Desktop
    participant Service as MarketplaceService
    participant Project as KiroProject
    participant FS as File System

    User->>Frontend: install "plugin@marketplace"
    Frontend->>Service: resolve_plugin_install_context()
    Service->>Service: Find marketplace in cache
    Service->>Service: Resolve plugin directory
    Service->>Service: Read plugin.json (or use defaults)
    Service-->>Frontend: PluginInstallContext

    Frontend->>Service: install_skills(context, project, filter, mode)
    loop Each skill directory
        Service->>Service: Parse SKILL.md frontmatter
        Service->>Service: Validate name
        Service->>Project: install_skill_from_dir(name, source)
        Project->>FS: Acquire file lock
        Project->>FS: Check for existing skill
        alt exists and not force
            Project-->>Service: Error (duplicate)
        else
            Project->>FS: Copy skill dir to .kiro/skills/{name}/
            Project->>FS: Update installed-skills.json
            Project-->>Service: OK
        end
    end
    Service-->>Frontend: InstallSkillsResult
```

**Key behaviors:**
- Skills are copied (not linked) to `.kiro/skills/{name}/SKILL.md`
- Multi-file skills with companion `.md` references are merged into single file
- File lock serializes concurrent installs of the same skill name
- `--force` overwrites existing; removes stale files from prior version
- Source and installed content hashes (BLAKE3) tracked for change detection

---

## Install Agents

```mermaid
sequenceDiagram
    participant Service as MarketplaceService
    participant Agent as Agent Module
    participant Project as KiroProject
    participant FS as File System

    Service->>Agent: discover_agents_in_dirs(scan_paths)
    Agent-->>Service: Vec<DiscoveredFile>

    alt format == "kiro-cli" (native)
        loop Each .json agent file
            Service->>Agent: parse_native_kiro_agent_file()
            Service->>Service: Check for MCP servers
            alt has MCP and !accept_mcp
                Service->>Service: Add to warnings (skipped)
            else
                Service->>Project: install_native_agent(source, name)
                Project->>FS: Copy JSON verbatim + track hashes
            end
        end
    else translated (Claude/Copilot)
        loop Each .md/.agent.md file
            Service->>Agent: parse_agent_file()
            Agent-->>Service: AgentDefinition
            Service->>Agent: map tools to Kiro equivalents
            Service->>Service: Check for MCP servers
            alt has MCP and !accept_mcp
                Service->>Service: Add to warnings (skipped)
            else
                Service->>Agent: build_kiro_json(definition)
                Service->>Project: install_agent(json, prompt, name)
                Project->>FS: Write .kiro/agents/{name}.json
                Project->>FS: Write .kiro/agents/prompts/{name}.md
                Project->>FS: Update installed-agents.json
            end
        end
    end
```

**Key behaviors:**
- MCP-bearing agents require explicit `--accept-mcp` opt-in
- Native agents (format: "kiro-cli") are copied verbatim
- Translated agents produce JSON config + prompt markdown
- Tool names are mapped between dialects (unmapped tools generate warnings)
- Companion files (native) are tracked separately for cross-plugin collision detection
- RAII rollback removes partial writes if tracking update fails

---

## Update Marketplace

```mermaid
flowchart TD
    Start["update(name?)"] --> Check{name provided?}
    Check -->|yes| Single["Find marketplace by name"]
    Check -->|no| All["Load all known marketplaces"]
    Single --> Pull
    All --> Pull["For each: pull_repo()"]
    Pull --> Regen["Regenerate plugin registry"]
    Regen --> Result["UpdateResult<br/>{updated, failed, skipped}"]
```

**Key behaviors:**
- Local (linked) marketplaces are skipped (no remote to pull from)
- Pull failures are recorded but don't abort other updates
- Plugin registry is regenerated after successful pull

---

## Cache Prune

```mermaid
flowchart TD
    Start["prune_orphans(mode)"] --> Dirs["Scan marketplace dirs"]
    Dirs --> OrphanDirs["Remove dirs not in known-marketplaces.json"]
    Start --> Staging["Scan for _pending_* dirs"]
    Staging --> RemoveStaging["Remove stale staging dirs"]
    Start --> Registries["Scan registries/ dir"]
    Registries --> OrphanReg["Remove .json files for unknown marketplaces"]
    Start --> Locks["Scan for .lock files"]
    Locks --> StaleLocks["Remove locks whose target dir is gone"]
    
    OrphanDirs --> Report["PruneReport"]
    RemoveStaging --> Report
    OrphanReg --> Report
    StaleLocks --> Report
```

---

## Project Discovery (Desktop App)

```mermaid
sequenceDiagram
    participant UI as Svelte Frontend
    participant Tauri as Tauri Backend
    participant FS as File System

    UI->>Tauri: discover_projects()
    Tauri->>Tauri: Load scan roots from settings
    loop Each scan root
        Tauri->>FS: Walk directories (max depth, skip hidden/build)
        FS-->>Tauri: Directories containing .kiro/
    end
    Tauri-->>UI: DiscoveredProject[]
    UI->>UI: Display in ProjectPicker
    User->>UI: Select project
    UI->>Tauri: set_active_project(path)
```

---

## Typed IPC Flow (Desktop)

```mermaid
flowchart LR
    Svelte["Svelte Component"] -->|"commands.listMarketplaces()"| Bindings["bindings.ts<br/>(auto-generated)"]
    Bindings -->|"invoke('list_marketplaces')"| Tauri["Tauri IPC"]
    Tauri -->|"deserialize args"| Handler["Rust Command Handler"]
    Handler -->|"call"| Core["kiro-market-core"]
    Core -->|"Result<T, Error>"| Handler
    Handler -->|"serialize response"| Tauri
    Tauri -->|"typed response"| Svelte
```

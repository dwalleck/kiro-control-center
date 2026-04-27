# Architecture

## Design Philosophy

**Shared core, thin frontends.** All business logic lives in `kiro-market-core`. The CLI (`kiro-market`) and desktop app (`kiro-control-center`) are presentation-only wrappers that delegate to the core library. This ensures behavior parity between interfaces and keeps the test surface concentrated.

## System Layers

```mermaid
graph TB
    subgraph Frontends
        CLI["kiro-market (CLI)"]
        Desktop["kcc (Tauri Desktop)"]
    end

    subgraph Core["kiro-market-core"]
        Service["MarketplaceService<br/>orchestrator"]
        Project["KiroProject<br/>disk operations"]
        Cache["CacheDir<br/>marketplace storage"]
        Git["Git Backend<br/>gix + CLI"]
        Agent["Agent Module<br/>parse/emit/discover"]
    end

    subgraph Storage["On-Disk State"]
        CacheFS["~/.cache/kiro-market/"]
        ProjectFS[".kiro/ directory"]
    end

    CLI --> Service
    Desktop -->|"IPC (tauri-specta)"| Service
    Service --> Project
    Service --> Cache
    Service --> Git
    Service --> Agent
    Cache --> CacheFS
    Project --> ProjectFS
```

## Key Architectural Decisions

### 1. Generic Git Backend

`MarketplaceService` is generic over a `GitBackend` trait. Production uses `GixCliBackend` (tries gix first, falls back to git CLI). Tests inject mock backends for deterministic behavior without network access.

### 2. File-Based State (No Database)

All persistence is JSON on disk:
- `~/.cache/kiro-market/` — marketplace clones, plugin registries
- `.kiro/installed-skills.json` — installed skill tracking
- `.kiro/installed-agents.json` — installed agent tracking
- `.kiro/settings.json` — project-level Kiro settings

Concurrent access is serialized via `fs4` file locks.

### 3. RAII Cleanup Guards

`DirCleanupGuard` auto-removes staging directories on failure (Drop). Call `.defuse()` on success to keep the directory. This prevents orphaned temp dirs from interrupted operations.

### 4. Typed IPC via tauri-specta

Tauri commands are defined in Rust with `specta::Type` derives. The `generate_types` test produces `bindings.ts` with full TypeScript types for all commands and their return types. The frontend never uses untyped `invoke()`.

### 5. Dual Git Backend Strategy

```mermaid
flowchart LR
    Clone["clone_repo()"] --> Gix["Try gix"]
    Gix -->|success| Done["Return OK"]
    Gix -->|failure| CLI["Try git CLI"]
    CLI -->|success| Done
    CLI -->|failure| Combined["Combine both errors"]
```

`gix` provides pure-Rust git without subprocess overhead. The CLI fallback handles edge cases (auth helpers, unusual SSH configs). Both errors are combined for diagnostics.

### 6. Platform Abstraction

Local marketplaces use OS-native linking:
- **Unix**: symlinks
- **Windows**: NTFS junctions (preferred), recursive copy (fallback)

`MarketplaceStorage` enum tracks which method was used so the UI can warn about copy-mode limitations.

## Security Architecture

```mermaid
flowchart TD
    Input["User Input"] --> Validate["validate_name()<br/>validate_relative_path()"]
    Validate -->|pass| Process["Process Request"]
    Validate -->|fail| Reject["Return ValidationError"]
    
    Process --> PathCheck["RelativePath newtype<br/>enforces at construction"]
    Process --> MCPCheck["MCP opt-in gate<br/>--accept-mcp required"]
    Process --> TLSCheck["TLS enforcement<br/>reject http:// by default"]
    Process --> LinkCheck["Symlink rejection<br/>in copy_dir_recursive"]
```

**Invariants:**
1. All user-supplied names pass `validate_name()` (rejects traversal, reserved names, control chars)
2. All paths use `RelativePath` newtype (rejects absolute paths, `..`, backslash)
3. MCP agents require explicit `--accept-mcp` opt-in
4. `http://` sources rejected unless `--allow-insecure-http` passed
5. `copy_dir_recursive` skips symlinks and hardlinks in source trees
6. Workspace-level `unsafe_code = "forbid"`

## Error Handling Strategy

Errors are organized into domain-specific enums (`MarketplaceError`, `PluginError`, `SkillError`, `AgentError`, `GitError`) unified by a top-level `Error` enum with `From` conversions. Each variant carries enough context for actionable error messages (paths, names, source chains).

The Tauri layer wraps core errors into `CommandError` with an `ErrorType` discriminant for frontend-friendly categorization.

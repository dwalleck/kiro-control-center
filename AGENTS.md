# AGENTS.md

<!-- tags: navigation, architecture, conventions, ci -->

> Concise guide for AI agents working in this codebase. For detailed documentation, see `.agents/summary/index.md`.

## Table of Contents

- [Directory Map](#directory-map) — where to find things
- [Architecture](#architecture) — shared-core pattern, key entry points
- [Non-Obvious Patterns](#non-obvious-patterns) — deviations from defaults
- [CI & Hooks](#ci--hooks) — what runs automatically
- [Security Invariants](#security-invariants) — constraints you must not violate
- [Custom Instructions](#custom-instructions) — human-maintained conventions

---

## Directory Map

<!-- tags: navigation, structure -->

```
crates/
├── kiro-market-core/src/       # Shared library — ALL business logic lives here
│   ├── service.rs              # MarketplaceService: primary orchestrator (add/remove/update/install)
│   ├── service/browse.rs       # Skill enumeration, plugin install context resolution
│   ├── cache.rs                # CacheDir: ~/.cache/kiro-market/ management, source detection
│   ├── project.rs              # KiroProject: .kiro/ directory operations (install/remove skills+agents+steering)
│   ├── git.rs                  # Dual-backend git (gix primary, CLI fallback)
│   ├── agent/                  # Agent parsing (Claude + Copilot + Native dialects), tool mapping, Kiro emission
│   ├── steering/               # Steering file discovery and installation
│   ├── plugin.rs               # Plugin discovery (scan dirs for plugin.json)
│   ├── skill.rs                # SKILL.md frontmatter parsing
│   ├── validation.rs           # Path/name validation (security-critical)
│   ├── kiro_settings.rs        # .kiro/settings.json typed registry
│   ├── platform.rs             # OS abstraction (symlinks vs junctions vs copy)
│   ├── file_lock.rs            # Cross-process file locking (fs4)
│   ├── hash.rs                 # BLAKE3 content hashing for change detection
│   ├── raii.rs                 # DirCleanupGuard (RAII temp dir removal)
│   └── error.rs                # Structured error hierarchy
├── kiro-market/src/            # CLI binary — thin clap wrapper over core
│   ├── cli.rs                  # Clap derive definitions
│   ├── main.rs                 # Command dispatch
│   └── commands/               # One module per subcommand
└── kiro-control-center/        # Desktop app (Tauri 2 + Svelte 5)
    ├── src-tauri/src/          # Rust backend
    │   ├── lib.rs              # Tauri command registration (tauri-specta)
    │   └── commands/           # Tauri IPC handlers (call into kiro-market-core)
    └── src/                    # Svelte 5 frontend
        ├── lib/bindings.ts     # Auto-generated TypeScript bindings (DO NOT EDIT)
        ├── lib/stores/         # Svelte 5 $state module pattern
        └── lib/components/     # UI components (BrowseTab, InstalledTab, etc.)
```

**Key entry points:**
- CLI: `crates/kiro-market/src/main.rs` → dispatches to `commands/` modules
- Desktop: `crates/kiro-control-center/src-tauri/src/lib.rs` → registers Tauri commands
- Core logic: `crates/kiro-market-core/src/service.rs` → `MarketplaceService`

---

## Architecture

<!-- tags: architecture, patterns -->

**Shared core, thin frontends**: `kiro-market-core` contains all logic. CLI and desktop app are presentation-only wrappers. Never put business logic in the frontend crates.

**Service layer**: `MarketplaceService` accepts a `GitBackend` generic for testability. Tests inject mock backends.

**File-based state**: All persistence is JSON on disk. Concurrent access serialized via `fs4` file locks. No database.

**RAII guards**: `DirCleanupGuard` auto-removes staging dirs on failure. Call `.defuse()` on success.

**Typed IPC**: `tauri-specta` generates `bindings.ts` from Rust types. Regenerate after changing Tauri commands:
```
cargo test -p kiro-control-center --lib -- --ignored generate_types
```

**Feature flags on core**:
- `cli` — clap derives (CLI crate)
- `specta` — TypeScript binding derives (Tauri crate)
- `test-support` — test utilities

---

## Non-Obvious Patterns

<!-- tags: patterns, gotchas -->

- **curl dependency is a shim**: `kiro-market-core` depends on `curl = { workspace = true }` solely to activate the `ssl` feature on `curl-sys` (pulled transitively by `gix-transport`). It's not used for HTTP requests. Removing it breaks TLS on static-libcurl builds.

- **Cargo.lock edits are blocked**: A Claude hook (`hook-block-cargo-lock` in xtask) prevents direct edits. Use `cargo update -p <crate>` instead. Override with `KIRO_ALLOW_LOCKFILE_EDIT=1`.

- **`bindings.ts` is generated**: Never edit `src/lib/bindings.ts` manually. It's overwritten by the `generate_types` test.

- **Svelte 5 runes**: Frontend uses `$state` objects exported from modules (not Svelte 4 stores). Mutations go through the deep state proxy on a const object.

- **Plugin reference format**: Always `plugin@marketplace` (split on first `@`).

- **Default scan paths**: When a plugin has no `plugin.json`, skills are discovered from `./skills/`, agents from `./agents/`, and steering from `./steering/`.

- **Dual git backend**: `gix` is tried first, then `git` CLI. Both errors are combined. Auth failures get user-friendly messages.

- **Platform linking**: Local marketplaces use symlinks (Unix) or NTFS junctions (Windows). If junctions fail, falls back to recursive copy. `MarketplaceStorage` enum tracks which method was used.

- **Self-cycle dev-dep**: `kiro-market-core` has itself as a dev-dependency with `features = ["test-support"]` to activate test utilities in integration tests (Cargo handles this without recursion).

- **BLAKE3 hash tracking**: Both source and installed content hashes are stored. Idempotent reinstalls are skipped when hashes match; content changes require `--force`.

---

## CI & Hooks

<!-- tags: ci, automation, hooks -->

**CI jobs** (all must pass for merge):
- `commitlint` — conventional commits (`feat|fix|docs|...` prefix required)
- `format` — `cargo fmt --all -- --check`
- `lint` — `cargo clippy --workspace -- -D warnings`
- `test` — full workspace on Linux, core+CLI on macOS/Windows
- `frontend` — `svelte-check` + `npm run build`
- `build-cli` — release build on 3 OS (artifacts uploaded)
- `build-tauri` — desktop app build on 3 OS (deb/dmg/msi)
- `cargo-deny` — license and advisory audit
- `assert-curl-tls` — verifies `curl-sys` has `ssl` feature active
- `coverage` — `cargo-llvm-cov` → Codecov

**Claude hooks** (`.claude/settings.json`):
- **PreToolUse** (Write/Edit/MultiEdit): runs `cargo xtask hook-block-cargo-lock` — blocks Cargo.lock edits
- **PostToolUse** (Write/Edit): runs `cargo xtask hook-post-edit` — runs `rustfmt` then `clippy` on the edited file's package

**Workspace lints**:
- `unsafe_code = "forbid"`
- `clippy::all = "warn"`, `clippy::pedantic = "warn"`

---

## Security Invariants

<!-- tags: security, constraints -->

These MUST be maintained in all changes:

1. **Path traversal prevention**: All user-supplied names pass through `validate_name()` and paths through `validate_relative_path()`. The `RelativePath` newtype enforces this at construction.

2. **MCP opt-in**: Agents with MCP servers require `--accept-mcp`. Never auto-install MCP-bearing agents.

3. **TLS by default**: `http://` marketplace sources are rejected unless `--allow-insecure-http` is explicitly passed.

4. **Symlink/hardlink rejection**: `copy_dir_recursive` skips symlinks and hardlinks in source trees.

5. **No unsafe code**: Workspace-level `unsafe_code = "forbid"`.

---

## Custom Instructions
<!-- This section is for human and agent-maintained operational knowledge.
     Add repo-specific conventions, gotchas, and workflow rules here.
     This section is preserved exactly as-is when re-running codebase-summary. -->


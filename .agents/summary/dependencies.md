# Dependencies

## Rust Dependencies (Workspace)

### Core Functionality

| Crate | Version | Purpose | Used By |
|-------|---------|---------|---------|
| `gix` | 0.81 | Pure-Rust git operations (clone, pull, checkout) | kiro-market-core |
| `curl` | 0.4 | **TLS shim only** — activates `ssl` feature on `curl-sys` for gix-transport. Not used for HTTP requests. | kiro-market-core |
| `fs4` | 0.13 | Cross-process file locking for concurrent access serialization | kiro-market-core |
| `blake3` | 1.5 | Content hashing for change detection (source/installed hash comparison) | kiro-market-core |
| `junction` | 1 | Windows NTFS junction creation for local marketplace linking | kiro-market-core (Windows only) |
| `tempfile` | 3 | Staging directories with OS-cleanup-on-Drop semantics | kiro-market-core |

### Serialization

| Crate | Version | Purpose | Used By |
|-------|---------|---------|---------|
| `serde` | 1 | Serialization framework (derive) | All crates |
| `serde_json` | 1 | JSON parsing/writing for all on-disk state | All crates |
| `serde_yaml_ng` | 0.10 | YAML frontmatter parsing in SKILL.md and agent files | kiro-market-core |

### CLI

| Crate | Version | Purpose | Used By |
|-------|---------|---------|---------|
| `clap` | 4 | Command-line argument parsing (derive API) | kiro-market |
| `colored` | 3 | Terminal color output | kiro-market |

### Desktop App

| Crate | Version | Purpose | Used By |
|-------|---------|---------|---------|
| `tauri` | 2 | Desktop app framework (webview + IPC) | kiro-control-center |
| `tauri-specta` | 2.0.0-rc.24 | Auto-generate TypeScript bindings from Rust types | kiro-control-center |
| `specta` | 2.0.0-rc.24 | Type reflection for TypeScript generation | kiro-control-center |
| `tauri-plugin-dialog` | 2 | Native file/folder dialog | kiro-control-center |
| `tauri-plugin-opener` | 2 | Open URLs/files with system default | kiro-control-center |

### Infrastructure

| Crate | Version | Purpose | Used By |
|-------|---------|---------|---------|
| `thiserror` | 2 | Derive `Error` trait for domain error types | kiro-market-core |
| `anyhow` | 1 | Ergonomic error handling in CLI/binary contexts | kiro-market |
| `tracing` | 0.1 | Structured logging | All crates |
| `tracing-subscriber` | 0.3 | Log output formatting with env-filter | kiro-market, kiro-control-center |
| `dirs` | 6 | Platform-standard directory paths (cache, config) | All crates |
| `chrono` | 0.4 | Timestamps for install tracking | All crates |
| `hex` | 0.4 | Hex encoding for hash display | kiro-market-core |

### Testing

| Crate | Version | Purpose | Used By |
|-------|---------|---------|---------|
| `rstest` | 0.26 | Parameterized test fixtures | All crates (dev) |
| `tempfile` | 3 | Temporary directories for test isolation | All crates (dev) |
| `tokio` | 1 | Async runtime for Tauri command tests | kiro-control-center (dev) |

---

## Frontend Dependencies (npm)

### Runtime

| Package | Version | Purpose |
|---------|---------|---------|
| `@tauri-apps/api` | ^2 | Tauri IPC from JavaScript |
| `@tauri-apps/plugin-dialog` | ^2.7.0 | Native dialog bindings |
| `@tauri-apps/plugin-opener` | ^2 | URL/file opener bindings |
| `tailwindcss` | ^4.2.2 | Utility-first CSS framework |
| `@tailwindcss/postcss` | ^4.2.2 | PostCSS integration for Tailwind |
| `postcss` | ^8.5.8 | CSS processing pipeline |

### Development

| Package | Version | Purpose |
|---------|---------|---------|
| `svelte` | ^5.0.0 | UI framework (runes mode) |
| `@sveltejs/kit` | ^2.9.0 | SvelteKit framework |
| `@sveltejs/adapter-static` | ^3.0.6 | Static site generation for Tauri |
| `@sveltejs/vite-plugin-svelte` | ^5.0.0 | Vite integration |
| `@tauri-apps/cli` | ^2 | Tauri build tooling |
| `vite` | ^6.0.3 | Build tool and dev server |
| `typescript` | ~5.6.2 | Type checking |
| `svelte-check` | ^4.0.0 | Svelte type checking |
| `@playwright/test` | ^1.59.1 | E2E testing |

---

## Dependency Notes

### Why `curl` exists but isn't used for HTTP

The `curl` crate dependency exists solely to activate the `ssl` feature on `curl-sys`, which is pulled transitively by `gix-transport`. Without this feature activation, static-libcurl builds (Windows minimal, vendored CI) would build `curl-sys` without a TLS backend, causing HTTPS clones to silently fail or fall back to plaintext. The CI job `assert-curl-tls` verifies this feature is active.

### Why `tempfile` appears in both deps and dev-deps

Production code uses `tempfile::TempDir` for staging directories (OS-cleanup-on-Drop replaces hand-rolled unique-name generation). Dev-deps use it for test isolation. The workspace entry covers both uses.

### External Services

This project has **no runtime external service dependencies** beyond git repositories. All state is local JSON files. No databases, no cloud APIs, no authentication services.

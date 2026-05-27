# Dependencies

<!-- tags: dependencies, external, crates, npm -->

## Rust Workspace Dependencies

### Serialization

| Crate | Version | Purpose |
|---|---|---|
| `serde` | 1 | Derive `Serialize`/`Deserialize` on all data types |
| `serde_json` | 1 | JSON read/write for tracking files, plugin manifests, Tauri IPC |
| `serde_yaml_ng` | 0.10 | YAML frontmatter parsing in `SKILL.md` and agent files |
| `toml` | 0.8 | Cargo manifest parsing in xtask (`parse_package_name`) |

### CLI

| Crate | Version | Purpose |
|---|---|---|
| `clap` | 4 | CLI argument parsing (derive mode). Optional in core via `cli` feature. |

### Git

| Crate | Version | Purpose | Notes |
|---|---|---|---|
| `gix` | 0.81 | Primary git backend: clone, pull, SHA verification | `blocking-network-client` + `blocking-http-transport-curl` features |
| `curl` | 0.4 | **TLS activator only** — not used for HTTP requests | Activates `ssl` feature on `curl-sys` (transitive via `gix-transport`). Without this, HTTPS clones silently fall back to plaintext on static-libcurl builds. The `assert-curl-tls` CI job guards this invariant. |

### Console

| Crate | Version | Purpose |
|---|---|---|
| `colored` | 3 | ANSI color output in CLI. Respects `NO_COLOR` env var. |

### Error Handling

| Crate | Version | Purpose |
|---|---|---|
| `thiserror` | 2 | Derive `Error` on structured error enums |
| `anyhow` | 1 | Used in CLI binary for top-level error propagation |

### Logging

| Crate | Version | Purpose |
|---|---|---|
| `tracing` | 0.1 | Structured logging macros throughout core and CLI |
| `tracing-subscriber` | 0.3 | Subscriber setup in CLI main (env-filter feature) |

### File System

| Crate | Version | Purpose |
|---|---|---|
| `dirs` | 6 | Resolves `~/.cache/` and home directory paths |
| `fs4` | 0.13 | Cross-process file locking for tracking files |
| `tempfile` | 3 | `TempDir` for staging directories during install |
| `junction` | 1 | Windows NTFS junction creation for local marketplace linking. Windows-only target dep. |

### Hashing

| Crate | Version | Purpose |
|---|---|---|
| `blake3` | 1.5 | Content hashing for change detection. Hashes stored as `blake3:<hex>` strings. |
| `hex` | 0.4 | Hex encoding/decoding for `BlakeHash` |

### Date/Time

| Crate | Version | Purpose |
|---|---|---|
| `chrono` | 0.4 | `installed_at` timestamps in tracking files. `serde` feature for JSON serialization. **Not** used in Tauri IPC types (specta's chrono feature is off). |

### Tauri (kiro-control-center only)

| Crate | Version | Purpose |
|---|---|---|
| `tauri` | 2 | Desktop app framework |
| `tauri-specta` | (workspace) | Generates TypeScript bindings from Rust command types |
| `specta` | 2.0.0-rc.24 | Type reflection for TS binding generation. Optional via `specta` feature. |
| `tauri_plugin_opener` | 2 | Opens files/URLs from the desktop app |
| `tauri_plugin_dialog` | 2.7.0 | Native file/folder picker dialogs |

### Testing

| Crate | Version | Purpose |
|---|---|---|
| `rstest` | 0.26 | Parameterized test fixtures |
| `tempfile` | 3 | Temporary directories in tests |

---

## npm Dependencies (kiro-control-center frontend)

### Runtime

| Package | Version | Purpose |
|---|---|---|
| `@tauri-apps/api` | ^2 | Tauri IPC `invoke()` and event APIs |
| `@tauri-apps/plugin-dialog` | ^2.7.0 | Native dialog plugin JS bindings |
| `@tauri-apps/plugin-opener` | ^2 | Opener plugin JS bindings |
| `tailwindcss` | ^4.2.2 | Utility CSS framework |
| `@tailwindcss/postcss` | ^4.2.2 | PostCSS integration for Tailwind 4 |
| `postcss` | ^8.5.8 | CSS processing |

### Dev

| Package | Version | Purpose |
|---|---|---|
| `svelte` | ^5.0.0 | UI framework (runes mode) |
| `@sveltejs/kit` | ^2.9.0 | SvelteKit app framework |
| `@sveltejs/adapter-static` | ^3.0.6 | Static adapter (Tauri uses static output) |
| `@sveltejs/vite-plugin-svelte` | ^5.0.0 | Vite plugin for Svelte |
| `vite` | ^6.0.3 | Build tool and dev server (port 1420) |
| `typescript` | ~5.6.2 | TypeScript compiler |
| `svelte-check` | ^4.0.0 | Svelte type checking (`npm run check`) |
| `vitest` | ^2.1.0 | Unit test runner for TypeScript/Svelte |
| `@playwright/test` | ^1.59.1 | End-to-end tests (`tests/e2e/`) |
| `@tauri-apps/cli` | ^2 | `npx tauri` commands |
| `@types/node` | ^25.5.2 | Node.js type definitions |

---

## Non-Obvious Dependency Notes

- **`curl` is a shim**: The `curl = { workspace = true }` entry in `kiro-market-core` exists solely to activate `ssl` on `curl-sys`. It is not used for any HTTP requests. Removing it breaks TLS on static-libcurl builds silently.

- **`specta` is optional**: Only activated when building the Tauri crate (`specta` feature). The core library compiles without it for CLI use.

- **`tempfile` is a runtime dep in core**: Used for `TempDir` staging directories during install, not just in tests.

- **`junction` is Windows-only**: Declared under `[target.'cfg(windows)'.dependencies]` in `kiro-market-core`.

- **`chrono` must not appear in Tauri IPC types**: `specta`'s chrono feature is disabled in `kiro-control-center`. The `bindings_export_plugin_catalog_view` test asserts no `DateTime` types appear in `bindings.ts`.

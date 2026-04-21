# kiro-market — Developer Guide

## Build
```bash
cargo build
```

## Test
```bash
cargo test                          # all tests
cargo test -p kiro-market-core      # core library tests
cargo test -p kiro-market           # CLI + integration tests
```

## Lint
```bash
cargo clippy --workspace -- -D warnings
```

## Pre-commit
Run all three before committing — CI enforces each:
- `cargo fmt --all --check`
- `cargo test --workspace`
- `cargo clippy --workspace --tests -- -D warnings`

## Project Structure
- `crates/kiro-market-core/` — library crate (types, parsing, git, cache, project state)
- `crates/kiro-market/` — binary crate (CLI commands)

## Code Style
- Edition 2024, rust-version 1.85.0
- `thiserror` for typed errors in kiro-market-core
- Error chain: `#[source]` on inner variants (e.g. `PluginError::ManifestReadFailed { #[source] source: io::Error }`), `#[error(transparent)]` on top-level `Error` variants so `.source()` walks through. At Tauri/log boundaries call `error_full_chain(&err)` from `kiro_market_core::error` — `err.to_string()` only prints outer Display and loses source detail.
- Prefer dedicated enum variants over `reason: String` sentinels when callers might branch on the semantic (e.g. `NotADirectory` / `SymlinkRefused` vs. a shared `DirectoryUnreadable { reason }`). `io::Error` goes directly in `#[source]` — no `Box` needed, it's `Send + Sync + 'static`.
- **Parse, don't validate, at deserialization boundaries.** Untrusted string fields from manifests (`marketplace.json`, `plugin.json`, agent frontmatter) get wrapped in a newtype with a private inner field and a fallible `new` — see `RelativePath` (`validation.rs:28`) and `GitRef` (`git.rs:34`) as templates. Implement `Deserialize` to route through `new` so `serde_json::from_slice` rejects bad input at parse time, not later. A free `validate_xyz(&Thing) -> Result<()>` that nothing constructs is usually a missed newtype.
- `anyhow` for error propagation in kiro-market binary
- `rstest` for parameterized tests, `tempfile` for test fixtures
- `clippy::all` and `clippy::pedantic` enabled as warnings
- `unsafe_code` is forbidden
- **Zero-tolerance in production code** (tests are exempt): no `.unwrap()`, no `.expect()`, no `let _ = ...` discarding a `Result`, no `#[allow(...)]` directives. If a lint or warning is wrong, fix the code or the lint config — don't suppress at the call site. Once one waiver lands, `rg unwrap` stops being a safety check.
- **Map external errors at the adapter boundary.** `gix`, `io`, `serde_json` errors get translated into typed `ErrorKind` variants inside the module that calls them (e.g. `git.rs`, `cache.rs`) — they never appear in the public API of `kiro-market-core`.

## Architecture
The tool reads Claude Code `marketplace.json` catalogs, discovers plugins and skills,
and installs them into Kiro CLI projects at `.kiro/skills/`.

**Dependencies point inward.** `kiro-market-core` is the domain core and must stay free of UI, Tauri, async-runtime, and frontend deps. The Tauri crate (`crates/kiro-control-center/src-tauri`) and the CLI both depend on the core; the core never depends on them. If you find yourself wanting to add `tauri`, `tokio`, or a UI crate to `kiro-market-core/Cargo.toml`, the abstraction belongs in the consumer crate instead.

Skill directories are copied wholesale (SKILL.md + `references/` companion files)
so that Kiro's native lazy loading can resolve companion files on demand.

### Service Layer
Marketplace operations (add/remove/update/list) live in `kiro-market-core::service::MarketplaceService`.
CLI and Tauri handlers are thin wrappers that construct the service, call it, and format output.
Domain logic is never duplicated between frontends.

### Git Abstraction
Git operations are abstracted behind the `GitBackend` trait (`kiro-market-core::git`).
`GixCliBackend` implements the trait using `gix` for clone/open and the system `git` CLI
for pull/checkout. The trait enables mock-based testing without filesystem git repos.

### Platform Abstraction
Local marketplace linking uses `kiro-market-core::platform` which provides
`create_local_link`/`is_local_link`/`remove_local_link`. On Unix this uses symlinks,
on Windows it uses directory junctions with copy fallback.

## Key Crate Dependencies
- `gix` + system `git` CLI — git operations (gix for clone/open, system git for pull/checkout)
- `clap` (derive) — CLI framework
- `serde` / `serde_json` / `serde_yaml` — JSON and YAML parsing
- `colored` — terminal output
- `dirs` — XDG path resolution

# kiro-market — Developer Guide

## Build
```bash
cargo build
```

## Test
```bash
cargo test                                             # all tests
cargo test -p kiro-market-core                         # core library tests
cargo test -p kiro-market                              # CLI + integration tests
cargo test -p kiro-control-center --lib -- --ignored   # regenerate bindings.ts
```

## Frontend (Tauri crate)
From `crates/kiro-control-center/`:
- `npm run check` — Svelte + TypeScript typecheck via `svelte-check`. Run after any core type change that flows through `bindings.ts`.

## Lint
```bash
cargo clippy --workspace -- -D warnings
```

## Pre-commit
Run all three before committing — CI enforces each:
- `cargo fmt --all --check`
- `cargo test --workspace`
- `cargo clippy --workspace --tests -- -D warnings`

## xtask
- `cargo xtask hook-post-edit` — wired into Claude Code `PostToolUse` for `.rs` edits. Runs `rustfmt` then `cargo clippy --package <derived> -- -D warnings`. The package is derived by walking up ancestors for the nearest `Cargo.toml` with a `[package]` table (`xtask::derive_package`), so new workspace crates are picked up automatically. Read/parse failures log to stderr; loop exhaust emits a "no usable Cargo.toml" diagnostic.
- `cargo xtask hook-block-cargo-lock` — blocks direct `Cargo.lock` edits. Override for one session via `KIRO_ALLOW_LOCKFILE_EDIT=1`.
- `cargo xtask plan-lint` — runs structural lint queries against the [tethys](https://github.com/dwalleck/rivets/tree/main/crates/tethys) index. First implemented gate is **gate-4-external-error-boundary**: a SQL query against the `attributes` and `symbols` tables that catches any `pub` enum variant carrying an external crate's error type (`serde_json`, `gix`, `reqwest`, `toml`) via `#[source]`. This is the rule the broken grep in `docs/plan-review-checklist.md` was supposed to enforce. Requires the `tethys` binary on PATH (or `TETHYS_BIN` env var); pass `--no-reindex` to query the existing `.rivets/index/tethys.db` without re-indexing first. Exits 1 on findings (CI gate fails).

## Project Structure
- `crates/kiro-market-core/` — library crate (types, parsing, git, cache, project state)
- `crates/kiro-market/` — binary crate (CLI commands)

## Code Style
- Edition 2024, rust-version 1.85.0
- `thiserror` for typed errors in kiro-market-core
- Error chain: `#[source]` on inner variants (e.g. `PluginError::ManifestReadFailed { #[source] source: io::Error }`), `#[error(transparent)]` on top-level `Error` variants so `.source()` walks through. At Tauri/log boundaries, AND in any wire-format `reason`/`error: String` field that crosses the FFI, use `error_full_chain(&err)` — not `err.to_string()`, which drops the source chain. See `SkippedPlugin::from_plugin_error` and `FailedSkill::install_failed` as the canonical constructors.
- Don't name a non-`Error` payload field `source` on a `thiserror`-derived variant — the name is reserved for the `Error::source()` impl and requires the type to implement `Error`. Rename (e.g. `plugin_source: StructuredSource`) and keep the wire-format name via the projection to `SkippedReason`.
- Prefer dedicated enum variants over `reason: String` sentinels when callers might branch on the semantic (e.g. `NotADirectory` / `SymlinkRefused` vs. a shared `DirectoryUnreadable { reason }`). `io::Error` goes directly in `#[source]` — no `Box` needed, it's `Send + Sync + 'static`.
- **Parse, don't validate, at deserialization boundaries.** Untrusted string fields from manifests (`marketplace.json`, `plugin.json`, agent frontmatter) get wrapped in a newtype with a private inner field and a fallible `new` — see `RelativePath` (`validation.rs:28`), `GitRef` (`git.rs:34`), and `AgentName` (`validation.rs`) as templates. Implement `Deserialize` to route through `new` so `serde_json::from_slice` rejects bad input at parse time, not later. A free `validate_xyz(&Thing) -> Result<()>` that nothing constructs is usually a missed newtype. **Exception:** keep raw `Option<String>` on a transient projection struct when post-parse routing needs to split failures across distinct error variants (e.g. `NativeAgentProjection.name` stays `Option<String>` so `MissingName` / `InvalidName(reason)` / `InvalidJson` route to three distinct `AgentError` variants instead of collapsing into `InvalidJson(serde_json::Error)`). The type-level guarantee still lands at the *bundle* boundary (`NativeAgentBundle.name: AgentName`).
- **Validation newtypes that may flow through Tauri bindings need `#[cfg_attr(feature = "specta", derive(specta::Type))]`** so they emit a TypeScript alias via `bindings.ts`. Match `RelativePath`'s shape (`validation.rs:26`). Skipping this on initial creation is a latent break — adding it later is harmless, but the moment a `#[tauri::command]` returns a type embedding the newtype, the Tauri crate stops compiling.
- **Classifier functions over error enums enumerate every variant.** `SkippedReason::from_plugin_error`, `PluginError::remediation_hint`, and any similar "project a `PluginError` into a narrower type or pick a branch per variant" function must match every variant explicitly — no `_ => None` / `_ => default`. A new `PluginError` variant should then force a compile-time classification decision rather than silently defaulting. Two classifiers that share the same input enum drift one `_` apart otherwise.
- **Classifier idempotent-payload rule.** When `classify_*_collision` returns `CollisionDecision::Idempotent(Box<T>)`, `T` must contain only data the classifier *actually sees* — not data the caller has but didn't pass in. The steering classifier shipped a bug where `T = InstalledSteeringOutcome` led it to substitute `dest` for the missing `source` path, leaking the destination into the wire-format `source` field on idempotent reinstalls. Fix: classifier returns a minimal echo type (e.g. `SteeringIdempotentEcho { prior_installed_hash: String }`) and the caller assembles the full outcome where `source.source` is in scope.
- `anyhow` for error propagation in kiro-market binary
- `rstest` for parameterized tests, `tempfile` for test fixtures
- `clippy::all` and `clippy::pedantic` enabled as warnings
- `unsafe_code` is forbidden
- **Zero-tolerance in production code** (tests are exempt): no `.unwrap()`, no `.expect()`, no `let _ = ...` discarding a `Result`, no `#[allow(...)]` directives. If a lint or warning is wrong, fix the code or the lint config — don't suppress at the call site. Once one waiver lands, `rg unwrap` stops being a safety check.
- **Map external errors at the adapter boundary.** `gix`, `serde_json`, `toml`, `reqwest` errors get translated into typed `ErrorKind` variants inside the module that calls them (e.g. `git.rs`, `cache.rs`, `agent/parse_native.rs`) — they never appear in the public API of `kiro-market-core`. (`io::Error` is std-library and exempted; it can carry through `#[source]`.) Recipe when the variant *would* carry an external error: `#[non_exhaustive]` enum + variant field `reason: String` (not `#[source]`) + a `pub(crate) fn` constructor that calls `error_full_chain(&err)`. Canonical examples: `parse_native::NativeParseFailure::invalid_json` (for `serde_json::Error`), `steering::tracking_malformed` (for `serde_json::Error` in `SteeringError::TrackingMalformed`). Tests should assert `err.source().is_none()` to lock the contract. **Enforced by `cargo xtask plan-lint --gate gate-4-external-error-boundary`** — a SQL query against the tethys index that flags any `pub` enum variant carrying an external crate's error type via `#[source]`.

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

### Tauri command handlers
Each `#[tauri::command]` splits into a thin wrapper plus a private `fn <name>_impl(svc: &MarketplaceService, ...) -> Result<T, CommandError>`. The wrapper does the globals work (`make_service()?`) and calls the `_impl`; the `_impl` is pure Rust and takes the service + primitives by reference. Tests exercise `_impl` directly using fixtures from `kiro_market_core::service::test_support` (activated via a `dev-dependencies` feature override). See `install_skills_impl` in `crates/kiro-control-center/src-tauri/src/commands/browse.rs` and its `#[cfg(test)] mod tests` for the exemplar. New commands follow this shape so their bodies are testable without a Tauri runtime.

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

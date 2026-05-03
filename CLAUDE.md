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
- `npm run dev` — vite serves on `http://localhost:1420` (Tauri convention; NOT vite's default 5173).
- `npm run test:e2e` — Playwright e2e at `tests/e2e/app.spec.ts`. Tests gate on `FIXTURE_MARKETPLACE_PATH` and `test.skip` cleanly when unset.

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
- `cargo xtask plan-lint` — runs structural lint queries against the [tethys](https://github.com/dwalleck/rivets/tree/main/crates/tethys) index. Gates implemented:
  - **gate-4-external-error-boundary** — SQL query against `attributes` and `symbols` that flags any `pub` enum variant carrying an external crate's error type (`serde_json`, `gix`, `reqwest`, `toml`) via `#[source]`. Replaces the broken grep in `docs/plan-review-checklist.md`.
  - **no-unwrap-in-production** — SQL query against `refs` joined to `symbols` and `files` that flags `.unwrap()` and `.expect()` calls in non-test production code, enforcing the CLAUDE.md "zero-tolerance" rule. Filters: `is_test = 0`, plus path-based exemptions for `tests/`, `benches/`, `test_support`, and `test_utils`.

  Requires the `tethys` binary on PATH (or `TETHYS_BIN` env var); pass `--no-reindex` to query the existing `.rivets/index/tethys.db` without re-indexing first; pass `--gate <NAME>` to run a single gate. Exits 1 on findings (CI gate fails).

## Planning
After writing a plan and before starting implementation, apply the 6 gates in `docs/plan-review-checklist.md` (Grounding / Threat Model / Wire Format / External Type Boundary / Type Design / Reference vs Transcription). The gates also fire as code-review questions on any change touching the public API of `kiro-market-core`. Originated from the PR #64 retrospective; Gate 6 added from the PR #96 retrospective (steering/agents scan-path bug — a faithful encoding of a plan that transcribed install-time *output* instead of citing install-time *mechanism*). Complement to (not replacement for) the upstream `superpowers:writing-plans` skill: invoke that skill first, then run the gates as a self-review pass before declaring the plan implementation-ready.

Plan-review = **two complementary passes**, not one:
1. **LSP-first** — `documentSymbol` on every file the plan modifies; `workspaceSymbol` to confirm cross-file references. Catches signature drift, missing exports, field-access typos in one call (vs. many greps). Cheap; do this first.
2. **Code-reviewer-style** — walk each task asking "does this do the right thing?" Catches behavioral semantics (cascade abort patterns, recoverable-vs-fatal classification), cross-task drift (when fixing pattern X in task N, search siblings for the same shape), action-item linkage (did the design doc say to do X, does any task actually do X?), and data-shape ambiguity (HashMap keyed by enough fields?).

Neither pass substitutes for the other. PR #93's experience: 11 LSP-first findings + 12 code-reviewer-style findings, almost no overlap. A plan that passes only LSP is half-reviewed. Diminishing-returns signal: when findings shrink to compiler-catchable shapes (`DateTime<Utc>` vs. specta features, `self.` vs. `Self::` for associated functions), stop adding plan-time amendments and let the implementation forcing-functions catch the rest.

## Project Structure
- `crates/kiro-market-core/` — library crate (types, parsing, git, cache, project state)
- `crates/kiro-market/` — binary crate (CLI commands)

## Worktree convention
Feature branches go in sibling directories: `~/repos/kiro-marketplace-cli-<topic>` (e.g. `kiro-marketplace-cli-plugin-impl`). Each worktree has its own `target/` and `node_modules/` — run `npm install` in `crates/kiro-control-center/` after creating a fresh worktree. Tethys also indexes per-worktree under `.rivets/index/`; the parent's index doesn't transfer.

Pattern: `git worktree add /home/dwalleck/repos/kiro-marketplace-cli-<topic> -b <branch> origin/main`. Cleanup after merge: `git worktree remove <path> && git branch -d <branch> && git pull --ff-only`.

## Code Style
- Edition 2024, rust-version 1.85.0
- `thiserror` for typed errors in kiro-market-core
- Error chain: `#[source]` on inner variants (e.g. `PluginError::ManifestReadFailed { #[source] source: io::Error }`), `#[error(transparent)]` on top-level `Error` variants so `.source()` walks through. At Tauri/log boundaries, AND in any wire-format `reason`/`error: String` field that crosses the FFI, use `error_full_chain(&err)` — not `err.to_string()`, which drops the source chain. See `SkippedPlugin::from_plugin_error` and `FailedSkill::install_failed` as the canonical constructors.
- Don't name a non-`Error` payload field `source` on a `thiserror`-derived variant — the name is reserved for the `Error::source()` impl and requires the type to implement `Error`. Rename (e.g. `plugin_source: StructuredSource`) and keep the wire-format name via the projection to `SkippedReason`.
- Prefer dedicated enum variants over `reason: String` sentinels when callers might branch on the semantic (e.g. `NotADirectory` / `SymlinkRefused` vs. a shared `DirectoryUnreadable { reason }`). `io::Error` goes directly in `#[source]` — no `Box` needed, it's `Send + Sync + 'static`.
- **Parse, don't validate, at deserialization boundaries.** Untrusted string fields from manifests (`marketplace.json`, `plugin.json`, agent frontmatter) get wrapped in a newtype with a private inner field and a fallible `new` — see `RelativePath` (`validation.rs:28`), `GitRef` (`git.rs:34`), and `AgentName` (`validation.rs`) as templates. Implement `Deserialize` to route through `new` so `serde_json::from_slice` rejects bad input at parse time, not later. A free `validate_xyz(&Thing) -> Result<()>` that nothing constructs is usually a missed newtype. **Exception:** keep raw `Option<String>` on a transient projection struct when post-parse routing needs to split failures across distinct error variants (e.g. `NativeAgentProjection.name` stays `Option<String>` so `MissingName` / `InvalidName(reason)` / `InvalidJson` route to three distinct `AgentError` variants instead of collapsing into `InvalidJson(serde_json::Error)`). The type-level guarantee still lands at the *bundle* boundary (`NativeAgentBundle.name: AgentName`).
- **Validation newtypes that may flow through Tauri bindings need `#[cfg_attr(feature = "specta", derive(specta::Type))]`** so they emit a TypeScript alias via `bindings.ts`. Match `RelativePath`'s shape (`validation.rs:26`). Skipping this on initial creation is a latent break — adding it later is harmless, but the moment a `#[tauri::command]` returns a type embedding the newtype, the Tauri crate stops compiling.
- **`chrono::DateTime<Utc>` cannot appear on `specta::Type`-derived structs.** `kiro-market-core`'s `specta` feature set is `["derive", "serde_json"]` — no `"chrono"` flag. Convert to `String` at the FFI boundary via `.to_rfc3339()` (precedent: `commands/installed.rs::InstalledSkillInfo.installed_at`). Every `DateTime<Utc>` in `project.rs` lives on a struct that intentionally does NOT derive `specta::Type`.
- **Classifier functions over error enums enumerate every variant.** `SkippedReason::from_plugin_error`, `PluginError::remediation_hint`, and any similar "project a `PluginError` into a narrower type or pick a branch per variant" function must match every variant explicitly — no `_ => None` / `_ => default`. A new `PluginError` variant should then force a compile-time classification decision rather than silently defaulting. Two classifiers that share the same input enum drift one `_` apart otherwise.
- **Classifier idempotent-payload rule.** When `classify_*_collision` returns `CollisionDecision::Idempotent(Box<T>)`, `T` must contain only data the classifier *actually sees* — not data the caller has but didn't pass in. The steering classifier shipped a bug where `T = InstalledSteeringOutcome` led it to substitute `dest` for the missing `source` path, leaking the destination into the wire-format `source` field on idempotent reinstalls. Fix: classifier returns a minimal echo type (e.g. `SteeringIdempotentEcho { prior_installed_hash: String }`) and the caller assembles the full outcome where `source.source` is in scope.
- `anyhow` for error propagation in kiro-market binary
- `rstest` for parameterized tests, `tempfile` for test fixtures
- `clippy::all` and `clippy::pedantic` enabled as warnings
- `unsafe_code` is forbidden
- **Zero-tolerance in production code** (tests are exempt): no `.unwrap()`, no `.expect()`, no `let _ = ...` discarding a `Result`, no `#[allow(...)]` directives. If a lint or warning is wrong, fix the code or the lint config — don't suppress at the call site. **Enforced by `cargo xtask plan-lint --gate no-unwrap-in-production`**. Deliberate exceptions (idiomatic Tauri/Specta startup panics, etc.) are registered in `xtask/src/plan_lint.rs`'s `ALLOWED_SITES` const with a written-down reason — that's the audit trail. Adding to the allowlist requires a code change reviewed in PR; there is no inline `#[allow(...)]` escape hatch.
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

The `_impl(svc, ...)` rule applies to **service-consuming** commands. Project-only reads (no `MarketplaceService` needed — e.g. `list_installed_skills`, `remove_skill` in `commands/installed.rs`) put the body inline in the wrapper with no `_impl` at all. Don't add an unused `svc` parameter to satisfy the rule mechanically.

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

# kiro-control-center — Agent and Developer Guide

<!-- tags: navigation, architecture, conventions, ci, security, workflow -->

> Shared instructions for this repository. `CLAUDE.md` and `AGENTS.md` must
> remain byte-identical; update both together. For deeper documentation, see
> `.agents/summary/index.md`.

## Table of Contents

- [Quick Commands](#quick-commands)
- [Directory Map](#directory-map)
- [Architecture](#architecture)
- [Non-Obvious Patterns](#non-obvious-patterns)
- [Tooling and Hooks](#tooling-and-hooks)
- [Work Tracking](#work-tracking-rivets)
- [Planning](#planning)
- [Worktree Convention](#worktree-convention)
- [Code Style](#code-style)
- [CI](#ci)
- [Security Invariants](#security-invariants)
- [Key Crate Dependencies](#key-crate-dependencies)
- [Custom Instructions](#custom-instructions)

---

## Quick Commands

### Build

```bash
cargo build
```

### Test

```bash
cargo test                                             # all tests
cargo test -p kiro-market-core                         # core library tests
cargo test -p kiro-market                              # CLI + integration tests
cargo test -p kiro-control-center tests::generate_types -- --exact --ignored
```

The final command regenerates `crates/kiro-control-center/src/lib/bindings.ts`.
CI runs it and verifies that regeneration leaves no diff.

### Frontend

Run frontend commands from `crates/kiro-control-center/`:

- `npm run check` — Svelte and TypeScript checks via `svelte-check`; run after
  any core type change that flows through `bindings.ts`.
- `npm run dev` — Vite on `http://localhost:1420` (Tauri convention, not Vite's
  default 5173).
- `npm run test:unit` — Vitest unit tests.
- `npm run test:e2e` — the Playwright suite under `tests/e2e/*.spec.ts`.
  Fixture-backed cases in `app.spec.ts` gate on `FIXTURE_MARKETPLACE_PATH` and
  call `test.skip` cleanly when it is unset; suites such as `agents.spec.ts`
  create their own temporary project.

Vitest covers extracted pure-logic helpers only: no jsdom, no
`@testing-library/svelte`, and no Tauri IPC module mocks. Component-level
testing is intentionally future scope. If logic in a `.svelte` file or a runes
store's `$state` /`$derived` needs testing, factor it into a non-`.svelte.ts`
module and test the helper.

For helpers that call Tauri commands or runes-based stores, inject dependencies
through the context type instead of `vi.mock` -ing `$lib/bindings` or
`$lib/stores/*.svelte.ts`. Tests construct `vi.fn()` fakes and pass them through
the context. `PluginActionContext` and `PluginRemoveContext`, including the
`installPlugin`, `removePlugin`, and `storeRefresh` dependencies, are the
canonical pattern.

### Lint and Pre-Commit

```bash
cargo clippy --workspace -- -D warnings
```

Before committing, run the stronger local checks:

```bash
cargo fmt --all --check
cargo test --workspace
cargo clippy --workspace --tests -- -D warnings
```

For changes under `crates/kiro-control-center/`, also run:

```bash
cd crates/kiro-control-center && npm run check
cd crates/kiro-control-center && npm run test:unit
```

### Commit Messages

CI validates every commit since `origin/main` against:

<!-- markdownlint-disable MD013 -->
```text
^(feat|fix|docs|style|refactor|perf|test|build|ci|chore|review)(\([a-z][a-z0-9-]*\))?!?: .{3,}
```
<!-- markdownlint-enable MD013 -->

The optional scope must start with a lowercase letter. `fix(2b): ...` fails; use
`fix(ui): ...`, `fix(phase-2b): ...`, or no scope. Existing history on `main` is
not rechecked.

---

## Directory Map

<!-- tags: navigation, structure -->

```text
crates/
├── kiro-market-core/src/       # Shared library — all business logic lives here
│   ├── service/mod.rs          # MarketplaceService: primary orchestrator
│   ├── service/browse.rs       # Enumeration, install context, catalog assembly
│   ├── cache.rs                # CacheDir and marketplace source detection
│   ├── project.rs              # .kiro/ skill, agent, and steering operations
│   ├── git.rs                  # gix-first clone with system-git fallback
│   ├── agent/                  # Claude, Copilot, and Native agent dialects
│   ├── steering/               # Steering discovery and installation
│   ├── plugin.rs               # Plugin discovery and plugin.json parsing
│   ├── skill.rs                # SKILL.md frontmatter parsing
│   ├── validation.rs           # Security-critical path/name newtypes
│   ├── kiro_settings.rs        # Typed .kiro/settings.json registry
│   ├── platform.rs             # Symlink, junction, and copy abstraction
│   ├── file_lock.rs            # Cross-process fs4 locking
│   ├── hash.rs                 # BLAKE3 change detection
│   ├── raii.rs                 # DirCleanupGuard
│   └── error.rs                # Structured error hierarchy
├── kiro-market/src/            # Thin clap CLI
│   ├── cli.rs                  # Clap derive definitions
│   ├── main.rs                 # Command dispatch
│   └── commands/               # One module per subcommand
└── kiro-control-center/        # Tauri 2 + Svelte 5 desktop app
    ├── src-tauri/src/
    │   ├── lib.rs              # Tauri command registration via tauri-specta
    │   └── commands/           # Thin IPC handlers over kiro-market-core
    └── src/
        ├── lib/bindings.ts     # Generated TypeScript bindings — do not edit
        ├── lib/stores/         # Svelte 5 $state module pattern
        └── lib/components/     # BrowseTab, InstalledTab, and other UI
xtask/src/
├── main.rs                     # Hook and xtask dispatch
└── plan_lint.rs                # Tethys-backed structural gates
```

**Key entry points:**

- CLI: `crates/kiro-market/src/main.rs` → `commands/`.
- Desktop: `crates/kiro-control-center/src-tauri/src/lib.rs` → Tauri commands.
- Core: `crates/kiro-market-core/src/service/mod.rs` → `MarketplaceService`.

---

## Architecture

<!-- tags: architecture, patterns -->

The tool reads Claude Code `marketplace.json` catalogs, discovers plugins,
skills, agents, and steering, and installs them into Kiro projects under
`.kiro/`, including skills at `.kiro/skills/`.

### Shared Core and Service Layer

Dependencies point inward. `kiro-market-core` is the domain core and must stay
free of UI, Tauri, async-runtime, and frontend dependencies. The CLI and Tauri
crates depend on core; core never depends on them. If code in core appears to
need `tauri`, `tokio`, or a UI crate, the abstraction belongs in the consumer
crate instead.

Marketplace operations live in `kiro_market_core::service::MarketplaceService`
(the Rust path corresponding to
`kiro-market-core::service::MarketplaceService`). The service stores a boxed
`GitBackend`, while its constructor accepts any `'static` backend so tests can
inject mocks. CLI and Tauri handlers construct the service, call it, and format
output; never duplicate domain logic between frontends. Keep `tauri`, `tokio`,
and UI dependencies out of `kiro-market-core/Cargo.toml`.

### Tauri Command Handlers

A service-consuming `#[tauri::command]` splits into a thin wrapper and a private
`fn <name>_impl(svc: &MarketplaceService, ...) -> Result<T, CommandError>`. The
wrapper handles globals such as `make_service()?`; `_impl(svc, ...)` takes the
service and primitives by reference and is tested directly in a
`#[cfg(test)] mod tests` with `kiro_market_core::service::test_support`.
`install_skills_impl` in
`crates/kiro-control-center/src-tauri/src/commands/browse.rs` is the exemplar.

Project-only commands that do not need `MarketplaceService`, such as
`list_installed_skills` and `remove_skill` in `commands/installed.rs`, keep
their body inline. Do not add an unused service parameter merely to match the
pattern.

### State, Cleanup, and IPC

Registry and install-tracking metadata is JSON on disk and guarded by `fs4` file
locks. Marketplace caches and installed content are filesystem trees; there is
no database.

`DirCleanupGuard` removes staging directories on failure. Call `.defuse()` only
after successful completion.

`tauri-specta` generates typed IPC bindings. Never edit
`crates/kiro-control-center/src/lib/bindings.ts` manually; regenerate it with
the command in [Test](#test). The `bindings_export_plugin_catalog_view` test
also guards the exported catalog shape.

Skill directories are copied wholesale, including `SKILL.md` and `references/`,
so Kiro's lazy loading can resolve companion files.

### Git Abstraction

Git operations are behind `kiro_market_core::git::GitBackend` (the
`kiro-market-core::git` module), implemented in production by `GixCliBackend`.
Cloning tries gix first, cleans up a partial destination on failure, and retries
with system git. If both clone backends fail, both causes are retained.
Requested-ref checkout and pull use system git; repository open and SHA
verification use gix. Recognized CLI authentication failures receive remediation
text.

### Platform Abstraction

Local marketplace linking uses the `kiro_market_core::platform` functions
`create_local_link`, `is_local_link`, and `remove_local_link`. Unix uses
symlinks. Windows uses NTFS
directory junctions and falls back to recursive copy if junction creation fails.
`MarketplaceStorage` records which storage method was used.

### Core Feature Flags

- `cli` — clap derives used by the CLI crate.
- `specta` — TypeScript binding derives used by the Tauri crate.
- `test-support` — test utilities for integration and frontend-wrapper tests.

---

## Non-Obvious Patterns

<!-- tags: patterns, gotchas -->

- **curl is a TLS feature shim:** `kiro-market-core` depends on
  `curl = { workspace = true }` only to activate the `ssl` feature on transitive
  `curl-sys` from `gix-transport`. It is not used for HTTP requests. Removing it
  breaks TLS on static-libcurl builds.
- **Cargo.lock is generated:** the Claude hook blocks direct edits. Use
  `cargo update -p <crate>` instead. `KIRO_ALLOW_LOCKFILE_EDIT=1` is the
  explicit one-session override.
- **Bindings are generated:** never edit `src/lib/bindings.ts` manually; the
  `generate_types` test overwrites it.
- **Svelte 5 runes:** stores export const `$state` objects rather than Svelte 4
  stores. Mutate through the deep state proxy.
- **Plugin references:** always `plugin@marketplace`, split on the first `@`.
- **Default scan paths:** when `plugin.json` is absent or the relevant manifest
  list is empty, discover skills from `./skills/`, agents from `./agents/`, and
  steering from `./steering/`.
- **Self-cycle dev dependency:** `kiro-market-core` lists itself under
  `dev-dependencies` with `features = ["test-support"]`; Cargo handles this
  without recursion.
- **BLAKE3 tracking:** source and installed content hashes are stored. Matching
  hashes make reinstalls idempotent; changed content requires `--force`.
- **chrono and IPC:** `chrono::DateTime<Utc>` cannot appear on `specta::Type`
  IPC structs because the Tauri crate does not enable Specta's chrono feature.
  Convert timestamps to RFC 3339 strings at the FFI boundary. The binding export
  test asserts that generated TypeScript contains no `DateTime` types.

---

## Tooling and Hooks

<!-- tags: automation, hooks, lint -->

### Claude Hooks

Hook registration lives in `.claude/settings.json`.

- **PreToolUse (`Write|Edit|MultiEdit`):** `cargo xtask hook-block-cargo-lock`
  blocks direct `Cargo.lock` edits.
- **PostToolUse (`Write|Edit|MultiEdit`):** `cargo xtask hook-post-edit` gates
  on `.rs` inside `hook_post_edit`, runs rustfmt, derives the nearest package
  with `xtask::derive_package` by walking ancestors for a `[package]`
  `Cargo.toml`, and runs `cargo clippy --package <derived> -- -D warnings`.
  Read/parse failures are reported; non-Rust edits return quickly.
- **Stop:** `cargo xtask hook-stop-frontend-check` dispatches to
  `hook_stop_frontend_check` and runs `npm run check` once per turn only when
  `git status --porcelain` reports dirty `.ts` or `.svelte` files under
  `crates/kiro-control-center/`. Pure-Rust turns pay no frontend-check cost.

The Stop hook emits findings and actionable infrastructure failures in a
`{"systemMessage": ...}` JSON envelope. It exits 0 so an infrastructure problem
does not abort the turn. `is_frontend_path` and
`parse_dirty_paths_from_git_status` keep dirty-path classification pure and
unit-testable. npm invocation handles Windows's `.cmd` shim via
`#[cfg(windows)]`, and git uses `-c core.quotePath=false` so non-ASCII paths are
not silently missed.

The PostToolUse and Stop hooks call `resolve_workspace_dir`
/`resolve_workspace_dir_inner` and resolve the workspace from the tool payload's
`cwd`, then `$CLAUDE_PROJECT_DIR`, then `current_dir`; failure of all three is
reported instead of silently using `Path::new(".")`. This keeps checks in an
isolated worktree rather than the parent checkout. `frontend_files_dirty` and
`classify_git_status_failure` treat benign failures such as “not a git
repository” as skips, while permissions, corruption, or lock contention produce
a system message.

### Plan Lint

`cargo xtask plan-lint` runs structural SQL queries against a
[tethys](https://github.com/dwalleck/rivets/tree/main/crates/tethys) index. The
registered gates are:

- `gate-4-external-error-boundary` — external crate error type behind
  `#[source]` on a public enum or struct.
- `no-unwrap-in-production` — `.unwrap()` or `.expect()` in non-test production
  code.
- `no-panic-in-production` — `panic!`, `todo!`, or `unimplemented!` in non-test
  production code.
- `non-exhaustive-error-enum` — public `*Error` enum in core missing
  `#[non_exhaustive]`.
- `no-frontend-deps-in-core` — `tauri` or `tokio` import in core.
- `ffi-enum-serde-tag` — public `Serialize + specta::Type` enum with payload
  variants missing an explicit serde representation.
- `no-marketplace-service-in-agents-authoring` — service imports in project-only
  agent-authoring commands.

Run one policy directly with commands such as
`cargo xtask plan-lint --gate no-unwrap-in-production` or
`cargo xtask plan-lint --gate gate-4-external-error-boundary`. `tethys` must be
on `PATH`, or set `TETHYS_BIN`. Use `--no-reindex` to query the existing
`.rivets/index/tethys.db`, or `--gate <NAME>` to run one gate. Index canaries
fail loudly when the index is empty or incomplete so gates cannot report a
vacuous success. The external-boundary query uses indexed `attributes` and
`symbols`. The unwrap/panic queries use indexed `refs`, `symbols`, and `files`,
require `is_test = 0`, and exempt `tests/`, `benches/`, `test_support`, and
`test_utils`. Findings exit 1. Plan-lint is currently a local/review gate unless
CI is explicitly updated to invoke it.

### Comment Lint

`cargo xtask comment-lint` scans `//` comments under `crates/` and `xtask/` for:

- four-character `kiro-XXXX` Rivets IDs;
- case-insensitive `PR #N` /`pr#N` (with or without the space) or `issue #N`
  references;
- reviewer-agent attribution names;
- process references such as bare `amendment` or `per A<digits>`.

Reviewer names include `code-reviewer`, `silent-failure-hunter`,
`comment-analyzer`, `pr-test-analyzer`, `type-design-analyzer`,
`code-simplifier`, `marketplace-security-reviewer`, `tauri-ipc-auditor`,
`plugin-validator`, `skill-reviewer`, and `gemini-code-assist` (see
`REVIEWER_AGENT_NAMES`).

Matching is comment-scoped, case-aware where appropriate, and word-boundary
aware; block comments and string literals are skipped. Embedded strings such as
`mykiro-uphh`, `kiro-uphhx`, `kiro-code-reviewer-v2`, and `preamendment` do not
match. `xtask/src/comment_lint.rs` self-skips because it must document the
patterns, and `xtask/src/plan_lint.rs` may contain originating rationale.
Deliberate exceptions belong in `comment_lint::ALLOWED_SITES` with a written
reason such as `LEGACY_BASELINE_REASON` or `FIXTURE_NAME_REASON`. Findings exit

1. Comment-lint is currently a local/review gate unless CI is explicitly updated
to invoke it.

Committed comments, including `///` and `//!` doc comments, tests, and
frontmatter under `crates/` and `xtask/`, must describe the invariant rather
than its historical PR, issue, reviewer, or plan-amendment origin. PR
descriptions, commit messages, and `git blame` are the durable audit trail;
`.agents-view/plan-slice-N.md` is not a durable in-code reference. Rationale
comments remain valuable: write “refusing hardlinked source — exfiltration
risk,” not “closes kiro-uphh from PR #119 review,” “per silent-failure-hunter
HIGH #2,” or “per A1 amendment.” Describe the behavior directly, for example:
“addExternalTool only touches tools[] — visibility and auto-allow are
orthogonal.”

---

## Work Tracking (Rivets)

The deferred-work backlog is `.rivets/issues.jsonl`, committed to git and
managed by the [`rivets`](https://github.com/dwalleck/rivets) CLI. The binary
must be on `PATH`; the sibling repository is `~/repos/rivets`. The issue prefix
is `kiro-`, configured once with `rivets init -p kiro` in `.rivets/config.yaml`;
do not change it. The tracker was initialized in commit `5e85b6e` with 17 issues
seeded from PR #113–#115 retrospectives.

### Check Before You Build

Before any non-trivial change, run:

```bash
rivets ready
rivets list --status open
```

Work may already be filed with design notes, acceptance criteria, and
dependencies. Use `rivets show <id>` before duplicating it in a new change.

### Daily Commands

- `rivets ready` — unblocked issues, hybrid priority/age order, default limit
  10.
- `rivets list --status open` — open backlog; combine with `--label <name>`,
  `--type bug`, or `-p <0-4>`.
- `rivets show <id> [<id>...]` — full issues, dependencies, design, and
  acceptance criteria.
- `rivets stats` — status totals and tracker reachability.
- `rivets blocked` — issues waiting on dependencies.
- `rivets update <id> --status in_progress` — claim work when starting.
- `rivets close <id> --reason "PR #NNN: <one-line summary>"` — close on merge.
- `rivets dep tree <id>` — dependency graph.
- `rivets list --json | jq ...` — JSON scripting; all subcommands support
  `--json`.

### Creating and Editing Issues

When review surfaces follow-up work, file a Rivets issue rather than leaving it
in a stale comment. Match the seeded style: `--description` names the
originating PR, paths, and finding; `--design` records the approach;
`--acceptance` records success criteria; and `--deps` records blockers. Use the
`blocks` relationship for changes that must happen together, not only strict
predecessor ordering; for example, `kiro-5qcb` cleanup blocks on its `kiro-kmj4`
enabler so `rivets ready` hides it until the enabler closes.

Rivets creates the four-character suffix at issue creation. If an earlier
description uses `kiro-<this-id>` or `kiro-CONTINGENT`, create the referenced
issue, immediately reread the original issue, and replace every placeholder with
the real ID. Commit `e7e3d9b` exists because this backfill was previously
missed.

Edit `.rivets/issues.jsonl` directly only for corrections that intentionally
must not change `updated_at`, such as placeholder backfills. Semantic changes to
status, priority, description, dependencies, or labels must use `rivets update`
so timestamps and `rivets stale` remain meaningful.

`.rivets/` is shared: Rivets writes tracked `config.yaml` and `issues.jsonl`;
tethys writes the untracked, regenerable `.rivets/index/`. The root ignore rule
must remain exactly `/.rivets/index/`; broadening it to `/.rivets/` would
untrack the backlog.

---

## Planning

After writing a plan and before implementation, apply all six gates in
`docs/plan-review-checklist.md`:

1. Grounding
2. Threat Model
3. Wire Format
4. External Type Boundary
5. Type Design
6. Reference vs. Transcription

These also apply as review questions to changes touching the public API of
`kiro-market-core`. They complement rather than replace
`superpowers:writing-plans`: invoke that skill first, then apply the six gates
before calling the plan implementation-ready. Gate 6 prevents plans from
transcribing observed output when they should cite and preserve the underlying
mechanism.

Plan review requires two complementary passes:

1. **LSP-first:** run `documentSymbol` for every modified file and
   `workspaceSymbol` for cross-file references. This catches signature drift,
   missing exports, and field-access mistakes cheaply.
2. **Behavioral review:** ask whether every task does the right thing. Check
   cascade behavior, recoverable-versus-fatal classification, sibling
   occurrences of the same pattern, linkage from design actions to tasks, and
   whether map keys/data shapes contain enough information.

Neither pass substitutes for the other. A prior review found 11 LSP-first issues
and 12 behavioral issues with almost no overlap. Stop adding plan-time
amendments when findings have diminished to compiler-catchable details such as
`DateTime<Utc>` versus Specta features or `self.` versus `Self::`; let
implementation checks handle the remainder.

---

## Worktree Convention

Feature worktrees live in sibling directories such as
`~/repos/kiro-control-center-<topic>`. Each worktree has its own `target/`,
`node_modules/`, and `.rivets/index/`. After creating one, run `npm install` in
`crates/kiro-control-center/`; the parent worktree's Tethys index does not
transfer.

```bash
git worktree add ~/repos/kiro-control-center-<topic> -b <branch> origin/main
```

After merge:

```bash
git worktree remove <path>
git branch -d <branch>
git pull --ff-only
```

---

## Code Style

<!-- tags: rust, typescript, errors, validation -->

- Rust edition 2024; minimum Rust version 1.85.0.
- `thiserror` for typed errors in `kiro-market-core`; `anyhow` for propagation
  in the `kiro-market` binary.
- `rstest` for parameterized tests and `tempfile` for filesystem fixtures.
- Workspace lints set `clippy::all = "warn"` and `clippy::pedantic = "warn"`.
- Workspace `unsafe_code = "forbid"`.

### Model Invalid States Out

Prefer models that make invalid states unrepresentable instead of permitting
bad combinations and validating them later.

- Use distinct newtypes for values that share a representation but not a
  meaning, so IDs, names, paths, and references cannot be accidentally mixed.
- Use enums or other sum types for mutually exclusive states. Each variant
  should carry only the fields valid in that state; avoid independent booleans
  or a bag of `Option` fields that permits contradictory combinations.
- Keep invariant-bearing fields private and expose fallible constructors or
  transition methods. Once constructed, a domain value should remain valid.
- Parse untrusted input into invariant-bearing types at the boundary. A
  permissive projection type is acceptable only when needed to classify parse
  failures; convert it immediately into the stricter domain model.
- When the type system cannot express an invariant alone, enforce it at the
  nearest durable boundary and cover that constraint with tests. Do not rely
  only on comments or downstream application checks.

### Error Chains and Boundaries

Use `#[source]` on inner typed-error variants (for example,
`PluginError::ManifestReadFailed { #[source] source: io::Error }`) and
`#[error(transparent)]` on top-level wrappers so `Error::source()` traverses the
chain. At Tauri/log boundaries, and for any wire-format `reason` or
`error: String`, use `error_full_chain(&err)` rather than `err.to_string()`.
`SkippedPlugin::from_plugin_error` and `FailedSkill::install_failed` are
canonical constructors.

Do not name a non-`Error` payload field `source` in a `thiserror` variant; that
name is reserved for `Error::source()`. Rename it, for example to
`plugin_source: StructuredSource`, and preserve the external wire name in a
projection such as `SkippedReason`.

Prefer dedicated variants over `reason: String` sentinels when callers may
branch on the semantic, such as `NotADirectory` and `SymlinkRefused`, rather
than `DirectoryUnreadable { reason }`. `io::Error` is `Send + Sync + 'static`
and can be stored directly behind `#[source]`; no `Box` is required.

Map external errors at the adapter boundary. Translate `gix`, `serde_json`,
`toml`, and `reqwest` failures into core-owned error types and variants, such as
`PluginError` or `NativeParseFailure`, in modules such as `git.rs`, `cache.rs`,
and `agent/parse_native.rs`; external crate error types must not appear in the
public API of `kiro-market-core`. `io::Error` is the standard-library exception.
When an external error would otherwise be a source field, use a `pub(crate) fn`
constructor that renders `error_full_chain` into a `reason: String`, and test
that `err.source().is_none()` locks the boundary contract.
`parse_native::NativeParseFailure::invalid_json` and
`steering::tracking_malformed` producing `SteeringError::TrackingMalformed` are
canonical mappings for `serde_json::Error`.

### Validation and IPC Types

Parse, do not merely validate, at deserialization boundaries. Wrap untrusted
manifest strings in newtypes with private fields and fallible constructors;
implement `Deserialize` through the constructor so `serde_json::from_slice`
fails at parse time. `RelativePath` (`validation.rs`), `GitRef` (`git.rs`), and
`AgentName` are templates. A free `validate_xyz(&Thing) -> Result<()>` that no
constructor invokes usually signals a missed newtype.

A transient projection may retain raw `Option<String>` only when post-parse
routing must distinguish errors. For example, `NativeAgentProjection.name`
permits separate `NativeParseFailure::{MissingName, InvalidName, InvalidJson}`
outcomes, while `NativeAgentBundle.name: AgentName` restores the type-level
invariant. `InvalidJson` stores a rendered `reason: String`, not the external
`serde_json::Error`.

Validation newtypes that can cross Tauri need
`#[cfg_attr(feature = "specta", derive(specta::Type))]`, matching
`RelativePath`. Core's Specta feature list is `["derive", "serde_json"]` and
intentionally omits `"chrono"`; convert `DateTime<Utc>` to `.to_rfc3339()` at
the FFI boundary, as `commands/installed.rs::InstalledSkillInfo.installed_at`
does.

### Exhaustiveness and Classifiers

Classifier functions such as `SkippedReason::from_plugin_error` and
`PluginError::remediation_hint` must enumerate every `PluginError` variant
explicitly; do not use `_ => None` or `_ => default`. A new error variant should
force a compile-time classification decision.

When `classify_*_collision` returns `CollisionDecision::Idempotent(Box<T>)`, `T`
may contain only data visible to the classifier. Do not use a full type such as
`InstalledSteeringOutcome` when the classifier cannot populate fields like
`source.source`; return a minimal echo such as
`SteeringIdempotentEcho { prior_installed_hash: String }` and let the caller
assemble the complete outcome.

For TypeScript discriminated unions such as `mode.kind` or `result.kind`, use an
exhaustive `switch (x.kind)` rather than `x.kind === "A" ? ... : ...` chains.
Include `default: { const _exhaustive: never = x; throw new Error(...); }`.
`formatSkippedSkill`, `formatSteeringWarning`, and `runPluginInstall` are
exemplars. At the definition site, pair a `satisfies` -anchored values list with
`Exclude<U, (typeof _VALUES)[number]> extends never ? true : never`, as
`_PLUGIN_ACTION_VALUES` and `_AssertPluginActionExhaustive` do. Instantiate the
check in value position (`const _assert: T = true`); an unused alias resolving
to `never` does not fail compilation.

### Production Failure Policy

Production code must not use `.unwrap()`, `.expect()`, `panic!`, `todo!`,
`unimplemented!`, `let _ = ...` to discard a `Result`, or inline `#[allow(...)]`
to hide a finding. Tests are exempt where appropriate. Fix the code or lint
configuration rather than suppressing locally. Deliberate framework-level
exceptions belong in audited `ALLOWED_SITES` constants with written reasons.

Automation covers part of this policy: plan-lint detects unwrap/expect and
selected panic macros. Discarded Results and arbitrary inline lint suppressions
remain compiler/review checks; do not claim the automated gates cover them.

---

## CI

<!-- tags: ci, automation -->

All required jobs are defined in `.github/workflows/ci.yml`:

- `commitlint` — validates the commit format above.
- `format` — `cargo fmt --all -- --check`.
- `lint` — `cargo clippy --workspace -- -D warnings`.
- `test` — full workspace on Linux and core + CLI on macOS/Windows; Linux also
  regenerates bindings and requires a clean diff.
- `frontend` — `npm run test:unit`, `npm run check`, and `npm run build`.
- `build-cli` — release CLI builds on Linux, macOS, and Windows.
- `build-tauri` — desktop builds on Linux, macOS, and Windows.
- `cargo-deny` — license and advisory policy.
- `assert-curl-tls` — confirms `curl-sys` has SSL enabled.
- `coverage` — `cargo-llvm-cov` uploaded to Codecov.
- `ci-success` — aggregate required gate.

Release artifacts include CLI binaries plus platform Tauri packages such as deb,
dmg, and msi outputs.

---

## Security Invariants

<!-- tags: security, constraints -->

These must hold in every change:

1. **Path traversal prevention:** user-supplied names must pass
   `validate_name()` and paths must pass `validate_relative_path()` or enter
   validated newtypes such as `RelativePath` before any filesystem boundary.
2. **MCP opt-in:** agents with MCP servers require `--accept-mcp`; never
   auto-install an MCP-bearing agent.
3. **TLS by default:** reject `http://` marketplace sources unless
   `--allow-insecure-http` is explicit.
4. **Link rejection during copy:** `copy_dir_recursive` skips symlinks on all
   supported platforms and hardlinked files on Unix. Windows-specific copy paths
   reject reparse points. Do not weaken these checks.
5. **No unsafe code:** workspace-level `unsafe_code = "forbid"`.

---

## Key Crate Dependencies

- `gix` plus system `git` — repository operations and fallback behavior.
- `clap` derive — CLI parsing.
- `serde`, `serde_json`, and `serde_yaml_ng` — serialization and JSON/YAML
  parsing.
- `thiserror` and `anyhow` — typed library errors and binary-level propagation.
- `colored` — terminal output.
- `dirs` — XDG and platform directory resolution.
- `fs4` — cross-process file locking.
- `blake3` — content-change detection.
- `tauri`, `tauri-specta`, and `specta` — desktop IPC and generated TypeScript
  types.
- `rstest` and `tempfile` — tests and fixtures.

---

## Custom Instructions

<!-- This section is for human and agent-maintained operational knowledge.
     Add repo-specific conventions, gotchas, and workflow rules here.
     This section is preserved exactly as-is when re-running
     codebase-summary. -->

The Svelte MCP server provides comprehensive Svelte 5 and SvelteKit
documentation. Use it as follows.

### Available Svelte MCP Tools

#### 1. `list-sections`

Use this first for any Svelte or SvelteKit task. It returns documentation
titles, use cases, and paths.

#### 2. `get-documentation`

After `list-sections`, inspect the reported use cases and fetch every
documentation section relevant to the task. It accepts one or multiple sections.

#### 3. `svelte-autofixer`

Whenever writing Svelte code, run `svelte-autofixer` before presenting or
finalizing it. Repeat until it returns no issues or suggestions.

#### 4. `playground-link`

After completing standalone example code, ask whether the user wants a Svelte
Playground link. Call this tool only after confirmation, and never when the code
was written into the user's project files.

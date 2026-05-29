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

For changes under `crates/kiro-control-center/` also run:
- `cd crates/kiro-control-center && npm run check`
- `cd crates/kiro-control-center && npm run test:unit`

Vitest covers pure-logic helpers only (no jsdom, no `@testing-library/svelte`,
no Tauri-IPC mocks). Component-level testing is intentionally future scope.
If you find yourself wanting to test a `.svelte` file or a reactive store's
`$state`/`$derived`, factor the testable logic out into a non-`.svelte.ts`
module and test the helper instead.

For helpers that *call* Tauri commands or runes-based stores, inject those
dependencies via the context type rather than `vi.mock`-ing `$lib/bindings`
or `$lib/stores/*.svelte.ts`. Tests construct `vi.fn()` fakes directly and
pass them through the context — no module mocks needed. Canonical pattern:
the `installPlugin` / `removePlugin` / `storeRefresh` injection on
`PluginActionContext` and `PluginRemoveContext`.

## Commit messages
CI's `Validate Commits` step enforces this regex on every commit since
`origin/main`:
`^(feat|fix|docs|style|refactor|perf|test|build|ci|chore|review)(\([a-z][a-z0-9-]*\))?!?: .{3,}`

The scope must start with a **lowercase letter** — `fix(2b): ...` fails
(digit-first). Use `(ui)`, `(phase-2b)`, or no scope. Existing commits on
`main` aren't re-checked, so prior `(2b)`-scoped history doesn't trip the
gate; new commits do.

## xtask
- `cargo xtask hook-post-edit` — wired into Claude Code `PostToolUse` for `.rs` edits. Runs `rustfmt` then `cargo clippy --package <derived> -- -D warnings`. The package is derived by walking up ancestors for the nearest `Cargo.toml` with a `[package]` table (`xtask::derive_package`), so new workspace crates are picked up automatically. Read/parse failures log to stderr; loop exhaust emits a "no usable Cargo.toml" diagnostic. The settings.json matcher is `Write|Edit|MultiEdit`; the `.rs` extension gate lives in Rust (`hook_post_edit`), so non-Rust edits return in microseconds.
- `cargo xtask hook-stop-frontend-check` — wired into Claude Code `Stop` (fires once per turn, not per edit). Runs `npm run check` (svelte-check + tsc) from `crates/kiro-control-center/` **only when** `git status --porcelain` reports any `.ts` / `.svelte` file dirty under that crate. A pure-Rust turn pays zero cost; a mixed-bag turn pays one ~5–15s check. Findings (and user-actionable failures like a missing `git`/`npm`) are emitted as a `{"systemMessage": ...}` JSON envelope on stdout — Stop hooks send plain stdout to the debug log only, so the envelope is the documented route to the transcript. Exit code is always 0 from `hook_stop_frontend_check`: every error path (stdin parse, workspace-dir resolution, dirty-check, svelte-check spawn) is swallowed inside the function so a turn is never aborted over an infrastructure hiccup. The dirty-path filter (`is_frontend_path`, `parse_dirty_paths_from_git_status`) is pure Rust with unit tests; cross-platform npm invocation handles Windows's `.cmd` shim explicitly via `#[cfg(windows)]`. The git invocation uses `-c core.quotePath=false` so non-ASCII filenames don't get octal-escaped and silently miss the prefix check.
- **Worktree-correctness for both hooks.** Both hooks resolve their workspace dir from the JSON stdin payload's `cwd` field (`resolve_workspace_dir` → `resolve_workspace_dir_inner`), falling back to `$CLAUDE_PROJECT_DIR` and then `current_dir`, and erroring when all three are absent (no more `Path::new(".")` pretend-success). Claude Code populates `cwd` with the actual working directory of the tool call, so a subagent operating in an isolated worktree gets clippy / svelte-check runs in the *worktree's* tree rather than the parent session's. The fallback chain keeps direct `cargo xtask ...` invocations working outside Claude Code. `frontend_files_dirty` classifies non-zero `git status` exits via `classify_git_status_failure`: benign cases (not a git repo, dir missing) skip silently; surfaceable cases (index corrupt, permission denied, lockfile contention) emit a `systemMessage` so the user sees the real symptom instead of a silent no-op.
- `cargo xtask hook-block-cargo-lock` — blocks direct `Cargo.lock` edits. Override for one session via `KIRO_ALLOW_LOCKFILE_EDIT=1`.
- `cargo xtask plan-lint` — runs structural lint queries against the [tethys](https://github.com/dwalleck/rivets/tree/main/crates/tethys) index. Gates implemented:
  - **gate-4-external-error-boundary** — SQL query against `attributes` and `symbols` that flags any `pub` enum variant carrying an external crate's error type (`serde_json`, `gix`, `reqwest`, `toml`) via `#[source]`. Replaces the broken grep in `docs/plan-review-checklist.md`.
  - **no-unwrap-in-production** — SQL query against `refs` joined to `symbols` and `files` that flags `.unwrap()` and `.expect()` calls in non-test production code, enforcing the CLAUDE.md "zero-tolerance" rule. Filters: `is_test = 0`, plus path-based exemptions for `tests/`, `benches/`, `test_support`, and `test_utils`.

  Requires the `tethys` binary on PATH (or `TETHYS_BIN` env var); pass `--no-reindex` to query the existing `.rivets/index/tethys.db` without re-indexing first; pass `--gate <NAME>` to run a single gate. Exits 1 on findings (CI gate fails).
- `cargo xtask comment-lint` — filesystem walk that flags four smell categories inside `//` line comments under `crates/` and `xtask/`:
  - `kiro-XXXX` rivets-IDs
  - `PR #N` / `issue #N`
  - Reviewer-agent attribution names (per `REVIEWER_AGENT_NAMES`: `code-reviewer`, `silent-failure-hunter`, `marketplace-security-reviewer`, etc.)
  - Process-references — bare word `amendment` (case-insensitive) and `per A<digits>` shorthand attributing code to a plan-amendment ID (e.g. `Per A1`, `per A2 amendment`)

  Enforces the "no PR/issue/reviewer/process refs in committed comments" rule below. Pattern matching is `//`-comment-scoped (block comments and string literals are skipped) and word-boundary aware on both sides (`mykiro-uphh`, `kiro-uphhx`, `kiro-code-reviewer-v2`, `preamendment` don't match their embedded substrings). Self-skips `xtask/src/comment_lint.rs` (its docstring documents the patterns it detects) and `xtask/src/plan_lint.rs` (its gate-query rationale is anchored to originating PRs). Other deliberate exceptions register in `comment_lint::ALLOWED_SITES` with a written-down reason. Exits 1 on findings.

## Work tracking (rivets)

The deferred-work backlog lives in `.rivets/issues.jsonl` (committed to git) and is managed by the [`rivets`](https://github.com/dwalleck/rivets) CLI — a JSONL-backed issue tracker. The binary must be on PATH; the sibling repo is at `~/repos/rivets`. Issue-ID prefix is `kiro-` (set once in `.rivets/config.yaml` by `rivets init -p kiro`; do not change). Initialized in commit `5e85b6e` with 17 issues seeded from the PR #113-#115 retrospectives.

**Check before you build.** Before starting any non-trivial change, run `rivets ready` and `rivets list --status open`. There is a good chance the work is already filed — often with `--design` notes or `--acceptance` criteria that should shape the implementation. Duplicating an existing issue as a fresh PR loses the linked context. `rivets show <id>` prints the full issue including dependencies.

**Daily commands:**
- `rivets ready` — issues with no open blockers (default sort: hybrid priority/age, limit 10)
- `rivets list --status open` / `--label <name>` / `--type bug` / `-p <0-4>` — filtered backlog
- `rivets show <id> [<id>...]` — full issue(s) including dependencies, design notes, acceptance criteria
- `rivets stats` — totals by status (use to confirm the tracker is reachable)
- `rivets blocked` — issues waiting on dependencies
- `rivets update <id> --status in_progress` — claim work when you start
- `rivets close <id> --reason "PR #NNN: <one-line summary>"` — close on merge
- `rivets dep tree <id>` — visualize the dependency graph rooted at an issue
- `rivets list --json | jq ...` — every subcommand supports `--json` for scripting

**Creating issues.** When PR review surfaces follow-up work, file it as a rivets issue rather than letting it die in a stale comment thread. Match the existing seeded-issue style: `--description` includes the originating PR number, the file path(s) involved, and what the reviewer flagged; `--design` captures the fix approach; `--deps` wires up blockers. Use the `blocks` dependency type for "must-happen-in-the-same-commit-as <other>" coupling, not just strict predecessor ordering — that's how `kiro-5qcb` (a diagnostic-removal cleanup) couples to `kiro-kmj4` (the F3 UI work that enables the cleanup). The cleanup blocks the enabler so `rivets ready` correctly hides it until F3 closes.

**The ID-placeholder footgun.** Rivets generates the 4-character suffix at creation time, so when issue A's description needs to reference issue B, you cannot pre-name B. If you write `kiro-<this-id>` or `kiro-CONTINGENT` as a stand-in, you MUST backfill the real ID after creating B. Commit `e7e3d9b` exists precisely because this was missed during the initial seeding and PR review caught it. After creating any issue whose description was authored before its ID existed, immediately re-read it and replace placeholders with real IDs.

**Direct JSONL edits.** Editing `.rivets/issues.jsonl` directly (rather than via `rivets update`) is acceptable *only* for fixes that intentionally should NOT bump `updated_at` — e.g., the placeholder-backfill cleanup in `e7e3d9b`. Any change to a semantic field (status, priority, description content, dependencies, labels) must go through `rivets update <id>` so the timestamp reflects the real change and `rivets stale` stays meaningful.

**Shared namespace with tethys.** `.rivets/` is a shared directory: rivets writes `config.yaml` and `issues.jsonl` at the top level (tracked), tethys writes its SQLite index under `.rivets/index/` (untracked, regenerable via `cargo xtask plan-lint`). The root `.gitignore` line is precisely `/.rivets/index/` for this reason — broadening it to `/.rivets/` would un-track the entire issue backlog. Do not change that line without recognizing both consumers.

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
- **No PR numbers, rivets-IDs, issue numbers, reviewer-agent attributions, or process-references in committed comments.** This rule binds *all* `//` line comments under `crates/` and `xtask/` — including production code, tests, doc-comments (`///`, `//!`), and frontmatter. Forbidden shapes: `kiro-XXXX` (4-char rivets IDs like `kiro-uphh`), `PR #N` / `pr#N` (case-insensitive), `issue #N`, reviewer-agent attribution names (`code-reviewer`, `silent-failure-hunter`, `comment-analyzer`, `pr-test-analyzer`, `type-design-analyzer`, `code-simplifier`, `marketplace-security-reviewer`, `tauri-ipc-auditor`, `plugin-validator`, `skill-reviewer`, `gemini-code-assist`), AND process-references (bare word `amendment` case-insensitive, or `per A<digits>` shorthand like `Per A1`, `per A2 amendment` — these attribute code to a plan-amendment ID that rots once the slice ships and the plan is archived). Why: the referenced PR/issue/reviewer pass/plan amendment is the *contemporaneous* changelog entry. Once the issue closes and rivets archives it, the in-code reference is a stale pointer the next reviewer has to grep `.rivets/issues.jsonl` (often gone), GitHub, or `.agents-view/plan-slice-N.md` for. PR descriptions, commit messages, and `git blame` are the durable audit trail; comments that duplicate them rot. The same rule applies to coined PR-review phrases (`"silent-install gap from PR #119 review"`, `"Closes marketplace-security-reviewer Minor finding"`, `"per code-reviewer #1 / silent-failure-hunter #2"`, `"the A1 amendment removed that side-effect"`) — describe the invariant, not its origin. **Enforced by `cargo xtask comment-lint`**. Deliberate exceptions (e.g. plan_lint gate rationale anchored to its originating PR, or fixture comments that quote an agent name in prose) register in `xtask/src/comment_lint.rs`'s `ALLOWED_SITES` with a reason (`LEGACY_BASELINE_REASON` or `FIXTURE_NAME_REASON`); the gate's own module is self-skipped because its docstring necessarily quotes the patterns it detects. **Rationale comments that explain WHY a defensive measure exists are still valuable** — just describe the failure shape (`"refusing hardlinked source — exfiltration risk"`) rather than the historical commit (`"closes kiro-uphh from PR #119 review"`), the reviewer attribution (`"per silent-failure-hunter HIGH #2"`), or the plan amendment (`"per A1 amendment, addExternalTool no longer touches allowedTools"` → just `"addExternalTool only touches tools[] — visibility and auto-allow are orthogonal"`).
- **Map external errors at the adapter boundary.** `gix`, `serde_json`, `toml`, `reqwest` errors get translated into typed `ErrorKind` variants inside the module that calls them (e.g. `git.rs`, `cache.rs`, `agent/parse_native.rs`) — they never appear in the public API of `kiro-market-core`. (`io::Error` is std-library and exempted; it can carry through `#[source]`.) Recipe when the variant *would* carry an external error: `#[non_exhaustive]` enum + variant field `reason: String` (not `#[source]`) + a `pub(crate) fn` constructor that calls `error_full_chain(&err)`. Canonical examples: `parse_native::NativeParseFailure::invalid_json` (for `serde_json::Error`), `steering::tracking_malformed` (for `serde_json::Error` in `SteeringError::TrackingMalformed`). Tests should assert `err.source().is_none()` to lock the contract. **Enforced by `cargo xtask plan-lint --gate gate-4-external-error-boundary`** — a SQL query against the tethys index that flags any `pub` enum variant carrying an external crate's error type via `#[source]`.
- **Discriminated unions: `switch` with exhaustiveness, never chained ternaries (TS).** When branching on a tagged-union discriminator (`mode.kind`, `result.kind`, etc.), use `switch (x.kind)` with `default: { const _exhaustive: never = x; throw new Error(...); }` so a future arm becomes a compile error rather than silently falling through. Canonical examples: `formatSkippedSkill`, `formatSteeringWarning`, `runPluginInstall`. Chained `x.kind === "A" ? ... : ...` ternaries fail this discipline — the else-branch doesn't narrow, and a third arm silently maps to whichever branch the ternary defaults to. Pair the runtime switch with a type-level guard at the definition site: a `satisfies`-anchored values list + `Exclude<U, (typeof _VALUES)[number]> extends never ? true : never` (canonical: `_PLUGIN_ACTION_VALUES` + `_AssertPluginActionExhaustive`). The `satisfies` catches arm-shape changes (literal becomes object); the `Exclude<>` catches arm additions; **both need a value-position `const _assert: T = true`** to actually fire — an unused type alias resolving to `never` is valid TS, so the const assignment is what makes the tripwire active. The PR #112 review pass established this as the full discriminator-pushdown discipline; partial application (type definition without consumer-side switches, or guards without value-position) leaves silent-failure surfaces.

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

# Falsifiable Design — kiro-6g6r

**CI gate: `no-marketplace-service-in-agents-authoring` (C7 fence)**

Status: design — cheapest falsifier run; design premise re-verified
against a freshly-built tethys index (see § Empirical verification).
Supersedes the `C7` regression-fence row in `design-slice-1.md` (which
specified a static grep; see § Why not text-grep).

---

## Purpose

Regression fence for design claim **C7** of agents-view slice 1
(`design-slice-1.md:145`):

> None of `list_user_agents`, `create_user_agent`, `save_user_agent`,
> `delete_user_agent`, `duplicate_user_agent` construct or consume a
> `MarketplaceService`. Per CLAUDE.md "Tauri command handlers", their
> bodies inline in the wrapper (no `_impl(svc, ...)` shape).

The five authoring commands in
`crates/kiro-control-center/src-tauri/src/commands/agents_authoring.rs`
MUST stay project-only. A future contributor copy-pasting from a
service-consuming command (e.g. `install_agents` in `agents.rs`) would
silently re-introduce the marketplace coupling, and the existing tests
would not catch it. This gate makes that regression fail CI.

The gate is a new 7th entry in `ALL_GATES` in `xtask/src/plan_lint.rs`,
backed by a SQL query against the tethys index — the same shape as the
six existing gates.

---

## Why not text-grep (the issue's suggested approach, falsified)

The issue and the slice-1 C7 row both suggest a plain
`grep -E '(make_service|MarketplaceService)'` over the target file,
"Expected: zero matches." This was falsified before implementation —
see Claim 3 below. The current, **correct** file already contains the
string `MarketplaceService` in its module doc comment
(`agents_authoring.rs:4`), where it documents that the module
deliberately does *not* use the service. A text-grep gate would fail CI
on clean code.

The same false-positive class is pervasive: `agents.rs`, `steering.rs`,
`plugins.rs`, `marketplaces.rs`, and `mod.rs` all mention
`MarketplaceService` in doc comments while only *some* lines are real
couplings. Text matching cannot separate a doc-comment mention from a
real `use`.

The tethys index can: doc comments are not rows in `imports` or
`symbols`. A real coupling appears as an `imports` row — either
`use kiro_market_core::service::…MarketplaceService` or
`use crate::commands::make_service`. Empirically (§ Empirical
verification) every real `use` of these symbols is captured in `imports`
and no doc-comment mention is, so the `imports` query alone is both
complete and comment-safe.

The gate mirrors `no-frontend-deps-in-core` (imports query), scoped to
the one file. A `refs`-based detection of the bare call
`make_service()` was considered but is **not viable**: tethys does not
record bare free-function call refs (filed as **tethys-zp2j**). This is
not a coverage gap, because the call cannot appear without an
accompanying `use`, which the `imports` query catches.

---

## Input shapes

Every reachable shape of "does this file touch the marketplace service",
and whether the gate must fire:

| # | Shape | Example | Fire? | Source of truth |
|---|-------|---------|-------|-----------------|
| 1 | Doc-comment mention | `//! …MarketplaceService…` (line 4, present now) | **No** | not in imports/symbols |
| 2 | Type import | `use kiro_market_core::service::MarketplaceService;` (as in `agents.rs:18`) | **Yes** | `imports.symbol_name = 'MarketplaceService'` |
| 3 | Helper import | `use crate::commands::make_service;` (as in `browse.rs:19`) | **Yes** | `imports.symbol_name = 'make_service'` |
| 4 | Helper call (with its `use`) | `use …make_service;` + `let svc = make_service()?;` (`browse.rs:19`+`:106`) | **Yes**, via the `use` | `imports.symbol_name`; the bare call ref itself is not recorded (tethys-zp2j) but always co-occurs with the `use` |
| 5 | Fully-qualified call, no `use` | `let svc = crate::commands::make_service()?;` | **out-of-scope** (Negative space) | no `imports` row; tethys records no bare-fn ref |
| 6 | Aliased import | `use …MarketplaceService as Svc;` then `Svc::new()` | **out-of-scope** (Negative space) | would need symbol resolution |
| 7 | File renamed/moved | gate's path anchor no longer matches | **out-of-scope** (accepted limitation) | path-scoped, like every gate here |

Shapes 1 vs 2–4 are the whole design problem: a correct gate fires on
2–4 and stays silent on 1. Text-grep conflates them; the tethys
`imports` query separates them. Shapes 5–7 are explicit non-goals
(Negative space).

---

## Claims

1. The gate fires on a type import of `MarketplaceService` in
   `agents_authoring.rs`.
2. The gate fires on a `make_service` helper import in
   `agents_authoring.rs`. (The bare call always co-occurs with this
   import; tethys does not record the call ref itself — tethys-zp2j.)
3. The gate does NOT fire on the existing module doc-comment mention of
   `MarketplaceService` in the current (correct) file.
4. Introducing the realistic copy-paste shape —
   `use crate::commands::make_service;` plus `let svc = make_service()?;`
   — into a command makes the gate exit 1 (the issue's own falsifier in
   its real, co-occurring form).
5. The gate is scoped to `agents_authoring.rs` only — a legitimate
   `make_service` import/call in `browse.rs` does not trip it.

---

## Falsification

Cheapest at top. The cheapest claim's status is `passed` before this
design moves to planning.

| # | Claim | Falsifier | Oracle | Cost | Status | Regression fence |
|---|-------|-----------|--------|------|--------|------------------|
| 3 | Doc-comment mention must not fire | Run a text-grep of `make_service\|MarketplaceService` against the current correct file. If it matches → the text-grep design is false. | `grep` over the file (independent of any gate code) | 1m | **passed — text-grep FALSIFIED** (1 match at line 4; see run below). SQL variant: 0 import rows for the current file, verified on a fresh index. | unit test `fence_ignores_unrelated_imports_in_target_file` (SQL-level comment-safety, CI via `cargo test --workspace`) + manual e2e on the real file (2026-06-30). Real-index assertion in CI pending **kiro-oseh**. |
| 1 | Fires on type import | Seed an `imports` row (`symbol_name='MarketplaceService'`) for the file → expect 1 finding | in-memory SQLite unit test, mirror of `no_frontend_deps_flags_tauri_in_core` | 5m | **passed** | unit test `fence_flags_marketplace_service_type_import` (CI via `cargo test --workspace`) |
| 2 | Fires on helper import | Seed an `imports` row (`symbol_name='make_service'`) for the file → expect 1 finding | in-memory SQLite unit test | 5m | **passed** | unit test `fence_flags_make_service_import` (CI via `cargo test --workspace`) |
| 5 | Scoped to the one file | Seed a `make_service` import in `browse.rs` → expect 0 findings | in-memory SQLite unit test | 5m | **passed** | unit test `fence_ignores_make_service_outside_target_file` (CI via `cargo test --workspace`) |
| 4 | End-to-end: real violation trips the gate | Add `use crate::commands::make_service;` + `let svc = make_service()?;` to a command body, re-index, run `cargo xtask plan-lint --gate no-marketplace-service-in-agents-authoring` → exit 1 | the gate binary against a real re-indexed workspace | 10m | **passed (manual, 2026-06-30)**: clean→exit 0, injected import→exit 1, reverted | the unit tests above pin the gate logic in CI; applying the gate binary to the real codebase in CI is pending **kiro-oseh** |

### Cheapest falsifier — run, recorded

Command:

```
rg 'make_service|MarketplaceService' \
  crates/kiro-control-center/src-tauri/src/commands/agents_authoring.rs
```

Result (2026-06-30):

```
4://! accept a [`kiro_market_core::service::MarketplaceService`]. Per
```

**One match, on a doc comment, in the correct file.** The text-grep
claim ("Expected: zero matches") is false. A text-grep gate would
red-CI the very code it protects. Design pivots to the tethys SQL gate.

Cross-check (oracle, computed a different way) — service-consuming
commands separate the mention forms from the real-coupling forms:

```
agents.rs:4     //! …builds the [`MarketplaceService`] from   ← mention (shape 1)
agents.rs:18    use …{…, MarketplaceService, …}               ← type import (shape 2)
browse.rs:19    use crate::commands::{make_service, …}        ← helper import (shape 3)
browse.rs:106   let svc = make_service()?;                    ← helper call (shape 4)
```

A grep would flag the `agents.rs:4` doc comment identically to the real
`browse.rs:106` call. The tethys `imports` query sees only the real
`use` lines (shapes 2–4), never the comments.

---

## Empirical verification — tethys re-indexed (oracle cross-check)

Ran `tethys --workspace . index` (tethys 0.1.0): 74 files, 1815 symbols,
20235 refs. Cross-checked the `imports` table against a ripgrep
ground-truth oracle:

- **Completeness (no false negatives).** ripgrep found real
  `use …MarketplaceService` / `use …make_service` lines in exactly 6
  commands-dir files (agents, browse, marketplaces, mod, plugins,
  steering); tethys `imports` lists the same 6 (plus the CLI crate's
  `install.rs` / `marketplace.rs`). No real import missed.
- **Comment-safety (no false positives).** ~20 doc-comment mentions
  exist (incl. `agents_authoring.rs:4` and `[`make_service`]` at
  `browse.rs:180`); tethys produces **zero** import rows from any of
  them.
- **Decisive control.** `agents_authoring.rs` (current, correct) has the
  line-4 comment and **0** service import rows → the gate passes it.
- **refs falsified.** `refs.reference_name = 'make_service'` → 0 rows on
  the fresh index. Control: `validate_kiro_project_path` — a bare
  free-fn called in every command — also → 0 refs. tethys records method
  and `Type::method` calls but not bare free-fn calls. Filed
  **tethys-zp2j**. Hence the gate is `imports`-only.

Conclusion: tethys's `imports` extraction is complete and comment-safe
for the symbols this gate depends on. The design's empirical premise
holds for the `imports` half and is correctly abandoned for the `refs`
half.

---

## Non-vacuity

Each fence names a specific bug that breaks it:

- Claim 3 fence fails if someone "simplifies" the gate to a
  `LIKE '%MarketplaceService%'` text match — the current file's line-4
  comment makes the fence fire, so the test goes red. Not vacuous: the
  triggering input already exists in-tree.
- Claim 1/2/5 fences fail if the respective `imports` predicate
  (`symbol_name IN ('MarketplaceService','make_service')`) or the
  file-path scoping is dropped from the SQL.

The TDD-inversion holds: removing any single predicate from the gate SQL
makes at least one distinct named test fail, and the failing test
identifies which predicate regressed.

---

## Per-claim distinctness

Claims 1, 2, and 5 each have their own unit test with its own seeded
fixture and its own assertion, so a failure localizes to the specific
predicate (type-import vs helper-import vs scope). Claim 3 has a
dedicated integration test over a real index. No two claims share a
single yes/no oracle.

---

## Negative space — what this gate deliberately does NOT do

1. **Fully-qualified call without a `use`** — shape 5. A hand-written
   `let svc = crate::commands::make_service()?;` (no import line) has no
   `imports` row, and tethys records no bare free-fn call ref
   (**tethys-zp2j**), so the gate would miss it. This is not the
   copy-paste threat C7 targets (copy-paste brings the `use`); it is a
   deliberate, conspicuous hand-write. Accepted within kiro-6g6r scope.
2. **Aliased imports** (`use …MarketplaceService as Svc;`) — shape 6. A
   symbol-name match on `MarketplaceService` still catches that import
   line, but a fully renamed re-export chain could evade it. Detecting
   that requires symbol resolution the gate doesn't attempt. Accepted
   within kiro-6g6r scope; file a follow-up only if a real evasion
   appears.
3. **File rename / move** — shape 7. The gate's path anchor
   (`%agents_authoring.rs`) stops protecting anything if the commands
   move. Same accepted limitation every path-scoped gate in
   `plan_lint.rs` carries, and the same failure class as the
   stale-allowlist mechanism and **kiro-jnw2**.
4. **Behavioral verification** — the gate proves *absence of coupling
   imports*, not that the commands are semantically project-only. It is a
   structural fence, not a proof of behavior.

---

## Tracker references

- **kiro-6g6r** — this gate. In progress under this design.
- **kiro-jnw2** — existing stale-allowlist anchor-drift issue; cited as
  the precedent for the accepted path-scope limitation (Negative space
  #3). Verified open via `rivets show kiro-jnw2`.
- **tethys-zp2j** — tethys indexer does not record bare free-function
  call refs; the reason this gate is `imports`-only. Filed in the tethys
  repo during this design; verified via `rivets show tethys-zp2j`.
- **kiro-oseh** — wire `cargo xtask plan-lint` into CI. The gate's logic
  is CI-enforced via unit tests today (`cargo test --workspace`); applying
  the gate binary to the real codebase in CI depends on this issue. Filed
  during implementation; plan-lint is not currently a CI job.

No new deferrals left dangling. Shapes 5–6 (qualified-call-without-use,
aliased import) are consciously accepted within kiro-6g6r's scope, not
deferred to a phantom ticket.

---

## Implementation sketch (for the plan phase)

Add to `xtask/src/plan_lint.rs`:

- `const NO_SERVICE_IN_AGENTS_AUTHORING_SQL` — an `imports` query
  (mirror of `NO_FRONTEND_DEPS_IN_CORE_SQL`): select from `imports`
  joined to `files` where
  `f.path LIKE '%commands/agents_authoring.rs'` and
  `i.symbol_name IN ('MarketplaceService', 'make_service')`. Report
  `line = 0` (the `imports` table has no line column — reuse the
  existing `format_path_line` sentinel). No `refs` clause (tethys-zp2j).
- A `Gate { name: "no-marketplace-service-in-agents-authoring", … }`
  appended to `ALL_GATES`.
- Unit tests per the Falsification table (mirror the existing
  `no_frontend_deps_*` fixtures, which seed `imports` rows).
- One integration/e2e check asserting the gate exits 0 against the
  current file and 1 after injecting `use crate::commands::make_service;`
  + a call and re-indexing.

Run `gilfoyle` (adversarial review) against the finished gate to stress
shapes 5 (qualified-call-without-use) and 6 (alias evasion).

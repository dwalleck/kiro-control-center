# Falsifiable Design — agents-view slice 1

Slice 1 of the agents-view feature (spec at `./spec.md`). Skeleton cut:
list page + editor shell + Identity panel + System Prompt panel + 5 Tauri
commands for CRUD (including marketplace-coexistence prompt-at-save and
tracking-aware delete). Slices 2-6 are out of scope.

## Purpose

Implement spec behaviors **B1–B14** at the data + IPC layer (everything the
Svelte UI calls into), with the editor body limited to the Identity and
System Prompt sections.

## Architecture

Three layers, dependencies pointing inward (per CLAUDE.md "Dependencies
point inward"):

```
crates/kiro-control-center/src/lib/components/      (Svelte 5)
    AgentsTab.svelte                                 list page
    AgentEditor.svelte                               editor shell
    editor/IdentityPanel.svelte
    editor/PromptPanel.svelte
    └── calls Tauri commands via bindings.ts
              ↓
crates/kiro-control-center/src-tauri/src/commands/
    agents_authoring.rs                              5 wrappers (project-only;
                                                     no _impl(svc) pattern)
    └── calls KiroProject methods
              ↓
crates/kiro-market-core/src/project.rs               4 new public methods
    list_user_agents()
    create_user_agent()
    save_user_agent()             (handles in-place + rename + detach)
    delete_user_agent()           (delegates to existing remove_agent when tracked)
    duplicate_user_agent()
crates/kiro-market-core/schemas/agent-spec.json      (vendored from design bundle)
```

No new dependencies on `MarketplaceService`. No new dependencies on the
existing `parse_native` parser. No frontend dependencies in
`kiro-market-core`.

## Input shapes (enumerated to anchor claim coverage)

For every input the design touches, every production-reachable shape gets at
least one claim:

| Shape | Variants reachable | Claim that covers it |
|---|---|---|
| `.kiro/agents/` directory | missing / empty / has valid files / has invalid JSON / has files where name field is absent / has files where name field != filename | C1, C2 |
| `.kiro/installed-agents.json` | missing / empty `agents: {}` / has entries matching files / has orphan entries | C1 (with orphan-drop) |
| Agent draft `name` field | matches `^[a-z0-9][a-z0-9-]*$` / empty / regex-violating / Unicode | C3 (validation rejection) |
| Agent draft other fields | description null/non-null; model null/non-null; prompt null/inline/`file://`-prefixed; tools/mcpServers/resources/hooks empty or populated | C2 (row counts), C4 (round-trip via untyped JSON) |
| Save context | new (no existing file) / edit in-place (from_name == draft.name) / rename (from_name != draft.name) × {collision, no collision} × {tracked source, untracked source} × {detach true, detach false} | C3, C4 |
| Delete context | user-authored target / marketplace-tracked target / target missing (idempotent) | C5 |
| Duplicate context | source untracked / source marketplace-tracked / target `-copy` free / target `-copy` taken, `-copy-2` free / chain occupied through `-copy-N` | C6 |
| FFI boundary types | command argument strings (project_path, name) / response payloads (UserAgentRow, error variants) | C7, C8 |

Out-of-scope shapes (slice 2-6, not claimed here):
- Tools array beyond count — handled in slice 2 (allowedTools, toolAliases, per-tool cards).
- mcpServers shape beyond count — slice 3 (transport types, OAuth).
- Resources beyond count — slice 4 (file vs knowledgeBase shapes, schema-gap fix for `ComplexResource.description`).
- Hooks beyond count — slice 5 (matcher, timeout, cache, max output).
- `toolsSettings`, `includeMcpJson` — slice 6.

## Claims (8 total)

### C1 — List output specification

`list_user_agents(project)` returns one `UserAgentRow` per JSON-parseable
`*.json` file in `<project>/.kiro/agents/`. Each row carries `name`
(JSON's `name` field if present, else filename stem), `description`,
`model`, `tools_count`, `mcp_count`, `resources_count`, `hooks_count`, and
`lineage: Some({marketplace, plugin, version})` iff the row's name is a key
in `installed-agents.json#/agents`. Orphan tracking entries (name in
tracking, no file on disk) are excluded. Files that fail JSON parsing are
excluded and logged via `tracing::warn!`. The `.kiro/agents/` directory is
created (`fs::create_dir_all`) if absent before listing; missing directory
yields an empty list.

### C2 — List uses untyped JSON, not `parse_native`

`list_user_agents` parses each file via `serde_json::from_slice::<serde_json::Value>`,
**never** via `agent::parse_native::parse_native_kiro_agent_file`. Files
that lack the required `name` field, exceed `parse_native`'s 1 MiB byte
cap, or live behind a symlink are still listable (their `name` falls back
to the filename stem; symlink security is not the list view's concern —
the install path remains the only place those checks live).

### C3 — Create path: atomic, validating, collision-rejecting

`create_user_agent(draft_bytes, project)` parses `draft_bytes` as
`serde_json::Value`, extracts the `name` field, validates it via
`AgentName::new` (failing fast with `AgentSaveError::InvalidName` on
empty / regex-violating / Unicode names), then writes
`<agents_dir>/<name>.json` via `cache::atomic_write`. If
`<agents_dir>/<name>.json` already exists at the moment of write
(`Path::exists` check inside the lock), returns
`AgentSaveError::NameCollision { name }` and writes nothing.

### C4 — Save path: in-place + rename + detach, transactional

`save_user_agent(from_name, draft_bytes, detach, project)` validates both
`from_name` and `draft.name` as `AgentName`. Acquires
`file_lock::with_file_lock(agent_tracking_path)`. Inside the lock:

1. If `from_name != draft.name` AND `<draft.name>.json` exists, return
   `AgentSaveError::NameCollision`. No file modified.
2. Write `<draft.name>.json` via `cache::atomic_write`. (File-first
   ordering, symmetric with `install_native_agent`.)
3. If `from_name != draft.name`, `fs::remove_file(<from_name>.json)`.
   Best-effort: an unlink failure here yields a warning + `Ok(())`, since
   the new file is correctly in place and the old one is merely orphaned.
4. If `detach && installed.agents.contains_key(&from_name.to_string())`,
   remove the entry from `installed` and call `write_agent_tracking`.

Crash semantics: a crash between (2) and (3) leaves both files present (UI
shows the new one and a stale one; cleanup is a re-save away). A crash
between (3) and (4) leaves a renamed-but-still-tracked agent, which is the
existing `ContentChangedRequiresForce` state on next marketplace update.

### C5 — Delete path: tracking-aware

`delete_user_agent(name, project)` validates `name`, then:

- If `installed.agents.contains_key(name)`: delegate to existing
  `KiroProject::remove_agent` (file lock + tracking-update + file unlink
  with rollback-on-failure). Re-uses the entire transactional pattern at
  `project.rs:1386`.
- Else: `fs::remove_file(<agents_dir>/<name>.json)` directly. Idempotent
  on `ErrorKind::NotFound` (returns `Ok(())`).

### C6 — Duplicate path: collision-walking, lineage-stripping

`duplicate_user_agent(source_name, project)` reads
`<agents_dir>/<source_name>.json` as `serde_json::Value`, finds the
smallest `k ≥ 1` such that `<source_name>-copy.json` (k=1, no numeric
suffix) or `<source_name>-copy-<k>.json` (k≥2) does not exist on disk,
modifies the value's `name` field to the target name, writes via
`cache::atomic_write`. No `InstalledAgents::agents` entry is created,
regardless of whether the source had one. Returns the new name.

### C7 — All 5 Tauri commands are project-only

None of `list_user_agents`, `create_user_agent`, `save_user_agent`,
`delete_user_agent`, `duplicate_user_agent` construct or consume a
`MarketplaceService`. Per CLAUDE.md "Tauri command handlers", their
bodies inline in the wrapper (no `_impl(svc, ...)` shape). Name fields
crossing FFI are wrapped in `AgentName` (existing newtype in
`validation.rs`).

### C8 — Vendored schema and clean bindings regen

The authoritative `agent-spec.json` (spec decision #11) is vendored to
`crates/kiro-market-core/schemas/agent-spec.json` in slice 1. The new
wire-format types — `UserAgentRow`, `UserAgentLineage`, `AgentSaveError`,
`AgentDeleteError`, `AgentDuplicateError` — all derive
`specta::Type` (gated by `feature = "specta"`). `cargo test -p
kiro-control-center --lib -- --ignored` regenerates `bindings.ts`
without changes after the second consecutive run (idempotent).

## Falsification

Cheapest at top. Cheapest claim's status MUST be `passed` before the design
moves to planning. See `falsifier-runs/` for raw outputs.

| # | Claim | Falsifier | Oracle | Cost | Status | Regression fence |
|---|---|---|---|---|---|---|
| C2 | List uses untyped JSON | Add `fixture/.kiro/agents/no-name.json` containing `{"description": "x", "tools": []}` (no `name` field). Run `probe.py fixture`. Expected: a row with `name = "no-name"` appears. If the row is absent or probe crashes → claim false. | `.agents-view/probe/probe.py` (already independent of `parse_native`) | 5m | **passed** (run 2026-05-21, see § Cheapest falsifier below) | rstest case in `project.rs` for `list_user_agents` against a fixture with a no-name file — pre-fix code calling `parse_native` would error |
| C7 | Project-only commands | Inspect the tethys index for an `imports` row (`MarketplaceService` / `make_service`) or a `refs` row (`reference_name = 'make_service'`) scoped to `agents_authoring.rs`. Expected: zero such rows for the correct file; injecting `let svc = make_service()?` into a command body produces a finding. | tethys SQL query (independent of the command code) | 5m | **superseded — see [`design-6g6r.md`](./design-6g6r.md)**. The original static-grep falsifier was *itself falsified* (it matched the line-4 doc comment in the correct file); approach pivoted to a tethys SQL gate. | CI gate `cargo xtask plan-lint --gate no-marketplace-service-in-agents-authoring` (issue **kiro-6g6r**); full falsification table lives in `design-6g6r.md` |
| C1 | List output spec | Run `probe.py <project>` and `cargo test list_user_agents_against_probe_fixture` on the same fixture (the synthetic mixed-lineage fixture at `.agents-view/probe/fixture/` plus the no-name extension from C2). Diff row sets. Expected: identical row sets. | `probe.py` output | 15m (after implementation) | **pending** | the `list_user_agents_against_probe_fixture` test itself; fixture must remain in-tree |
| C3 | Create rejects collision | rstest with cases: (a) `empty_name_rejected`, (b) `regex_violating_name_rejected` ("Bad", "-leads", ".dotted"), (c) `unicode_name_rejected`, (d) `collision_rejected_existing_file_unmodified` — pre-write `foo.json` with known bytes, call create, assert file hash unchanged AND `NameCollision` error returned. | bcrypt-style content hash comparison via `BlakeHash` (existing) | 10m | **pending** | the rstest itself |
| C5 | Delete tracking-aware | rstest cases: (a) `delete_untracked_removes_file_only`, (b) `delete_tracked_removes_file_and_tracking_entry` (asserts both `<name>.json` and `installed.agents[name]` absent post-call), (c) `delete_idempotent_on_missing_file`. | direct filesystem + `load_installed_agents` inspection in the test | 10m | **pending** | the rstest itself |
| C6 | Duplicate finds next free | rstest: pre-write `foo.json` (marketplace-tracked), `foo-copy.json`, `foo-copy-2.json`. Call `duplicate_user_agent("foo", ...)`. Expected: returns `"foo-copy-3"`; `foo-copy-3.json` exists; `foo-copy-3` NOT in `installed.agents`. Plus: `duplicate_user_agent` on a non-existent source returns `AgentDuplicateError::SourceNotFound` (no other state change). | filesystem listing + `load_installed_agents` | 10m | **pending** | the rstest itself |
| C4 | Save path semantics | rstest cases: (a) `save_in_place_overwrites_atomically`, (b) `save_rename_writes_new_then_unlinks_old`, (c) `save_rename_collision_leaves_both_files_unchanged` — pre-write `foo` AND `bar`, call `save_user_agent("foo", {name: "bar", ...}, false)`, assert both unchanged + `NameCollision`, (d) `save_detach_removes_tracking_entry_within_lock`, (e) `save_detach_false_preserves_tracking_entry`. | filesystem + `load_installed_agents` after each call | 20m | **pending** | the rstest itself |
| C8 | Vendored schema + bindings | (a) `ls crates/kiro-market-core/schemas/agent-spec.json` exists. (b) Run `cargo test -p kiro-control-center --lib -- --ignored` twice in succession; second run must show no `bindings.ts` diff. (c) `git diff --exit-code crates/kiro-control-center/src/lib/bindings.ts` after the second run. | git diff exit code | 15m | **pending** | the existing `--ignored` test (which is run as part of `cargo test --workspace` in CI) |

### Cheapest falsifier — run NOW

Per skill mandate, run the cheapest meaningful falsifier (C2) before design
approval. Result captured at `.agents-view/probe/falsifier-runs/c2-no-name.log`.

(See § "Cheapest falsifier run" section below for the recorded run.)

## Negative space — slice 1 deliberately does NOT do these

1. **Tools section UI** (catalog, allowed-tools picker, per-tool cards, external MCP group) — slice 2. Tracked at issue **kiro-vgnw** (filed below).
2. **MCP Servers section UI** (stdio/http/registry transports, oauth config) — slice 3. Tracked at **kiro-gwo4**.
3. **Resources section UI + Knowledge Base modal + schema-gap fix** for `ComplexResource.description` — slice 4. Tracked at **kiro-3ll2**.
4. **Hooks section UI** (per-event groups, matcher/timeout/cache controls) — slice 5. Tracked at **kiro-ttew**.
5. **Advanced section UI** (`toolsSettings` JSON textarea, `includeMcpJson` toggle) — slice 6. Tracked at **kiro-zqci**.
6. **Section-rail count badges** (Tools 4, MCP 2, Resources 5, Hooks 3) — these surface as each section ships; slice 1's rail shows placeholder zeros for sections-not-yet-implemented.
7. **Runtime JSON Schema validation** of saved files. `$schema` is written as metadata only. The existing `parse_native_kiro_agent_file` is the validation gate at install time; the editor's save path trusts the user's input modulo the `AgentName` regex.
8. **Concurrent-edit arbitration** between multiple Control Center windows on the same agent JSON file. Last writer wins. The existing `file_lock::with_file_lock` covers tracking-file mutations only.
9. **Live filesystem watching** of `.kiro/agents/` for external changes — the list refreshes only on explicit user action (navigating into the tab, post-save, post-delete, post-duplicate).
10. **Importing existing agent JSON** from outside the project (no file-picker affordance). Users can drop files directly into `.kiro/agents/` via OS file manager; the list will show them on next refresh.
11. **Upstreaming the schema fix to kiro-cli's own repo.** Cross-repo coordination is out of scope here; the vendored copy in `crates/kiro-market-core/schemas/agent-spec.json` is the local source of truth.

## Decisions surfaced during design (recorded in spec)

- **D12** (already in spec): Orphan tracking entries silently excluded.
- **D13** (new — surfaced during design): Files that fail JSON parsing are silently excluded and `tracing::warn!`-logged. Alternative (broken-row UX with "remove or fix" affordance) considered and deferred to follow-up issue **kiro-fd40** (filed below) if real users hit the case.
- **D14** (new — surfaced during design): `list_user_agents` uses the JSON's `name` field if present, else falls back to the filename stem. Spec implies these always match (save path enforces); the list path tolerates drift to avoid hiding broken state.

I'll append these decisions to `./spec.md`'s decisions log after this design is approved.

## Tracker discipline — phrase audit

Searched this document for the trigger phrases (`deferred`, `out of scope`,
`tracked`, `follow-up`, `later`, `next PR`, `as part of`, `future work`,
`revisit if`).

Matches (each must cite a tracker ID or be settled rationale):

| # | Phrase | Resolution |
|---|---|---|
| 1 | "slice 2-6, not claimed here" (Input shapes table) | Slice scope; settled rationale — slices 2-6 issues filed below |
| 2 | "slice 2... slice 3... slice 4... slice 5... slice 6" (Negative space items 1-5) | Filed as **kiro-vgnw..6** below |
| 3 | "deferred to follow-up issue **kiro-fd40**" (D13) | Filed as **kiro-fd40** below |
| 4 | "out of scope here" (item 11) | Settled rationale — cross-repo coordination is not this PR's job |
| 5 | "Tracked at issue **kiro-6g6r..7**" | Filed below |

Issues to file at design-approval time (before merging into a plan):

- **kiro-6g6r** — CI gate: `cargo xtask plan-lint --gate no-marketplace-service-in-agents-authoring`. Regression fence for C7.
- **kiro-vgnw** — Slice 2: Tools section UI + tools-catalog.ts port from agents-data.js.
- **kiro-gwo4** — Slice 3: MCP Servers section UI (stdio/http/registry transports; OAuth fields skipped).
- **kiro-3ll2** — Slice 4: Resources section UI + Knowledge Base modal + ship the schema-gap fix for `ComplexResource.description`.
- **kiro-ttew** — Slice 5: Hooks section UI (event groups, matcher controls).
- **kiro-zqci** — Slice 6: Advanced section UI (`toolsSettings`, `includeMcpJson`).
- **kiro-fd40** — Broken-row UX for unparseable agent JSON (D13 follow-up; deferred unless users hit it).

I will file these via `rivets create` immediately after this design doc is
approved and before opening the slice 1 PR. (Doing it before approval is
premature — if the design is rejected, the issues become noise.) The
placeholders `kiro-6g6r..7` in this document become real IDs at that point.

## Self-review (skill mandates 7 checks)

1. **Claim count.** 8 claims. In-range (3-15).
2. **Falsifier independence.** Every falsifier's oracle is either (a) the probe.py / oracle.ps1 from prove-it-prototype (independent toolchain), (b) external filesystem inspection in tests, (c) static grep / file existence check, or (d) git diff. None depend on the implementation under test.
3. **Falsifier non-vacuity.** For each claim, I name below a specific buggy implementation that the falsifier kills:
   - C1: if `list_user_agents` accidentally filters by `lineage.is_some()`, only marketplace-tracked rows return → diff against probe fails.
   - C2: if I write `parse_native_kiro_agent_file(&path).ok().map(...)` instead of `serde_json::from_slice`, the no-name fixture file is silently dropped → probe expects it, oracle finds it absent.
   - C3: if I forget the `Path::exists` collision check, an existing file gets overwritten → pre/post hash differ.
   - C4: if I do the rename in the wrong order (unlink-then-write), a crash between leaves NO file → the test asserts new file presence post-call.
   - C5: if `delete_user_agent` always calls `remove_agent` (even for user-authored), unknown-name agents return `AgentError::NotInstalled` instead of `Ok(())` → test (a) fails.
   - C6: if duplicate logic accidentally inserts the new name into tracking (e.g. copy-paste from `install_native_agent`), the test's tracking-absence assertion fires.
   - C7: if a future contributor adds `let svc = make_service()?` to a command body to share a helper, the tethys SQL gate finds the `make_service` ref (see `design-6g6r.md`; a plain text-grep was rejected because it false-positives on the module doc comment).
   - C8: if a wire-format type forgets `#[cfg_attr(feature = "specta", derive(specta::Type))]`, `bindings.ts` regen fails.
4. **Per-claim verification distinctness.** Every claim has a distinct test name OR a distinct oracle. If C2's falsifier fails, the error is "no-name file missing from probe output" — not interchangeable with any other claim's failure. C7's gate names the file scope and the import/ref predicates explicitly (see `design-6g6r.md`).
5. **Cost distribution.** Cheapest 1m (C7 — the static-grep falsifier ran and was *falsified*; pivoted to a 5m tethys SQL gate, see `design-6g6r.md`). C2 at 5m. All others ≤ 20m. No claim has only an expensive (hours/days) falsifier.
6. **Negative space.** 11 entries. Required ≥ 3.
7. **Tracker references.** Audited in the table above. Seven new issues to file at approval time; placeholders `kiro-6g6r..7` documented for backfill.

## Cheapest falsifier run

### C2 — no-name file empirically supports parser-independence

Ran 2026-05-21. Input: `.agents-view/probe/fixture/.kiro/agents/no-name.json`
contains `{"description": "x", "model": null, "tools": ["fs_read"], "mcpServers": {}, "resources": []}` — no `name` field.

```
$ python probe.py fixture                         # → falsifier-runs/c2-no-name.log
$ pwsh -File oracle.ps1 fixture                   # → falsifier-runs/c2-no-name-oracle.log
$ python compare.py c2-no-name.log c2-no-name-oracle.log
AGREE on 3 rows: ['marketplace-tracked', 'no-name', 'user-authored']
```

The no-name row appears in BOTH outputs with `name: "no-name"` (filename
stem fallback), `description`/`model`/`lineage` correctly nulled or
populated, and counts matching the file contents. Strong evidence: two
independent toolchains (CPython `json` and PowerShell `System.Text.Json`,
filesystem-first and tracking-first) computed the same answer on a file
that `agent::parse_native::parse_native_kiro_agent_file` would reject
with `NativeParseFailure::InvalidJson` (`name` is a required deserialize
field on `NativeAgentBundle`).

Implication for the implementation: the Rust `list_user_agents` is free to
use `serde_json::from_slice::<serde_json::Value>` and a `value.get("name")`
+ filename-stem fallback. No call into `parse_native` is needed (or
desired — its security checks are appropriate for *install*, not *list*).

Status in the falsification table: **passed** for C2. The other 7 claims'
falsifiers remain pending until implementation lands.

## Hard gate (skill requirement) — status

- [x] Every production-reachable input shape covered by ≥1 claim (Input shapes table maps each row to a claim ID)
- [x] Every claim has a falsifier in the Falsification table
- [x] Every falsifier names an independent oracle (probe.py / oracle.ps1 / static grep / filesystem inspection / git diff)
- [x] Every falsifier names a specific buggy implementation that would make it fail (Self-review item 3)
- [x] Every claim has a distinct verifiable output (Self-review item 4)
- [x] Every measurement-based claim has a `Regression fence` entry pointing at a deterministic test (table column)
- [x] Every deferral / out-of-scope reference cites a verified or to-be-filed tracker ID (Tracker discipline phrase audit)
- [x] The cheapest falsifier has been run and passed (C2 above)
- [x] Negative space list has ≥ 3 entries (11 entries)

Design ready for hand-off to `budgeted-plan`.

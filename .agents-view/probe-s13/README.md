# prove-it-prototype — S13 (AgentEditor.svelte shell)

Run date: 2026-05-22.
Branch: `agents-view-slice-1-part-2`.
Slice: S13 in `.agents-view/plan-slice-1.md`.

## Smallest factual question

> **Post-A1/A3, what is the exact contract that `AgentEditor.svelte`
> (S13) must consume from its parent (`AgentsTab.svelte`) and from
> the Tauri IPC layer (`bindings.ts`)?**

S13 is a Svelte editor shell. The hardening that landed after S12
(amendments A1 — `SaveOutcome`, A2 — `validate_draft_json_payload`,
A3 — `AgentsTabMode` moved into helpers) extended the IPC contract.
If my mental model of the post-amendment contract is wrong, the
editor will compile-error or silently lose the orphan-warning
plumb-through that A1 was added to enable.

## Step 0 — prior art

Searched `.rivets/issues.jsonl` for any S13-related ticket. Hits:

- **kiro-vgnw** (slice 2, Tools UI) explicitly references slice 1
  shipping "skeleton: list + editor shell + Identity + System Prompt"
  — confirms S13 is the editor shell I'm probing.
- **kiro-vsi1** (PR #120 test gaps) — backend test gaps; not
  S13-blocking.
- **kiro-78io** (filename/JSON-name drift) — confirms `UserAgentRow.name`
  is the filename stem post-PR #120 hardening (relevant to what S13
  shows in its title).
- **kiro-ttew / kiro-zqci / kiro-3ll2 / kiro-gwo4** — slice-2-through-6
  trackers. The disabled section-rail entries S13 must render are
  pinned by these.

No prior art for the AgentEditor.svelte file itself. Five minutes
spent; below the upper bound.

## Probe

`probe.mjs` — Node script that reads the **generated** bindings.ts
and the helpers module and prints what S13 will import. Honest text
extraction with regex; if the regex misses, the disagreement with
the oracle would expose it.

```
$ node probe.mjs . > probe.out
```

## Oracle

`oracle.ps1` — PowerShell script that reads the **Rust source files**
that specta generated bindings.ts from. Independent toolchain
(PowerShell vs Node), independent file (Rust source vs TypeScript
output), independent parser (`Select-String` regex on `.rs` vs
JS regex on `.ts`).

The oracle is independent because:
1. It reads upstream Rust (`crates/kiro-market-core/src/user_agent.rs`,
   `crates/kiro-control-center/src-tauri/src/{error.rs,commands/agents_authoring.rs}`).
2. The probe reads downstream generated TypeScript
   (`crates/kiro-control-center/src/lib/bindings.ts`).
3. specta is the bridge between them — and its idempotency is
   already verified by the C8 regression fence
   (`cargo test -p kiro-control-center --lib generate_types --
   --ignored` runs cleanly twice with no diff). So if probe and
   oracle agree, both my mental model and specta's emission are
   confirmed.

For the frontend-only `AgentsTabMode` (no Rust upstream), the
oracle uses a different independence axis: it reads the
**consumer** (`AgentsTab.svelte`) and confirms each arm
(`list`/`new`/`edit`) is referenced in actual mode-transition
code, not just in the type definition.

```
$ powershell -NoProfile -File oracle.ps1 -Root . > oracle.out
```

## Agreement

Six items checked, all agree:

| # | Topic | Probe says | Oracle says | Match |
|---|---|---|---|---|
| 1 | `SaveOutcome.orphan_left_behind` | `string \| null` | `Option<String>` | ✓ |
| 2 | `UserAgentRow` field set | name/description/model + 4 counts + lineage | identical Rust mirror | ✓ |
| 3 | `UserAgentLineage` | 3 fields, version nullable | identical Rust mirror | ✓ |
| 4 | `ErrorType` variants (8) | snake_case unions | PascalCase enum, same set | ✓ |
| 5 | 5 command signatures | TS `commands.*UserAgent` | 5 `#[tauri::command]` fns | ✓ |
| 6 | `AgentsTabMode` arms | `list`/`new`/`edit` in helpers | same 3 strings used as mode transitions in `AgentsTab.svelte` (L97/109/205/259) | ✓ |

Raw outputs in `probe.out` and `oracle.out`.

## What I learned

Five things my mental model didn't have before the probe ran:

1. **`SaveOutcome.orphan_left_behind` is required-but-nullable on the
   wire, not optional.** The plan amendment A1 wrote it as
   `orphan_left_behind?: string | null` (with `?`), but specta emits
   Rust `Option<String>` as `string | null` always-present. The
   editor's save handler checks `outcome.orphan_left_behind !== null`,
   not `if (outcome.orphan_left_behind)` (which would still work for
   the truthy/falsy split but isn't strictly type-correct under
   `--strict`).

2. **`createUserAgent` takes `name` and `draftJson` as separate IPC
   args**, not just the JSON blob. Defense-in-depth: the IPC boundary
   validates the name independently of whatever's inside the JSON, and
   `KiroProject::create_user_agent` enforces that the JSON's `name`
   field equals the wrapper arg (spec D14). The editor must extract
   the name from its form state AND pass it explicitly.

3. **`UserAgentRow.name` is canonically the filename stem**, not the
   JSON `name` field (spec D14, post-PR #120 review). When the editor
   reads `mode.row.name` it's reading the filename stem, and any
   user-typed rename in the Identity panel becomes the new `draft_name`
   passed to `saveUserAgent`. Drift between filename and JSON `name`
   is only ever resolved by the save path's overwrite of the JSON's
   `name` field to match `draft_name`.

4. **The parent (`AgentsTab.svelte`) already wires the editor branch
   as a placeholder.** Lines 254-269 contain a stub topbar that uses
   `headerLabel(mode)` and a "Editor not yet available" body. S13
   replaces the *body* of the `{:else}` branch — it does not refactor
   the parent's branching, the `mode` state, the `refresh()`
   call-site, or the toast helper.

5. **The editor owns its own topbar.** Per the React reference
   (`Kiro Control Center Design System/design_handoff_agents/source/AgentEditor.jsx:60-72`),
   the topbar shows back-arrow + agent icon + name + `.kiro/agents/<name>.json`
   path chip + Cancel + Save buttons — far richer than the parent's
   placeholder which uses `headerLabel(mode)` alone. Once S13 ships,
   the parent's placeholder topbar inside the `{:else}` branch is
   gone; `headerLabel` becomes unreferenced from `AgentsTab.svelte`
   (it stays in helpers as a vitest-tested function with no caller in
   the tab — acceptable since later slices may re-introduce a
   parent-side topbar for nested editor states; revisit at S17 if
   not).

## Hard-gate checklist (skill requirement)

- [x] Probe written and runs against the real codebase
      (`probe.mjs`, exits 0 against `crates/kiro-control-center/src/lib/`)
- [x] Oracle defined and produces output
      (`oracle.ps1`, exits 0 against the Rust source)
- [x] Probe and oracle agree on at least one non-trivial slice
      (six items agree, including the `SaveOutcome` shape that
      drove the A1 amendment)
- [x] One-sentence "what I learned" written down (item 1 above is
      the load-bearing surprise: `string | null` vs optional `?`)

Ready to hand off to falsifiable-design / planning. (For S13
specifically, the design + plan already exist in
`.agents-view/design-slice-1.md` and `.agents-view/plan-slice-1.md`;
this probe re-verifies the contract the existing plan assumes,
post-amendments A1-A3.)

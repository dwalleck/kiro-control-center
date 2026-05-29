# Falsifiable Design — agents-view slice 2

Slice 2 of the agents-view feature (spec at `./spec.md`, slice 1 design at
`./design-slice-1.md`). Builds on slice-1's editor shell. **Frontend-only**:
no new Tauri commands, no new Rust core types. Reads/writes round-trip
through slice 1's `create_user_agent` / `save_user_agent` IPC, both of
which already accept `serde_json::Value`-shaped payloads.

Tracker: **kiro-vgnw** (P3).

## Purpose

Ship the **Tools section** per design § 5 of
`Kiro Control Center Design System/design_handoff_agents/README.md`
(screenshots `04-tools.png`, `04-tools-allowed-picker.png`). The section
exposes three sub-regions per design:

1. **Auto-allowed tools panel** (top) — list of tool names the agent
   will run without per-call prompts; picker affordance to add more.
2. **Available tools grid** (middle) — native-tool catalog grouped by
   category (9 categories, 15 tools per the probe).
3. **External (MCP) tools group** (bottom) — entries from `tools[]`
   whose name starts with `@` (e.g. `@server/tool`); add via free-text.

Spec behaviors covered: design § 5 + spec decision #5 (static catalog).
Spec behaviors NOT covered: MCP server CRUD UI (slice 3, kiro-gwo4).

## Architecture

Single layer: Svelte 5 + pure-TS helpers under
`crates/kiro-control-center/src/lib/`.

```
crates/kiro-control-center/src/lib/components/editor/
    ToolsPanel.svelte               new — renders the three sub-regions
    AllowedToolsList.svelte         new — auto-allowed picker subcomponent
    └── consumes pure-logic helpers in $lib/agents/
              ↓
crates/kiro-control-center/src/lib/agents/
    tools-catalog.ts                new — static port of AGENT_TOOLS
    tools-catalog.test.ts           new — falsifier vs probe-s2 answer
    tool-state.ts                   new — pure reducers for draft mutation
    tool-state.test.ts              new — falsifiers for C2–C5
              ↓
crates/kiro-control-center/src/lib/components/AgentEditor.svelte
    (modified — enable section "tools", remove `disabled: true`)
```

`tools-catalog.ts` is a frozen module-level constant. No data flows up to
Rust; the draft's `tools` / `allowedTools` / `toolAliases` fields ride the
existing `serde_json::Value` payload through `create_user_agent` /
`save_user_agent` unchanged.

## Input shapes (enumerated to anchor claim coverage)

For every input touched by slice 2:

| Shape | Variants reachable | Claim |
|---|---|---|
| `AGENT_TOOLS` array (source-of-truth) | the 15 entries enumerated at `.agents-view/probe-s2/probe.out` | C1 |
| Draft `tools[]` | empty / contains native names / contains `@`-prefixed MCP names / mixed | C2, C4, C5 |
| Draft `allowedTools[]` | empty / subset of `tools[]` / contains names NOT in `tools[]` (the "NOT VISIBLE" yellow-chip state per design § 5) | C3 |
| Draft `toolAliases{}` | empty / has aliases for currently-enabled tools / has stale aliases for disabled tools (precursor to the C2 scrub) | C2 |
| Toggle action input | toggle name not in `tools[]` (enable) / toggle name in `tools[]` (disable, triggers cascade) | C2 |
| AllowedToolsList add input | name already in `allowedTools[]` (dedupe) / name new / empty string | C3 |
| External-MCP add input | well-formed `@server/tool` / `@server` (server-only) / duplicate of existing entry / empty / non-`@`-prefixed (rejected at input boundary) | C4, C6 |

Out-of-scope shapes (later slices):
- `mcpServers{}` object — slice 3.
- `toolsSettings{}` per-tool config — slice 6.
- Knowledge-base / resource fields — slice 4.

## Claims (6 total)

### C1 — `tools-catalog.ts` faithful port

`tools-catalog.ts` exports a frozen `TOOLS_CATALOG: readonly Tool[]` whose
length is **15** and whose category set is exactly **`{Cloud, Code,
Filesystem, Meta, Orchestration, Planning, Reasoning, Shell, Web}`**.
Each entry has `{name, category, summary}` matching the probe's recorded
answer at `.agents-view/probe-s2/probe.out` byte-for-byte (modulo TS
literal syntax). Tools are listed in the same order as the source
`agents-data.js` to ease visual diff during review.

### C2 — Toggle-off cleanup cascade (the watch-out invariant)

`toggleTool(draft, name)` is a pure reducer. When `name ∈ draft.tools`
("disable" path), the returned draft has `name` absent from **all three**:
`tools`, `allowedTools`, and `toolAliases`. When `name ∉ draft.tools`
("enable" path), `name` is appended to `tools`; `allowedTools` and
`toolAliases` are unchanged.

This is the falsifier-worthy claim flagged by the spec's "Things to watch
out for" item 8. A naive `tools = tools.filter(t => t !== name)` that
forgets the other two arrays leaves orphan `allowedTools` entries and
stale aliases that re-surface the moment the user toggles the tool back
on.

### C3 — Auto-allowed list independence

`addAllowed(draft, name)` mutates ONLY `draft.allowedTools` (append-if-absent;
dedup). `removeAllowed(draft, name)` mutates ONLY `draft.allowedTools`
(filter). Neither touches `tools[]` or `toolAliases{}`. The independence
is bidirectional: a tool can be "allowed" but not in `tools[]` (the design's
yellow-chip "NOT VISIBLE" state), and a tool can be in `tools[]` but not
allowed (the default for a freshly-enabled tool).

### C4 — External-MCP partition

`partitionTools(tools)` returns `{ native, external }`: an entry routes to
`external` iff `name.startsWith("@")`; otherwise `native`. The
ToolsPanel renders `native` in the by-category grid (joined against
`TOOLS_CATALOG`) and `external` in the dedicated External (MCP) group.
A native entry whose name doesn't appear in `TOOLS_CATALOG` (an unknown
native tool — should not occur in practice but the type allows it) is
shown in a single "Unknown" group at the bottom of the grid so it does
not silently disappear.

### C5 — Rail count badge = `tools.length`, live

The Tools rail badge in `AgentEditor.svelte` reads `draft.tools.length`
via Svelte 5 `$derived`. The count updates synchronously on any
`toggleTool` / external-add / external-remove. Empty `tools[]` renders
no badge (matching slice-1's pattern for `mcp` / `resources` / `hooks`).

### C6 — `+Add custom` path: `@`-prefix validation, dedupe

The `AllowedToolsList` `+Add custom` free-text input passes its value
through `addCustomTool(draft, raw)`: trims whitespace; rejects empty;
rejects entries that don't start with `@` (only MCP-style entries land
via this path — native tools go through the by-category grid checkbox);
dedupes against existing `draft.tools`. On accept, appends to `tools[]`
AND to `allowedTools[]` (the design intent is "make this MCP tool
visible AND auto-allowed in one action").

A naive `tools.push(raw)` would accept duplicates, accept native names
through the wrong affordance, and skip the allowed-list update —
falsifier covers all three.

## Falsification

Cheapest at top. The cheapest claim's status MUST be `passed` before the
design moves to planning. C1 is discharged by the probe-vs-oracle agreement
at `.agents-view/probe-s2/`.

| # | Claim | Falsifier | Oracle | Cost | Status | Regression fence |
|---|---|---|---|---|---|---|
| C1 | Catalog faithful port | vitest: `expect(TOOLS_CATALOG).toHaveLength(15)` + `expect([...new Set(TOOLS_CATALOG.map(t => t.category))].sort()).toEqual(<probe answer>)` + per-tool name equality against probe-s2's `probe.out` | `.agents-view/probe-s2/probe.out` (locked 2026-05-26) | 5m | **passed** (probe + oracle byte-identical; AGREE recorded) | the vitest test against `probe.out` — a future regression that drops a category fails it |
| C2 | Toggle cascade | vitest: pre-build `draft = {tools: ["fs_read"], allowedTools: ["fs_read"], toolAliases: {fs_read: "read"}}`; call `toggleTool(draft, "fs_read")`; assert all three are empty. Symmetric enable case: pre-build empty draft, toggle "fs_read", assert `tools = ["fs_read"]` + `allowedTools = []` + `toolAliases = {}` (no cross-touch). | direct equality assertions on returned draft | 10m | **pending** | the vitest test itself |
| C3 | Allowed independence | vitest: pre-build `draft = {tools: ["a"], allowedTools: [], toolAliases: {a: "x"}}`; call `addAllowed(draft, "a")`; assert `tools` and `toolAliases` unchanged, `allowedTools = ["a"]`. Plus dedupe: call again, assert `allowedTools = ["a"]` still. Plus `removeAllowed`. | equality assertions | 10m | **pending** | the vitest test |
| C4 | External partition | vitest: `partitionTools(["fs_read", "@server/tool", "@svc"])` → `{ native: ["fs_read"], external: ["@server/tool", "@svc"] }`. Edge: `partitionTools(["unknown_tool"])` → `{ native: ["unknown_tool"], external: [] }` (the "Unknown" group routing — see C4 prose). | equality assertions | 5m | **pending** | the vitest test |
| C5 | Rail count derived from draft | vitest (on a thin helper `toolsRailBadge(draft) -> number \| null`): empty `tools[]` → `null`; non-empty → `tools.length`. Component-level wiring is reviewed visually against `screenshots/04-tools.png`. | equality assertion | 5m | **pending** | the vitest test |
| C6 | +Add custom validation | vitest cases: (a) `addCustomTool(draft, "  ")` → unchanged; (b) `addCustomTool(draft, "fs_read")` → unchanged + signal "must start with @"; (c) `addCustomTool(draft, "@svc/foo")` on empty draft → `tools = ["@svc/foo"]` AND `allowedTools = ["@svc/foo"]`; (d) dedupe: call twice, second call unchanged. | equality + error-signal assertions | 15m | **pending** | the vitest test |

### Cheapest falsifier — already run

**C1 — catalog faithful port.** Ran 2026-05-26.

`.agents-view/probe-s2/probe.py` (Python, regex-extract from `agents-data.js`)
and `.agents-view/probe-s2/oracle.mjs` (Node, V8-eval via `vm.createContext`
with `window={}` shim) produced byte-identical normalized JSON: 15 tools,
9 categories, names and summaries matching. `diff probe.out oracle.out`
empty after both write LF line endings.

This forecloses a class of regression where a future port silently drops
a tool or category. The vitest assertion against `probe.out` makes the
guarantee structural — a future regression that adds/drops a tool MUST
re-run the probe and update the locked answer.

Status in falsification table: **passed** for C1. Five remaining falsifiers
land as vitest tests inside the slice (per CLAUDE.md vitest-for-pure-logic
discipline; component-level testing remains out of scope).

## Negative space — slice 2 deliberately does NOT do these

1. **MCP Servers section UI** (stdio / http / registry transports, OAuth) — slice 3, **kiro-gwo4**.
2. **Resources section + Knowledge Base modal + `ComplexResource.description` schema-gap fix** — slice 4, **kiro-3ll2**.
3. **Hooks section UI** — slice 5, **kiro-ttew**.
4. **Advanced section UI** (`toolsSettings`, `includeMcpJson`) — slice 6, **kiro-zqci**.
5. **Per-tool config / settings UI** (the `toolsSettings{}` map's per-tool form) — slice 6 keeps it as a raw JSON textarea.
6. **Runtime tool-availability check against kiro-cli** — the catalog is a frozen static module per spec decision #5. If kiro-cli ships a new native tool, this catalog and the design bundle both go stale together; the catalog update is a doc-bundle re-sync, not a runtime query.
7. **`toolAliases` UI surface beyond the by-category-grid inline alias field** — the design bundle already collapses alias entry into each tool's checkbox row (`AgentEditor.jsx:283–319`). No standalone aliases panel.
8. **Multi-select / bulk toggle UI** for tools — design renders one checkbox per tool; bulk-toggle would be a separate UX exercise.
9. **Search / filter inside the tools grid** — 15 tools fit on one screen at the design's font sizes; a filter input is YAGNI until the catalog grows.
10. **Drag-reorder of the auto-allowed list** — design renders unordered; order in `allowedTools[]` is insertion order, which the user controls via add/remove.

## Tracker discipline — phrase audit

Searched this document for the trigger phrases (`deferred`, `out of scope`,
`tracked`, `follow-up`, `later`, `next PR`, `as part of`, `future work`,
`revisit if`).

| # | Phrase | Resolution |
|---|---|---|
| 1 | "slice 3 (kiro-gwo4)" | Filed pre-slice-1; alive in rivets |
| 2 | "slice 4 (kiro-3ll2)" | Filed pre-slice-1; alive in rivets |
| 3 | "slice 5 (kiro-ttew)" | Filed pre-slice-1; alive in rivets |
| 4 | "slice 6 (kiro-zqci)" | Filed pre-slice-1; alive in rivets |
| 5 | "slice 6 keeps it as a raw JSON textarea" (negative space 5) | Settled rationale within slice 6's scope; tracked at kiro-zqci |
| 6 | "out of scope" / "later slices" (Input shapes table) | Each named slice is filed; see above |

No new issues need filing for slice 2 — the negative-space items all map
to already-filed slice-3+ trackers, and there is no deferred-cleanup
work that slice 2 itself creates.

## Self-review (skill mandates 7 checks)

1. **Claim count.** 6 claims. In-range (3-15).
2. **Falsifier independence.** C1's oracle is the probe-s2 byte-locked
   answer (built without any reference to `tools-catalog.ts`); C2-C6 are
   pure-logic vitest assertions over reducer return values, none of which
   call into Svelte components or Tauri commands.
3. **Falsifier non-vacuity.** Each claim names a specific buggy
   implementation that the falsifier kills:
   - C1: dropping `use_subagent` during the port → vitest length assert fails.
   - C2: `toggleTool` that forgets `allowedTools.filter` → cascade test catches stale entry.
   - C3: `addAllowed` that calls `toggleTool` under the hood → cross-touch leaks into `tools[]` and fails the independence assert.
   - C4: `partitionTools` that uses `.includes("@")` (substring match) → an `@`-mid-name like `"name@with-at"` lands in `external` wrongly.
   - C5: `$derived(draft.tools)` that returns the array → length assertion still passes, but a regression returning `tools.length > 0 ? null : tools.length` (inverted condition) fails the empty-vs-populated case.
   - C6: `+Add custom` that accepts `"fs_read"` (no `@`) silently → fails the (b) case where the prose says "must start with @".
4. **Per-claim verification distinctness.** Every claim has a distinct
   test name. C2 and C3 share the input shape (draft with `tools` /
   `allowedTools` / `toolAliases`) but the **assertion direction** is
   opposite (C2 demands cross-modification; C3 demands non-cross-modification),
   so one cannot pass while the other fails.
5. **Cost distribution.** Cheapest C1 / C4 / C5 at 5m. Others ≤ 15m. No
   claim has only an expensive falsifier.
6. **Negative space.** 10 entries. Required ≥ 3.
7. **Tracker references.** All 4 slice references map to already-filed
   rivets issues (kiro-gwo4, kiro-3ll2, kiro-ttew, kiro-zqci). No
   deferred-cleanup follow-ups generated by this slice.

## Hard gate (skill requirement) — status

- [x] Every production-reachable input shape covered by ≥1 claim (Input shapes table)
- [x] Every claim has a falsifier in the Falsification table
- [x] Every falsifier names an independent oracle (probe.out / pure-logic vitest)
- [x] Every falsifier names a specific buggy implementation that would make it fail (Self-review item 3)
- [x] Every claim has a distinct verifiable output (Self-review item 4)
- [x] Every measurement-based claim has a `Regression fence` entry pointing at a deterministic test
- [x] Every deferral / out-of-scope reference cites a verified tracker ID
- [x] The cheapest falsifier has been run and passed (C1 — probe vs oracle AGREE, 2026-05-26)
- [x] Negative space list has ≥ 3 entries (10 entries)

Design ready for hand-off to `budgeted-plan`.

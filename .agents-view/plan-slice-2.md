# Budgeted Plan — agents-view slice 2

Source: design at `./design-slice-2.md` (6 claims, cheapest falsifier C1 passed
via probe-s2 agreement). Spec: `./spec.md`. Prove-it-prototype:
`./probe-s2/README.md` (AGREE on 15 tools / 9 categories, 2026-05-26).

Each sub-slice below is independently reviewable. The contract is the slice's
**Claim / Oracle / Stress fixture / Loop budget / Files / Verification**
fields — the code blocks under "Code (advisory)" are suggestions, not
dictation. Implementation order is dependency-driven: each slice's tests
assume the previous slice's code is present. The whole sequence is one PR;
sub-slices become individual commits for reviewable bisection.

---

## S1 — `tools-catalog.ts` static port + vitest

**Claim:** C1. The frozen catalog matches the probe's locked answer.

**Oracle:** `.agents-view/probe-s2/probe.out` (15 tools, 9 categories,
names+summaries locked 2026-05-26).

**Stress fixture:** vitest cases:
1. `TOOLS_CATALOG.length === 15`
2. `[...new Set(TOOLS_CATALOG.map(t => t.category))].sort()` equals the
   probe's category list verbatim
3. Every entry in `probe.out#/tools` appears in `TOOLS_CATALOG` with
   matching `name`, `category`, `summary` (driven by `it.each(...)`)
4. `Object.isFrozen(TOOLS_CATALOG)` is `true` AND attempting to
   mutate an entry throws in strict mode (TS strict is on)

A regression that drops `use_subagent` during port falsifies (1) and
the per-entry assertion in (3). A regression that drifts a `summary`
string falsifies (3) for that one entry without touching the count.

**Loop budget:** N/A — module-level constant.

**Files:**
- `crates/kiro-control-center/src/lib/agents/tools-catalog.ts` (new, ~50 LOC: the catalog literal + type)
- `crates/kiro-control-center/src/lib/agents/tools-catalog.test.ts` (new, ~40 LOC)

**Code (advisory):**
```ts
// tools-catalog.ts
export type ToolCategory =
  | "Cloud" | "Code" | "Filesystem" | "Meta"
  | "Orchestration" | "Planning" | "Reasoning" | "Shell" | "Web";

export type Tool = {
  readonly name: string;
  readonly category: ToolCategory;
  readonly summary: string;
};

// Source-of-truth: design bundle's agents-data.js, locked via
// .agents-view/probe-s2/. Tools listed in source order so a visual
// diff during port review matches line-by-line.
export const TOOLS_CATALOG: readonly Tool[] = Object.freeze([
  { name: "fs_read",      category: "Filesystem",    summary: "Read files, directories, and images" },
  { name: "fs_write",     category: "Filesystem",    summary: "Create, edit, insert into files" },
  // ... 13 more
]) as readonly Tool[];
```

**Verification:**
- [ ] All 4 vitest cases pass
- [ ] `npm run check` passes
- [ ] **No `console.log` / no IO** — pure module-level constant
- [ ] Catalog import lands at <1 KB gzipped (`npm run build` size delta)

---

## S2 — `tool-state.ts` pure reducers + vitest

**Claim:** C2, C3, C4, C6. The four reducers that drive draft mutation.

**Oracle:** vitest assertions on returned draft objects (deep equality).

**Stress fixture:** vitest test groups by reducer:

`toggleTool(draft, name)`:
1. **Disable path with full cascade**: pre-build `{tools: ["fs_read"], allowedTools: ["fs_read"], toolAliases: {fs_read: "read"}}`; toggle "fs_read"; assert all three are empty arrays / empty object
2. **Disable path with partial state**: pre-build `{tools: ["fs_read"], allowedTools: ["other"], toolAliases: {}}`; toggle "fs_read"; assert `tools = []`, `allowedTools` unchanged (other tool's allowed status intact), `toolAliases` unchanged
3. **Enable path**: pre-build empty; toggle "fs_read"; assert `tools = ["fs_read"]`, others unchanged
4. **Idempotence guard against double-call**: enable then enable again; assert `tools = ["fs_read"]` (no duplicates)

`addAllowed(draft, name)`:
5. Add to empty allowedTools, name not in tools[] — accepted (the "yellow chip / NOT VISIBLE" state per design § 5)
6. Add duplicate — returns unchanged
7. Empty name — returns unchanged

`removeAllowed(draft, name)`:
8. Remove existing — drops only from `allowedTools`; `tools` + `toolAliases` unchanged
9. Remove non-existent — returns unchanged (idempotent)

`partitionTools(tools)`:
10. Pure native — `["fs_read", "grep"]` → `{native: ["fs_read", "grep"], external: []}`
11. Pure MCP — `["@svc/foo", "@bar"]` → `{native: [], external: ["@svc/foo", "@bar"]}`
12. Mixed — partition preserves source order within each group
13. Edge: `["name@embedded"]` — must NOT match `external`; substring-`includes("@")` bug falsified here

`addCustomTool(draft, raw)`:
14. Whitespace-only — `{ok: false, reason: "empty"}`
15. Non-`@`-prefixed — `{ok: false, reason: "not-mcp"}` (e.g. `"fs_read"`)
16. Well-formed new — `{ok: true, draft: {tools: ["@svc/foo"], allowedTools: ["@svc/foo"], ...}}` (both fields appended)
17. Duplicate — `{ok: false, reason: "duplicate"}` (already in `tools[]`)
18. Trimmed input — `"  @svc/foo  "` → `"@svc/foo"`, accepted

A naive `tools.filter(t => t !== name)` in `toggleTool` falsifies (1).
A naive `allowedTools = tools` shortcut in `removeAllowed` falsifies (8).
A `.includes("@")` substring-match in `partitionTools` falsifies (13).

**Loop budget:** All reducers are O(|tools|). Production scale ≤ 50
tools per draft (15 native + ~35 MCP would be high). Per-call ops ≤ 100.

**Files:**
- `crates/kiro-control-center/src/lib/agents/tool-state.ts` (new, ~80 LOC)
- `crates/kiro-control-center/src/lib/agents/tool-state.test.ts` (new, ~150 LOC)

**Code (advisory):**
```ts
import type { AgentDraft } from "$lib/agent-draft";  // existing slice-1 type

type ToolsDraft = Pick<AgentDraft, "tools" | "allowedTools" | "toolAliases">;

export function toggleTool(draft: ToolsDraft, name: string): ToolsDraft {
  if (draft.tools.includes(name)) {
    // Cascade per C2: scrub all three fields. The watch-out invariant
    // from the spec's "Things to watch out for" #8.
    const { [name]: _alias, ...remainingAliases } = draft.toolAliases;
    return {
      tools: draft.tools.filter((t) => t !== name),
      allowedTools: draft.allowedTools.filter((t) => t !== name),
      toolAliases: remainingAliases,
    };
  }
  return { ...draft, tools: [...draft.tools, name] };
}

export function partitionTools(
  tools: readonly string[],
): { native: string[]; external: string[] } {
  const native: string[] = [];
  const external: string[] = [];
  for (const t of tools) {
    // Anchored at index 0 — not `.includes("@")`. A name like
    // "name@embedded" must route to `native`. C4 / case (13) locks it.
    if (t.startsWith("@")) external.push(t);
    else native.push(t);
  }
  return { native, external };
}

export type AddCustomResult =
  | { ok: true; draft: ToolsDraft }
  | { ok: false; reason: "empty" | "not-mcp" | "duplicate" };

export function addCustomTool(
  draft: ToolsDraft,
  raw: string,
): AddCustomResult {
  const trimmed = raw.trim();
  if (!trimmed) return { ok: false, reason: "empty" };
  if (!trimmed.startsWith("@")) return { ok: false, reason: "not-mcp" };
  if (draft.tools.includes(trimmed)) return { ok: false, reason: "duplicate" };
  return {
    ok: true,
    draft: {
      ...draft,
      tools: [...draft.tools, trimmed],
      allowedTools: draft.allowedTools.includes(trimmed)
        ? draft.allowedTools
        : [...draft.allowedTools, trimmed],
    },
  };
}

// addAllowed / removeAllowed elided — same shape, single field mutation.
```

**Verification:**
- [ ] All 18 vitest cases pass
- [ ] `npm run check` passes
- [ ] `tool-state.ts` has no Svelte imports — pure TS module
- [ ] Discriminated `AddCustomResult` includes a `_exhaustive: never` consumer in S3 (the panel) so a future reason arm forces a compile error

---

## S3 — `toolsRailBadge` helper + vitest

**Claim:** C5. The rail count derives cleanly from draft state.

**Oracle:** vitest equality assertions.

**Stress fixture:** vitest cases:
1. `toolsRailBadge({tools: []})` → `null`
2. `toolsRailBadge({tools: ["fs_read"]})` → `1`
3. `toolsRailBadge({tools: ["fs_read", "@svc/foo"]})` → `2` (mixes native + MCP — both count)
4. **Adversarial**: pass a frozen `tools` array — must not mutate it

A regression that returns `tools.length || null` would still pass (1)/(2)/(3)
but a future change to "count native only" would silently flip behavior;
the explicit vitest case for mixed-content (3) keeps the contract honest.

**Loop budget:** N/A.

**Files:**
- `crates/kiro-control-center/src/lib/agents/tool-state.ts` (append, ~10 LOC)
- `crates/kiro-control-center/src/lib/agents/tool-state.test.ts` (append, ~20 LOC)

**Code (advisory):**
```ts
export function toolsRailBadge(draft: Pick<ToolsDraft, "tools">): number | null {
  return draft.tools.length === 0 ? null : draft.tools.length;
}
```

**Verification:**
- [ ] All 4 vitest cases pass
- [ ] Live wiring in S5 uses `$derived(toolsRailBadge(draft))`

---

## S4 — `AllowedToolsList.svelte` (auto-allowed picker subcomponent)

**Claim:** Spec § 5 of design — the top region of the Tools section.

**Oracle:** Visual fidelity to `screenshots/04-tools-allowed-picker.png`.
Logic correctness flows from S2's `addAllowed` / `removeAllowed` /
`addCustomTool` reducers (already falsified).

**Stress fixture:** Manual visual check + the S6 component-wiring assertion
that the picker's `+Add custom` callback routes through `addCustomTool`
(not a direct `tools.push`). A plausible bug: the picker bypasses
`addCustomTool` and calls a local `push` that skips the `@`-prefix
validation. The S2 reducer test passes, but the user can still type
`"fs_read"` and get a non-MCP entry into `tools[]`. Mitigated by:
- the picker's UI only mounts `addCustomTool`'s callback — no escape hatch
- the S6 wiring test asserts the dispatched event payload IS the
  reducer's return shape (`AddCustomResult`)

**Loop budget:** `{#each enabled}` + `{#each catalog}` for suggestion
list. ≤ 50 + 15 = 65 items. Trivial.

**Files:**
- `crates/kiro-control-center/src/lib/components/editor/AllowedToolsList.svelte` (new, ~120 LOC mostly markup)

**Code (advisory):** See `AgentEditor.jsx` lines 336–453 (`AllowedToolsList`)
for the React reference. Port to Svelte 5 runes:
- props: `allowed: readonly string[]`, `enabled: readonly string[]`,
  `catalog: readonly Tool[]`, `onAdd: (name: string) => void`,
  `onRemove: (name: string) => void`, `onAddCustom: (raw: string) => AddCustomResult`
- state: local `pickerQuery: $state("")` + `pickerOpen: $state(false)`
- The "NOT VISIBLE" yellow chip per design — render when an
  `allowed` entry is NOT in `enabled` (the yellow-chip state)

**Verification:**
- [ ] Component renders without TS errors
- [ ] `+Add custom` button calls `onAddCustom`, surfaces `{ok: false}` reason as inline error
- [ ] Yellow chip rendered iff `allowed[i]` is NOT in `enabled` (per design)
- [ ] **LOC overage** acknowledged: ~120 LOC mostly markup; logic in S2

---

## S5 — `ToolsPanel.svelte` (three sub-regions)

**Claim:** Spec § 5 of design — the section composition.

**Oracle:** Visual fidelity to `screenshots/04-tools.png`. Logic comes
from S2's `partitionTools` (already falsified for C4).

**Stress fixture:** Manual visual check. Plausible bug: panel renders
the by-category grid against `draft.tools` filtered by category instead
of against `TOOLS_CATALOG`. Result: native tools NOT in the agent's
`tools[]` don't appear in the grid (user can never enable them). Caught
by S6 (the editor's smoke render with an empty draft) — the grid must
show all 15 catalog entries even when `draft.tools` is empty.

**Loop budget:** `{#each categories}` + nested `{#each tools-in-cat}`.
Production: 9 + 15 = 24 iterations. Trivial.

**Files:**
- `crates/kiro-control-center/src/lib/components/editor/ToolsPanel.svelte` (new, ~180 LOC)

**Code (advisory):**
- Props: `draft: ToolsDraft`, `onChange: (next: ToolsDraft) => void`
- Sub-region 1 (top): `<AllowedToolsList ... />` consuming `onAdd`/`onRemove`/`onAddCustom` that route through S2 reducers and call `onChange` with the new draft
- Sub-region 2 (middle): `{#each categoryOrder as cat}` rendering the
  catalog filtered to `t.category === cat`; each tool's checkbox
  toggles via `toggleTool` reducer; alias input per `AgentEditor.jsx:283`
  inline (no separate panel — design § 5 keeps it on the row)
- Sub-region 3 (bottom): `partitionTools(draft.tools).external` rendered
  as a chip list with remove-X buttons; an `<input type="text" />` +
  `+Add` button routes through `addCustomTool`

The `categoryOrder` constant pins the visual order to match the
screenshots; not derived from `TOOLS_CATALOG` (otherwise the order
silently drifts when the catalog is re-sorted alphabetically by name).

**Verification:**
- [ ] All 15 native tools render across 9 categories even with empty `draft.tools`
- [ ] Checking a checkbox calls `toggleTool` and propagates via `onChange`
- [ ] External MCP entries render at the bottom, separated visually
- [ ] **LOC overage** acknowledged: ~180 LOC mostly markup

---

## S6 — Enable the Tools section in `AgentEditor.svelte`

**Claim:** C5 (rail badge live) + spec § 5 integration.

**Oracle:** Visual smoke: opening the editor, the Tools section is
clickable in the rail (no longer disabled), renders `ToolsPanel`, and
the rail badge shows `null` initially (empty `draft.tools`) then `1`
after toggling one tool.

**Stress fixture:** A regression that wires `ToolsPanel` but forgets to
flip the `disabled` flag on the rail entry → the section renders but the
rail button is unclickable. Caught by manual visual check + the existing
slice-1 e2e walkthrough (which won't fail per se, but the reviewer
notices). Cheaper structural test: a vitest snapshot on the
`SECTIONS` literal asserting `id: "tools"` has `enabled: true` (or no
`enabled` field, depending on the current shape).

**Loop budget:** N/A.

**Files:**
- `crates/kiro-control-center/src/lib/components/AgentEditor.svelte` (modified ~10 LOC: enable "tools" in `SECTIONS`, route the active-section branch to `<ToolsPanel>`, wire `onChange` to update `draft.tools` / `allowedTools` / `toolAliases` together)

**Code (advisory):**
```svelte
<script lang="ts">
  // SECTIONS literal: drop the `enabled: false, note: "Slice 2"` on tools.
  // Add to the active-section dispatch:
  //   {#if active === "tools"}
  //     <ToolsPanel
  //       draft={{ tools: draft.tools, allowedTools: draft.allowedTools, toolAliases: draft.toolAliases }}
  //       onChange={(next) => Object.assign(draft, next)}
  //     />
  //   {/if}
  //
  // Rail count:
  //   {#if active.id === "tools"}
  //     {@const badge = toolsRailBadge(draft)}
  //     {#if badge !== null}<span class="...">{badge}</span>{/if}
  //   {/if}
</script>
```

**Verification:**
- [ ] `SECTIONS[2].enabled` is no longer `false` (or the analogous shape)
- [ ] Active-section branch renders `<ToolsPanel>` for `active.id === "tools"`
- [ ] Rail badge derives via `$derived(toolsRailBadge(draft))`
- [ ] Save round-trip works: open editor → toggle 2 tools → save → reload list → re-open same agent → 2 tools still toggled (manual smoke test, not a new e2e)

---

## S7 — Extend Playwright e2e with one tools assertion

**Claim:** Round-trip survives the `serde_json::Value` pipeline (slice-1
C2 already establishes the principle; this exercises it for the tools
fields specifically).

**Oracle:** Filesystem inspection of the saved agent's JSON.

**Stress fixture:** Mid-test, after slice-1's "create + edit identity"
path, click into the Tools section, toggle one native tool (`fs_read`),
toggle one custom MCP tool (`@svc/foo`), save. Read
`<project>/.kiro/agents/<name>.json` and assert:
- `tools` array contains both `"fs_read"` and `"@svc/foo"`
- `allowedTools` array contains `"@svc/foo"` (the +Add custom flow auto-allows)
- `toolAliases` is `{}` (no alias UI exercised in this test)

A regression where the panel mutates `draft.tools` but the editor's
save handler omits the field from the serialized payload → assertion
fails because `tools` is missing from the JSON.

**Loop budget:** N/A.

**Wall budget:** ≤ 60s addition to existing e2e total (CI-conservative).

**Files:**
- `crates/kiro-control-center/tests/e2e/agents.spec.ts` (modified — append ~30 LOC: one new test path "create agent with tools, persist round-trip")

**Verification:**
- [ ] `npm run test:e2e -- --grep "tools"` passes (gated on `FIXTURE_MARKETPLACE_PATH` per slice-1 convention)
- [ ] Test reads JSON from disk and asserts the three fields explicitly

---

## Cross-slice verification

After all 7 sub-slices land, run the standard pre-commit sweep:

- `cargo fmt --all --check`
- `cargo clippy --workspace --tests -- -D warnings`
- `cargo test --workspace`
- `cd crates/kiro-control-center && npm run check`
- `cd crates/kiro-control-center && npm run test:unit`
- `cd crates/kiro-control-center && npm run test:e2e`
- `cargo run -p xtask -- comment-lint` — green (no new PR/issue/reviewer references; the slice-2 work has no PR-fallout tail yet)
- `cargo test -p kiro-control-center --lib -- --ignored` — `bindings.ts` unchanged (slice 2 doesn't add new Tauri commands, so this should be a no-op)

If `bindings.ts` IS modified, it indicates an accidental new IPC type
landed — investigate before committing.

## Slice-2 success criteria recap (from spec)

| # | Criterion | This slice's coverage |
|---|---|---|
| S3 | Type-check, lint, format pass | Cross-slice verification above |
| S5 | Editor's emitted JSON parses cleanly through `parse_native.rs` | Already locked by slice 1's S5 round-trip test; slice 2 only adds JSON-shape data, no new Rust types crossed |
| S7 | Visual fidelity to design bundle | Manual review against `screenshots/04-tools.png` + `04-tools-allowed-picker.png` |
| S8 (full feature) | All 7 sections + KB modal implemented | Slice 2 contributes the Tools section only; later slices fill the rest |

## Open questions (none blocking)

None. The probe locked the catalog answer; the spec's "things to watch
out for" #8 is encoded as C2; the cross-section cleanup invariant is
expressed as a pure reducer.

If a question surfaces during implementation that requires deviation
from this plan, treat it the same way slice 1's S13 amendments were
treated: append an "Amendment YYYY-MM-DD" block to this plan, run any
new falsifier, and proceed.

## Plan ready for `checkpointed-build`

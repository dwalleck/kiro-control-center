# F3: Inline per-failure UI in InstalledTab.svelte — design 2026-05-11

Implements `kiro-kmj4` (and, by same-commit coupling, closes `kiro-5qcb`). Builds on the
tagged-enum `FailedAgent` wire format shipped in PR #113
(`docs/plans/2026-05-09-failed-agent-discriminator-design.md` §F3).

## Problem

Today, after `runPluginInstall` returns from an Update action in
`InstalledTab.svelte`, the user sees only the count-level banner produced
by `formatInstallPluginResult` (e.g. `"Updated demo-plugin: 1 steering
failed · 8 agents failed"`). The per-item reasons that the Rust backend
already sends — `FailedSkill.error`, `FailedSteeringFile.error`,
`FailedAgent.error` (each carrying the full chain via `error_full_chain`) —
are not surfaced inline.

A temporary `console.error` diagnostic in
`crates/kiro-control-center/src/lib/plugin-actions.ts:130-149` dumps the
failure arrays to DevTools so the per-item context isn't lost entirely.
This was scaffolding by design (commented as such) and is documented as
removable when the inline UI lands.

The visual target is the existing `runPluginRemove` per-failure `<details>`
panel at `crates/kiro-control-center/src/lib/components/InstalledTab.svelte:280-324`,
which already renders skills/steering/agents removed-plus-failures after
a remove action. The install path needs the same treatment.

## Design

Three units land together. Boundaries chosen so the visible UI work, the
discriminator-pushdown discipline, and the outcome-shape change can each
be reviewed independently.

### Unit 1 — Format helpers (`format.ts`)

Three new pure functions, joining the existing family
(`formatSkippedSkill`, `formatInstallWarning`, `formatSteeringWarning`,
`formatParseFailure`):

| Helper                       | Input                       | Discriminator |
|------------------------------|-----------------------------|---------------|
| `formatFailedSkill`          | `FailedSkill`               | `kind.kind` on `FailedSkillReason` |
| `formatFailedSteeringFile`   | `FailedSteeringFile_Serialize` | (single shape — no switch) |
| `formatFailedAgent`          | `FailedAgent` (= `FailedAgent_Serialize`) | `entry.kind` over `agent` / `unparseable_agent` / `companion_bundle` |

`formatFailedAgent` is the load-bearing case. The switch shape is
already documented in `plugin-actions.ts:160-172`; this PR implements it
verbatim and pairs the runtime switch with a value-position
exhaustiveness assert anchored to `FailedAgent["kind"]` per CLAUDE.md
discriminator-pushdown discipline (precedent: `_PLUGIN_ACTION_VALUES` +
`_AssertPluginActionExhaustive` in `stores/plugin-updates.ts:135-137`):

```ts
const _FAILED_AGENT_KINDS = ["agent", "unparseable_agent", "companion_bundle"]
  as const satisfies readonly FailedAgent["kind"][];
type _AssertFailedAgentKindExhaustive =
  Exclude<FailedAgent["kind"], (typeof _FAILED_AGENT_KINDS)[number]> extends never ? true : never;
const _assertFailedAgentKindExhaustive: _AssertFailedAgentKindExhaustive = true;
```

The same value-position assert is applied to `FailedSkillReason["kind"]`
inside `formatFailedSkill` (two variants today — `install_failed`,
`requested_but_not_found`).

Rendering shape per variant:

| Variant                              | Label form |
|--------------------------------------|------------|
| `FailedSkill { name, error, kind }`  | `${name} — ${error}` (kind narrows the surrounding context message) |
| `FailedSteeringFile { source, error }` | `${source} — ${error}` |
| `FailedAgent::Agent { name, source_path, error }` | `${name} (${source_path}) — ${error}` |
| `FailedAgent::UnparseableAgent { source_path, error }` | `${source_path} (unparseable) — ${error}` |
| `FailedAgent::CompanionBundle { plugin, conflicts, error }` | `${plugin} bundle [${conflicts.join(", ") || "no enumeration"}] — ${error}` |

### Unit 2 — Outcome shape (`plugin-actions.ts`)

Extend `PluginActionOutcome`'s `ok` arm to carry the install result so
the caller can render per-item failures:

```ts
export type PluginActionOutcome =
  | { kind: "ok"; banner: PluginBanner; installResult: InstallPluginResult_Serialize }
  | { kind: "fail"; error: string };
```

Backwards-compatible: `BrowseTab.svelte` (the other consumer at
`components/BrowseTab.svelte:790`) keeps reading `outcome.banner` and
ignores the new field. `InstalledTab.svelte` opts in.

### Unit 3 — Panel state + markup (`InstalledTab.svelte`)

New `$state` vars mirroring the existing `removeResult` / `removeResultPlugin`:

```ts
let installResult: InstallPluginResult_Serialize | null = $state(null);
let installResultPlugin: string | null = $state(null);
let installResultHasFailures = $derived.by(() => {
  if (installResult === null) return false;
  return (
    installResult.skills.failed.length +
      installResult.steering.failed.length +
      installResult.agents.failed.length >
    0
  );
});
```

The panel is a direct twin of the remove panel
(`InstalledTab.svelte:280-324`), gated on `installResult &&
installResultPlugin && installResultHasFailures`. Container color:
warning when `anyInstalled && anyFailed`, error when `!anyInstalled &&
anyFailed`. `<details open={true}>` (always-open since the panel only
appears when there's something to show). Body sections per non-empty
failure slice, each entry rendered through the matching `format*` helper.

Reset rules (mirror remove panel exactly):

- At the top of `updatePlugin()` — clear before action.
- In the project-switch effect (`InstalledTab.svelte:248-261`) — add
  `installResult = null; installResultPlugin = null;`.
- Dismiss-× button clears both.

The setter inside `updatePlugin`'s `outcome.kind === "ok"` branch stores
`installResult` only when the failure sum is positive — same shape as
the `$derived` above (inline `(skills.failed.length +
steering.failed.length + agents.failed.length) > 0`), not via a second
call to `formatInstallPluginResult` (which would recompute summary
parts for no reason and create a drift risk against the `$derived`).
This avoids leaving stale state when a clean install completes (panel
won't render anyway, but the state is cleaner).

## Wire format

No Rust changes. No `bindings.ts` regen needed. The change is FE-internal
plumbing of an already-serialized result.

TS surface diff:

```ts
// Before
export type PluginActionOutcome =
  | { kind: "ok"; banner: PluginBanner }
  | { kind: "fail"; error: string };

// After
export type PluginActionOutcome =
  | { kind: "ok"; banner: PluginBanner; installResult: InstallPluginResult_Serialize }
  | { kind: "fail"; error: string };
```

## Consumer impact

| File | Change |
|------|--------|
| `crates/kiro-control-center/src/lib/format.ts` | +3 new helpers (`formatFailedSkill`, `formatFailedSteeringFile`, `formatFailedAgent`) with value-position exhaustiveness asserts |
| `crates/kiro-control-center/src/lib/plugin-actions.ts` | `PluginActionOutcome.ok` adds `installResult`. Remove diagnostic block (lines 130-149). Remove forward-looking F3 comment block (lines 160-172). |
| `crates/kiro-control-center/src/lib/components/InstalledTab.svelte` | +2 state vars, +1 `$derived`, panel markup mirroring `removeResult` panel, reset rules in `updatePlugin()` + project-switch effect, dismiss handler. |
| `crates/kiro-control-center/src/lib/components/BrowseTab.svelte` | No code change. Continues consuming `outcome.banner` only; the new `installResult` field is ignored (backwards-compatible). |
| `crates/kiro-control-center/src/lib/plugin-actions.test.ts` | Extend "install success" test to assert `outcome.installResult` shape. Add "install with failures" test verifying the failures arrays survive intact. |
| `crates/kiro-control-center/src/lib/format.test.ts` | +3 test blocks, one per new formatter, covering each discriminated variant + happy path. |

## Coupled cleanup (closes `kiro-5qcb`)

Same commit removes:

- `plugin-actions.ts:130-149` — the `console.error` diagnostic block (was
  always scaffolding; comment explicitly notes "Follow-up work will
  surface these failures inline in the UI").
- `plugin-actions.ts:160-172` — the forward-looking F3 switch-shape
  comment block (implementation now exists in `format.ts`).

PR description names both issues so `rivets close kiro-5qcb` and
`rivets close kiro-kmj4` can both be wired to the merge.

## Testing

**Vitest (pure helpers, per CLAUDE.md "no jsdom, no
`@testing-library/svelte`"):**

- `format.test.ts` — three new `describe` blocks:
  - `formatFailedSkill`: install_failed variant; requested_but_not_found variant; assertNever fires for unknown kind (use `@ts-expect-error` to construct the invalid value).
  - `formatFailedSteeringFile`: single-shape case (one happy-path assertion).
  - `formatFailedAgent`: agent variant; unparseable_agent variant; companion_bundle variant (length-0 conflicts and length-1 conflicts); assertNever fires for unknown kind.
- `plugin-actions.test.ts`:
  - Extend the existing "install success" test (`describe("runPluginInstall")` line 76+) to assert `outcome.installResult` equals the IPC result.
  - Add new test: "install with failures returns failures intact on outcome.installResult" — populate `r.skills.failed`, `r.steering.failed`, `r.agents.failed` with the three FailedAgent variants and assert each survives.

**Component-level (panel render, dismiss, project-switch reset):**
out of scope per CLAUDE.md ("Component-level testing is intentionally
future scope"). Validated manually instead.

**Manual (dev server):**
1. `cd crates/kiro-control-center && npm run dev`
2. In the kiro-control-center project itself (the canonical "agents
   committed to git" reproducer per the F3 design doc), trigger an
   Update on a plugin known to produce companion-bundle conflicts.
3. Verify the inline panel renders with per-agent failure text.
4. Verify dismiss-× clears the panel without affecting the banner.
5. Verify switching projects clears the panel.

## Migration plan

1. **`format.ts`** — add the three helpers + exhaustiveness asserts.
   Write the assertion-bearing tests in `format.test.ts` first
   (TDD-style); helpers second.
2. **`plugin-actions.ts`** — extend `PluginActionOutcome.ok` arm with
   `installResult`. Update both `outcome.kind === "ok"` return sites
   (one for install success). Remove `console.error` diagnostic and
   forward-looking comment block.
3. **`plugin-actions.test.ts`** — fix existing tests (the outcome shape
   widened; existing assertions still hold but the type may need a
   narrowing) and add the new "install with failures" test.
4. **`InstalledTab.svelte`** — add state, $derived, panel markup;
   wire reset rules in `updatePlugin()` and project-switch effect; add
   dismiss handler.
5. **Verify clean** — `npm run check`, `npm run test:unit`, manual
   dev-server validation.
6. **Workspace gates** — `cargo fmt --all --check`, `cargo test
   --workspace`, `cargo clippy --workspace --tests -- -D warnings`.
7. **Rivets bookkeeping** — `rivets close kiro-kmj4 --reason "PR #NNN:
   inline per-failure UI"` and `rivets close kiro-5qcb --reason "PR
   #NNN: diagnostic removed with F3"` on merge.

## Follow-on work (intentionally not in this PR)

### B1. `BrowseTab.svelte` adopts the panel

`BrowseTab.svelte` also calls `runPluginInstall` (line 790). It currently
ignores the new `installResult` field. A future PR can lift the same
panel markup into a shared `InstallResultPanel.svelte` component (or
just duplicate it — the existing remove panel is duplicated by the
install panel here too) and surface per-item failures on the catalog
install path.

**Why deferred:** issue scopes to `InstalledTab.svelte`; widening to
`BrowseTab` is independent leverage that can ship on its own timeline.

### B2. Installed-items section inside the panel

The current panel shows failures only. A future expansion could mirror
the remove panel exactly by also listing successfully-installed items
(skill names, steering destinations, agent names). The count banner
already covers this at summary level — adding per-item rendering is a
preference, not a bug fix.

**Why deferred:** YAGNI for first cut. The failure surface is the
load-bearing user need; success-detail is bonus.

### B3. Component-level test coverage

Once the project decides to adopt `@testing-library/svelte` or an
equivalent, panel render + dismiss + project-switch-reset are the kind
of behaviors that benefit from component-level tests. Not deferred to
a tracked issue — it's a project-wide testing-policy decision, not
F3-specific work.

## Risks

- **Outcome shape widening breaks `BrowseTab.svelte`.** Mitigation:
  field is additive, BrowseTab reads `outcome.banner` only and never
  destructures the rest. `npm run check` will catch any structural
  miss.
- **Discriminator-pushdown drift.** A future `FailedAgent` variant added
  on the Rust side must update `_FAILED_AGENT_KINDS` *and* the switch.
  The value-position assert is what makes that lapse a compile error
  rather than a silent runtime fallthrough. Tests cover the assertNever
  path explicitly.
- **Same-commit coupling discipline.** `kiro-5qcb` (diagnostic
  removal) and `kiro-kmj4` (this UI) must land together. Splitting them
  re-introduces the per-item visibility gap (if diagnostic removed
  first) or noise (if removed after). PR description must name both.

## Verification gates (CLAUDE.md plan-review checklist)

- **Gate 1 — Grounding**: every change cites a specific file:line in
  the "Consumer impact" table and the migration plan steps. ✓
- **Gate 2 — Threat Model**: no new attack surface — internal FE
  plumbing of already-validated wire data. ✓
- **Gate 3 — Wire Format**: TS surface diff for `PluginActionOutcome`
  documented above; no Rust/bindings change. ✓
- **Gate 4 — External Type Boundary**: no external error types crossed.
  `AgentError` already projected to `string` via `serialize_agent_error`
  in PR #113; this PR consumes that string opaquely. ✓
- **Gate 5 — Type Design**: discriminator-pushdown discipline (switch +
  value-position exhaustiveness assert) applied to both
  `FailedAgent["kind"]` and `FailedSkillReason["kind"]`. ✓
- **Gate 6 — Reference vs Transcription**: design cites the existing
  remove-panel mechanism (`InstalledTab.svelte:280-324`) and the
  `_PLUGIN_ACTION_VALUES` exhaustiveness precedent
  (`stores/plugin-updates.ts:135-137`), not just the output shape they
  produce. ✓

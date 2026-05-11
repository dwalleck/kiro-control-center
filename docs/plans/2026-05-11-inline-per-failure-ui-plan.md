# F3 Inline Per-Failure UI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.
>
> **Plan style:** This plan deliberately omits copy-paste-ready code bodies. Each step specifies the signature/contract, the precedent file you must read first, the decisions and their rationale, and the verification hint. You are expected to engage with each design choice — not transcribe. If you find yourself wanting a literal code block, re-read the precedent and the design doc instead. (Rationale: feedback memory `feedback_plans_interfaces_not_code.md`.)

**Goal:** Render per-item install/update failures inline in `InstalledTab.svelte`, consuming the tagged-enum `FailedAgent` wire shape from PR #113. Close `kiro-kmj4` and `kiro-5qcb` in the same commit.

**Architecture:** Three pure render helpers in `format.ts` (with discriminator-pushdown exhaustiveness asserts) → outcome-shape extension in `plugin-actions.ts` carries the install result → twin `<details>` panel in `InstalledTab.svelte` mirroring the existing `runPluginRemove` panel. Failures-only display. `BrowseTab.svelte` unchanged (ignores new outcome field).

**Spec:** `docs/plans/2026-05-11-inline-per-failure-ui-design.md` (read this first; tasks below assume the spec is loaded).

**Tech Stack:** TypeScript, Svelte 5 (runes), Vitest. No Rust changes; no `bindings.ts` regeneration.

---

## File structure

| File | Change kind | Responsibility |
|------|-------------|----------------|
| `crates/kiro-control-center/src/lib/format.ts` | modify | Three new render helpers + exhaustiveness asserts |
| `crates/kiro-control-center/src/lib/format.test.ts` | modify | Vitest coverage for the three helpers |
| `crates/kiro-control-center/src/lib/plugin-actions.ts` | modify | Widen `PluginActionOutcome.ok` arm; remove temporary diagnostic + forward-looking comment block |
| `crates/kiro-control-center/src/lib/plugin-actions.test.ts` | modify | Fix existing test for new outcome shape; add failures-carried test |
| `crates/kiro-control-center/src/lib/components/InstalledTab.svelte` | modify | State + `$derived` + panel markup + reset rules + dismiss |
| `crates/kiro-control-center/src/lib/components/BrowseTab.svelte` | unchanged | Consumes only `outcome.banner`; new field passes through silently |

All paths relative to repo root. Mental model: `format.ts` is the pure-logic layer (vitest-safe), `plugin-actions.ts` is the IPC-orchestration layer (vitest-safe via injection), `InstalledTab.svelte` is the composition layer (manual-only per CLAUDE.md).

---

## Prerequisites

- [ ] **P1: Branch setup**

Decide branch strategy. Two reasonable choices:

- **Continue on `chore/init-rivets-issue-tracker`** if you want F3 chained behind the rivets-init PR. F3 work depends on rivets being initialized to track issue status, so this is defensible.
- **New worktree off `main`** per CLAUDE.md worktree convention (`~/repos/kiro-marketplace-cli-f3-inline-failure-ui` with branch `feat/f3-inline-failure-ui`). Clean separation.

Default to the worktree option if you don't have a strong reason otherwise; the rivets section in CLAUDE.md is independent doc work that can land separately.

- [ ] **P2: Read the precedent files**

Before writing any code, read these in this order. Each is short.

1. `docs/plans/2026-05-11-inline-per-failure-ui-design.md` — the spec; load fully.
2. `crates/kiro-control-center/src/lib/format.ts` — existing render helpers (`formatSkippedSkill`, `formatInstallWarning`, `formatSteeringWarning`, `formatParseFailure`). These are the pattern you'll match.
3. `crates/kiro-control-center/src/lib/stores/plugin-updates.ts:115-145` — the canonical `_PLUGIN_ACTION_VALUES` + `_AssertPluginActionExhaustive` pattern. The new helpers' exhaustiveness asserts must follow this exact shape.
4. `crates/kiro-control-center/src/lib/plugin-actions.ts:130-172` — the temporary diagnostic block + the forward-looking F3 comment block. Both removed in Task 5.
5. `crates/kiro-control-center/src/lib/components/InstalledTab.svelte:39-49, 248-263, 280-324` — the `removeResult` state declarations, the project-switch reset effect, and the existing per-failure `<details>` panel. The install panel is a structural twin.
6. `crates/kiro-control-center/src/lib/plugin-actions.test.ts:17-100` — fixture builders (`emptyInstallResult`, `emptyRemoveResult`, `makeInstallCtx`) and the existing `runPluginInstall` test shape.
7. `crates/kiro-control-center/src/lib/bindings.ts` — search for `FailedAgent`, `FailedSkill`, `FailedSkillReason`, `FailedSteeringFile_Serialize` to confirm field names. Do not assume — read.

---

## Task 1: `formatFailedSteeringFile` (simplest, no switch)

**Files:**
- Modify: `crates/kiro-control-center/src/lib/format.ts`
- Test: `crates/kiro-control-center/src/lib/format.test.ts`

- [ ] **Step 1: Write the failing test**

In `format.test.ts`, add `describe("formatFailedSteeringFile")` after the existing format-helper describes. Cases:

1. **Happy path** — input has `source: "some/file.md"`, `error: "permission denied: foo"`. Expected output: the render shape from design doc §Format helpers (the `${source} — ${error}` row).

Pattern precedent: any existing `describe("formatXxx")` block in this file (e.g. `formatSkippedSkill`). Use bare object literals; no helper needed for a single case.

Type imports: `FailedSteeringFile_Serialize` from `$lib/bindings`.

- [ ] **Step 2: Run the test and verify it fails**

```bash
cd crates/kiro-control-center && npm run test:unit -- format.test.ts
```

Expected: test fails with "formatFailedSteeringFile is not a function" or similar import error.

- [ ] **Step 3: Implement `formatFailedSteeringFile` in `format.ts`**

Signature: `export function formatFailedSteeringFile(f: FailedSteeringFile_Serialize): string`

Render shape: per design doc §Format helpers table (the `FailedSteeringFile` row).

Decision: no switch needed — `FailedSteeringFile` is a single-shape type today. If it grows variants in the future (per the deferred F2 in `docs/plans/2026-05-09-failed-agent-discriminator-design.md`), revisit with the same discriminator-pushdown pattern then. Do **not** preemptively wrap in a switch; YAGNI.

Type import: `FailedSteeringFile_Serialize` from `$lib/bindings`. (Note the `_Serialize` suffix — bindings.ts exports both serialize and deserialize variants; pick the serialize one for FE-from-Rust consumption.)

- [ ] **Step 4: Run the test and verify it passes**

```bash
cd crates/kiro-control-center && npm run test:unit -- format.test.ts
```

Expected: PASS.

- [ ] **Step 5: Type check + commit**

```bash
cd crates/kiro-control-center && npm run check
```

Expected: zero errors.

```bash
git add crates/kiro-control-center/src/lib/format.ts crates/kiro-control-center/src/lib/format.test.ts
git commit -m "$(cat <<'EOF'
feat(format): add formatFailedSteeringFile render helper

Single-shape render helper joining the existing format.ts family.
No switch (FailedSteeringFile is a single shape today); revisit when
F2 — the parallel tagged-enum restructure — lands.

Refs: docs/plans/2026-05-11-inline-per-failure-ui-design.md
EOF
)"
```

---

## Task 2: `formatFailedSkill` (FailedSkillReason switch)

**Files:**
- Modify: `crates/kiro-control-center/src/lib/format.ts`
- Test: `crates/kiro-control-center/src/lib/format.test.ts`

- [ ] **Step 1: Write the failing test**

In `format.test.ts`, add `describe("formatFailedSkill")`. Cases:

1. **`install_failed` variant** — input has `name: "my-skill"`, `error: "io error: ..."`, `kind: { kind: "install_failed" }`. Expected: design doc §Format helpers row for FailedSkill (the `${name} — ${error}` shape). The `kind.kind` is consumed for context — the rendered string may or may not include "(install failed)" depending on whether the surrounding context (the panel section heading) already conveys "Skill failed"; check the design doc shape and match it exactly.
2. **`requested_but_not_found` variant** — input as above but with `kind: { kind: "requested_but_not_found", plugin: <PluginName> }`. Expected: render incorporates the requested-but-not-found semantic. If your read of the design table suggests two different render shapes per variant, your switch reflects that; if one shape, the switch normalizes.
3. **assertNever path** — construct an invalid kind via `@ts-expect-error`, e.g. `{kind: "totally_new_variant"} as unknown as FailedSkillReason`, and assert that calling `formatFailedSkill` with it throws.

Pattern precedent: existing `formatSkippedSkill` in `format.ts` — `s.reason.kind` switch with `default: const _exhaustive: never = s.reason; throw new Error(...)`.

- [ ] **Step 2: Run the test and verify it fails**

```bash
cd crates/kiro-control-center && npm run test:unit -- format.test.ts
```

Expected: tests for the new describe block fail (function undefined). Existing tests in the file continue to pass.

- [ ] **Step 3: Implement `formatFailedSkill` in `format.ts`**

Signature: `export function formatFailedSkill(f: FailedSkill): string`

Discipline:
- Switch on `f.kind.kind` (note the double `.kind` — `FailedSkill.kind` is `FailedSkillReason`, which is itself a discriminated union with its own `kind` discriminator).
- Default arm uses `const _exhaustive: never = f.kind; throw new Error(...)` per precedent in `formatSkippedSkill`.
- Add a module-level value-position exhaustiveness assert anchored to `FailedSkillReason["kind"]` per the pattern at `stores/plugin-updates.ts:135-137`. Name it `_FAILED_SKILL_REASON_KINDS` / `_AssertFailedSkillReasonExhaustive` / `_assertFailedSkillReasonExhaustive`. This is non-optional — the value-position const is what makes the tripwire fire (unused type aliases resolving to `never` are valid TS).

Decision: switch + value-position assert pair (not just `assertNever` inside the switch). The switch alone catches *missing* arms at the call site; the value-position assert catches *added* arms in the type definition that the values list missed. CLAUDE.md "discriminator-pushdown discipline" requires both.

Type imports: `FailedSkill`, `FailedSkillReason` from `$lib/bindings`.

- [ ] **Step 4: Run the test and verify it passes**

```bash
cd crates/kiro-control-center && npm run test:unit -- format.test.ts
```

Expected: all describe blocks pass.

- [ ] **Step 5: Type check + commit**

```bash
cd crates/kiro-control-center && npm run check
```

Expected: zero errors. If the value-position assert isn't firing (e.g. you typoed `extends never` to `extends any`), `npm run check` won't catch it — the test for the assertNever path is what catches that. Verify both gates pass.

```bash
git add crates/kiro-control-center/src/lib/format.ts crates/kiro-control-center/src/lib/format.test.ts
git commit -m "$(cat <<'EOF'
feat(format): add formatFailedSkill render helper

Switch on FailedSkillReason kind paired with a value-position
exhaustiveness assert anchored to FailedSkillReason["kind"] —
per CLAUDE.md discriminator-pushdown discipline. Precedent:
_PLUGIN_ACTION_VALUES at stores/plugin-updates.ts.

Refs: docs/plans/2026-05-11-inline-per-failure-ui-design.md
EOF
)"
```

---

## Task 3: `formatFailedAgent` (the load-bearing one)

**Files:**
- Modify: `crates/kiro-control-center/src/lib/format.ts`
- Test: `crates/kiro-control-center/src/lib/format.test.ts`

- [ ] **Step 1: Write the failing test**

In `format.test.ts`, add `describe("formatFailedAgent")`. Cases:

1. **`agent` variant** — `kind: "agent"`, `name: <AgentName>`, `source_path: "agents/code-reviewer.md"`, `error: "io error: permission denied"`. Expected: design doc row for `FailedAgent::Agent`.
2. **`unparseable_agent` variant** — `kind: "unparseable_agent"`, `source_path: "agents/broken.md"`, `error: "missing frontmatter fence"`. Expected: design doc row for `UnparseableAgent`.
3. **`companion_bundle` with `conflicts: []`** — covers the `MultipleScanRootsNotSupported` rejection-pre-enumeration case. Expected: design doc row for `CompanionBundle`, with the empty-conflicts placeholder phrasing (`[no enumeration]` per the design table; verify against the table you see).
4. **`companion_bundle` with `conflicts: ["agents/prompts/x.md"]`** — covers the length-1 engine output. Expected: single-conflict render.
5. **assertNever path** — invalid kind via `@ts-expect-error`, assert throw. Same shape as the Task 2 assertNever case.

Pattern precedent for fixtures: read `plugin-actions.test.ts:17-33` for `emptyInstallResult()`. Don't share a builder across format and plugin-actions tests; each test file builds its own fixtures (project convention is small inline literals).

- [ ] **Step 2: Run the test and verify it fails**

```bash
cd crates/kiro-control-center && npm run test:unit -- format.test.ts
```

Expected: new describe block fails (function undefined).

- [ ] **Step 3: Implement `formatFailedAgent` in `format.ts`**

Signature: `export function formatFailedAgent(entry: FailedAgent): string`

Discipline:
- Switch on `entry.kind` over `"agent"` / `"unparseable_agent"` / `"companion_bundle"`. Default arm uses `assertNever`-style throw per `formatSkippedSkill` precedent.
- Module-level value-position exhaustiveness assert anchored to `FailedAgent["kind"]` per `stores/plugin-updates.ts:135-137` pattern. Naming: `_FAILED_AGENT_KINDS` / `_AssertFailedAgentKindExhaustive` / `_assertFailedAgentKindExhaustive`.
- For the `companion_bundle` arm, handle both `conflicts: []` (rejection-pre-enumeration; empty placeholder per design table) and `conflicts: [...]` (length-1+ engine output) in one branch — both render through the same expression, just `.join()` on the array.

Decision rationale (already in spec but worth re-stating at implementation time): the `error` field is opaque pre-rendered diagnostic text from Rust's `error_full_chain` (see `bindings.ts` doc-comment on `FailedAgent_Serialize`). Render directly, don't parse, don't attempt to truncate — the panel is the inspection surface and the user wants the full chain.

Type imports: `FailedAgent` from `$lib/bindings` (this is the discriminated union; `FailedAgent_Serialize` and `FailedAgent_Deserialize` are the serde halves which collapse to the same union for FE consumption).

- [ ] **Step 4: Run the test and verify it passes**

```bash
cd crates/kiro-control-center && npm run test:unit -- format.test.ts
```

Expected: all five cases pass.

- [ ] **Step 5: Type check + commit**

```bash
cd crates/kiro-control-center && npm run check
```

Expected: zero errors.

```bash
git add crates/kiro-control-center/src/lib/format.ts crates/kiro-control-center/src/lib/format.test.ts
git commit -m "$(cat <<'EOF'
feat(format): add formatFailedAgent render helper

Switch on FailedAgent kind discriminator (agent / unparseable_agent /
companion_bundle) paired with a value-position exhaustiveness assert
anchored to FailedAgent["kind"] — per CLAUDE.md
discriminator-pushdown discipline. Consumes the tagged-enum wire
shape from PR #113. Renders the error chain (pre-rendered by Rust's
error_full_chain) opaquely.

Refs: docs/plans/2026-05-11-inline-per-failure-ui-design.md,
docs/plans/2026-05-09-failed-agent-discriminator-design.md F3
EOF
)"
```

---

## Task 4: Extend `PluginActionOutcome` to carry `installResult`

**Files:**
- Modify: `crates/kiro-control-center/src/lib/plugin-actions.ts`
- Modify: `crates/kiro-control-center/src/lib/plugin-actions.test.ts`

- [ ] **Step 1: Write the failing test**

In `plugin-actions.test.ts`, extend the existing "install success" test (around line 76) to assert that `outcome.installResult` is set and equals the IPC result. Then add one new test in the same `describe("runPluginInstall")`:

**Test name:** "install with failures: outcome.installResult carries each FailedX entry intact"

Setup:
- Build an `InstallPluginResult_Serialize` populated with at least one entry in each of `skills.failed`, `steering.failed`, `agents.failed`. The `agents.failed` array should include all three `FailedAgent` variants.
- IPC mock returns `{ status: "ok", data: <that result> }`.

Assertions:
- `outcome.kind === "ok"`
- `outcome.installResult.skills.failed` contains the expected `FailedSkill` entry
- `outcome.installResult.steering.failed` contains the expected `FailedSteeringFile` entry
- `outcome.installResult.agents.failed` length and per-element discriminator match the input

Pattern precedent: existing tests in same `describe` block. Reuse `emptyInstallResult()` builder; mutate slices before passing.

- [ ] **Step 2: Run the test and verify it fails**

```bash
cd crates/kiro-control-center && npm run test:unit -- plugin-actions.test.ts
```

Expected: the new assertions fail because `outcome.installResult` doesn't exist on the `ok` arm yet. TypeScript may also flag `outcome.installResult` access — that's part of the red.

- [ ] **Step 3: Widen `PluginActionOutcome.ok` and update returns**

In `plugin-actions.ts`:
1. Update the `PluginActionOutcome` type alias to add `installResult: InstallPluginResult_Serialize` to the `ok` arm.
2. Update the return statement at the end of `runPluginInstall`'s success path (currently `return { kind: "ok", banner: { ... } }`) to also include `installResult: result.data`.
3. Remove the forward-looking F3 comment block (currently around lines 160-172) — the implementation now exists in `format.ts`. Keep the diagnostic block for now (Task 5 removes it in the same commit as the UI lands).

Decision rationale: keeping the diagnostic for one more commit is fine — it's pure scaffolding and the same-commit coupling with kiro-5qcb is about *not landing the UI without removing the diagnostic*, not the inverse. Removing the F3 comment now is correct because keeping it once `format.ts` has the helpers would be misleading.

Type import: `InstallPluginResult_Serialize` is already imported at the top of `plugin-actions.ts` — no new import needed.

- [ ] **Step 4: Run all unit tests; type check**

```bash
cd crates/kiro-control-center && npm run test:unit && npm run check
```

Expected: vitest passes (including the new failures-carried test and the existing install-success test). `npm run check` zero errors. If `BrowseTab.svelte` or any other consumer fails type-check because they destructure the `ok` arm strictly, fix by leaving the extra field unused (don't narrow the destructure).

- [ ] **Step 5: Commit**

```bash
git add crates/kiro-control-center/src/lib/plugin-actions.ts crates/kiro-control-center/src/lib/plugin-actions.test.ts
git commit -m "$(cat <<'EOF'
refactor(plugin-actions): widen PluginActionOutcome to carry installResult

The `ok` arm now exposes the full InstallPluginResult_Serialize so
consumers can render per-item failures. BrowseTab.svelte continues
reading only `outcome.banner` — backwards-compatible.

Also removes the forward-looking F3 switch-shape comment block (the
implementation now exists in format.ts).

Refs: docs/plans/2026-05-11-inline-per-failure-ui-design.md
EOF
)"
```

---

## Task 5: Inline per-failure panel in `InstalledTab.svelte` + diagnostic removal

This is the load-bearing task. **Same-commit coupling: this commit also removes the temporary diagnostic block in `plugin-actions.ts`, closing `kiro-5qcb`.**

**Files:**
- Modify: `crates/kiro-control-center/src/lib/components/InstalledTab.svelte`
- Modify: `crates/kiro-control-center/src/lib/plugin-actions.ts` (diagnostic removal)

- [ ] **Step 1: Add state declarations in InstalledTab.svelte**

Mirror the existing `removeResult` / `removeResultPlugin` / `removeResultHasFailures` declarations at `InstalledTab.svelte:39-49`. New names: `installResult`, `installResultPlugin`, `installResultHasFailures`.

Types: `installResult: InstallPluginResult_Serialize | null`, `installResultPlugin: string | null`, the `$derived` sums `.failed.length` across `skills`, `steering`, `agents`.

Type import: add `InstallPluginResult_Serialize` to the existing `import type { InstalledSkillInfo, InstalledPluginInfo, RemovePluginResult } from "$lib/bindings"` at the top of the script block. Also import the three new format helpers from `$lib/format`.

- [ ] **Step 2: Wire setter in `updatePlugin()`**

Inside `updatePlugin()`'s `try` block, when `outcome.kind === "ok"` (currently at line ~173):

- Compute the failure-sum inline: `outcome.installResult.skills.failed.length + outcome.installResult.steering.failed.length + outcome.installResult.agents.failed.length`.
- If positive, set `installResult = outcome.installResult` and `installResultPlugin = plugin`. Otherwise leave both as `null` (the panel won't render anyway, but state stays clean).

Decision rationale (per spec): don't call `formatInstallPluginResult` a second time just to read `anyFailed` — the slices are right there. Avoids drift against the `$derived` (which uses the same inline shape).

Also: at the top of `updatePlugin()`, alongside the existing `installError = null; installMessage = null; ...` resets, add `installResult = null; installResultPlugin = null;`.

- [ ] **Step 3: Add panel markup**

Add a new `{#if ...}` block in the template, positioned adjacent to the existing remove panel (around `InstalledTab.svelte:280-324`). Twin structure:

- Outer container `<div>` with conditional color classes — same pattern as remove panel's `removeResultHasFailures` ternary, but the install panel's color decision uses install-summary semantics. Look at `formatInstallPluginResult` (in `format.ts`) for the `anyInstalled` / `anyFailed` booleans; mirror the remove panel's "warning if partial / error if total-fail / success if clean" three-way (here only the first two are reachable since the panel hides on clean).
- `<details open={true}>` — always open since the panel only appears when there are failures (the remove panel uses `open={removeResultHasFailures}` because it shows on clean removes too).
- Summary text: "Show failures" (no installed-items line — failures-only scope per spec).
- Body sections per slice. For each, iterate the failures with `{#each ...}` and render `<div><b>{label}:</b> {format(failure)}</div>` where `label` is "Skill failed" / "Steering failed" / "Agent failed" and `format` is the matching helper.
- Dismiss `<button>` mirroring the remove panel's, clearing `installResult` and `installResultPlugin`.

Decision rationale: visual twin of the remove panel for consistency. Don't extract a shared component yet — premature abstraction (the two panels' contents differ structurally, and extracting now closes the door on B2 / B3 follow-ups that might want different shapes).

`{#each}` key: skills use `f.name`; steering use `f.source`; agents have no single stable id across variants — use the index as `(f, i)` key (acceptable for transient, non-reordered lists). Lint/check should accept this.

- [ ] **Step 4: Add project-switch reset**

In the existing project-switch `$effect` at `InstalledTab.svelte:248-261`, add `installResult = null;` and `installResultPlugin = null;` alongside the existing resets. Same block where `removeResult` and friends are cleared.

- [ ] **Step 5: Remove the diagnostic block in `plugin-actions.ts`**

Delete the `console.error` diagnostic block at `plugin-actions.ts:130-149` (the block that logs `result.data.{skills,steering,agents}.failed` to DevTools). The accompanying comment is also part of the block; remove it together. The implementation has now superseded the diagnostic.

Verification: search the file for `[plugin-actions]` after the edit — only the two `storeRefresh` / `tab refresh` failure logs should remain. The "had per-item failures" log is gone.

- [ ] **Step 6: Type check**

```bash
cd crates/kiro-control-center && npm run check
```

Expected: zero errors. Common pitfalls:
- Forgot to import the new format helpers — fix the import line.
- Used `f` as both the each-item and outer-scope variable — Svelte 5 lints this; rename.
- `installResultHasFailures` typed as boolean but the `$derived` returns `number > 0` — should already be `boolean`, but if you wrote the sum without the comparison, fix.

- [ ] **Step 7: Run all unit tests**

```bash
cd crates/kiro-control-center && npm run test:unit
```

Expected: PASS (the diagnostic removal doesn't have unit-test coverage; the panel doesn't either — both are out of vitest scope per CLAUDE.md).

- [ ] **Step 8: Manual smoke (dev server)**

```bash
cd crates/kiro-control-center && npm run dev
```

In the dev server:
1. Pick the `kiro-control-center` project (canonical reproducer per `2026-05-09-failed-agent-discriminator-design.md`: agents are git-tracked, so a companion-bundle install produces orphan-file conflicts).
2. Trigger an Update on a plugin that produces companion-bundle conflicts.
3. Verify: banner appears with count summary; below it, the new `<details>` panel appears already-open showing each per-item failure with the format helpers' output.
4. Click the dismiss × — panel disappears, banner remains.
5. Switch to a different project — panel and banner both clear.
6. Open DevTools console — verify no `[plugin-actions]` "had per-item failures" log (the diagnostic was removed). The two `storeRefresh` / `tab refresh` error logs remain available (only fire on refresh failures).

If you can't run the dev server (sandbox / CI / no display), say so explicitly in the PR description — do not claim manual validation passed without performing it.

- [ ] **Step 9: Commit (closes kiro-kmj4 + kiro-5qcb)**

```bash
git add crates/kiro-control-center/src/lib/components/InstalledTab.svelte crates/kiro-control-center/src/lib/plugin-actions.ts
git commit -m "$(cat <<'EOF'
feat(ui): inline per-failure panel in InstalledTab.svelte

Renders skills/steering/agents install-failure entries inline after
an Update action, mirroring the existing runPluginRemove <details>
panel. Consumes the tagged-enum FailedAgent wire shape via
formatFailedAgent's discriminator switch.

Failures-only display (banner already shows installed counts).
Panel hides on clean installs. Dismiss-× clears installResult +
installResultPlugin; project-switch effect clears both.

Same-commit coupling: removes the temporary console.error
diagnostic in plugin-actions.ts (kiro-5qcb). Per the coupling
contract — diagnostic removal must land WITH the inline UI, not
before (visibility gap) or after (transient noise).

Closes kiro-kmj4. Closes kiro-5qcb.

Refs: docs/plans/2026-05-11-inline-per-failure-ui-design.md
EOF
)"
```

---

## Task 6: Workspace gates + rivets close

- [ ] **Step 1: Pre-commit workspace gates**

```bash
cargo fmt --all --check
cargo test --workspace
cargo clippy --workspace --tests -- -D warnings
cd crates/kiro-control-center && npm run check && npm run test:unit
```

Expected: all green. No Rust changes were made; the workspace gates run anyway because CI enforces them on every change. If `cargo test --workspace` fails on something unrelated, that's a separate issue — flag it before submitting.

- [ ] **Step 2: Open PR**

```bash
gh pr create --title "feat: inline per-failure UI in InstalledTab.svelte (kiro-kmj4)" --body "$(cat <<'EOF'
## Summary

- Renders per-item install/update failures inline in `InstalledTab.svelte`, mirroring the existing remove-summary panel.
- Adds three pure render helpers (`formatFailedSkill`, `formatFailedSteeringFile`, `formatFailedAgent`) in `format.ts` with value-position exhaustiveness asserts per CLAUDE.md discriminator-pushdown discipline.
- Widens `PluginActionOutcome.ok` arm to carry `installResult: InstallPluginResult_Serialize`; `BrowseTab.svelte` ignores the new field (backwards-compatible).
- Removes the temporary `console.error` diagnostic in `plugin-actions.ts` in the same commit as the UI lands — per the kiro-5qcb same-commit coupling contract.

Closes kiro-kmj4 and kiro-5qcb (both rivets-tracked).

## Design

`docs/plans/2026-05-11-inline-per-failure-ui-design.md`

## Test plan

- [x] `npm run test:unit` — three new format-helper test blocks + extended plugin-actions outcome test
- [x] `npm run check` — typecheck
- [x] `cargo fmt --check`, `cargo test --workspace`, `cargo clippy --workspace --tests -- -D warnings`
- [x] Manual dev-server smoke against the kiro-control-center project (canonical companion-bundle conflict reproducer): trigger Update on a plugin known to produce per-item failures, verify panel renders, verify dismiss + project-switch clears state, verify diagnostic console log is gone.

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

- [ ] **Step 3: After merge — close rivets issues**

On `main` (post-merge), close both tracked issues with the merging PR number. Following the rivets work-tracking convention in CLAUDE.md (reason field = `PR #NNN: <one-line summary>`):

```bash
rivets close kiro-kmj4 --reason "PR #NNN: inline per-failure UI in InstalledTab.svelte"
rivets close kiro-5qcb --reason "PR #NNN: diagnostic removed with F3 (same-commit coupling)"
```

Replace `NNN` with the actual merged PR number from step 2. Commit the `.rivets/issues.jsonl` updates separately on main with a `chore(rivets): close kiro-kmj4 + kiro-5qcb` commit.

---

## Self-review summary

**Spec coverage check:**
- §Design §Unit 1 (Format helpers) → Tasks 1, 2, 3 (one helper per task)
- §Design §Unit 2 (Outcome shape) → Task 4
- §Design §Unit 3 (Panel state + markup) → Task 5
- §Coupled cleanup (kiro-5qcb diagnostic removal) → Task 5 step 5 (same commit as panel)
- §Testing (vitest helpers, manual smoke) → Tasks 1-3 (helper tests), Task 4 (outcome test), Task 5 step 8 (manual)
- §Verification gates → Task 6 step 1

**Placeholder scan:** No "TBD" / "TODO" / "fill in details". `PR #NNN` placeholders in Task 6 step 3 are intentional (filled at merge time per CLAUDE.md rivets convention) — they're the only deferred fillers.

**Type-consistency check:** Helper names consistent across tasks (`formatFailedSkill`, `formatFailedSteeringFile`, `formatFailedAgent`). State names consistent (`installResult`, `installResultPlugin`, `installResultHasFailures`). Outcome field name consistent (`installResult` on the `ok` arm).

**Plan-style note:** Per `feedback_plans_interfaces_not_code.md`, no copy-paste-ready code bodies. Each step specifies signature/contract + precedent file:line + decision rationale + verification hint. Implementer is expected to engage, not transcribe.

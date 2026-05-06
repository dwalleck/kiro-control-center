# Phase 2b — deferred follow-up tasks

**Status:** Open. Prerequisite: PR #108 (`feat/phase-2b-update-detection-ui`) merged.
**Source:** Multi-agent review pass on PR #108 + post-fix simplifier pass. All Critical, all Important, and the highest-value simplifications landed in commit `5728869` on the PR-108 branch. The items below were explicitly deferred at the time as either (a) too broad to bundle without expanding the PR scope, (b) blocked on Rust-side decisions, or (c) needing fixture work.

This document is self-contained — an agent picking it up does not need the prior conversation context. Citations are pinned at the post-`5728869` state of the codebase. Validate each citation with `git grep` before editing; re-base may have shifted line numbers.

---

## Recommended PR grouping

| PR | Tasks | Net impact |
|---|---|---|
| **PR-A — Cross-tab plugin-action refactor** | T1, T2, T3, T4 | ~−80 lines, removes drift risk between BrowseTab + InstalledTab. Highest-value structural change. |
| **PR-B — Type hardening** | T5, T6, T7 | Small. Cosmetic clarity + invariant locking. Could be folded into PR-A. |
| **PR-C — PluginCard branch collapse** | T8 | ~−10 lines. Self-contained to one component. |
| **PR-D — Wire-format `Option<Vec<T>>` cleanup** | T9 | Crosses Rust/FE boundary. Needs a Rust-side decision. |
| **PR-E — e2e coverage for 2b dynamic behavior** | T10 | Needs new test fixture. Likely largest single-PR effort here. |

PR-A is the recommended starting point: it has the highest line savings, removes the most drift risk, and lands as one reviewable unit.

---

## PR-A — Cross-tab plugin-action refactor

The two tabs each carry: an `ErrorSource` union with overlapping shape, an `as ErrorSource` cast at update-check key construction, a `refreshAfterPluginAction` helper, and a banner-projection `$effect` triplet. These four duplications fold together cleanly.

### T1 — Extract shared `error-source.ts` module

**Motivation.** The `update-check<DELIM><remediation><DELIM><marketplace>` key shape is constructed in two files. Both use `as ErrorSource` because TS can't narrow a template-literal expression with non-literal interpolations. A constructor localizes the cast to one tested function.

**Files affected.**
- `crates/kiro-control-center/src/lib/components/BrowseTab.svelte:42-67` — `PLUGINS_ERR_PREFIX`, `SKILLS_ERR_PREFIX`, `BULK_SKILLS_ERR_PREFIX`, `ERR_MARKETPLACES`, `ERR_INSTALLED_PLUGINS`, `ERR_UPDATE_FETCH`, `UPDATE_CHECK_PREFIX`, `ErrorSource`, `_AssertNarrow`, `pluginsErrKey`, `skillsErrKey`, `bulkSkillsErrKey`.
- `crates/kiro-control-center/src/lib/components/BrowseTab.svelte:459-460` — the `as ErrorSource` cast.
- `crates/kiro-control-center/src/lib/components/InstalledTab.svelte:34-41` — `ERR_INSTALLED_PLUGINS`, `ERR_UPDATE_FETCH`, `UPDATE_CHECK_PREFIX`, `ErrorSource`.
- `crates/kiro-control-center/src/lib/components/InstalledTab.svelte:213-214` — the `as ErrorSource` cast.

**New file.** `crates/kiro-control-center/src/lib/error-source.ts`.

**Target shape.** Export the **shared slice** plus the **constructor** that hides the cast. Each tab keeps its own `ErrorSource` union (the slice is just one alternative inside it):

```ts
// in error-source.ts
export const UPDATE_CHECK_PREFIX = "update-check" as const;
export const ERR_INSTALLED_PLUGINS = "installed-plugins" as const;
export const ERR_UPDATE_FETCH = "update-fetch" as const;

export type UpdateCheckKey =
  `${typeof UPDATE_CHECK_PREFIX}${typeof DELIM}${string}${typeof DELIM}${string}`;

export const updateCheckErrKey = (
  remediation: RemediationClass,
  marketplace: string,
): UpdateCheckKey =>
  `${UPDATE_CHECK_PREFIX}${DELIM}${remediation}${DELIM}${marketplace}` as UpdateCheckKey;

export const parseUpdateCheckKey = (
  key: UpdateCheckKey,
): { remediation: string; marketplace: string } => {
  const [, remediation, marketplace] = key.split(DELIM);
  return { remediation, marketplace };
};
```

Each tab's own `ErrorSource` then composes `UpdateCheckKey | ...own-keys`. The `as ErrorSource` casts at the construction sites collapse into `updateCheckErrKey(...)` calls.

**Tests.** Add `crates/kiro-control-center/src/lib/error-source.test.ts`:
- `updateCheckErrKey("stale_cache", "acme")` returns the exact 3-segment string.
- `parseUpdateCheckKey(updateCheckErrKey(r, mp))` round-trips.
- A `_AssertNarrow` regression-test type matches the existing pattern at `BrowseTab.svelte:62`.

**Acceptance criteria.**
- Both tabs import `UPDATE_CHECK_PREFIX`, `ERR_UPDATE_FETCH`, `ERR_INSTALLED_PLUGINS`, `UpdateCheckKey`, `updateCheckErrKey`, `parseUpdateCheckKey` from `error-source.ts`.
- Zero `as ErrorSource` casts at update-check construction sites.
- `npm run check` clean; new round-trip test passes.

**Trade-off.** A new `lib/` module, but it's a single file with one explicit purpose. The casts are exactly the points the type system was previously failing to enforce.

### T2 — Extract `lib/plugin-actions.ts` and consolidate `runPluginInstall` + `runPluginUpdate` + `runPluginRemove`

**Motivation.** `BrowseTab.runPluginInstall` and `InstalledTab.updatePlugin` are byte-identical except for the post-action refresh function (which fetches the install set in BrowseTab vs. re-runs `refresh()` in InstalledTab). `InstalledTab.removePlugin` shares the same scaffolding plus a `RemovePluginResult` projection step.

**Files affected.**
- `crates/kiro-control-center/src/lib/components/BrowseTab.svelte:799-841` — `runPluginInstall` (the merged install/update handler from PR #108).
- `crates/kiro-control-center/src/lib/components/InstalledTab.svelte:104-179` — `removePlugin`, `updatePlugin`.
- Both tabs' `refreshAfterPluginAction` helpers (`BrowseTab.svelte:786-797`, `InstalledTab.svelte:93-102`) get inlined into the new module.

**New file.** `crates/kiro-control-center/src/lib/plugin-actions.svelte.ts` (or a non-svelte module if you can keep it pure — see "Trade-off").

**Target shape.** Pure-data return + caller-provided side-effect callbacks:

```ts
export type PluginActionMode = "install" | "update";

export type PluginActionContext = {
  marketplace: string;
  plugin: string;
  projectPath: string;
  forceInstall: boolean;
  acceptMcp: boolean;
  refresh: () => Promise<void>;     // tab's local refresh
};

export type PluginActionOutcome =
  | { kind: "ok"; banner: { error: string | null; message: string | null; warning: string | null } }
  | { kind: "fail"; error: string };

export async function runPluginInstall(
  ctx: PluginActionContext,
  mode: PluginActionMode,
): Promise<PluginActionOutcome>;

export async function runPluginRemove(
  ctx: Omit<PluginActionContext, "forceInstall" | "acceptMcp">,
): Promise<PluginActionOutcome & { removeResult?: RemovePluginResult }>;
```

The store call (`pluginUpdates.refresh`) and the Tauri command call live in the helper. The post-action refresh ordering ("`pluginUpdates.refresh` first, then local") is enforced inside the helper, so the cascade rule has one home.

**Tests.** Add `crates/kiro-control-center/src/lib/plugin-actions.test.ts`. Mock `commands.installPlugin` + `commands.removePlugin` + the refresh callbacks. Cases:
- Install success: `kind: "ok"`, `banner.message` populated, `banner.error` null. Verifies `force` is `forceInstall` for `mode: "install"` and `true` for `mode: "update"`.
- Install with `anyFailed && !anyInstalled`: `kind: "ok"` (still ok at the wrapper level — the command succeeded), `banner.error` populated.
- Install with warnings: `banner.warning` populated.
- Tauri command returns error: `kind: "fail"`.
- Tauri command throws: `kind: "fail"`, error message reflects the throw.
- Post-action refresh throws: outcome is `kind: "ok"` but a refresh-failed sub-banner surfaces (see C4 in the original review). Decide whether to fold this into the outcome shape or keep the `refreshAfterPluginAction` try/catch separate.

**Acceptance criteria.**
- Both tabs call into the shared helpers; no `commands.installPlugin` / `commands.removePlugin` calls in component files.
- The "cascade ordering" claim from PR #108 (`pluginUpdates.refresh` before local refresh) is enforced in the shared helper, not at each call site.
- New unit tests cover all 6 outcome paths.
- `npm run check` clean; existing 44 unit tests still pass.

**Trade-off.** Side-effect callbacks (`refresh`, `setBanner`) make the helper less pure. If you keep `pluginUpdates.refresh` calls *inside* the helper, the helper takes a hard dep on the store module — that's fine since the store is also pure-logic-test-excluded. The alternative — return a list of "next actions" the caller dispatches — is purer but pushes the cascade-ordering rule back to the call sites, which defeats the consolidation. **Recommend the side-effect-callback shape.**

### T3 — Extract `usePluginUpdateBanners(...)` rune helper

**Motivation.** Three `$effect` blocks per tab project store state into the local `fetchErrors` map: (a) re-run scan on `projectPath` change, (b) project failure groups into update-check keys + dispose stale ones, (c) project toplevel `pluginUpdates.fetchError` into a single banner. Identical between tabs except for the structured-log prefix.

**Files affected.**
- `crates/kiro-control-center/src/lib/components/BrowseTab.svelte:443-477` — the three `$effect` blocks plus the `update-check<DELIM>...` projection logic.
- `crates/kiro-control-center/src/lib/components/InstalledTab.svelte:198-236` — same triplet.

**New file.** `crates/kiro-control-center/src/lib/stores/plugin-update-banners.svelte.ts`.

**Target shape.** A rune helper that takes the consumer's `fetchErrors` map and the tab's banner-key namespace constants:

```ts
// Caller invokes inside its own component context. Runs three $effect
// blocks under the hood — one for each projection.
export function usePluginUpdateBanners(args: {
  projectPath: () => string,
  fetchErrors: SvelteMap<UpdateCheckKey | typeof ERR_UPDATE_FETCH | string, string>,
  logPrefix: string,
}): void;
```

`projectPath` is passed as a thunk so the helper can read it inside its own `$effect` and register the dependency in the helper's reactive scope.

**Tests.** Vitest cannot test `$effect` directly per the project's testing scope rule (CLAUDE.md). The pure parts are already tested by `groupFailures` in `plugin-updates.test.ts`. Add a follow-up integration test in e2e (T10's banner-stack test) to exercise the helper end-to-end.

**Acceptance criteria.**
- Both tabs invoke `usePluginUpdateBanners({ projectPath: () => projectPath, fetchErrors, logPrefix: "[BrowseTab]"|"[InstalledTab]" })` at the script-tag top level.
- The three `$effect` blocks in each tab are deleted.
- Stale-key disposal logic (the `for (const k of fetchErrors.keys()) ... delete` loop) lives inside the helper.
- `npm run check` clean; e2e for banner stacking still passes (currently does, since it only asserts UI behavior).

**Trade-off.** Svelte 5's rune system supports calling `$effect` from `.svelte.ts` modules invoked at component-init time. Verify with the Svelte MCP server if unsure (see CLAUDE.md tooling). If runes can't cross the module boundary cleanly, fall back to a plain helper that returns `{ projectFailures, projectFetchError }` callbacks the caller wraps in `$effect` themselves — saves less but still extracts the projection logic.

### T4 — Fold `refreshAfterPluginAction` into T2

`BrowseTab.svelte:786-797` and `InstalledTab.svelte:93-102` are 10-line near-duplicates. Once T2 lands, both inline into the shared `runPluginInstall` / `runPluginRemove` helpers. No standalone work — listed for visibility.

**Acceptance criteria.** No `refreshAfterPluginAction` function remains in either tab; the post-action refresh logic lives in `lib/plugin-actions.svelte.ts`.

---

## PR-B — Type hardening

Small, isolated, can land independently or fold into PR-A.

### T5 — Brand or factory-protect `FailureGroup`

**Motivation.** `FailureGroup` (`crates/kiro-control-center/src/lib/stores/plugin-updates.ts:54-59`) has a `remediationHint` field that's *derivable* from `(remediation, marketplace)`, but the type doesn't enforce that consumers can't construct `{ remediation: "stale_cache", marketplace: "x", remediationHint: "Contact owner..." }` (a mismatched-hint instance).

**Options.**
- **Brand the type.** Add a `__brand: never` phantom field that only `groupFailures` can produce. Mechanical but adds noise.
- **Unexport the type alias.** Export only `groupFailures` and `(typeof groupFailures)["return"][number]` (or use `ReturnType<typeof groupFailures>[number]`). Forces consumers to receive the type via inference.

**Recommend option 2** — less ceremony, same guarantee.

**Files.** `crates/kiro-control-center/src/lib/stores/plugin-updates.ts:54-59`. Update consumers in BrowseTab + InstalledTab to drop the named import (Svelte's type inference will keep working).

**Tests.** No new tests; the existing `groupFailures` tests cover the construction path. Compile-time guarantee.

**Acceptance criteria.** `FailureGroup` is no longer exported; consumers reference it via inference. `npm run check` clean.

### T6 — Lock the `Extract<PluginAction, ...>` semantics with a comment

**Motivation.** `PluginAction` is currently a string union. If a future refactor adds object payloads (`{ kind: "install"; force: boolean } | ...`), `Extract<PluginAction, "install" | "update">` silently changes meaning (it would filter by full object shape). The narrow path through this footgun is one comment + a test.

**Files.** `crates/kiro-control-center/src/lib/stores/plugin-updates.ts:64-70` (the `PluginAction` declaration).

**Target.** Add one line above the type:
```ts
// String union, not a tagged-object union. If this grows object payloads,
// the `Extract<PluginAction, "install" | "update">` narrowings in BrowseTab
// + InstalledTab will silently change meaning — switch to `Exclude<>` then.
```

**Tests.** Optional: add a compile-time assertion that `Extract<PluginAction, "install"> extends string` to lock the assumption. Lives in the existing test file.

**Acceptance criteria.** Comment + (optional) compile-time guard added.

### T7 — Rename `Extract<PluginAction, ...>` to named per-tab unions

**Motivation.** Cosmetic. `Extract<PluginAction, "install" | "update">` reads as plumbing; `BrowseAction` reads as a domain concept.

**Files.** `crates/kiro-control-center/src/lib/stores/plugin-updates.ts` (add exports), `BrowseTab.svelte:104` and `InstalledTab.svelte:23` (use the named types).

**Target.**
```ts
// in plugin-updates.ts
export type BrowseAction = Extract<PluginAction, "install" | "update">;
export type InstalledAction = Extract<PluginAction, "remove" | "update">;
```

**Acceptance criteria.** No `Extract<PluginAction, ...>` literals in component files.

---

## PR-C — PluginCard branch collapse

### T8 — Collapse `installing` / `updating` branches

**Motivation.** `PluginCard.svelte:73-92` renders `installing` and `updating` as two separate `{#if}` branches with identical disabled-button markup, differing only in the label text and `aria-label`. ~10 lines of duplication.

**Files.** `crates/kiro-control-center/src/lib/components/PluginCard.svelte:73-92`.

**Target shape.**
```svelte
{#if installing || updating}
  {@const verb = installing ? "Install" : "Updat"}
  <button type="button" disabled aria-busy="true" aria-label="{verb}ing {plugin.name}" class="...">
    {verb}ing…
  </button>
{:else if failure && installed}
  ...
```

(The `verb` ternary dropping the trailing `e` is awkward; an explicit `installing ? "Installing" : "Updating"` for both label + aria reads cleaner. Pick whichever your reviewer prefers.)

**Tests.** No unit tests for `.svelte` files per project policy. Existing e2e coverage indirectly verifies the disabled-button rendering.

**Acceptance criteria.** Single branch handles both states; markup is otherwise unchanged. `npm run check` clean.

---

## PR-D — Wire-format `Option<Vec<T>>` cleanup

### T9 — Drop `?? []` coalesces in `formatRemovePluginResult` if Rust emits empty arrays

**Motivation.** `RemovePluginResult.skills.removed?: string[]`, `.failures?: ...` etc. (the `?` suffix) means the wire format treats `undefined` and `[]` as the same state. The FE coalesces with `?? []` in 6 places (`format.ts:265-270`) plus the `<details>` template in InstalledTab (`InstalledTab.svelte:289-308`). If the Rust side stops skipping these fields when empty (`#[serde(skip_serializing_if = "Vec::is_empty")]` removed), the FE type's `?` suffix can be dropped and the coalesces vanish.

**Investigation step.** Find the Rust struct definition of `RemovePluginResult` and its sub-results. Probably under `crates/kiro-market-core/src/service.rs` or `crates/kiro-market-core/src/plugin.rs`. Check for `#[serde(skip_serializing_if = "Vec::is_empty")]` or `#[serde(default)]` on the relevant fields.

**Decision.**
- If the skip is intentional (saves wire bytes for a common no-op case), keep the FE coalesces. Close T9.
- If not, remove the skip, regenerate `bindings.ts` (`cargo test -p kiro-control-center --lib -- --ignored`), drop the FE coalesces.

**Files (FE side, if going forward).**
- `crates/kiro-control-center/src/lib/format.ts:265-270` — drop `?? []` from the 6 destructure-and-coalesce lines.
- `crates/kiro-control-center/src/lib/components/InstalledTab.svelte:289-308` — drop `?? []` from the `<details>` template's `{#if (...).length > 0}` and `{#each ...}` blocks.

**Tests.** The existing `formatRemovePluginResult` test at `format.test.ts:194-201` ("treats undefined removed/failures as empty arrays") becomes either irrelevant (if FE type drops `?`) or tightens to "treats empty arrays as empty" — adjust.

**Acceptance criteria.** Decision documented. If the change goes through, FE coalesces gone; e2e Remove flow still passes.

**Trade-off.** Wire-format change ripples through other FE consumers of `RemovePluginResult`. Audit any callers besides the format helper before committing to this.

---

## PR-E — e2e coverage for 2b dynamic behavior

### T10 — Fixture-backed e2e tests for 2b update-detection scenarios

**Motivation.** The PR-108 unit-test coverage is solid, but the e2e suite at `crates/kiro-control-center/tests/e2e/app.spec.ts` covers only the happy paths (Up to date, Remove → `<details>`). The dynamic 2b behaviors that ship under PR #108 are still uncovered end-to-end:

- `ContentChanged` status-vs-button divergence — column reads "Content changed since install", button reads "Update (content changed)". The contract is unit-tested via `statusUpdateLabel` / `actionUpdateLabel` (`plugin-updates.test.ts`); e2e would catch CSS / DOM regressions that unit tests can't.
- `Update → vN` button rendering when a manifest version bump is detected.
- "Update check failed" pill + `kindLabel` tooltip on `manifest_unreadable` / `marketplace_unavailable` failures.
- Banner stacking + 3-cap-with-overflow `+N more items` row when 4+ keys land in `fetchErrors`.
- Project-switch cleanup: banners / `pendingPluginActions` / `fetchErrors` clear when `projectPath` changes mid-flight.
- Update/Remove mutex: in-flight action disables both row buttons.

**Fixture work needed.**
- A `FIXTURE_BUMPED_MARKETPLACE_PATH` with the same plugin manifest as `FIXTURE_MARKETPLACE_PATH` but a higher `version` field. After installing the original then swapping the marketplace pointer to the bumped fixture, the `Update → vN` and column "vX → vY" should appear.
- A `FIXTURE_BROKEN_MARKETPLACE_PATH` (already exists per `FIXTURE_BROKEN_MARKETPLACE_PATH` reference at `app.spec.ts:158`) — verify it triggers `manifest_unreadable` or `manifest_invalid`.
- A way to populate `fetchErrors` with 4+ entries to exercise the overflow row. Likely needs a fixture marketplace with multiple broken plugins.

**Files.**
- `crates/kiro-control-center/tests/e2e/app.spec.ts` — add new tests under the `Phase 2b — update detection UI` describe block.
- Possibly new fixtures under `crates/kiro-control-center/tests/e2e/fixtures/` (check existing structure).
- CI-side script in `.github/workflows/` if the bumped-fixture flow needs a setup step.

**Test scaffolding (acceptance criteria).** At least 4 new tests, each `test.skip` cleanly when its fixture envvar is unset:
1. `version-bump renders 'Update → vN' button + 'vX → vY' status` — install, bump fixture, navigate to Installed, assert pill + button text.
2. `manifest_unreadable surfaces 'Update check failed' pill with kindLabel tooltip` — broken-fixture case.
3. `4+ banners produce '+N more items' overflow row` — stack assertion.
4. `Update + Remove buttons disable while either action is in-flight` — click Update, assert Remove is disabled before the action resolves.

**Trade-off.** Largest fixture surface area of any task here. Don't bundle into PR-A.

---

## Verification commands (all PRs)

Run these from `crates/kiro-control-center/`:
```bash
npm run check          # svelte-check (must be 0 errors, 0 warnings)
npm run test:unit      # vitest (currently 44 tests; should grow)
npm run test:e2e       # Playwright; FIXTURE envvars optional
```

Run from the repo root:
```bash
cargo fmt --all --check
cargo test --workspace
cargo clippy --workspace --tests -- -D warnings
cargo xtask plan-lint  # all 6 gates must pass
```

T9 (the wire-format change) additionally requires:
```bash
cargo test -p kiro-control-center --lib -- --ignored  # regenerates bindings.ts
```

If `bindings.ts` regenerates with diffs, those diffs are part of the PR.

---

## Out of scope

These were considered and explicitly **not** added to this list:
- **Simplifier #11** — `formatInstallPluginResult` block-scoped `{ const ... }` for skills/steering/agents. Three-block structure is the right tradeoff; extracting forces three new helper signatures over a single-call body.
- **Simplifier #12** — 6 noun-pluralization blocks in `formatRemovePluginResult` could share `countWithNoun`. Borderline cosmetic; explicit form is more grep-able.
- **Cross-file `errorMessage(e: unknown)` lift** — the helper exists in `MarketplacesTab.svelte` and the `e instanceof Error ? e.message : String(e)` pattern repeats in BrowseTab + InstalledTab catches. Low value vs. an import everywhere; defer indefinitely.
- **PluginUpdatesStore.refresh same-path concurrent calls** — the monotonic `#latestRequestId` guard already handles the A→B→A interleave. Don't re-open.

---

## Provenance

- Multi-agent review pass: code-reviewer, pr-test-analyzer, silent-failure-hunter, type-design-analyzer, comment-analyzer, code-simplifier, all run on PR #108.
- Post-fix simplifier pass: code-simplifier re-run after C1–C5 + I2–I10 + initial simplifications landed.
- All Critical (C1–C5) and Important (I1–I10) findings landed in commit `5728869`.
- Simplifications #1, #2, #3, #9 from the post-fix pass also landed in `5728869`.
- This document covers what was deferred at the time, with citations valid at that commit.

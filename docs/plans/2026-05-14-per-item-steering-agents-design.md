# Design — per-item steering/agent install/remove (`kiro-zx73`)

**Status:** ready to execute (lighter-touch than slice 1's full gilfoyle pipeline because the patterns mirror existing precedent).
**Tracker:** [`kiro-zx73`](.) — refreshed 2026-05-14 after slice 4 friction promoted it from stretch to scheduled.
**Predecessor:** the BrowseTab redesign (slices 1–4, commits 4601457 → 4b46c46).

## Why lighter touch

Slice 1 ran the full gilfoyle pipeline (probe → falsifiable-design → budgeted-plan → checkpointed-build) which paid off — caught the SkippedItem wire-format bug before it shipped. The patterns established there cover this work entirely:

- Testable Tauri command shape (`_impl(svc, ...)` per CLAUDE.md, precedent: `install_skills_impl`)
- Classifier discipline (`#[non_exhaustive]` enums + exhaustive matches)
- Wire-format probe (round-trip serde assertions on any new tagged enum)
- `parse-don't-validate` newtypes at FFI boundaries

Reconnaissance found that **all the project-side install/remove primitives already exist** (`KiroProject::install_steering_file`, `remove_steering_file`, `install_agent`, `remove_agent`, `install_native_agent`). The work is exclusively:

1. Wire **name-filtered service-layer entry points** that loop the existing project primitives
2. Add **four Tauri commands** exposing those entry points
3. Wire the **drawer's apply-diff** to use them

So I'm writing one focused design doc with claims + falsifiers + slice plan, and executing slice-by-slice with the same per-slice gates as before. No standalone falsifiable-design / budgeted-plan invocations.

## Non-goals (negative space)

1. **Not adding remove_plugin_steering / remove_plugin_agents bulk variants.** Whole-plugin remove already exists via `removePlugin`. The new per-item removes are scoped to one file/agent.
2. **Not changing existing install_plugin_steering / install_plugin_agents semantics by default.** Adding `InstallFilter` widens their signature; `InstallFilter::All` preserves today's behavior.
3. **Not adding per-companion-file granularity for native agent companions.** Native agents own a companion bundle as a unit; that ownership boundary stays.
4. **Not changing the catalog read** — slice 1's `list_plugin_catalog_for_marketplace` already returns the per-item flags this work consumes.
5. **Not retiring the slice-4 explanatory banner.** Once this work ships, the banner becomes dead code and gets removed in this PR's last commit.

## Architecture

Three new (or widened) service methods, four new Tauri commands, drawer wiring update.

### Service layer (`kiro-market-core`)

**Widen** existing `install_plugin_steering` and `install_plugin_agents` to take an `InstallFilter<'_>` parameter (mirrors `install_skills`'s shape). `InstallFilter::All` preserves today's whole-plugin behavior; `InstallFilter::Names(&[...])` filters discovered items by their installation name.

```rust
// In src/service/mod.rs — modified signatures:
impl MarketplaceService {
    pub fn install_plugin_steering(
        project: &KiroProject,
        plugin_dir: &Path,
        scan_paths: &[String],
        filter: &InstallFilter<'_>,        // NEW
        ctx: SteeringInstallContext<'_>,
    ) -> InstallSteeringResult;

    pub fn install_plugin_agents(
        // ... existing params ...
        filter: &InstallFilter<'_>,        // NEW
    ) -> InstallAgentsResult;
}
```

The `filter` skip happens **after** discovery + parse but **before** the project-side install call — same shape as `install_skills`'s `filter_matches` check. For steering the join key is the steering file's relative path under the scan root; for agents it's the parsed agent name (matching the catalog's join key per slice 1's S3).

**No new project-side methods needed** — `KiroProject::remove_steering_file`, `remove_agent` already exist.

### Tauri commands (`crates/kiro-control-center/src-tauri`)

Four new commands following the established `_impl(svc, ...)` testable-shape pattern:

```rust
#[tauri::command] pub async fn install_steering_files(...)  -> Result<InstallSteeringResult_Serialize, CommandError>;
#[tauri::command] pub async fn remove_steering_file(...)    -> Result<(), CommandError>;
#[tauri::command] pub async fn install_agents(...)          -> Result<InstallAgentsResult_Serialize, CommandError>;
#[tauri::command] pub async fn remove_agent(...)            -> Result<(), CommandError>;
```

All four go in `crates/kiro-control-center/src-tauri/src/commands/` — `install_steering_files` and `remove_steering_file` join `commands/steering.rs`; `install_agents` and `remove_agent` join `commands/agents.rs`. Each follows the wrapper + `_impl(svc, ...)` split.

The two install commands wire through to the new filtered service methods; the two remove commands wrap `KiroProject::remove_steering_file` / `KiroProject::remove_agent` with the standard `validate_kiro_project_path` + name validation.

### Drawer wiring (`CustomizeDrawer.svelte` + `BrowseTab.svelte`)

`CustomizeDrawerDiff` widens to:

```ts
type CustomizeDrawerDiff = {
  skills:   { install: string[]; remove: string[] };
  steering: { install: string[]; remove: string[] };  // NEW
  agents:   { install: string[]; remove: string[] };  // NEW
};
```

Drawer changes:
- Drop `noInteractiveItems` derivation and the explanatory banner
- Drop the `read-only` italic labels on Steering/Agents section headers
- Make Steering/Agents checkboxes interactive (parallel `selectedSteering` / `selectedAgents` SvelteSets, same toggle pattern as skills)
- Diff math extends to all three categories

`applyDrawerDiff` in BrowseTab gains three more round-trips (mirroring the existing skills install/remove): batch install via the new commands, per-item remove loops. Banner composition stays in the same `installMessage`/`installError` channel.

## Falsification

| # | Claim | Falsifier | Oracle | Status | Regression fence |
|---|-------|-----------|--------|--------|------------------|
| 1 | `install_plugin_steering` with `InstallFilter::All` produces the same `InstallSteeringResult` as before the signature change. | rstest: install plugin with 3 steering files using `InstallFilter::All`; assert all 3 land in `result.installed`, none in `failed`. | Hand-built fixture's known file list. | pending | unit test `install_plugin_steering_all_filter_preserves_legacy_behavior` |
| 2 | `install_plugin_steering` with `InstallFilter::Names(&["a"])` installs ONLY the named file even when others are present. | rstest: plugin with `a.md`, `b.md`, `c.md`; install `Names(&["a.md"])`; assert `result.installed.len() == 1` and the destination dir contains only `a.md`. | Filesystem walk of `.kiro/steering/` after install. | pending | unit test `install_plugin_steering_names_filter_installs_only_listed` |
| 3 | `install_plugin_steering` with `InstallFilter::Names(&["nope"])` (a name no discovered file matches) returns a `failed` entry citing the missing name, NOT a silent empty install. | rstest as above; assert `result.failed` non-empty with `RequestedButNotFound`-equivalent variant. | Same fixture; result inspection. | pending | unit test `install_plugin_steering_names_filter_reports_unmatched_names` |
| 4 | `install_plugin_agents` with `InstallFilter::Names(&["actual-name"])` installs ONLY the named agent — using the **parsed** agent name as the join key, not the source filename (the load-bearing distinction from slice 1's S3). | rstest: plugin with `wrong-filename.md` declaring `name: actual-name`; install `Names(&["actual-name"])`; assert installed; install `Names(&["wrong-filename"])` instead; assert NOT installed. | Two-call test — second call's empty install is the negative oracle. | pending | unit test `install_plugin_agents_names_filter_joins_on_parsed_name` |
| 5 | Tauri `_impl` for each new command takes `&MarketplaceService` + primitive args (per CLAUDE.md), so the body is unit-testable without a Tauri runtime. | Read each `_impl` signature; assert it matches `install_skills_impl`'s shape. Inspection via grep. | The signature itself; if `_impl` takes `tauri::State` or `KiroProject` directly it fails the convention. | pending | structural assertion in code review; backed by unit tests on each `_impl` |
| 6 | Each new Tauri command's wire shape surfaces in `bindings.ts` after regen with no chrono/`DateTime` types in the new exports. | Grep `bindings.ts` for `installSteeringFiles`, `removeSteeringFile`, `installAgents`, `removeAgent`; grep for `chrono` / `DateTime<` in non-doc lines around them. | The regenerated `bindings.ts` — independent of design intent. | pending | extend `bindings_export_plugin_catalog_view` test to also assert these four commands |
| 7 | `applyDrawerDiff` in BrowseTab fires install_steering_files when `diff.steering.install.length > 0`, fires install_agents when `diff.agents.install.length > 0`, and fires per-item removes for both categories when present. | Manual smoke (no e2e fixture for this today): open drawer on a plugin with mixed item types, toggle one of each, hit Apply, verify the banner reads "Installed: a, b, c | Removed: 1". | Banner contents reflect the round-trip outcomes. | pending | manual; no automated regression fence — file as `kiro-XXXX` follow-up if needed |
| 8 | The slice-4 banner ("This plugin ships only steering and/or agents...") is removed by this PR's last commit. | Grep CustomizeDrawer.svelte for the banner string after the last commit lands. | Source code. | pending | grep assertion via PR diff review |

### Cheapest falsifier — run

C1 is cheapest (5m). Run it as a regression sentinel: confirm that adding the filter parameter doesn't break today's whole-plugin install behavior. The risk is that `InstallFilter::All` silently misroutes (e.g., implementer forgets to wire it through and every install fails). Run before approving the design.

## Slices

Each slice has a unit-test gate + cargo clippy gate. The visual slice (B) needs a manual smoke since there's no automated UI test for the drawer.

### Slice A — Backend

| # | Slice | Files | Implements |
|---|---|---|---|
| A1 | Widen `install_plugin_steering` to take `InstallFilter` | `service/mod.rs` (modify), all callers (update to pass `InstallFilter::All`) | C1 + C2 + C3 |
| A2 | Widen `install_plugin_agents` to take `InstallFilter` | `service/mod.rs` (modify), all callers | C4 |
| A3 | New Tauri command `install_steering_files` + `_impl` + tests | `commands/steering.rs` | C5 (steering) + C6 (steering install) |
| A4 | New Tauri command `remove_steering_file` + `_impl` + tests | `commands/steering.rs` | C5 + C6 |
| A5 | New Tauri command `install_agents` + `_impl` + tests | `commands/agents.rs` | C5 (agents) + C6 (agents install) |
| A6 | New Tauri command `remove_agent` + `_impl` + tests | `commands/agents.rs` | C5 + C6 |
| A7 | Bindings regen + extend `bindings_export_plugin_catalog_view` to cover new types | `bindings.ts` (regenerate), `lib.rs` (extend test) | C6 (full) |

### Slice B — Frontend

| # | Slice | Files | Implements |
|---|---|---|---|
| B1 | Widen `CustomizeDrawerDiff`; make Steering/Agents sections interactive; drop the `noInteractiveItems` banner and read-only labels | `CustomizeDrawer.svelte` | (UI prerequisite — no claim, slice 4's claims still hold) |
| B2 | Widen `applyDrawerDiff` in BrowseTab to delegate steering install/remove and agent install/remove | `BrowseTab.svelte` | C7 |
| B3 | Manual smoke + close `kiro-zx73` with PR ref | (none) | C7 |

Slice A1 and A2 are sequential (both touch `service/mod.rs`'s install methods + callers); A3/A4/A5/A6 are independent and can be combined into one commit each (Tauri command + tests are tightly scoped). A7 is a single-file regen + assertion. Total: ~6 commits across slice A, ~2 commits across slice B.

## Cheapest falsifier execution

**Claim:** C1 — `install_plugin_steering` with `InstallFilter::All` produces the same `InstallSteeringResult` as before the signature change.

**Approach:** Establish the contract to preserve by running the existing baseline test against today's (pre-change) `install_plugin_steering` signature. Slice A1's gate is then "this same test still passes after the filter parameter is added with a default of `InstallFilter::All`."

**Command:**

```text
cargo test -p kiro-market-core --lib install_plugin_steering_discovers_and_installs_all_files
```

**Result:** `1 passed; 0 failed`. Baseline behavior pinned: 2 steering files in fixture → 2 in `result.installed`, none failed, both files exist on disk under `.kiro/steering/`, idempotent reinstall produces `InstallOutcomeKind::Idempotent` for all.

**What this validates:** the contract A1 must preserve. After A1, this same test still passes (callers pass `InstallFilter::All` to get today's behavior). C2 then exercises `InstallFilter::Names(...)` for the new branch.


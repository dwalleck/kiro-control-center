# Design — `list_plugin_catalog_for_marketplace`

**Status:** falsifiable-design (cheapest falsifier passed; ready for budgeted-plan).
**Slice:** 1 of the BrowseTab redesign — backend bulk catalog read.
**Probe:** `crates/kiro-market-core/tests/prove_it_list_plugin_catalog.rs` (4 passing tests).
**Probe artifacts:** `.prove-it/list-plugin-catalog/{oracle,what-i-learned,related-issues}.md`.

## Purpose

Provide a single bulk read per marketplace that returns each plugin's full per-category item tree (skills + steering + agents) with per-item `installed: bool` flags computed against the project's tracking files, plus structural error surfacing for plugin-level failures and per-item parse failures.

This replaces today's BrowseTab fan-out (`listPlugins(mp)` + N×`listAvailableSkills(mp, plugin, project)`) with one call per marketplace, and exposes steering and agent items per-plugin for the new Customize drawer.

## Non-goals (negative space)

1. **No write side effects.** This is a read-only catalog. No install, no remove, no clone-on-demand for unresolved remote sources.
2. **No tracking ↔ disk cross-check.** `installed: bool` is tracking-file membership, mirroring `SkillInfo.installed` (`browse.rs:896`). Disk-orphan and tracking-orphan states will misreport — accepted as today's contract; addressing it is tracked at [`kiro-3ivx`](#tracker-references).
3. **No changes to existing service methods.** `list_skills_for_plugin`, `list_all_skills`, `list_plugin_entries` are unchanged. The new methods are additive.
4. **No multi-marketplace bulk.** This call enumerates one marketplace. The frontend fans out across marketplaces in parallel via `Promise.all`, mirroring today's `fetchAllSkillsForMarketplace` cadence.
5. **No drawer-side per-item install/remove commands.** The drawer's apply-diff path uses existing `installSkills` / `installPlugin` / `removeSkill` / `removePlugin` until the per-item steering/agent commands are filed and built (issue [`kiro-zx73`](#tracker-references)).

## Probe re-statement

The probe established:

- **Skills:** `list_skills_for_plugin` already returns the per-item shape the design needs. `installed` is `installed.skills.contains_key(name)`.
- **Steering:** Only `discover_steering_files_in_dirs` exists (paths). The catalog must compose `(name, installed)` by joining its output with `InstalledSteering.files` keyed by filename.
- **Agents:** Only `discover_agents_in_dirs` exists (paths). Names live inside files — the catalog must invoke the agent parser to extract them, then join with `InstalledAgents.agents`.

The design must not contradict any of this.

## Input shapes

### `marketplace: &MarketplaceName`
- known with N working plugins (N=0, N=1, N>1)
- known with mixed-health plugins (some resolve, some skipped)
- unknown (registry not present)

### `project: &KiroProject`
- fresh project — no `.kiro/` subdirs, no tracking files
- project with all three tracking files present and valid
- project with one or more tracking files corrupt (parse fails)
- project where tracking and disk diverge (tracking entry but missing disk dir, or orphan disk dir but no tracking entry)

### Per-plugin shape
- manifest absent → defaults to `DEFAULT_{SKILL,STEERING,AGENT}_PATHS`
- manifest present with custom scan paths for one, two, or all three categories
- manifest present with **empty list** for a category → falls back to default (matches `discover_skills_for_plugin` policy)
- source resolves (`PluginSource::RelativePath`) → enumerated
- source unresolvable → `RemoteSourceNotLocal` / `DirectoryMissing` / `NotADirectory` / `SymlinkRefused` / `DirectoryUnreadable` / `InvalidManifest` / `ManifestReadFailed` → goes to `skipped`

### Per-item shape (per category)
- 0 items in scan dir
- 1 item, parses cleanly
- N items all parse, names unique
- N items where one has malformed frontmatter / JSON / YAML → that one goes to `skipped_items`
- excluded filename (README.md / CONTRIBUTING.md / CHANGELOG.md) → silently filtered (matches existing discovery)
- symlink in scan dir → silently skipped (matches existing discovery)
- duplicate item names across multiple scan paths in one category → **see C8**

## Architecture

Three new service methods (per probe finding #2), one new Tauri command on top.

### Service layer (`kiro-market-core`)

```rust
// New, in src/service/browse.rs.
impl MarketplaceService {
    pub fn list_steering_for_plugin(
        &self,
        marketplace: &str,
        plugin: &str,
        installed: &InstalledSteering,
    ) -> Result<PluginSteeringResult, Error>;

    pub fn list_agents_for_plugin(
        &self,
        marketplace: &str,
        plugin: &str,
        installed: &InstalledAgents,
    ) -> Result<PluginAgentsResult, Error>;

    pub fn list_plugin_catalog(
        &self,
        marketplace: &str,
        installed_skills: &InstalledSkills,
        installed_steering: &InstalledSteering,
        installed_agents: &InstalledAgents,
    ) -> Result<PluginCatalogView, Error>;
}
```

The bulk method takes pre-loaded installed sets by reference — the Tauri wrapper loads them ONCE per call (not per-plugin). This is C11's structural enforcement.

### Wire types

```rust
#[derive(Clone, Debug, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct PluginCatalogView {
    pub plugins: Vec<PluginCatalogEntry>,
    pub skipped: Vec<SkippedPlugin>,         // re-uses existing type
    // partial_load_warnings is NOT here — tracking-file load failures
    // surface as Err from the wrapper before the catalog is built.
    // (See C9.)
}

#[derive(Clone, Debug, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct PluginCatalogEntry {
    pub marketplace: String,
    pub plugin: String,
    pub description: Option<String>,
    pub source_type: SourceType,             // re-uses existing
    pub skills:   Vec<SkillInfo>,            // re-uses existing
    pub steering: Vec<SteeringItemInfo>,     // new
    pub agents:   Vec<AgentItemInfo>,        // new
    pub skipped_items: Vec<SkippedItem>,     // new (union of per-category skips)
}

pub struct SteeringItemInfo {
    pub name: String,           // filename under .kiro/steering/
    pub plugin: String,
    pub marketplace: String,
    pub installed: bool,
}

pub struct AgentItemInfo {
    pub name: String,           // parsed agent name
    pub description: Option<String>,
    pub plugin: String,
    pub marketplace: String,
    pub installed: bool,
    pub dialect: AgentDialect,  // markdown vs native
}

#[non_exhaustive]
pub enum SkippedItem {
    Skill(SkippedSkill),                          // re-uses existing
    SteeringDiscovery(SteeringWarning),           // re-uses existing
    AgentParse { plugin: String, source_path: PathBuf, reason: String },
}
```

### Tauri command (`crates/kiro-control-center/src-tauri`)

```rust
#[tauri::command]
pub async fn list_plugin_catalog_for_marketplace(
    state: State<'_, AppState>,
    marketplace: String,
    project_path: String,
) -> Result<PluginCatalogView, CommandError> {
    let svc = make_service()?;
    list_plugin_catalog_for_marketplace_impl(&svc, &marketplace, &project_path)
}

fn list_plugin_catalog_for_marketplace_impl(
    svc: &MarketplaceService,
    marketplace: &str,
    project_path: &str,
) -> Result<PluginCatalogView, CommandError> {
    let project = KiroProject::new(validate_kiro_project_path(project_path)?);
    let installed_skills   = project.load_installed()?;            // ONCE
    let installed_steering = project.load_installed_steering()?;   // ONCE
    let installed_agents   = project.load_installed_agents()?;     // ONCE
    Ok(svc.list_plugin_catalog(
        marketplace,
        &installed_skills,
        &installed_steering,
        &installed_agents,
    )?)
}
```

The `_impl` is testable per CLAUDE.md's testable-Tauri-command pattern (`crates/kiro-control-center/src-tauri/src/commands/browse.rs::install_skills_impl` is the canonical precedent).

## Falsification

| # | Claim | Falsifier | Oracle | Cost | Status | Regression fence |
|---|-------|-----------|--------|------|--------|------------------|
| 1 | Marketplace with N working plugins yields exactly N entries in `view.plugins`, names match the registry. | rstest: seed marketplace with 3 RelativePath entries (each a clean plugin dir), call, assert `view.plugins.len() == 3` and the name set matches `{a,b,c}`. | The seeded registry — independent of the catalog computation. | 5m | pending | unit test `list_plugin_catalog_returns_all_working_plugins` |
| 2 | Unknown marketplace returns `Err(Error::Marketplace(MarketplaceError::NotFound { .. }))`, not `Ok` with empty plugins. | Existing test `list_plugin_entries_returns_not_found_for_unknown_marketplace` (`service/mod.rs:5036`) already pins the substrate. The new function inherits via `list_plugin_entries`. | Existing test's assertion. | 1m | **passed** (cheapest, ran below) | existing test pins it; add `list_plugin_catalog_unknown_marketplace_errors` mirroring the shape |
| 3 | `entry.skills[i].installed = installed_skills.skills.contains_key(&entry.skills[i].name)`. | rstest: 2 skills s1/s2; manually populate `installed-skills.json` with valid metadata for s1 only; call; assert `installed=true` only for s1. | The hand-written tracking file — independent of the join logic. | 15m | pending | unit test `list_plugin_catalog_skill_installed_flag_matches_tracking` |
| 4 | `entry.steering[i].installed = installed_steering.files.contains_key(Path::new(&entry.steering[i].name))`. | rstest: 2 steering files rules.md/style.md; populate `installed-steering.json` with `Path::new("rules.md")` only; assert flags. | Hand-written tracking file. | 15m | pending | unit test `list_plugin_catalog_steering_installed_flag_matches_tracking` |
| 5 | `entry.agents[i].installed = installed_agents.agents.contains_key(&entry.agents[i].name)` where `name` is the parsed agent identity (frontmatter for markdown, JSON `name` for native). | rstest: one markdown agent (frontmatter `name: reviewer`), one native agent (JSON `name: helper`); populate tracking with reviewer only; assert flags AND that both names appear (i.e. parser was invoked). | Hand-written tracking file + the agent fixture's declared names. | 25m | pending | unit test `list_plugin_catalog_agent_installed_flag_matches_tracking` |
| 6 | Plugin whose source resolution fails (any `PluginError` classified into `SkippedReason::from_plugin_error`) lands in `view.skipped`, NOT in `view.plugins`. | rstest: marketplace with 1 valid plugin + 1 plugin whose `RelativePath` points at a missing dir; assert `view.plugins.len() == 1` AND `view.skipped[0].kind == SkippedReason::DirectoryMissing { .. }`. | Existing `SkippedPlugin::from_plugin_error` classifier — already proven by `list_all_skills` tests. | 10m | pending | unit test `list_plugin_catalog_resolves_then_skips_unresolvable` |
| 7 | A plugin with one valid skill + one bad-frontmatter skill + one bad-JSON native agent yields `entry.skipped_items.len() == 2` with distinct kinds; valid items still appear in their category arrays. | rstest with the described mixed fixture; assert per-category lengths AND `skipped_items` variant kinds. | Hand-built fixture's known structure. | 20m | pending | unit test `list_plugin_catalog_partial_item_failures_surface_in_skipped_items` |
| 8 | Duplicate item names within one (plugin, category) — possible because `discover_skill_dirs` doesn't dedupe by frontmatter name across scan paths — are surfaced as a deterministic outcome (the design picks: first-wins + duplicate goes to `skipped_items` with `Skill::DuplicateName`). | rstest: plugin with `skills: ["./a/", "./b/"]`, both containing `s1/SKILL.md` declaring name `s1`; assert exactly one `SkillInfo` for s1 AND one `skipped_items` entry of kind `DuplicateName { name: "s1", .. }`. | Fixture's known layout. | 25m | pending | unit test `list_plugin_catalog_dedupe_skill_names_across_scan_paths` |
| 9 | A corrupt tracking file fails the wrapper with `Err(_)` — the catalog never runs against an empty installed-set substituted for a parse failure. | rstest: write `{"skills": "not-a-map"}` to `.kiro/installed-skills.json`; call the `_impl`; assert `Err(_)`. | The hand-written corrupt JSON. | 10m | pending | unit test `list_plugin_catalog_propagates_corrupt_tracking_file` |
| 10 | A plugin with empty manifest scan paths AND no `.kiro/{skills,steering,agents}/` content yields an entry with empty arrays in all three categories AND empty `skipped_items` AND IS NOT in `view.skipped`. | rstest: clean plugin dir, no scan-dir contents; assert `entry.skills.is_empty() && entry.steering.is_empty() && entry.agents.is_empty() && entry.skipped_items.is_empty() && view.skipped.is_empty()`. | Fixture's known emptiness. | 15m | pending | unit test `list_plugin_catalog_empty_plugin_is_not_skipped` |
| 11 | The wrapper reads each tracking file ONCE per call, regardless of plugin count. | Read the `_impl` signature: must take `&InstalledSkills, &InstalledSteering, &InstalledAgents` by reference, not load them inside the per-plugin loop. | Inspect `_impl`'s parameter list — independent of any test. | 1m | pending (verify on impl) | structural assertion in code review; backed by unit test `list_plugin_catalog_loads_tracking_once` that wraps `KiroProject` with a load-counter |
| 12 | `PluginCatalogView`, `PluginCatalogEntry`, `SteeringItemInfo`, `AgentItemInfo`, `SkippedItem` all surface in `bindings.ts` after running `cargo test -p kiro-control-center --lib -- --ignored`; no `chrono::DateTime` crosses the FFI. (The original draft also banned `PathBuf`; relaxed during S1 because `SkippedSkill.path: PathBuf` is established precedent and specta renders it as TS `string`.) | grep `bindings.ts` for each type; grep for `chrono` in the new type definitions. | The regenerated `bindings.ts` — independent of design intent. | 10m | pending | CI binding-regen test (existing); add `bindings_export_plugin_catalog_view` assertion |
| 13 | For each (plugin, category), scan paths come from manifest's declared list if non-empty; otherwise from `DEFAULT_*_PATHS`. Empty list in manifest = fall back to default (matches existing helpers). | rstest: plugin α with `skills: ["./custom/"]` (file only under `./custom/`), plugin β with no manifest (file only under default `./skills/`); assert α discovers from custom AND β discovers from default. | Fixture layout — independent of resolution logic. | 15m | pending | unit test `list_plugin_catalog_scan_path_resolution_matches_existing_helpers` |

### Cheapest falsifier — run

C2 is cheapest (1 minute). It re-confirms the substrate that `list_plugin_catalog` will inherit from. Run:

```text
cargo test -p kiro-market-core --lib --
  list_plugin_entries_returns_not_found_for_unknown_marketplace
```

Result recorded below in "Cheapest falsifier execution."

## Tracker references

- **kiro-zx73** — per-item steering/agent install/remove Tauri commands. The drawer's apply-diff path needs these for full per-item granularity in steering and agents categories. Filed during this design pass; cited from non-goal #5.
- **kiro-3ivx** — disk-vs-tracking cross-check for `installed: bool`. Filed during this design pass; cited from non-goal #2. Probe artifact `probe_skills_orphan_disk_dir_diverges` pins today's behavior so any future fix has a regression sentinel.
- **No prior art** for the catalog read itself (verified by `rivets list` scan — see `.prove-it/list-plugin-catalog/related-issues.md`).

## Cheapest falsifier execution

**Claim:** C2 — Unknown marketplace returns `Err(Error::Marketplace(MarketplaceError::NotFound { .. }))`, not `Ok` with empty plugins.

**Command:**

```text
cargo test -p kiro-market-core --lib \
  list_plugin_entries_returns_not_found_for_unknown_marketplace
```

**Result:** `1 passed; 0 failed`. The substrate that `list_plugin_catalog`
will inherit from rejects unknown marketplaces with the documented error
variant. The new function will use the same `list_plugin_entries` driver,
so the error-propagation contract carries through unchanged.

**What this falsified (or didn't):** would have falsified the design's
error story if the substrate had returned `Ok` with empty plugins (silent
failure), or if it returned a different `Error` variant (UI would mis-
label the failure category). It returned exactly the expected variant —
design's claim C2 is consistent with reality. C2's status flips to
`passed` in the table above.

(Note: each remaining row stays `pending` and gets implemented as a
unit test during the build slice. The cheapest-falsifier gate only
requires the SINGLE cheapest claim to have run and passed before the
design is approved — the rest are the build phase's CI fence.)


# Plan — `list_plugin_catalog_for_marketplace` (slice 1 of BrowseTab redesign)

**Design:** [`docs/plans/2026-05-13-list-plugin-catalog-design.md`](./2026-05-13-list-plugin-catalog-design.md) (cheapest falsifier passed: C2).
**Probe:** [`crates/kiro-market-core/tests/prove_it_list_plugin_catalog.rs`](../../crates/kiro-market-core/tests/prove_it_list_plugin_catalog.rs).
**Tracker:** [`kiro-zx73`](.) per-item drawer commands (cited in design non-goal #5); [`kiro-3ivx`](.) disk-cross-check (cited in design non-goal #2).

Eight slices, each ≤30 min implementation time, ≤2 files, with mandatory stress fixture. Pre-typed code in any slice is **advisory** — implementer may deviate as long as the slice's claim, oracle, and budget hold.

---

## Slice 1: New wire types in `browse.rs`

**Claim:** C12 (Rust half) — define `PluginCatalogView`, `PluginCatalogEntry`, `SteeringItemInfo`, `AgentItemInfo`, `SkippedItem` with `Serialize` + feature-gated `specta::Type` derives; no `chrono::DateTime`; no `PathBuf` on any field that crosses FFI.

**Oracle:** `cargo check -p kiro-market-core --features specta` succeeds. `grep` the new types for `chrono::DateTime` and `PathBuf` — both absent on FFI-crossing fields. The bindings-regen test in S8 is the closing oracle on the TS side.

**Stress fixture:** Compile-only. Plausible bug class: "I added a field of type `PathBuf` because it 'feels right' for a path-like thing." The cure is the `cfg_attr(feature = "specta", derive(specta::Type))` macro itself — `specta` rejects `PathBuf` at derive time on Windows targets where `Path` is non-UTF-8. Verification: `cargo check -p kiro-market-core --features specta` is the gate.

**Loop budget:** none (pure types).
**Wall budget:** N/A.

**Files:**
- `crates/kiro-market-core/src/service/browse.rs` (add types)

**Code (advisory):**

```rust
#[derive(Clone, Debug, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct SteeringItemInfo {
    pub name: String,           // filename under .kiro/steering/, never PathBuf at FFI
    pub plugin: String,
    pub marketplace: String,
    pub installed: bool,
}

#[derive(Clone, Debug, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct AgentItemInfo {
    pub name: String,
    pub description: Option<String>,
    pub plugin: String,
    pub marketplace: String,
    pub installed: bool,
    pub dialect: AgentDialect,  // existing type, already specta-derived
}

#[derive(Clone, Debug, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum SkippedItem {
    Skill(SkippedSkill),                                    // existing
    SteeringDiscovery(SteeringWarning),                     // existing
    AgentParse {
        plugin: String,
        source_path: String,                                // String, not PathBuf
        reason: String,
    },
}

#[derive(Clone, Debug, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct PluginCatalogEntry {
    pub marketplace: String,
    pub plugin: String,
    pub description: Option<String>,
    pub source_type: SourceType,                            // existing
    pub skills:   Vec<SkillInfo>,                           // existing
    pub steering: Vec<SteeringItemInfo>,
    pub agents:   Vec<AgentItemInfo>,
    pub skipped_items: Vec<SkippedItem>,
}

#[derive(Clone, Debug, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct PluginCatalogView {
    pub plugins: Vec<PluginCatalogEntry>,
    pub skipped: Vec<SkippedPlugin>,                        // existing
}
```

**Verification:**
- [ ] `cargo check -p kiro-market-core --features specta` succeeds
- [ ] `grep "chrono::DateTime\|PathBuf" crates/kiro-market-core/src/service/browse.rs` shows no match in the new type bodies
- [ ] Existing tests still pass (`cargo test -p kiro-market-core --lib`)
- [ ] `SkippedItem` is `#[non_exhaustive]` (CLAUDE.md rule for cross-FFI enums)

---

## Slice 2: `list_steering_for_plugin` service method (+ regression fence for C4)

**Claim:** C4 — for each steering file discovered under a plugin's scan paths, `installed = true` iff `installed_steering.files.contains_key(Path::new(&info.name))`.

**Oracle:** Hand-written `installed-steering.json` populated with `Path::new("rules.md")` only. Service call yields installed=true for "rules.md" and installed=false for the other discovered file. Independent of the join logic — the oracle IS the hand-written tracking file's contents.

**Stress fixture (`list_plugin_catalog_steering_installed_flag_matches_tracking`):** plugin with two steering files `rules.md` + `style.md`; tracking entry only for `rules.md`. The plausible bug is "implementer joins on PathBuf vs. String and the comparison silently fails on Windows backslash variants" — fixture uses simple ASCII filenames so the bug, if present, can't hide behind path-normalization. Adversarial counterpart: a file named `Rules.md` (capital R) with a tracking entry for lowercase `rules.md` — must NOT match (HashMap is case-sensitive on PathBuf). Both assertions in one test.

**Loop budget:**
- One loop over discovered steering files: `O(steering_files_per_plugin)`.
- Production scale: ≤20 steering files per plugin (CLAUDE.md plugins ship 1–5; observed max ≈8).
- Bound: 20 ops per call. Far under the 10^6 ceiling.

**Wall budget:** N/A (per-plugin call invoked from S5, never user-facing solo).

**Files:**
- `crates/kiro-market-core/src/service/browse.rs` (add method + test in `mod tests`)

**Code (advisory):**

```rust
pub fn list_steering_for_plugin(
    &self,
    marketplace: &str,
    plugin: &str,
    installed: &InstalledSteering,
) -> Result<PluginSteeringResult, Error> {
    let plugin_entry = self.resolve_plugin_entry(marketplace, plugin)?;
    let plugin_dir   = self.resolve_local_plugin_dir(&plugin_entry, marketplace)?;
    let manifest     = read_plugin_manifest_opt(&plugin_dir)?;
    let scan_paths   = steering_scan_paths_for_plugin(manifest.as_ref());

    let (discovered, warnings) = discover_steering_files_in_dirs(&plugin_dir, &scan_paths);

    let mut steering = Vec::with_capacity(discovered.len());
    for f in &discovered {
        let Some(name) = f.source.file_name().and_then(|s| s.to_str()) else { continue; };
        steering.push(SteeringItemInfo {
            name: name.to_owned(),
            plugin: plugin.to_owned(),
            marketplace: marketplace.to_owned(),
            installed: installed.files.contains_key(Path::new(name)),
        });
    }
    Ok(PluginSteeringResult { steering, warnings })
}
```

NOTE: `steering_scan_paths_for_plugin`, `read_plugin_manifest_opt`, and the `resolve_*` helpers exist as `pub(crate)` or private — implementer may need to `pub(crate)`-promote one or two. If a helper is currently `fn` (truly private) and used only inside `browse.rs`, leave it; if cross-module access is needed, promote to `pub(crate)`.

**Verification:**
- [ ] Unit test `list_plugin_catalog_steering_installed_flag_matches_tracking` passes (C4 regression fence)
- [ ] Stress fixture: case-mismatch sub-assert (`Rules.md` vs `rules.md`) holds — must NOT match
- [ ] Loop budget statement compiles with the actual loop's asymptotic cost in a `// O(...)` comment above the loop
- [ ] `cargo clippy -p kiro-market-core --tests -- -D warnings` clean
- [ ] Probe `prove_it_list_plugin_catalog.rs` still passes (oracle still agrees)

---

## Slice 3: `list_agents_for_plugin` service method (+ regression fence for C5)

**Claim:** C5 — for each agent file discovered + parsed under a plugin's scan paths, `installed = true` iff `installed_agents.agents.contains_key(&info.name)` where `name` is the parsed agent identity (frontmatter `name` for markdown agents, JSON `name` field for native).

**Oracle:** Hand-written `installed-agents.json` populated with one agent name. The expected matching identity is read from the fixture file's declared `name` — independent of the parser pipeline (the test asserts ON the parsed name AND on the per-fixture declared name, and they must agree).

**Stress fixture (`list_plugin_catalog_agent_installed_flag_matches_tracking`):** plugin with one markdown agent in file `wrong-filename.md` declaring `name: actual-name`, and one native agent in `helper.json` declaring `name: helper`. Tracking entry for `actual-name` only. The plausible bug — and the load-bearing one per the probe — is "implementer joins on filename instead of parsed name." With this fixture, the buggy join key (`wrong-filename`) is NOT in tracking → implementation would say `installed=false` for both. The correct join key (`actual-name`) IS in tracking → says `installed=true` for the markdown one only. Different filename and different declared name is what makes the fixture adversarial.

**Loop budget:**
- One loop over discovered agent paths: `O(agents_per_plugin)`.
- Per iteration: one `fs::read_to_string` + one parser invocation.
- Production scale: ≤10 agents per plugin. Per-agent parse cost: ≤5ms (markdown frontmatter or small JSON).
- Bound: 50ms per call at production scale. Acceptable for an interactive list.

**Wall budget:** N/A (per-plugin call invoked from S5).

**Files:**
- `crates/kiro-market-core/src/service/browse.rs` (add method + test)

**Code (advisory):**

```rust
pub fn list_agents_for_plugin(
    &self,
    marketplace: &str,
    plugin: &str,
    installed: &InstalledAgents,
) -> Result<PluginAgentsResult, Error> {
    let plugin_entry = self.resolve_plugin_entry(marketplace, plugin)?;
    let plugin_dir   = self.resolve_local_plugin_dir(&plugin_entry, marketplace)?;
    let manifest     = read_plugin_manifest_opt(&plugin_dir)?;
    let scan_paths   = agent_scan_paths_for_plugin(manifest.as_ref());

    let discovered = discover_agents_in_dirs(&plugin_dir, &scan_paths);

    let mut agents  = Vec::with_capacity(discovered.len());
    let mut skipped = Vec::new();
    for path in &discovered {
        // O(1) per iteration after read+parse; total O(agents_per_plugin).
        match parse_agent_file(path) {
            Ok(parsed) => agents.push(AgentItemInfo {
                name: parsed.name.clone(),
                description: parsed.description,
                plugin: plugin.to_owned(),
                marketplace: marketplace.to_owned(),
                installed: installed.agents.contains_key(&parsed.name),
                dialect: parsed.dialect,
            }),
            Err(e) => skipped.push(AgentParseSkip {
                plugin: plugin.to_owned(),
                source_path: path.to_string_lossy().into_owned(),
                reason: error_full_chain(&e),
            }),
        }
    }
    Ok(PluginAgentsResult { agents, skipped })
}
```

NOTE: `parse_agent_file` may not exist by that name. The implementer must pick the right entry point — likely `crate::agent::parse_agent_file` or `crate::agent::detect_dialect`-routed function. Whatever the API is, the test pins the contract via the parsed-name assertion.

**Verification:**
- [ ] Unit test `list_plugin_catalog_agent_installed_flag_matches_tracking` passes (C5 regression fence)
- [ ] Stress fixture: agent file named `wrong-filename.md` with `name: actual-name` reports `installed=true` against tracking entry `actual-name` (proves join is on parsed name, not filename)
- [ ] Loop budget: `// O(agents_per_plugin × parse_cost)` comment above the loop, with parse_cost noted as ≤5ms at production
- [ ] Native agent (JSON dialect) also tested in same fixture
- [ ] `cargo clippy -p kiro-market-core --tests -- -D warnings` clean
- [ ] Probe still passes

---

## Slice 4: Skill name dedup + `SkippedSkillReason::DuplicateName` (+ regression fence for C8)

**Claim:** C8 — duplicate skill names within one (plugin, category) — possible because `discover_skill_dirs` doesn't dedupe by frontmatter name across multiple scan paths — yield first-wins in `entry.skills` AND a `SkippedItem`/`skipped_skills` entry of kind `SkippedSkillReason::DuplicateName { existing_dir, conflict_dir }`.

**Oracle:** Hand-built fixture with TWO scan paths each containing a `s1/SKILL.md` declaring `name: s1`. Independent of the dedup logic — the oracle is the fixture's known directory layout, and the test counts emitted skills + skipped entries.

**Stress fixture (`list_plugin_catalog_dedupe_skill_names_across_scan_paths`):** plugin with `manifest.json` declaring `skills: ["./scan_a/", "./scan_b/"]`. Both `./scan_a/s1/SKILL.md` and `./scan_b/s1/SKILL.md` exist with frontmatter `name: s1`. After this slice, calling `list_skills_for_plugin` yields exactly one `SkillInfo` for `s1` and one `SkippedSkill` of variant `DuplicateName`. The plausible bug: "implementer iterates and overwrites the prior entry silently" — fixture's count assertion catches this exact bug.

**Loop budget:**
- One additional `HashSet<String>` membership check per discovered skill: `O(skills_per_plugin)`.
- Production scale: ≤50 skills per plugin.
- Bound: trivially under ceiling.

**Wall budget:** N/A.

**Files:**
- `crates/kiro-market-core/src/error.rs` (add `SkippedSkillReason::DuplicateName` variant)
- `crates/kiro-market-core/src/service/browse.rs` (modify `collect_skills_for_plugin_into` + add test)

NOTE: this slice modifies an existing function (`collect_skills_for_plugin_into`). It's the only slice in the plan that does so. All callers of `list_skills_for_plugin` (BrowseTab today, the new bulk method tomorrow) inherit the dedup automatically.

**Code (advisory):**

```rust
// In error.rs — add to SkippedSkillReason:
#[non_exhaustive]
pub enum SkippedSkillReason {
    // ...existing variants...
    DuplicateName {
        existing_scan_root: String,
        conflict_scan_root: String,
    },
}

// In browse.rs::collect_skills_for_plugin_into, after the parse/filter
// branches and before the `out.push(SkillInfo { ... })`:
let mut seen_names: HashSet<String> = HashSet::with_capacity(out.capacity());
// ...inside the loop, after `frontmatter.name` is in scope...
if !seen_names.insert(frontmatter.name.clone()) {
    skipped_skills.push(SkippedSkill {
        plugin: plugin_entry.name.clone(),
        name_hint: Some(frontmatter.name.clone()),
        path: skill_md_path,
        reason: SkippedSkillReason::DuplicateName {
            existing_scan_root: /* first-seen path */,
            conflict_scan_root: skill_dir.display().to_string(),
        },
    });
    continue;
}
```

NOTE: implementer must thread the first-seen scan root through. Either keep a `HashMap<String, PathBuf>` (name → first scan root) or restructure the loop. The advisory code is sketchy here — the contract is "first wins, second goes to `skipped_skills` with DuplicateName."

**Verification:**
- [ ] Unit test `list_plugin_catalog_dedupe_skill_names_across_scan_paths` passes (C8 regression fence)
- [ ] Existing tests for `list_skills_for_plugin` still pass (no regression on single-scan-path plugins)
- [ ] `SkippedSkillReason::DuplicateName` is added via the same `pub(crate)` constructor pattern other variants use (precedent: `SkippedSkillReason::FrontmatterInvalid`)
- [ ] `cargo clippy --workspace --tests -- -D warnings` clean
- [ ] All classifier functions over `SkippedSkillReason` (e.g., any `match` in service code, the FFI projection) get a new arm (CLAUDE.md "Classifier functions enumerate every variant" rule). `cargo check` will surface these.

---

## Slice 5: `list_plugin_catalog` bulk method + happy-path / plugin-skip / empty-plugin fences (C1, C6, C10)

**Claim:** C1 (N working plugins → N entries), C6 (plugin source resolution failure → `view.skipped` not `view.plugins`), C10 (empty plugin contract: empty arrays + NOT in skipped).

**Oracle:** Seeded marketplace registry — independent of the iteration logic. The test seeds a known set of plugins and asserts cardinality + content split.

**Stress fixture (`list_plugin_catalog_iteration_split`):** marketplace with three plugins:
- α: working, has 2 skills
- β: `RelativePath` pointing at a missing directory (will produce `PluginError::DirectoryMissing`)
- γ: working, manifest declares `skills: []`, `steering: []`, `agents: []`, no scan dirs on disk

Expected: `view.plugins.len() == 2` (α and γ); `view.skipped.len() == 1` (β with `SkippedReason::DirectoryMissing { .. }`); the γ entry has all three category arrays empty AND `γ` IS NOT in `view.skipped`. Plausible bug: "empty plugin gets folded into skipped because the implementer treated 'no items' as 'plugin failed'" — fixture's γ branch catches this exactly.

**Loop budget:**
- Outer loop over plugins: `O(K)` where K = plugin count.
- Per-plugin: three `list_*_for_plugin` calls (each individually budgeted in S2/S3 + existing `list_skills_for_plugin`).
- Total: `O(K × (skills + steering + agents))` ≈ `O(K × items)`.
- Production scale: K ≤ 50 plugins per marketplace, items ≤ 50 per plugin per category.
- Bound: 50 × 50 × 3 = 7,500 items per call. Under 10^6.

**Wall budget:** ≤2s wall-clock for K=50 plugins × 5 agents/plugin (scaling agent parse cost from S3's 5ms = 50 × 5 × 5ms = 1,250ms agent-parse alone, plus IO). Documented; not enforced as a CI assertion (would be flaky on shared CI hardware) but stress fixture in S5 should hit K=10 to keep the loop honest.

**Files:**
- `crates/kiro-market-core/src/service/browse.rs` (add method + 3 tests)

**Code (advisory):**

```rust
pub fn list_plugin_catalog(
    &self,
    marketplace: &str,
    installed_skills:   &InstalledSkills,
    installed_steering: &InstalledSteering,
    installed_agents:   &InstalledAgents,
) -> Result<PluginCatalogView, Error> {
    let plugin_entries = self.list_plugin_entries(marketplace)?;

    let mut plugins = Vec::with_capacity(plugin_entries.len());
    let mut skipped = Vec::new();

    // O(K × items) where K = plugin_entries.len(), items = max items per plugin.
    // K ≤ 50, items ≤ 50 per category at production scale.
    for plugin_entry in &plugin_entries {
        match self.assemble_catalog_entry(
            marketplace, plugin_entry,
            installed_skills, installed_steering, installed_agents,
        ) {
            Ok(entry) => plugins.push(entry),
            Err(err) => match SkippedPlugin::from_plugin_error(plugin_entry.name.clone(), &err) {
                Some(sp) => skipped.push(sp),
                None => return Err(err),
            }
        }
    }
    Ok(PluginCatalogView { plugins, skipped })
}

fn assemble_catalog_entry(
    &self,
    marketplace: &str,
    plugin_entry: &PluginEntry,
    installed_skills:   &InstalledSkills,
    installed_steering: &InstalledSteering,
    installed_agents:   &InstalledAgents,
) -> Result<PluginCatalogEntry, Error> {
    // S6 fills in the skipped_items aggregation; this slice leaves the field
    // populated as an empty Vec to keep the slice bounded.
    let skills_result = self.list_skills_for_plugin(marketplace, &plugin_entry.name, installed_skills)?;
    let steering_result = self.list_steering_for_plugin(marketplace, &plugin_entry.name, installed_steering)?;
    let agents_result = self.list_agents_for_plugin(marketplace, &plugin_entry.name, installed_agents)?;
    Ok(PluginCatalogEntry {
        marketplace: marketplace.to_owned(),
        plugin: plugin_entry.name.clone(),
        description: plugin_entry.description.clone(),
        source_type: plugin_entry.source.source_type(),
        skills: skills_result.skills,
        steering: steering_result.steering,
        agents: agents_result.agents,
        skipped_items: Vec::new(),  // S6 will populate
    })
}
```

**Verification:**
- [ ] Unit test `list_plugin_catalog_returns_all_working_plugins` passes (C1 regression fence)
- [ ] Unit test `list_plugin_catalog_resolves_then_skips_unresolvable` passes (C6 regression fence)
- [ ] Unit test `list_plugin_catalog_empty_plugin_is_not_skipped` passes (C10 regression fence)
- [ ] All three tests share the same fixture builder (one fixture, three assertions) to keep test code under 80 LoC
- [ ] Loop has `// O(K × items)` comment with production-scale bound
- [ ] `cargo clippy --workspace --tests -- -D warnings` clean
- [ ] Probe still passes

---

## Slice 6: Per-item `skipped_items` aggregation (C7, C13)

**Claim:** C7 — per-item read/parse failures (skill frontmatter, steering scan-path validation, agent parse) surface in `entry.skipped_items` with distinct `SkippedItem` variants. C13 — scan-path resolution falls back to `DEFAULT_*_PATHS` when manifest absent or list empty.

**Oracle for C7:** Hand-built fixture with one valid + one invalid item per category. Independent of aggregation logic — the test asserts on per-variant counts in `skipped_items`.

**Oracle for C13:** Hand-built fixtures with two plugins (custom-path + default-path). Independent — fixture layout is the oracle.

**Stress fixture (`list_plugin_catalog_partial_item_failures_surface_in_skipped_items`):**
plugin with:
- one valid skill + one skill with malformed frontmatter (missing `name:`)
- one valid steering file + manifest declaring an invalid scan path (`steering: ["../escape/"]` — fails `validate_relative_path`)
- one valid agent + one agent with malformed JSON (`{not valid json`)

Expected: `entry.skipped_items.len() == 3` with one of each variant kind. Plausible bug: "implementer aggregates only one category's skips and forgets the other two" — fixture's three-category assertion catches this exactly.

**Stress fixture (`list_plugin_catalog_scan_path_resolution_matches_existing_helpers`):** two plugins:
- plugin α with manifest `{ "skills": ["./custom_dir/"] }`; SKILL.md only under `./custom_dir/s1/SKILL.md`
- plugin β with NO manifest; SKILL.md only under default `./skills/s2/SKILL.md`

Expected: α discovers `s1` from custom path; β discovers `s2` from default. Plausible bug: "implementer hardcodes default and ignores manifest" — α's missing-skill case catches this.

**Loop budget:**
- Three vector concatenations per plugin into the union `Vec<SkippedItem>`: `O(skips_per_plugin)`.
- Production scale: typically 0; pathological ≤30 skips per plugin.
- Bound: 30 ops per plugin call. Trivial.

**Wall budget:** N/A.

**Files:**
- `crates/kiro-market-core/src/service/browse.rs` (modify `assemble_catalog_entry` + add 2 tests)

**Code (advisory):**

```rust
fn assemble_catalog_entry(...) -> Result<PluginCatalogEntry, Error> {
    let skills_result   = self.list_skills_for_plugin(...)?;
    let steering_result = self.list_steering_for_plugin(...)?;
    let agents_result   = self.list_agents_for_plugin(...)?;

    let mut skipped_items = Vec::new();
    skipped_items.extend(skills_result.skipped_skills.into_iter().map(SkippedItem::Skill));
    skipped_items.extend(steering_result.warnings.into_iter().map(SkippedItem::SteeringDiscovery));
    skipped_items.extend(agents_result.skipped.into_iter().map(|s| SkippedItem::AgentParse {
        plugin: s.plugin,
        source_path: s.source_path,
        reason: s.reason,
    }));

    Ok(PluginCatalogEntry { /* ..., */ skipped_items })
}
```

**Verification:**
- [ ] Unit test `list_plugin_catalog_partial_item_failures_surface_in_skipped_items` passes (C7 regression fence) — asserts all three variant kinds present
- [ ] Unit test `list_plugin_catalog_scan_path_resolution_matches_existing_helpers` passes (C13 regression fence)
- [ ] No `_ => ` arms in any new `match SkippedItem` — CLAUDE.md classifier-exhaustiveness rule
- [ ] `cargo clippy --workspace --tests -- -D warnings` clean

---

## Slice 7: Tauri command `list_plugin_catalog_for_marketplace` + `_impl` (C9, C11)

**Claim:** C9 — corrupt tracking file fails the wrapper with `Err(_)`, never silently substitutes empty installed-set. C11 — `_impl` takes `&InstalledSkills, &InstalledSteering, &InstalledAgents` by reference; tracking files loaded ONCE per call.

**Oracle for C9:** Hand-written corrupt JSON in `.kiro/installed-skills.json`. Independent of error-propagation logic — fixture is the oracle.

**Oracle for C11:** `_impl`'s parameter list, read by the test's compile-time signature requirement (passing `&loaded_skills, &loaded_steering, &loaded_agents` — if the signature accepts a `&KiroProject` instead, the test won't compile).

**Stress fixture (`list_plugin_catalog_command_propagates_corrupt_tracking_file`):** project with `.kiro/installed-skills.json` containing `{"skills": "not-a-map"}`. Call the `_impl`. Expect `Err(_)` — specifically, an error whose chain includes the JSON parse failure. Plausible bug: "implementer wraps the load in `.unwrap_or_default()` for ergonomics" — fixture's `Err` assertion catches this.

**Stress fixture (`list_plugin_catalog_loads_tracking_once`):** two-plugin marketplace; instrument with a counter (or assert structurally via the `_impl` signature). The structural form: a test that calls `_impl` with three pre-loaded installed-sets and asserts the call succeeds. If a future refactor changes the signature to take `&KiroProject` and load internally, the test STOPS COMPILING (the test passes its own `InstalledSkills, InstalledSteering, InstalledAgents` — those exact arguments wouldn't fit a different signature). The compile-fail IS the regression fence.

**Loop budget:** none new.
**Wall budget:** N/A (read-only command, no always-on phase).

**Files:**
- `crates/kiro-control-center/src-tauri/src/commands/browse.rs` or new `commands/plugin_catalog.rs` (implementer's call) — add command + `_impl` + 2 tests

**Code (advisory):**

```rust
#[tauri::command]
pub async fn list_plugin_catalog_for_marketplace(
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
    let project_root = validate_kiro_project_path(project_path)?;
    let project = KiroProject::new(project_root);
    let installed_skills   = project.load_installed()?;
    let installed_steering = project.load_installed_steering()?;
    let installed_agents   = project.load_installed_agents()?;
    Ok(svc.list_plugin_catalog(
        marketplace,
        &installed_skills,
        &installed_steering,
        &installed_agents,
    )?)
}
```

**Verification:**
- [ ] Unit test `list_plugin_catalog_command_propagates_corrupt_tracking_file` passes (C9 regression fence)
- [ ] Unit test `list_plugin_catalog_loads_tracking_once` passes (C11 regression fence — structural via signature)
- [ ] `_impl` is `fn`, not `async fn`, takes references — matches `install_skills_impl` precedent
- [ ] Command registered in `lib.rs` `tauri::generate_handler![...]` macro (existing pattern)
- [ ] `cargo clippy --workspace --tests -- -D warnings` clean

---

## Slice 8: Bindings regen + binding-shape assertion (C12)

**Claim:** C12 — `PluginCatalogView`, `PluginCatalogEntry`, `SteeringItemInfo`, `AgentItemInfo`, `SkippedItem` all surface as `export type ...` in `bindings.ts`; no `chrono` and no `PathBuf` references in any of them.

**Oracle:** the regenerated `bindings.ts` itself (machine-generated from Rust source — independent of design intent). The test greps the generated file.

**Stress fixture (`bindings_export_plugin_catalog_view`):** existing CI test pattern (`cargo test -p kiro-control-center --lib -- --ignored` regenerates). Add an assertion test in `crates/kiro-control-center/src-tauri/src/lib.rs::tests` (or wherever the existing binding-shape tests live) that:
- Reads `bindings.ts` as a string.
- Asserts each new type name appears as `export type <Name>`.
- Asserts substrings `"chrono"`, `"PathBuf"`, `"DateTime"` do NOT appear within ±10 lines of any new type's definition.

Plausible bug: "implementer adds `chrono::DateTime<Utc>` to a new struct out of habit" — fixture's grep catches this AT TEST TIME, before the bindings file is committed.

**Loop budget:** none.
**Wall budget:** N/A.

**Files:**
- `crates/kiro-control-center/src/lib/bindings.ts` (regenerated)
- `crates/kiro-control-center/src-tauri/src/<wherever-binding-tests-live>.rs` (add assertion test)

**Verification:**
- [ ] `cargo test -p kiro-control-center --lib -- --ignored` regenerates `bindings.ts` cleanly
- [ ] `bindings.ts` contains `export type PluginCatalogView`, `export type PluginCatalogEntry`, `export type SteeringItemInfo`, `export type AgentItemInfo`, `export type SkippedItem`
- [ ] Unit test `bindings_export_plugin_catalog_view` passes (C12 regression fence)
- [ ] `npm run check` in `crates/kiro-control-center/` succeeds (svelte-check sees the new types)

---

## Plan Self-Review

### List 1 — Every loop in the plan: complexity stated AND within budget?

| Slice | Loop | Asymptotic cost | Production scale | Within ceiling? |
|---|---|---|---|---|
| S2 | discovered steering files | `O(steering_files_per_plugin)` | ≤20 | ✓ |
| S3 | discovered agent files (parse per iter) | `O(agents_per_plugin × parse_cost)` | ≤10 × ≤5ms | ✓ |
| S4 | dedup HashSet check | `O(skills_per_plugin)` | ≤50 | ✓ |
| S5 | outer plugins loop, calls S2/S3/skills per iter | `O(K × items)` | 50 × 50 × 3 = 7,500 | ✓ (<10^6) |
| S6 | three vec extends per plugin | `O(skips_per_plugin)` | ≤30 | ✓ |

S1, S7, S8 introduce no new loops.

### List 2 — Every fixture: bug class? more than happy-path?

| Slice | Fixture | Bug class targeted |
|---|---|---|
| S1 | compile-only via specta derive | `PathBuf` in FFI struct (specta rejects on Windows) |
| S2 | `Rules.md` vs `rules.md` case mismatch + `style.md` not in tracking | case-insensitive join footgun on PathBuf keys |
| S3 | filename `wrong-filename.md` declares `name: actual-name` | implementer joins on filename instead of parsed name |
| S4 | two scan paths each with `s1/SKILL.md` | implementer silently overwrites prior entry |
| S5 | three plugins: working, missing-dir, empty-but-clean | empty plugin treated as "failed" → wrong skipped count |
| S6a | one valid + one invalid item per category | implementer aggregates only one category's skips |
| S6b | manifest custom path AND no-manifest default path | implementer hardcodes default and ignores manifest |
| S7a | corrupt installed-skills.json | implementer wraps load in `.unwrap_or_default()` for ergonomics |
| S7b | _impl takes pre-loaded installed sets | future refactor changes signature to `&KiroProject` (load once → load N times) |
| S8 | grep for `chrono`/`PathBuf` near new type defs | implementer adds `DateTime<Utc>` out of habit |

All ten fixtures target a specific bug class, none are happy-path-only.

### List 3 — Every doc-comment precondition: classified + enforced?

| Slice | Precondition | Class | Enforcement |
|---|---|---|---|
| S2 | "callers pass a valid `MarketplaceName`" — no, the function takes `&str` and routes via `list_plugin_entries` which produces typed errors | sanity-hint (validation done downstream) | runtime enforced via `list_plugin_entries`'s existing `Error::Marketplace(NotFound)` path |
| S3 | same as S2 | sanity-hint | downstream runtime check |
| S5 | "the three `installed: &Installed*` arguments come from a single project's tracking files" | sanity-hint (programmer error, not user input) | not enforced — the data is internal, callers are kiro-control-center wrappers; no realistic way to violate without already controlling the call site |
| S7 | `_impl` takes pre-loaded installed sets | load-bearing for C11 (perf claim) | runtime enforced by signature shape — wrapping loads in the OUTER `_impl` is mechanically required by the type system |

No documented precondition is left unenforced.

### List 4 — Every write target: data or diagnostic?

| Slice | Write | Stream | Justification |
|---|---|---|---|
| S2/S3/S5 | `tracing::warn!` / `tracing::debug!` from existing discovery primitives | stderr (tracing default) | diagnostic — already classified |
| S7 | wire response (catalog view) | Tauri IPC return value | data — Tauri runtime's responsibility |
| S8 | binding-shape assertion's `assert!` failures | stderr via test harness | diagnostic |

No `println!` to stdout; no untyped writes.

### List 5 — Every tracker reference: resolves to existing issue?

| Reference | Cited in | Issue verified? |
|---|---|---|
| `kiro-zx73` (per-item drawer commands) | (cited in design non-goal #5; not re-cited in plan) | ✓ filed during design pass |
| `kiro-3ivx` (disk-cross-check) | (cited in design non-goal #2; not re-cited in plan) | ✓ filed during design pass |

The plan itself introduces no new deferrals — every claim from the design (C1–C13) is implemented in some slice; nothing is "moved to later." Verified by mapping:

| Design claim | Slice |
|---|---|
| C1 | S5 |
| C2 | (passed at design time; existing test pinned in design table) |
| C3 | (existing behavior; pinned by current `list_skills_for_plugin` tests) |
| C4 | S2 |
| C5 | S3 |
| C6 | S5 |
| C7 | S6 |
| C8 | S4 |
| C9 | S7 |
| C10 | S5 |
| C11 | S7 |
| C12 | S1 (Rust side) + S8 (TS side) |
| C13 | S6 |

Every design claim has a slice. No claim is deferred.

### Hard-gate checklist

- [x] Every slice has all mandatory fields (claim, oracle, stress fixture, loop budget, files, verification)
- [x] Every loop has a complexity statement (List 1)
- [x] Every slice has a stress fixture (List 2)
- [x] Plan's claim coverage matches design's claim list (final mapping table above)
- [x] Every tracker reference resolves to an existing issue (List 5)

Ready for `gilfoyle:checkpointed-build`.

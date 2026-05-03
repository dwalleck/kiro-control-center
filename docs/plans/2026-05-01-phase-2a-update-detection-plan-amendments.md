# Phase 2a — Plan Amendments

> **Status:** plan-review pass per `docs/plan-review-checklist.md`.
> Fixes drift between `2026-05-01-phase-2a-update-detection-plan.md` and the
> actual SHA at `40a6fc6` (post-PR-#95 main + Phase 2a design + plan).
> Format follows the precedent set by `2026-04-30-phase-1-5-type-safety-plan-amendments.md`.

5-gates pass via LSP-first + code-reviewer-style discipline (per A-8 + the
"two complementary passes" rule in CLAUDE.md). Pass found **3 CRITICAL**,
**3 IMPORTANT**, and **1 OBSERVATION**. The CRITICAL findings are
compile-blockers / wrong-shape errors of exactly the class P1.5-1 and
P1.5-2 caught — API-shape assumptions baked into the plan against an
out-of-date mental model.

Each amendment cites the gate that fired, names the original plan
text, gives the amended text, and explains the rationale. Apply
during execution; **P2a-1 requires a one-question design decision**
before Task 2 begins (call it out below).

---

## P2a-1 — Gate 5: `PluginEntry` has no `version` field; `available_version` resolution as written won't compile

**Original (Task 2, Step 2 — `check_plugin_for_update`):**

```rust
let plugin_entries = self.list_plugin_entries(plugin_info.marketplace.as_str())?;
let plugin_entry = plugin_entries
    .iter()
    .find(|p| p.name == plugin_info.plugin.as_str())
    .ok_or_else(|| { Error::Plugin(crate::error::PluginError::NotFound { ... }) })?;

let available_version = plugin_entry.version.clone();
```

**Drift.** `PluginEntry` is defined in `crates/kiro-market-core/src/marketplace.rs:32-36` with exactly three fields: `name: String`, `description: Option<String>`, `source: PluginSource`. There is **no `version` field**. The version field that the design references lives on `PluginManifest` (`crates/kiro-market-core/src/plugin.rs:17` — `pub version: Option<String>`) — i.e. it's in the per-plugin `plugin.json`, not in the marketplace's `marketplace.json` plugin list. `list_plugin_entries` returns `Vec<PluginEntry>` (verified at `service/mod.rs:1012-1042`). Calling `.version` on a `PluginEntry` won't compile.

The design's "Detection logic" step 3 also refers to "the marketplace plugin manifest's `version`" — meaning `PluginManifest.version`, not `PluginEntry.version`. The plan slid from "`PluginManifest.version`" to "`plugin_entry.version`" without noticing they're different types.

**Required design decision before Task 2 begins:**

(A) **Cache-load `plugin.json` per plugin during detection.** Resolve `PluginEntry.source` to a cache directory, `fs::read(plugin_dir.join("plugin.json"))`, parse via `PluginManifest::from_json`, read `manifest.version`. One extra file read per installed plugin during the scan. Matches the existing install path's resolution (which already loads `plugin.json` to discover the install contents).

(B) **Extend `PluginEntry` to include `version: Option<String>` populated at marketplace-load time.** Marketplace catalog parser (`try_read_manifest`) gains a "load each plugin's manifest upfront" step. Changes the marketplace catalog parse semantics; touches more code than (A); affects every consumer of `PluginEntry` (specta type emission included).

**Recommended: (A).** Keeps `PluginEntry` (the wire-format type for the marketplace catalog list) unchanged. Version is only needed for detection, not for the catalog browse UI. Matches existing install-path resolution pattern.

**Amended.** Replace Task 2's `check_plugin_for_update` `available_version` block:

```rust
// Resolve the plugin's cache directory by joining marketplace_path + the
// PluginSource. Both PluginSource::RelativePath and PluginSource::Structured
// resolve to a per-plugin subdir under <marketplace_root>; reuse the
// existing resolution helper (search install code for the pattern — likely
// in MarketplaceService::resolve_plugin_install_context).
let cache_dir = self.marketplace_path(plugin_info.marketplace.as_str());
let plugin_dir = match &plugin_entry.source {
    crate::marketplace::PluginSource::RelativePath(rel) => cache_dir.join(rel.as_str()),
    crate::marketplace::PluginSource::Structured(_) => {
        // Verify the existing resolution helper covers Structured sources;
        // most likely resolve_plugin_install_context has the canonical logic.
        // Reuse, don't reimplement.
        todo!("reuse resolve_plugin_install_context's PluginSource resolution");
    }
};

let manifest_bytes = std::fs::read(plugin_dir.join("plugin.json"))
    .map_err(|e| /* wrap into a typed PluginError::ManifestReadFailed-style variant */)?;
let manifest = crate::plugin::PluginManifest::from_json(&manifest_bytes)
    .map_err(|e| /* wrap — serde_json::Error must NOT leak per CLAUDE.md gate-4 */)?;
let available_version = manifest.version;
```

The plan must also (i) pick or define the right error variants for "plugin.json missing in cache" and "plugin.json malformed in cache" (both are per-plugin failures that should land in `PluginUpdateFailure.reason` via `error_full_chain`); CLAUDE.md's "map external errors at adapter boundary" recipe applies — `serde_json::Error` does NOT cross the public API; wrap it in a `#[non_exhaustive]` variant with `reason: String + error_full_chain(&err)`. (ii) Update Task 2's required-research LSP queries to include `workspaceSymbol query=resolve_plugin_install_context` so the implementer reuses the existing `PluginSource` resolution.

**Rationale.** Gate 5 (Type Design) and Gate 1 (Grounding). This is a compile-blocking error and the central data-flow assumption of Task 2; the LSP-first pass on `marketplace.rs` would have caught it. Without this fix the implementer wastes hours discovering the gap and then has to redesign the cache lookup mid-task. Also surfaces a Gate 4 concern (a new `serde_json::Error` translation point in `service/mod.rs`).

---

## P2a-2 — Gate 5: existing field is `failed: Vec<...>`, not `failures: Vec<...>`; rename inconsistency throughout the plan

**Original (Task 5 + Task 6, repeatedly):**

> Plan: `failures: Vec<RemovePluginFailure>` (file-structure summary, Step 1 commit message, Step 2 prose, "If `RemovePluginFailure` is referenced anywhere…")
> Plan Task 6 search: `grep -rn "skills_removed\|steering_removed\|agents_removed\|RemovePluginFailure" crates/kiro-control-center/`

**Drift.** The actual field on the *current* `RemovePluginResult` is `failed: Vec<RemovePluginFailure>` (`project.rs:392`), not `failures`. Existing rstests assert on `result.failed`. The Task 6 grep also misses references to the old field (`failed`).

The plan's *new* `RemoveSkillsResult` / `RemoveSteeringResult` / `RemoveAgentsResult` use a field named `failures` (Task 4, Step 1). Mixing the new naming convention with the old `failed` is fine in isolation, but the plan never explicitly calls out the rename — implementer reading "failures" everywhere may try to grep for `result.failed` references and miss the new-vs-old distinction.

**Amended.** Add to Task 5 Step 1:

> **Field-name change:** the existing `RemovePluginResult.failed: Vec<RemovePluginFailure>` becomes `result.<content_type>.failures: Vec<RemoveItemFailure>` per sub-result. The new types use `failures` (matching Task 4's additive types). Migrate callers explicitly; do not silently keep `failed` for backward compat — the type is breaking-changing anyway.

In Task 6 Step 1, expand the search:

```bash
grep -rn "skills_removed\|steering_removed\|agents_removed\|RemovePluginFailure\|\.failed\b" crates/kiro-control-center/
```

Filter results — `result.failed` also appears on `InstallSkillsResult` / `InstallSteeringResult` / `InstallAgentsResult`; only `RemovePluginResult.failed` references need updating. Use surrounding context to disambiguate.

**Rationale.** Gate 5 (type-shape contract). One sentence of plan prose prevents 30 minutes of confusion when the implementer sees `result.failures` in the migration sketch and `result.failed` in the existing code.

---

## P2a-3 — Gate 5: `KiroProject::remove_plugin` migration sketch references non-existent helpers (`load_installed_skills_or_default`, `remove_skill_internal`)

**Original (Task 5, Step 3 migration sketch):**

```rust
let mut skills = self.load_installed_skills_or_default()?;
// ...
match self.remove_skill_internal(&mut skills, &skill_name) { ... }
self.save_installed_skills(&skills)?;
```

**Drift.** Looking at the actual `remove_plugin` body (`project.rs:1429-1552`):
- The skill loader is `self.load_installed()` (returns `InstalledSkills`), not `self.load_installed_skills_or_default()`. There is no `_or_default` suffix; the loader already returns default on missing-file (`project.rs:795-806`).
- The cascade calls `self.remove_skill(name)` (line 1447), not `self.remove_skill_internal(&mut skills, &skill_name)`. There is no `remove_skill_internal` helper that takes `&mut InstalledSkills`. The current impl re-loads inside `remove_skill`.
- There is no `save_installed_skills(&skills)` call after the loop — the skill removal helper writes its own tracking on each call.
- `load_installed_steering` / `load_installed_agents` exist (lines 1009, 959), but again the cascade body relies on the inner `remove_*` helpers to handle their own writes.

The plan's migration sketch invents an in-memory-mutate-then-batch-save pattern that doesn't match the current code, and references two API names (`load_installed_skills_or_default`, `remove_skill_internal`) that don't exist. The implementer following this sketch produces non-compiling code AND changes the I/O semantics (one save per cascade vs. many in current code).

**Amended.** Rewrite Task 5 Step 3's migration sketch to match the existing skeleton — keep the per-iteration `remove_skill(name)` / `remove_steering_file(rel)` / `remove_agent(name)` calls and only change where successes/failures land. Concretely:

```rust
let skills = self.load_installed()?;
let skills_to_remove: Vec<String> = skills.skills.iter()
    .filter(|(_, meta)| meta.marketplace == *marketplace && meta.plugin == *plugin)
    .map(|(name, _)| name.clone()).collect();
for name in &skills_to_remove {
    match self.remove_skill(name) {
        Ok(()) => result.skills.removed.push(name.clone()),
        Err(e) => {
            warn!(skill = %name, plugin = %plugin, marketplace = %marketplace, error = %e, "remove_plugin: skill removal failed");
            result.skills.failures.push(RemoveItemFailure {
                item: name.clone(),
                error: crate::error::error_full_chain(&e),
            });
        }
    }
}

// Steering: load_installed_steering(), filter, for-loop calls self.remove_steering_file(rel).
// Agents: load_installed_agents(), filter, for-loop calls self.remove_agent(name).
// Native companions: trailing if-let on self.remove_native_companions_for_plugin(plugin, marketplace).
//   On Err -> push to result.agents.failures with item: format!("native_companions:{plugin}")
//   (the step is plugin-scoped, not per-companion-file).
```

**Native-companions removed-list ambiguity (design clarification needed):** The plan says `result.agents.removed` includes "native companion paths". But `remove_native_companions_for_plugin` returns `()` — it does NOT currently expose the list of removed companion paths. Pick one:

- **(α)** Limit `agents.removed` to "translated agent names + native agent names". Native_companions success = "the cascade step ran without error" but yields no per-file entries. Document this in Task 5 step 3.
- **(β)** Refactor `remove_native_companions_for_plugin` to return `Vec<PathBuf>` (the unlinked paths). Adds scope to Task 5.

**Recommended: (α)** for Phase 2a tight scope. Document as a Phase 2b follow-up if the UI wants per-companion granularity. Update Task 5 Step 3's `removed: Vec<String>` doc-comment for `RemoveAgentsResult` to reflect this.

**Rationale.** Gate 5 + Gate 1. The migration sketch is the highest-risk part of Task 5; a wrong sketch is exactly the kind of plan defect that costs an implementer cycles. The "removed companion paths" question is a real design ambiguity that the implementer cannot resolve without going back to the user.

---

## P2a-4 — Gate 5 (IMPORTANT): `source_hash` is `Option<String>` only on skills/agents; steering & native_companions store it as plain `String` — legacy fallback applies asymmetrically

**Original:** plan + design repeatedly reference "if `source_hash` is None for any tracked file (legacy install pre-Stage-1), fall back to version-string comparison only".

**Drift.** The four tracking-meta types differ:
- `InstalledSkillMeta.source_hash: Option<String>` (`project.rs:45`)
- `InstalledAgentMeta.source_hash: Option<String>` (`project.rs:80`)
- `InstalledSteeringMeta.source_hash: String` (`project.rs:151`) — NOT optional
- `InstalledNativeCompanionsMeta.source_hash: String` (`project.rs:117`) — NOT optional

The legacy-fallback flag therefore only ever flips inside the skills + agents arms of the walk; steering and native_companions always have a hash to compare. The plan's framing implies symmetric handling.

**Amended.** Add to Task 2 Step 2 implementer notes:

> **Per-content-type source_hash shape:** `InstalledSkillMeta.source_hash` and `InstalledAgentMeta.source_hash` are `Option<String>` (legacy entries pre-Stage-1 carry `None`). `InstalledSteeringMeta.source_hash` and `InstalledNativeCompanionsMeta.source_hash` are plain `String` (these tracking files were introduced after Stage 1 — no legacy entries exist). The legacy-fallback flag therefore only ever flips inside the skills + agents arms of the walk; steering and native_companions always have a hash to compare.

Also update the design's Detection-logic step 4 to mirror this.

**Rationale.** Gate 5. Half a finding because the implementer would notice on first compile (`Option<String>` vs `String` mismatch is loud), but the plan's framing implies symmetric handling, and an implementer might add unnecessary `Some(_)` wrapping or `if let Some(_) = ` ceremony for steering/native_companions.

---

## P2a-5 — Gate 5 (IMPORTANT): `change_signal` decision-table semantics need a doc-comment to clarify "false" means "no observed drift" not "no drift exists"

**Original (Task 2, Step 2):**

```rust
let change_signal = match (content_drift, version_differs, legacy_fallback) {
    (_, true, _) => Some(UpdateChangeSignal::VersionBumped),
    (true, false, _) => Some(UpdateChangeSignal::ContentChanged),
    (false, false, true) => None,
    (false, false, false) => None,
};
```

**Drift.** When `legacy_fallback = true`, content_drift is *unknown*, not *false*. The plan models it as `(content_drift: false, legacy_fallback: true)` because legacy entries skip the hash check. That's defensible, but a future reader of the code sees `content_drift = false` for a legacy-fallback plugin and thinks the hash check ran clean.

**Amended.** Add a doc-comment on `scan_plugin_for_content_drift`:

> Returns `(content_drift, legacy_fallback)` where `content_drift = false && legacy_fallback = true` means "no drift detected among hashable entries; some entries had no source_hash so a clean miss is possible." Callers should treat legacy_fallback as "drift undetectable" rather than "drift absent."

Optional secondary: rename `content_drift` to `observed_content_drift` in the helper signature and decision-table. Not load-bearing.

**Rationale.** Gate 5. The semantic reading is correct end-to-end, but the implementer-note prose is ambiguous. One line of doc-comment closes a future-maintainer-style finding.

---

## P2a-6 — Gate 3 (IMPORTANT): Task 6 underestimates Tauri test breakage; 6 assertions across 2 test functions, not 1

**Original (Task 5 Step 6 + Task 6 Step 1):**

> Task 5 step 6: "The Tauri crate will NOT compile after this commit — `commands/plugins.rs::remove_plugin` may still reference fields that no longer exist (e.g., `result.skills_removed`)."
> Task 6 step 1: "Expected: matches in `commands/plugins.rs::tests` (or similar) referencing the old field names. Update each."

**Drift.** `commands/plugins.rs` lines 512-514, 549-551 contain six concrete assertions on `counts.skills_removed` / `.steering_removed` / `.agents_removed`. The Tauri command's `remove_plugin` wrapper itself doesn't read these fields (it just passes the result through), so the wrapper compiles fine. But `commands/plugins.rs::tests` is part of the same crate, so the **whole crate's test compilation fails** after Task 5. Two test functions need updates — `remove_plugin_returns_zeros_for_nonexistent_pair` (line 500), `remove_plugin_returns_expected_counts_after_install` (line 521) — six total assertions across both. The Task 6 sketch only shows one before/after pair.

**Amended.** Update Task 6 step 1 with explicit file-path + line-number guidance:

```bash
# Expected hits (verified in worktree at HEAD 40a6fc6):
# - crates/kiro-control-center/src-tauri/src/commands/plugins.rs:512-514, 549-551
#   (two test functions:
#    - remove_plugin_returns_zeros_for_nonexistent_pair (line 500)
#    - remove_plugin_returns_expected_counts_after_install (line 521))
# - crates/kiro-control-center/src/lib/bindings.ts:954, 992-994, 1006
#   (TypeScript declarations — regenerated automatically in Task 7)
grep -rn "skills_removed\|steering_removed\|agents_removed\|RemovePluginFailure" crates/kiro-control-center/
```

In Task 6 step 2, change "update assertions" to "update **all six assertions across both test functions**", with a sketch showing the second test (`remove_plugin_returns_expected_counts_after_install`) in addition to the first.

Also add a note to Task 5 step 6: "Run `cargo test -p kiro-control-center --no-run` between Task 5 and Task 6 to confirm the expected breakage shape (compile errors at the 6 assertion sites, not at the wrapper)."

**Rationale.** Gate 3 + action linkage. Without this, an implementer running the Task 6 grep, finding the hits, and updating only the first test will hit a confusing test failure in the second one.

---

## P2a-7 — OBSERVATION: Tauri command registration snippet in Task 3 doesn't match the existing import style; `skip_serializing_if` note worth adding to Task 2

**Original (Task 3, Step 3):**

```rust
let builder = tauri_specta::Builder::<tauri::Wry>::new()
    .commands(tauri_specta::collect_commands![
```

**Drift.** The actual `lib.rs:11-40` uses `use tauri_specta::{collect_commands, Builder};` and the `Builder::<tauri::Wry>::new().commands(collect_commands![...])` shape (no `tauri_specta::` qualifier on the macro). The plan's snippet is functionally correct but doesn't match the existing import style.

A-25 from Phase 1's amendments warned about `tauri-specta` 2.0.0-rc.24 unified-mode rejecting `skip_serializing_if`. The plan correctly omits `skip_serializing_if` from new types. But the existing `InstalledSkillMeta.source_hash` carries `#[serde(default, skip_serializing_if = "Option::is_none")]` (`project.rs:44`) — that's safe because the meta types are NOT specta-derived; this is not in tension with A-25.

**Amended.** Update Task 3 Step 3 snippet to match the actual import style:

```rust
// In lib.rs, find the existing `Builder::<tauri::Wry>::new().commands(collect_commands![...])`
// invocation. Append `commands::plugins::detect_plugin_updates,` after the existing
// `commands::plugins::remove_plugin,` line (currently line 40).
```

Add a one-line note in Task 2 Step 2 implementer notes:

> The `source_hash` field on the meta types uses `#[serde(default, skip_serializing_if = "Option::is_none")]`. This is safe because the meta types are NOT specta-derived; A-25's restriction applies only to `specta::Type` structs.

**Rationale.** Observation only — implementer would catch the import style on file-read. Captured for completeness.

---

## Gates not flagged

- **Gate 2 (Threat Model)** — pass. The new detection scan reads from local cache; no new untrusted byte sources. `validate_kiro_project_path` covers the IPC path-arg gate.
- **Gate 4 (External Type Boundary)** — partial pass with an action item from P2a-1. The new `serde_json::Error` translation point introduced by loading `plugin.json` per plugin must follow CLAUDE.md's "`#[non_exhaustive]` enum + `reason: String` + `error_full_chain`" recipe. `cargo xtask plan-lint --gate gate-4-external-error-boundary` will catch any leak; flagging proactively here so the implementer chooses the variant up front rather than retrofitting.

## Summary of changes

- **P2a-1 (CRITICAL)**: `PluginEntry.version` doesn't exist; resolve via cache-load `plugin.json` → `PluginManifest.version`. **Requires user design decision (A) vs (B); recommend (A).**
- **P2a-2 (CRITICAL)**: existing field is `failed: Vec<RemovePluginFailure>`, not `failures`; explicit rename note.
- **P2a-3 (CRITICAL)**: migration sketch references non-existent helpers (`load_installed_skills_or_default`, `remove_skill_internal`); rewrite to match `self.load_installed()` + per-iteration `self.remove_skill(name)`. **Sub-decision (α) vs (β) for native_companions removed-list scope; recommend (α).**
- **P2a-4 (IMPORTANT)**: `source_hash` is asymmetric — `Option<String>` on skills/agents only, plain `String` on steering/native_companions; document the asymmetry.
- **P2a-5 (IMPORTANT)**: doc-comment on `scan_plugin_for_content_drift` clarifying `content_drift = false` semantics under legacy fallback.
- **P2a-6 (IMPORTANT)**: 6 assertions across 2 Tauri test functions, not 1; explicit file:line guidance.
- **P2a-7 (OBSERVATION)**: Tauri command registration snippet style + `skip_serializing_if` note for meta types.

**Two design decisions to lock before Task 1:** P2a-1's (A) vs (B), P2a-3's (α) vs (β). Both have recommended defaults.

No design-doc revisions required at the architecture level. The amendments are execution-time corrections; the design at `2026-04-30-phase-2a-update-detection-design.md` stands as written modulo the typed field-shape details captured here.

## References

- `docs/plan-review-checklist.md` — the 5 gates this pass applied
- `docs/plans/2026-04-30-phase-1-5-type-safety-plan-amendments.md` — Phase 1.5 amendments format precedent (especially P1.5-1 / P1.5-2 for compile-error class catches)
- `docs/plans/2026-04-29-plugin-first-install-plan-amendments.md` — Phase 1's amendments, especially A-8 (LSP-first discipline rule), A-15 (cascade keep-going policy), A-16 (marketplace-aware cleanup), A-25 (skip_serializing_if rule)
- Source SHA at review time: `40a6fc6` (post-PR-#95 main + Phase 2a design + plan)
- Worktree: `/home/dwalleck/repos/kiro-marketplace-cli-phase-2a` on branch `feat/phase-2a-update-detection`

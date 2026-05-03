# Phase 2a — Update Detection (Backend) — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `MarketplaceService::detect_plugin_updates` (hybrid hash + version detection over installed plugins) plus a Tauri command, and reshape `RemovePluginResult` from opaque counts into per-content-type sub-results (A2 — bundled here per Phase 1.5 design's deferral). Backend-only; Phase 2b (UI) ships separately.

**Architecture:** Detection walks every installed-file entry across the 4 tracking files (skills, steering, agents, native_companions), recomputes the marketplace cache hash using the same hash function the install path used (`hash::hash_dir_tree` for skill directories, `hash::hash_artifact` for steering/agent files), and compares against the tracking file's stored `source_hash`. If any hash differs OR the manifest version is non-equal to the most-recently-installed version, the plugin gets an entry in `DetectUpdatesResult.updates` with a `change_signal` discriminating version-bump vs content-drift. Per-plugin failures (cache-miss, hash failure) land in `DetectUpdatesResult.failures`. The reshape replaces `RemovePluginResult { skills_removed: u32, ... }` with a 3-sub-result struct symmetric to `InstallPluginResult`, dropping the `RemovePluginFailure.content_type: String` discriminator in favor of the parent type.

**Tech Stack:** Rust edition 2024, `thiserror`, `serde` with `transparent`/tagged enums, `specta::Type` cfg-attr, `rstest` for tests, `cargo xtask plan-lint` for the project's structural gates. Reuses `hash::hash_dir_tree` / `hash::hash_artifact` from existing Stage-1 content-hash work (no new hash module).

**Companion design doc:** `2026-04-30-phase-2a-update-detection-design.md`

---

## File structure

| File | Status | Responsibility |
|---|---|---|
| `crates/kiro-market-core/src/service/mod.rs` | Modify | Add `detect_plugin_updates`, `DetectUpdatesResult`, `PluginUpdateInfo`, `PluginUpdateFailure`, `UpdateChangeSignal` + their JSON-shape locks + behavioral tests |
| `crates/kiro-market-core/src/project.rs` | Modify | Reshape `RemovePluginResult` to `{ skills, steering, agents }`; add `RemoveSkillsResult`, `RemoveSteeringResult`, `RemoveAgentsResult`, `RemoveItemFailure`; remove obsolete `RemovePluginFailure` type; migrate `KiroProject::remove_plugin` body to populate the new sub-result types; update existing rstests asserting on the old shape |
| `crates/kiro-control-center/src-tauri/src/commands/plugins.rs` | Modify | Add `detect_plugin_updates` Tauri command with `_impl(svc, project_path)` shape; existing `remove_plugin` wrapper unchanged but returns the new shape (tests update) |
| `crates/kiro-control-center/src-tauri/src/lib.rs` | Modify | Register `detect_plugin_updates` in `tauri_specta::collect_commands!` |
| `crates/kiro-control-center/src/lib/bindings.ts` | Regenerate | Auto-generated; emits new types + restructured `RemovePluginResult` |

**Frontend changes are zero in 2a.** `bindings.ts` regen surfaces type changes to TS, but no Svelte component consumes them yet. The intermediate state is the same shape Phase 1.5 already proved is fine.

---

## Task 1: Define detection types in `service/mod.rs`

**Files:**
- Modify: `crates/kiro-market-core/src/service/mod.rs` (add types + JSON-shape lock tests)

This task is purely additive — adds the four new types and locks their wire format. Nothing in the rest of the workspace uses them yet.

- [ ] **Step 1: Add the type definitions**

In `crates/kiro-market-core/src/service/mod.rs`, near the existing `InstallPluginResult` (around line 440), add:

```rust
/// Result of [`MarketplaceService::detect_plugin_updates`] — a scan over
/// installed plugins. `updates` lists plugins with available updates;
/// `failures` lists plugins the scan couldn't check (marketplace gone
/// from cache, manifest malformed, hash computation failure). Plugins
/// with no update available are absent from both vecs (the implicit
/// "everything's fine" set).
#[derive(Clone, Debug, Default, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct DetectUpdatesResult {
    #[serde(default)]
    pub updates: Vec<PluginUpdateInfo>,
    #[serde(default)]
    pub failures: Vec<PluginUpdateFailure>,
}

/// A single plugin with an update available. `installed_version` is
/// `None` for legacy installs whose tracking file lacked the version
/// field; `available_version` is `None` when the marketplace plugin
/// manifest itself lacks a version. The `change_signal` discriminates
/// between manifest-version change and content-drift-without-version-bump.
#[derive(Clone, Debug, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct PluginUpdateInfo {
    pub marketplace: crate::validation::MarketplaceName,
    pub plugin: crate::validation::PluginName,
    pub installed_version: Option<String>,
    pub available_version: Option<String>,
    pub change_signal: UpdateChangeSignal,
}

/// A plugin the update scan couldn't check. `reason` is the rendered
/// error chain via [`crate::error::error_full_chain`] per CLAUDE.md FFI
/// rule (any wire-format `reason`/`error: String` field uses
/// `error_full_chain(&err)`, not `err.to_string()`).
#[derive(Clone, Debug, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct PluginUpdateFailure {
    pub marketplace: crate::validation::MarketplaceName,
    pub plugin: crate::validation::PluginName,
    pub reason: String,
}

/// Why an update is being surfaced. Tagged enum for FFI per the
/// `ffi-enum-serde-tag` plan-lint gate (PR #91): `#[serde(tag = "kind",
/// rename_all = "snake_case")]` produces `{ "kind": "version_bumped" }`
/// in JSON, which `tauri-specta` emits as a discriminated TS union.
#[derive(Clone, Debug, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum UpdateChangeSignal {
    /// Manifest version string differs (with or without content hash diff).
    /// FE renders "Update v1.0 → v1.1".
    VersionBumped,
    /// Manifest version unchanged but at least one source-hash diff
    /// detected. FE renders "Content updated since install".
    ContentChanged,
}
```

- [ ] **Step 2: Add JSON-shape lock tests**

In the existing `mod tests` block of `service/mod.rs`, near the `install_plugin_result_json_shape_*` tests, add:

```rust
    #[test]
    fn detect_updates_result_json_shape_default_empty() {
        let result = DetectUpdatesResult::default();
        let json = serde_json::to_value(&result).expect("serialize");
        assert_eq!(json["updates"], serde_json::json!([]));
        assert_eq!(json["failures"], serde_json::json!([]));
    }

    #[test]
    fn detect_updates_result_json_shape_with_one_update_and_one_failure() {
        use crate::service::test_support::{mp, pn};
        let result = DetectUpdatesResult {
            updates: vec![PluginUpdateInfo {
                marketplace: mp("mp1"),
                plugin: pn("p1"),
                installed_version: Some("1.0".into()),
                available_version: Some("1.1".into()),
                change_signal: UpdateChangeSignal::VersionBumped,
            }],
            failures: vec![PluginUpdateFailure {
                marketplace: mp("mp2"),
                plugin: pn("p2"),
                reason: "marketplace not in cache".into(),
            }],
        };
        let json = serde_json::to_value(&result).expect("serialize");
        assert_eq!(json["updates"][0]["marketplace"], "mp1");
        assert_eq!(json["updates"][0]["plugin"], "p1");
        assert_eq!(json["updates"][0]["installed_version"], "1.0");
        assert_eq!(json["updates"][0]["available_version"], "1.1");
        assert_eq!(json["updates"][0]["change_signal"]["kind"], "version_bumped");
        assert_eq!(json["failures"][0]["marketplace"], "mp2");
        assert_eq!(json["failures"][0]["plugin"], "p2");
        assert_eq!(json["failures"][0]["reason"], "marketplace not in cache");
    }

    #[test]
    fn plugin_update_info_json_shape_version_bumped() {
        use crate::service::test_support::{mp, pn};
        let info = PluginUpdateInfo {
            marketplace: mp("mp"),
            plugin: pn("p"),
            installed_version: Some("1.0".into()),
            available_version: Some("1.1".into()),
            change_signal: UpdateChangeSignal::VersionBumped,
        };
        let json = serde_json::to_value(&info).expect("serialize");
        assert_eq!(json["change_signal"]["kind"], "version_bumped");
    }

    #[test]
    fn plugin_update_info_json_shape_content_changed() {
        use crate::service::test_support::{mp, pn};
        let info = PluginUpdateInfo {
            marketplace: mp("mp"),
            plugin: pn("p"),
            installed_version: Some("1.0".into()),
            available_version: Some("1.0".into()),
            change_signal: UpdateChangeSignal::ContentChanged,
        };
        let json = serde_json::to_value(&info).expect("serialize");
        assert_eq!(json["change_signal"]["kind"], "content_changed");
    }
```

- [ ] **Step 3: Run tests + lint**

```bash
cargo test -p kiro-market-core --lib detect_updates_result_json_shape plugin_update_info_json_shape 2>&1 | tail -10
cargo clippy -p kiro-market-core --tests -- -D warnings 2>&1 | tail -5
cargo fmt --all
```

Expected: 4 tests pass; clippy clean.

- [ ] **Step 4: Commit**

```bash
git add crates/kiro-market-core/src/service/mod.rs
git commit -m "$(cat <<'EOF'
feat(core): add update-detection types (Phase 2a step 1/7)

Adds DetectUpdatesResult, PluginUpdateInfo, PluginUpdateFailure, and
UpdateChangeSignal in service/mod.rs. UpdateChangeSignal uses
#[serde(tag = "kind", rename_all = "snake_case")] per the
ffi-enum-serde-tag plan-lint gate (PR #91). All vec fields use
#[serde(default)] no skip_serializing_if (per A-25, tauri-specta
unified-mode rejects it).

JSON-shape lock tests pin the wire format for both UpdateChangeSignal
variants and the default-empty + populated cases of DetectUpdatesResult.

No callers yet; Task 2 implements MarketplaceService::detect_plugin_updates.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 2: Implement `MarketplaceService::detect_plugin_updates`

**Files:**
- Modify: `crates/kiro-market-core/src/service/mod.rs` (add the method + private helper + behavioral tests)

This is the meaty task. Implements the hybrid hash + version detection logic with per-plugin failure surfacing and legacy-install fallback.

### Required research (LSP-first per A-8)

Before writing, run:

```
LSP operation=documentSymbol filePath=crates/kiro-market-core/src/project.rs
LSP operation=workspaceSymbol query=hash_dir_tree
LSP operation=workspaceSymbol query=hash_artifact
LSP operation=workspaceSymbol query=marketplace_path
LSP operation=workspaceSymbol query=PluginManifest
```

Note from the symbol map:
- Exact signature of `KiroProject::installed_plugins() -> Result<InstalledPluginsView>` (returns `Vec<InstalledPluginInfo>` + `partial_load_warnings`)
- Exact signature of `crate::hash::hash_dir_tree(dir: &Path)` (returns `Result<String>`) and `hash::hash_artifact(scan_root: &Path, rel_paths: &[PathBuf])` (returns `Result<String>`)
- Exact signature of `MarketplaceService::marketplace_path(name: &str) -> PathBuf` (still `&str`-typed per Phase 1.5 design)
- The `PluginManifest` shape — specifically the `version: Option<String>` field and how to load it from a marketplace cache entry

`partial_load_warnings` from `installed_plugins()` is NOT surfaced into `DetectUpdatesResult` — those are tracking-file-load failures already surfaced by the existing `installed_plugins` Tauri command. Detection only checks plugins that loaded successfully.

### Implementation steps

- [ ] **Step 1: Write failing behavioral tests**

In `service/mod.rs::tests`, add the following tests (each test sets up a fixture marketplace + Kiro project, then asserts the detection result):

```rust
    use crate::service::test_support::{mp, pn};

    #[test]
    fn detect_plugin_updates_happy_path_no_updates() {
        // Fixture: marketplace with plugin v1.0, project with installed v1.0
        // (matching content hashes).
        let (svc, _temp) = temp_service_with_seeded_marketplace(/* see helper */);
        let dir = tempfile::tempdir().expect("tempdir");
        let project_root = make_kiro_project(dir.path());
        let project = crate::project::KiroProject::new(project_root);
        // ... install plugin at v1.0 ...
        svc.install_plugin(&project, &mp("mp"), &pn("p"), InstallMode::New, false)
            .expect("install");

        let result = svc.detect_plugin_updates(&project).expect("scan");
        assert!(result.updates.is_empty(), "no updates expected: {:?}", result);
        assert!(result.failures.is_empty(), "no failures expected: {:?}", result);
    }

    #[test]
    fn detect_plugin_updates_version_bump() {
        // Fixture: marketplace with plugin v1.0, install, then bump marketplace
        // manifest to v1.1 (same content). Expect: VersionBumped.
        // ...
        let result = svc.detect_plugin_updates(&project).expect("scan");
        assert_eq!(result.updates.len(), 1);
        assert_eq!(result.updates[0].plugin, "p");
        assert_eq!(result.updates[0].installed_version, Some("1.0".into()));
        assert_eq!(result.updates[0].available_version, Some("1.1".into()));
        assert!(matches!(
            result.updates[0].change_signal,
            UpdateChangeSignal::VersionBumped
        ));
    }

    #[test]
    fn detect_plugin_updates_content_drift_without_version_bump() {
        // Fixture: marketplace with plugin v1.0, install, then mutate a skill
        // file content in the marketplace cache (but keep version: 1.0).
        // Expect: ContentChanged.
        // ...
        let result = svc.detect_plugin_updates(&project).expect("scan");
        assert_eq!(result.updates.len(), 1);
        assert_eq!(result.updates[0].installed_version, Some("1.0".into()));
        assert_eq!(result.updates[0].available_version, Some("1.0".into()));
        assert!(matches!(
            result.updates[0].change_signal,
            UpdateChangeSignal::ContentChanged
        ));
    }

    #[test]
    fn detect_plugin_updates_per_plugin_failure_surfacing() {
        // Fixture: install plugin from marketplace mp1, then remove the
        // marketplace from the cache (or rename the dir). Expect: empty
        // updates, one failure with reason mentioning the marketplace.
        // ...
        let result = svc.detect_plugin_updates(&project).expect("scan");
        assert!(result.updates.is_empty());
        assert_eq!(result.failures.len(), 1);
        assert_eq!(result.failures[0].marketplace, "mp1");
        assert!(!result.failures[0].reason.is_empty());
    }

    #[test]
    fn detect_plugin_updates_legacy_fallback_source_hash_none() {
        // Fixture: install plugin v1.0, then directly mutate the tracking
        // file to set source_hash: None (simulating pre-Stage-1 install).
        // Bump marketplace to v1.1. Expect: VersionBumped.
        // ...
        let result = svc.detect_plugin_updates(&project).expect("scan");
        assert_eq!(result.updates.len(), 1);
        assert!(matches!(
            result.updates[0].change_signal,
            UpdateChangeSignal::VersionBumped
        ));
    }

    #[test]
    fn detect_plugin_updates_legacy_fallback_no_version_bump_returns_no_update() {
        // Fixture: install v1.0, set source_hash: None, leave marketplace at
        // v1.0. Content drift undetectable -> no entry in updates.
        let result = svc.detect_plugin_updates(&project).expect("scan");
        assert!(result.updates.is_empty(),
            "legacy install with matching version should show no update (drift undetectable)");
    }

    #[test]
    fn detect_plugin_updates_mixed_scenario() {
        // Fixture: 4 installed plugins:
        //   p1: no update (matching version + content)
        //   p2: version bumped (v1.0 -> v1.1)
        //   p3: content drift (v1.0, but skill file mutated)
        //   p4: marketplace removed from cache (failure)
        // Expect: 2 updates (p2 + p3), 1 failure (p4), p1 absent from both.
        let result = svc.detect_plugin_updates(&project).expect("scan");
        assert_eq!(result.updates.len(), 2);
        assert_eq!(result.failures.len(), 1);
        let updated_plugins: Vec<&str> = result.updates.iter().map(|u| u.plugin.as_str()).collect();
        assert!(updated_plugins.contains(&"p2"));
        assert!(updated_plugins.contains(&"p3"));
        assert_eq!(result.failures[0].plugin, "p4");
    }

    #[test]
    fn detect_plugin_updates_per_plugin_granularity() {
        // Fixture: plugin with 3 skills + 2 steering + 1 agent installed.
        // Mutate ONE steering file in the marketplace cache. Expect: ONE
        // entry in updates (per-plugin granularity, not per-file).
        let result = svc.detect_plugin_updates(&project).expect("scan");
        assert_eq!(result.updates.len(), 1);
        assert!(matches!(
            result.updates[0].change_signal,
            UpdateChangeSignal::ContentChanged
        ));
    }
```

Run them and confirm they FAIL:

```bash
cargo test -p kiro-market-core --lib detect_plugin_updates 2>&1 | tail -15
```

Expected: all 8 tests fail with "no method `detect_plugin_updates` for `MarketplaceService`".

- [ ] **Step 2: Add the method + private helper**

In `service/mod.rs`, add to the `impl MarketplaceService` block:

```rust
    /// Scan installed plugins, comparing each tracking-file `version` and
    /// `source_hash` against the corresponding marketplace plugin manifest +
    /// source files in the local cache. Reads from local cache only;
    /// callers run `update_marketplaces` first if they want fresh data.
    ///
    /// "Update available" = at least one source-hash differs from the
    /// tracking entry's `source_hash`, OR the marketplace plugin manifest's
    /// `version` is not byte-equal to the most-recently-installed version
    /// across the three tracking files. Strict string inequality on
    /// versions, no semver — downgrades pushed by marketplace owners are
    /// surfaced.
    ///
    /// Per-plugin failures (marketplace gone from cache, plugin removed
    /// from manifest, manifest malformed, hash recomputation failed) land
    /// in `failures`, not in `Result::Err`. Plugins with no update available
    /// are absent from both vecs.
    ///
    /// Legacy fallback: if any tracked file's `source_hash` is `None`
    /// (pre-Stage-1 install), drop back to version-only comparison for that
    /// plugin. Same versions in legacy mode -> no entry in updates (content
    /// drift undetectable until next install).
    ///
    /// # Errors
    ///
    /// Returns `Err` only when the toplevel `installed_plugins()` read
    /// fails (project layout broken). Per-plugin errors land in `failures`.
    pub fn detect_plugin_updates(
        &self,
        project: &crate::project::KiroProject,
    ) -> Result<DetectUpdatesResult, Error> {
        let view = project.installed_plugins()?;

        let mut updates = Vec::new();
        let mut failures = Vec::new();

        for plugin_info in view.plugins {
            match self.check_plugin_for_update(&plugin_info) {
                Ok(Some(update)) => updates.push(update),
                Ok(None) => {} // plugin is up-to-date
                Err(err) => failures.push(PluginUpdateFailure {
                    marketplace: plugin_info.marketplace.clone(),
                    plugin: plugin_info.plugin.clone(),
                    reason: crate::error::error_full_chain(&err),
                }),
            }
        }

        Ok(DetectUpdatesResult { updates, failures })
    }

    /// Check a single installed plugin against its marketplace cache entry.
    /// Returns `Ok(Some(_))` if an update is available, `Ok(None)` if up-to-date.
    /// Returns `Err` for the per-plugin failure cases (marketplace gone, etc.)
    /// — the caller maps these to `PluginUpdateFailure`.
    fn check_plugin_for_update(
        &self,
        plugin_info: &crate::project::InstalledPluginInfo,
    ) -> Result<Option<PluginUpdateInfo>, Error> {
        // Step 1: Resolve the marketplace + plugin in the cache.
        let plugin_entries = self.list_plugin_entries(plugin_info.marketplace.as_str())?;
        let plugin_entry = plugin_entries
            .iter()
            .find(|p| p.name == plugin_info.plugin.as_str())
            .ok_or_else(|| {
                Error::Plugin(crate::error::PluginError::NotFound {
                    plugin: plugin_info.plugin.as_str().to_string(),
                    marketplace: plugin_info.marketplace.as_str().to_string(),
                })
            })?;

        let available_version = plugin_entry.version.clone();

        // Step 2: Walk every installed-file entry for this plugin and
        // recompute the marketplace cache hash. First mismatch wins (no
        // need to hash remaining files for the same plugin once we know
        // there's drift).
        //
        // Implementer note: the meta types live on InstalledPluginInfo in
        // a partially-aggregated shape. To get the per-file source_hash
        // values, you need to re-load the tracking files (or extend
        // InstalledPluginInfo with the source_hash field). Recommend the
        // re-load approach to keep InstalledPluginInfo's wire format
        // stable.
        //
        // For each content type (skills, steering, agents, native_companions):
        //   - Filter the tracking file's entries to the matching plugin
        //   - For each entry with source_hash: Some(stored):
        //       - Compute the cache hash via the appropriate hash function
        //         (hash_dir_tree for skill directories, hash_artifact for
        //         steering and agent files)
        //       - If stored != cache, set content_drift = true and break
        //   - For each entry with source_hash: None, set legacy_fallback = true
        //
        // After the walk:
        //   - If content_drift: return Some(VersionBumped or ContentChanged
        //     based on version comparison)
        //   - Else if version differs: return Some(VersionBumped)
        //   - Else if legacy_fallback and version matches: return None
        //     (drift undetectable)
        //   - Else: return None (up-to-date)

        let installed_version = plugin_info.installed_version.clone();
        let (content_drift, legacy_fallback) =
            self.scan_plugin_for_content_drift(plugin_info)?;

        let version_differs = installed_version != available_version;

        let change_signal = match (content_drift, version_differs, legacy_fallback) {
            (_, true, _) => Some(UpdateChangeSignal::VersionBumped),
            (true, false, _) => Some(UpdateChangeSignal::ContentChanged),
            (false, false, true) => None, // legacy fallback, versions match -> drift undetectable
            (false, false, false) => None, // up-to-date
        };

        Ok(change_signal.map(|signal| PluginUpdateInfo {
            marketplace: plugin_info.marketplace.clone(),
            plugin: plugin_info.plugin.clone(),
            installed_version,
            available_version,
            change_signal: signal,
        }))
    }

    /// Returns `(content_drift_detected, legacy_fallback_seen)`. Walks
    /// the 4 tracking files for entries matching this plugin. First
    /// content-drift detection short-circuits subsequent file hashes
    /// (within the same plugin); legacy_fallback is sticky for the scan.
    fn scan_plugin_for_content_drift(
        &self,
        plugin_info: &crate::project::InstalledPluginInfo,
    ) -> Result<(bool, bool), Error> {
        // Implementer: this requires re-loading the tracking files to access
        // the per-entry source_hash values. See KiroProject::load_installed_*
        // helpers (private; may need pub(crate) exposure).
        //
        // Iterate skills, steering, agents, native_companions — for entries
        // with marketplace == plugin_info.marketplace AND plugin == plugin_info.plugin:
        //   - source_hash: Some(stored) -> recompute cache hash, compare
        //   - source_hash: None -> set legacy_fallback = true, skip hash check
        // Short-circuit on first hash mismatch.
        //
        // Cache path resolution:
        //   - Skills: <marketplace_path>/skills/<skill_name>/  (dir, hash_dir_tree)
        //   - Steering: <marketplace_path>/steering/<rel_path>  (file, hash_artifact)
        //   - Agents (translated): <marketplace_path>/agents/<rel_path>  (hash_artifact)
        //   - Agents (native): <marketplace_path>/agents/<rel_path>  (hash_artifact)
        //   - Native companions: <marketplace_path>/<companion_paths>  (hash_artifact)
        //
        // Use self.marketplace_path(plugin_info.marketplace.as_str()) for the base.

        todo!("scan_plugin_for_content_drift: see implementer notes above")
    }
```

**Implementer note for `scan_plugin_for_content_drift`:** the function above leaves the actual file-walking logic as `todo!()` because the exact tracking-file accessors depend on what's `pub(crate)` vs `pub` in `project.rs`. During implementation:

1. Use `LSP documentSymbol` on `crates/kiro-market-core/src/project.rs` to find the `load_installed_skills` / `load_installed_steering` / `load_installed_agents` / `load_installed_native_companions` accessors. They're likely `pub(crate)` already (the existing `installed_plugins` aggregator uses them).
2. If any are private, expose them as `pub(crate)` — these are internal helpers, not new public API.
3. Walk the entries; filter by `meta.marketplace == plugin_info.marketplace && meta.plugin == plugin_info.plugin`.
4. For each matching entry, branch on `meta.source_hash.as_deref()`:
   - `Some(stored)`: recompute cache hash via the appropriate hash function; compare; short-circuit on mismatch.
   - `None`: set `legacy_fallback = true`; continue.
5. Return `(content_drift, legacy_fallback)`.

The `todo!()` ensures the function won't ship without a real implementation — the tests will fail at the panic site, surfacing the gap.

- [ ] **Step 3: Add the test helper for fixtures**

The behavioral tests in Step 1 need a few fixture-builder helpers. Add to `service/test_support.rs` (or a new `mod test_helpers` inside `service/mod.rs::tests` if you prefer to keep them test-only):

```rust
/// Test helper: seed a marketplace with one plugin containing one skill,
/// then mutate the skill's content (or version) to simulate an update.
/// Returns the marketplace name + plugin name as newtypes.
#[cfg(any(test, feature = "test-support"))]
pub fn seed_marketplace_with_one_plugin(
    cache_root: &Path,
    marketplace: &str,
    plugin: &str,
    version: &str,
) -> (MarketplaceName, PluginName) {
    // ... write a marketplace.json + plugin.json + skill SKILL.md ...
}

/// Test helper: mutate the marketplace cache to bump a plugin's version
/// without changing any file content.
#[cfg(any(test, feature = "test-support"))]
pub fn bump_plugin_version_in_cache(
    cache_root: &Path,
    marketplace: &MarketplaceName,
    plugin: &PluginName,
    new_version: &str,
);

/// Test helper: mutate the marketplace cache by appending a byte to one
/// of the plugin's skill files. Leaves the manifest version unchanged.
#[cfg(any(test, feature = "test-support"))]
pub fn mutate_plugin_skill_content(
    cache_root: &Path,
    marketplace: &MarketplaceName,
    plugin: &PluginName,
    skill_name: &str,
);

/// Test helper: directly rewrite a tracking file to set source_hash: None
/// on one entry (simulates pre-Stage-1 install).
#[cfg(any(test, feature = "test-support"))]
pub fn null_source_hash_for_skill(
    project_root: &Path,
    marketplace: &MarketplaceName,
    plugin: &PluginName,
    skill_name: &str,
);
```

- [ ] **Step 4: Run tests + iterate**

```bash
cargo test -p kiro-market-core --lib detect_plugin_updates 2>&1 | tail -20
```

Expected: tests pass once `scan_plugin_for_content_drift` is implemented. Iterate until all 8 pass.

```bash
cargo test -p kiro-market-core --lib 2>&1 | grep "test result:" | tail -3
cargo clippy -p kiro-market-core --tests -- -D warnings
cargo fmt --all
```

- [ ] **Step 5: Commit**

```bash
git add crates/kiro-market-core/src/service/mod.rs crates/kiro-market-core/src/service/test_support.rs
git commit -m "$(cat <<'EOF'
feat(core): implement detect_plugin_updates with hybrid hash + version (Phase 2a step 2/7)

MarketplaceService::detect_plugin_updates walks every installed-file
entry across the 4 tracking files (skills, steering, agents,
native_companions), recomputes the marketplace cache hash using the
same hash function the install path used (hash_dir_tree for skill
directories, hash_artifact for steering and agent files), and
compares against the tracking file's stored source_hash.

Update-available logic:
- Any source-hash mismatch OR manifest version != installed version
  -> Some(PluginUpdateInfo)
- VersionBumped vs ContentChanged classified by version-string compare
- Legacy fallback: source_hash: None on any tracked file -> version-only
  comparison; same versions -> no entry (drift undetectable)

Per-plugin failures (marketplace gone, hash failure, plugin removed
from manifest) land in DetectUpdatesResult.failures via error_full_chain
projection — match the A-12 cascade pattern from remove_plugin.

Toplevel Result::Err reserved for "couldn't read tracking files at all"
(installed_plugins() failed). Per-plugin checks are total: a single bad
plugin doesn't knock out scan visibility for the rest.

8 behavioral rstests cover happy path, version bump, content drift,
per-plugin failure surfacing, legacy fallback (both with and without
version diff), mixed scenario, and per-plugin granularity (one mutated
file -> one update entry, not many).

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 3: Add the `detect_plugin_updates` Tauri command

**Files:**
- Modify: `crates/kiro-control-center/src-tauri/src/commands/plugins.rs`
- Modify: `crates/kiro-control-center/src-tauri/src/lib.rs` (register)

- [ ] **Step 1: Write failing tests**

In `crates/kiro-control-center/src-tauri/src/commands/plugins.rs::tests`, add:

```rust
    #[test]
    fn detect_plugin_updates_impl_happy_path() {
        let (svc, _temp) = temp_service();
        let dir = tempfile::tempdir().expect("tempdir");
        let project_path = make_kiro_project(dir.path());
        let result = detect_plugin_updates_impl(&svc, &project_path)
            .expect("scan succeeds with empty project");
        assert!(result.updates.is_empty());
        assert!(result.failures.is_empty());
    }

    #[test]
    fn detect_plugin_updates_impl_rejects_invalid_project_path() {
        let (svc, _temp) = temp_service();
        let result = detect_plugin_updates_impl(&svc, "/nonexistent/path/to/project");
        let err = result.expect_err("invalid project path must be rejected");
        assert_eq!(err.kind, ErrorType::Validation);
    }
```

Run them:

```bash
cargo test -p kiro-control-center --lib detect_plugin_updates_impl 2>&1 | tail -10
```

Expected: FAIL with "cannot find function `detect_plugin_updates_impl`".

- [ ] **Step 2: Implement the wrapper + `_impl`**

In `crates/kiro-control-center/src-tauri/src/commands/plugins.rs`, add (near the existing `install_plugin_impl`):

```rust
fn detect_plugin_updates_impl(
    svc: &MarketplaceService,
    project_path: &str,
) -> Result<DetectUpdatesResult, CommandError> {
    let project_root = validate_kiro_project_path(project_path)?;
    let project = KiroProject::new(project_root);
    svc.detect_plugin_updates(&project)
        .map_err(CommandError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn detect_plugin_updates(
    project_path: String,
) -> Result<DetectUpdatesResult, CommandError> {
    let svc = make_service()?;
    detect_plugin_updates_impl(&svc, &project_path)
}
```

Update the imports at the top of the file:

```rust
use kiro_market_core::service::{
    DetectUpdatesResult,                           // NEW
    InstallMode, InstallPluginResult, MarketplaceService,
};
```

(Adjust to match the actual import order in the file — the existing import block is alphabetical-ish.)

- [ ] **Step 3: Register in `lib.rs`**

In `crates/kiro-control-center/src-tauri/src/lib.rs`, find the `tauri_specta::collect_commands!` invocation and add `detect_plugin_updates` to the list. Example shape:

```rust
let builder = tauri_specta::Builder::<tauri::Wry>::new()
    .commands(tauri_specta::collect_commands![
        // ... existing commands ...
        commands::plugins::install_plugin,
        commands::plugins::list_installed_plugins,
        commands::plugins::remove_plugin,
        commands::plugins::detect_plugin_updates,         // NEW
        // ... rest of existing commands ...
    ])
    // ...
```

(Verify exact `collect_commands!` invocation by reading the file — Phase 1.5 didn't change registration order, so the existing pattern is the template.)

- [ ] **Step 4: Run tests + verify build**

```bash
cargo test -p kiro-control-center --lib detect_plugin_updates_impl 2>&1 | tail -5
cargo build -p kiro-control-center 2>&1 | tail -5
cargo clippy -p kiro-control-center --tests -- -D warnings
cargo fmt --all
```

Expected: 2 tests pass, Tauri crate builds clean.

- [ ] **Step 5: Commit**

```bash
git add crates/kiro-control-center/src-tauri/src/commands/plugins.rs \
        crates/kiro-control-center/src-tauri/src/lib.rs
git commit -m "$(cat <<'EOF'
feat(tauri): add detect_plugin_updates Tauri command (Phase 2a step 3/7)

Wraps MarketplaceService::detect_plugin_updates with the standard
_impl(svc, project_path) shape per CLAUDE.md (service-consuming
command). Validates project_path via validate_kiro_project_path
(existing PR #83 pattern).

No MarketplaceName::new / PluginName::new construction at the IPC
boundary — the wrapper takes no name args (it scans the whole project).
The returned DetectUpdatesResult's newtype-typed fields enforce
parse-don't-validate at the deserialization boundary.

Registered in tauri_specta::collect_commands! in lib.rs. Tests cover
happy path (empty project) and validate_kiro_project_path rejection.

bindings.ts regen happens in Task 7.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 4: Add new `RemovePluginResult` sub-result types (additive prep)

**Files:**
- Modify: `crates/kiro-market-core/src/project.rs` (add types + JSON-shape locks)

This task is purely additive — adds the four new sub-result types but does NOT yet replace the existing `RemovePluginResult`. That's Task 5. This split lets Task 4 be a small, easy-to-review commit and Task 5 the focused migration.

- [ ] **Step 1: Add the new types**

In `crates/kiro-market-core/src/project.rs`, near the existing `RemovePluginResult` (search for `pub struct RemovePluginResult`), add (BEFORE `RemovePluginResult` so the new types are in scope):

```rust
/// Per-content-type sub-result for [`RemovePluginResult`]. Mirrors
/// the install-side [`InstallSkillsResult`] shape.
#[derive(Clone, Debug, Default, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct RemoveSkillsResult {
    #[serde(default)]
    pub removed: Vec<String>, // skill names
    #[serde(default)]
    pub failures: Vec<RemoveItemFailure>,
}

/// Per-content-type sub-result for [`RemovePluginResult`]. Mirrors
/// the install-side `InstallSteeringResult` shape.
#[derive(Clone, Debug, Default, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct RemoveSteeringResult {
    #[serde(default)]
    pub removed: Vec<String>, // rendered via Path::display()
    #[serde(default)]
    pub failures: Vec<RemoveItemFailure>,
}

/// Per-content-type sub-result for [`RemovePluginResult`]. Mirrors
/// the install-side [`crate::service::InstallAgentsResult`] shape.
/// `removed` is a flat vec of translated agent names + native agent
/// names + native companion file paths (rendered) — matches the
/// install-side asymmetry where native companions are agent-side
/// artifacts.
#[derive(Clone, Debug, Default, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct RemoveAgentsResult {
    #[serde(default)]
    pub removed: Vec<String>, // agent names + native companion paths, flat
    #[serde(default)]
    pub failures: Vec<RemoveItemFailure>,
}

/// One failure during a per-content-type removal step. The discriminator
/// (which content type) is the parent type — no `content_type: String`
/// field needed (it's expressed structurally via the parent's field
/// name in [`RemovePluginResult`]).
#[derive(Clone, Debug, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct RemoveItemFailure {
    /// The skill/agent name or steering rel-path rendered via
    /// [`std::path::Path::display`].
    pub item: String,
    /// Rendered error chain via [`crate::error::error_full_chain`] —
    /// wire format per CLAUDE.md FFI rule.
    pub error: String,
}
```

- [ ] **Step 2: Add JSON-shape lock tests**

In the existing `mod tests` block of `project.rs`, add:

```rust
    #[test]
    fn remove_skills_result_json_shape_default_empty() {
        let result = RemoveSkillsResult::default();
        let json = serde_json::to_value(&result).expect("serialize");
        assert_eq!(json["removed"], serde_json::json!([]));
        assert_eq!(json["failures"], serde_json::json!([]));
    }

    #[test]
    fn remove_skills_result_json_shape_with_populated_removed() {
        let result = RemoveSkillsResult {
            removed: vec!["alpha".into(), "beta".into()],
            failures: vec![],
        };
        let json = serde_json::to_value(&result).expect("serialize");
        assert_eq!(json["removed"], serde_json::json!(["alpha", "beta"]));
        assert_eq!(json["failures"], serde_json::json!([]));
    }

    #[test]
    fn remove_skills_result_json_shape_with_populated_failure() {
        let result = RemoveSkillsResult {
            removed: vec![],
            failures: vec![RemoveItemFailure {
                item: "broken".into(),
                error: "io: permission denied".into(),
            }],
        };
        let json = serde_json::to_value(&result).expect("serialize");
        assert_eq!(json["failures"][0]["item"], "broken");
        assert_eq!(json["failures"][0]["error"], "io: permission denied");
    }
```

(Symmetric tests for `RemoveSteeringResult` and `RemoveAgentsResult` — same shape, different type names. Three tests per sub-result; 9 tests total.)

- [ ] **Step 3: Verify build**

```bash
cargo build -p kiro-market-core --tests 2>&1 | tail -5
cargo test -p kiro-market-core --lib remove_skills_result remove_steering_result remove_agents_result 2>&1 | tail -10
cargo clippy -p kiro-market-core --tests -- -D warnings
cargo fmt --all
```

Expected: clean build; 9 JSON-shape tests pass.

- [ ] **Step 4: Commit**

```bash
git add crates/kiro-market-core/src/project.rs
git commit -m "$(cat <<'EOF'
feat(core): add RemovePluginResult sub-result types (Phase 2a step 4/7)

Additive prep for A2 (RemovePluginResult reshape). Adds:
- RemoveSkillsResult { removed: Vec<String>, failures: Vec<RemoveItemFailure> }
- RemoveSteeringResult { same shape }
- RemoveAgentsResult { same shape — flat vec for translated + native + companions }
- RemoveItemFailure { item, error } — shared across all three sub-results
  because the discriminator is the parent type

The existing RemovePluginResult { skills_removed: u32, ... } is
unchanged in this commit — Task 5 restructures it to use the new
sub-results. Splitting the additive types from the cascade migration
keeps each commit small and reviewable.

JSON-shape lock tests cover default-empty + populated-removed +
populated-failure for each sub-result (9 tests total).

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 5: Restructure `RemovePluginResult` + migrate `KiroProject::remove_plugin`

**Files:**
- Modify: `crates/kiro-market-core/src/project.rs` (heavy lift — restructure `RemovePluginResult`, migrate cascade body, remove obsolete `RemovePluginFailure`, update existing rstests)

This is the heavy task for A2. Breaking wire-format change.

- [ ] **Step 1: Restructure `RemovePluginResult`**

Replace the existing `RemovePluginResult` definition with:

```rust
/// Result of [`KiroProject::remove_plugin`] — per-content-type
/// sub-results, symmetric with [`crate::service::InstallPluginResult`].
/// Native companions fold into [`RemoveAgentsResult`] (matches the
/// install-side asymmetry where native companions are agent-side
/// artifacts).
///
/// No `marketplace` / `plugin` echo fields — caller already passed
/// those args to `remove_plugin`. (Different from
/// [`crate::service::InstallPluginResult`] which gained `marketplace`
/// in Phase 1.5 A4 because that type lives in lists where
/// self-identification is needed.)
#[derive(Clone, Debug, Default, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct RemovePluginResult {
    pub skills: RemoveSkillsResult,
    pub steering: RemoveSteeringResult,
    pub agents: RemoveAgentsResult,
}
```

Delete the old definition entirely (the `skills_removed: u32` / `steering_removed: u32` / `agents_removed: u32` fields and the `failures: Vec<RemovePluginFailure>` field).

- [ ] **Step 2: Remove the obsolete `RemovePluginFailure` type**

Delete the entire `pub struct RemovePluginFailure { ... }` definition (search for `pub struct RemovePluginFailure`). The new `RemoveItemFailure` (added in Task 4) supersedes it; the parent type expresses the discriminator structurally.

If `RemovePluginFailure` is referenced anywhere else in the workspace (search via `LSP workspaceSymbol query=RemovePluginFailure`), update those references — they should all be inside `KiroProject::remove_plugin`'s body which gets migrated in Step 3.

- [ ] **Step 3: Migrate `KiroProject::remove_plugin`'s body**

Find `pub fn remove_plugin(...)`. The current body increments per-content-type counters (`skills_removed: u32 += 1`) and pushes failures into a single flat `Vec<RemovePluginFailure>`. After migration, it pushes successes into `result.skills.removed` / `result.steering.removed` / `result.agents.removed` and failures into `result.skills.failures` / `result.steering.failures` / `result.agents.failures`.

Sketch of the migration shape (the existing cascade structure stays — only the result-type accesses change):

```rust
pub fn remove_plugin(
    &self,
    marketplace: &crate::validation::MarketplaceName,
    plugin: &crate::validation::PluginName,
) -> crate::error::Result<RemovePluginResult> {
    let mut result = RemovePluginResult::default();

    // === Skills cascade ===
    let mut skills = self.load_installed_skills_or_default()?;
    let skills_to_remove: Vec<String> = skills
        .skills
        .iter()
        .filter(|(_, meta)| meta.marketplace == *marketplace && meta.plugin == *plugin)
        .map(|(name, _)| name.clone())
        .collect();
    for skill_name in skills_to_remove {
        match self.remove_skill_internal(&mut skills, &skill_name) {
            Ok(()) => result.skills.removed.push(skill_name),
            Err(err) => {
                tracing::warn!(skill = %skill_name, plugin = %plugin, marketplace = %marketplace, error = %err, "remove_plugin: skill removal failed");
                result.skills.failures.push(RemoveItemFailure {
                    item: skill_name,
                    error: crate::error::error_full_chain(&err),
                });
            }
        }
    }
    self.save_installed_skills(&skills)?;

    // === Steering cascade === (symmetric)
    // ...

    // === Agents cascade === (symmetric, including native companions folded in)
    // ...

    Ok(result)
}
```

**Native_companions cascade step:** the existing implementation calls `remove_native_companions_for_plugin` as a separate step. Per A2's design, native_companion failures land in `result.agents.failures` (same parent as agents). The successful-removal items (companion file paths rendered) go into `result.agents.removed` alongside translated and native agent names — flat vec.

The structural ordering of cascade steps (skills → steering → agents → native_companions) and the per-step error policy (keep going on failures, don't abort) are preserved — only the destination of successes/failures changes.

- [ ] **Step 4: Update existing `KiroProject::remove_plugin` rstests**

In `project.rs::tests`, find the existing rstests on `remove_plugin` (search for `fn remove_plugin_` and `fn remove_native_companions_`). They currently assert on the old shape (`result.skills_removed: u32`, `result.failures: Vec<RemovePluginFailure>` with `content_type: "skill"`). Update assertions to the new shape:

Before (example):
```rust
let result = project.remove_plugin(&mp("mp"), &pn("p")).expect("ok");
assert_eq!(result.skills_removed, 1);
assert_eq!(result.steering_removed, 0);
assert_eq!(result.agents_removed, 0);
assert!(result.failures.is_empty());
```

After:
```rust
let result = project.remove_plugin(&mp("mp"), &pn("p")).expect("ok");
assert_eq!(result.skills.removed, vec!["my-skill".to_string()]);
assert!(result.steering.removed.is_empty());
assert!(result.agents.removed.is_empty());
assert!(result.skills.failures.is_empty());
assert!(result.steering.failures.is_empty());
assert!(result.agents.failures.is_empty());
```

Tests that exercised per-step failure landing (e.g., `remove_plugin_drops_native_companions_entry_for_matching_marketplace`, `remove_plugin_continues_after_skill_failure`) need their failure assertions updated:

Before:
```rust
assert_eq!(result.failures.len(), 1);
assert_eq!(result.failures[0].content_type, "skill");
assert_eq!(result.failures[0].item, "broken-skill");
```

After:
```rust
assert_eq!(result.skills.failures.len(), 1);
assert_eq!(result.skills.failures[0].item, "broken-skill");
```

(Same shape for steering / agents — failure lands in the right sub-result by virtue of which cascade step generated it.)

The A-16 cross-marketplace tests (`remove_plugin_only_removes_matching_marketplace_plugin_pair`, `remove_native_companions_for_plugin_only_removes_matching_marketplace`) keep their existing logic — only the `removed` field assertions update.

- [ ] **Step 5: Add new JSON-shape lock for restructured `RemovePluginResult`**

```rust
    #[test]
    fn remove_plugin_result_json_shape_locks_default_empty() {
        let result = RemovePluginResult::default();
        let json = serde_json::to_value(&result).expect("serialize");
        assert!(json["skills"].is_object());
        assert!(json["steering"].is_object());
        assert!(json["agents"].is_object());
        assert_eq!(json["skills"]["removed"], serde_json::json!([]));
        assert_eq!(json["skills"]["failures"], serde_json::json!([]));
    }

    #[test]
    fn remove_plugin_result_json_shape_with_populated_removed_and_failures() {
        let result = RemovePluginResult {
            skills: RemoveSkillsResult {
                removed: vec!["alpha".into()],
                failures: vec![],
            },
            steering: RemoveSteeringResult {
                removed: vec![],
                failures: vec![RemoveItemFailure {
                    item: "broken.md".into(),
                    error: "io: permission denied".into(),
                }],
            },
            agents: RemoveAgentsResult::default(),
        };
        let json = serde_json::to_value(&result).expect("serialize");
        assert_eq!(json["skills"]["removed"][0], "alpha");
        assert_eq!(json["steering"]["failures"][0]["item"], "broken.md");
        assert_eq!(json["agents"]["removed"], serde_json::json!([]));
    }
```

- [ ] **Step 6: Build and run tests**

```bash
cargo build -p kiro-market-core 2>&1 | tail -10
cargo test -p kiro-market-core --lib remove_plugin 2>&1 | tail -20
cargo test -p kiro-market-core --lib 2>&1 | grep "test result:" | tail -3
cargo clippy -p kiro-market-core --tests -- -D warnings
cargo fmt --all
```

Expected: clean build; all `remove_plugin_*` rstests pass under the new assertion shape; full kiro-market-core suite green.

The Tauri crate will NOT compile after this commit — `commands/plugins.rs::remove_plugin` may still reference fields that no longer exist (e.g., `result.skills_removed`). Task 6 fixes the Tauri crate.

- [ ] **Step 7: Commit**

```bash
git add crates/kiro-market-core/src/project.rs
git commit -m "$(cat <<'EOF'
refactor(core): A2 — restructure RemovePluginResult into sub-results (Phase 2a step 5/7)

RemovePluginResult { skills_removed: u32, steering_removed: u32,
agents_removed: u32, failures: Vec<RemovePluginFailure> } becomes
RemovePluginResult { skills: RemoveSkillsResult, steering:
RemoveSteeringResult, agents: RemoveAgentsResult } — symmetric with
InstallPluginResult.

Each sub-result carries `removed: Vec<String>` (item names; for
steering, paths rendered via Path::display(); for agents, flat vec
including translated agent names, native agent names, and native
companion paths) and `failures: Vec<RemoveItemFailure>`. The shared
RemoveItemFailure type replaces RemovePluginFailure — the
content_type: String discriminator goes away because the parent type
expresses it structurally.

KiroProject::remove_plugin's body migration:
- Per-content-type cascade structure unchanged (skills -> steering ->
  agents -> native_companions, keep going on per-step failures)
- Successes append to result.<content_type>.removed
- Failures append to result.<content_type>.failures via error_full_chain
- Native_companions failures land in result.agents.failures (same
  parent as agents, matching install-side asymmetry)

A-16 marketplace-aware cleanup preserved at all 4 cascade filters
(meta.marketplace == *marketplace && meta.plugin == *plugin).

Existing rstests on remove_plugin migrated to the new assertion shape
(result.skills.removed instead of result.skills_removed, etc.). New
JSON-shape lock test pins the restructured wire format.

The Tauri crate doesn't compile after this commit — Task 6 fixes
commands/plugins.rs.

Breaking wire-format change. No compat shim — single-PR-per-phase
pattern means the consumer (Phase 2b UI) updates at the same time as
the producer (this PR's bindings.ts regen).

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 6: Update `commands/plugins.rs::remove_plugin` to the new shape

**Files:**
- Modify: `crates/kiro-control-center/src-tauri/src/commands/plugins.rs` (Tauri wrapper unchanged but tests update; verify no leftover references to old shape)

The Tauri `remove_plugin` wrapper signature is unchanged (it still takes `String` args and returns `Result<RemovePluginResult, CommandError>`). The wire-format change ripples through specta's bindings.ts regeneration (Task 7); the Tauri crate's Rust code only needs updates where tests assert on the old shape.

- [ ] **Step 1: Find references to old `RemovePluginResult` shape**

```bash
cd /home/dwalleck/repos/kiro-marketplace-cli
grep -rn "skills_removed\|steering_removed\|agents_removed\|RemovePluginFailure" crates/kiro-control-center/
```

Expected: matches in `commands/plugins.rs::tests` (or similar) referencing the old field names. Update each.

- [ ] **Step 2: Update existing `remove_plugin` tests**

In `commands/plugins.rs::tests` (search for `fn remove_plugin_`), update assertions:

Before:
```rust
assert_eq!(result.skills_removed, 2);
```

After:
```rust
assert_eq!(result.skills.removed.len(), 2);
// or stronger: assert_eq!(result.skills.removed, vec!["alpha", "beta"]);
```

If any test asserted on `RemovePluginFailure.content_type`, update to read from the appropriate sub-result's `failures` vec.

- [ ] **Step 3: Verify the workspace compiles**

```bash
cargo build --workspace 2>&1 | tail -10
cargo test -p kiro-control-center --lib remove_plugin 2>&1 | tail -10
cargo clippy --workspace --tests -- -D warnings
cargo fmt --all
```

Expected: clean workspace build; Tauri remove_plugin tests pass.

- [ ] **Step 4: Commit**

```bash
git add crates/kiro-control-center/src-tauri/src/commands/plugins.rs
git commit -m "$(cat <<'EOF'
refactor(tauri): consume new RemovePluginResult sub-result shape (Phase 2a step 6/7)

Tauri remove_plugin wrapper signature is unchanged — it still takes
String args and returns Result<RemovePluginResult, CommandError>. The
wire-format change ripples through specta's bindings.ts regen (Task 7).

Test assertions migrated:
- result.skills_removed: u32 -> result.skills.removed: Vec<String>
- result.failures[i].content_type discriminator -> appropriate
  sub-result's failures vec

A2 reshape's frontend ripple lands in Phase 2b (the UI PR consuming
the new bindings.ts).

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 7: Regenerate `bindings.ts`, run all pre-commit gates, open PR

**Files:**
- Regenerate: `crates/kiro-control-center/src/lib/bindings.ts`

- [ ] **Step 1: Regenerate `bindings.ts`**

```bash
cd /home/dwalleck/repos/kiro-marketplace-cli  # or the worktree
cargo test -p kiro-control-center --lib generate_types -- --ignored 2>&1 | tail -5
```

Expected: pass. Verify the new types appear:

```bash
grep -E "^export type (DetectUpdatesResult|PluginUpdateInfo|PluginUpdateFailure|UpdateChangeSignal|RemoveSkillsResult|RemoveSteeringResult|RemoveAgentsResult|RemoveItemFailure)\b" crates/kiro-control-center/src/lib/bindings.ts
```

All 8 should appear. Verify shapes:
- `DetectUpdatesResult` — `{ updates: PluginUpdateInfo[], failures: PluginUpdateFailure[] }`
- `UpdateChangeSignal` — discriminated union: `{ kind: "version_bumped" } | { kind: "content_changed" }`
- `RemovePluginResult` — `{ skills: RemoveSkillsResult, steering: RemoveSteeringResult, agents: RemoveAgentsResult }`

The frontend code (`BrowseTab.svelte`, `InstalledTab.svelte`) doesn't need changes in Phase 2a — TS treats the type changes as a wire-format diff that no Svelte component currently consumes (the existing FE doesn't show update indicators or read individual removed item names). Phase 2b consumes them.

- [ ] **Step 2: Run all pre-commit gates**

```bash
cargo fmt --all --check
cargo clippy --workspace --tests -- -D warnings
cargo test --workspace 2>&1 | grep "test result:" | tail -10
cd crates/kiro-control-center && npm run check 2>&1 | tail -5 && cd ../..
TETHYS_BIN=/home/dwalleck/repos/rivets/target/release/tethys cargo xtask plan-lint 2>&1 | tail -10
```

All MUST pass. If `npm run check` reports new errors related to the migrated bindings, investigate — most likely the FE has a typed reference to `result.skills_removed` somewhere that needs updating to `result.skills.removed.length`. The expected zero-FE-change state holds only if the FE doesn't currently consume `RemovePluginResult` count fields. If it does, either:
- Update the FE call site as part of this PR (cleanest)
- Capture as a Phase 2b finding in the amendments doc and ship a stub

If `cargo xtask plan-lint` reports any findings, document them in `2026-05-01-phase-2a-update-detection-plan-amendments.md` (creating it if needed) per the precedent set by Phase 1 + 1.5.

- [ ] **Step 3: Commit `bindings.ts`**

```bash
git add crates/kiro-control-center/src/lib/bindings.ts
git commit -m "$(cat <<'EOF'
chore(bindings): regenerate TS bindings for Phase 2a (step 7/7)

Emits the new detection types (DetectUpdatesResult, PluginUpdateInfo,
PluginUpdateFailure, UpdateChangeSignal as discriminated union) and
the restructured RemovePluginResult { skills, steering, agents }
shape. Frontend code unchanged in Phase 2a — Phase 2b consumes these
types.

All pre-commit gates pass:
- cargo fmt --all --check: clean
- cargo clippy --workspace --tests -- -D warnings: clean
- cargo test --workspace: all green
- npm run check (frontend): clean
- cargo xtask plan-lint (all gates): OK

Phase 2a complete. The detection backend and A2 reshape ship as a
backend-only PR; Phase 2b adds the Update indicator UI on plugin
cards and the refreshed Remove toast consuming these types.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

- [ ] **Step 4: Push the branch**

```bash
git push -u origin feat/phase-2a-update-detection
```

(If you're not on a worktree branch yet, the implementer should create one before starting Task 1. See `superpowers:using-git-worktrees`.)

- [ ] **Step 5: Open PR**

```bash
gh pr create --title "feat: update detection + RemovePluginResult reshape (Phase 2a backend)" --body "$(cat <<'EOF'
## Summary

Phase 2a of the plugin lifecycle. Adds `MarketplaceService::detect_plugin_updates` (hybrid hash + version detection over installed plugins) plus a Tauri command, and reshapes \`RemovePluginResult\` from opaque counts into per-content-type sub-results (A2 — bundled here per Phase 1.5 design's deferral). Backend-only; Phase 2b (UI) ships separately.

Background: Phase 1 design's Phase 2 sketch (\`2026-04-29-plugin-first-install-design.md\`) deferred the implementation plan until Phase 1 shipped. Phase 1.5 (PR #95) closed the swap-arg footgun so Phase 2's APIs use \`MarketplaceName\` / \`PluginName\` newtypes from day one.

## What's in scope

- **Detection: hybrid hash + version.** Hash drives "update available?"; version provides the human-readable label. Catches author-hygiene gaps where content drifts without a manifest version bump (markdown-heavy plugins).
- **Per-plugin failure surfacing.** \`DetectUpdatesResult { updates, failures }\` matches the A-12 cascade pattern. A single unreachable marketplace doesn't knock out scan visibility for the rest.
- **\`UpdateChangeSignal\`** discriminated enum (\`#[serde(tag = "kind", rename_all = "snake_case")]\`) — complies with the \`ffi-enum-serde-tag\` plan-lint gate (PR #91).
- **Legacy fallback** for pre-Stage-1 installs (\`source_hash: None\`) — drop back to version-only comparison; same versions = no entry (drift undetectable until next install).
- **A2: \`RemovePluginResult\` reshape** — symmetric with \`InstallPluginResult\`'s sub-results; \`RemoveItemFailure\` shared across the three sub-results (parent type expresses the discriminator).

## What's NOT in scope

- Phase 2b UI work (Update indicator, Update button wiring, refreshed Remove toast)
- Hash memoization in marketplace cache (perf optimization)
- Per-content-type update granularity (rejected per design)
- Auto-update / background polling (rejected per design)
- HashMap-key newtypes, \`From<CoreError>\` exhaustiveness fix, cross-marketplace idempotency edge — separate phases per Phase 2a design's "Out of scope"

## Test plan

- [x] \`cargo fmt --all --check\` clean
- [x] \`cargo clippy --workspace --tests -- -D warnings\` clean
- [x] \`cargo test --workspace\` all green (8 new behavioral detection tests + 4 detection JSON-shape locks + 9 sub-result JSON-shape locks + restructured \`remove_plugin\` rstests)
- [x] \`cargo xtask plan-lint\` all gates OK (including \`ffi-enum-serde-tag\` for the new \`UpdateChangeSignal\`)
- [x] \`npm run check\` clean (frontend types via regenerated \`bindings.ts\`)

## References

- Design: \`docs/plans/2026-04-30-phase-2a-update-detection-design.md\`
- Plan: \`docs/plans/2026-05-01-phase-2a-update-detection-plan.md\`
- Predecessor: PR #95 (Phase 1.5, MarketplaceName/PluginName newtypes)
- Phase 1 design's Phase 2 sketch: \`2026-04-29-plugin-first-install-design.md\` (lines 185-233)

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

---

## Self-review

### Spec coverage

| Design section | Covered by |
|---|---|
| `DetectUpdatesResult`, `PluginUpdateInfo`, `PluginUpdateFailure`, `UpdateChangeSignal` types | Task 1 |
| Detection logic (hybrid hash + version) | Task 2 |
| Legacy fallback | Task 2 (tests + impl) |
| Per-plugin failure surfacing | Task 2 (tests + impl) |
| Tauri `detect_plugin_updates` command | Task 3 |
| New sub-result types (A2 prep) | Task 4 |
| `RemovePluginResult` restructure + `KiroProject::remove_plugin` cascade migration | Task 5 |
| `RemovePluginFailure` removal | Task 5 |
| Tauri `remove_plugin` consumer update | Task 6 |
| `bindings.ts` regen | Task 7 |
| Pre-commit gates + PR open | Task 7 |
| 5-gates self-review | Task 7 (during pre-commit) |

All design requirements have a task.

### Placeholder scan

- No "TBD" / "TODO" / "implement later" entries.
- One `todo!()` macro in Task 2's `scan_plugin_for_content_drift` skeleton — intentional, with a detailed implementer note explaining what to fill in (the implementer is expected to use LSP to finalize the tracking-file accessor signatures).
- All commit messages have actual content (no "fill in details").

### Type consistency

- `DetectUpdatesResult { updates, failures }` consistent across Task 1 (definition), Task 2 (impl), Task 3 (Tauri tests), Task 7 (bindings.ts verification).
- `UpdateChangeSignal::VersionBumped` / `ContentChanged` consistent across Task 1 (enum definition), Task 2 (test assertions), Task 7 (bindings.ts shape verification).
- `RemovePluginResult { skills, steering, agents }` consistent across Task 4 (sub-result types), Task 5 (parent restructure + cascade migration), Task 6 (Tauri consumer update), Task 7 (bindings.ts).
- `RemoveItemFailure { item, error }` consistent across all sub-results.
- The compile-state warnings ("Tauri crate doesn't compile after Task 5 — Task 6 fixes") align with the actual ripple — Task 5 changes the core type, Task 6 updates the Tauri consumer.

### Bite-sized granularity

Each task has 4-7 numbered steps; each step is a single action (write tests / run / implement / lint / commit). Commit cadence per task ≈ 1 commit; total ≈ 7 commits.

---

**Plan complete.** Suggested execution: subagent-driven, one task per fresh subagent, two-stage review between tasks per the `superpowers:subagent-driven-development` skill. Tasks 2 and 5 are the heavy lifts; Tasks 1, 3, 4, 6, 7 are smaller focused commits. Apply the 5-gates checklist to this plan before kicking off Task 1 — capture any drift in `2026-05-01-phase-2a-update-detection-plan-amendments.md` per the Phase 1 + 1.5 precedent.

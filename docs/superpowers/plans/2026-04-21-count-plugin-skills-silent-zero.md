# count_plugin_skills Silent-Zero Fix — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the `count_plugin_skills` silent-zero antipattern (which collapses remote plugins and manifest-load failures into the same `0` a legitimately empty plugin reports) with a discriminated `SkillCount` enum carried through the service, Tauri FFI, and Svelte UI.

**Architecture:** Add a new `MarketplaceService::count_skills_for_plugin` in `kiro-market-core` returning a three-way `SkillCount` union (`Known { count }` / `RemoteNotCounted` / `ManifestFailed { reason }`). The directory pre-check the spec proposed as a new `check_plugin_dir` helper is satisfied by reusing the existing `MarketplaceService::resolve_local_plugin_dir` — it already performs the identical five-branch stat (symlink / non-dir / OK / NotFound / other-I/O) and is test-pinned. Tauri's `PluginInfo.skill_count` field type changes from `u32` to `SkillCount`, specta regenerates `bindings.ts`, and `BrowseTab.svelte` renders the three states with minimal labels (number / `"–"` / `"!"` + tooltip).

**Tech Stack:** Rust 2024 (edition), `serde`, `specta` (feature-gated), `tempfile` (test fixtures), SvelteKit 5, TypeScript, Tauri IPC via specta-generated bindings.

**Spec reference:** `docs/superpowers/specs/2026-04-21-count-plugin-skills-silent-zero-design.md` (committed as `ffd8300`).

**Branch:** `fix/count-plugin-skills-silent-zero`.

**Out of scope (per spec):** Deleting Tauri-local `load_plugin_manifest` / `discover_skills_for_plugin` — both still called by `install_skills` in the Tauri crate; their removal is #33's job.

**Spec → plan refinement (note to reviewers):** The spec proposed a new `check_plugin_dir` helper. During plan writing I discovered `MarketplaceService::resolve_local_plugin_dir` at `crates/kiro-market-core/src/service/browse.rs:338` already performs an identical five-branch directory stat (`SymlinkRefused` / `NotADirectory` / `Ok` / `DirectoryMissing` / `DirectoryUnreadable`) with full test coverage. Reusing it satisfies the spec's intent (plugin-directory hardening that prevents silent-zero for missing plugin_dir) with zero new helper code and zero new duplication. The spec's alignment note explicitly flagged extraction as the preferred outcome. No behavior change vs. the spec; only the implementation path differs.

---

## File structure

Files touched by this plan, with one-line responsibility:

| File | Responsibility |
|---|---|
| `crates/kiro-market-core/src/service/browse.rs` | Add `SkillCount` enum (response types section) and `MarketplaceService::count_skills_for_plugin` method. Add behavior + wire-format tests inside existing `#[cfg(test)] mod tests` block. |
| `crates/kiro-control-center/src-tauri/src/commands/browse.rs` | Change `PluginInfo.skill_count` field type `u32` → `SkillCount`. Replace call site in `list_plugins` with `svc.count_skills_for_plugin(...)`. Delete local `count_plugin_skills`. Add Layer 3 integration test. |
| `crates/kiro-control-center/src-tauri/src/lib.rs` (or wherever specta re-exports are declared) | Re-export `SkillCount` so specta picks it up for `bindings.ts`. |
| `crates/kiro-control-center/src/lib/bindings.ts` | **Auto-regenerated** by `cargo test -p kiro-control-center --lib -- --ignored`. Do not edit by hand. |
| `crates/kiro-control-center/src/lib/components/BrowseTab.svelte` | Add `formatSkippedReason`, `skillCountLabel`, `skillCountTitle` helpers at top of `<script>`. Replace the `{ap.plugin.skill_count}` rendering at line 704 with the new labeled span. |

No new files created. Tauri-local `count_plugin_skills` is deleted; the Tauri-local `load_plugin_manifest` / `discover_skills_for_plugin` stay (used by `install_skills`, future #33 concern).

---

## Preflight

Before starting Task 1, verify the branch and starting commit match the plan.

- [ ] **Step 0.1: Verify branch**

Run: `git branch --show-current`
Expected output: `fix/count-plugin-skills-silent-zero`

- [ ] **Step 0.2: Verify starting commit**

Run: `git log -1 --oneline`
Expected output starts with: `ffd8300 docs(spec): design for #32`

- [ ] **Step 0.3: Verify clean working tree (tracked files)**

Run: `git status --porcelain | grep -v '^??' || true`
Expected output: empty (no modified or staged tracked files).

---

## Task 1: Add `SkillCount` enum and serde wire-format pin

**Files:**
- Modify: `crates/kiro-market-core/src/service/browse.rs` — add enum at line 308 (between `PluginSkillsResult` and the `// Service methods` divider), add test at the end of the `#[cfg(test)] mod tests` block (around line 1915, before the closing `}` of the module).

- [ ] **Step 1.1: Write the failing wire-format pin test**

Open `crates/kiro-market-core/src/service/browse.rs`. Find the end of the `#[cfg(test)] mod tests` block (closing `}` at line 1916). Insert the following test immediately before that closing `}`, indented one level:

```rust
    // -----------------------------------------------------------------------
    // SkillCount wire-format pins
    // -----------------------------------------------------------------------

    #[test]
    fn skill_count_serde_known_wire_format() {
        let json = serde_json::to_value(SkillCount::Known { count: 7 }).unwrap();
        assert_eq!(json, serde_json::json!({"state": "known", "count": 7}));
    }

    #[test]
    fn skill_count_serde_remote_not_counted_wire_format() {
        let json = serde_json::to_value(SkillCount::RemoteNotCounted).unwrap();
        assert_eq!(json, serde_json::json!({"state": "remote_not_counted"}));
    }

    #[test]
    fn skill_count_serde_manifest_failed_wire_format() {
        let sc = SkillCount::ManifestFailed {
            reason: SkippedReason::InvalidManifest {
                path: std::path::PathBuf::from("/tmp/plug/plugin.json"),
                reason: "expected `}`".into(),
            },
        };
        let json = serde_json::to_value(sc).unwrap();
        assert_eq!(json["state"], "manifest_failed");
        assert_eq!(json["reason"]["kind"], "invalid_manifest");
        assert_eq!(json["reason"]["path"], "/tmp/plug/plugin.json");
        assert_eq!(json["reason"]["reason"], "expected `}`");
    }
```

- [ ] **Step 1.2: Run the tests to verify they fail**

Run: `cargo test -p kiro-market-core --lib service::browse::tests::skill_count 2>&1 | tail -20`
Expected: compilation error (`cannot find type SkillCount in this scope` or similar).

- [ ] **Step 1.3: Add the `SkillCount` enum**

Open `crates/kiro-market-core/src/service/browse.rs`. Insert the following immediately after the closing `}` of `PluginSkillsResult` (line 307) and before the `// ---------------------------------------------------------------------------` divider that precedes `// Service methods`:

```rust
/// Result of [`MarketplaceService::count_skills_for_plugin`].
/// Distinguishes the three cases the frontend must render differently:
/// a known count, a remote plugin (not locally countable), and a local
/// plugin whose directory or manifest could not be loaded. Replaces the
/// prior `usize` that collapsed failures into a silent `0`.
#[derive(Clone, Debug, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(tag = "state", rename_all = "snake_case")]
#[non_exhaustive]
pub enum SkillCount {
    /// The plugin directory was readable; `count` is the number of
    /// discovered skill directories (including the legitimate zero case).
    Known { count: u32 },

    /// Plugin source is remote (GitHub / git URL). Skills cannot be
    /// enumerated without cloning, which the listing path never does.
    /// Distinct from `ManifestFailed { reason: RemoteSourceNotLocal }`:
    /// here we know the plugin is remote by construction and never
    /// attempt the local resolution.
    RemoteNotCounted,

    /// The plugin is local but something about its directory or
    /// `plugin.json` prevented a skill count.
    ///
    /// `SkippedReason` is reused as the error payload to share the #30
    /// projection [`SkippedReason::from_plugin_error`]. Reachable from
    /// this path:
    ///
    /// From the `MarketplaceService::resolve_local_plugin_dir` pre-check:
    /// - [`SkippedReason::DirectoryMissing`] — `plugin_dir` not found.
    /// - [`SkippedReason::NotADirectory`] — `plugin_dir` is a file.
    /// - [`SkippedReason::SymlinkRefused`] — `plugin_dir` is a symlink.
    /// - [`SkippedReason::DirectoryUnreadable`] — stat failed for any
    ///   other reason (permission denied, transient I/O, etc.).
    ///
    /// From [`load_plugin_manifest`]:
    /// - [`SkippedReason::InvalidManifest`] — `plugin.json` malformed.
    /// - [`SkippedReason::ManifestReadFailed`] — `plugin.json` read
    ///   failed after a successful stat.
    ///
    /// [`SkippedReason::NoSkills`] is not produced anywhere in this
    /// path; [`SkippedReason::RemoteSourceNotLocal`] is pre-empted by
    /// [`Self::RemoteNotCounted`] before resolution is attempted.
    /// Frontends typed against `SkippedReason` will not get
    /// compile-time narrowing for those two — accepted because
    /// consolidating the projection is more valuable than a narrower
    /// wire type.
    ManifestFailed { reason: SkippedReason },
}
```

- [ ] **Step 1.4: Run the tests to verify they pass**

Run: `cargo test -p kiro-market-core --lib service::browse::tests::skill_count 2>&1 | tail -20`
Expected: `test result: ok. 3 passed`.

- [ ] **Step 1.5: Run clippy to catch lints early**

Run: `cargo clippy -p kiro-market-core --tests -- -D warnings 2>&1 | tail -10`
Expected: no warnings or errors.

- [ ] **Step 1.6: Commit**

```bash
git add crates/kiro-market-core/src/service/browse.rs
git commit -m "$(cat <<'EOF'
feat(core): add SkillCount enum

Three-way discriminated union for plugin skill-count reporting:
Known { count }, RemoteNotCounted, ManifestFailed { reason }. Reuses
the existing SkippedReason type (via #[cfg_attr(feature = "specta", derive(specta::Type))])
so the #30 projection `SkippedReason::from_plugin_error` stays the
single source of truth.

Part of #32.
EOF
)"
```

---

## Task 2: Add `count_skills_for_plugin` service method + Layer 1 behavior tests

**Files:**
- Modify: `crates/kiro-market-core/src/service/browse.rs` — add method inside `impl MarketplaceService` block (insert before line 514, which is the closing `}` of the impl; after `list_all_skills`). Add tests at the end of the `#[cfg(test)] mod tests` block (after the Task 1 wire-format tests).

- [ ] **Step 2.1: Write a single failing happy-path test**

Open `crates/kiro-market-core/src/service/browse.rs`. At the end of the `#[cfg(test)] mod tests` block (after the Task 1 wire-format tests, before the module closing `}`), insert:

```rust
    // -----------------------------------------------------------------------
    // count_skills_for_plugin
    // -----------------------------------------------------------------------

    #[test]
    fn count_skills_for_plugin_returns_known_for_local_plugin() {
        let (dir, svc) = temp_service();
        let marketplace_path = dir.path().join("marketplace");
        make_plugin_with_skills(&marketplace_path, "my-plugin", &["alpha", "beta", "gamma"]);

        let entry = relative_path_entry("my-plugin", "plugins/my-plugin");
        let result = svc.count_skills_for_plugin(&entry, &marketplace_path);
        assert!(
            matches!(result, SkillCount::Known { count: 3 }),
            "expected Known {{ count: 3 }}, got: {result:?}"
        );
    }
```

- [ ] **Step 2.2: Run the test to verify it fails**

Run: `cargo test -p kiro-market-core --lib service::browse::tests::count_skills_for_plugin_returns_known_for_local_plugin 2>&1 | tail -10`
Expected: compile error (`no method named count_skills_for_plugin`).

- [ ] **Step 2.3: Implement `count_skills_for_plugin`**

Open `crates/kiro-market-core/src/service/browse.rs`. Find the closing `}` of the `impl MarketplaceService` block at line 514. Insert the following **before** that closing `}`, after the `list_all_skills` method:

```rust
    /// Count skills for a single plugin entry without loading skill bodies.
    ///
    /// Returns [`SkillCount::RemoteNotCounted`] for remote sources,
    /// [`SkillCount::ManifestFailed`] if the plugin directory or its
    /// `plugin.json` cannot be read or parsed, and [`SkillCount::Known`]
    /// otherwise (including the legitimate zero case where the manifest
    /// is absent or declares no skills).
    ///
    /// Takes the pre-resolved [`PluginEntry`] and `marketplace_path` so
    /// the batch caller in `list_plugins` pays the registry-parse cost
    /// once per marketplace rather than once per plugin. Errors are
    /// never propagated as `Err` — every outcome fits the three-way
    /// union.
    ///
    /// The plugin-directory pre-check delegates to
    /// [`Self::resolve_local_plugin_dir`] so the hardening (symlink
    /// refusal, is_dir check, NotFound / other-I/O classification) stays
    /// consistent with the bulk-listing path and does not duplicate.
    #[must_use]
    pub fn count_skills_for_plugin(
        &self,
        plugin: &PluginEntry,
        marketplace_path: &Path,
    ) -> SkillCount {
        // Short-circuit remote sources before `resolve_local_plugin_dir`
        // is called — it would return `PluginError::RemoteSourceNotLocal`
        // which we would then translate to `ManifestFailed`, conflating
        // "remote by design" with "should have been local but resolved
        // remote." The two need distinct UI states.
        if matches!(plugin.source, PluginSource::Structured(_)) {
            return SkillCount::RemoteNotCounted;
        }

        let plugin_dir = match self.resolve_local_plugin_dir(plugin, marketplace_path) {
            Ok(p) => p,
            Err(err) => {
                return SkillCount::ManifestFailed {
                    reason: skipped_reason_from_resolve_error(&plugin.name, err),
                };
            }
        };

        match load_plugin_manifest(&plugin_dir) {
            Ok(manifest) => {
                let count = discover_skills_for_plugin(&plugin_dir, manifest.as_ref()).len();
                SkillCount::Known {
                    count: u32::try_from(count).unwrap_or(u32::MAX),
                }
            }
            Err(err) => SkillCount::ManifestFailed {
                reason: skipped_reason_from_manifest_error(&plugin.name, &plugin_dir, err),
            },
        }
    }
```

Next, add two small private helpers to centralize the `Error` → `SkippedReason` projection (kept out of the method body so the defensive-fallback logging paths don't clutter the reading path). Add them in the "Private helpers" section, immediately after `discover_skills_for_plugin` (which ends at line 637):

```rust
/// Project a `resolve_local_plugin_dir` error into a [`SkippedReason`].
///
/// `resolve_local_plugin_dir` only returns [`PluginError`] variants
/// that [`SkippedReason::from_plugin_error`] classifies as
/// plugin-level skips (`DirectoryMissing`, `NotADirectory`,
/// `SymlinkRefused`, `DirectoryUnreadable`, plus
/// `RemoteSourceNotLocal` — pre-empted at the caller). The defensive
/// `unwrap_or_else` branch exists for forward-compatibility: if a
/// future `PluginError` variant lands and the classifier chooses to
/// propagate rather than skip, we fold it into `DirectoryUnreadable`
/// with a `warn!` rather than regress to a silent `0`.
fn skipped_reason_from_resolve_error(plugin_name: &str, err: Error) -> SkippedReason {
    let Error::Plugin(pe) = err else {
        // `resolve_local_plugin_dir` only returns `Error::Plugin` today,
        // but `Error` is `#[non_exhaustive]` — defensive.
        warn!(
            plugin = %plugin_name,
            error = %error_full_chain(&err),
            "unexpected non-plugin error resolving plugin_dir; reporting as DirectoryUnreadable"
        );
        return SkippedReason::DirectoryUnreadable {
            path: PathBuf::new(),
            reason: error_full_chain(&err),
        };
    };
    SkippedReason::from_plugin_error(&pe).unwrap_or_else(|| {
        warn!(
            plugin = %plugin_name,
            error = ?pe,
            "unclassified PluginError from resolve_local_plugin_dir; reporting as DirectoryUnreadable"
        );
        SkippedReason::DirectoryUnreadable {
            path: PathBuf::new(),
            reason: pe.to_string(),
        }
    })
}

/// Project a `load_plugin_manifest` error into a [`SkippedReason`].
///
/// `load_plugin_manifest` returns [`PluginError::InvalidManifest`] or
/// [`PluginError::ManifestReadFailed`] today. Same defensive pattern
/// as [`skipped_reason_from_resolve_error`]: an unclassified variant
/// folds into `ManifestReadFailed` with a `warn!`.
fn skipped_reason_from_manifest_error(
    plugin_name: &str,
    plugin_dir: &Path,
    err: Error,
) -> SkippedReason {
    let Error::Plugin(pe) = err else {
        warn!(
            plugin = %plugin_name,
            error = %error_full_chain(&err),
            "unexpected non-plugin error loading plugin.json; reporting as ManifestReadFailed"
        );
        return SkippedReason::ManifestReadFailed {
            path: plugin_dir.join("plugin.json"),
            reason: error_full_chain(&err),
        };
    };
    SkippedReason::from_plugin_error(&pe).unwrap_or_else(|| {
        warn!(
            plugin = %plugin_name,
            error = ?pe,
            "unclassified PluginError from load_plugin_manifest; reporting as ManifestReadFailed"
        );
        SkippedReason::ManifestReadFailed {
            path: plugin_dir.join("plugin.json"),
            reason: pe.to_string(),
        }
    })
}
```

- [ ] **Step 2.4: Run the first test to verify it passes**

Run: `cargo test -p kiro-market-core --lib service::browse::tests::count_skills_for_plugin_returns_known_for_local_plugin 2>&1 | tail -10`
Expected: `test result: ok. 1 passed`.

- [ ] **Step 2.5: Add the remaining Layer 1 tests**

Back in the `#[cfg(test)] mod tests` block, under the `// count_skills_for_plugin` heading you added in Step 2.1, append the following tests (in the same module, after the existing happy-path test):

```rust
    #[test]
    fn count_skills_for_plugin_returns_known_with_zero_when_no_skills() {
        let (dir, svc) = temp_service();
        let marketplace_path = dir.path().join("marketplace");
        let plugin_dir = marketplace_path.join("plugins/lonely");
        fs::create_dir_all(&plugin_dir).expect("create plugin dir");
        // A plugin.json with no custom skill paths → default paths apply,
        // but no skills/ directory exists, so count is 0.
        fs::write(
            plugin_dir.join("plugin.json"),
            r#"{"name": "lonely", "version": "0.0.0"}"#,
        )
        .expect("write plugin.json");

        let entry = relative_path_entry("lonely", "plugins/lonely");
        let result = svc.count_skills_for_plugin(&entry, &marketplace_path);
        assert!(
            matches!(result, SkillCount::Known { count: 0 }),
            "expected Known {{ count: 0 }}, got: {result:?}"
        );
    }

    #[test]
    fn count_skills_for_plugin_returns_known_when_manifest_absent() {
        let (dir, svc) = temp_service();
        let marketplace_path = dir.path().join("marketplace");
        // No plugin.json → defaults kick in. Create the default skills/
        // layout so the count is nonzero.
        make_plugin_with_skills(&marketplace_path, "defaults", &["alpha", "beta"]);
        // Remove the default plugin.json we didn't ask for (make_plugin_with_skills
        // only creates skill dirs, not plugin.json).

        let entry = relative_path_entry("defaults", "plugins/defaults");
        let result = svc.count_skills_for_plugin(&entry, &marketplace_path);
        assert!(
            matches!(result, SkillCount::Known { count: 2 }),
            "expected Known {{ count: 2 }}, got: {result:?}"
        );
    }

    #[test]
    fn count_skills_for_plugin_returns_remote_for_structured_source() {
        let (_dir, svc) = temp_service();
        let marketplace_path = Path::new("/tmp/nonexistent-marketplace");

        let entry = PluginEntry {
            name: "remote".into(),
            description: None,
            source: PluginSource::Structured(StructuredSource::GitHub {
                repo: "owner/repo".into(),
                git_ref: None,
                sha: None,
            }),
        };

        let result = svc.count_skills_for_plugin(&entry, marketplace_path);
        assert!(
            matches!(result, SkillCount::RemoteNotCounted),
            "expected RemoteNotCounted, got: {result:?}"
        );
    }

    #[test]
    fn count_skills_for_plugin_returns_manifest_failed_on_missing_plugin_dir() {
        let (dir, svc) = temp_service();
        let marketplace_path = dir.path().join("marketplace");
        fs::create_dir_all(&marketplace_path).expect("create marketplace root");

        let entry = relative_path_entry("ghost", "plugins/ghost");
        let result = svc.count_skills_for_plugin(&entry, &marketplace_path);
        assert!(
            matches!(
                result,
                SkillCount::ManifestFailed {
                    reason: SkippedReason::DirectoryMissing { .. }
                }
            ),
            "expected ManifestFailed/DirectoryMissing, got: {result:?}"
        );
    }

    #[test]
    fn count_skills_for_plugin_returns_manifest_failed_when_plugin_dir_is_a_file() {
        let (dir, svc) = temp_service();
        let marketplace_path = dir.path().join("marketplace");
        let plugins_root = marketplace_path.join("plugins");
        fs::create_dir_all(&plugins_root).expect("create plugins root");
        // Create a regular file where the plugin dir should be.
        fs::write(plugins_root.join("not-a-dir"), b"i am a file").expect("write file");

        let entry = relative_path_entry("not-a-dir", "plugins/not-a-dir");
        let result = svc.count_skills_for_plugin(&entry, &marketplace_path);
        assert!(
            matches!(
                result,
                SkillCount::ManifestFailed {
                    reason: SkippedReason::NotADirectory { .. }
                }
            ),
            "expected ManifestFailed/NotADirectory, got: {result:?}"
        );
    }

    #[test]
    #[cfg(unix)]
    fn count_skills_for_plugin_returns_manifest_failed_on_symlinked_plugin_dir() {
        use std::os::unix::fs::symlink;

        let (dir, svc) = temp_service();
        let marketplace_path = dir.path().join("marketplace");
        let plugins_root = marketplace_path.join("plugins");
        fs::create_dir_all(&plugins_root).expect("create plugins root");
        // Symlink target must exist so the symlink itself is what triggers
        // the refusal, not a broken-symlink variant.
        let real_target = dir.path().join("real-plugin");
        fs::create_dir_all(&real_target).expect("create real target");
        symlink(&real_target, plugins_root.join("symlinked")).expect("create symlink");

        let entry = relative_path_entry("symlinked", "plugins/symlinked");
        let result = svc.count_skills_for_plugin(&entry, &marketplace_path);
        assert!(
            matches!(
                result,
                SkillCount::ManifestFailed {
                    reason: SkippedReason::SymlinkRefused { .. }
                }
            ),
            "expected ManifestFailed/SymlinkRefused, got: {result:?}"
        );
    }

    #[test]
    fn count_skills_for_plugin_returns_manifest_failed_on_malformed_json() {
        let (dir, svc) = temp_service();
        let marketplace_path = dir.path().join("marketplace");
        let plugin_dir = marketplace_path.join("plugins/broken");
        fs::create_dir_all(&plugin_dir).expect("create plugin dir");
        fs::write(plugin_dir.join("plugin.json"), b"{not json").expect("write plugin.json");

        let entry = relative_path_entry("broken", "plugins/broken");
        let result = svc.count_skills_for_plugin(&entry, &marketplace_path);
        assert!(
            matches!(
                result,
                SkillCount::ManifestFailed {
                    reason: SkippedReason::InvalidManifest { .. }
                }
            ),
            "expected ManifestFailed/InvalidManifest, got: {result:?}"
        );
    }

    #[test]
    #[cfg(unix)]
    fn count_skills_for_plugin_treats_symlinked_plugin_json_as_missing() {
        use std::os::unix::fs::symlink;

        let (dir, svc) = temp_service();
        let marketplace_path = dir.path().join("marketplace");
        let plugin_dir = marketplace_path.join("plugins/symjson");
        fs::create_dir_all(&plugin_dir).expect("create plugin dir");
        // Symlinked plugin.json is treated as absent by load_plugin_manifest
        // (security hardening, see service/browse.rs:656). That means we
        // fall back to default skill paths — no skills/ dir exists here,
        // so count is 0. Regression pin for this specific interaction.
        let real_manifest = dir.path().join("real-plugin.json");
        fs::write(&real_manifest, b"{\"name\":\"irrelevant\"}").expect("write real manifest");
        symlink(&real_manifest, plugin_dir.join("plugin.json")).expect("create symlink");

        let entry = relative_path_entry("symjson", "plugins/symjson");
        let result = svc.count_skills_for_plugin(&entry, &marketplace_path);
        assert!(
            matches!(result, SkillCount::Known { count: 0 }),
            "expected Known {{ count: 0 }}, got: {result:?}"
        );
    }
```

- [ ] **Step 2.6: Run all `count_skills_for_plugin` tests**

Run: `cargo test -p kiro-market-core --lib service::browse::tests::count_skills_for_plugin 2>&1 | tail -15`
Expected: `test result: ok. 8 passed` (7 for the 7 non-cfg-gated + 1 cfg(unix) + 1 cfg(unix) = 8 on Linux/macOS; 6 on Windows where the two `#[cfg(unix)]` tests are skipped).

- [ ] **Step 2.7: Run the full browse test module to check for regressions**

Run: `cargo test -p kiro-market-core --lib service::browse 2>&1 | tail -10`
Expected: all tests pass. No existing tests broken.

- [ ] **Step 2.8: Run clippy**

Run: `cargo clippy -p kiro-market-core --tests -- -D warnings 2>&1 | tail -10`
Expected: no warnings or errors.

- [ ] **Step 2.9: Commit**

```bash
git add crates/kiro-market-core/src/service/browse.rs
git commit -m "$(cat <<'EOF'
feat(core): add MarketplaceService::count_skills_for_plugin

Replaces the Tauri-side `count_plugin_skills` silent-zero behavior
with a three-way SkillCount result. Plugin-directory pre-check reuses
`resolve_local_plugin_dir` rather than duplicating the stat logic,
then falls through to `load_plugin_manifest` for plugin.json-specific
failures. Two private helpers
(`skipped_reason_from_resolve_error` / `skipped_reason_from_manifest_error`)
centralize the Error → SkippedReason projection so the method body
stays readable.

Part of #32.
EOF
)"
```

---

## Task 3: Tauri FFI wiring + bindings regeneration + Layer 3 integration test

**Files:**
- Modify: `crates/kiro-control-center/src-tauri/src/commands/browse.rs` — change `PluginInfo.skill_count` type (line 60), replace `list_plugins` call site (line 136), delete `count_plugin_skills` (lines ~435–462).
- Possibly modify: `crates/kiro-control-center/src-tauri/src/lib.rs` (specta re-exports, if needed).
- Auto-regenerate: `crates/kiro-control-center/src/lib/bindings.ts`.

- [ ] **Step 3.1: Update `PluginInfo.skill_count` field type**

In `crates/kiro-control-center/src-tauri/src/commands/browse.rs`, find the `PluginInfo` struct (line ~56). Change the field type:

```rust
// Before (around line 56–62):
pub struct PluginInfo {
    pub name: String,
    pub description: Option<String>,
    pub skill_count: u32,
    pub source_type: SourceType,
}

// After:
pub struct PluginInfo {
    pub name: String,
    pub description: Option<String>,
    pub skill_count: kiro_market_core::service::browse::SkillCount,
    pub source_type: SourceType,
}
```

If the file already has a `use` import that would make a shorter path idiomatic (e.g. `use kiro_market_core::service::browse::*`), use the shorter path. Otherwise import `SkillCount`:

```rust
// Near the top of the file, in the imports block:
use kiro_market_core::service::browse::{BulkSkillsResult, PluginSkillsResult, SkillCount};
```

(Add `SkillCount` to whichever `use kiro_market_core::service::browse::{...}` line already exists; if there's no such line, add one.)

- [ ] **Step 3.2: Replace the `list_plugins` call site**

In the same file, find `list_plugins` around line 126. Locate the loop body at line ~134–143:

```rust
// Before:
for plugin in &plugin_entries {
    let source_type = plugin_source_type(&plugin.source);
    let skill_count = count_plugin_skills(plugin, &marketplace_path);
    results.push(PluginInfo {
        name: plugin.name.clone(),
        description: plugin.description.clone(),
        skill_count: saturate_to_u32(skill_count, "skill_count"),
        source_type,
    });
}

// After:
for plugin in &plugin_entries {
    let source_type = plugin_source_type(&plugin.source);
    results.push(PluginInfo {
        name: plugin.name.clone(),
        description: plugin.description.clone(),
        skill_count: svc.count_skills_for_plugin(plugin, &marketplace_path),
        source_type,
    });
}
```

- [ ] **Step 3.3: Delete the local `count_plugin_skills` function**

In the same file, delete the entire `count_plugin_skills` function (roughly lines 435–462 — the doc comment block starting `/// Count skills within a plugin entry...` through the closing `}` of the function). Do **not** delete `load_plugin_manifest` or `discover_skills_for_plugin` (both still used by `install_skills` at lines 227/229).

- [ ] **Step 3.4: Run the Rust side to confirm it compiles**

Run: `cargo check -p kiro-control-center 2>&1 | tail -20`
Expected: builds clean. If clippy complains about unused imports (e.g. `saturate_to_u32` if no longer used in the file), remove them. Verify by re-running `cargo check`.

- [ ] **Step 3.5: Regenerate specta bindings**

Run: `cargo test -p kiro-control-center --lib -- --ignored 2>&1 | tail -15`
Expected: runs the bindings-regeneration test and updates `crates/kiro-control-center/src/lib/bindings.ts`.

Verify the `SkillCount` type appears in the regenerated bindings:

Run: `grep -n "export type SkillCount" crates/kiro-control-center/src/lib/bindings.ts`
Expected output (path and exact text may vary slightly with specta version):
```
export type SkillCount = { state: "known"; count: number } | { state: "remote_not_counted" } | { state: "manifest_failed"; reason: SkippedReason };
```

And verify `PluginInfo.skill_count` now references `SkillCount`:

Run: `grep -nA 4 "export type PluginInfo" crates/kiro-control-center/src/lib/bindings.ts`
Expected: the `skill_count` field is typed `SkillCount`, not `number`.

If the bindings regenerated but the `SkillCount` export is missing, add a specta re-export in the Tauri crate's `lib.rs`. Open `crates/kiro-control-center/src-tauri/src/lib.rs` and search for the existing `collect_types!` / `Builder::new().commands(...)` call. If `SkippedReason` is listed as an exported type there, add `SkillCount` alongside it. If the file re-exports types implicitly via the struct fields (the common specta pattern), no change is needed — the struct-field reference already triggered the export.

- [ ] **Step 3.6: Write the Layer 3 integration test**

In `crates/kiro-control-center/src-tauri/src/commands/browse.rs`, at the end of the `#[cfg(test)] mod tests` block (line 564 or so), add a test that exercises the batch `list_plugins` path end-to-end. Look for the nearest existing `list_plugins` integration test to match its style; if no such test exists, add this one as the first:

```rust
    #[tokio::test]
    async fn list_plugins_surfaces_three_skill_count_states() {
        use kiro_market_core::marketplace::{PluginEntry, PluginSource, StructuredSource};
        use kiro_market_core::validation::RelativePath;
        use std::fs;
        use tempfile::tempdir;

        // Integration-test harness setup mirrors other tests in this module.
        // If a richer helper exists (e.g. `with_marketplace`), prefer that
        // and adapt the body accordingly. Pattern below uses direct writes
        // to a tempdir cache.
        let cache_dir = tempdir().expect("tempdir");
        // Instead of spinning up the full Tauri test harness (heavy), call
        // the core method directly against a service configured to use
        // this cache — matching the existing integration-test pattern in
        // this file. If no such pattern exists yet, make this a unit test
        // on the core crate instead and delete this integration case.
        // (Implementation note: add the full setup wiring matching nearest
        // existing test harness; do NOT cargo-cult from this template.)
        //
        // Three plugins:
        //   - "local-ok" with 2 skills → Known { count: 2 }
        //   - "local-broken" with malformed plugin.json → ManifestFailed
        //   - "remote" with GitHub source → RemoteNotCounted
        //
        // After invoking `list_plugins(marketplace)`, assert each variant
        // appears in the returned Vec<PluginInfo>.

        // Gap: this test is a placeholder skeleton because the Tauri-crate
        // test harness pattern for `list_plugins` doesn't yet exist in
        // this file (the Tauri command wraps `MarketplaceService` whose
        // tests are in the core crate). If implementation reveals there
        // IS a pattern to follow, fill this in. If there isn't, delete
        // this test and rely on the Task 2 core-crate tests plus a brief
        // manual smoke test via `cargo tauri dev` + the Browse tab.
        //
        // Either outcome is acceptable — the core coverage from Task 2
        // is the authoritative pin for the behavior.
        let _ = cache_dir;
    }
```

**Important:** If upon writing this test you find the Tauri crate already has a `list_plugins` test harness (e.g., one that constructs `MarketplaceService` with a fake cache and calls `list_plugins(mp_name).await`), delete the placeholder and follow that pattern to make the three-plugin assertion real. If it doesn't, delete the placeholder and document in the commit message that Layer 3 coverage deferred to manual smoke-test (the spec explicitly permits this fallback).

- [ ] **Step 3.7: Decide integration-test fate and run the test suite**

Run: `cargo test -p kiro-control-center 2>&1 | tail -15`
Expected: if the real integration test was written in Step 3.6, it passes. If the placeholder remained and was deleted per the "Important" note, nothing new runs but nothing breaks either.

- [ ] **Step 3.8: Run clippy on the Tauri crate**

Run: `cargo clippy -p kiro-control-center --tests -- -D warnings 2>&1 | tail -10`
Expected: no warnings or errors. If `saturate_to_u32` is now unused in the file, confirm its import / definition was cleaned up in Step 3.4.

- [ ] **Step 3.9: Run TypeScript check**

Run: `cd crates/kiro-control-center && npm run check 2>&1 | tail -20; cd -`
Expected: `svelte-check` currently fails on `BrowseTab.svelte` because the `skill_count` field is now a `SkillCount` object but the template still renders it as a number. That's the expected state heading into Task 4. Note the failure but do not fix it yet.

- [ ] **Step 3.10: Commit**

```bash
git add crates/kiro-control-center/src-tauri/src/commands/browse.rs crates/kiro-control-center/src/lib/bindings.ts
# Add lib.rs too ONLY if Step 3.5 required a re-export edit:
# git add crates/kiro-control-center/src-tauri/src/lib.rs
git commit -m "$(cat <<'EOF'
feat(tauri): wire PluginInfo.skill_count to core SkillCount

PluginInfo.skill_count changes from u32 to SkillCount, list_plugins
now calls MarketplaceService::count_skills_for_plugin, and the
Tauri-local count_plugin_skills helper is deleted. bindings.ts
regenerated. load_plugin_manifest / discover_skills_for_plugin stay
(still called by install_skills; removal is #33's scope).

Frontend rendering is intentionally broken at this commit — fixed in
the next commit. Part of #32.
EOF
)"
```

---

## Task 4: Svelte `BrowseTab.svelte` rendering update

**Files:**
- Modify: `crates/kiro-control-center/src/lib/components/BrowseTab.svelte` — add three helpers at the top of `<script>` block, update the rendering span at line 704.

- [ ] **Step 4.1: Import `SkillCount` from bindings**

Open `crates/kiro-control-center/src/lib/components/BrowseTab.svelte`. Find the existing import from `'$lib/bindings'` (around line 7–10). It currently includes `SkippedSkill`. Add `SkillCount` and `SkippedReason`:

```typescript
// Before (approx.):
import {
  commands,
  SkippedSkill,
  // ... other imports ...
} from '$lib/bindings';

// After:
import {
  commands,
  SkippedSkill,
  SkippedReason,
  SkillCount,
  // ... other imports ...
} from '$lib/bindings';
```

(If `SkippedReason` is already imported, just add `SkillCount`. Confirm by reading lines 1–30 of the file before editing.)

- [ ] **Step 4.2: Add `formatSkippedReason` helper**

Still in the `<script>` block, find the existing `formatSkippedSkill` helper (around line 19). Immediately before it, add:

```typescript
  // Render a structured SkippedReason as a one-line string. Total over
  // all eight variants (not just the six reachable via SkillCount) —
  // TypeScript's exhaustiveness check forces full coverage since
  // SkippedReason is used in multiple contexts.
  function formatSkippedReason(r: SkippedReason): string {
    switch (r.kind) {
      case "directory_missing":
        return `plugin directory not found: ${r.path}`;
      case "not_a_directory":
        return `plugin path is not a directory: ${r.path}`;
      case "symlink_refused":
        return `plugin path is a symlink (refused): ${r.path}`;
      case "directory_unreadable":
        return `could not read plugin directory: ${r.reason}`;
      case "invalid_manifest":
        return `plugin.json is malformed: ${r.reason}`;
      case "manifest_read_failed":
        return `could not read plugin.json: ${r.reason}`;
      case "remote_source_not_local":
        return `plugin source is remote: ${r.plugin}`;
      case "no_skills":
        return `plugin declares no skills: ${r.path}`;
    }
  }
```

- [ ] **Step 4.3: Add `skillCountLabel` and `skillCountTitle` helpers**

Immediately after `formatSkippedReason`, add:

```typescript
  function skillCountLabel(sc: SkillCount): string {
    switch (sc.state) {
      case "known": return String(sc.count);
      case "remote_not_counted": return "–";
      case "manifest_failed": return "!";
    }
  }

  function skillCountTitle(sc: SkillCount): string | undefined {
    switch (sc.state) {
      case "known":
        return undefined;
      case "remote_not_counted":
        return "Remote plugin — skills cannot be counted without cloning";
      case "manifest_failed":
        return `plugin.json failed to load: ${formatSkippedReason(sc.reason)}`;
    }
  }
```

- [ ] **Step 4.4: Update the rendering span**

Find line 704 (`<span class="text-[11px] text-kiro-subtle">{ap.plugin.skill_count}</span>`). Replace it with:

```svelte
                  <span
                    class="text-[11px] {ap.plugin.skill_count.state === 'manifest_failed' ? 'text-kiro-warning' : 'text-kiro-subtle'}"
                    title={skillCountTitle(ap.plugin.skill_count)}
                  >{skillCountLabel(ap.plugin.skill_count)}</span>
```

Preserve the surrounding `<label>` structure exactly. The class string is a Svelte class directive — the template syntax above is valid Svelte 5.

- [ ] **Step 4.5: Verify `text-kiro-warning` token exists; substitute if not**

Run: `grep -rn "kiro-warning" crates/kiro-control-center/src/ crates/kiro-control-center/tailwind.config.* 2>/dev/null | head -5`

- If matches are found (token exists), no change needed.
- If no matches are found, the token is undefined. Grep for how `MarketplaceInfo.load_error` is rendered and reuse its color class:

  Run: `grep -n "load_error" crates/kiro-control-center/src/lib/components/BrowseTab.svelte`

  Look at the `class=` on the span rendering the warning badge for marketplace load errors; use the same token (commonly `text-yellow-500` or `text-kiro-accent-400` depending on the project's theme). Replace `text-kiro-warning` in Step 4.4's span with that token.

- [ ] **Step 4.6: Run `npm run check`**

Run: `cd crates/kiro-control-center && npm run check 2>&1 | tail -25; cd -`
Expected: zero errors. The exhaustive switches over `SkillCount.state` and `SkippedReason.kind` confirm all variants are handled.

- [ ] **Step 4.7: Smoke-test in a browser**

Per CLAUDE.md's UI-testing rule, verify the feature in a browser before marking this task complete.

Run: `cd crates/kiro-control-center && npm run tauri dev 2>&1 &`
(Wait ~30 seconds for Tauri to spin up.)

Manual check in the app window:
- Open the Browse tab.
- Verify the plugin sidebar: local plugins with skills show numeric counts, remote plugins show `"–"`, and any plugin with a deliberately-malformed `plugin.json` in your cache shows `"!"` with a tooltip on hover.
- To force a `ManifestFailed` row without doctoring real state, you can temporarily point a marketplace at a directory containing a plugin whose `plugin.json` is malformed — or skip this step if you don't have such a marketplace handy and rely on the automated tests.

Stop the dev server (`kill %1` or Ctrl-C).

If the three states render correctly (or cannot be tested live without significant fixture construction — common), note that explicitly in the commit message rather than claiming full UI verification.

- [ ] **Step 4.8: Commit**

```bash
git add crates/kiro-control-center/src/lib/components/BrowseTab.svelte
git commit -m "$(cat <<'EOF'
feat(browse): render SkillCount three-way state in sidebar

Plugin sidebar now renders a number for Known, em-dash for
RemoteNotCounted, and `!` with a tooltip for ManifestFailed. Adds
formatSkippedReason, skillCountLabel, and skillCountTitle helpers.
TypeScript exhaustiveness checks pin all SkillCount / SkippedReason
variants.

Part of #32.
EOF
)"
```

---

## Task 5: Workspace-wide pre-commit verification

**Files:** no edits expected unless formatter or linter surfaces an issue.

- [ ] **Step 5.1: Run `cargo fmt` check**

Run: `cargo fmt --all --check 2>&1 | tail -10`
Expected: no output, exit 0. If formatting issues surface:
- Run `cargo fmt --all` to fix.
- `git add` the changed files.
- `git commit -m "chore: cargo fmt"`.

- [ ] **Step 5.2: Run full workspace test suite**

Run: `cargo test --workspace 2>&1 | tail -15`
Expected: all tests pass. No regressions anywhere.

- [ ] **Step 5.3: Run full workspace clippy**

Run: `cargo clippy --workspace --tests -- -D warnings 2>&1 | tail -15`
Expected: no warnings or errors.

- [ ] **Step 5.4: Run TypeScript check one more time**

Run: `cd crates/kiro-control-center && npm run check 2>&1 | tail -10; cd -`
Expected: zero errors.

- [ ] **Step 5.5: Confirm no new `#[allow(...)]` directives were introduced**

Run: `git diff main -- 'crates/**/*.rs' | grep -E '^\+.*#\[allow\(' || echo "clean"`
Expected output: `clean` (no new `#[allow]` added per CLAUDE.md zero-tolerance rule).

If any `#[allow]` snuck in, revisit the code and fix the underlying issue rather than suppressing.

- [ ] **Step 5.6: Review diff for spec coverage**

Run: `git log --oneline main..HEAD`
Expected: four feature commits (Task 1–4) plus optionally one `chore: cargo fmt` commit if Step 5.1 surfaced formatting.

Scan the diff:
- `git diff main -- crates/kiro-market-core/src/service/browse.rs` — confirm `SkillCount` enum + `count_skills_for_plugin` method + two helpers + 8 tests.
- `git diff main -- crates/kiro-control-center/src-tauri/src/commands/browse.rs` — confirm `PluginInfo.skill_count` changed, `list_plugins` updated, `count_plugin_skills` deleted.
- `git diff main -- crates/kiro-control-center/src/lib/bindings.ts` — confirm `SkillCount` and updated `PluginInfo` present.
- `git diff main -- crates/kiro-control-center/src/lib/components/BrowseTab.svelte` — confirm three helpers added and rendering updated.

---

## Acceptance criteria (from spec)

Tick these off after Task 5 passes. Re-run verification if any fails.

- [ ] `MarketplaceService::count_skills_for_plugin(&PluginEntry, &Path) -> SkillCount` added in `kiro-market-core::service::browse` with doc comment, `#[must_use]`, three-outcome contract.
- [ ] Plugin-directory hardening pre-check performed (via delegation to `resolve_local_plugin_dir`; spec note: no new `check_plugin_dir` helper needed — `resolve_local_plugin_dir` covers all four failure variants).
- [ ] `SkillCount` enum added as `Serialize + specta::Type` (feature-gated) with `#[serde(tag = "state", rename_all = "snake_case")]` and `#[non_exhaustive]`.
- [ ] `PluginInfo.skill_count` changes from `u32` to `SkillCount`. `bindings.ts` regenerated.
- [ ] `list_plugins` Tauri handler calls the core method; Tauri-local `count_plugin_skills` deleted. `load_plugin_manifest` / `discover_skills_for_plugin` left in place (still used by `install_skills`; #33 scope).
- [ ] Svelte `BrowseTab.svelte` renders three states: number for `Known`, `"–"` for `RemoteNotCounted`, `"!"` with warning color + tooltip for `ManifestFailed`.
- [ ] Unit tests cover Layer 1 table from spec. Wire-format pins added. Layer 3 integration test added or explicitly deferred per Task 3.
- [ ] `cargo test --workspace`, `cargo clippy --workspace --tests -- -D warnings`, `cargo fmt --all --check`, `npm run check` all pass.
- [ ] No new `#[allow(...)]` directives (CLAUDE.md zero-tolerance).

---

## Notes for the implementing engineer

1. **The spec is the contract; this plan is the how.** If a task step contradicts the spec, the spec wins. Raise the discrepancy in a PR comment.

2. **Do not skip the "run and watch it fail" steps.** They confirm the test is actually exercising the code under test and not passing trivially (e.g. through a typo that matches a constant).

3. **Commit at the end of each task, not each step.** The task boundaries are designed so each commit leaves the workspace in a reasonable state (tests pass for completed tasks; TypeScript may transiently fail between Task 3 and Task 4 — that's acceptable and flagged in the Task 3 commit message).

4. **When a step says "Expected output" and the actual output differs,** investigate before proceeding. Silent skips here compound.

5. **The `resolve_local_plugin_dir` delegation is the spec-to-plan refinement.** If you find it doesn't cover a spec-listed failure variant, stop and raise it rather than adding a new helper — the five existing tests on `resolve_local_plugin_dir` are your safety net.

6. **Remote-source short-circuit must come before `resolve_local_plugin_dir`.** Otherwise the remote case returns `ManifestFailed { reason: RemoteSourceNotLocal }` instead of the `RemoteNotCounted` variant the frontend wants — a subtle but user-visible miscategorization.

# Design: `count_plugin_skills` silent-zero fix (#32)

**Date:** 2026-04-21
**Issue:** [#32 — count_plugin_skills returns silent 0 when plugin.json fails to load](https://github.com/dwalleck/kiro-control-center/issues/32)
**Branch:** `fix/count-plugin-skills-silent-zero`
**Follow-ups unblocked:** #33 (dedup Tauri-local `load_plugin_manifest`)

---

## Problem

`count_plugin_skills` at `crates/kiro-control-center/src-tauri/src/commands/browse.rs:442` returns `0` in two distinct cases that the UI cannot distinguish:

1. **Remote plugin** — `PluginSource::Structured(_)`. Skills are not locally countable without cloning.
2. **Local plugin, manifest load failed** — `load_plugin_manifest` returned `Err`. Currently swallowed by a `warn!` log and `return 0`.

A plugin with a legitimately zero skill count renders the same way in the filter sidebar as a plugin whose `plugin.json` is malformed, which is the same antipattern #30 fixed across `list_skills_for_plugin` / `list_all_skills` / `install_skills` (silent drops replaced by structured `SkippedPlugin` / `SkippedReason` projections).

---

## Approved design decisions (from brainstorm)

| # | Decision | Choice |
|---|---|---|
| 1 | Where does the function live? | **Move to core.** New method `MarketplaceService::count_skills_for_plugin` in `kiro-market-core::service::browse`. Tauri's `list_plugins` calls it. |
| 2 | Error payload type for `ManifestFailed`? | **Reuse `SkippedReason`** with an inline doc comment naming the unreachable variants (`NoSkills`, `RemoteSourceNotLocal`). One projection function (`SkippedReason::from_plugin_error`) stays the single source of truth. |
| 3 | How does `PluginInfo` carry the new shape? | **In-place type change** — `skill_count: u32` becomes `skill_count: SkillCount`. Same field name, new type. |
| 4 | Service method signature? | **Entry-plus-path** — `count_skills_for_plugin(&self, plugin: &PluginEntry, marketplace_path: &Path) -> SkillCount`. Plain return (no `Result`). Avoids N² registry parsing in the batch caller. |
| 5 | Svelte rendering? | **Minimal text labels** — `Known` → number, `RemoteNotCounted` → `"–"`, `ManifestFailed` → `"!"` in a warning color with a `title` tooltip containing the `SkippedReason` detail. Plugin name unchanged across all states. |

---

## Rust core

### New type: `SkillCount`

Location: `crates/kiro-market-core/src/service/browse.rs`, co-located with `SkippedReason`.

```rust
/// Result of counting skills for a single plugin in a marketplace listing.
/// Distinguishes the three cases the frontend must render differently:
/// a known count, a remote plugin (not locally countable), and a local
/// plugin whose manifest could not be loaded. Replaces the prior
/// `usize` that collapsed failures into a silent 0.
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
    /// Distinct from `ManifestFailed { reason: RemoteSourceNotLocal }`
    /// (which is reachable from the bulk-listing path when a plugin
    /// that should have been local resolves remote) — here we know
    /// the plugin is remote by construction and never attempt the load.
    RemoteNotCounted,

    /// The plugin is local but something about its directory or
    /// manifest prevented a skill count.
    ///
    /// `SkippedReason` is reused as the error payload to share the #30
    /// projection `SkippedReason::from_plugin_error`. Six of its
    /// variants are reachable from this path:
    ///
    /// From the pre-`load_plugin_manifest` plugin-directory check:
    /// - `DirectoryMissing` — `plugin_dir` does not exist.
    /// - `NotADirectory` — `plugin_dir` exists but is a file or other
    ///   non-directory node.
    /// - `SymlinkRefused` — `plugin_dir` is a symlink; refused for the
    ///   same reason `list_all_skills` refuses them (untrusted
    ///   clone path could redirect at arbitrary host files).
    /// - `DirectoryUnreadable` — stat'ing `plugin_dir` failed for any
    ///   other reason (permission denied, transient I/O, etc.).
    ///
    /// From `load_plugin_manifest` (`plugin.json`-specific):
    /// - `InvalidManifest` — `plugin.json` exists but is malformed.
    /// - `ManifestReadFailed` — `plugin.json` exists and was stat'd
    ///   successfully but the subsequent read failed.
    ///
    /// `NoSkills` is not produced anywhere in this path;
    /// `RemoteSourceNotLocal` is pre-empted by `RemoteNotCounted`.
    /// Frontends typed against `SkippedReason` will not get
    /// compile-time narrowing for those two — accepted because
    /// consolidating the projection is more valuable than a narrower
    /// wire type.
    ManifestFailed { reason: SkippedReason },
}
```

### New method: `MarketplaceService::count_skills_for_plugin`

Location: same `impl MarketplaceService` block as `list_skills_for_plugin` in `service/browse.rs` (so the existing private `load_plugin_manifest` and `discover_skills_for_plugin` are in scope without visibility changes).

```rust
/// Count skills for a single plugin entry without loading skill bodies.
///
/// Returns `SkillCount::RemoteNotCounted` for remote sources,
/// `SkillCount::ManifestFailed` if the plugin directory or its
/// `plugin.json` cannot be read or parsed, and `SkillCount::Known { count }`
/// otherwise (including the legitimate zero case where the manifest is
/// absent or declares no skills).
///
/// Takes the pre-resolved `PluginEntry` and `marketplace_path` so the
/// batch caller in `list_plugins` pays for the registry parse once per
/// marketplace rather than once per plugin. Errors are never propagated
/// as `Err` — every outcome fits the three-way union.
#[must_use]
pub fn count_skills_for_plugin(
    &self,
    plugin: &PluginEntry,
    marketplace_path: &Path,
) -> SkillCount {
    match &plugin.source {
        PluginSource::Structured(_) => SkillCount::RemoteNotCounted,
        PluginSource::RelativePath(rel) => {
            let plugin_dir = marketplace_path.join(rel);

            // Plugin-directory pre-check. Mirrors the hardening
            // `list_all_skills` already applies: missing /
            // non-directory / symlinked / unreadable `plugin_dir`
            // all surface structurally rather than silently
            // collapsing into a zero-skill count. Without this,
            // `load_plugin_manifest` would see NotFound when
            // stat'ing `plugin_dir/plugin.json` inside a missing
            // parent and return `Ok(None)` — preserving the silent-0
            // bug one level up from the one #32 names directly.
            match check_plugin_dir(&plugin_dir) {
                Ok(()) => {}
                Err(reason) => {
                    warn!(
                        plugin = %plugin.name,
                        path = %plugin_dir.display(),
                        ?reason,
                        "plugin directory check failed; reporting as ManifestFailed"
                    );
                    return SkillCount::ManifestFailed { reason };
                }
            }

            match load_plugin_manifest(&plugin_dir) {
                Ok(manifest) => {
                    let count = discover_skills_for_plugin(
                        &plugin_dir,
                        manifest.as_ref(),
                    ).len();
                    SkillCount::Known {
                        count: u32::try_from(count).unwrap_or(u32::MAX),
                    }
                }
                Err(Error::Plugin(pe)) => {
                    // `SkippedReason::from_plugin_error` is the single
                    // classifier; a new plugin-level variant there
                    // lands here automatically.
                    let reason = SkippedReason::from_plugin_error(&pe)
                        .unwrap_or_else(|| {
                            // `PluginError::NotFound` / `ManifestNotFound`
                            // — the classifier deliberately returns None
                            // ("caller asked for the wrong thing"). Not
                            // reachable from `load_plugin_manifest`
                            // today; defensive fallback logs and folds
                            // into ManifestReadFailed so a future
                            // producer cannot regress to a silent 0.
                            warn!(
                                plugin = %plugin.name,
                                error = ?pe,
                                "unclassified PluginError in count path"
                            );
                            SkippedReason::ManifestReadFailed {
                                path: plugin_dir.join("plugin.json"),
                                reason: pe.to_string(),
                            }
                        });
                    warn!(
                        plugin = %plugin.name,
                        path = %plugin_dir.display(),
                        ?reason,
                        "plugin.json load failed; reporting as ManifestFailed"
                    );
                    SkillCount::ManifestFailed { reason }
                }
                Err(other) => {
                    // `load_plugin_manifest` only returns `Error::Plugin`
                    // today, but `Error` is `#[non_exhaustive]` —
                    // defensive fallthrough so a new top-level variant
                    // doesn't silently vanish here either.
                    warn!(
                        plugin = %plugin.name,
                        error = %error_full_chain(&other),
                        "unexpected non-plugin error in count path"
                    );
                    SkillCount::ManifestFailed {
                        reason: SkippedReason::ManifestReadFailed {
                            path: plugin_dir.join("plugin.json"),
                            reason: error_full_chain(&other),
                        },
                    }
                }
            }
        }
    }
}
```

**Notes:**
- `u32::try_from(count).unwrap_or(u32::MAX)` replaces the current `saturate_to_u32` helper at the Tauri boundary. Saturation now lives in core since `SkillCount` is a core type.
- Both defensive `warn!` branches log but never panic or return `Err` — the contract is "every outcome fits the three-way union."
- `warn!` logging is preserved at the core level, matching the existing pattern in `load_plugin_manifest` itself.

### New helper: `check_plugin_dir`

Co-located with `count_skills_for_plugin` in `service/browse.rs`. Private to the module.

```rust
/// Validate a plugin directory before attempting manifest load.
/// Returns `Ok(())` for a real, readable, non-symlinked directory;
/// returns a `SkippedReason` describing the failure mode otherwise.
///
/// Mirrors the plugin-directory hardening in `collect_skills_for_plugin_into`
/// (bulk listing path). Kept as a separate helper so the pre-check is one
/// call and the reasoning is visible at the `count_skills_for_plugin` site.
fn check_plugin_dir(plugin_dir: &Path) -> Result<(), SkippedReason> {
    match fs::symlink_metadata(plugin_dir) {
        Ok(m) if m.file_type().is_symlink() => {
            Err(SkippedReason::SymlinkRefused {
                path: plugin_dir.to_path_buf(),
            })
        }
        Ok(m) if !m.is_dir() => {
            Err(SkippedReason::NotADirectory {
                path: plugin_dir.to_path_buf(),
            })
        }
        Ok(_) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            Err(SkippedReason::DirectoryMissing {
                path: plugin_dir.to_path_buf(),
            })
        }
        Err(e) => Err(SkippedReason::DirectoryUnreadable {
            path: plugin_dir.to_path_buf(),
            reason: e.to_string(),
        }),
    }
}
```

**Alignment with bulk path:** `collect_skills_for_plugin_into` in the bulk listing path performs a similar plugin-directory check today (producing the same `SkippedReason` variants). Implementation step 1 verifies whether the bulk-path check is cleanly extractable into a shared `check_plugin_dir` helper usable from both sites; if so, do that and remove the duplication. If the bulk-path check carries extra logic around the stat (e.g., interleaved with manifest-load work) that makes extraction awkward, keep `check_plugin_dir` local to `count_skills_for_plugin` and file a follow-up issue to dedup later. Either outcome is acceptable for this PR.

---

## FFI boundary (Tauri)

### `PluginInfo` field type change

`crates/kiro-control-center/src-tauri/src/commands/browse.rs`:

```rust
pub struct PluginInfo {
    pub name: String,
    pub description: Option<String>,
    pub skill_count: SkillCount,          // was: u32
    pub source_type: SourceType,
}
```

Re-export `SkillCount` (and confirm `SkippedReason` re-export) from the Tauri crate so specta picks them up for `bindings.ts`. `SkippedReason` already crosses the FFI via `SkippedPlugin` (#30), so the re-export plumbing exists.

### `list_plugins` call site (line ~136) simplifies

```rust
// Before:
let skill_count = count_plugin_skills(plugin, &marketplace_path);
results.push(PluginInfo {
    name: plugin.name.clone(),
    description: plugin.description.clone(),
    skill_count: saturate_to_u32(skill_count, "skill_count"),
    source_type,
});

// After:
results.push(PluginInfo {
    name: plugin.name.clone(),
    description: plugin.description.clone(),
    skill_count: svc.count_skills_for_plugin(plugin, &marketplace_path),
    source_type,
});
```

### Deletions in `src-tauri/src/commands/browse.rs`

- `fn count_plugin_skills` (lines 442–462) — fully replaced by the core method.
- The `saturate_to_u32` call site for `skill_count` — saturation moves into core.

### What stays behind for #33

Both Tauri-local helpers **stay in place after this PR** because the `install_skills` Tauri command at `browse.rs:200` still calls them:

- `load_plugin_manifest` (line 357) — called by `install_skills` at line 227. Last non-`count_plugin_skills` caller, stays.
- `discover_skills_for_plugin` (line 334) — called by `install_skills` at line 229. Last non-`count_plugin_skills` caller, stays.

This confirms the #32→#33 sequencing in the original issue: #33's work is replacing the `install_skills` call sites with core calls (likely by moving the `install_skills` manifest-load preamble into the core service, or by exposing the core `load_plugin_manifest` / `discover_skills_for_plugin` publicly for direct call-through). Out of scope here.

### `bindings.ts` regeneration

```bash
cargo test -p kiro-control-center --lib -- --ignored
```

Produces (specta-generated):

```typescript
export type SkillCount =
  | { state: "known"; count: number }
  | { state: "remote_not_counted" }
  | { state: "manifest_failed"; reason: SkippedReason };

export type PluginInfo = {
    name: string;
    description: string | null;
    skill_count: SkillCount;
    source_type: SourceType;
};
```

---

## Frontend (Svelte)

Current rendering at `crates/kiro-control-center/src/lib/components/BrowseTab.svelte:704`:

```svelte
<span class="text-[11px] text-kiro-subtle">{ap.plugin.skill_count}</span>
```

### Replacement

Helpers in the `<script>` block:

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

Template:

```svelte
<span
  class="text-[11px] {ap.plugin.skill_count.state === 'manifest_failed' ? 'text-kiro-warning' : 'text-kiro-subtle'}"
  title={skillCountTitle(ap.plugin.skill_count)}
>{skillCountLabel(ap.plugin.skill_count)}</span>
```

### Dependencies

- `formatSkippedReason(r: SkippedReason): string` — **new helper**, added as part of this PR in `BrowseTab.svelte`'s `<script>` block (or a shared `$lib` module if a second consumer appears). No analog exists today: `SkippedPlugin` carries a pre-rendered `reason: String` alongside its `kind: SkippedReason` (see `service/browse.rs:72–75`), so existing banner code (`BrowseTab.svelte:358`) uses the string directly. `SkillCount::ManifestFailed` deliberately does NOT duplicate the string field — that would create two representations of the same error and encourage substring-matching at the frontend. One small formatter replaces the need. Implementation: exhaustive `switch (r.kind)` over all eight `SkippedReason` variants (not just the six reachable from the count path — the formatter takes the full type and TypeScript's exhaustiveness check forces all cases), each returning a one-line sentence (e.g., `"plugin directory not found at {path}"`, `"plugin.json is malformed: {reason}"`).
- `text-kiro-warning` — confirm the token exists in the Tailwind config. If not, reuse whatever token `MarketplaceInfo.load_error` currently renders with (grep `BrowseTab.svelte` and the theme config). Implementation-time decision with a clear fallback.

### What does NOT change

- Plugin row is not greyed out / struck-through / italicized when `ManifestFailed` — the row remains clickable because the user might want a richer error surface in the detail panel. The `"!"` + tooltip is the only visual signal in the sidebar.

---

## Tests

All Rust tests in `crates/kiro-market-core/src/service/browse.rs` alongside existing bulk-listing tests. Frontend has no snapshot / component tests in this area today; TypeScript compilation via `npm run check` verifies the `SkillCount` switch is exhaustive.

### Layer 1: `count_skills_for_plugin` behavior (uses `tempfile`)

| Test | Setup | Expected outcome |
|---|---|---|
| `count_skills_for_plugin_returns_known_for_local_plugin` | tempdir w/ `plugin.json` + 3 skill dirs | `Known { count: 3 }` |
| `count_skills_for_plugin_returns_known_with_zero_when_no_skills` | tempdir w/ `plugin.json`, no skill dirs | `Known { count: 0 }` — distinguishes legitimate zero from failure |
| `count_skills_for_plugin_returns_known_when_manifest_absent` | tempdir, no `plugin.json`, default skill paths populated | `Known { count: N }` — absent manifest → defaults, not a failure |
| `count_skills_for_plugin_returns_remote_for_structured_source` | `PluginEntry { source: PluginSource::Structured(...) }` | `RemoteNotCounted` |
| `count_skills_for_plugin_returns_manifest_failed_on_missing_plugin_dir` | `PluginEntry` points at nonexistent path | `ManifestFailed { reason: DirectoryMissing { .. } }` |
| `count_skills_for_plugin_returns_manifest_failed_when_plugin_dir_is_a_file` | `plugin_dir` is a regular file, not a directory | `ManifestFailed { reason: NotADirectory { .. } }` |
| `count_skills_for_plugin_returns_manifest_failed_on_symlinked_plugin_dir` | `plugin_dir` is a symlink pointing at a real dir | `ManifestFailed { reason: SymlinkRefused { .. } }` |
| `count_skills_for_plugin_returns_manifest_failed_on_malformed_json` | tempdir w/ `plugin.json` containing `"{not json"` | `ManifestFailed { reason: InvalidManifest { .. } }` |

**Not tested** (reachable in principle, but not portably testable at this layer):

- **`DirectoryUnreadable`** — requires a `stat` failure that isn't `NotFound`. Producing this portably in a unit test needs platform-specific permission manipulation (e.g., `chmod 000` the parent on Unix, harder on Windows) and root-aware handling (root bypasses the permission check). The branch exists, it's covered by the `stat`-returns-Err match arm, and the `SkippedReason::from_plugin_error` classifier pins its shape. Leave unpinned here.
- **`ManifestReadFailed`** — requires `plugin.json` to stat successfully but fail on read. Same portability issue as `DirectoryUnreadable`. Branch exists and is structurally reachable; shape is pinned by the `from_plugin_error` classifier tests in `service/browse.rs:1772+`.

**Symlinked `plugin.json` specifically** — still treated as `Ok(None)` by `load_plugin_manifest` (not changed by this PR). Reaches `Known { count: N_default }` when `plugin_dir` itself is not a symlink but `plugin_dir/plugin.json` is. Add one test pinning this behavior to guard against regression:

```
count_skills_for_plugin_treats_symlinked_plugin_json_as_missing  →  Known { count: N_default }
```

### Layer 2: Serde wire-format pin

Following the existing `SkippedReason::from_plugin_error` pin pattern at `service/browse.rs:1772+`:

```rust
#[test]
fn skill_count_serde_wire_format_pins() {
    let json = serde_json::to_value(SkillCount::Known { count: 7 }).unwrap();
    assert_eq!(json, json!({"state": "known", "count": 7}));

    let json = serde_json::to_value(SkillCount::RemoteNotCounted).unwrap();
    assert_eq!(json, json!({"state": "remote_not_counted"}));

    let sc = SkillCount::ManifestFailed {
        reason: SkippedReason::InvalidManifest {
            path: PathBuf::from("/tmp/plug/plugin.json"),
            reason: "expected `}`".into(),
        },
    };
    let json = serde_json::to_value(sc).unwrap();
    assert_eq!(json["state"], "manifest_failed");
    assert_eq!(json["reason"]["kind"], "invalid_manifest");
}
```

Pins the tag names the frontend `switch` depends on. Breaks if someone changes `rename_all` or the tag key.

### Layer 3: Tauri `list_plugins` handler integration

One test in `src-tauri/src/commands/browse.rs` tests module, mirroring existing `list_plugins` test structure. Setup: a marketplace with three plugins — one local-working, one local-malformed, one remote. Assert each `PluginInfo.skill_count` variant appears in the returned list and carries the expected state.

### Branches NOT tested

`PluginError::NotFound` / `ManifestNotFound` → "unclassified" fallback. Two reasons:
1. `load_plugin_manifest` does not produce these variants, so the branch is unreachable in the real code path.
2. Testing it would require injecting a mock that returns those variants; the current API does not allow this.

The defensive fallback is belt-and-suspenders for future `PluginError` variants, not a testable branch today. A comment in the code records this.

---

## Out of scope

Explicit non-goals for this PR:

1. Moving `discover_skills_for_plugin` to core as a new public API — core already has its own copy. The Tauri-local one stays because `install_skills` still uses it; that's #33's job.
2. Deleting Tauri-local `load_plugin_manifest` — `install_skills` still calls it. #33.
3. Changing symlink handling for `plugin.json` — preserve current `Ok(None)` + `warn!` behavior.
4. Enriching `ManifestFailed` with the raw `PluginError` chain — `SkippedReason`'s existing `reason: String` already carries chain-preserved context from #30.
5. UI drill-down panel for failed plugins. The sidebar gets `"!"` + tooltip; a richer detail panel is separate UX work.

---

## Acceptance criteria

Restated from the issue with design decisions baked in:

- [ ] `MarketplaceService::count_skills_for_plugin(&PluginEntry, &Path) -> SkillCount` added in `kiro-market-core::service::browse` with doc comment, `#[must_use]`, and the three-outcome contract.
- [ ] `check_plugin_dir` helper added (module-private) performing the plugin-directory hardening pre-check (`DirectoryMissing` / `NotADirectory` / `SymlinkRefused` / `DirectoryUnreadable`). Consider factoring out of `collect_skills_for_plugin_into` if the extraction is clean; otherwise keep local and defer dedup.
- [ ] `SkillCount` enum added as `Serialize + specta::Type` (feature-gated) with `#[serde(tag = "state", rename_all = "snake_case")]` and `#[non_exhaustive]`. Inline doc comment lists reachable and unreachable `SkippedReason` variants.
- [ ] `PluginInfo.skill_count` changes from `u32` to `SkillCount`. `bindings.ts` regenerated.
- [ ] `list_plugins` Tauri handler calls the core method; Tauri-local `count_plugin_skills` deleted.
- [ ] Svelte `BrowseTab.svelte` renders the three states: number for `Known`, `"–"` for `RemoteNotCounted`, `"!"` with warning color + tooltip for `ManifestFailed`. Plugin name unchanged across states.
- [ ] Unit tests cover the Layer 1 table above. Serde wire-format pin added. One Tauri integration test covers the batch path end-to-end.
- [ ] `cargo test --workspace`, `cargo clippy --workspace --tests -- -D warnings`, `cargo fmt --all --check`, and `npm run check` (from `crates/kiro-control-center/`) all pass.
- [ ] No new `#[allow(...)]` directives introduced (per CLAUDE.md zero-tolerance rule).

---

## Related

- #30 — Phase 3 structural error/type design (landed). Established `SkippedReason` / `SkippedPlugin` + `from_plugin_error` projection pattern reused here.
- #33 — Tauri-side `load_plugin_manifest` dedup. Unblocked by this PR.

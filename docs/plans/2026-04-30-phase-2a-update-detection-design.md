# Phase 2a — Update Detection (Backend) — Design

> **Status:** design draft. Implementation plan to be written next via the `superpowers:writing-plans` skill once this design is approved. Phase 2a ships as a backend-only PR; Phase 2b (UI) follows as a separate PR consuming these types.

## Problem

PR #95 (Phase 1.5 type-safety hardening) closed the `MarketplaceName` / `PluginName` swap-arg footgun and shipped the A4 marketplace field on `InstallPluginResult`. Two gaps remain on the path to a complete plugin lifecycle:

1. **No way to detect that a plugin has updates.** A user installs `kiro-code-reviewer` v1.0 on Monday. The marketplace owner pushes v1.1 on Wednesday. The user has no signal — the Installed tab shows v1.0 forever, and the user only finds out by re-installing manually. The Phase 1 design (`2026-04-29-plugin-first-install-design.md`, lines 27-31, 185-233) sketched this work as Phase 2 and deferred the implementation plan until Phase 1 shipped.
2. **`RemovePluginResult` returns opaque counts.** After `kiro-market remove plugin@marketplace`, the toast says "Removed 3 skills, 1 steering file, 2 agents" — the user knows the magnitude but not which items. The Phase 1.5 design (decision A2, line 197) explicitly deferred the reshape to bundle here in Phase 2 because the wire-format change needs frontend consumption landing alongside.

Phase 2a covers the **backend** for both. Phase 2b will land the UI surfaces (Update indicator on plugin cards, Update button wiring, refreshed Remove toast) as a separate PR.

## Approach

**Detection signal: hybrid hash + version.** Compare each installed file's `source_hash` (recorded by Stage-1 content-hash work) against the marketplace cache file's current hash. If any differ, an update is available. The version-string comparison provides the human-readable label (`UpdateChangeSignal::VersionBumped` for `v1.0 → v1.1`, `UpdateChangeSignal::ContentChanged` when content drifted without a version bump). The hybrid catches author hygiene gaps — markdown-heavy plugins (most of them) often see content edits without manifest version increments.

**Per-plugin failure surfacing.** A scan covers many `(marketplace, plugin)` pairs. Some can fail independently (marketplace not in local cache, plugin removed from manifest, manifest malformed). The scan returns a `DetectUpdatesResult { updates, failures }` — plugins with no update available are absent from both vecs (the implicit "everything's fine" set). Matches the A-12 cascade pattern (`remove_plugin`) and the `installed_plugins.partial_load_warnings` pattern from Phase 1.

**`RemovePluginResult` reshape.** Restructure into per-content-type sub-results (`RemoveSkillsResult`, `RemoveSteeringResult`, `RemoveAgentsResult`), each with `removed: Vec<String>` and `failures: Vec<RemoveItemFailure>`. Symmetric with `InstallPluginResult`'s sub-result structure (`InstallSkillsResult`, etc.). Native companions fold into `RemoveAgentsResult` (matching install-side asymmetry).

**Action: existing `install_plugin` with `InstallMode::Force`.** No new Tauri command for the update *action*. Phase 1.5 already shipped the right signature (`install_plugin(project, &MarketplaceName, &PluginName, InstallMode::Force, accept_mcp)`); the FE just calls it when the user clicks "Update".

## User-locked decisions

These came out of the `2026-04-30` brainstorming conversation. Documented here so they don't drift during implementation:

1. **Scope: pure update flow.** Phase 2a is strictly update detection + A2 reshape. No bundled tech-debt cleanups (e.g., `From<CoreError>` exhaustiveness, cross-marketplace idempotency edge, HashMap-key newtypes). Rationale: tight PR scope is the explicit constraint; bundled work inflates review surface and dilutes focus. The deferred items remain captured for future phases.

2. **Implementation split: 2a backend + 2b UI.** Backend ships first as its own PR (no FE consumer yet); UI follows as a second PR. Mirrors the steering precedent (PR 83 backend + PR 92 UI) which kept reviews focused. Phase 1.5 already proved that `bindings.ts` regeneration without a FE consumer is fine.

3. **Failure handling: per-plugin failure surfacing.** Scan results are `DetectUpdatesResult { updates, failures }` — a separate failures vec, not Option-typed fields on the success type. Matches the A-12 cascade pattern. Toplevel `Result::Err` is reserved for "couldn't read tracking files at all"; per-marketplace / per-plugin failures land in `failures`. UI consumption is two `.find()` calls per card.

4. **A2 reshape: per-content-type sub-results.** `RemovePluginResult { skills, steering, agents }` symmetric with `InstallPluginResult`. Shared `RemoveItemFailure` type across the three sub-results — discriminator is the parent type, no `content_type: String` field. Native companions fold into `RemoveAgentsResult.removed` flat. **No `marketplace` / `plugin` echo fields** — caller already knows what they asked to remove. (Different from Phase 1.5 A4, which added `marketplace` to `InstallPluginResult` because that type lives in lists where self-identification matters.)

5. **Detection signal: hybrid hash + version.** Hash drives the "update available?" decision; version provides the human-readable label. `UpdateChangeSignal` is a tagged enum (`#[serde(tag = "kind", rename_all = "snake_case")]`) per the `ffi-enum-serde-tag` plan-lint gate. **Legacy fallback:** plugins installed before Stage-1 hash tracking (`source_hash: None` on tracking entries) fall back to version-only comparison; classified as `VersionBumped` if versions differ, otherwise no entry (content drift undetectable until next install).

## Phase 2a architecture

### Update detection

```rust
// crates/kiro-market-core/src/service/mod.rs

impl MarketplaceService {
    /// Scan installed plugins, comparing each tracking-file `version` and
    /// `source_hash` against the corresponding marketplace plugin manifest +
    /// source files in the local cache.
    ///
    /// "Update available" = either (a) at least one source hash differs from
    /// the corresponding `source_hash` in the tracking file, or (b) the
    /// marketplace plugin manifest's `version` is not byte-equal to the
    /// highest installed version across the three tracking files. Strict
    /// string inequality on versions, no semver — downgrades pushed by
    /// marketplace owners are surfaced.
    ///
    /// Reads from the local marketplace cache. Callers who want fresh data
    /// run `update_marketplaces` first (existing pattern).
    ///
    /// Per-plugin failures (marketplace gone from cache, plugin removed
    /// from manifest, manifest malformed) land in `failures`, not in
    /// `Result::Err`. Plugins with no update available are absent from
    /// both vecs.
    pub fn detect_plugin_updates(
        &self,
        project: &KiroProject,
    ) -> Result<DetectUpdatesResult, Error>;
}

#[derive(Debug, Default, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct DetectUpdatesResult {
    #[serde(default)]
    pub updates: Vec<PluginUpdateInfo>,
    #[serde(default)]
    pub failures: Vec<PluginUpdateFailure>,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct PluginUpdateInfo {
    pub marketplace: MarketplaceName,
    pub plugin: PluginName,
    pub installed_version: Option<String>,    // None when tracking files have no version
    pub available_version: Option<String>,    // None when manifest also lacks version
    pub change_signal: UpdateChangeSignal,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct PluginUpdateFailure {
    pub marketplace: MarketplaceName,
    pub plugin: PluginName,
    pub reason: String,    // error_full_chain at the boundary (per CLAUDE.md FFI rule)
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum UpdateChangeSignal {
    /// Manifest version string differs (with or without content hash diff).
    /// FE renders "Update v1.0 → v1.1".
    VersionBumped,
    /// Manifest version unchanged but at least one source-hash diff detected.
    /// FE renders "Content updated since install".
    ContentChanged,
}
```

**Detection logic** (per installed plugin, called from inside `detect_plugin_updates`):

1. Walk every installed-file entry across the 4 tracking files (skills, steering, agents, native_companions) for this `(marketplace, plugin)`.
2. For each entry with a non-None `source_hash`, recompute the hash of the corresponding marketplace-cache content and compare. **The hash function MUST match what the install path used** — `hash::hash_dir_tree` for skills (skills are directories per `project.rs:910,930`), `hash::hash_artifact` for steering and agents (single-file or rel-paths-list per `service/mod.rs:1669,1741`). Re-applying the install-path hash function ensures byte-equal comparisons; using a different hash function would produce false positives.
3. If any recomputed hash differs OR if the marketplace plugin manifest's `version` differs from the version of the most recently installed file across the three tracking files (matches `installed_plugins`'s `latest_install`-keyed selection at `project.rs::installed_plugins`) → there's an update; classify by version-string comparison (`VersionBumped` if versions differ, `ContentChanged` if versions match but hashes differ).
4. **Legacy fallback:** if `source_hash` is None for any tracked file (legacy install pre-Stage-1), fall back to version-string comparison only. If versions differ → `VersionBumped`. If versions match → no entry (content drift undetectable for legacy installs).
5. Per-plugin scan errors (cache file not found, hash computation failure, marketplace gone) get an entry in `failures` with `error_full_chain(&err)` as `reason`.

**Tauri command:** `detect_plugin_updates(project_path: String) -> Result<DetectUpdatesResult, CommandError>`. Service-consuming command — uses the `_impl(svc, project_path)` pattern per CLAUDE.md. Validates `project_path` via `validate_kiro_project_path`. No `MarketplaceName::new` / `PluginName::new` construction at the IPC boundary because the wrapper takes no name args (it scans the whole project). The returned `DetectUpdatesResult`'s newtype-typed fields enforce parse-don't-validate at the deserialization boundary.

**Result-only surface:** no toplevel `Result::Err` for "couldn't reach marketplace X" — that's a per-plugin failure in `failures`. The toplevel `Err` is reserved for "couldn't read tracking files at all" (project layout broken) — same shape as `installed_plugins` already has.

### A2 — `RemovePluginResult` reshape (per-content-type sub-results)

```rust
// crates/kiro-market-core/src/project.rs

#[derive(Debug, Default, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct RemovePluginResult {
    pub skills: RemoveSkillsResult,
    pub steering: RemoveSteeringResult,
    pub agents: RemoveAgentsResult,
}

#[derive(Debug, Default, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct RemoveSkillsResult {
    #[serde(default)]
    pub removed: Vec<String>,                   // skill names
    #[serde(default)]
    pub failures: Vec<RemoveItemFailure>,
}

#[derive(Debug, Default, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct RemoveSteeringResult {
    #[serde(default)]
    pub removed: Vec<String>,                   // rendered via Path::display() (matches existing failures.item shape)
    #[serde(default)]
    pub failures: Vec<RemoveItemFailure>,
}

#[derive(Debug, Default, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct RemoveAgentsResult {
    #[serde(default)]
    pub removed: Vec<String>,                   // translated agent names + native agent names + native companion paths, flat
    #[serde(default)]
    pub failures: Vec<RemoveItemFailure>,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct RemoveItemFailure {
    pub item: String,                           // skill/agent name or steering rel-path rendered
    pub error: String,                          // error_full_chain at the boundary
}
```

**Three deliberate decisions:**

1. **Shared `RemoveItemFailure` type** across the three sub-results rather than per-content-type failure types. The discriminator is the *parent type* (you know what content type failed because you read it from `result.skills.failures`). The existing `RemovePluginFailure.content_type: String` discriminator goes away because it's now expressed through the parent.
2. **Native companions fold into `RemoveAgentsResult.removed`** as a flat vec of mixed names and paths. Matches install-side asymmetry where native companions are agent-side artifacts. If FE later wants native-vs-translated breakdown, that's an additive field change.
3. **No `marketplace` / `plugin` echo fields** on `RemovePluginResult` — caller already knows what they asked to remove (different from Phase 1.5 A4 which added `marketplace` to `InstallPluginResult` because that type lives in lists where self-identification is needed).

**Cascade behavior unchanged.** The cascade still keeps making progress on remaining content types when one step fails (per A-15). Per-content-type failures land in the parent's `failures` vec; the cascade returns `RemovePluginResult` populated as far as it got. Native_companions failures land in `agents.failures` as part of the agent cascade step.

**`#[serde(default)]` on each vec field** for legacy-JSON tolerance, no `skip_serializing_if` (per A-25, tauri-specta unified-mode rejects it).

## Wire format / FFI

**New types crossing FFI:** `DetectUpdatesResult`, `PluginUpdateInfo`, `PluginUpdateFailure`, `UpdateChangeSignal`, restructured `RemovePluginResult`, `RemoveSkillsResult`, `RemoveSteeringResult`, `RemoveAgentsResult`, `RemoveItemFailure`. All use `#[cfg_attr(feature = "specta", derive(specta::Type))]`. All vec fields use `#[serde(default)]` no `skip_serializing_if` per A-25. The `UpdateChangeSignal` enum complies with `ffi-enum-serde-tag` (PR #91 plan-lint gate) via `#[serde(tag = "kind", rename_all = "snake_case")]`.

**Wire-format JSON-shape locks** (rstest cases) — one each for:
- `DetectUpdatesResult` default empty
- `DetectUpdatesResult` populated with one update + one failure
- `PluginUpdateInfo` with `change_signal: VersionBumped`
- `PluginUpdateInfo` with `change_signal: ContentChanged`
- Restructured `RemovePluginResult` default empty
- Restructured `RemovePluginResult` populated with removed + failure entries

Mirror PR #94's `install_plugin_result_json_shape_locks_default_subresults` pattern.

**Backward compat for `RemovePluginResult`:** A2 is a *breaking* wire-format change. The previous shape (`skills_removed: u32, steering_removed: u32, agents_removed: u32, failures: Vec<RemovePluginFailure>`) ships in PR #94 and #95 main today. **No compat shim** — the single-PR-per-phase pattern means consumers update at the same time as the producer. The Tauri command `remove_plugin` returns the new shape; bindings.ts regen propagates it; Phase 2b UI is the only consumer.

## Module map

| File | Status | Responsibility |
|---|---|---|
| `crates/kiro-market-core/src/service/mod.rs` | Modify | Add `detect_plugin_updates`, `DetectUpdatesResult`, `PluginUpdateInfo`, `PluginUpdateFailure`, `UpdateChangeSignal` |
| `crates/kiro-market-core/src/project.rs` | Modify | Reshape `RemovePluginResult`; add `RemoveSkillsResult` / `RemoveSteeringResult` / `RemoveAgentsResult` / `RemoveItemFailure`; update `KiroProject::remove_plugin` to populate the new sub-result types; remove the `RemovePluginFailure` type |
| `crates/kiro-market-core/src/cache.rs` | Modify (small, optional) | If detection-scan code lives outside the existing `hash::hash_dir_tree` / `hash::hash_artifact` call sites, add a helper that resolves a `(marketplace, plugin, content-type, rel_path)` tuple to the right cache path + hash function; otherwise reuse the existing hash module directly from `service/mod.rs::detect_plugin_updates`. |
| `crates/kiro-control-center/src-tauri/src/commands/plugins.rs` | Modify | Add `detect_plugin_updates` Tauri command (`_impl(svc, project_path)` shape per CLAUDE.md service-consuming pattern); `remove_plugin` wrapper unchanged but returns the new shape |
| `crates/kiro-control-center/src-tauri/src/lib.rs` | Modify | Register `detect_plugin_updates` in `collect_commands!` |
| `crates/kiro-control-center/src/lib/bindings.ts` | Regenerate | Auto-generated; emits new types + restructured `RemovePluginResult` |
| `crates/kiro-market-core/tests/integration_*.rs` | Modify | Update `RemovePluginResult`-asserting integration tests to the new shape |

**Frontend changes are zero in 2a** — `bindings.ts` regeneration will surface the type changes to TS, but no Svelte component consumes them yet (that's 2b). The intermediate state is the same shape Phase 1.5 already proved is fine.

## Testing strategy

### `detect_plugin_updates` (new)

- **Happy path no updates** — fixture marketplace with plugin v1.0 (matching content hashes), project with installed v1.0 → `DetectUpdatesResult { updates: [], failures: [] }`
- **Version bump detection** — fixture marketplace with plugin v1.1, project with installed v1.0 → `updates: [{ change_signal: VersionBumped, available_version: Some("1.1"), installed_version: Some("1.0") }]`
- **Content drift detection (no version bump)** — fixture marketplace where a skill file's content is mutated but `version` unchanged → `updates: [{ change_signal: ContentChanged }]`
- **Per-plugin failure surfacing** — fixture project with installed plugin from a marketplace not in cache → `updates: [], failures: [{ reason: "..." }]` (verify reason chains via `error_full_chain`)
- **Legacy fallback (source_hash: None)** — fixture project with `source_hash: None` on tracking entries (pre-Stage-1 install); marketplace at v1.1, installed at v1.0 → `updates: [{ change_signal: VersionBumped }]`; same versions → `updates: []` (content drift undetectable)
- **Mixed scenario** — project with 4 installed plugins: one no-update, one version-bumped, one content-drift, one whose marketplace is missing → asserts each plugin lands in the right vec with the right `change_signal`
- **Multiple content types in one plugin** — plugin with 3 skills + 2 steering + 1 agent; only one steering file has content drift → `updates: [{ change_signal: ContentChanged }]` (per-plugin granularity, not per-file)

### A2 reshape (existing tests modified)

- Existing `KiroProject::remove_plugin` rstests (PR #94's A-12 cascade tests at `project.rs:5567+`) update assertions from `result.skills_removed: u32` etc. to `result.skills.removed: Vec<String>`; verify named items appear
- Per-content-type failure landing — fixture where steering removal fails mid-cascade; assert failure lands in `result.steering.failures`, not `result.skills.failures`
- Native companions cascade-step failure → lands in `result.agents.failures`
- Empty-cascade case (no installed entries for plugin) → `RemovePluginResult { skills: default, steering: default, agents: default }`

### JSON-shape locks (new rstests in `service/mod.rs::tests` + `project.rs::tests`)

- `detect_updates_result_json_shape_locks_default_empty`
- `detect_updates_result_json_shape_with_one_update_and_one_failure`
- `plugin_update_info_json_shape_version_bumped`
- `plugin_update_info_json_shape_content_changed`
- `remove_plugin_result_json_shape_locks_default_empty`
- `remove_plugin_result_json_shape_with_populated_subresult` (skills.removed populated, steering.failures populated)

### Tauri command tests (`commands/plugins.rs::tests`)

- `detect_plugin_updates_impl_happy_path` — uses `temp_service` fixture; verifies the wrapper threads `MarketplaceService::detect_plugin_updates` correctly
- `detect_plugin_updates_impl_rejects_invalid_project_path` — `validate_kiro_project_path` rejection surfaces as `ErrorType::Validation`

### Plan-lint

`cargo xtask plan-lint` continues to enforce: gate-4-external-error-boundary (no external error leaks), no-unwrap-in-production, ffi-enum-serde-tag (covers the new `UpdateChangeSignal`).

## Out of scope

Documented here so they don't drift into the plan:

- **Phase 2b UI work** — Update indicator on plugin cards, Update button wiring, refreshed Remove toast. Separate PR; consumes the types defined here.
- **Hash memoization in marketplace cache** — perf optimization (compute source hashes once per `kiro-market update`, store in cache index, detection becomes metadata-only reads). Realistic projects probably take <100ms for a fresh-hash scan; can ship later if needed.
- **Per-content-type update granularity** — rejected for v1 per Phase 1 design's "Phase 2 alternatives" section; plugins are coherent bundles.
- **Auto-update / background polling** — rejected for v1 per Phase 1 design; security-sensitive (a malicious marketplace could push a hostile MCP server).
- **HashMap-key newtypes** (`SkillName`, `AgentName` for the keys) — deferred to a later phase. The I9 walkers + Phase 1.5 newtype `Deserialize` already give belt-and-suspenders.
- **`From<CoreError>` exhaustiveness fix** — PR #95 silent-failure-hunter S3 finding; pre-existing CLAUDE.md classifier rule violation. Sibling task.
- **Cross-marketplace same-plugin idempotency edge** (`project.rs:2937-2956`) — PR #95 marketplace-security-reviewer suggestion; pre-existing behavior. May matter for Phase 2b UI work but doesn't block 2a backend.
- **CSP `csp: null` hardening, TOCTOU lock-spanning, vitest setup** — different category (security/test infra); separate phases.
- **`RemovePluginResult` backward-compat shim** — A2 is a breaking wire-format change shipped together with its consumer; no shim.

## 5-Gates self-review

### Gate 1 — Grounding

**Real incident driving this work?** Yes, two:
1. PR #95's review process flagged that `detect_plugin_updates` would inherit the swap-arg footgun if Phase 1.5 hadn't closed it first — Phase 2a is the natural follow-on.
2. The "are people actually good about incrementing plugin versions?" concern surfaced during the 2026-04-30 brainstorm — drove the hybrid hash detection (decision #5). Markdown-heavy plugins (most of them) are exactly where authors under-bump, and version-only detection would silently miss those updates.

### Gate 2 — Threat Model

**Untrusted inputs:**

- **Tracking-file content** (`InstalledSkillMeta.marketplace`, `source_hash`, etc.) — already parse-validated at `serde_json::from_slice` via Phase 1.5's newtype `Deserialize` impls. No new untrusted parse points.
- **Marketplace plugin manifest fields** (`version`, file lists) — already validated by existing core parsers (`PluginManifest`, `RelativePath`); Phase 2a doesn't introduce new untrusted parse points.
- **Marketplace cache source files** — `fs::read` + xxhash. The cache is populated by `kiro-market update` from authenticated git operations; the contents are trusted at hash-time. The hash itself is xxhash (fast, non-cryptographic) — appropriate for change detection, not authentication.
- **`project_path` from Tauri FFI** — `validate_kiro_project_path` from PR 83 still applies; new `detect_plugin_updates` wrapper calls it.

### Gate 3 — Wire Format / FFI

**`UpdateChangeSignal` enum** uses `#[serde(tag = "kind", rename_all = "snake_case")]` — complies with `ffi-enum-serde-tag` plan-lint gate (PR #91). All new types use `#[cfg_attr(feature = "specta", derive(specta::Type))]`. All vec fields use `#[serde(default)]` no `skip_serializing_if` (A-25).

**JSON-shape rstests** (listed in Testing Strategy) lock the contract for new types. `DetectUpdatesResult` and the restructured `RemovePluginResult` default-empty + populated cases both pinned.

**Breaking change:** `RemovePluginResult`'s reshape is wire-format-breaking. No compat shim — single-PR-per-phase pattern means the consumer (Phase 2b UI) updates at the same time as the producer (this PR's `bindings.ts` regen). Phase 2b PR depends on this PR.

### Gate 4 — External Type Boundary

**No new external errors introduced** in `kiro-market-core`'s public API. The cache-read helper in `cache.rs` may surface `io::Error` on read failures — wrap in a typed variant per CLAUDE.md "map external errors at adapter boundary" recipe (`#[non_exhaustive]` enum + `reason: String` + `error_full_chain`). `cargo xtask plan-lint --gate gate-4-external-error-boundary` will catch any leak. The new `PluginUpdateFailure.reason: String` field is the wire-format projection of any per-plugin error chain.

### Gate 5 — Type Design

- **`UpdateChangeSignal` as enum** rather than two booleans (`version_bumped: bool, content_changed: bool`) — single source of truth, exhaustive matching enforced, FE switch statement is total.
- **`DetectUpdatesResult { updates, failures }` shape** distinguishes "data" from "failures" at the type level — the FE knows what kind of thing it's looking at without inspecting an `Option<error>` field.
- **Newtype-typed name fields** throughout (`MarketplaceName`, `PluginName`) — Phase 1.5's invariant carries through.
- **`Option<String>` for versions** distinguishes "no version field in manifest" from "matches" — sentinel-free.
- **Per-content-type sub-results** for `RemovePluginResult` — each sub-result is a self-contained unit (removed + failures), and the parent type discriminator removes the need for a stringly-typed `content_type: String` field.
- **No new validation newtypes introduced.** Existing newtypes (`MarketplaceName`, `PluginName`, `RelativePath`, `AgentName`) cover the parse-don't-validate boundaries.

## References

- `2026-04-29-plugin-first-install-design.md` — Phase 1 + Phase 2 sketch (post-1.5 refresh applied 2026-04-30)
- `2026-04-30-phase-1-5-type-safety-design.md` — Phase 1.5 design; A4 `marketplace` field, A2 deferral note (line 197 in design)
- `2026-04-30-phase-1-5-type-safety-plan-amendments.md` — 8 amendments (P1.5-1 through P1.5-8); especially A-25 (`skip_serializing_if` rejection on FFI types)
- `2026-04-29-plugin-first-install-plan-amendments.md` — Phase 1's amendments, especially A-12 (cascade orphan recovery), A-15 (cascade keeps making progress on partial failures), A-16 (marketplace-aware native_companions cleanup), A-25 (skip_serializing_if rule)
- `2026-04-23-stage1-content-hash-primitive-plan.md` — Stage 1 content-hash work that populated `source_hash` on tracking-file meta types (load-bearing for the hybrid detection signal)
- PR #94 — Phase 1 (plugin-first install)
- PR #95 — Phase 1.5 (type-safety hardening); convergent reviewer findings on swap-arg footgun closed
- `docs/plan-review-checklist.md` — 5-gates self-review used above
- CLAUDE.md — `validation.rs` newtype template, FFI rules (`error_full_chain`, no `skip_serializing_if` per A-25), `_impl(svc, ...)` Tauri command pattern, "map external errors at adapter boundary" recipe

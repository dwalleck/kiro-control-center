# Plugin-First Install — Design

> **Status:** Phase 1 shipped (PR #94, main merge `9ff4e7b`). Phase 1.5 type-safety hardening shipped (PR #95, main merge `a9f7b97`) — `MarketplaceName` / `PluginName` newtypes + A4 marketplace field on `InstallPluginResult`. Phase 2 plan deferred; sketch refreshed 2026-04-30 to reflect newtypes and bundle A2 (`RemovePluginResult` shape symmetry).

## Problem

PR 92 wired `install_plugin_steering` into the desktop UI, completing one slice of an end-to-end install path for steering files. Manual testing on `dwalleck/kiro-starter-kit` revealed two adjacent gaps that compose into one architectural issue:

1. **Steering installs are invisible after the fact.** The `Installed` side-menu view lists only skills (`commands.listInstalledSkills`); a successful steering install produces no observable state change in the UI. The user has no way to discover what they have installed, no way to remove it, and no signal that the install actually succeeded after the green banner fades.
2. **Plugin authors think in packages.** A plugin like `kiro-code-reviewer` ships steering + agents + skills as a coherent bundle — the agent uses the steering as context, the skills are tools the agent invokes. Installing one piece without the others imports a half-functional reviewer. The current UI lets the user install any one piece without the others, with no signal that the bundle is meant to be coherent.

The current UI treats **skills as the first-class object**, with steering as a recently-bolted-on side action and agents not yet wired at all. The data model in `kiro-market-core` already treats the three content types as peers (separate `installed-*.json` tracking files, separate install paths). The asymmetry is in the UI/orchestration layer above core, not in core itself.

## Approach

**Make plugins the first-class user-facing object** while keeping content-type-first separation in core. Two phases:

### Phase 1 — Plugin-first install + lifecycle

- A new Tauri-level coordinator `install_plugin(marketplace, plugin, force, project_path)` runs all three install paths (skills + steering + agents) in one call. Returns a unified `InstallPluginResult` aggregating the three sub-results.
- A new `list_installed_plugins(project_path)` Tauri command reads the three tracking files and groups by `(marketplace, plugin)` for the UI.
- A new `remove_plugin(marketplace, plugin, project_path)` cascades removal through the three tracking files plus on-disk files.
- `install_plugin_agents` Tauri command lands as part of this work (subsumes the previously-deferred "item 4" from the post-PR-83 follow-up list) — `MarketplaceService::install_plugin_agents` already exists in core; the wrapper has just never been wired.
- BrowseTab's primary surface becomes plugin cards, each with a single "Install" action (all-or-nothing per the user's decision below). The skill-grid view is preserved as a secondary path so power users can still pick subsets.
- InstalledTab's primary surface becomes installed-plugin rows showing the per-content-type breakdown, with "Remove" cascading through `remove_plugin`.

### Phase 2 — Updates

- Detection: compare `installed-*.json` `version` fields against current marketplace plugin manifest version. Plugin has an update available if **any** content from that plugin has a newer version in the marketplace.
- Surface: per-plugin "Update available" indicator on the plugin card.
- Action: `install_plugin` with `force=true` (effectively a re-install). No new Tauri command needed for the action; one new command for detection.

## User-locked decisions

These came out of the `2026-04-29` design conversation. Documented here so they don't drift during implementation:

1. **Customization granularity: all-or-nothing.** Phase 1 ships one "Install plugin" button per plugin that installs everything the plugin declares. No per-content-type checkboxes in v1. If users ask for subset-install, that becomes a Phase 1.1 follow-up. *Rationale:* matches plugin-author intent (plugins ship as coherent bundles); reduces decision-paralysis for new users; keeps the UI simpler.
2. **Update granularity: per-plugin (with per-content-type fallback documented for posterity).** When the marketplace plugin manifest moves from v1 to v2, the UI surfaces "Update available" at the plugin level and re-installs all of the plugin's content. The alternative — per-content-type updates — is rejected for v1 because it adds nuance most users don't need; if a plugin's steering changes, treating that as a plugin-level update keeps the bundle coherent. Per-content-type updates are sketched in **Phase 2 alternatives** below for future reference.

## Phase 1 architecture

### Backend coordinator

```rust
// crates/kiro-market-core/src/service/mod.rs
impl MarketplaceService {
    /// Install everything a plugin declares — skills, steering, agents
    /// — in a single call. Aggregates the three per-type results so a
    /// caller sees one coherent outcome for the whole plugin.
    ///
    /// Errors fail fast: if `resolve_plugin_install_context` errors,
    /// nothing is attempted. Per-content-type partial failures (e.g.
    /// one steering file fails to write) land in the corresponding
    /// sub-result (`InstallSteeringResult.failed`) without aborting
    /// the other content types. This matches each existing
    /// `install_plugin_*` function's individual error policy.
    pub fn install_plugin(
        &self,
        project: &KiroProject,
        marketplace: &str,
        plugin: &str,
        mode: InstallMode,
        accept_mcp: bool,
    ) -> Result<InstallPluginResult, Error>;
}

pub struct InstallPluginResult {
    pub plugin: String,
    pub version: Option<String>,
    /// `None` when the plugin declares no skills (or has them but none
    /// passed name filtering). `Some` even when all installs failed —
    /// the inner `failed` vec carries the detail.
    pub skills: Option<InstallSkillsResult>,
    pub steering: Option<InstallSteeringResult>,
    pub agents: Option<InstallAgentsResult>,
}
```

`Option<...>` distinguishes "this content type wasn't applicable" (no agents declared) from "this content type was attempted and yielded zero installs" (declared 1 file, that file failed).

### Tauri command surface

| Command | Status | Purpose |
|---|---|---|
| `install_skills` | Existing | Per-content install (kept for skill-grid path) |
| `install_plugin_steering` | Existing | Per-content install (kept) |
| `install_plugin_agents` | **New (Phase 1)** | Per-content install for agents (mirror steering) |
| `install_plugin` | **New (Phase 1)** | Coordinator |
| `list_installed_skills` | Existing | Per-content list (kept) |
| `list_installed_plugins` | **New (Phase 1)** | Aggregator (skills + steering + agents grouped by plugin) |
| `remove_skill` | Existing | Per-content remove (kept) |
| `remove_plugin` | **New (Phase 1)** | Cascade remove |

Per-content-type list/remove for steering and agents are *not* added in Phase 1. The plugin-level aggregator covers the InstalledTab use case; per-content-type remove can be added later when there's a UX driver for it.

### Plugin-aware tracking aggregation

`list_installed_plugins` reads all three `installed-*.json` files and groups by `(marketplace, plugin)`:

```rust
pub struct InstalledPluginInfo {
    pub marketplace: String,
    pub plugin: String,
    /// Highest version across the three content types. They may
    /// differ if the user installed at different times — the latest
    /// wins for the "what version do I have?" UX question.
    pub installed_version: Option<String>,
    pub skill_count: u32,
    pub steering_count: u32,
    pub agent_count: u32,
    /// Per-type detail for InstalledTab drill-down. Empty vecs when
    /// nothing of that type is installed.
    pub installed_skills: Vec<String>,        // skill names
    pub installed_steering: Vec<PathBuf>,     // relative paths under .kiro/steering/
    pub installed_agents: Vec<String>,        // agent names
    pub earliest_install: chrono::DateTime<chrono::Utc>,
    pub latest_install: chrono::DateTime<chrono::Utc>,
}
```

A plugin appears in the result if it has at least one installed entry across the three tracking files.

### remove_plugin cascade

```rust
// crates/kiro-market-core/src/project.rs
impl KiroProject {
    /// Remove every tracked entry from this plugin across all three
    /// content types. Unlinks the on-disk files, updates the three
    /// tracking files atomically (each `with_file_lock`'d), and
    /// returns aggregated counts.
    pub fn remove_plugin(
        &self,
        marketplace: &str,
        plugin: &str,
    ) -> Result<RemovePluginResult, Error>;
}

pub struct RemovePluginResult {
    pub skills_removed: u32,
    pub steering_removed: u32,
    pub agents_removed: u32,
}
```

If any sub-removal fails, return early with the error — `remove_plugin` is best-effort but doesn't try to roll back. The CLAUDE.md "tracking files are user-owned" note still applies: we never delete files we don't have a tracking entry for.

### Frontend changes

**BrowseTab restructure.** Today's structure:
- Filter bar
- Filter chips
- Skill grid (the primary surface)
- Bottom action bar (force toggle + "Install N selected" + "Install steering for X")

After Phase 1:
- Filter bar (unchanged)
- Filter chips (unchanged)
- **View toggle:** `[Plugins] [Skills]` (Plugins = default)
- **Plugins view:** plugin cards with Install button per card, content-count breakdown, description
- **Skills view:** today's skill grid (preserved for power users)
- Bottom action bar simplified: force toggle stays for the Skills view; per-plugin install lives on the cards

**InstalledTab restructure.** Today's structure:
- Single skills table

After Phase 1:
- **Plugins** section (primary): rows with plugin name, content-count breakdown, "Remove" button
- **All skills** section (collapsed by default): today's flat skills table for users who liked it

The cosmetic Force-reinstall checkbox issue you flagged earlier dissolves when the install action moves to the plugin card. The card's Install button can carry its own state (e.g. error → "Retry with overwrite" inline button) without a global toggle. The skill-grid Force toggle stays for the secondary path.

### Wire format / FFI shape

`InstallPluginResult` and `InstalledPluginInfo` cross the FFI. They embed existing types (`InstallSkillsResult`, `InstallSteeringResult`, `InstallAgentsResult`) which already have `Serialize + cfg_attr(specta::Type)` from earlier PRs. The new aggregate types follow the same pattern.

Per the `ffi-enum-serde-tag` plan-lint gate from PR 91: any new public enum that ships in these aggregates needs `#[serde(tag = "kind", rename_all = "snake_case")]`. None are introduced in Phase 1 (the result types are structs), but worth noting for any error variants that surface in the `RemovePluginResult` sibling types.

### Concurrent-install behavior

Today's UI gates concurrent skill installs with the global `installing: boolean` flag. After Phase 1, plugin installs are per-card and can run in parallel — analogous to PR 92's `pendingSteeringInstalls: SvelteSet<string>` keyed by `pluginKey(marketplace, plugin)`. Two plugins installing in parallel don't block each other. The skill-grid Install button keeps its existing single-flight behavior since the existing `installing` flag covers the same scope.

The banner-collision concern (code-reviewer finding from PR 92's review) is addressed by replacing the single global `installError`/`installMessage` with **per-card status**. Each plugin card carries its own status text. The bottom-bar banner collapses to "X / Y plugin installs in flight" or similar coarse summary when multiple are running.

## Phase 2 architecture (sketch)

> **Post-Phase-1.5 type-refresh (2026-04-30):** The signatures below were updated after PR #95 merged the `MarketplaceName` / `PluginName` newtypes. `PluginUpdateInfo`'s name fields use the newtypes (parse-don't-validate at deserialization, compile-enforced argument order across the install/update API surface). The update action uses `InstallMode::Force`, not the pre-1.5 `force: bool` shape. **A2** (`RemovePluginResult` shape symmetry — drop `_count: u32`, return `Vec<String>` per content type) was deferred from Phase 1.5 for bundling here, since the wire-format change needs frontend updates landing alongside.

### Update detection

```rust
// crates/kiro-market-core/src/service/mod.rs
impl MarketplaceService {
    /// Compare each installed-plugin entry's `version` field against
    /// the current marketplace manifest's plugin version. Returns the
    /// list of plugins that have an update available, with both the
    /// installed and available versions.
    ///
    /// "Update available" semantics: for the `(marketplace, plugin)`
    /// pair, if the marketplace's plugin manifest carries a `version`
    /// that does not equal the highest installed version across the
    /// three tracking files, return that plugin in the result. No
    /// semver comparison — strict string inequality is enough; we
    /// don't want to silently skip downgrades a marketplace owner
    /// intentionally pushed.
    pub fn detect_plugin_updates(
        &self,
        project: &KiroProject,
    ) -> Result<Vec<PluginUpdateInfo>, Error>;
}

pub struct PluginUpdateInfo {
    pub marketplace: MarketplaceName,
    pub plugin: PluginName,
    pub installed_version: Option<String>,
    pub available_version: Option<String>,
}
```

Tauri command `detect_plugin_updates(project_path)` returns the same shape. Per Phase 1.5's lesson (PR #95 I1), the wrapper does not need to construct newtypes from FE input — `detect_plugin_updates` reads from already-validated tracking files, and `PluginUpdateInfo`'s newtype-typed fields enforce parse-don't-validate at the deserialization boundary.

### Update action

No new Tauri command. The update action is `install_plugin(project, &marketplace, &plugin, InstallMode::Force, accept_mcp)` — the existing `MarketplaceService::install_plugin` signature (post-Phase-1.5) already takes the newtypes and `InstallMode::Force`. The frontend renders an "Update" button in place of "Install" when `detect_plugin_updates` shows the plugin needs one.

### A2 — `RemovePluginResult` shape symmetry (bundled here per Phase 1.5 design)

Phase 1's `RemovePluginResult { skills_removed: u32, steering_removed: u32, agents_removed: u32 }` is asymmetric with `InstallPluginResult` (which carries `Vec`-typed sub-results, not just counts). Phase 1.5 deferred this reshape so the wire-format change could land alongside the InstalledTab UI work that consumes it.

```rust
pub struct RemovePluginResult {
    pub marketplace: MarketplaceName,
    pub plugin: PluginName,
    pub skills_removed: Vec<String>,        // skill names
    pub steering_removed: Vec<PathBuf>,     // relative paths under .kiro/steering/
    pub agents_removed: Vec<String>,        // agent names
}
```

The frontend's "Removed N items" toast becomes "Removed: alpha, beta, gamma" — actionable detail without a follow-up navigation. Counts are recoverable via `.len()` on each Vec.

### Phase 2 alternatives (rejected for v1, documented for future)

**Per-content-type updates.** A plugin's steering files change but the skills don't. The user wants to update only the steering. To do this:
- `detect_plugin_updates` returns per-content-type version diffs, not just a single per-plugin diff.
- The "Update" button gets a dropdown: "Update everything / Update steering only / Update agents only" etc.
- Implementation: probably a new `update_plugin_content(marketplace, plugin, types: Vec<ContentType>)` Tauri command.

This is rejected for v1 because (a) the user's stated intent is "plugins are coherent bundles" and per-type updates fight that intent, and (b) the underlying core code is `force=true` per-type, so the surface is purely a UI concern that can be added later without changing the data model.

**Auto-update.** Background polling for updates and applying them without user action. Out of scope; security-sensitive (a malicious marketplace could push a version that adds a hostile MCP server). Always require an explicit user click.

## Out of scope (Phase 1 + Phase 2)

- Plugin uninstall confirmation dialog (just a button for now; cascade-remove is the contract).
- Plugin search beyond the existing skill filter input (the filter applies to skill names; a plugin-name filter is a Phase 1.1 polish).
- Per-content-type install customization (the rejected alternative above).
- Per-content-type list/remove Tauri commands (use the plugin-level aggregator + cascade for now).
- Plugin dependencies (some plugins might require others; deferred).
- Marketplace-level "install all plugins" or "update all plugins" batch actions.
- Plugin ratings / popularity / signing / publisher trust signals.
- Auto-update / background update polling.
- Plan-lint gate for "every plugin-shaped type that crosses FFI must specta::Type" — PR 91's `ffi-enum-serde-tag` gate covers enums; a struct-side gate isn't yet in scope but worth tracking.

## Testing strategy

- Each new Tauri command splits into wrapper + `_impl`; `_impl` is unit-tested directly against `service::test_support` fixtures, mirroring the PR 83 / PR 92 pattern.
- New core APIs (`MarketplaceService::install_plugin`, `KiroProject::remove_plugin`) get rstest cases against `temp_service` fixtures.
- Aggregation logic (`installed_plugins`) gets a fixture with mixed content-type tracking and asserts the grouping shape.
- Wire-format JSON shape locks for any new struct that crosses FFI (mirror `steering_warning_variants_json_shape` from PR 83).
- Frontend: Playwright e2e test in `tests/e2e/app.spec.ts` for the happy-path "click Install on plugin card → see counts in InstalledTab" flow.

## Module map

| File | Status | Responsibility |
|---|---|---|
| `crates/kiro-market-core/src/service/mod.rs` | Modify | Add `install_plugin`, `detect_plugin_updates`, `InstallPluginResult`, `PluginUpdateInfo` |
| `crates/kiro-market-core/src/project.rs` | Modify | Add `installed_plugins`, `remove_plugin`, `InstalledPluginInfo`, `RemovePluginResult` |
| `crates/kiro-control-center/src-tauri/src/commands/agents.rs` | **New** | `install_plugin_agents` Tauri command (mirror `commands/steering.rs`) |
| `crates/kiro-control-center/src-tauri/src/commands/plugins.rs` | **New** | `install_plugin`, `list_installed_plugins`, `remove_plugin`, `detect_plugin_updates` Tauri commands |
| `crates/kiro-control-center/src-tauri/src/commands/mod.rs` | Modify | Register new modules |
| `crates/kiro-control-center/src-tauri/src/lib.rs` | Modify | Add new commands to `invoke_handler!` |
| `crates/kiro-control-center/src/lib/components/BrowseTab.svelte` | Modify | Plugin cards, view toggle, simpler bottom bar |
| `crates/kiro-control-center/src/lib/components/InstalledTab.svelte` | Modify | Plugins-grouped view + collapsible flat skills |
| `crates/kiro-control-center/src/lib/components/PluginCard.svelte` | **New** | Reusable plugin card with Install/Update/Remove states |
| `crates/kiro-control-center/src/lib/bindings.ts` | Regenerate | New types via `cargo test ... -- --ignored` |
| `crates/kiro-control-center/tests/e2e/app.spec.ts` | Modify | Plugin-install happy-path test |

## 5-Gates self-review

### Gate 1 — Grounding

**Real incident driving this work?** Yes. PR 92 shipped steering install but the user manually testing on `kiro-starter-kit` immediately hit the "I installed something but the Installed tab shows nothing" gap. The plugin-first reframe came from the user's observation that plugins are designed as coherent bundles (a code-review agent without its steering isn't useful). Both grounded in real user feedback.

### Gate 2 — Threat Model

**Untrusted inputs:**
- Plugin manifest fields (`version`, content lists) — already validated by existing core parsers (`PluginManifest`, `RelativePath`, etc.); Phase 1 doesn't introduce new untrusted parse points.
- `project_path` from Tauri FFI — `validate_kiro_project_path` from PR 83 still applies; new Tauri commands must call it for any path-bearing operation. **Action item:** include `validate_kiro_project_path` calls in all new commands' `_impl` functions.
- Marketplace-supplied agent files in MCP-bearing plugins — `accept_mcp` flag flows through `install_plugin` to `install_plugin_agents`; the existing opt-in semantics must not be silently bypassed by the coordinator. **Action item:** plumb `accept_mcp` through `install_plugin` from the Tauri layer; default to `false` if the frontend doesn't send it.

### Gate 3 — Wire Format / FFI Shape

**New types crossing the FFI:**
- `InstallPluginResult` — struct, embeds existing `Option<InstallSkillsResult>` etc. Serialize + cfg_attr(specta::Type) on the new struct; the inner types already carry the asymmetric serde from PR 83 work. The `_Serialize`/`_Deserialize` TS noise from item 3a will propagate up to the new struct — accepted as known cosmetic debt.
- `InstalledPluginInfo` — struct of primitives + `Vec<String>`/`Vec<PathBuf>`. Clean Serialize.
- `PluginUpdateInfo` — struct of primitives + `Option<String>`. Clean.
- `RemovePluginResult` — struct of `u32`s. Clean.

**Action item:** explicit JSON-shape rstest case for `InstallPluginResult` to lock the aggregate shape (mirroring `steering_warning_variants_json_shape`).

### Gate 4 — External Type Boundary

No new external errors are introduced. The coordinator and aggregator wrap existing typed errors. `KiroProject::remove_plugin` reads three tracking files via existing `load_installed_*` paths — `serde_json::Error` is already mapped at those boundaries via `tracking_malformed`-style constructors. Phase 1 doesn't add new adapter-boundary work.

`cargo xtask plan-lint --gate gate-4-external-error-boundary` will cover this automatically.

### Gate 5 — Type Design

- `InstallPluginResult` uses `Option<T>` for "wasn't applicable" — distinguishes "plugin has no agents declared" (`None`) from "agents were attempted, all failed" (`Some` with empty `installed`). Encodes the distinction in the type rather than as a magic `len() == 0` check.
- `InstalledPluginInfo` — flat struct, all `pub` fields. Internal state, not parsed from external input. Acceptable for a DTO.
- `RemovePluginResult` — `u32` counts; saturating-cast is the existing pattern. `try_from` + warn-and-saturate via the existing `saturate_to_u32` helper.
- `PluginUpdateInfo` keeps `installed_version: Option<String>` and `available_version: Option<String>` rather than a single sentinel. `None` means "tracking file said no version" (which is legal for older installs).

**No new validation newtypes** are introduced; existing newtypes (`RelativePath`, `AgentName`) already cover the parse-don't-validate boundaries. **Action item:** if any new field accepts a path string from the FFI, it goes through `RelativePath::new` or equivalent at the boundary.

## References

- `2026-04-23-stage3-steering-import-plan.md` — original steering plan, established `_impl` pattern Phase 1 follows
- `2026-04-17-agent-tauri-wiring-plan.md` — pre-PR-83 agent wiring sketch (some content reusable for the Phase 1 agent Tauri command)
- `docs/plan-review-checklist.md` — 5-gates self-review used above
- PR 83 (steering install backend), PR 91 (plan-lint tag gate), PR 92 (steering install UI) — direct precedents

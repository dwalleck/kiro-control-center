# Phase 1.5 — Type-Safety Hardening — Design

> **Status:** design draft. Implementation plan to be written next via the `superpowers:writing-plans` skill once this design is approved.

## Problem

PR #94 (Phase 1, plugin-first install) shipped 23 commits and 909 tests, with an 8-reviewer aggregated review producing 3 Critical and 13 Important findings — all addressed before merge. Three of those findings converged on a single underlying weakness in `kiro-market-core`: **the `marketplace` and `plugin` strings are unstructured `&str` / `String`**, accepted by 7+ public API entry points in argument-order pairs that the type system cannot enforce.

Concretely:

```rust
pub fn remove_plugin(&self, marketplace: &str, plugin: &str) -> Result<RemovePluginResult, Error>;
pub fn remove_native_companions_for_plugin(&self, plugin: &str, marketplace: &str) -> Result<()>;
```

The two functions order their arguments differently. A caller that swaps them silently writes the marketplace string into the plugin slot. The compiler is mute. The same shape repeats across `install_plugin`, `install_skills`, `install_plugin_steering`, `install_plugin_agents`, `resolve_plugin_install_context`, plus the tracking-file meta types' `marketplace: String` and `plugin: String` fields.

Phase 1 worked around this surface in three places:

1. **I9 walkers** (`validate_tracking_skill_keys`, `validate_tracking_agent_keys`) walk HashMap keys after deserialization to reject path-traversal entries.
2. **I10 IPC validation** (`validate_name(marketplace)?` calls in every Tauri `_impl`) rejects malformed FE-supplied names at the boundary.
3. **A-12 / A-24 cascade orphan recovery** assumes the cascade can drop tracking entries it received valid pairs for — a `(marketplace, plugin)` swap would silently drop the wrong tracking row.

These are runtime gates. The type system gives no help. Phase 2 — update detection — would inherit the same surface (`detect_plugin_updates(marketplace, plugin)`), propagating the footgun.

## Approach

**Encode the marketplace/plugin string invariant in the type system.** Two newtypes — `MarketplaceName` and `PluginName` — replace `String` / `&str` at every internal boundary in `kiro-market-core`. Construction goes through a fallible `new` that routes to the existing `validate_name`. Tracking-file struct fields adopt the newtypes too, so `serde_json::from_slice` rejects malformed names at parse time (parse-don't-validate per CLAUDE.md template). `serde(transparent)` keeps the JSON wire format identical to today's strings — no migration of installed projects' tracking files.

The Tauri command surface keeps its `String` parameters (frontend callers naturally pass strings; specta-aliased newtypes don't enforce nominal types in TypeScript without branded patterns). The IPC `_impl`s construct the newtype early via `MarketplaceName::new(...)?`, replacing the I10 `validate_name(...)?` calls — same effective gate, but the resulting handle proves provenance for the rest of the function body.

This is mechanical, rippling work. Argument-swap bugs become compile errors. The I9 walkers stay (they validate HashMap *keys*, which Phase 1.5 doesn't touch); when a future phase newtypes the keys themselves, those walkers retire.

## User-locked decisions

These came out of the `2026-04-30` brainstorming conversation. Documented here so they don't drift during implementation:

1. **Phase 1.5 is pure polish/hardening.** Phase 2 (update detection) ships separately as its own feature PR after 1.5 lands. Rationale: type-design is *Phase 1 completeness* — the swap-arg footgun would propagate into Phase 2's `detect_plugin_updates(marketplace, plugin)` if not closed first. Phase 2 is a coherent feature with its own design surface; bundling dilutes review focus.

2. **Type-safety hardening is the anchor.** Of the four review-deferral themes (type-safety, security, UX polish, testing infra), type-safety is the highest-conviction work — three reviewers convergent on the swap-arg risk, and the CLAUDE.md template (`RelativePath` / `AgentName` precedent) is ready to apply. Security work (CSP, TOCTOU) is more disruptive and gets its own focused phase later. Testing infra is foundational but doesn't block Phase 2.

3. **Subset within type-safety: A1 + A4.** The full type-design bucket has four items:
   - **A1.** `MarketplaceName` / `PluginName` newtypes — the meat
   - **A2.** `RemovePluginResult` shape symmetry (drop `_count: u32`, return `Vec<String>`) — wire-format change with frontend ripples
   - **A3.** `InstallAgentsResult` dual-track collapse (`installed: Vec<String>` + `installed_native: Vec<...>`) — annotated as legacy-presenter scaffolding, low conviction
   - **A4.** `InstallPluginResult` add `marketplace` field — trivial, rides for free

   Phase 1.5 is **A1 + A4**. A2 defers to a UI-touching phase (likely bundled with Phase 2's "Update available" indicator work where the result-shape change can land alongside frontend updates). A3 defers indefinitely; the dual-track is documented tech debt, not a bug source.

4. **Naming: `*Name`, not `*Id`.** Matches the existing `AgentName` precedent in `validation.rs`. The strings ARE names (used in path joining, log output, tracking-file keys); `MarketplaceId` / `PluginId` would imply opaque identifiers which they aren't.

5. **No `Default` derive on the newtypes.** Verified by `grep`: `InstallPluginResult::default()` is never called anywhere — the two JSON-shape rstests construct the struct field-by-field, and `MarketplaceService::install_plugin` at `service/mod.rs:1992` does the same. Phase 1's implementer added `Default` to `InstallPluginResult`'s derive list but no consumer uses it. Phase 1.5 drops `Default` from `InstallPluginResult` and does NOT derive `Default` on the newtypes. Matches existing precedent (`RelativePath`, `AgentName`, `GitRef` deliberately don't derive `Default`); avoids the degenerate-state hazard (`MarketplaceName(String::new())` would be a value the type's own validator rejects); zero test churn. A future reader who tries `InstallPluginResult::default()` will see a compile error rather than silently construct a degenerate value.

## Phase 1.5 architecture

### New types

Two newtypes added to `crates/kiro-market-core/src/validation.rs` next to the existing `RelativePath` / `AgentName`:

```rust
/// Marketplace name as it appears in `marketplace.json` and tracking files.
/// Validated against the existing `validate_name` rules at construction:
/// non-empty, no NUL/control bytes, no path-traversal, no Windows-reserved names.
///
/// Deliberately does NOT derive `Default` — `MarketplaceName::default()` would
/// have to return `MarketplaceName(String::new())`, which `validate_name` rejects.
/// Matches the existing `RelativePath` / `AgentName` / `GitRef` precedent.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct MarketplaceName(String);

impl MarketplaceName {
    /// Construct after validation. Routes through [`validate_name`].
    pub fn new(s: impl Into<String>) -> Result<Self, ValidationError> {
        let s = s.into();
        validate_name(&s)?;
        Ok(MarketplaceName(s))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for MarketplaceName {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Self::new(s).map_err(serde::de::Error::custom)
    }
}

impl Display for MarketplaceName { /* delegates to inner */ }
impl AsRef<str> for MarketplaceName { /* delegates */ }
impl PartialEq<str> for MarketplaceName { /* for ergonomic comparisons */ }
impl PartialEq<&str> for MarketplaceName { /* same */ }
```

`PluginName` has the same shape. Both compile to a wire-format string, fail-loud at construction or deserialization, and prove provenance once instantiated.

### Propagation scope

| Surface | Today | Phase 1.5 | Reasoning |
|---|---|---|---|
| Tracking-file meta types (`InstalledSkillMeta.marketplace`, etc.) | `String` | `MarketplaceName` / `PluginName` | Parse-don't-validate at `serde_json::from_slice` — malformed entries reject at load |
| Core function signatures (`install_plugin`, `remove_plugin`, etc.) | `&str, &str` | `&MarketplaceName, &PluginName` | Compiler enforces argument order |
| Result type fields (`InstalledPluginInfo.marketplace`, `InstalledPluginInfo.plugin`, `InstallPluginResult.plugin`, new `InstallPluginResult.marketplace` from A4) | `String` | `MarketplaceName` / `PluginName` | Wire format stays string via `serde(transparent)` |
| Tauri command wrapper signatures (`install_plugin(marketplace: String, plugin: String, ...)`) | `String` | `String` (unchanged) | FE callers naturally pass strings |
| Tauri `_impl` interior | `validate_name(marketplace)?` then pass `&str` | `let marketplace = MarketplaceName::new(marketplace)?;` then pass `&marketplace` | Construction at IPC boundary supersedes I10 |
| Frontend (`bindings.ts`, BrowseTab, InstalledTab) | `string` | `string` (specta emits aliases but TS doesn't enforce nominal) | No frontend code change |

**HashMap keys** (`InstalledSkills.skills: HashMap<String, InstalledSkillMeta>` keyed by skill name) stay `String`. The I9 walkers (`validate_tracking_skill_keys`, `validate_tracking_agent_keys`) shipped in PR #94 continue to validate keys at load. Newtyping the keys themselves (`SkillName`, etc.) is a separate, larger scope and out of Phase 1.5.

### A4: `marketplace` field on `InstallPluginResult`

```rust
// `Default` derive dropped — never called; A1 newtypes don't derive Default.
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct InstallPluginResult {
    pub marketplace: MarketplaceName,   // NEW (A4)
    pub plugin: PluginName,             // changed from String per A1
    pub version: Option<String>,
    pub skills: InstallSkillsResult,
    pub steering: InstallSteeringResult,
    pub agents: InstallAgentsResult,
}
```

`MarketplaceService::install_plugin` already accepts `marketplace: &MarketplaceName` (post-A1); assign `marketplace.clone()` to the new field. Symmetric with `InstalledPluginInfo` which already carries both fields.

The frontend's install banner currently reads `"Plugin ${plugin}: ..."`. With `marketplace` available, BrowseTab could optionally render `"${marketplace}/${plugin}"` for disambiguation. Marked **optional polish** — install banners are per-card so the marketplace is visually adjacent already, and forcing a frontend touch into a backend-only PR would expand scope.

## Migration strategy

The newtype change ripples broadly. Implementation work in this order avoids intermediate-state breakage. Each step ends with `cargo test --workspace` green.

| Step | Files | Effect |
|---|---|---|
| 1. Define `MarketplaceName` + `PluginName` | `validation.rs` (+1 unit) | Adds types; nothing uses them yet |
| 2. Migrate tracking-file meta types | `project.rs` (`InstalledSkillMeta`, `InstalledSteeringMeta`, `InstalledAgentMeta`, `InstalledNativeCompanionsMeta`) | Parse-validated at load. `serde(transparent)` accepts existing JSON unchanged. Internal users update via `as_str()` / `Display` |
| 3. Migrate `installed_plugins` aggregator + `InstalledPluginInfo` field types | `project.rs` | Result types use newtypes |
| 4. Migrate `KiroProject` removal API (`remove_plugin`, `remove_skill`, `remove_steering_file`, `remove_agent`, `remove_native_companions_for_plugin`) | `project.rs` | Function signatures use `&MarketplaceName` / `&PluginName`. Internal callers updated |
| 5. Migrate `MarketplaceService` install API (`install_plugin`, `install_skills`, `install_plugin_steering`, `install_plugin_agents`, `resolve_plugin_install_context`) | `service/mod.rs`, `service/browse.rs` | Function signatures + `PluginInstallContext` field types. **A4 lands here** |
| 6. Update Tauri `_impl`s | `commands/{agents,plugins,steering,browse,installed}.rs` | Replace `validate_name(x)?` with `let x = MarketplaceName::new(x)?;`. Pass `&x` to core |
| 7. Regenerate `bindings.ts` | `crates/kiro-control-center/src/lib/bindings.ts` | Emits `MarketplaceName` / `PluginName` as TS string aliases |
| 8. Verify I10 cleanup | `commands/*.rs` | The `validate_name` calls from Phase 1's I10 are now redundant (step 6 replaced them) |

**Critical: I9 walkers stay.** `validate_tracking_skill_keys` and `validate_tracking_agent_keys` in `project.rs` validate HashMap *keys*, which step 2 doesn't touch. Don't delete them.

## Testing strategy

### New tests (`validation.rs::tests`)

- `marketplace_name_rejects_empty`
- `marketplace_name_rejects_traversal` (`..`, `../etc`, etc.)
- `marketplace_name_rejects_nul_byte`
- `marketplace_name_rejects_control_chars`
- `marketplace_name_rejects_windows_reserved` (`CON`, `NUL`, etc., per existing `validate_name` rules)
- `marketplace_name_round_trips_through_serde` (serialize → deserialize → equal)
- `marketplace_name_deserialize_rejects_malformed_via_parse_dont_validate` — feed malformed JSON to `serde_json::from_str::<MarketplaceName>(...)`, assert `Err`
- `plugin_name_*` — same set

### New tests (`project.rs::tests`)

- `installed_skills_deserialize_rejects_malformed_marketplace_in_meta` — locks the parse-don't-validate contract at the tracking-file boundary. The point of the newtype is that the I9 walkers become belt-and-suspenders rather than load-bearing.

### Wire-format JSON-shape locks (existing tests modified)

`install_plugin_result_json_shape_locks_default_subresults` and `install_plugin_result_json_shape_with_populated_subresult` (added in PR #94 per A-5) need updates to reflect the new `marketplace` field. Both should continue to assert that `marketplace` and `plugin` serialize as plain strings (the `serde(transparent)` contract).

### Existing tests modified

~30 sites currently passing `"mp"` / `"p"` literals to core APIs need `MarketplaceName::new("mp").expect("test fixture")`. Add helpers to `crates/kiro-market-core/src/service/test_support.rs`:

```rust
#[cfg(any(test, feature = "test-support"))]
pub fn mp(s: &str) -> MarketplaceName {
    MarketplaceName::new(s).expect("test marketplace name")
}

#[cfg(any(test, feature = "test-support"))]
pub fn pn(s: &str) -> PluginName {
    PluginName::new(s).expect("test plugin name")
}
```

Test fixtures become `mp("mp")` / `pn("p")` — readable and idiomatic.

## Out of scope

Documented here so they don't drift into the plan:

- **A2 `RemovePluginResult` shape symmetry** (drop `_count: u32`, return `Vec<String>` per content type) — defer to a UI-touching phase, likely bundled with Phase 2's "Update available" work where the result-shape change can land alongside frontend updates.
- **A3 `InstallAgentsResult` dual-track collapse** — defer indefinitely; the dual-track is annotated as legacy-presenter scaffolding, not a bug source.
- **HashMap-key newtypes** (`SkillName`, etc.) — keys stay `String`; I9 walkers continue to validate. Newtype-the-keys is bigger scope; leave for a future "tracking-file types" phase.
- **`*Error::NotInstalled { name: String }` variants** — stay `String`. Refactoring every error site to carry `PluginName` is high-churn for low gain since error messages are wire-projected to strings anyway.
- **Frontend nominal-type migration** — `bindings.ts` will emit `MarketplaceName` / `PluginName` as TS string aliases via `specta::Type`, but BrowseTab / InstalledTab code stays `string`-typed. TypeScript doesn't enforce nominal types ergonomically without branded patterns; the value of newtypes is at the Rust boundary.
- **Phase 2 (updates)** — `detect_plugin_updates`, "Update available" UI, `force=true` re-install wiring. Separate feature PR after 1.5 lands.
- **`csp: null` hardening, TOCTOU lock-spanning, vitest setup** — these were the bucket (b)/(c)/(d) themes not chosen for 1.5. Track for a future hardening phase.
- **A4 frontend banner uplift** — render `${marketplace}/${plugin}` in install banners. Optional polish; the marketplace is visually adjacent on the plugin card, so the banner doesn't strictly need it.

## 5-Gates self-review

### Gate 1 — Grounding

**Real incident driving this work?** Yes. PR #94's 8-reviewer aggregated review had **3 reviewers convergent** on the swap-arg footgun (`remove_plugin(marketplace: &str, plugin: &str)` accepts swapped args silently). The type-design reviewer flagged it as Critical with concrete reasoning: `remove_native_companions_for_plugin(plugin: &str, marketplace: &str)` already orders its arguments differently, the bug is latent, and there are 19+ sites where the same string-typed pair appears. The CLAUDE.md `validation.rs` template (`RelativePath` / `AgentName` precedent) is ready to apply.

### Gate 2 — Threat Model

**Untrusted inputs:**

- **Tracking-file content** (`InstalledSkillMeta.marketplace`, etc.) — A1 makes these parse-validated at `serde_json::from_slice`. Closes the gap PR #94's I9 walkers cover via post-load walking; the walkers remain belt-and-suspenders for HashMap keys (out of scope) but become structurally redundant for meta fields.
- **Tauri command FE strings** — `MarketplaceName::new(marketplace)?` at the IPC boundary supersedes PR #94's I10 `validate_name(marketplace)?` calls. Same effective gate, with a typed handle proving provenance for the rest of the function body.
- **Plugin manifest fields** — already validated by existing core parsers (`PluginManifest`, `RelativePath`, etc.); Phase 1.5 doesn't introduce new untrusted parse points.

### Gate 3 — Wire Format / FFI

**`serde(transparent)` keeps the wire format identical to today's strings.** JSON tracking files round-trip without migration. `bindings.ts` emits TypeScript string aliases via `specta::Type` — the TS shape is unchanged for consumers who don't opt into nominal typing.

**Existing JSON-shape rstest locks** continue to pin the contract. New tests assert the parse-don't-validate behavior at deserialization (a malformed `marketplace` in tracking JSON fails to deserialize, surfacing at `load_installed*()` rather than reaching the cascade).

**Action item:** verify the existing `InstallPluginResult` JSON-shape rstests (added in PR #94 per A-5: `install_plugin_result_json_shape_locks_default_subresults` and the populated-subresult companion) and the `installed_plugins_groups_skills_steering_agents_by_marketplace_plugin_pair` aggregator test continue to pass after the field-type change. The newtype fields under `serde(transparent)` should serialize byte-identically to the prior String fields.

### Gate 4 — External Type Boundary

**No new external errors introduced.** `MarketplaceName::new` returns `ValidationError` — typed, internal to `kiro-market-core`. The existing plan-lint gate `cargo xtask plan-lint --gate gate-4-external-error-boundary` will continue to pass.

### Gate 5 — Type Design

**This phase IS the Gate 5 work.** It encodes the "this is a validated marketplace/plugin name, not a raw string" invariant in the type system — making argument-order swap-safe at compile time, eliminating the swap-arg bug class entirely. The newtypes use a private inner field (per CLAUDE.md template) so degenerate values can't be constructed without going through `new`.

**No `Default` derive** on the newtypes — verified by grep that `InstallPluginResult::default()` is never called anywhere. Phase 1.5 drops `Default` from `InstallPluginResult` and does NOT derive `Default` on the newtypes. The newtypes can only be constructed via `new` (which validates) or `Deserialize` (which routes through `new`). Degenerate empty-string values are unrepresentable. Matches existing precedent for the validation newtypes in this crate.

## Module map

| File | Status | Responsibility |
|---|---|---|
| `crates/kiro-market-core/src/validation.rs` | Modify | Add `MarketplaceName`, `PluginName` |
| `crates/kiro-market-core/src/project.rs` | Modify | Migrate `Installed*Meta` field types, removal API signatures, aggregator |
| `crates/kiro-market-core/src/service/mod.rs` | Modify | Migrate install API signatures, `InstallPluginResult` field types (incl. A4 `marketplace`) |
| `crates/kiro-market-core/src/service/browse.rs` | Modify | Migrate `PluginInstallContext` field types, `resolve_plugin_install_context*` signatures |
| `crates/kiro-market-core/src/service/test_support.rs` | Modify | Add `mp(&str) -> MarketplaceName` and `pn(&str) -> PluginName` helpers |
| `crates/kiro-control-center/src-tauri/src/commands/{agents,plugins,steering,browse,installed}.rs` | Modify | Replace `validate_name(...)?` with `MarketplaceName::new(...)?` in `_impl`s |
| `crates/kiro-control-center/src/lib/bindings.ts` | Regenerate | Auto-generated; emits `MarketplaceName` / `PluginName` TS aliases |
| `crates/kiro-market-core/tests/integration_native_install.rs` | Modify | Test fixture sites — `mp("mp")` / `pn("p")` |
| `crates/kiro-market/src/commands/install.rs` | Modify | CLI signature update to construct newtypes from clap-parsed strings |

## References

- `2026-04-29-plugin-first-install-design.md` — Phase 1 design
- `2026-04-29-plugin-first-install-plan.md` — Phase 1 plan (10 tasks)
- `2026-04-29-plugin-first-install-plan-amendments.md` — 25 amendments (A-1 through A-25; A-24 closed in PR #94 per the I3 fix)
- PR #94 — Phase 1 implementation (23 commits, merged `9ff4e7b`)
- PR #94's 8-reviewer aggregated review (in conversation; convergent finding on swap-arg footgun across type-design / silent-failure / comment-analyzer)
- CLAUDE.md `validation.rs` template — `RelativePath`, `AgentName`, `GitRef` precedents
- `docs/plan-review-checklist.md` — 5-gates self-review used above

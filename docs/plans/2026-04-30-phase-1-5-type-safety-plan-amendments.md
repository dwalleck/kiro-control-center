# Phase 1.5 — Plan Amendments

> **Status:** plan-review pass per `docs/plan-review-checklist.md`.
> Fixes drift between `2026-04-30-phase-1-5-type-safety-plan.md` and the
> actual SHA at `9ff4e7b` (post-PR-#94 `main`). Format follows the
> precedent set by `2026-04-29-plugin-first-install-plan-amendments.md`.

5-gates pass via LSP-first discipline (per A-8 in the Phase 1
amendments doc). Gates 2, 3, 4, and most of Gate 5 pass clean — the
plan's design carried through. Gate 1 (grounding) found 4 specific
drift points; Gate 5 found one rationale gap worth documenting inline.

Each amendment cites the gate that fired, names the original plan
text, gives the amended text, and explains the rationale. Apply
during execution; they don't require re-opening the design conversation.

---

## P1.5-1 — Gate 1: Task 6 step 1 comparison `p.name == *plugin` won't compile

**Original (plan Task 6 step 1):**

```rust
let entry = plugin_entries
    .iter()
    .find(|p| p.name == *plugin)        // PluginName: PartialEq<str> via the &PluginEntry path
    .ok_or_else(|| ...)?;
```

**Drift.** `p.name: String` and `plugin: &PluginName`, so `*plugin: PluginName`. The expression `String == PluginName` requires `impl PartialEq<PluginName> for String` — which the plan's Task 1 doesn't define. The reverse comparison `*plugin == p.name` would require `impl PartialEq<String> for PluginName` — also not defined. Only `PartialEq<str>` and `PartialEq<&str>` are defined on the newtypes (matching the `AgentName` precedent).

The plan's parenthetical comment ("PluginName: PartialEq<str> via the &PluginEntry path") is incorrect — `PartialEq<str>` doesn't help here because the LHS is `String`, not `str`/`PluginName`.

**Amended.**

```rust
let entry = plugin_entries
    .iter()
    .find(|p| p.name == plugin.as_str())
    .ok_or_else(|| ...)?;
```

`String == &str` works via the standard library's `PartialEq<&str> for String`. No newtype-specific impls needed.

**Rationale.** Gate 1 — uncompilable code in a plan code block. The Phase 1 precedent (A-1, A-14, A-21) caught the same class of bug across multiple tasks; this is the Phase 1.5 equivalent. Verified by tracing: `plan.rs:Task 6 step 1` → newtype `PartialEq` impls in plan Task 1 → standard library trait coverage for `String`/`str`/`&str` interop.

---

## P1.5-2 — Gate 1: Task 6 step 1 `PluginError::NotFound` variant fields and name are wrong

**Original (plan Task 6 step 1):**

```rust
.ok_or_else(|| Error::Plugin(PluginError::PluginNotFound {
    plugin: plugin.as_str().to_string(),
}))?;
```

**Drift.** Verified by reading `crates/kiro-market-core/src/error.rs:63-65`:

```rust
/// The requested plugin was not found inside its marketplace.
#[error("plugin `{plugin}` not found in marketplace `{marketplace}`")]
NotFound { plugin: String, marketplace: String },
```

Two issues:

1. **Variant name is `NotFound`, not `PluginNotFound`.** The plan's name doesn't exist on `PluginError`.
2. **Variant has TWO fields, not one** — `plugin` AND `marketplace`. The plan's code only fills the first.

The current source at `service/browse.rs:756-761` (pre-migration) shows the right shape:

```rust
.ok_or_else(|| {
    Error::Plugin(PluginError::NotFound {
        plugin: plugin.to_owned(),
        marketplace: marketplace.to_owned(),
    })
})?;
```

**Amended.**

```rust
.ok_or_else(|| {
    Error::Plugin(PluginError::NotFound {
        plugin: plugin.as_str().to_string(),
        marketplace: marketplace.as_str().to_string(),
    })
})?;
```

(Combine with P1.5-1's fix to the `find` call.)

**Rationale.** Gate 1 grounding — verified the actual `PluginError` definition rather than guessing the variant shape. The plan's Task 6 step 1 code was written from memory of "PluginNotFound" semantics rather than the exact source. Two-line fix; same execution path.

---

## P1.5-3 — Gate 5: Task 1 step 3 needs an explicit rationale for the `Ord`/`PartialOrd` derive

**Original (plan Task 1 step 3 and Task 3 step 2):**

The plan says:

> Note: `MarketplaceName` and `PluginName` derive `Eq` and `Ord` is **not** derived. `BTreeMap` requires `Ord`. Add `Ord` and `PartialOrd` derives to both newtypes in `validation.rs`:
>
> ```rust
> #[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
> ```

**Drift.** This is a NEW pattern for `kiro-market-core` — the existing newtypes (`RelativePath`, `AgentName`, `GitRef`) deliberately don't derive `Ord`. The plan adds the derive for a real reason (BTreeMap key requirement at `installed_plugins`'s aggregator), but doesn't document the rationale next to the derive line itself. Future readers (and reviewers) will see the divergence and wonder why.

The Phase 1.5 design doc's **decision #5** documents the parallel "no `Default`" choice with a verbatim "matches existing precedent" note. The `Ord` derive deviation needs the same treatment — but in the OPPOSITE direction (we're adding a derive the others don't have).

**Amended.** Replace Task 1 step 3's derive line with the documented variant:

```rust
// Ord/PartialOrd derived for use as BTreeMap keys in `installed_plugins`'s
// aggregator (project.rs). Lexicographic ordering on the inner string is
// well-defined and semantically equivalent to String's ordering — no
// surprise for callers. Deviates from RelativePath/AgentName/GitRef
// (which don't derive Ord) because none of those types are used as
// map keys today.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(transparent)]
pub struct MarketplaceName(String);
```

(Same comment + derive on `PluginName`.)

Add a Task 1 test that locks the contract (in addition to the existing `marketplace_name_ord_is_lexicographic_on_inner` test the plan already includes):

```rust
#[test]
fn marketplace_name_ord_matches_inner_string_ord() {
    let a = MarketplaceName::new("alpha").expect("valid");
    let b = MarketplaceName::new("bravo").expect("valid");
    assert_eq!(
        a.cmp(&b),
        a.as_str().cmp(b.as_str()),
        "MarketplaceName::cmp must match inner String::cmp byte-for-byte"
    );
}
```

**Rationale.** Gate 5 type design — when introducing a derive that deviates from the existing newtype precedent, document the rationale at the derive site. The Phase 1 amendments precedent (A-25's wire-format-rule comment, the no-Default note in design.md) shows the project values "explain the choice next to the choice."

---

## P1.5-4 — Gate 1: Task 7 step 1 should cite Phase 1 I10's `From<ValidationError> for CommandError`

**Original (plan Task 7 step 1):**

```rust
let marketplace = kiro_market_core::validation::MarketplaceName::new(marketplace)?;
let plugin = kiro_market_core::validation::PluginName::new(plugin)?;
```

**Drift.** This relies on the `?` operator converting `ValidationError` → `CommandError`. The conversion exists at `crates/kiro-control-center/src-tauri/src/error.rs:113` (verified via LSP), but it was added by Phase 1 I10 as part of PR #94 — a reader who only sees the Phase 1.5 plan won't know whether the conversion exists. If they think it's missing, they may write a manual `.map_err(...)` or worse, propose adding the impl in this PR (creating a redundant edit).

**Amended.** Add a one-line note to Task 7 step 1, just before the `MarketplaceName::new` snippet:

> The `?` operator converts `ValidationError` → `CommandError` via the `From<ValidationError> for CommandError` impl in `error.rs:113`, added by PR #94's I10 work. No new conversion logic needed in this task.

**Rationale.** Gate 1 grounding for the implementer's mental model — the plan should make load-bearing dependencies explicit. The same discipline as Phase 1's plan, which cited specific PR-83 work that the new code relied on.

---

## P1.5-5 — Gate 1: Task 3 step 7 conflates two compile-state notions

**Original (plan Task 3 step 7):**

> Many compilation errors expected during sub-step iteration. Work through them one by one. By the end of step 6, the kiro-market-core crate should build clean.
>
> ```bash
> cargo test -p kiro-market-core 2>&1 | grep "test result:" | tail -5
> ```
>
> Expected: all `kiro-market-core` tests pass (the migration is internal; behavior is preserved).
>
> **The `kiro-control-center` and `kiro-market` crates will NOT compile yet** — they call functions whose signatures just changed. That's expected; Tasks 5–7 fix them. Do not attempt to compile the full workspace until Task 7.

**Drift.** The note is correct in intent but conflates two compile-state concepts:

1. **Within `kiro-market-core` itself** — Task 3 changes ripple through ~50 sites in `project.rs`. The crate either compiles end-to-end (all sites updated) or it doesn't. There is no "intermediate compiles, final doesn't" — `cargo build -p kiro-market-core` is a single atomic check per task.

2. **Across the workspace** — `kiro-control-center` and `kiro-market` call into the migrated APIs. After Task 3 they don't compile until Tasks 5–7 update their call sites. This is the genuine multi-task ripple.

The plan's note is technically right but reads as if even kiro-market-core might be in a half-broken state during the task. It isn't — the implementer must batch all of Task 3's sub-steps into one coherent commit (or use `as_str()` shims at compatibility points if they want intermediate commits).

**Amended.** Replace Task 3 step 7's note with:

> **Compile boundaries:**
>
> - Within `kiro-market-core` itself, **all of Task 3's sub-steps must land coherently**. The crate either compiles or it doesn't — there's no half-state. If the implementer wants intermediate commits within Task 3, use `meta.marketplace.as_str()` shims at internal call sites that haven't been migrated yet, then strip the shims in a follow-up step.
> - **Across the workspace**, `kiro-control-center` and `kiro-market` will NOT compile after Task 3 — their call sites still pass `&str` where `&MarketplaceName` is now required. That's expected; Tasks 4–7 fix the workspace ripple. Do not run `cargo build --workspace` until Task 7.

**Rationale.** Gate 1 — clarity for the implementer's expectations. The Phase 1 PR #94 implementer reports cited "intermediate non-compiling state" several times; making the boundary explicit avoids the same confusion in Phase 1.5.

---

## P1.5-6 — Implementation finding: Task 3's intra-crate ripple is wider than the plan anticipated

**Surfaced during.** Task 3 implementation (commit `9fbbfd9`). The plan's Task 3 step 7 said only the workspace crates (`kiro-control-center`, `kiro-market`) would fail to compile after Task 3, with `kiro-market-core` itself going green. In practice, three meta-construction sites *inside* `kiro-market-core` also have to be migrated in Task 3 (because the crate must compile end-to-end as a unit):

- `crates/kiro-market-core/src/service/mod.rs::install_skills` (around line 1110) — builds `InstalledSkillMeta` from `marketplace: &str, plugin: &str` parameters that won't be migrated until Task 5 changes `MarketplaceService::install_skills`'s signature.
- `crates/kiro-market-core/src/service/mod.rs::install_translated_agent` (around line 1519) — builds `InstalledAgentMeta` from `ctx: AgentInstallContext<'_>` whose `marketplace`/`plugin` fields stay `&str` until Task 5 migrates `AgentInstallContext`.
- `crates/kiro-market-core/src/service/mod.rs::install_one_native_agent` and `install_native_companions_for_plugin` (around lines 1691, 1771) — same shape, take `ctx: AgentInstallContext<'_>` whose fields are still `&str` pre-Task-5.

Plus one inside `project.rs` itself: `install_steering_file_locked` (around line 1878) holds a `SteeringInstallContext<'_>` whose fields stay `&str` until Task 4 migrates that struct.

**Why the plan missed it.** The plan correctly identified the *call sites that change in each task* but didn't trace the *meta-construction sites that consume those parameters*. An `InstalledSkillMeta` construction inside `install_skills` is downstream of `install_skills`'s parameter type — when we change the meta type's field type in Task 3, the construction site has to be updated even if its surrounding fn's signature doesn't change until Task 5.

**Mitigation chosen.** Task 3 added two `pub(crate) fn from_internal_unchecked` constructors on `MarketplaceName` and `PluginName` (mirroring the existing `RelativePath::from_internal_unchecked` precedent at `validation.rs:56` — `debug_assert!` validation, no panic in release, identical recipe). The four shim sites use them with explicit "Phase 1.5 Task 3 transient shim — Task 4/5 strips" comments. Task 4's `SteeringInstallContext` migration strips the `project.rs` shim; Task 5's `AgentInstallContext` + `MarketplaceService::install_skills` migrations strip the three `service/mod.rs` shims.

**Forward-looking rule.** When a plan migrates a struct field type (X), the plan should also enumerate the *construction sites* of that struct, not just the *call sites of the function holding it*. Construction sites and parameter sites are separate axes of ripple.

**Rationale.** Captured as audit trail per "forward motion + amendments" execution rule. The implementer correctly identified the intra-crate ripple, applied the existing `from_internal_unchecked` recipe (not a new pattern), and marked every shim site for Task 4/5 to strip. No design-doc revision needed.

---

## P1.5-7 — Implementation finding: `from_internal_unchecked` constructors added to both newtypes

**Surfaced during.** Task 3 implementation (commit `9fbbfd9`). Coupled to P1.5-6 — these are the constructors that enable the transient shim sites to compile without introducing `unwrap`/`expect` in production code.

**Drift.** The plan's Task 1 spec defined `MarketplaceName::new` and `PluginName::new` as the only public constructors. Task 3's intra-crate ripple (P1.5-6) needed a way to construct the newtypes from strings that *had already been validated upstream* (at the marketplace catalog parse layer) without re-running validation or returning a `Result`.

**Resolved by.** Adding `pub(crate) fn from_internal_unchecked(value: String) -> Self` to both newtypes:

```rust
/// See [`RelativePath::from_internal_unchecked`].
pub(crate) fn from_internal_unchecked(value: String) -> Self {
    debug_assert!(
        validate_name(&value).is_ok(),
        "MarketplaceName::from_internal_unchecked called with invalid name: {value:?}"
    );
    MarketplaceName(value)
}
```

This mirrors the **existing** `RelativePath::from_internal_unchecked` precedent at `crates/kiro-market-core/src/validation.rs:56`. It's not a new pattern — it's the codebase's established recipe for "internal post-validation construction without re-validating." `pub(crate)` keeps it off the public API; `debug_assert!` catches bugs in dev/test builds.

**Forward-looking expectation.** The four `from_internal_unchecked` call sites (3 in `service/mod.rs`, 1 in `project.rs`) are TRANSIENT — Tasks 4 and 5 strip them when the surrounding context structs (`SteeringInstallContext`, `AgentInstallContext`) and `MarketplaceService::install_skills` migrate to take the newtypes directly. The constructors themselves stay (they're general infrastructure matching `RelativePath`'s shape), but the call sites should disappear.

**Verification expectation.** After Task 5 lands, `grep -rn "from_internal_unchecked" crates/kiro-market-core/src/` should return only the `validation.rs` definition lines and the `RelativePath` precedent — no call sites in `service/mod.rs` (or anywhere else). If any survive past Task 5, that's a finding to capture.

**Site-count drift.** Tasks 1-4 deep review (`9a3b297..a80455c`) found that the actual call-site count is 5 (not 4): all in `service/mod.rs` after Task 4 stripped the `project.rs` site. Lines 1117/1120 (`install_skills`), 1523/1526 (`install_translated_agent`), 1696/1698 (`install_one_native_agent`), 1776/1778 (`install_native_companions_for_plugin`), 2005/2006 (`install_plugin`'s `SteeringInstallContext` construction added by Task 4). Task 5 strips all five.

**IMPORTANT — release-build exposure pre-Task-5.** Two of the five shim sites (`install_skills` 1117/1120 and `install_plugin` 2005/2006) are reached via PUBLIC `MarketplaceService` methods that still take `marketplace: &str, plugin: &str`. `from_internal_unchecked`'s `debug_assert!` is stripped in release builds, so a release-build CLI consumer that doesn't pre-validate could plant a malformed name into a tracking file. The Tauri direction is safe (callers do `validate_name(...)?` first); the CLI direction is not (`crates/kiro-market/src/commands/install.rs` parses but doesn't `validate_name`). Closing this is a Task 5 hard requirement — Task 5 must migrate `MarketplaceService::install_skills` and `install_plugin`'s public signatures to `&MarketplaceName, &PluginName` so the shim sites disappear by construction. No interim mitigation needed since Task 5 is the next dispatched task.

**Rationale.** Captured as audit trail. The implementer correctly invoked the existing precedent rather than introducing a new pattern, and the constructors will continue to be useful infrastructure even after the Phase 1.5 shim sites disappear (any future internal post-validation construction can reach for the same recipe).

---

## P1.5-8 — Implementation finding: Task 5 removed `from_internal_unchecked` from `MarketplaceName` / `PluginName` instead of keeping them as infrastructure

**Surfaced during.** Task 5 implementation. P1.5-7 said the `pub(crate) fn from_internal_unchecked` constructors on `MarketplaceName` and `PluginName` would stay as general infrastructure even after their Phase 1.5 shim call sites disappeared. In practice, leaving them as dead code triggered `dead_code` warnings, which `cargo clippy --tests -- -D warnings` (the project's pre-commit gate) treats as errors. CLAUDE.md's zero-tolerance policy on `#[allow(...)]` directives forecloses the warning-suppression escape hatch.

**Resolved by.** Removing both `pub(crate) fn from_internal_unchecked` definitions from `validation.rs` (along with their doc comments). The `RelativePath::from_internal_unchecked` precedent at `validation.rs:56` is unaffected — it has a real call site (`plugin.rs:133` via `DiscoveredPlugin::as_relative_path`) so it stays.

**Forward-looking expectation.** If a future task needs to construct a `MarketplaceName` or `PluginName` from an already-validated string without re-running validation, re-adding the constructor is a 7-line edit and the `RelativePath` precedent is right there. The deletion is reversible. The decision rule going forward: a `pub(crate) fn from_internal_unchecked` on a newtype that has no callers is a code smell, not infrastructure.

**Asymmetry left in place — `AgentError::PathOwnedByOtherPlugin.owner: String`.** Task 4 migrated `SteeringError::PathOwnedByOtherPlugin.owner` to `PluginName`. The sibling variant `AgentError::PathOwnedByOtherPlugin.owner` (at `error.rs:332`) still carries `String`. The construction site at `project.rs:2966` reads from a `&String` HashMap key — the in-design choice was to keep `HashMap<String, _>` keys as `String` rather than `PluginName`. Migrating just the error variant would either (a) require adding a NEW `from_internal_unchecked` shim site (defeating Task 5's purpose) or (b) require migrating the HashMap key (out of scope per the design doc). Task 5 leaves the asymmetry in place. If a follow-up wants to close it, the shape is "migrate the HashMap key first, then the variant." Logged here so the asymmetry is auditable rather than accidental.

**Rationale.** Captured as audit trail per "forward motion + amendments" execution rule. The deletion is the right call given the project's clippy posture; the forward-looking note keeps the door open for re-introduction if a future need actually materializes.

---

## Gates not flagged

- **Gate 2 (Threat Model)** — pass. The plan migrates existing surface; no new untrusted byte sources. The newtype's `Deserialize` adds defensive validation at parse time, strengthening the existing trust model. PR #94's I9 walkers + the new newtype `Deserialize` gives belt-and-suspenders for tracking-file content.
- **Gate 3 (Wire Format / FFI Shape)** — pass. `serde(transparent)` keeps wire format byte-identical. The plan's Task 1 step 1 includes round-trip and transparent-serialization tests that lock the contract. Existing JSON files round-trip through the migrated meta types without migration.
- **Gate 4 (External Type Boundary)** — pass. No new `pub` enum variants introduced. `MarketplaceName::new` returns `ValidationError` (internal type with no external-error fields). `cargo xtask plan-lint --gate gate-4-external-error-boundary` is expected to pass during Task 8 step 4.
- **Gate 5 (Type Design)** — mostly pass. Parse-don't-validate ✅; specta cfg-attr on validation newtypes ✅; classifier exhaustiveness N/A; classifier idempotent-payload rule N/A; enum-vs-boolean-pair N/A. The one finding (P1.5-3 above) is rationale documentation, not a substantive gap.

## Summary of changes

- **P1.5-1**: Task 6 step 1 comparison `p.name == *plugin` → `p.name == plugin.as_str()`. Compiles.
- **P1.5-2**: Task 6 step 1 `PluginError::PluginNotFound { plugin }` → `PluginError::NotFound { plugin, marketplace }`. Matches actual variant shape.
- **P1.5-3**: Add inline rationale comment on `Ord/PartialOrd` derive, plus a regression test asserting cmp matches inner String cmp.
- **P1.5-4**: One-line note in Task 7 step 1 citing Phase 1 I10's `From<ValidationError> for CommandError`.
- **P1.5-5**: Tighter wording in Task 3 step 7 separating crate-level vs workspace-level compile boundaries.
- **P1.5-6** (implementation finding): Task 3's intra-crate ripple wider than plan anticipated; 4 transient shim sites added with `from_internal_unchecked` per P1.5-7.
- **P1.5-7** (implementation finding): `pub(crate) fn from_internal_unchecked` added to `MarketplaceName` and `PluginName` mirroring the existing `RelativePath::from_internal_unchecked` precedent. Tasks 4-5 strip the call sites; the constructors themselves stay as infrastructure.
- **P1.5-8** (implementation finding): Task 5 removed the `from_internal_unchecked` constructors (no remaining callers triggered `dead_code`; CLAUDE.md's no-`#[allow]` rule blocked suppression). Reversible if a future task needs them. Also documents the deliberate `AgentError::PathOwnedByOtherPlugin.owner: String` asymmetry that Task 5 chose to leave in place.

No design-doc revisions required. The amendments are execution-time corrections; the architecture in `2026-04-30-phase-1-5-type-safety-design.md` stands as written.

## References

- `docs/plan-review-checklist.md` — the 5 gates this pass applied
- `2026-04-29-plugin-first-install-plan-amendments.md` — Phase 1's amendments doc, format precedent (especially A-1 / A-14 / A-21 for compile-error class catches)
- Source SHA at review time: `9ff4e7b` (post-PR-#94 `main`)

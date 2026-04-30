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

No design-doc revisions required. The amendments are execution-time corrections; the architecture in `2026-04-30-phase-1-5-type-safety-design.md` stands as written.

## References

- `docs/plan-review-checklist.md` — the 5 gates this pass applied
- `2026-04-29-plugin-first-install-plan-amendments.md` — Phase 1's amendments doc, format precedent (especially A-1 / A-14 / A-21 for compile-error class catches)
- Source SHA at review time: `9ff4e7b` (post-PR-#94 `main`)

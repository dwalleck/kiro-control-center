# Phase 1.5 — Type-Safety Hardening — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace `marketplace: &str` / `plugin: &str` with validated `MarketplaceName` / `PluginName` newtypes across the install / remove / aggregate API surface in `kiro-market-core`, and add the missing `marketplace` field to `InstallPluginResult` (A4) so the wire format is symmetric with `InstalledPluginInfo`.

**Architecture:** Newtypes are defined in `validation.rs` next to the existing `RelativePath` / `AgentName` precedent — `serde(transparent)` for byte-identical JSON, `Deserialize` routed through the fallible `new` for parse-don't-validate at tracking-file load. They flow through `kiro-market-core` API signatures, tracking-file struct fields, and result types; Tauri command wrappers stay `String`-typed (constructing newtypes early in `_impl`) and the frontend stays `string`-typed. The migration is mechanical but rippling — argument-swap bugs become compile errors.

**Tech Stack:** Rust edition 2024, `thiserror`, `serde` with `transparent`, `specta::Type` cfg-attr, `rstest` for tests, `cargo xtask plan-lint` for the project's structural gates.

**Companion design doc:** `2026-04-30-phase-1-5-type-safety-design.md`

---

## File structure

| File | Status | Responsibility |
|---|---|---|
| `crates/kiro-market-core/src/validation.rs` | Modify | Add `MarketplaceName`, `PluginName` (model after `AgentName`) |
| `crates/kiro-market-core/src/service/test_support.rs` | Modify | Add `mp(&str) -> MarketplaceName` and `pn(&str) -> PluginName` helpers |
| `crates/kiro-market-core/src/project.rs` | Modify | Migrate 4 tracking-file meta types, `InstalledPluginInfo`, `installed_plugins` aggregator, `KiroProject` removal/install API, free helpers |
| `crates/kiro-market-core/src/steering/types.rs` | Modify | Migrate `SteeringInstallContext` and `SteeringError::PathOwnedByOtherPlugin.owner` |
| `crates/kiro-market-core/src/service/mod.rs` | Modify | Migrate `AgentInstallContext`, `InstallPluginResult` (incl. A4 `marketplace` field), `MarketplaceService` install API, the install free fns |
| `crates/kiro-market-core/src/service/browse.rs` | Modify | Migrate `MarketplaceService::resolve_plugin_install_context*` |
| `crates/kiro-market-core/tests/integration_native_install.rs` | Modify | Update test fixtures |
| `crates/kiro-market/src/commands/install.rs` | Modify | CLI installs construct newtypes from clap-parsed strings |
| `crates/kiro-control-center/src-tauri/src/commands/agents.rs` | Modify | Replace `validate_name(...)?` with `MarketplaceName::new(...)?` |
| `crates/kiro-control-center/src-tauri/src/commands/plugins.rs` | Modify | Same |
| `crates/kiro-control-center/src-tauri/src/commands/steering.rs` | Modify | Same |
| `crates/kiro-control-center/src-tauri/src/commands/browse.rs` | Modify | Same |
| `crates/kiro-control-center/src-tauri/src/commands/installed.rs` | Modify | `remove_skill` constructs `PluginName` (via `validate_name` removal) — actually no, `remove_skill` takes a skill name not a plugin name; check whether anything in `installed.rs` needs migration |
| `crates/kiro-control-center/src/lib/bindings.ts` | Regenerate | Auto-generated; emits `MarketplaceName` / `PluginName` TS aliases |

---

## Task 1: Define `MarketplaceName` and `PluginName` newtypes

**Files:**
- Modify: `crates/kiro-market-core/src/validation.rs` — append after `AgentName`'s test module
- Test: same file's `tests` module

This task is self-contained: it adds two new types and their tests. Nothing in the rest of the workspace uses them yet, so the workspace stays compile-green throughout.

- [ ] **Step 1: Write failing tests**

Open `crates/kiro-market-core/src/validation.rs` and append to `mod tests`:

```rust
    // ──── MarketplaceName ────────────────────────────────────────────────

    #[test]
    fn marketplace_name_new_accepts_valid() {
        let name = MarketplaceName::new("kiro-starter-kit").expect("valid");
        assert_eq!(name.as_str(), "kiro-starter-kit");
    }

    #[test]
    fn marketplace_name_new_rejects_empty() {
        assert!(MarketplaceName::new("").is_err());
    }

    #[test]
    fn marketplace_name_new_rejects_traversal() {
        assert!(MarketplaceName::new("../etc").is_err());
        assert!(MarketplaceName::new("..").is_err());
    }

    #[test]
    fn marketplace_name_new_rejects_nul_byte() {
        assert!(MarketplaceName::new("foo\0bar").is_err());
    }

    #[test]
    fn marketplace_name_partial_eq_against_str_and_ref_str() {
        let name = MarketplaceName::new("mp").expect("valid");
        assert_eq!(name, *"mp");
        assert_eq!(name, "mp");
    }

    #[test]
    fn marketplace_name_accessors_round_trip() {
        let name = MarketplaceName::new("mp").expect("valid");
        let s = name.clone().into_inner();
        assert_eq!(s, "mp");
        assert_eq!(name.as_str(), "mp");
    }

    #[derive(serde::Deserialize)]
    struct MarketplaceNameWrapper {
        name: MarketplaceName,
    }

    #[test]
    fn deserialize_marketplace_name_accepts_valid() {
        let w: MarketplaceNameWrapper =
            serde_json::from_str(r#"{"name":"kiro-starter-kit"}"#).expect("valid");
        assert_eq!(w.name.as_str(), "kiro-starter-kit");
    }

    #[test]
    fn deserialize_marketplace_name_rejects_traversal() {
        let result: Result<MarketplaceNameWrapper, _> =
            serde_json::from_str(r#"{"name":"../etc"}"#);
        assert!(result.is_err());
    }

    #[test]
    fn deserialize_marketplace_name_rejects_empty() {
        let result: Result<MarketplaceNameWrapper, _> =
            serde_json::from_str(r#"{"name":""}"#);
        assert!(result.is_err());
    }

    #[test]
    fn serialize_marketplace_name_is_transparent_string() {
        let name = MarketplaceName::new("mp").expect("valid");
        let json = serde_json::to_string(&name).expect("serialize");
        assert_eq!(json, r#""mp""#);
    }

    #[test]
    fn marketplace_name_round_trips_through_serde_json() {
        let name = MarketplaceName::new("kiro-starter-kit").expect("valid");
        let json = serde_json::to_string(&name).expect("serialize");
        let parsed: MarketplaceName = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed, name);
    }

    // ──── PluginName ────────────────────────────────────────────────────

    #[test]
    fn plugin_name_new_accepts_valid() {
        let name = PluginName::new("kiro-code-reviewer").expect("valid");
        assert_eq!(name.as_str(), "kiro-code-reviewer");
    }

    #[test]
    fn plugin_name_new_rejects_empty() {
        assert!(PluginName::new("").is_err());
    }

    #[test]
    fn plugin_name_new_rejects_traversal() {
        assert!(PluginName::new("../etc").is_err());
    }

    #[test]
    fn plugin_name_new_rejects_nul_byte() {
        assert!(PluginName::new("foo\0bar").is_err());
    }

    #[test]
    fn plugin_name_partial_eq_against_str_and_ref_str() {
        let name = PluginName::new("p").expect("valid");
        assert_eq!(name, *"p");
        assert_eq!(name, "p");
    }

    #[derive(serde::Deserialize)]
    struct PluginNameWrapper {
        name: PluginName,
    }

    #[test]
    fn deserialize_plugin_name_rejects_traversal() {
        let result: Result<PluginNameWrapper, _> =
            serde_json::from_str(r#"{"name":"../etc"}"#);
        assert!(result.is_err());
    }

    #[test]
    fn serialize_plugin_name_is_transparent_string() {
        let name = PluginName::new("p").expect("valid");
        let json = serde_json::to_string(&name).expect("serialize");
        assert_eq!(json, r#""p""#);
    }
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cd /home/dwalleck/repos/kiro-marketplace-cli-phase-1-5
cargo test -p kiro-market-core marketplace_name 2>&1 | tail -10
```

Expected: FAIL with `error[E0433]: cannot find type 'MarketplaceName' in this scope`.

- [ ] **Step 3: Add the types**

In `validation.rs`, after the `impl Deserialize<'de> for AgentName` block (around line 213) and before `WINDOWS_RESERVED_NAMES` (line 216), insert:

```rust
/// Validated marketplace name. Routes through [`validate_name`] at construction
/// — non-empty, no NUL/control bytes, no path-traversal, no Windows-reserved
/// names. The `serde(transparent)` representation keeps the JSON wire format
/// byte-identical to a plain string; `Deserialize` is routed through `new` so
/// `serde_json::from_slice` rejects malformed names at parse time.
///
/// Deliberately does NOT derive `Default` — `MarketplaceName::default()` would
/// return `MarketplaceName(String::new())` which `validate_name` rejects.
/// Matches the existing `RelativePath` / `AgentName` / `GitRef` precedent.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(transparent)]
pub struct MarketplaceName(String);

impl MarketplaceName {
    /// Construct after validation. Returns [`ValidationError`] on rejection.
    pub fn new(value: impl Into<String>) -> Result<Self, ValidationError> {
        let value = value.into();
        validate_name(&value)?;
        Ok(MarketplaceName(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_inner(self) -> String {
        self.0
    }
}

impl TryFrom<&str> for MarketplaceName {
    type Error = ValidationError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl TryFrom<String> for MarketplaceName {
    type Error = ValidationError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl AsRef<str> for MarketplaceName {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for MarketplaceName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.0, f)
    }
}

impl PartialEq<str> for MarketplaceName {
    fn eq(&self, other: &str) -> bool {
        self.0 == other
    }
}

impl PartialEq<&str> for MarketplaceName {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

impl<'de> Deserialize<'de> for MarketplaceName {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

/// Validated plugin name. Same shape as [`MarketplaceName`]; see that type's
/// documentation for the construction and serde contract.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(transparent)]
pub struct PluginName(String);

impl PluginName {
    pub fn new(value: impl Into<String>) -> Result<Self, ValidationError> {
        let value = value.into();
        validate_name(&value)?;
        Ok(PluginName(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_inner(self) -> String {
        self.0
    }
}

impl TryFrom<&str> for PluginName {
    type Error = ValidationError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl TryFrom<String> for PluginName {
    type Error = ValidationError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl AsRef<str> for PluginName {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for PluginName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.0, f)
    }
}

impl PartialEq<str> for PluginName {
    fn eq(&self, other: &str) -> bool {
        self.0 == other
    }
}

impl PartialEq<&str> for PluginName {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

impl<'de> Deserialize<'de> for PluginName {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

```bash
cargo test -p kiro-market-core marketplace_name plugin_name 2>&1 | tail -15
```

Expected: ~16 tests pass.

- [ ] **Step 5: Lint and commit**

```bash
cargo clippy -p kiro-market-core --tests -- -D warnings
cargo fmt --all
git add crates/kiro-market-core/src/validation.rs
git commit -m "feat(core): add MarketplaceName and PluginName newtypes (A1 step 1/8)

Validated newtypes for marketplace and plugin name strings. Route through
the existing validate_name (non-empty, no NUL/control bytes, no traversal,
no Windows-reserved). serde(transparent) keeps wire format byte-identical
to a plain string; Deserialize routed through new for parse-don't-validate
at tracking-file load.

Models after AgentName's shape (TryFrom impls, AsRef<str>, Display,
PartialEq<str>, PartialEq<&str>, Deserialize). Deliberately does NOT
derive Default — matches RelativePath/AgentName/GitRef precedent.

No callers yet; subsequent commits in this PR migrate the kiro-market-core
public API surface."
```

---

## Task 2: Add `mp` and `pn` test helpers

**Files:**
- Modify: `crates/kiro-market-core/src/service/test_support.rs`

Test fixtures across the workspace pass `"mp"` / `"p"` literals to functions that will, after the migration, expect `&MarketplaceName` / `&PluginName`. Without a helper, every test site grows a `MarketplaceName::new("mp").expect("...")` boilerplate. Add focused helpers.

- [ ] **Step 1: Add the helpers**

Append to `crates/kiro-market-core/src/service/test_support.rs`:

```rust
/// Test helper: construct a [`MarketplaceName`] from a string literal,
/// panicking on validation failure. The plan's test fixtures pass `"mp"`,
/// `"plug-a"`, etc. — values controlled by the test author, not user
/// input. `expect` is the right shape here because a fixture failure
/// is a bug, not an error to handle.
#[cfg(any(test, feature = "test-support"))]
pub fn mp(s: &str) -> crate::validation::MarketplaceName {
    crate::validation::MarketplaceName::new(s)
        .unwrap_or_else(|e| panic!("test fixture: invalid marketplace name {s:?}: {e}"))
}

/// Test helper: construct a [`PluginName`] from a string literal. See
/// [`mp`] for the contract.
#[cfg(any(test, feature = "test-support"))]
pub fn pn(s: &str) -> crate::validation::PluginName {
    crate::validation::PluginName::new(s)
        .unwrap_or_else(|e| panic!("test fixture: invalid plugin name {s:?}: {e}"))
}
```

- [ ] **Step 2: Verify compile**

```bash
cargo build -p kiro-market-core --tests 2>&1 | tail -5
```

Expected: clean build (no usages yet, but compile must pass — `unwrap_or_else` with `panic!` is the test-fixture idiom; `expect` would also work but `unwrap_or_else` carries the original error message).

- [ ] **Step 3: Lint and commit**

```bash
cargo clippy -p kiro-market-core --tests -- -D warnings
cargo fmt --all
git add crates/kiro-market-core/src/service/test_support.rs
git commit -m "feat(test-support): add mp/pn helpers for MarketplaceName/PluginName fixtures (A1 step 2/8)

Test fixtures across the workspace pass 'mp' / 'p' literals to functions
that — after the A1 migration — expect &MarketplaceName / &PluginName.
Centralising the construction in mp/pn keeps fixtures terse.

The helpers use unwrap_or_else with panic! because a malformed test
fixture is a bug, not an error to handle. Cfg-gated to test + the
test-support feature so the Tauri crate's dev-dependencies activate
the same helpers via feature override."
```

---

## Task 3: Migrate `project.rs` — meta types, removal API, install paths, aggregator

**Files:**
- Modify: `crates/kiro-market-core/src/project.rs` (the heavy lift — 4 meta types, 2 internal context structs, 5 removal methods, 6+ install methods/free fns, the `installed_plugins` aggregator + `InstalledPluginInfo`, and ~30 test fixtures)

This task is the biggest single chunk because the changes ripple together — splitting them would require `.as_str()` shims in intermediate states. Single coherent commit.

The migration shape: **every place that today reads / writes / accepts `marketplace` or `plugin` as `&str` / `String` becomes `&MarketplaceName` / `&PluginName` / `MarketplaceName` / `PluginName`**. Internal callers update to pass through; tracking-file deserializers automatically validate via the newtype's `Deserialize`.

- [ ] **Step 1: Migrate the four tracking-file meta types**

Change the field types. Search-and-replace in `project.rs`:

```rust
// InstalledSkillMeta (line 24-46) — change marketplace/plugin from String
pub struct InstalledSkillMeta {
    pub marketplace: MarketplaceName,
    pub plugin: PluginName,
    pub version: Option<String>,
    pub installed_at: DateTime<Utc>,
    // hash fields unchanged
}

// InstalledAgentMeta — same change
// InstalledNativeCompanionsMeta — same change
// InstalledSteeringMeta — same change
```

Add to imports at top of `project.rs`:

```rust
use crate::validation::{MarketplaceName, PluginName};
```

- [ ] **Step 2: Migrate `InstalledPluginInfo` and `installed_plugins` aggregator**

```rust
// InstalledPluginInfo at line 266
pub struct InstalledPluginInfo {
    pub marketplace: MarketplaceName,
    pub plugin: PluginName,
    pub installed_version: Option<String>,
    pub skill_count: u32,
    // ... rest unchanged
    pub installed_skills: Vec<String>,         // skill names — out of scope (see design)
    pub installed_steering: Vec<std::path::PathBuf>,
    pub installed_agents: Vec<String>,         // agent names — out of scope (see design)
    pub earliest_install: String,
    pub latest_install: String,
}
```

Inside `installed_plugins` (`KiroProject` impl, around line 1013), the `BTreeMap<(String, String), Acc>` key becomes `BTreeMap<(MarketplaceName, PluginName), Acc>`:

```rust
let mut by_pair: BTreeMap<(MarketplaceName, PluginName), Acc> = BTreeMap::new();

// In each loop body:
let acc = by_pair
    .entry((meta.marketplace.clone(), meta.plugin.clone()))
    .or_default();
```

The terminal `.map(...)` block populates `InstalledPluginInfo.marketplace` and `.plugin` directly with the newtypes — no conversion needed:

```rust
.map(|((marketplace, plugin), mut acc)| {
    acc.skills.sort();
    acc.steering.sort();
    acc.agents.sort();
    let (latest_install_dt, installed_version) =
        acc.latest.map_or_else(|| (now, None), |(t, v)| (t, v));
    let earliest_install_dt = acc.earliest.unwrap_or(now);
    InstalledPluginInfo {
        marketplace,
        plugin,
        installed_version,
        // ... unchanged
    }
})
```

Note: `MarketplaceName` and `PluginName` derive `Eq` and `Ord` is **not** derived. `BTreeMap` requires `Ord`. Add `Ord` and `PartialOrd` derives to both newtypes in `validation.rs`:

```rust
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
```

(Apply to both `MarketplaceName` and `PluginName`. Add the corresponding test in `validation.rs::tests`:)

```rust
#[test]
fn marketplace_name_ord_is_lexicographic_on_inner() {
    let a = MarketplaceName::new("alpha").expect("valid");
    let b = MarketplaceName::new("bravo").expect("valid");
    assert!(a < b);
}
```

- [ ] **Step 3: Migrate `KiroProject` removal API signatures**

```rust
impl KiroProject {
    pub fn remove_plugin(
        &self,
        marketplace: &MarketplaceName,
        plugin: &PluginName,
    ) -> crate::error::Result<RemovePluginResult> { ... }

    pub fn remove_native_companions_for_plugin(
        &self,
        plugin: &PluginName,
        marketplace: &MarketplaceName,
    ) -> crate::error::Result<()> { ... }
}
```

Inside `remove_plugin`, the filter clauses become:

```rust
let skills_to_remove: Vec<String> = skills
    .skills
    .iter()
    .filter(|(_, meta)| meta.marketplace == *marketplace && meta.plugin == *plugin)
    .map(|(name, _)| name.clone())
    .collect();
```

(`MarketplaceName` derives `PartialEq`, so `meta.marketplace == *marketplace` is the comparison — same shape as before, just typed.)

The `RemovePluginFailure { content_type, item: ..., error }` constructions inside `remove_plugin`'s match arms keep `String` for `content_type` and `item` (these are wire-format strings, not validated identifiers).

The native_companions cleanup gets the marketplace-aware check (already in place from PR #94's A-16 work) — the comparison reads `meta.marketplace == *marketplace`.

- [ ] **Step 4: Migrate `KiroProject` install paths**

```rust
impl KiroProject {
    pub fn install_native_agent(
        &self,
        bundle: &crate::agent::NativeAgentBundle,
        marketplace: &MarketplaceName,
        plugin: &PluginName,
        version: Option<&str>,
        source_hash: &str,
        mode: crate::service::InstallMode,
    ) -> Result<InstalledNativeAgentOutcome, AgentError> { ... }

    pub fn stage_native_companion_files(
        &self,
        plugin: &PluginName,
        scan_root: &Path,
        rel_paths: &[PathBuf],
    ) -> crate::error::Result<(tempfile::TempDir, String)> { ... }
}

// Internal context structs at line 219, 556
struct NativeCompanionsInput<'a> {
    pub scan_root: &'a Path,
    pub rel_paths: &'a [PathBuf],
    pub marketplace: &'a MarketplaceName,
    pub plugin: &'a PluginName,
    pub version: Option<&'a str>,
    pub source_hash: &'a str,
    pub mode: crate::service::InstallMode,
}

struct CompanionInput<'a> {
    pub marketplace: &'a MarketplaceName,
    pub plugin: &'a PluginName,
    pub version: Option<&'a str>,
    pub agents_root: &'a Path,
    pub prompt_rel: &'a Path,
}
```

Free helper signatures (in the same file) update too — every place that today takes `plugin: &str` becomes `plugin: &PluginName`:

```rust
fn classify_native_collision(
    installed: &InstalledAgents,
    agent_name: &str,           // unchanged — this is the agent identifier, not marketplace/plugin
    plugin: &PluginName,
    source_hash: &str,
    json_target: &Path,
    mode: crate::service::InstallMode,
) -> crate::error::Result<CollisionDecision<InstalledNativeAgentOutcome>> { ... }

fn classify_steering_collision(
    installed: &InstalledSteering,
    rel_path: &Path,
    plugin: &PluginName,
    source_hash: &str,
    dest: &Path,
    mode: crate::service::InstallMode,
) -> Result<CollisionDecision<SteeringIdempotentEcho>, crate::steering::SteeringError> { ... }

fn synthesize_companion_entry(
    installed: &mut InstalledAgents,
    input: &CompanionInput<'_>,
) -> crate::error::Result<()> { ... }

fn strip_transferred_paths_from_other_plugins(
    installed: &mut InstalledAgents,
    plugin: &PluginName,
    rel_paths: &[PathBuf],
    agents_dir: &Path,
) -> crate::error::Result<()> { ... }

fn diff_prior_companion_files(
    installed: &InstalledAgents,
    plugin: &PluginName,
    rel_paths: &[PathBuf],
) -> Vec<PathBuf> { ... }

fn remove_companion_files_best_effort(
    rel_paths: &[PathBuf],
    agents_dir: &Path,
    plugin: &PluginName,
)
```

- [ ] **Step 5: Update meta-construction sites**

Every place that today does:

```rust
let meta = InstalledSkillMeta {
    marketplace: ctx.marketplace.to_string(),
    plugin: ctx.plugin.to_string(),
    // ...
};
```

becomes:

```rust
let meta = InstalledSkillMeta {
    marketplace: ctx.marketplace.clone(),
    plugin: ctx.plugin.clone(),
    // ...
};
```

(The context structs hold `&MarketplaceName` / `&PluginName` after Task 5; for now in Task 3, where `ctx.marketplace` is still `&str` from un-migrated context structs, you'll temporarily call `MarketplaceName::new(ctx.marketplace).expect("ctx already validated")` OR you can accept that this single task has a half-migrated state and the workspace doesn't cleanly compile until Tasks 4 + 5 land. **This task ends with the workspace IN A NON-COMPILING STATE if done in isolation** — see Step 7 note below.)

- [ ] **Step 6: Update `project.rs::tests` test fixtures (~30 sites)**

Every test that constructs an `InstalledSkillMeta` / `InstalledAgentMeta` / etc. with `marketplace: "mp".to_string()` becomes:

```rust
use crate::service::test_support::{mp, pn};

let meta = InstalledSkillMeta {
    marketplace: mp("mp"),
    plugin: pn("plug-a"),
    // ...
};
```

Tests that call `project.remove_plugin("mp", "p")` become `project.remove_plugin(&mp("mp"), &pn("p"))`. Same for `remove_native_companions_for_plugin`.

Tests that build JSON tracking files with `serde_json::json!(...)` strings — these test the deserialization path. Since `MarketplaceName::Deserialize` routes through `new`, the JSON strings just need to remain valid identifiers. The existing `"mp"` / `"plug-a"` test values are valid; nothing changes in the JSON construction itself.

The one exception: tests that intentionally construct INVALID names to test rejection (e.g., `load_installed_rejects_path_traversal_in_skills_key`) — these continue to use raw strings in the JSON and assert that `load_installed` returns `Err`. The newtype's `Deserialize` rejection now happens at the load layer, possibly making the I9 walker's path-traversal check redundant for the meta fields (still relevant for HashMap keys). Verify the existing tests still pass; the assertion shape (`.is_err()`) is unchanged.

- [ ] **Step 7: Build and test**

```bash
cargo build -p kiro-market-core 2>&1 | tail -20
```

Many compilation errors expected during sub-step iteration. Work through them one by one. By the end of step 6, the kiro-market-core crate should build clean.

```bash
cargo test -p kiro-market-core 2>&1 | grep "test result:" | tail -5
```

Expected: all `kiro-market-core` tests pass (the migration is internal; behavior is preserved).

**The `kiro-control-center` and `kiro-market` crates will NOT compile yet** — they call functions whose signatures just changed. That's expected; Tasks 5–7 fix them. Do not attempt to compile the full workspace until Task 7.

- [ ] **Step 8: Lint and commit (kiro-market-core only)**

```bash
cargo clippy -p kiro-market-core --tests -- -D warnings
cargo fmt --all
git add crates/kiro-market-core/src/project.rs crates/kiro-market-core/src/validation.rs
git commit -m "refactor(core): migrate project.rs to MarketplaceName/PluginName (A1 step 3/8)

Migrates project.rs's marketplace/plugin string surface to the validated
newtypes:

- 4 tracking-file meta types (InstalledSkillMeta, InstalledAgentMeta,
  InstalledNativeCompanionsMeta, InstalledSteeringMeta) — parse-don't-validate
  at serde load via the newtype Deserialize impls
- InstalledPluginInfo + installed_plugins aggregator
- KiroProject::remove_plugin and remove_native_companions_for_plugin
  signatures (compiler now enforces argument order — closes the swap-arg
  footgun the type-design reviewer flagged on PR #94)
- KiroProject::install_native_agent + stage_native_companion_files
- Internal NativeCompanionsInput / CompanionInput context structs
- Free helpers (classify_native_collision, classify_steering_collision,
  synthesize_companion_entry, strip_transferred_paths_from_other_plugins,
  diff_prior_companion_files, remove_companion_files_best_effort)
- ~30 test fixtures via mp() / pn() helpers

Adds Ord+PartialOrd derives to MarketplaceName/PluginName so they can key
the BTreeMap in installed_plugins.

The kiro-control-center and kiro-market crates do NOT compile after this
commit — Tasks 5-7 fix them. Subsequent commits in this PR will."
```

---

## Task 4: Migrate `steering/types.rs`

**Files:**
- Modify: `crates/kiro-market-core/src/steering/types.rs`

`SteeringInstallContext.marketplace`/`plugin` and `SteeringError::PathOwnedByOtherPlugin.owner` carry plugin-name strings.

- [ ] **Step 1: Migrate `SteeringInstallContext`**

```rust
// Around line 20
#[derive(Clone, Copy)]
pub struct SteeringInstallContext<'a> {
    pub mode: InstallMode,
    pub marketplace: &'a crate::validation::MarketplaceName,
    pub plugin: &'a crate::validation::PluginName,
    pub version: Option<&'a str>,
}
```

- [ ] **Step 2: Migrate `SteeringError::PathOwnedByOtherPlugin.owner`**

```rust
// Around line 55
PathOwnedByOtherPlugin {
    rel: PathBuf,
    owner: crate::validation::PluginName,
}
```

The `Display` impl for `SteeringError` currently formats `owner` via `{owner}`. After the migration, `PluginName: Display` delegates to the inner `String`, so the rendered message is byte-identical.

- [ ] **Step 3: Run tests**

```bash
cargo test -p kiro-market-core steering -- --nocapture 2>&1 | tail -10
```

Expected: all steering tests pass. The wire-format `serialize_steering_error` tests continue to lock the rendered string shape; `PluginName::Display` produces the same output as `String::Display` for valid names.

- [ ] **Step 4: Lint and commit**

```bash
cargo clippy -p kiro-market-core --tests -- -D warnings
cargo fmt --all
git add crates/kiro-market-core/src/steering/types.rs
git commit -m "refactor(core): migrate SteeringInstallContext + SteeringError to PluginName (A1 step 4/8)

SteeringInstallContext.marketplace/plugin become &MarketplaceName/&PluginName.
SteeringError::PathOwnedByOtherPlugin.owner becomes PluginName. Display
contract preserved (PluginName: Display delegates to inner String).

The steering install path's wire format is unchanged — serialize_steering_error
renders to the same string shape."
```

---

## Task 5: Migrate `service/mod.rs` — install API + `InstallPluginResult` + A4

**Files:**
- Modify: `crates/kiro-market-core/src/service/mod.rs`

This is the second-heaviest task. Migrates the `MarketplaceService` install API surface, `AgentInstallContext`, the install free fns, and lands A4 (`marketplace` field on `InstallPluginResult`).

- [ ] **Step 1: Migrate `AgentInstallContext`**

```rust
// Around line 359
pub struct AgentInstallContext<'a> {
    pub mode: InstallMode,
    pub accept_mcp: bool,
    pub marketplace: &'a crate::validation::MarketplaceName,
    pub plugin: &'a crate::validation::PluginName,
    pub version: Option<&'a str>,
}
```

- [ ] **Step 2: Migrate `InstallPluginResult` and add A4 `marketplace` field**

```rust
// Around line 419 — note: drop the Default derive (verified per design doc:
// no consumer calls InstallPluginResult::default() and the newtypes don't
// derive Default)
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct InstallPluginResult {
    pub marketplace: crate::validation::MarketplaceName,    // NEW (A4)
    pub plugin: crate::validation::PluginName,              // changed from String per A1
    pub version: Option<String>,
    pub skills: InstallSkillsResult,
    pub steering: crate::steering::InstallSteeringResult,
    pub agents: InstallAgentsResult,
}
```

- [ ] **Step 3: Migrate `MarketplaceService::install_plugin` signature and body**

```rust
// Around line 1924
pub fn install_plugin(
    &self,
    project: &crate::project::KiroProject,
    marketplace: &crate::validation::MarketplaceName,
    plugin: &crate::validation::PluginName,
    mode: InstallMode,
    accept_mcp: bool,
) -> Result<InstallPluginResult, Error> {
    let ctx = self.resolve_plugin_install_context(marketplace, plugin)?;

    let skills = self.install_skills(
        project,
        &ctx.skill_dirs,
        &InstallFilter::All,
        mode,
        marketplace,
        plugin,
        ctx.version.as_deref(),
    );

    let steering = Self::install_plugin_steering(
        project,
        &ctx.plugin_dir,
        &ctx.steering_scan_paths,
        crate::steering::SteeringInstallContext {
            mode,
            marketplace,
            plugin,
            version: ctx.version.as_deref(),
        },
    );

    let agents = Self::install_plugin_agents(
        project,
        &ctx.plugin_dir,
        &ctx.agent_scan_paths,
        ctx.format,
        AgentInstallContext {
            mode,
            accept_mcp,
            marketplace,
            plugin,
            version: ctx.version.as_deref(),
        },
    );

    Ok(InstallPluginResult {
        marketplace: marketplace.clone(),    // NEW (A4)
        plugin: plugin.clone(),
        version: ctx.version,
        skills,
        steering,
        agents,
    })
}
```

- [ ] **Step 4: Migrate `MarketplaceService::install_skills` signature**

```rust
// Around line 1035
pub fn install_skills(
    &self,
    project: &crate::project::KiroProject,
    skill_dirs: &[PathBuf],
    filter: &InstallFilter<'_>,
    mode: InstallMode,
    marketplace: &crate::validation::MarketplaceName,
    plugin: &crate::validation::PluginName,
    version: Option<&str>,
) -> InstallSkillsResult { ... }
```

Internal callers update too. Inside the method body, anywhere that constructs an `InstalledSkillMeta`:

```rust
let meta = InstalledSkillMeta {
    marketplace: marketplace.clone(),
    plugin: plugin.clone(),
    version: version.map(str::to_string),
    installed_at: chrono::Utc::now(),
    source_hash: Some(source_hash.clone()),
    installed_hash: Some(installed_hash.clone()),
};
```

- [ ] **Step 5: Migrate the install free fns**

`install_plugin_steering` (around line 1349), `install_translated_agents_inner` (1420), `install_native_kiro_cli_agents_inner` (1549), `install_one_native_agent` (1607), `install_native_companions_for_plugin` (1699) — these are private/free and take the context structs that now hold newtypes. The signatures change only where they accept `&str` directly (none do — they all flow through `AgentInstallContext` / `SteeringInstallContext`). Verify by reading their bodies and patching any `meta.marketplace = ctx.marketplace.to_string()` to `meta.marketplace = ctx.marketplace.clone()`.

- [ ] **Step 6: Update tests in `service/mod.rs::tests`**

The tests in `mod tests` (line 2040+) construct contexts with `"mp"` / `"p"` literals. Use `mp("mp")` / `pn("p")` helpers:

```rust
use crate::service::test_support::{mp, pn};

let result = svc
    .install_plugin(&project, &mp("mp"), &pn("p"), InstallMode::New, false)
    .expect("install_plugin happy path");
```

The `InstallPluginResult` JSON-shape rstests (4774, 4808) must update to match the new shape:

```rust
#[test]
fn install_plugin_result_json_shape_locks_default_subresults() {
    let result = InstallPluginResult {
        marketplace: mp("mp"),
        plugin: pn("p"),
        version: Some("1.0.0".into()),
        skills: InstallSkillsResult::default(),
        steering: crate::steering::InstallSteeringResult::default(),
        agents: InstallAgentsResult::default(),
    };
    let json = serde_json::to_value(&result).expect("serialize");
    assert_eq!(json["marketplace"], "mp");
    assert_eq!(json["plugin"], "p");
    assert_eq!(json["version"], "1.0.0");
    assert!(json["skills"].is_object());
    assert!(json["steering"].is_object());
    assert!(json["agents"].is_object());
}

#[test]
fn install_plugin_result_json_shape_with_populated_subresult() {
    let result = InstallPluginResult {
        marketplace: mp("mp"),
        plugin: pn("p"),
        version: Some("1.0.0".into()),
        skills: InstallSkillsResult {
            installed: vec!["alpha".into()],
            ..InstallSkillsResult::default()
        },
        steering: crate::steering::InstallSteeringResult::default(),
        agents: InstallAgentsResult::default(),
    };
    let json = serde_json::to_value(&result).expect("serialize");
    let skills = json.pointer("/skills").expect("skills field exists");
    assert!(skills.is_object());
    assert_eq!(
        skills
            .pointer("/installed")
            .and_then(|v| v.as_array())
            .map(Vec::len),
        Some(1),
    );
    // A4: marketplace field serializes as a plain string via serde(transparent)
    assert_eq!(json["marketplace"], "mp");
}
```

- [ ] **Step 7: Build kiro-market-core**

```bash
cargo build -p kiro-market-core --tests 2>&1 | tail -10
cargo test -p kiro-market-core 2>&1 | grep "test result:" | tail -5
```

Expected: all kiro-market-core tests pass. (Tauri + CLI crates still don't compile; Tasks 6-7 fix them.)

- [ ] **Step 8: Lint and commit**

```bash
cargo clippy -p kiro-market-core --tests -- -D warnings
cargo fmt --all
git add crates/kiro-market-core/src/service/mod.rs
git commit -m "refactor(core): migrate service install API to newtypes; add A4 marketplace field (A1 step 5/8)

- AgentInstallContext.marketplace/plugin now &MarketplaceName / &PluginName
- InstallPluginResult.plugin: PluginName (was String); A4 adds .marketplace: MarketplaceName
- InstallPluginResult drops the (unused) Default derive
- MarketplaceService::install_plugin and install_skills signatures take
  &MarketplaceName / &PluginName — argument order is now compiler-enforced
- Free fns (install_plugin_steering, install_translated_agents_inner,
  install_native_kiro_cli_agents_inner, install_one_native_agent,
  install_native_companions_for_plugin) flow newtype contexts through
- ~12 test fixtures via mp() / pn() helpers
- The two install_plugin_result_json_shape_locks_* tests assert the A4
  marketplace field serializes as a plain string via serde(transparent)"
```

---

## Task 6: Migrate `service/browse.rs::resolve_plugin_install_context*`

**Files:**
- Modify: `crates/kiro-market-core/src/service/browse.rs`

- [ ] **Step 1: Migrate the method signatures**

```rust
// Around line 719
pub fn resolve_plugin_install_context(
    &self,
    marketplace: &crate::validation::MarketplaceName,
    plugin: &crate::validation::PluginName,
) -> Result<PluginInstallContext, Error> {
    let mp_path = self.marketplace_path(marketplace.as_str());
    let plugin_entries = self.list_plugin_entries(marketplace.as_str())?;
    let entry = plugin_entries
        .iter()
        .find(|p| p.name == *plugin)        // PluginName: PartialEq<str> via the &PluginEntry path
        .ok_or_else(|| Error::Plugin(PluginError::PluginNotFound {
            plugin: plugin.as_str().to_string(),
        }))?;
    // ... rest of method body unchanged
}
```

Note: `marketplace_path` and `list_plugin_entries` (both on `MarketplaceService`) take `&str` and stay that way — they're out of scope per the design's "non-install API stays String-typed" decision. Use `marketplace.as_str()` to bridge.

`PluginEntry.name` is a `String` (out of scope for migration); the comparison `p.name == *plugin` works because `PluginName: PartialEq<str>` and the `*plugin` deref pattern.

- [ ] **Step 2: Migrate the existing tests in `service/browse.rs::tests`**

The `resolve_plugin_install_context_*` tests use string literals; switch to `mp("mp1")` / `pn("myplugin")`:

```rust
use crate::service::test_support::{mp, pn};

let ctx = svc
    .resolve_plugin_install_context(&mp("mp1"), &pn("myplugin"))
    .expect("resolve");
```

About 6 test sites in `tests` need this update.

- [ ] **Step 3: Build and test**

```bash
cargo test -p kiro-market-core resolve_plugin_install_context 2>&1 | tail -10
```

Expected: all 9 resolve_plugin_install_context_* tests pass.

- [ ] **Step 4: Lint and commit**

```bash
cargo clippy -p kiro-market-core --tests -- -D warnings
cargo fmt --all
git add crates/kiro-market-core/src/service/browse.rs
git commit -m "refactor(core): migrate resolve_plugin_install_context to newtypes (A1 step 6/8)

resolve_plugin_install_context now takes &MarketplaceName / &PluginName.
Internal calls to non-migrated MarketplaceService methods (marketplace_path,
list_plugin_entries — both out of scope per Phase 1.5 design) bridge via
marketplace.as_str()."
```

---

## Task 7: Migrate Tauri `_impl`s — `commands/{agents,plugins,steering,browse,installed}.rs`

**Files:**
- Modify: `crates/kiro-control-center/src-tauri/src/commands/agents.rs`
- Modify: `crates/kiro-control-center/src-tauri/src/commands/plugins.rs`
- Modify: `crates/kiro-control-center/src-tauri/src/commands/steering.rs`
- Modify: `crates/kiro-control-center/src-tauri/src/commands/browse.rs`
- Modify: `crates/kiro-control-center/src-tauri/src/commands/installed.rs`

The Tauri command wrappers stay `String`-typed (FE callers pass strings; the design locks this). Inside each `_impl`, replace `validate_name(marketplace)?` with `MarketplaceName::new(marketplace)?` (and same for `plugin`). The resulting `MarketplaceName` flows to the (now-migrated) core API.

- [ ] **Step 1: Update `commands/agents.rs::install_plugin_agents_impl`**

```rust
fn install_plugin_agents_impl(
    svc: &MarketplaceService,
    marketplace: &str,
    plugin: &str,
    mode: InstallMode,
    accept_mcp: bool,
    project_path: &str,
) -> Result<InstallAgentsResult, CommandError> {
    let project_root = validate_kiro_project_path(project_path)?;
    let marketplace = kiro_market_core::validation::MarketplaceName::new(marketplace)?;
    let plugin = kiro_market_core::validation::PluginName::new(plugin)?;
    let ctx = svc
        .resolve_plugin_install_context(&marketplace, &plugin)
        .map_err(CommandError::from)?;
    let project = KiroProject::new(project_root);

    Ok(MarketplaceService::install_plugin_agents(
        &project,
        &ctx.plugin_dir,
        &ctx.agent_scan_paths,
        ctx.format,
        AgentInstallContext {
            mode,
            accept_mcp,
            marketplace: &marketplace,
            plugin: &plugin,
            version: ctx.version.as_deref(),
        },
    ))
}
```

The previous `validate_name(marketplace)?; validate_name(plugin)?;` lines from PR #94 (Phase 1 I10) are now redundant — the newtype's `new` performs the same validation. Delete those calls.

- [ ] **Step 2: Update `commands/plugins.rs`** (3 commands: `install_plugin_impl`, `list_installed_plugins`, `remove_plugin`)

```rust
fn install_plugin_impl(
    svc: &MarketplaceService,
    marketplace: &str,
    plugin: &str,
    mode: InstallMode,
    accept_mcp: bool,
    project_path: &str,
) -> Result<InstallPluginResult, CommandError> {
    let project_root = validate_kiro_project_path(project_path)?;
    let marketplace = kiro_market_core::validation::MarketplaceName::new(marketplace)?;
    let plugin = kiro_market_core::validation::PluginName::new(plugin)?;
    let project = KiroProject::new(project_root);
    svc.install_plugin(&project, &marketplace, &plugin, mode, accept_mcp)
        .map_err(CommandError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn remove_plugin(
    marketplace: String,
    plugin: String,
    project_path: String,
) -> Result<RemovePluginResult, CommandError> {
    let project_root = validate_kiro_project_path(&project_path)?;
    let marketplace = kiro_market_core::validation::MarketplaceName::new(marketplace)?;
    let plugin = kiro_market_core::validation::PluginName::new(plugin)?;
    let project = KiroProject::new(project_root);
    project
        .remove_plugin(&marketplace, &plugin)
        .map_err(CommandError::from)
}
```

`list_installed_plugins` doesn't take marketplace/plugin args — no change needed beyond fixing any compile errors from `InstalledPluginsView.plugins[*].marketplace` now being `MarketplaceName` instead of `String` (probably nothing — the wrapper just returns the view).

- [ ] **Step 3: Update `commands/steering.rs::install_plugin_steering_impl`**

Same pattern: replace `validate_name(...)?` calls with `MarketplaceName::new(...)?` / `PluginName::new(...)?`, pass references to the migrated `MarketplaceService::install_plugin_steering`.

- [ ] **Step 4: Update `commands/browse.rs::install_skills_impl`** and `list_plugins`

`install_skills_impl` follows the same pattern. `list_plugins` only takes `marketplace: &str`; construct `MarketplaceName::new(marketplace)?` and pass through (note: `list_plugin_entries` itself stays `&str`-typed per the design's non-install scope, so the newtype is constructed for validation but `as_str()` is used for the actual call).

- [ ] **Step 5: Update `commands/installed.rs::remove_skill`**

`remove_skill` takes a skill name (not a plugin name) — no migration needed for the skill-name parameter. But `KiroProject::remove_skill` is in scope for the migration? **Check:** `KiroProject::remove_skill(name: &str)` is at `project.rs:797`. Looking at the design, only the plugin-cascade methods are listed. The skill-name parameter is not in scope.

The Tauri `remove_skill` wrapper passes `name: String` to `KiroProject::remove_skill`; both stay `&str`-typed. No change in this task for `installed.rs::remove_skill`.

- [ ] **Step 6: Update Tauri tests**

All 5 command files have `#[cfg(test)] mod tests` blocks. The tests construct call args with `"mp".to_string()` literals; the wrapper signatures still take `String`, so the test inputs don't change. But tests that call core APIs directly (e.g., `install_plugin_impl(&svc, "mp", "p", ...)`) need to pass `&MarketplaceName` / `&PluginName`:

```rust
use kiro_market_core::service::test_support::{mp, pn};

#[test]
fn install_plugin_impl_orchestrates_all_three_paths() {
    // ...
    let project_root_str = make_kiro_project(dir.path());
    let result = install_plugin_impl(
        &svc,
        "mp",
        "p",
        InstallMode::New,
        false,
        &project_root_str,
    )
    .expect("happy path");
    // assertions: result.marketplace == "mp" (PluginName: PartialEq<&str>)
    assert_eq!(result.marketplace, "mp");
    assert_eq!(result.plugin, "p");
}
```

The `install_plugin_impl_rejects_traversal_in_marketplace` and `..._rejects_nul_byte_in_plugin` tests (added in PR #94 I10) continue to work — the validation now happens via `MarketplaceName::new`'s ValidationError instead of a direct `validate_name` call, but the resulting `CommandError` shape is the same.

- [ ] **Step 7: Verify the workspace finally compiles**

```bash
cargo build --workspace 2>&1 | tail -10
```

Expected: clean. The Tauri crate should now compile against the migrated core types.

```bash
cargo test --workspace 2>&1 | grep "test result:" | tail -10
```

Expected: all tests across all crates pass.

- [ ] **Step 8: Lint and commit**

```bash
cargo clippy --workspace --tests -- -D warnings
cargo fmt --all
git add crates/kiro-control-center/src-tauri/src/commands/agents.rs \
        crates/kiro-control-center/src-tauri/src/commands/plugins.rs \
        crates/kiro-control-center/src-tauri/src/commands/steering.rs \
        crates/kiro-control-center/src-tauri/src/commands/browse.rs \
        crates/kiro-control-center/src-tauri/src/commands/installed.rs
git commit -m "refactor(tauri): construct MarketplaceName/PluginName at IPC boundary (A1 step 7/8)

Replaces PR #94's I10 validate_name(...)? calls with the newtype
constructor pattern. Same effective gate, but the resulting handle
proves provenance for the rest of the function body.

The Tauri command wrappers stay String-typed — FE callers pass strings;
specta-aliased newtypes don't enforce nominal types in TS without branded
patterns. The newtype is constructed in the _impl after
validate_kiro_project_path."
```

---

## Task 8: Migrate `kiro-market` CLI + integration tests + bindings + final sweep + open PR

**Files:**
- Modify: `crates/kiro-market/src/commands/install.rs`
- Modify: `crates/kiro-market-core/tests/integration_native_install.rs`
- Regenerate: `crates/kiro-control-center/src/lib/bindings.ts`

- [ ] **Step 1: Update CLI install command**

In `crates/kiro-market/src/commands/install.rs`, after clap parses the user's `plugin@marketplace` string, construct the newtypes:

```rust
use kiro_market_core::validation::{MarketplaceName, PluginName};

// After parsing plugin_ref into (plugin_str, marketplace_str)...
let marketplace = MarketplaceName::new(marketplace_str)
    .with_context(|| format!("invalid marketplace name: {marketplace_str}"))?;
let plugin = PluginName::new(plugin_str)
    .with_context(|| format!("invalid plugin name: {plugin_str}"))?;

// Then pass through:
svc.install_plugin(&project, &marketplace, &plugin, mode, args.accept_mcp)
```

(Use `anyhow::Context` since the binary uses `anyhow` per CLAUDE.md.)

The CLI's existing tests in `tests/cli_install.rs` (or wherever) — verify they still pass. The CLI accepts strings on the command line; validation now happens in the newtype constructor instead of separately. Error message shape may shift slightly; if tests assert exact error strings, update assertions.

- [ ] **Step 2: Update `integration_native_install.rs`**

Read the file:

```bash
grep -nE "marketplace:|plugin:|install_plugin" crates/kiro-market-core/tests/integration_native_install.rs | head -20
```

Update test fixtures to use `mp(...)` / `pn(...)` from `service::test_support`. The `test-support` feature must be enabled — verify via the existing `[dev-dependencies]` block in `kiro-market-core/Cargo.toml`.

- [ ] **Step 3: Regenerate bindings.ts**

```bash
cargo test -p kiro-control-center --lib generate_types -- --ignored 2>&1 | tail -3
```

Expected: pass. Verify the new types appear:

```bash
grep -E "^export type (MarketplaceName|PluginName)\b" crates/kiro-control-center/src/lib/bindings.ts
```

Both should resolve to `string` (via `serde(transparent)` + specta).

The frontend code (`BrowseTab.svelte`, `InstalledTab.svelte`) doesn't need changes — TS treats the aliases as `string` (the frontend's `pluginKey(p.marketplace, p.plugin)` calls work unchanged).

- [ ] **Step 4: Apply the 5-gates plan-review checklist**

Per `docs/plan-review-checklist.md`, apply each gate:

```bash
TETHYS_BIN=/home/dwalleck/repos/rivets/target/release/tethys cargo xtask plan-lint 2>&1 | tail -10
```

Expected: all 6 sub-gates OK (gate-4-external-error-boundary, no-unwrap-in-production, no-panic-in-production, non-exhaustive-error-enum, no-frontend-deps-in-core, ffi-enum-serde-tag).

Document any gate findings in `2026-04-30-phase-1-5-type-safety-plan-amendments.md` per the precedent.

- [ ] **Step 5: Run all pre-commit gates**

```bash
cargo fmt --all --check
cargo clippy --workspace --tests -- -D warnings
cargo test --workspace 2>&1 | grep "test result:" | tail -10
cd crates/kiro-control-center && npm run check 2>&1 | tail -3 && cd ../..
```

All green required.

- [ ] **Step 6: Commit and push**

```bash
git add crates/kiro-market/src/commands/install.rs \
        crates/kiro-market-core/tests/integration_native_install.rs \
        crates/kiro-control-center/src/lib/bindings.ts
git commit -m "refactor(cli+test): finish A1 migration; regenerate bindings (A1 step 8/8)

- CLI install_plugin command constructs MarketplaceName/PluginName from
  clap-parsed strings before calling core APIs (anyhow with_context on
  validation failure)
- Integration tests in kiro-market-core/tests/integration_native_install.rs
  use mp() / pn() helpers
- bindings.ts regenerated; MarketplaceName and PluginName emit as 'string'
  type aliases via specta(transparent). Frontend code unchanged."
git push -u origin feat/phase-1-5-types
```

- [ ] **Step 7: Open PR**

```bash
gh pr create --title "feat: type-safety hardening — MarketplaceName/PluginName newtypes (Phase 1.5)" --body "$(cat <<'EOF'
## Summary

Phase 1.5 of the plugin-first install architecture. Closes the swap-arg footgun in 7+ public APIs by introducing validated `MarketplaceName` / `PluginName` newtypes, and adds the missing `marketplace` field to `InstallPluginResult` for symmetry with `InstalledPluginInfo` (A4).

Background: PR #94 (Phase 1) shipped with an 8-reviewer aggregated review where 3 reviewers convergent on the swap-arg risk. The CLAUDE.md \`validation.rs\` template (`RelativePath`, `AgentName`) was the natural pattern to apply.

## What's in scope

- **A1: `MarketplaceName` / `PluginName` newtypes.** Defined in `validation.rs` next to the existing `AgentName` precedent. `serde(transparent)` keeps wire format byte-identical; `Deserialize` routed through `new` (parse-don't-validate at tracking-file load).
- **A4: `marketplace` field on `InstallPluginResult`.** One-line struct addition; populated by `install_plugin` orchestrator.

Migration scope (per design):
- 4 tracking-file meta types (`InstalledSkillMeta`, `InstalledAgentMeta`, `InstalledNativeCompanionsMeta`, `InstalledSteeringMeta`) — parse-validated at \`serde_json::from_slice\`
- `InstalledPluginInfo` and the `installed_plugins` aggregator
- `KiroProject` removal API: \`remove_plugin\`, \`remove_native_companions_for_plugin\`, plus internal install paths and free helpers
- `MarketplaceService` install API: \`install_plugin\`, \`install_skills\`, \`install_plugin_steering\`, \`install_plugin_agents\`, \`resolve_plugin_install_context\`
- `AgentInstallContext` + `SteeringInstallContext`
- `SteeringError::PathOwnedByOtherPlugin.owner`
- Tauri command \`_impl\`s: replaces I10 \`validate_name(...)?\` with \`MarketplaceName::new(...)?\`
- CLI install command: constructs newtypes from clap-parsed strings

## What's NOT in scope (deferred)

- A2 \`RemovePluginResult\` shape symmetry (drop \`_count\`, return \`Vec<String>\`) — bundle with Phase 2 UI work
- A3 \`InstallAgentsResult\` dual-track collapse — annotated as legacy-presenter scaffolding, low conviction
- HashMap-key newtypes (\`SkillName\`, etc.) — keys stay \`String\`; PR #94's I9 walkers still validate
- \`MarketplaceService\` non-install API (\`add\`, \`remove\`, \`update\`, \`list\`, \`marketplace_path\`, etc.) and \`MarketplaceAddResult.name\` — out of scope per Phase 1.5 design
- Frontend nominal-type migration — \`bindings.ts\` emits aliases but BrowseTab/InstalledTab stay \`string\`-typed

## Test plan

- [x] \`cargo fmt --all --check\` clean
- [x] \`cargo clippy --workspace --tests -- -D warnings\` clean
- [x] \`cargo test --workspace\` all green (including ~16 new newtype tests)
- [x] \`cargo xtask plan-lint\` all 6 sub-gates OK
- [x] \`npm run check\` clean (frontend types via regenerated bindings.ts)
- [x] Manual verification: \`InstallPluginResult\` JSON-shape rstests confirm \`marketplace\` and \`plugin\` serialize as plain strings (\`serde(transparent)\` contract holds)
- [ ] Manual smoke against \`dwalleck/kiro-starter-kit\`: install kiro-code-reviewer, verify both A4 marketplace field flows through and the swap-arg compile errors prevented in core would also surface in any future caller

## References

- Design: \`docs/plans/2026-04-30-phase-1-5-type-safety-design.md\`
- Plan: \`docs/plans/2026-04-30-phase-1-5-type-safety-plan.md\`
- Predecessor: PR #94 (Phase 1, plugin-first install)
- 8-reviewer aggregated review on PR #94 — Critical convergent finding: swap-arg footgun

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

---

## Self-review

### Spec coverage

| Design section | Covered by |
|---|---|
| New types (`MarketplaceName`, `PluginName`) | Task 1 |
| Test helpers (`mp`, `pn`) | Task 2 |
| Tracking-file meta type migration | Task 3 (steps 1, 6) |
| `InstalledPluginInfo` + `installed_plugins` migration | Task 3 (step 2) |
| `KiroProject` removal API migration | Task 3 (step 3) |
| `KiroProject` install API + free helpers migration | Task 3 (step 4) |
| `SteeringInstallContext` + `SteeringError` migration | Task 4 |
| `AgentInstallContext` migration | Task 5 (step 1) |
| `InstallPluginResult` migration + A4 `marketplace` field | Task 5 (step 2) |
| `MarketplaceService` install API migration | Task 5 (steps 3-5) |
| `resolve_plugin_install_context` migration | Task 6 |
| Tauri `_impl` migration (5 files) | Task 7 |
| CLI install command migration | Task 8 (step 1) |
| Integration tests + bindings.ts regen | Task 8 (steps 2-3) |
| 5-gates plan-review checklist | Task 8 (step 4) |
| Final sweep + open PR | Task 8 (steps 5-7) |

All design requirements have a task. The "open question" from the design doc (whether to derive `Default`) is resolved in Task 5 step 2 — drop `Default` from `InstallPluginResult`.

### Placeholder scan

- No "TBD" / "TODO" / "implement later" entries.
- Every code block contains the actual code (signatures, body fragments, full structs where applicable).
- Test code blocks include real `assert_eq!`s, not placeholder assertions.
- Commands include exact paths and expected output.

### Type consistency

- `MarketplaceName` and `PluginName` use the same shape (verified via Task 1's spec).
- `mp()` / `pn()` test helpers consistent across Tasks 3-8.
- `InstallPluginResult.marketplace` (Task 5 step 2) is consistent with the design's struct definition.
- `KiroProject::remove_plugin(&MarketplaceName, &PluginName)` argument order is preserved across Tasks 3 and 7 (the Tauri `_impl` calls match the core signature).
- `SteeringInstallContext.marketplace: &MarketplaceName` (Task 4) matches the consumer in Task 5 step 3 (where `install_plugin` constructs it).

### Open follow-ups (post-merge)

- Apply the 5-gates plan-review checklist on this plan (per Task 8 step 4) and write `2026-04-30-phase-1-5-type-safety-plan-amendments.md` if any gate fires. The checklist explicitly says to do this BEFORE implementation; for Phase 1.5 the discipline is the same as Phase 1.

---

**Plan complete.** Suggested execution: subagent-driven, one task per subagent, two-stage review between tasks. Tasks 3 and 5 are the heavy lifts; the rest are straightforward.

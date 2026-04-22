# Design: Tauri-side `load_plugin_manifest` dedup (#33)

**Date:** 2026-04-21
**Issue:** [#33 — Tauri-side load_plugin_manifest duplicates core; replace with core call](https://github.com/dwalleck/kiro-control-center/issues/33)
**Branch:** `fix/tauri-load-plugin-manifest-dedup`

---

## Problem

`crates/kiro-control-center/src-tauri/src/commands/browse.rs` maintains two helper functions — `load_plugin_manifest` (line 356) and `discover_skills_for_plugin` (line 333) — that duplicate the core versions in `crates/kiro-market-core/src/service/browse.rs`. The Tauri versions translate errors into `CommandError` strings; the core versions return typed `PluginError` variants. Any future hardening (symlink policy tweak, manifest-version parse change) must land twice or the two paths silently diverge.

The two Tauri-local helpers have a single remaining caller: the `install_skills` Tauri command's 22-line preamble (`browse.rs:206-228`), which resolves `plugin_dir`, loads `plugin.json`, and enumerates skill directories before calling the core service. (The other former caller, `count_plugin_skills`, was removed when `MarketplaceService::count_skills_for_plugin` landed in core.)

---

## Approved design decisions (from brainstorm)

| # | Decision | Choice |
|---|---|---|
| 1 | Design approach | **Wrapper method.** Add `MarketplaceService::resolve_plugin_install_context` to core. Core helpers stay module-private; they become implementation details of the new method. |
| 2 | Method signature | **Name-based lookup:** `resolve_plugin_install_context(&self, marketplace: &str, plugin: &str) -> Result<PluginInstallContext, Error>`. Mirrors `list_skills_for_plugin`'s shape. Called once per Tauri invocation, so no batch-efficiency concern. |
| 3 | Return type shape | **Minimal:** `PluginInstallContext { version: Option<String>, skill_dirs: Vec<PathBuf> }`. Exactly what `install_skills` downstream needs. `plugin_dir` is a preamble-internal value and stays hidden. |

Rejected alternatives:

- **Expose `load_plugin_manifest` / `discover_skills_for_plugin` as `pub`.** Publicizes helpers that carry specific security hardening (symlink refusal on `plugin.json`) that shouldn't be re-derived by other callers. Encourages future consumers to assemble their own preambles in unintended ways.
- **Thick `install_skills_for_plugin` method that wraps the whole install flow.** Over-engineered for a single Tauri caller; the CLI doesn't benefit (it already drives `install_skills` via its own flow with pre-resolved `skill_dirs`).
- **Extended `PluginInstallContext` including `plugin_dir`.** Speculative use case — no current caller needs it. Adding fields later is friction-free since the type is Rust-internal.
- **Flat tuple return.** Works but makes destructuring order load-bearing; a named struct is safer.

---

## Rust core

### New type: `PluginInstallContext`

Location: `crates/kiro-market-core/src/service/browse.rs`, placed near the existing response types (`BulkSkillsResult`, `PluginSkillsResult`, `SkillCount`), before the `// Service methods` divider.

```rust
/// Inputs that [`MarketplaceService::install_skills`] needs for a
/// single-plugin install, resolved from a `(marketplace, plugin)` pair.
///
/// Constructed by [`MarketplaceService::resolve_plugin_install_context`].
/// Rust-internal only — never crosses the FFI boundary, so no `Serialize`
/// or `specta::Type` derive. The type is `pub` so frontend handlers can
/// hold onto the resolved inputs between the context-resolution call and
/// the install call without pulling the preamble logic back into each
/// handler.
#[derive(Clone, Debug)]
pub struct PluginInstallContext {
    pub version: Option<String>,
    pub skill_dirs: Vec<PathBuf>,
}
```

**Properties:**
- `pub` so the method can return it.
- `Clone + Debug` — minimal useful derive set; no `PartialEq` needed because no test compares instances.
- No `Serialize` / `specta::Type` — never crosses the Tauri FFI.
- Two public fields named for the `install_skills` arguments they supply (`version: Option<&str>`, `skill_dirs: &[PathBuf]`).

### New method: `MarketplaceService::resolve_plugin_install_context`

Location: same `impl MarketplaceService` block as `list_skills_for_plugin`, `list_all_skills`, and `count_skills_for_plugin` in `service/browse.rs`. Co-location means the module-private `load_plugin_manifest` and `discover_skills_for_plugin` helpers are directly callable without any visibility change.

```rust
/// Resolve the inputs [`Self::install_skills`] needs for a single plugin.
///
/// Performs the registry lookup, plugin-directory resolution,
/// `plugin.json` load, and skill-directory enumeration that Tauri
/// and CLI callers previously assembled by hand.
///
/// # Errors
///
/// - [`Error::Marketplace`] / [`Error::Io`] / [`Error::Json`] from
///   [`Self::list_plugin_entries`] (unknown marketplace, corrupt or
///   unreadable registry).
/// - [`Error::Plugin`] ([`PluginError::NotFound`]) if `plugin` is not
///   in the marketplace.
/// - [`Error::Plugin`] ([`PluginError::DirectoryMissing`] /
///   [`PluginError::NotADirectory`] / [`PluginError::SymlinkRefused`] /
///   [`PluginError::DirectoryUnreadable`] /
///   [`PluginError::RemoteSourceNotLocal`]) from
///   [`Self::resolve_local_plugin_dir`].
/// - [`Error::Plugin`] ([`PluginError::InvalidManifest`] /
///   [`PluginError::ManifestReadFailed`]) from [`load_plugin_manifest`]
///   if `plugin.json` is present but malformed or unreadable.
///
/// All errors propagate rather than fold into a partial-success shape
/// — the caller explicitly asked to install this plugin, so missing
/// directories, malformed manifests, and remote sources are hard
/// failures, not skips.
pub fn resolve_plugin_install_context(
    &self,
    marketplace: &str,
    plugin: &str,
) -> Result<PluginInstallContext, Error> {
    let marketplace_path = self.marketplace_path(marketplace);
    let plugin_entries = self.list_plugin_entries(marketplace)?;
    let plugin_entry = plugin_entries
        .iter()
        .find(|p| p.name == plugin)
        .ok_or_else(|| {
            Error::Plugin(PluginError::NotFound {
                plugin: plugin.to_owned(),
                marketplace: marketplace.to_owned(),
            })
        })?;
    let plugin_dir = self.resolve_local_plugin_dir(plugin_entry, &marketplace_path)?;
    let manifest = load_plugin_manifest(&plugin_dir)?;
    let version = manifest.as_ref().and_then(|m| m.version.clone());
    let skill_dirs = discover_skills_for_plugin(&plugin_dir, manifest.as_ref());
    Ok(PluginInstallContext { version, skill_dirs })
}
```

**Design notes:**
- `NotFound` construction matches `list_skills_for_plugin`'s existing pattern verbatim. A future classifier that projects `PluginError::NotFound` into a skip reason would need to enumerate both sites; that's the price of consistency with the established pattern.
- Every inner call uses `?`, so `#[source]` chains propagate intact to `.source()` walkers — CLAUDE.md compliant.
- Doc comment is WHY-focused per the CLAUDE.md Code Comments rule: explains the caller relationship and the hard-failure propagation policy vs. browse-path skips.
- The CLI's `install` command has its own `load_plugin_manifest` duplicate at `crates/kiro-market/src/commands/install.rs:396` — **out of scope** for this PR. A follow-up could converge it on the same method.

---

## Tauri collapse

### `install_skills` preamble shrinks from 22 lines to 4

File: `crates/kiro-control-center/src-tauri/src/commands/browse.rs:199-240`.

```rust
pub async fn install_skills(
    marketplace: String,
    plugin: String,
    skills: Vec<String>,
    force: bool,
    project_path: String,
) -> Result<InstallSkillsResult, CommandError> {
    let svc = make_service()?;
    let ctx = svc
        .resolve_plugin_install_context(&marketplace, &plugin)
        .map_err(CommandError::from)?;
    let project = KiroProject::new(PathBuf::from(&project_path));
    Ok(svc.install_skills(
        &project,
        &ctx.skill_dirs,
        &InstallFilter::Names(&skills),
        InstallMode::from(force),
        &marketplace,
        &plugin,
        ctx.version.as_deref(),
    ))
}
```

The existing `CommandError::from(Error)` mapping at `src-tauri/src/error.rs` already handles every `PluginError` variant the new method propagates (including `NotFound`, the directory failures, and the manifest failures). No FFI-level error-translation work needed.

### Deletions in `src-tauri/src/commands/browse.rs`

Verified zero-caller state after the preamble collapse:

- `fn load_plugin_manifest` (line 356, ~80 lines including doc comment and match arms) — delete entirely.
- `fn discover_skills_for_plugin` (line 333, ~15 lines) — delete entirely.
- `use kiro_market_core::plugin::{discover_skill_dirs, PluginManifest}` — drop `discover_skill_dirs` (only used by the deleted `discover_skills_for_plugin`). `PluginManifest` may still be needed elsewhere; grep post-deletion.
- Any `tracing` imports (`debug`, `warn`) that become unused.

`cargo clippy --workspace --tests -- -D warnings` will flag any unused import that slips through.

### Regression guard

Before deleting, confirm both helpers have zero callers:

```bash
grep -n 'load_plugin_manifest\|discover_skills_for_plugin' crates/kiro-control-center/src-tauri/src/commands/browse.rs
```

Expected after the `install_skills` edit: only the function definitions themselves appear. No call sites.

---

## Tests

All Rust tests in `crates/kiro-market-core/src/service/browse.rs` alongside the existing `count_skills_for_plugin` tests. Reuse the fixtures (`temp_service`, `make_plugin_with_skills`, `relative_path_entry`) and marketplace-registration helpers that `list_skills_for_plugin` tests already use.

### Layer 1: `resolve_plugin_install_context` behavior

Seven tests:

| Test | Setup | Expected |
|---|---|---|
| `resolve_plugin_install_context_returns_context_for_local_plugin` | Registered marketplace; tempdir plugin with `plugin.json` (`{"name": "p", "version": "1.2.3"}`) + 3 skill dirs | `Ok({ version: Some("1.2.3"), skill_dirs: [3 paths] })` |
| `resolve_plugin_install_context_returns_empty_skill_dirs_when_no_skills` | Registered marketplace; tempdir plugin with `plugin.json`, no skills/ | `Ok({ version: Some(_), skill_dirs: [] })` |
| `resolve_plugin_install_context_returns_none_version_when_manifest_has_no_version` | Registered marketplace; tempdir plugin with `{"name": "p"}` (no version field) + skills | `Ok({ version: None, skill_dirs: [N] })` |
| `resolve_plugin_install_context_errors_on_unknown_marketplace` | Call with a marketplace name that isn't registered | `Err(Error::Marketplace(_))` — the inner variant is a `list_plugin_entries` implementation detail; pin only the top-level shape, matching the sibling `list_skills_for_plugin_unknown_marketplace_errors` test |
| `resolve_plugin_install_context_errors_on_plugin_not_found` | Registered marketplace, nonexistent plugin name | `Err(Error::Plugin(PluginError::NotFound { .. }))` |
| `resolve_plugin_install_context_errors_on_missing_plugin_dir` | Registered marketplace; registry entry points at a path that doesn't exist on disk | `Err(Error::Plugin(PluginError::DirectoryMissing { .. }))` |
| `resolve_plugin_install_context_errors_on_malformed_plugin_json` | Registered marketplace; tempdir plugin with `plugin.json` containing `"{not json"` | `Err(Error::Plugin(PluginError::InvalidManifest { .. }))` |

Test-body style matches `count_skills_for_plugin_*`: `assert!(matches!(...), "expected X, got: {result:?}")`.

### Not tested (same rationale as #32)

- **`ManifestReadFailed` / `DirectoryUnreadable`**: chmod-based / root-aware; portability story is ugly. Already pinned at the `PluginError` layer via `SkippedReason::from_plugin_error` tests.
- **`SymlinkRefused` on `plugin_dir`**: `resolve_local_plugin_dir`'s own test suite covers this. No need to re-pin at the wrapper layer.
- **`RemoteSourceNotLocal`**: `resolve_local_plugin_dir` produces this for `Structured` sources; already pinned at that layer.
- **Tauri-layer `install_skills` integration**: no pre-existing Tauri-command test harness (same blocker as #32's Layer 3). Core-crate tests are authoritative.
- **CLI-side `install`**: out of scope; not touched by this PR.

---

## Out of scope

1. **CLI-side `load_plugin_manifest` dedup** at `crates/kiro-market/src/commands/install.rs:396`. Separate duplicate in a different crate; the issue text is explicit about the Tauri-side dedup. Could be filed as a follow-up.
2. **Exposing `load_plugin_manifest` / `discover_skills_for_plugin` as `pub`.** They stay module-private; the new wrapper method is the only external entry point.
3. **Changing `install_skills`'s signature or its `#[allow(clippy::too_many_arguments)]`.** The 6-positional-arg shape and the allow directive are issue #34's scope.
4. **Regenerating `bindings.ts`.** No FFI-type changes (the new struct never crosses specta).
5. **Any UI changes.** `install_skills` returns the same `InstallSkillsResult` as before; frontend rendering unchanged.

---

## Acceptance criteria

- [ ] `PluginInstallContext` struct added in `service/browse.rs` with `version: Option<String>` + `skill_dirs: Vec<PathBuf>`; `Clone + Debug`; no serde/specta derives.
- [ ] `MarketplaceService::resolve_plugin_install_context(&str, &str) -> Result<PluginInstallContext, Error>` added in the `impl` block with the other `*_for_plugin` methods.
- [ ] Tauri `install_skills` preamble collapses to one `resolve_plugin_install_context` call + one `install_skills` call.
- [ ] Tauri-local `load_plugin_manifest` and `discover_skills_for_plugin` deleted from `src-tauri/src/commands/browse.rs`. Unused imports cleaned up.
- [ ] Seven new behavior tests added (per the Layer 1 table).
- [ ] No new `#[allow(...)]` directives (CLAUDE.md zero-tolerance).
- [ ] `cargo fmt --all --check`, `cargo test --workspace`, `cargo clippy --workspace --tests -- -D warnings`, `npm run check` all pass.
- [ ] No stale issue-number references (`#30`, `#32`, `#33`) in new doc comments — describe the pattern, not the ticket (CLAUDE.md comment rule).
- [ ] New and touched comments respect the new CLAUDE.md Code Comments rule: default no-comment; WHY-not-WHAT; no task-reference / changelog / commented-out-dead-code comments; delete rule-violating comments found inside the diff.

---

## Related

- **#32** — established `count_skills_for_plugin` as the sibling core method and removed the Tauri-local `count_plugin_skills` that was blocking this dedup.
- **#34** — Clean up pre-existing `#[allow(clippy::too_many_arguments)]` on `install_skills`. This PR leaves the allow in place; #34's scope.
- **Follow-up (unfiled)**: CLI-side `load_plugin_manifest` dedup at `crates/kiro-market/src/commands/install.rs:396`.

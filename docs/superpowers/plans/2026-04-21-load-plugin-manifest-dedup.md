# Tauri `load_plugin_manifest` Dedup Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the `install_skills` Tauri command's 22-line preamble (which duplicates core logic via two Tauri-local helpers) with a single call to a new `MarketplaceService::resolve_plugin_install_context` method; delete the Tauri-local duplicates afterward.

**Architecture:** Add one Rust struct (`PluginInstallContext`, `pub` but Rust-internal — no serde/specta) and one service method in `kiro-market-core::service::browse`. The method performs the registry lookup → plugin-dir resolution → manifest load → skill-dir enumeration the Tauri preamble used to do inline, and returns the two values `install_skills` actually needs (`version: Option<String>`, `skill_dirs: Vec<PathBuf>`). The core method reuses the existing module-private `load_plugin_manifest` and `discover_skills_for_plugin` helpers — no visibility changes.

**Tech Stack:** Rust 2024 (edition, 1.85.0), `thiserror`, `tracing`, `tempfile` (test fixtures). No Tauri-side additions — only deletions.

**Spec reference:** `docs/superpowers/specs/2026-04-21-load-plugin-manifest-dedup-design.md` (committed as `c034ace`).

**Branch:** `fix/tauri-load-plugin-manifest-dedup`, off current `main`.

**Out of scope (per spec):** CLI-side `load_plugin_manifest` at `crates/kiro-market/src/commands/install.rs:396` — a separate duplicate in a different crate, left for a future follow-up. No bindings.ts regeneration (the new struct never crosses the FFI boundary).

---

## File structure

Files touched by this plan:

| File | Responsibility |
|---|---|
| `crates/kiro-market-core/src/service/browse.rs` | Add `PluginInstallContext` struct (after `SkillCount`, before the `// Service methods` divider at line 359) and `resolve_plugin_install_context` method (inside `impl MarketplaceService`, before the closing `}` at line 635, after `count_skills_for_plugin`). Add seven tests at the end of the `#[cfg(test)] mod tests` block (after the existing `count_skills_for_plugin_*` tests). |
| `crates/kiro-control-center/src-tauri/src/commands/browse.rs` | Collapse `install_skills` preamble at line 199. Delete `discover_skills_for_plugin` at line 333 and `load_plugin_manifest` at line 356. Clean up unused imports. |

No new files. No FFI changes. No `bindings.ts` regeneration.

---

## Preflight

- [ ] **Step 0.1: Verify branch**

Run: `git branch --show-current`
Expected: `fix/tauri-load-plugin-manifest-dedup`

- [ ] **Step 0.2: Verify starting commit**

Run: `git log -1 --oneline`
Expected starts with: `c034ace docs(spec): design for #33`

- [ ] **Step 0.3: Verify branch is based on current `main`**

Run: `git log origin/main..HEAD --oneline`
Expected: exactly one commit — `c034ace docs(spec): design for #33 Tauri load_plugin_manifest dedup`.

- [ ] **Step 0.4: Confirm pre-implementation test baseline**

Run: `cargo test -p kiro-market-core --lib service::browse 2>&1 | tail -5`
Expected: `test result: ok. <N> passed; 0 failed` (record `<N>` — the new tests will add 7 to this count).

---

## Task 1: Add `PluginInstallContext` + `resolve_plugin_install_context` + behavior tests

**Files:**
- Modify: `crates/kiro-market-core/src/service/browse.rs`

All struct, method, and tests land in one commit because the struct exists only as the method's return type, and the method's only behavior is to produce the struct — they are one logical unit.

- [ ] **Step 1.1: Write the first failing happy-path test**

Open `crates/kiro-market-core/src/service/browse.rs`. Find the end of the `#[cfg(test)] mod tests` block (closing `}` at line 2373). Immediately before that closing `}`, insert:

```rust
    // -----------------------------------------------------------------------
    // resolve_plugin_install_context
    // -----------------------------------------------------------------------

    #[test]
    fn resolve_plugin_install_context_returns_context_for_local_plugin() {
        let (dir, svc) = temp_service();
        let entries = vec![relative_path_entry("myplugin", "plugins/myplugin")];
        let marketplace_path = seed_marketplace_with_registry(dir.path(), &svc, "mp1", &entries);
        make_plugin_with_skills(&marketplace_path, "myplugin", &["alpha", "beta", "gamma"]);
        fs::write(
            marketplace_path.join("plugins").join("myplugin").join("plugin.json"),
            br#"{"name": "myplugin", "version": "1.2.3"}"#,
        )
        .expect("write plugin.json");

        let ctx = svc
            .resolve_plugin_install_context("mp1", "myplugin")
            .expect("happy path");
        assert_eq!(ctx.version.as_deref(), Some("1.2.3"));
        assert_eq!(ctx.skill_dirs.len(), 3);
    }
```

- [ ] **Step 1.2: Run the test to confirm it fails**

Run: `cargo test -p kiro-market-core --lib service::browse::tests::resolve_plugin_install_context_returns_context_for_local_plugin 2>&1 | tail -10`
Expected: compile error — either `cannot find function resolve_plugin_install_context` or the equivalent missing-method error on `MarketplaceService`.

- [ ] **Step 1.3: Add the `PluginInstallContext` struct**

In the same file, insert the following **between lines 357 and 359** — that is, immediately after the closing `}` of `SkillCount::ManifestFailed { reason: SkippedReason }`, and immediately before the `// ---------------------------------------------------------------------------` divider that introduces `// Service methods`:

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

- [ ] **Step 1.4: Add the `resolve_plugin_install_context` method**

In the same file, find the `impl MarketplaceService` block that opens at line 363. Its closing `}` is at line 635. The last method inside it (`count_skills_for_plugin`) ends around line 634. Insert the following **immediately before** the closing `}` at line 635 (i.e. after `count_skills_for_plugin` and before `}`), indented to match the other methods in the impl block:

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

- [ ] **Step 1.5: Run the first test to confirm it passes**

Run: `cargo test -p kiro-market-core --lib service::browse::tests::resolve_plugin_install_context_returns_context_for_local_plugin 2>&1 | tail -10`
Expected: `test result: ok. 1 passed`.

- [ ] **Step 1.6: Add the remaining six behavior tests**

In the same `#[cfg(test)] mod tests` block, immediately after the test you added in Step 1.1 (and before the module's closing `}`), append the following six tests:

```rust
    #[test]
    fn resolve_plugin_install_context_returns_empty_skill_dirs_when_no_skills() {
        let (dir, svc) = temp_service();
        let entries = vec![relative_path_entry("lonely", "plugins/lonely")];
        let marketplace_path = seed_marketplace_with_registry(dir.path(), &svc, "mp1", &entries);
        let plugin_dir = marketplace_path.join("plugins").join("lonely");
        fs::create_dir_all(&plugin_dir).expect("create plugin dir");
        fs::write(
            plugin_dir.join("plugin.json"),
            br#"{"name": "lonely", "version": "0.1.0"}"#,
        )
        .expect("write plugin.json");

        let ctx = svc
            .resolve_plugin_install_context("mp1", "lonely")
            .expect("happy path");
        assert_eq!(ctx.version.as_deref(), Some("0.1.0"));
        assert!(ctx.skill_dirs.is_empty());
    }

    #[test]
    fn resolve_plugin_install_context_returns_none_version_when_manifest_has_no_version() {
        let (dir, svc) = temp_service();
        let entries = vec![relative_path_entry("nover", "plugins/nover")];
        let marketplace_path = seed_marketplace_with_registry(dir.path(), &svc, "mp1", &entries);
        make_plugin_with_skills(&marketplace_path, "nover", &["one"]);
        fs::write(
            marketplace_path.join("plugins").join("nover").join("plugin.json"),
            br#"{"name": "nover"}"#,
        )
        .expect("write plugin.json");

        let ctx = svc
            .resolve_plugin_install_context("mp1", "nover")
            .expect("happy path");
        assert!(ctx.version.is_none(), "expected no version, got: {:?}", ctx.version);
        assert_eq!(ctx.skill_dirs.len(), 1);
    }

    #[test]
    fn resolve_plugin_install_context_errors_on_unknown_marketplace() {
        let (_dir, svc) = temp_service();
        let err = svc
            .resolve_plugin_install_context("does-not-exist", "anyplugin")
            .expect_err("unknown marketplace must error");
        // The inner MarketplaceError variant is an implementation detail
        // of list_plugin_entries; pin only the top-level Error::Marketplace
        // shape, matching the sibling list_skills_for_plugin_unknown_marketplace_errors
        // test.
        assert!(
            matches!(err, Error::Marketplace(_)),
            "expected Error::Marketplace, got: {err:?}"
        );
    }

    #[test]
    fn resolve_plugin_install_context_errors_on_plugin_not_found() {
        let (dir, svc) = temp_service();
        let entries = vec![relative_path_entry("alpha", "plugins/alpha")];
        seed_marketplace_with_registry(dir.path(), &svc, "mp1", &entries);

        let err = svc
            .resolve_plugin_install_context("mp1", "does-not-exist")
            .expect_err("unknown plugin must error");
        assert!(
            matches!(
                err,
                Error::Plugin(PluginError::NotFound { ref plugin, .. })
                    if plugin == "does-not-exist"
            ),
            "expected Plugin::NotFound, got: {err:?}"
        );
    }

    #[test]
    fn resolve_plugin_install_context_errors_on_missing_plugin_dir() {
        let (dir, svc) = temp_service();
        // Registry entry claims the plugin lives at "plugins/ghost", but
        // the directory is never created — the resolver must surface
        // DirectoryMissing rather than silently falling back to defaults.
        let entries = vec![relative_path_entry("ghost", "plugins/ghost")];
        seed_marketplace_with_registry(dir.path(), &svc, "mp1", &entries);

        let err = svc
            .resolve_plugin_install_context("mp1", "ghost")
            .expect_err("missing plugin_dir must error");
        assert!(
            matches!(err, Error::Plugin(PluginError::DirectoryMissing { .. })),
            "expected Plugin::DirectoryMissing, got: {err:?}"
        );
    }

    #[test]
    fn resolve_plugin_install_context_errors_on_malformed_plugin_json() {
        let (dir, svc) = temp_service();
        let entries = vec![relative_path_entry("broken", "plugins/broken")];
        let marketplace_path = seed_marketplace_with_registry(dir.path(), &svc, "mp1", &entries);
        let plugin_dir = marketplace_path.join("plugins").join("broken");
        fs::create_dir_all(&plugin_dir).expect("create plugin dir");
        fs::write(plugin_dir.join("plugin.json"), b"{not json").expect("write plugin.json");

        let err = svc
            .resolve_plugin_install_context("mp1", "broken")
            .expect_err("malformed plugin.json must error");
        assert!(
            matches!(err, Error::Plugin(PluginError::InvalidManifest { .. })),
            "expected Plugin::InvalidManifest, got: {err:?}"
        );
    }
```

Note: there are only six tests here because Step 1.1 already added the seventh (the happy path). Running all seven as a group in Step 1.7 will confirm the full set.

- [ ] **Step 1.7: Run all `resolve_plugin_install_context` tests**

Run: `cargo test -p kiro-market-core --lib service::browse::tests::resolve_plugin_install_context 2>&1 | tail -15`
Expected: `test result: ok. 7 passed`.

- [ ] **Step 1.8: Run the full `service::browse` test module to check for regressions**

Run: `cargo test -p kiro-market-core --lib service::browse 2>&1 | tail -5`
Expected: all tests pass, exceeding Preflight baseline `<N>` by 7.

- [ ] **Step 1.9: Run clippy**

Run: `cargo clippy -p kiro-market-core --tests -- -D warnings 2>&1 | tail -10`
Expected: no warnings or errors.

- [ ] **Step 1.10: Commit**

```bash
git add crates/kiro-market-core/src/service/browse.rs
git commit -m "$(cat <<'EOF'
feat(core): add MarketplaceService::resolve_plugin_install_context

Replaces the Tauri install command's 22-line preamble with a single
core call. The new method wraps the registry lookup, plugin-dir
resolution, plugin.json load, and skill-dir enumeration that both
Tauri and CLI callers had assembled by hand. Returns a minimal
PluginInstallContext { version, skill_dirs } — exactly what
`install_skills` needs downstream. Module-private load_plugin_manifest
and discover_skills_for_plugin helpers stay private; they become
implementation details of the new method.

Part of #33.
EOF
)"
```

---

## Task 2: Collapse Tauri `install_skills` preamble + delete duplicate helpers

**Files:**
- Modify: `crates/kiro-control-center/src-tauri/src/commands/browse.rs`

- [ ] **Step 2.1: Collapse the `install_skills` preamble**

Open `crates/kiro-control-center/src-tauri/src/commands/browse.rs`. Find the `install_skills` function at line 199. Its body currently runs from roughly line 206 to 239. Replace the entire preamble (lines 206 through 228) and the final `svc.install_skills(...)` call (lines 231 through 239) — that is, the whole function body — with this shorter version:

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

Preserve the `#[tauri::command]` / `#[specta::specta]` attributes and the doc comment above the function unchanged (they describe the command's contract, not the preamble that's being collapsed).

- [ ] **Step 2.2: Confirm zero remaining callers of the two Tauri-local helpers**

Run: `grep -n 'load_plugin_manifest\|discover_skills_for_plugin' crates/kiro-control-center/src-tauri/src/commands/browse.rs`
Expected: only the function definitions themselves appear (the `fn discover_skills_for_plugin(` line and the `fn load_plugin_manifest(` line, plus any doc-comment mentions like `/// Matches the hardening in ... load_plugin_manifest`). No call-site references.

If any call site appears, STOP and investigate — a caller was missed in Step 2.1.

- [ ] **Step 2.3: Delete `fn discover_skills_for_plugin`**

In the same file, delete the entire `discover_skills_for_plugin` function starting at line 333, including its doc comment. The function spans from the `///` comment line immediately above `fn discover_skills_for_plugin(` through the closing `}` of the function body (roughly 10–15 lines total).

- [ ] **Step 2.4: Delete `fn load_plugin_manifest`**

In the same file, delete the entire `load_plugin_manifest` function (originally at line 356, shifted up after Step 2.3). Delete its doc comment block and the whole function body — everything from the `/// Load a plugin.json from the given directory.` doc-comment start through the closing `}` of the function.

- [ ] **Step 2.5: Clean up unused imports**

After Task 2.3–2.4, several imports likely become unused. Run `cargo check -p kiro-control-center 2>&1 | tail -20` to see what clippy flags.

Candidates to remove (verify each via `cargo check` before removing — some may still be used by other functions in the file):
- `use std::fs;` — `load_plugin_manifest` was the only user.
- `use std::path::Path;` — `discover_skills_for_plugin` took `&Path`. `PathBuf` likely stays.
- `use kiro_market_core::plugin::{discover_skill_dirs, PluginManifest};` — `discover_skill_dirs` was the inner helper of `discover_skills_for_plugin`; `PluginManifest` was `load_plugin_manifest`'s return type.
- `use tracing::{debug, warn};` — both were used in the deleted functions; check whether other code in the file still logs.

Remove any import the compiler flags as unused. Re-run `cargo check -p kiro-control-center` after edits to confirm clean.

- [ ] **Step 2.6: Run the full Rust workspace test suite**

Run: `cargo test --workspace 2>&1 | tail -10`
Expected: all tests pass. `install_skills`'s integration path is unchanged from the caller's perspective, so any pre-existing test should still work.

- [ ] **Step 2.7: Run workspace clippy**

Run: `cargo clippy --workspace --tests -- -D warnings 2>&1 | tail -10`
Expected: no warnings or errors.

- [ ] **Step 2.8: Run TypeScript check (no FFI change expected)**

Run: `cd crates/kiro-control-center && npm run check 2>&1 | tail -10; cd -`
Expected: 0 errors, 0 warnings. No bindings regeneration needed — `PluginInstallContext` never crosses the FFI, and `install_skills`'s Tauri signature (`marketplace: String, plugin: String, skills: Vec<String>, force: bool, project_path: String -> Result<InstallSkillsResult, CommandError>`) is unchanged.

- [ ] **Step 2.9: Commit**

```bash
git add crates/kiro-control-center/src-tauri/src/commands/browse.rs
git commit -m "$(cat <<'EOF'
refactor(tauri): replace install_skills preamble with core call

install_skills now calls MarketplaceService::resolve_plugin_install_context
instead of assembling the preamble inline. The Tauri-local
load_plugin_manifest and discover_skills_for_plugin helpers —
duplicates of the core versions — are deleted along with the imports
that only they used.

Closes #33.
EOF
)"
```

---

## Task 3: Workspace-wide verification

**Files:** no edits expected unless a formatter or linter surfaces an issue.

- [ ] **Step 3.1: Run `cargo fmt` check**

Run: `cargo fmt --all --check 2>&1 | tail -10`
Expected: no output, exit 0. If formatting issues surface:
- Run `cargo fmt --all` to fix.
- `git add` the changed files.
- `git commit -m "chore: cargo fmt"`.

- [ ] **Step 3.2: Full workspace test run**

Run: `cargo test --workspace 2>&1 | tail -15`
Expected: all tests pass, including the 7 new `resolve_plugin_install_context_*` tests from Task 1.

- [ ] **Step 3.3: Full workspace clippy run**

Run: `cargo clippy --workspace --tests -- -D warnings 2>&1 | tail -15`
Expected: no warnings or errors.

- [ ] **Step 3.4: TypeScript check**

Run: `cd crates/kiro-control-center && npm run check 2>&1 | tail -10; cd -`
Expected: 0 errors, 0 warnings.

- [ ] **Step 3.5: Confirm no new `#[allow(...)]` directives**

Run: `git diff origin/main..HEAD -- 'crates/**/*.rs' | grep -E '^\+.*#\[allow\(' || echo "clean"`
Expected output: `clean`. (Per CLAUDE.md zero-tolerance rule.)

If the grep surfaces anything, revisit the offending code and fix the root cause rather than suppressing — do not commit a new `#[allow]`.

- [ ] **Step 3.6: Confirm no new `.unwrap()` / `.expect()` in production code**

Run: `git diff origin/main..HEAD -- 'crates/**/src/**/*.rs' ':(exclude)crates/**/src/**/tests.rs' | grep -E '^\+.*\.(unwrap|expect)\(' | head -10`

Scan the output. Matches inside `#[cfg(test)] mod tests { ... }` blocks are **allowed** (tests are exempt per CLAUDE.md). Matches in production paths — reject and refactor.

Expected: either empty, or every match is visually identifiable as being inside a test block.

- [ ] **Step 3.7: Confirm no stale issue-number references in new code**

Run: `git diff origin/main..HEAD -- 'crates/**/*.rs' | grep -E '^\+.*#[0-9]+\b' | head -10`
Expected: empty, or the only matches are in commit-message-style contexts (which shouldn't be in the code at all — the commit messages carry that context).

Per the new CLAUDE.md Code Comments rule: describe the pattern, not the ticket. If any `#32` / `#33` / etc. reference appears in a doc comment added by this branch, rephrase to describe the architectural role directly.

- [ ] **Step 3.8: Inspect the commit log**

Run: `git log --oneline origin/main..HEAD`
Expected: exactly two or three commits:
1. `<sha>` `feat(core): add MarketplaceService::resolve_plugin_install_context` (Task 1)
2. `<sha>` `refactor(tauri): replace install_skills preamble with core call` (Task 2)
3. *(optional)* `<sha>` `chore: cargo fmt` (Task 3 Step 3.1, only if fmt drift surfaced)

The original spec commit `c034ace` is the branch's base, not counted in `origin/main..HEAD` after any potential post-merge rebase would apply. If `c034ace` does appear in the log, that's also expected (it's on this branch, ahead of `main`).

---

## Acceptance criteria (from spec)

Tick these off after Task 3 passes. Re-run verification if any fails.

- [ ] `PluginInstallContext` struct added in `service/browse.rs` with `version: Option<String>` + `skill_dirs: Vec<PathBuf>`; `Clone + Debug`; no serde/specta derives.
- [ ] `MarketplaceService::resolve_plugin_install_context(&str, &str) -> Result<PluginInstallContext, Error>` added in the `impl` block with the other `*_for_plugin` methods.
- [ ] Tauri `install_skills` preamble collapses to one `resolve_plugin_install_context` call + one `install_skills` call.
- [ ] Tauri-local `load_plugin_manifest` and `discover_skills_for_plugin` deleted. Unused imports cleaned up.
- [ ] Seven new behavior tests added at `service/browse.rs`'s `#[cfg(test)] mod tests` (the one from Step 1.1 + six from Step 1.6).
- [ ] No new `#[allow(...)]` directives (CLAUDE.md zero-tolerance).
- [ ] `cargo fmt --all --check`, `cargo test --workspace`, `cargo clippy --workspace --tests -- -D warnings`, `npm run check` all pass.
- [ ] No stale issue-number references (`#30`, `#32`, `#33`) in new doc comments.
- [ ] New and touched comments respect the CLAUDE.md Code Comments rule: default no-comment; WHY-not-WHAT; no task-reference / changelog / commented-out-dead-code; delete rule-violating comments found inside the diff.

---

## Notes for the implementing engineer

1. **The spec is the contract; this plan is the how.** If a step contradicts the spec, flag it in a PR comment — don't silently diverge.

2. **Do not skip the "run and watch it fail" steps** (Steps 1.2, 1.5). They confirm the test actually exercises the code under test and isn't trivially passing.

3. **`seed_marketplace_with_registry` and `make_plugin_with_skills` are existing fixtures** at `service/browse.rs:977` and `:988`. Don't re-implement them; import via `use super::*` already exists in the test module.

4. **`make_plugin_with_skills` does not write a `plugin.json`.** It only creates the skill directory tree. Tests that need a `plugin.json` (happy path + no-version-field + malformed) must write it explicitly after calling the helper.

5. **`PluginError::NotFound { plugin, marketplace }`** is the existing variant in `crates/kiro-market-core/src/error.rs`. Construct it exactly like `list_skills_for_plugin` does (see its implementation in the same file for the pattern).

6. **No bindings regeneration needed.** `PluginInstallContext` never crosses the specta boundary — it's consumed in the Tauri command's Rust body and never reaches the `PluginInfo` / `InstallSkillsResult` / other exported types. If the bindings regeneration test (`cargo test -p kiro-control-center --lib -- --ignored`) flags a change, investigate — it probably means a type you didn't intend to export is reachable from a specta-exported surface.

7. **CLI-side `load_plugin_manifest` at `crates/kiro-market/src/commands/install.rs:396` is out of scope.** Do not touch it; leave a future follow-up for the CLI-side dedup.

8. **The new type is `pub` but has no serde derives — that's deliberate.** The spec explicitly calls this out. Resist any clippy-driven temptation to add `Serialize` or `PartialEq`; they are not required and add maintenance surface for no current consumer.

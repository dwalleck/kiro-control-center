# Install ↔ Detect Symmetry Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make every `Installed*Meta` self-contained at detect time by recording the manifest scan-path install used. Detection becomes a direct lookup — no probing, no fallbacks. Closes issue #97 and the structural pattern that made #97 the third instance after PR #96 closed steering and agents via probe-fallback helpers.

**Architecture:** Add required `source_scan_root: RelativePath` to `InstalledSkillMeta` / `InstalledSteeringMeta` / `InstalledNativeCompanionsMeta`; tighten agents' existing `source_path: Option<RelativePath>` to `RelativePath`. Tighten `source_hash: Option<String>` → `String` on skills + agents (already required on steering / native_companions). Replace `scan_plugin_for_content_drift`'s probe helpers and dialect-fallback with one straight-line lookup per artifact type. Pre-existing tracking files become invalid by design — no migration story (no users).

**Tech Stack:** Rust edition 2024, `serde` (with `RelativePath` newtype enforcing forward-slash separators at construction), `thiserror`, `rstest`. No new crates. `cargo xtask plan-lint` gates stay green throughout.

**Spec:** `docs/plans/2026-05-03-install-detect-symmetry-design.md`

**Branch:** `fix/install-detect-symmetry` (worktree at `~/repos/kiro-marketplace-cli-install-detect-symmetry/`), stacked on `feat/phase-2a-update-detection`.

---

## File Structure

| File | Responsibility | Change shape |
|---|---|---|
| `crates/kiro-market-core/src/validation.rs` | `RelativePath` newtype | Add associated function `RelativePath::from_path_under(&Path, &Path) -> Result<Self, ValidationError>` for converting `Path` → `RelativePath` with forward-slash normalization. New unit test for backslash inputs. |
| `crates/kiro-market-core/src/project.rs` | Tracking schema (4 `Installed*Meta` types + on-disk `InstalledSkills` / `InstalledAgents` / `InstalledSteering` aggregates) | Add required `source_scan_root: RelativePath` to skills / steering / native_companions metas. Tighten agents `source_path` to required. Tighten skills + agents `source_hash` and `installed_hash` to required `String`. Update internal install helpers + test fixtures. |
| `crates/kiro-market-core/src/plugin.rs` | `PluginManifest` + `discover_skill_dirs` | Refactor `discover_skill_dirs` to return `Vec<DiscoveredSkill { scan_root, skill_dir }>` instead of `Vec<PathBuf>`. Update tests. |
| `crates/kiro-market-core/src/service/browse.rs` | Browse-side discovery + scan-path helpers | Update `discover_skills_for_plugin` for new `discover_skill_dirs` shape. Revert `agent_scan_paths_for_plugin` / `steering_scan_paths_for_plugin` from `pub(super)` back to private (no longer consumed outside the module). |
| `crates/kiro-market-core/src/service/mod.rs` | Service layer — install paths + `scan_plugin_for_content_drift` + helpers | Update install paths to populate new fields. Simplify `scan_plugin_for_content_drift` to direct lookup per artifact type. Delete `hash_artifact_in_scan_paths`, `agent_hash_inputs`'s dialect-fallback branch, the I-N7 actionable-error branch, the `legacy_fallback` flag and its threading, and the `manifest` parameter on `scan_plugin_for_content_drift`. Tighten `relative_source_path_for_tracking` to return `Result` (or delete in favor of `RelativePath::from_path_under` directly). |
| `crates/kiro-control-center/src/lib/bindings.ts` | Generated TS bindings | Regenerated after schema changes via `cargo test -p kiro-control-center --lib -- --ignored`. |

**Why this decomposition.** `validation.rs` is the home for `RelativePath` and its helpers; the new `from_path_under` belongs there. `project.rs` owns the tracking schema; all four schema changes are in one file so the `Cargo.toml`-level visibility surface doesn't grow. `plugin.rs` and `browse.rs` changes are confined to the skills discovery refactor (single-callsite). `service/mod.rs` is already large and concentrated; the deletions exceed the additions, so net file size drops.

**Files NOT touched:** `crates/kiro-market-core/src/error.rs` — no new error variants needed; the agents install-fails-on-bad-source-path case routes through the existing `AgentError::InstallFailed { path, source: Box<Error> }` with a synthesized inner `io::Error`. Avoids adding a public-API surface this plan doesn't otherwise require.

---

## Pre-work: confirm the worktree

- [ ] **Step 0.1: Verify the worktree state**

```bash
cd /home/dwalleck/repos/kiro-marketplace-cli-install-detect-symmetry
git status
git log --oneline -3
```

Expected:
- Branch: `fix/install-detect-symmetry`
- HEAD: `d586eb6 docs(plans): install↔detect symmetry — tracking schema foundation design`
- Working tree clean (or only untracked files unrelated to this plan).

If HEAD differs, do not proceed — the plan assumes the design doc commit is present and the branch is based on `feat/phase-2a-update-detection`.

- [ ] **Step 0.2: Confirm the baseline test suite passes**

```bash
cargo test --workspace
```

Expected: ~1000 tests pass, 0 fail. (PR #96 + the design-doc commit; nothing else.)

This baseline is what each subsequent task's "verify tests pass" step compares against. If the baseline is red, stop — don't try to fix unrelated failures as part of this plan.

---

## Task 1: Add `RelativePath::from_path_under` shared helper

**Why first.** The `path_to_relative_under_plugin` recipe is needed at 4+ sites in commits 2-5 (skills install, steering install, native_companions install, agents install). Extract first as a no-op refactor — `relative_source_path_for_tracking` becomes a thin wrapper that delegates to the new helper and preserves its current `Option<RelativePath>` return shape. Subsequent commits use the new helper directly with `Result`. This isolates the new code path under test before any callers depend on it.

**Files:**
- Modify: `crates/kiro-market-core/src/validation.rs` — add associated function `RelativePath::from_path_under`, add unit test
- Modify: `crates/kiro-market-core/src/service/mod.rs:2830-2868` (`relative_source_path_for_tracking`) — delegate to the new helper

- [ ] **Step 1.1: Read `RelativePath` to confirm the constructor signature and error type**

```bash
grep -n "impl RelativePath\|pub fn new\|ValidationError\|pub enum ValidationError" \
    crates/kiro-market-core/src/validation.rs crates/kiro-market-core/src/error.rs \
    | head -20
```

Note the `RelativePath::new` signature and which error type it returns. Steps below assume `Result<Self, ValidationError>` (the most likely shape based on the existing `validate_relative_path` siblings); adapt to the actual signature.

- [ ] **Step 1.2: Write the failing test for the new helper**

Add to the test module in `crates/kiro-market-core/src/validation.rs`:

```rust
#[test]
fn from_path_under_normalizes_backslashes() {
    use std::path::PathBuf;
    // Synthesise a path with backslash components. PathBuf is just bytes
    // underneath, so this works on any platform — tests the Windows-native
    // input shape without requiring Windows CI.
    let plugin_dir = PathBuf::from("/tmp/plugin");
    let source = PathBuf::from("/tmp/plugin/agents\\reviewer.md");
    let rel = RelativePath::from_path_under(&source, &plugin_dir)
        .expect("backslash path under plugin_dir should normalize");
    assert_eq!(rel.as_str(), "agents/reviewer.md");
}

#[test]
fn from_path_under_rejects_path_outside_base() {
    use std::path::PathBuf;
    let plugin_dir = PathBuf::from("/tmp/plugin");
    let outside = PathBuf::from("/etc/passwd");
    assert!(
        RelativePath::from_path_under(&outside, &plugin_dir).is_err(),
        "path not under plugin_dir must error, not silently produce a bogus rel"
    );
}

#[test]
fn from_path_under_round_trips_simple_unix_path() {
    use std::path::PathBuf;
    let plugin_dir = PathBuf::from("/tmp/plugin");
    let source = PathBuf::from("/tmp/plugin/agents/reviewer.md");
    let rel = RelativePath::from_path_under(&source, &plugin_dir).expect("valid input");
    assert_eq!(rel.as_str(), "agents/reviewer.md");
}
```

- [ ] **Step 1.3: Run the test to verify it fails (function doesn't exist yet)**

```bash
cargo test -p kiro-market-core --lib validation::tests::from_path_under
```

Expected: compile error — `no function or associated item named 'from_path_under' found`.

- [ ] **Step 1.4: Add the helper to `RelativePath`**

Add to the `impl RelativePath { ... }` block in `crates/kiro-market-core/src/validation.rs`:

```rust
/// Convert a `Path` to a [`RelativePath`] by stripping `base` and
/// normalising path separators to forward-slash. Returns `Err` if
/// `path` is not under `base` or the resulting relative path fails
/// `RelativePath::new` validation.
///
/// Forward-slash conversion is required because `RelativePath::new`
/// rejects backslashes for cross-platform portability of the wire
/// format (pinned by `relative_path_newtype_rejects_traversal_at_construction`).
/// On Windows, `Path::strip_prefix(...).to_string_lossy()` returns
/// backslashes; without normalisation, `RelativePath::new` fails.
///
/// # Errors
///
/// Returns `ValidationError::InvalidRelativePath` if `path` is not
/// under `base`, or whatever error `RelativePath::new` produces if
/// the normalised string fails validation.
pub fn from_path_under(path: &Path, base: &Path) -> Result<Self, ValidationError> {
    let rel = path
        .strip_prefix(base)
        .map_err(|_| ValidationError::InvalidRelativePath {
            path: path.display().to_string(),
            reason: format!(
                "path is not under base directory `{}`",
                base.display()
            ),
        })?;
    let rel_str = rel
        .components()
        .filter_map(|c| match c {
            std::path::Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/");
    Self::new(rel_str)
}
```

You may need to add `use std::path::Path;` if not already imported.

If the actual `ValidationError` variants differ from `InvalidRelativePath { path, reason }`, adapt the error construction to match the real shape. The intent is "return whatever ValidationError variant best fits 'path is not under base'."

- [ ] **Step 1.5: Run the test to verify it passes**

```bash
cargo test -p kiro-market-core --lib validation::tests::from_path_under
```

Expected: 3 tests pass.

- [ ] **Step 1.6: Refactor `relative_source_path_for_tracking` to delegate**

Replace the body of `relative_source_path_for_tracking` in `crates/kiro-market-core/src/service/mod.rs` (lines ~2830-2868) with:

```rust
fn relative_source_path_for_tracking(
    path: &Path,
    plugin_dir: &Path,
) -> Option<crate::validation::RelativePath> {
    match crate::validation::RelativePath::from_path_under(path, plugin_dir) {
        Ok(rp) => Some(rp),
        Err(e) => {
            warn!(
                path = %path.display(),
                plugin_dir = %plugin_dir.display(),
                error = %e,
                "agent path could not be expressed as RelativePath; \
                 source_path will fall back to dialect default during \
                 update detection"
            );
            None
        }
    }
}
```

The `warn!` log preserves the existing observability; only the conversion mechanism changes. The two original log sites (strip-prefix failure and RelativePath::new failure) collapse into one because `from_path_under` returns either error through the same variant.

- [ ] **Step 1.7: Run the workspace tests to verify no regression**

```bash
cargo test --workspace
```

Expected: same baseline test count as Step 0.2, all passing. The refactor is behavior-preserving (Option-returning, warn-on-failure).

- [ ] **Step 1.8: Run clippy + fmt + plan-lint**

```bash
cargo clippy --workspace --tests -- -D warnings && \
  cargo fmt --all --check && \
  TETHYS_BIN=/home/dwalleck/repos/rivets/target/release/tethys cargo xtask plan-lint
```

Expected: all clean, all 6 plan-lint gates OK.

- [ ] **Step 1.9: Commit**

```bash
git add crates/kiro-market-core/src/validation.rs \
        crates/kiro-market-core/src/service/mod.rs && \
  git commit -m "$(cat <<'EOF'
refactor(validation): extract RelativePath::from_path_under helper

No behavior change. Generalises the path-to-RelativePath conversion
recipe currently embedded in service::mod::relative_source_path_for_tracking
into an associated function on RelativePath, so the four install sites
in subsequent commits can reuse it instead of duplicating the
forward-slash normalisation. relative_source_path_for_tracking becomes
a one-line wrapper that preserves the existing warn-and-None contract
its caller depends on.

Added tests:
- backslash normalisation (synthesises a Windows-native PathBuf on
  any platform; pins the cross-platform contract)
- path-outside-base rejection
- simple Unix path round-trip

Foundation for fix/install-detect-symmetry.
EOF
)"
```

---

## Task 2: Tighten `InstalledAgentMeta.source_path` to required

**Why.** Closes the "no users → no probe-fallback" design choice for agents. Removes the dialect-fallback branch from `agent_hash_inputs`, the I-N7 actionable-error branch from `scan_plugin_for_content_drift`, and the `Option` wrapper everywhere. Install-time, `relative_source_path_for_tracking` failures become typed errors instead of warn-and-skip.

**Files:**
- Modify: `crates/kiro-market-core/src/project.rs:62-108` (`InstalledAgentMeta`) — drop `Option` from `source_path`
- Modify: `crates/kiro-market-core/src/service/mod.rs:1716-1725` — install site, propagate `from_path_under` error as `AgentError::InstallFailed`
- Modify: `crates/kiro-market-core/src/service/mod.rs:2830-2868` — delete `relative_source_path_for_tracking` (now unused; replaced by direct `RelativePath::from_path_under` calls in install sites)
- Modify: `crates/kiro-market-core/src/service/mod.rs:2792-2814` (`agent_hash_inputs`) — drop `Option` from signature, delete dialect-fallback
- Modify: `crates/kiro-market-core/src/service/mod.rs:2567-2641` (agents loop in `scan_plugin_for_content_drift`) — drop `.as_ref()`, delete I-N7 branch
- Update test fixtures across `service::tests` and `project::tests` (compiler-driven)

- [ ] **Step 2.1: Write the failing install-fails-on-bad-source-path test**

Add to `crates/kiro-market-core/src/service/mod.rs` test module (near other install-error tests):

```rust
#[test]
fn install_translated_agent_fails_when_source_path_outside_plugin_dir() {
    // The discovery layer shouldn't yield such paths in practice, but
    // when it does (defensive against future invariant breaks), the
    // install MUST surface a typed error rather than silently skipping
    // — required-field schema means "no source_path = can't install".
    use crate::service::test_support::{mp, pn};
    use std::path::PathBuf;

    let plugin_dir = PathBuf::from("/tmp/install-symmetry-test/plugin");
    // `path_outside_plugin_dir` is exactly that — a sibling, not a child.
    let outside = PathBuf::from("/tmp/install-symmetry-test/sibling/agent.md");

    // Direct test of the construction fallback path: ensure
    // RelativePath::from_path_under errors so the install site has a
    // condition to check.
    assert!(
        crate::validation::RelativePath::from_path_under(&outside, &plugin_dir).is_err(),
        "precondition: from_path_under must reject paths outside base"
    );

    // Wire through the install pipeline would require a full
    // marketplace fixture; this test pins the from_path_under
    // contract that the install site at service/mod.rs:1716 will
    // call. Actual install-pipeline coverage comes from existing
    // install_plugin_agents tests + the compiler-driven update of
    // every InstalledAgentMeta construction site.
    let _ = (mp("mp"), pn("p")); // anchor unused-import lint
}
```

- [ ] **Step 2.2: Run the test to verify it currently passes (precondition test only — Task 1 already added `from_path_under`)**

```bash
cargo test -p kiro-market-core --lib install_translated_agent_fails_when_source_path_outside_plugin_dir
```

Expected: PASS. (This is a contract-pin, not a regression-from-broken-state. The real behavior change happens at the install site below, where the compiler will force the issue once we tighten the type.)

- [ ] **Step 2.3: Tighten `InstalledAgentMeta.source_path` to required**

In `crates/kiro-market-core/src/project.rs`, find the `InstalledAgentMeta` struct (line ~62) and change:

```rust
// BEFORE
#[serde(default, skip_serializing_if = "Option::is_none")]
pub source_path: Option<RelativePath>,

// AFTER
pub source_path: RelativePath,
```

Also update the doc comment to remove the "`None` for legacy entries installed before this field was added" sentence (no legacy entries — the no-users assumption from the spec). Replace with: "Required at install time; populated via [`RelativePath::from_path_under`] at install. Drift detection at [`crate::service::MarketplaceService::scan_plugin_for_content_drift`] uses this directly to locate the source for hash recomputation."

- [ ] **Step 2.4: Build to surface every InstalledAgentMeta construction site**

```bash
cargo build -p kiro-market-core 2>&1 | head -60
```

Expected: compile errors at every site that constructs `InstalledAgentMeta` without the now-required field. From the inventory in the spec, these are at minimum:
- `crates/kiro-market-core/src/service/mod.rs:1716` (the production install site)
- `crates/kiro-market-core/src/project.rs:2605` (another install path)
- `crates/kiro-market-core/src/project.rs:3490, 3513, 3842, 4659` (test fixtures)
- `crates/kiro-market-core/src/service/mod.rs:6797, 6984, 7074` (test fixtures)

Walk every error and fix per the rules below.

- [ ] **Step 2.5: Update the production install site (service/mod.rs:1716)**

Replace the meta construction:

```rust
// BEFORE
let meta = crate::project::InstalledAgentMeta {
    marketplace: ctx.marketplace.clone(),
    plugin: ctx.plugin.clone(),
    version: ctx.version.map(String::from),
    installed_at: chrono::Utc::now(),
    dialect: def.dialect,
    source_path: relative_source_path_for_tracking(&path, plugin_dir),
    source_hash: None,
    installed_hash: None,
};

// AFTER
let source_path = match crate::validation::RelativePath::from_path_under(&path, plugin_dir) {
    Ok(rp) => rp,
    Err(e) => {
        // Discovery should always yield paths under plugin_dir; if this
        // ever fails it's a defensive catch for a future invariant
        // break. Surface as a typed install failure rather than a
        // silent skip — the required-field schema means we cannot
        // safely install without a recorded source_path.
        result.failed.push(FailedAgent {
            name: Some(def.name.clone()),
            source_path: path.clone(),
            error: crate::error::AgentError::InstallFailed {
                path: path.clone(),
                source: Box::new(crate::error::Error::Io(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!(
                        "discovered agent path `{}` is not expressible as a \
                         RelativePath under plugin_dir `{}`: {e}",
                        path.display(),
                        plugin_dir.display(),
                    ),
                ))),
            },
        });
        continue;
    }
};
let meta = crate::project::InstalledAgentMeta {
    marketplace: ctx.marketplace.clone(),
    plugin: ctx.plugin.clone(),
    version: ctx.version.map(String::from),
    installed_at: chrono::Utc::now(),
    dialect: def.dialect,
    source_path,
    source_hash: None,
    installed_hash: None,
};
```

The `continue` skips this agent — the rest of the for-loop processes other agents in the same install batch. Mirrors the existing per-agent error handling shape elsewhere in this function.

- [ ] **Step 2.6: Update the project.rs install site (project.rs:2605)**

Find the construction at line ~2605 and add a `source_path: <appropriate RelativePath value>` field. The value depends on which install path this is — read 50 lines of surrounding context to understand. If it's the same install path that already had the relative_source_path_for_tracking call somewhere upstream, propagate that source_path through. If it constructed `source_path: None`, replace with `source_path: <derived from path arg>` matching the install function's signature.

If the function's signature doesn't currently carry the source path information, this is a sign the call site needs the same kind of fix as Step 2.5 — propagate the call up to where the path is in scope.

- [ ] **Step 2.7: Update test fixtures**

For every `InstalledAgentMeta { ... }` construction site in tests (compiler will list them), add `source_path: <path>` where `<path>` is a sensible default for the test:

```rust
// Example: in tests where the agent name is "reviewer" and dialect is Native
source_path: crate::validation::RelativePath::new("agents/reviewer.json").expect("valid relative path"),
```

For Copilot / Claude dialects, mirror the per-dialect filename convention:
- Native → `agents/<name>.json`
- Claude → `agents/<name>.md`
- Copilot → `agents/<name>.agent.md`

Tests that previously asserted on `meta.source_path.is_none()` need updating — the field is no longer Option, so the assertion is dead. Either delete that assertion or replace with a positive check on the new value.

- [ ] **Step 2.8: Tighten `agent_hash_inputs` signature**

In `crates/kiro-market-core/src/service/mod.rs:2792`, replace the function with:

```rust
/// Compute the `(base_dir, filename)` pair that recreates the
/// install-side hash recipe for a tracked agent. Used by
/// [`MarketplaceService::scan_plugin_for_content_drift`] so detection
/// hashes match the bytes the install side fed into BLAKE3 — the
/// install hashes via `hash_artifact(parent, &[filename])` so detection
/// MUST also pass `(parent, filename)` rather than `(agents_dir,
/// "subdir/file.md")`, since the digest includes the rel-path bytes.
///
/// `source_path` is relative to `plugin_dir` (always present after the
/// install-detect symmetry pass — agents tracking schema requires it).
/// We split it back into `(plugin_dir + rel.parent(), rel.file_name())`
/// to match the install recipe.
fn agent_hash_inputs(
    plugin_dir: &Path,
    source_path: &crate::validation::RelativePath,
) -> (PathBuf, PathBuf) {
    let full = plugin_dir.join(source_path.as_str());
    let parent = full
        .parent()
        .map_or_else(|| plugin_dir.to_path_buf(), Path::to_path_buf);
    let fname = full
        .file_name()
        .map_or_else(|| PathBuf::from(source_path.as_str()), PathBuf::from);
    (parent, fname)
}
```

The dialect parameter and dialect-fallback branch are gone. The function signature shrinks from 4 params to 2.

- [ ] **Step 2.9: Update the agents loop in `scan_plugin_for_content_drift`**

In `crates/kiro-market-core/src/service/mod.rs` around line 2567-2641, replace the agents loop body. The current code (paraphrased):

```rust
let (base, filename) = agent_hash_inputs(plugin_dir, name, meta.dialect, meta.source_path.as_ref());
let computed = match crate::hash::hash_artifact(&base, std::slice::from_ref(&filename)) {
    Ok(h) => h,
    Err(crate::hash::HashError::ReadFailed { path, source })
        if source.kind() == std::io::ErrorKind::NotFound && meta.source_path.is_none() =>
    {
        // I-N7 actionable-error branch — 30 lines
        return Err(...);
    }
    Err(e) => return Err(e.into()),
};
```

Becomes:

```rust
let (base, filename) = agent_hash_inputs(plugin_dir, &meta.source_path);
let computed = crate::hash::hash_artifact(&base, std::slice::from_ref(&filename))?;
```

The I-N7 branch is dead (it only fired when `source_path.is_none()`). The dialect parameter is gone. The match-on-NotFound recovery is gone — any hash error propagates via `?` as a normal per-plugin failure.

- [ ] **Step 2.10: Delete `relative_source_path_for_tracking`**

The wrapper introduced in Task 1 has no remaining callers — Step 2.5 inlined `RelativePath::from_path_under` at the only production site, and there are no tests calling it directly. Verify no callers remain:

```bash
grep -rn "relative_source_path_for_tracking" crates/
```

Expected: only the function definition itself appears (the test from Task 1 that referenced it indirectly via the `from_path_under` precondition no longer needs it). Delete the entire function (`crates/kiro-market-core/src/service/mod.rs:2830-2868`).

- [ ] **Step 2.11: Delete the Copilot legacy-fallback test**

In `crates/kiro-market-core/src/service/mod.rs`, find the `detect_plugin_updates_copilot_agent_legacy_fallback` test (around line 6807) — its docstring describes itself as a deferred test for the dialect-fallback branch. Delete the entire test function (including its multi-paragraph docstring). The branch it tests is gone.

- [ ] **Step 2.12: Verify tests pass**

```bash
cargo test --workspace 2>&1 | grep "test result\|FAIL" | tail -10
```

Expected: All passes, count drops by 1 (deleted Copilot legacy test). Compiler-driven test updates in Step 2.7 cover any test fixtures that needed the new `source_path` field.

- [ ] **Step 2.13: Run clippy + fmt + plan-lint**

```bash
cargo clippy --workspace --tests -- -D warnings && \
  cargo fmt --all --check && \
  TETHYS_BIN=/home/dwalleck/repos/rivets/target/release/tethys cargo xtask plan-lint
```

Expected: clean.

- [ ] **Step 2.14: Commit**

```bash
git add -A && git commit -m "$(cat <<'EOF'
refactor(agents): tighten source_path to required, drop dialect-fallback

InstalledAgentMeta.source_path goes from Option<RelativePath> to
required RelativePath. Install site at service/mod.rs uses
RelativePath::from_path_under directly; failures (path not under
plugin_dir, RelativePath::new rejection) become typed
AgentError::InstallFailed instead of silent warn-and-None — required
field schema means we cannot safely install without a recorded
source_path.

Detection-side cleanups:
- agent_hash_inputs sheds the dialect parameter and the dialect-
  fallback branch — only path-splitting remains.
- The I-N7 actionable-error branch in scan_plugin_for_content_drift's
  agents loop (which only fired when source_path.is_none()) is dead
  code; deleted.
- relative_source_path_for_tracking is no longer called anywhere;
  deleted.
- detect_plugin_updates_copilot_agent_legacy_fallback test is deleted —
  the dialect-fallback path it exercised no longer exists.

source_hash on InstalledAgentMeta stays Option<String> for now;
tightened in commit 5 alongside the cross-cutting legacy_fallback
flag removal.

Per design docs/plans/2026-05-03-install-detect-symmetry-design.md
section "Schema changes" + "Detection simplification".
EOF
)"
```

---

## Task 3: Add `InstalledSkillMeta.source_scan_root` (closes #97)

**Why.** Closes issue #97 directly. The skills-side scan-path bug becomes a non-issue once detection looks up the install-recorded scan root instead of hardcoding `plugin_dir.join("skills")`.

**Files:**
- Modify: `crates/kiro-market-core/src/project.rs:25-51` (`InstalledSkillMeta`) — add `source_scan_root: RelativePath`
- Modify: `crates/kiro-market-core/src/plugin.rs:399-456` (`discover_skill_dirs`) — refactor to return `Vec<DiscoveredSkill>`
- Modify: `crates/kiro-market-core/src/service/browse.rs:931-942` (`discover_skills_for_plugin`) — call new shape
- Modify: `crates/kiro-market-core/src/service/mod.rs:1316-1323` — install site populates new field
- Modify: `crates/kiro-market-core/src/service/mod.rs:2510-2543` — skills loop in `scan_plugin_for_content_drift` does direct lookup; remove the "KNOWN BUG (issue #97)" comment
- Add test: `detect_plugin_updates_skills_with_custom_scan_path_no_false_drift`
- Update test fixtures across `project::tests` and `service::tests` (compiler-driven)

- [ ] **Step 3.1: Write the failing regression test (closes #97)**

Add to `crates/kiro-market-core/src/service/mod.rs` test module, next to the sibling `detect_plugin_updates_steering_with_custom_scan_path_no_false_drift`:

```rust
/// Closes #97: a plugin declaring `skills: ["./packs/"]` installs skill
/// dirs under `<plugin_dir>/packs/<name>/`. Pre-fix detection hardcoded
/// `<plugin_dir>/skills/<name>/` and would surface every such plugin as
/// a HashFailed failure on every scan. Post-fix, detection consults
/// `meta.source_scan_root` (recorded at install time) and looks at the
/// right path.
#[test]
fn detect_plugin_updates_skills_with_custom_scan_path_no_false_drift() {
    use crate::project::KiroProject;
    use crate::service::test_support::{
        relative_path_entry, seed_marketplace_with_registry, temp_service,
    };
    use std::fs;

    let (dir, svc) = temp_service();
    let entries = vec![relative_path_entry("p", "plugins/p")];
    let mp_path = seed_marketplace_with_registry(dir.path(), &svc, "mp", &entries);
    let plugin_dir = mp_path.join("plugins/p");

    // Skill source under a NON-default scan path. Pre-fix detection
    // would look at <plugin_dir>/skills/alpha and hit NotFound.
    let packs_dir = plugin_dir.join("packs");
    let alpha_dir = packs_dir.join("alpha");
    fs::create_dir_all(&alpha_dir).expect("create skill dir");
    fs::write(
        alpha_dir.join("SKILL.md"),
        "---\nname: alpha\ndescription: test\n---\n",
    )
    .expect("write SKILL.md");

    // Manifest declares the custom scan path.
    fs::write(
        plugin_dir.join("plugin.json"),
        br#"{"name":"p","version":"1.0","skills":["./packs/"]}"#,
    )
    .expect("write plugin.json");

    let project_tmp = tempfile::tempdir().expect("project tempdir");
    let project = KiroProject::new(project_tmp.path().to_path_buf());

    svc.install_plugin(&project, &mp("mp"), &pn("p"), InstallMode::New, false)
        .expect("install");

    // Sanity: install actually placed the skill so the tracking entry
    // exists for detection to compare against.
    assert!(
        project_tmp.path().join(".kiro/skills/alpha/SKILL.md").exists(),
        "precondition: skill must be installed"
    );

    let result = svc.detect_plugin_updates(&project).expect("detect");
    assert!(
        result.updates.is_empty(),
        "expected no updates for an unchanged skill source under a custom \
         scan path; got: {:?}",
        result.updates
    );
    assert!(
        result.failures.is_empty(),
        "expected no failures (pre-fix would return a HashFailed because \
         detection looked under hardcoded `./skills/`); got: {:?}",
        result.failures
    );
}
```

- [ ] **Step 3.2: Run the test to verify it fails (compile or runtime)**

```bash
cargo test -p kiro-market-core --lib detect_plugin_updates_skills_with_custom_scan_path_no_false_drift
```

Expected: either compile error (if InstalledSkillMeta hasn't been updated yet) or runtime FAIL with "expected no failures" because detection still uses the hardcoded path.

If it compiles and fails at runtime, this confirms the bug exists. If you need to make it compile AND fail (rather than compile-error), proceed to Step 3.3 first to add the field, then come back.

- [ ] **Step 3.3: Add `source_scan_root` to `InstalledSkillMeta`**

In `crates/kiro-market-core/src/project.rs`, modify the `InstalledSkillMeta` struct (line ~25):

```rust
pub struct InstalledSkillMeta {
    pub marketplace: MarketplaceName,
    pub plugin: PluginName,
    pub version: Option<String>,
    pub installed_at: DateTime<Utc>,
    pub source_hash: Option<String>,    // unchanged for now; tightened in Task 6
    pub installed_hash: Option<String>, // unchanged for now; tightened in Task 6
    /// Scan root (relative to `plugin_dir`) that this skill was
    /// installed from. Required at install time; populated from the
    /// `DiscoveredSkill.scan_root` field which `discover_skill_dirs`
    /// returns alongside each found skill directory. Drift detection
    /// at [`crate::service::MarketplaceService::scan_plugin_for_content_drift`]
    /// uses this to locate the source skill dir for hash recomputation,
    /// closing #97 (the hardcoded `plugin_dir.join("skills")` bug).
    pub source_scan_root: RelativePath,
}
```

- [ ] **Step 3.4: Refactor `discover_skill_dirs` to return `DiscoveredSkill`**

In `crates/kiro-market-core/src/plugin.rs`, add a new type and refactor the function (line ~399):

```rust
/// One skill directory discovered under a plugin's manifest-declared
/// skill scan paths. Carries both the scan root and the resolved
/// skill directory so install can record the scan root for later
/// drift detection.
#[derive(Debug, Clone)]
pub struct DiscoveredSkill {
    /// Absolute path to the scan root that contains `skill_dir`,
    /// e.g. `<plugin_dir>/skills/` or `<plugin_dir>/packs/`.
    /// Recorded on `InstalledSkillMeta.source_scan_root` (after
    /// `strip_prefix(plugin_dir)`) so detection knows where to look.
    pub scan_root: PathBuf,
    /// Absolute path to the skill directory itself,
    /// e.g. `<plugin_dir>/skills/alpha/`. Contains a `SKILL.md`.
    pub skill_dir: PathBuf,
}

/// Discover skill directories under one or more scan paths. ...
/// (preserve the existing doc comment, adjust the return type sentence)
#[must_use]
pub fn discover_skill_dirs(plugin_root: &Path, skill_paths: &[&str]) -> Vec<DiscoveredSkill> {
    let mut found = Vec::new();

    for &path_str in skill_paths {
        if let Err(e) = crate::validation::validate_relative_path(path_str) {
            warn!(
                path = path_str,
                error = %e,
                "skipping skill path that fails validation"
            );
            continue;
        }

        let candidate = plugin_root.join(path_str);

        if path_str.ends_with('/') || path_str.ends_with('\\') {
            // Scan subdirectories for those containing SKILL.md.
            // The candidate path IS the scan root for this branch.
            match fs::read_dir(&candidate) {
                Ok(entries) => {
                    for entry in entries {
                        let entry = match entry {
                            Ok(e) => e,
                            Err(e) => {
                                warn!(
                                    path = %candidate.display(),
                                    error = %e,
                                    "failed to read directory entry, skipping"
                                );
                                continue;
                            }
                        };
                        let entry_path = entry.path();
                        if entry_path.is_dir() && entry_path.join(SKILL_MD).exists() {
                            found.push(DiscoveredSkill {
                                scan_root: candidate.clone(),
                                skill_dir: entry_path,
                            });
                        }
                    }
                }
                Err(e) => {
                    debug!(
                        path = %candidate.display(),
                        error = %e,
                        "failed to read skill scan directory"
                    );
                }
            }
        } else if candidate.is_dir() && candidate.join(SKILL_MD).exists() {
            // Bare-path branch: the scan root is the candidate's
            // parent (or plugin_root if no parent under it). The
            // skill dir IS the candidate.
            let scan_root = candidate
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| plugin_root.to_path_buf());
            found.push(DiscoveredSkill {
                scan_root,
                skill_dir: candidate,
            });
        } else {
            debug!(
                path = %candidate.display(),
                "skill path does not contain SKILL.md, skipping"
            );
        }
    }

    found.sort_by(|a, b| a.skill_dir.cmp(&b.skill_dir));
    found
}
```

The `Vec<PathBuf>` → `Vec<DiscoveredSkill>` change is the structural one. The `scan_root` derivation has two branches matching the existing two discovery branches.

- [ ] **Step 3.5: Update `discover_skill_dirs`'s test module**

In `crates/kiro-market-core/src/plugin.rs` test module, find tests that call `discover_skill_dirs` and assert on the returned `Vec<PathBuf>`. Update them to extract `.skill_dir` for the existing assertions, OR add new assertions on `.scan_root`. The compiler will list every site.

Pattern:
```rust
// BEFORE
let dirs = discover_skill_dirs(&plugin_root, &skill_paths);
assert_eq!(dirs[0], plugin_root.join("skills/foo"));

// AFTER
let found = discover_skill_dirs(&plugin_root, &skill_paths);
assert_eq!(found[0].skill_dir, plugin_root.join("skills/foo"));
assert_eq!(found[0].scan_root, plugin_root.join("skills"));
```

- [ ] **Step 3.6: Update `discover_skills_for_plugin` (browse.rs:931)**

In `crates/kiro-market-core/src/service/browse.rs`, find `discover_skills_for_plugin` (line ~931). Two options:
- (a) Change its return type to `Vec<DiscoveredSkill>` (forces all callers to deal with the new shape)
- (b) Keep its return type as `Vec<PathBuf>` for back-compat, extracting `.skill_dir` from `DiscoveredSkill` records

Pick (a) — only one caller (the install path at service/mod.rs); adapting the caller is mechanical and preserves the `scan_root` information into the install site.

```rust
fn discover_skills_for_plugin(
    plugin_dir: &Path,
    manifest: Option<&PluginManifest>,
) -> Vec<crate::plugin::DiscoveredSkill> {
    let skill_paths: Vec<&str> = if let Some(m) = manifest.filter(|m| !m.skills.is_empty()) {
        m.skills.iter().map(String::as_str).collect()
    } else {
        crate::DEFAULT_SKILL_PATHS.to_vec()
    };

    crate::plugin::discover_skill_dirs(plugin_dir, &skill_paths)
}
```

- [ ] **Step 3.7: Update the install-skills caller (service/mod.rs around 1280-1330)**

The caller iterates the returned `Vec<DiscoveredSkill>` and constructs `InstalledSkillMeta`. Two changes:

(a) The loop variable previously named `skill_dir: &PathBuf` becomes `discovered: &DiscoveredSkill`. Update field accesses (`skill_dir` → `discovered.skill_dir`, plus access to `discovered.scan_root` for the new field).

(b) When constructing the meta, populate `source_scan_root` via `RelativePath::from_path_under(&discovered.scan_root, plugin_dir)`. Same error-handling shape as Task 2 (continue with FailedSkill on error):

```rust
let source_scan_root = match crate::validation::RelativePath::from_path_under(
    &discovered.scan_root,
    plugin_dir,
) {
    Ok(rp) => rp,
    Err(e) => {
        warn!(
            scan_root = %discovered.scan_root.display(),
            plugin_dir = %plugin_dir.display(),
            error = %e,
            "discovered skill scan_root not under plugin_dir; skipping",
        );
        result.failed.push(FailedSkill::install_failed(
            frontmatter.name.clone(),
            &Error::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("scan_root not under plugin_dir: {e}"),
            )),
        ));
        continue;
    }
};

let meta = crate::project::InstalledSkillMeta {
    marketplace: marketplace.clone(),
    plugin: plugin.clone(),
    version: version.map(str::to_owned),
    installed_at: chrono::Utc::now(),
    source_hash: None,           // tightened in Task 6
    installed_hash: None,        // tightened in Task 6
    source_scan_root,
};
```

The `discovered.skill_dir` is what gets passed to `project.install_skill_from_dir` (already by reference, no change there).

- [ ] **Step 3.8: Update the skills loop in `scan_plugin_for_content_drift`**

In `crates/kiro-market-core/src/service/mod.rs` (around line 2510-2543), replace the skills loop. The current code (with the long "KNOWN BUG (issue #97)" comment block) becomes:

```rust
// Skills — direct lookup against the install-recorded scan root.
// Closes #97; install populates `source_scan_root` so detection
// hashes against the same directory the install copied from.
for (name, meta) in &installed_skills.skills {
    if meta.marketplace == plugin_info.marketplace && meta.plugin == plugin_info.plugin {
        match &meta.source_hash {
            Some(stored) => {
                let skill_dir = plugin_dir
                    .join(meta.source_scan_root.as_str())
                    .join(name);
                let computed = crate::hash::hash_dir_tree(&skill_dir)?;
                if computed != *stored {
                    content_drift = true;
                    return Ok((content_drift, legacy_fallback));
                }
            }
            None => legacy_fallback = true,
        }
    }
}
```

Delete the entire "KNOWN BUG (issue #97)" comment block (~18 lines). The `source_hash: Option` handling stays for now (tightened in Task 6).

- [ ] **Step 3.9: Update `InstalledSkillMeta` construction sites (compiler-driven)**

```bash
cargo build -p kiro-market-core 2>&1 | grep "missing field" | head
```

Walk every error and add `source_scan_root` field. Default value for tests:
```rust
source_scan_root: crate::validation::RelativePath::new("skills").expect("valid"),
```

For tests that exercise non-default scan paths, use the appropriate scan root string.

Also: any test that calls `install_plugin_skills`, `install_plugin`, or downstream entry points and then asserts on tracking-file shape will pick up the new field automatically (the install populates it). Round-trip tests should still pass — only the JSON shape grows by one field.

- [ ] **Step 3.10: Run the regression test**

```bash
cargo test -p kiro-market-core --lib detect_plugin_updates_skills_with_custom_scan_path_no_false_drift
```

Expected: PASS. The test from Step 3.1 should now go green because install records `source_scan_root: "packs"` and detection looks under `<plugin_dir>/packs/alpha/` instead of the hardcoded `<plugin_dir>/skills/alpha/`.

- [ ] **Step 3.11: Run the workspace tests**

```bash
cargo test --workspace 2>&1 | grep "test result\|FAIL" | tail -10
```

Expected: all pass, count up by 1 (new regression test). Compiler-driven fixture updates kept everything else green.

- [ ] **Step 3.12: Run clippy + fmt + plan-lint**

```bash
cargo clippy --workspace --tests -- -D warnings && \
  cargo fmt --all --check && \
  TETHYS_BIN=/home/dwalleck/repos/rivets/target/release/tethys cargo xtask plan-lint
```

Expected: clean.

- [ ] **Step 3.13: Commit**

```bash
git add -A && git commit -m "$(cat <<'EOF'
feat(skills): add InstalledSkillMeta.source_scan_root + use at detect time

Closes #97. The skills detection loop in scan_plugin_for_content_drift
now reads meta.source_scan_root instead of hardcoding plugin_dir.join("skills"),
so a plugin declaring skills: ["./packs/"] is detected at the right path.

Schema:
- New required field InstalledSkillMeta.source_scan_root: RelativePath.
- discover_skill_dirs returns Vec<DiscoveredSkill { scan_root, skill_dir }>
  instead of Vec<PathBuf> so install has the scan root in scope when
  building the meta. Single existing caller (discover_skills_for_plugin)
  updated.

Install:
- Populates source_scan_root via RelativePath::from_path_under (the
  helper from Task 1) at the only production install site
  (service/mod.rs:1316). Failures route to result.failed.

Detection:
- Skills loop becomes a 4-line direct lookup. The "KNOWN BUG (issue
  #97)" comment block deleted (the bug it documented is now closed).

Test:
- New regression test detect_plugin_updates_skills_with_custom_scan_path_no_false_drift
  installs a plugin under ./packs/, runs detect, asserts no failures
  and no updates. Direct sibling of the steering and native_companions
  regression tests added in PR #96 review-of-review.

source_hash: Option<String> on InstalledSkillMeta unchanged for now;
tightened in Task 6 alongside the cross-cutting legacy_fallback flag
removal.

Per design docs/plans/2026-05-03-install-detect-symmetry-design.md.
EOF
)"
```

---

## Task 4: Add `InstalledSteeringMeta.source_scan_root`

**Why.** Symmetric with skills; converts the steering loop from probe-fallback to direct lookup. Pre-existing regression test from PR #96 review-of-review (`detect_plugin_updates_steering_with_custom_scan_path_no_false_drift`) keeps passing — the install path now populates the new field automatically.

**Files:**
- Modify: `crates/kiro-market-core/src/project.rs:155-186` (`InstalledSteeringMeta` + surrounding) — add field
- Modify: `crates/kiro-market-core/src/project.rs:1959-1969` — install site populates new field
- Modify: `crates/kiro-market-core/src/service/mod.rs:2553-2565` — steering loop uses direct lookup
- Update fixtures (compiler-driven)

- [ ] **Step 4.1: Add the field to `InstalledSteeringMeta`**

In `crates/kiro-market-core/src/project.rs:165`:

```rust
pub struct InstalledSteeringMeta {
    pub marketplace: MarketplaceName,
    pub plugin: PluginName,
    pub version: Option<String>,
    pub installed_at: DateTime<Utc>,
    pub source_hash: String,
    pub installed_hash: String,
    /// Scan root (relative to `plugin_dir`) that this steering file was
    /// installed from. Required at install time; populated from
    /// `DiscoveredNativeFile.scan_root` (which steering shares with the
    /// agent discover sites) via `RelativePath::from_path_under`. Drift
    /// detection at [`crate::service::MarketplaceService::scan_plugin_for_content_drift`]
    /// uses this directly to locate the source file for hash recomputation,
    /// replacing PR #96's `hash_artifact_in_scan_paths` probe helper.
    pub source_scan_root: RelativePath,
}
```

- [ ] **Step 4.2: Update the install site (project.rs:1959)**

Find the `InstalledSteeringMeta { ... }` construction at line ~1959. The surrounding install function has access to `source: &DiscoveredSteeringFile` (or similar — verify the exact name) which carries `scan_root: PathBuf`. Add the new field:

```rust
let source_scan_root = match crate::validation::RelativePath::from_path_under(
    &source.scan_root,
    plugin_dir, // verify this var name in scope; may need to be passed as arg
) {
    Ok(rp) => rp,
    Err(e) => {
        warn!(
            scan_root = %source.scan_root.display(),
            error = %e,
            "steering scan_root not under plugin_dir; skipping",
        );
        return Err(crate::steering::SteeringError::SourceReadFailed {
            path: source.scan_root.clone(),
            source: std::io::Error::new(std::io::ErrorKind::InvalidInput, format!("{e}")),
        }
        .into());
    }
};

installed.files.insert(
    rel_path.to_path_buf(),
    InstalledSteeringMeta {
        marketplace: ctx.marketplace.clone(),
        plugin: ctx.plugin.clone(),
        version: ctx.version.map(str::to_owned),
        installed_at: chrono::Utc::now(),
        source_hash: source_hash.to_owned(),
        installed_hash: installed_hash.clone(),
        source_scan_root,
    },
);
```

If `plugin_dir` isn't currently in scope at this call site, walk back to the caller that knows it and thread it through. The `DiscoveredNativeFile` fields are: `scan_root: PathBuf`, `source: PathBuf` — `source.strip_prefix(scan_root)` yields the `rel_path` already used in tracking.

- [ ] **Step 4.3: Update the steering loop in `scan_plugin_for_content_drift`**

Replace the steering loop body in `crates/kiro-market-core/src/service/mod.rs` (around line 2553). The current code:

```rust
let steering_paths = crate::service::browse::steering_scan_paths_for_plugin(manifest);
for (rel_path, meta) in &installed_steering.files {
    if meta.marketplace == plugin_info.marketplace && meta.plugin == plugin_info.plugin {
        let computed = hash_artifact_in_scan_paths(
            plugin_dir,
            &steering_paths,
            std::slice::from_ref(rel_path),
        )?;
        ...
    }
}
```

Becomes:

```rust
for (rel_path, meta) in &installed_steering.files {
    if meta.marketplace == plugin_info.marketplace && meta.plugin == plugin_info.plugin {
        let scan_root = plugin_dir.join(meta.source_scan_root.as_str());
        let computed = crate::hash::hash_artifact(&scan_root, std::slice::from_ref(rel_path))?;
        if computed != meta.source_hash {
            content_drift = true;
            return Ok((content_drift, legacy_fallback));
        }
    }
}
```

The `steering_paths` derivation outside the loop and the `hash_artifact_in_scan_paths` call disappear. Direct lookup replaces probe.

- [ ] **Step 4.4: Update fixtures (compiler-driven)**

```bash
cargo build -p kiro-market-core 2>&1 | grep "missing field" | head
```

Walk every site. Default for tests: `source_scan_root: crate::validation::RelativePath::new("steering").expect("valid")`.

The existing `detect_plugin_updates_steering_with_custom_scan_path_no_false_drift` regression test should keep passing without modification — it uses `svc.install_plugin`, which now populates the field automatically. Verify by running it:

```bash
cargo test -p kiro-market-core --lib detect_plugin_updates_steering_with_custom_scan_path_no_false_drift
```

Expected: PASS.

- [ ] **Step 4.5: Run workspace tests + clippy + fmt + plan-lint**

```bash
cargo test --workspace 2>&1 | grep "test result\|FAIL" | tail -5 && \
  cargo clippy --workspace --tests -- -D warnings && \
  cargo fmt --all --check && \
  TETHYS_BIN=/home/dwalleck/repos/rivets/target/release/tethys cargo xtask plan-lint
```

Expected: all green.

- [ ] **Step 4.6: Commit**

```bash
git add -A && git commit -m "$(cat <<'EOF'
feat(steering): add InstalledSteeringMeta.source_scan_root + use at detect time

Symmetric with the skills change in Task 3. The steering detection loop
in scan_plugin_for_content_drift switches from PR #96's
hash_artifact_in_scan_paths probe to a direct lookup against the
install-recorded scan root.

Schema: new required field InstalledSteeringMeta.source_scan_root: RelativePath.

Install: populates the field via RelativePath::from_path_under, sourcing
scan_root from the DiscoveredSteeringFile (already in scope at the
install site).

Detection: steering loop becomes a 5-line direct lookup. The probe
helper hash_artifact_in_scan_paths still exists but is now used only by
the native-companions loop; both removed in Task 6 once that loop is
also migrated.

The existing regression test detect_plugin_updates_steering_with_custom_scan_path_no_false_drift
keeps passing without modification — install now populates the new
field automatically through the real install pipeline.

Per design docs/plans/2026-05-03-install-detect-symmetry-design.md.
EOF
)"
```

---

## Task 5: Add `InstalledNativeCompanionsMeta.source_scan_root`

**Why.** Symmetric with skills + steering. Closes the last loop that depended on `hash_artifact_in_scan_paths`, so Task 6 can delete the helper entirely.

**Files:**
- Modify: `crates/kiro-market-core/src/project.rs:124-141` (`InstalledNativeCompanionsMeta`) — add field
- Modify: `crates/kiro-market-core/src/project.rs:2409, 2892` — install sites populate new field
- Modify: `crates/kiro-market-core/src/service/mod.rs:2643-2659` — native_companions loop uses direct lookup
- Update fixtures (compiler-driven)

- [ ] **Step 5.1: Add the field to `InstalledNativeCompanionsMeta`**

In `crates/kiro-market-core/src/project.rs:124`:

```rust
pub struct InstalledNativeCompanionsMeta {
    pub marketplace: MarketplaceName,
    pub plugin: PluginName,
    pub version: Option<String>,
    pub installed_at: DateTime<Utc>,
    pub files: Vec<PathBuf>,
    pub source_hash: String,
    pub installed_hash: String,
    /// Scan root (relative to `plugin_dir`) that this companion bundle
    /// was installed from. Required at install time. Single-scan-root
    /// invariant is enforced upstream by `multiple_companion_scan_roots`,
    /// so all `files` resolve under this single root. Drift detection
    /// uses this directly, replacing PR #96's
    /// `hash_artifact_in_scan_paths` probe.
    pub source_scan_root: RelativePath,
}
```

- [ ] **Step 5.2: Update install sites (project.rs:2409 and project.rs:2892)**

The first site (line ~2409, inside `or_insert_with`) and the second (line ~2892) both need the new field. Both have access to `scan_root` (passed via `NativeCompanionsInput.scan_root` per the spec).

For each construction site:

```rust
InstalledNativeCompanionsMeta {
    // ... existing fields ...
    source_scan_root: crate::validation::RelativePath::from_path_under(
        input.scan_root,
        plugin_dir,
    )
    .map_err(|e| <appropriate AgentError variant>)?,
}
```

The exact error variant depends on the surrounding function's `Result` type. If it returns `crate::error::Result<...>`, wrap as `Error::Io` or `Error::Validation` matching the function's existing error-handling pattern. Read the surrounding 30 lines to pick the right shape.

- [ ] **Step 5.3: Update the native_companions loop in `scan_plugin_for_content_drift`**

Replace the loop in `crates/kiro-market-core/src/service/mod.rs` (around line 2643):

```rust
// Native companions — direct lookup, single-scan-root per
// MultipleScanRootsNotSupported invariant.
for meta in installed_agents.native_companions.values() {
    if meta.marketplace == plugin_info.marketplace && meta.plugin == plugin_info.plugin {
        let scan_root = plugin_dir.join(meta.source_scan_root.as_str());
        let computed = crate::hash::hash_artifact(&scan_root, &meta.files)?;
        if computed != meta.source_hash {
            content_drift = true;
            return Ok((content_drift, legacy_fallback));
        }
    }
}
```

The `agent_paths` derivation and the `hash_artifact_in_scan_paths` call disappear. Both are dead code at this point (no remaining callers); removed in Task 6.

- [ ] **Step 5.4: Update fixtures (compiler-driven)**

```bash
cargo build -p kiro-market-core 2>&1 | grep "missing field" | head
```

Walk every site. Default for tests: `source_scan_root: crate::validation::RelativePath::new("agents").expect("valid")`.

The existing `detect_plugin_updates_native_companions_with_custom_scan_path_no_false_drift` regression test currently constructs `InstalledNativeCompanionsMeta` by hand (not via install). It will need the new field added to the fixture — set to `RelativePath::new("companions").expect("valid")` to match the test's `manifest.agents: ["./companions/"]`. Verify the test then passes with the new field.

- [ ] **Step 5.5: Run workspace tests + clippy + fmt + plan-lint**

```bash
cargo test --workspace 2>&1 | grep "test result\|FAIL" | tail -5 && \
  cargo clippy --workspace --tests -- -D warnings && \
  cargo fmt --all --check && \
  TETHYS_BIN=/home/dwalleck/repos/rivets/target/release/tethys cargo xtask plan-lint
```

Expected: all green.

- [ ] **Step 5.6: Commit**

```bash
git add -A && git commit -m "$(cat <<'EOF'
feat(native-companions): add InstalledNativeCompanionsMeta.source_scan_root + use at detect time

Symmetric with the skills + steering changes. Closes the last loop in
scan_plugin_for_content_drift that depended on PR #96's
hash_artifact_in_scan_paths probe — Task 6 removes the helper outright
along with the legacy_fallback flag.

Schema: new required field InstalledNativeCompanionsMeta.source_scan_root.

Install: populates from NativeCompanionsInput.scan_root (already in
scope; the single-scan-root invariant enforced by
multiple_companion_scan_roots means all bundle files share one root).

Detection: native_companions loop becomes a 5-line direct lookup.

The existing regression test detect_plugin_updates_native_companions_with_custom_scan_path_no_false_drift
gets the new field added to its hand-crafted tracking entry; the test
intent is unchanged.

Per design docs/plans/2026-05-03-install-detect-symmetry-design.md.
EOF
)"
```

---

## Task 6: Delete probe helpers + tighten `source_hash`

**Why.** Now that all four detection loops use direct lookup, `hash_artifact_in_scan_paths` and the `legacy_fallback` flag are dead code. Tightening `source_hash: Option<String>` → `String` on skills + agents symmetrises with steering / native_companions and deletes the last legacy-fallback machinery.

**Files:**
- Modify: `crates/kiro-market-core/src/project.rs:25-51` (`InstalledSkillMeta`) — tighten `source_hash` and `installed_hash`
- Modify: `crates/kiro-market-core/src/project.rs:62-108` (`InstalledAgentMeta`) — tighten `source_hash` and `installed_hash`
- Modify: `crates/kiro-market-core/src/service/mod.rs:2500-2662` (`scan_plugin_for_content_drift`) — drop `manifest` parameter, drop `legacy_fallback` from return tuple, simplify all four loops
- Modify: `crates/kiro-market-core/src/service/mod.rs:~2310` (`check_plugin_for_update`) — adjust the call to `scan_plugin_for_content_drift` for new signature
- Delete: `crates/kiro-market-core/src/service/mod.rs:2665-2714` (`hash_artifact_in_scan_paths` + its docstring)
- Modify: `crates/kiro-market-core/src/service/browse.rs:949-973` — revert `agent_scan_paths_for_plugin` and `steering_scan_paths_for_plugin` from `pub(super)` to `fn` (private)
- Update fixtures (compiler-driven)
- Delete tests: `detect_plugin_updates_legacy_fallback_source_hash_none`, `detect_plugin_updates_legacy_fallback_no_version_bump_returns_no_update`, `detect_plugin_updates_agent_legacy_fallback_source_hash_none`

- [ ] **Step 6.1: Tighten `InstalledSkillMeta.source_hash` and `installed_hash`**

In `crates/kiro-market-core/src/project.rs:25`, change:

```rust
// BEFORE
#[serde(default, skip_serializing_if = "Option::is_none")]
pub source_hash: Option<String>,
#[serde(default, skip_serializing_if = "Option::is_none")]
pub installed_hash: Option<String>,

// AFTER
pub source_hash: String,
pub installed_hash: String,
```

Update doc comments to reflect required-not-optional. Same change for `InstalledAgentMeta` at line ~62.

- [ ] **Step 6.2: Update install sites that constructed `source_hash: None`**

Compiler will list them. Replace `source_hash: None` → `source_hash: <actual_hash>` everywhere. The install paths already compute the hash via `hash_dir_tree` / `hash_artifact` — propagate that value.

For sites where the hash isn't yet computed at the construction point, compute it earlier in the function and pass through. The skills install at service/mod.rs needs to compute `hash_dir_tree(&discovered.skill_dir)?` and pass through.

For test fixtures that used `source_hash: None`, replace with a synthetic hash literal:

```rust
source_hash: "blake3:0000000000000000000000000000000000000000000000000000000000000000".to_owned(),
installed_hash: "blake3:0000000000000000000000000000000000000000000000000000000000000000".to_owned(),
```

- [ ] **Step 6.3: Simplify `scan_plugin_for_content_drift` signature**

In `crates/kiro-market-core/src/service/mod.rs:2500`, change the function signature:

```rust
// BEFORE
fn scan_plugin_for_content_drift(
    plugin_info: &crate::project::InstalledPluginInfo,
    plugin_dir: &Path,
    manifest: Option<&crate::plugin::PluginManifest>,
    installed_skills: &crate::project::InstalledSkills,
    installed_steering: &crate::project::InstalledSteering,
    installed_agents: &crate::project::InstalledAgents,
) -> Result<(bool, bool), Error>

// AFTER
fn scan_plugin_for_content_drift(
    plugin_info: &crate::project::InstalledPluginInfo,
    plugin_dir: &Path,
    installed_skills: &crate::project::InstalledSkills,
    installed_steering: &crate::project::InstalledSteering,
    installed_agents: &crate::project::InstalledAgents,
) -> Result<bool, Error>
```

Drop `manifest` parameter (unused now that detection doesn't probe scan paths). Drop the `legacy_fallback` from the return tuple — return `bool` (just `content_drift`).

- [ ] **Step 6.4: Simplify all four loop bodies**

With `source_hash` now required, drop the `match &meta.source_hash { Some(stored) => { ... }, None => legacy_fallback = true }` shape from skills and agents loops. Just unconditionally compute and compare:

```rust
// Skills (post-tightening)
for (name, meta) in &installed_skills.skills {
    if meta.marketplace == plugin_info.marketplace && meta.plugin == plugin_info.plugin {
        let skill_dir = plugin_dir.join(meta.source_scan_root.as_str()).join(name);
        let computed = crate::hash::hash_dir_tree(&skill_dir)?;
        if computed != meta.source_hash {
            return Ok(true);
        }
    }
}

// Steering (already direct lookup; just replace `meta.source_hash` access shape)
for (rel_path, meta) in &installed_steering.files {
    if meta.marketplace == plugin_info.marketplace && meta.plugin == plugin_info.plugin {
        let scan_root = plugin_dir.join(meta.source_scan_root.as_str());
        let computed = crate::hash::hash_artifact(&scan_root, std::slice::from_ref(rel_path))?;
        if computed != meta.source_hash {
            return Ok(true);
        }
    }
}

// Agents (post-tightening)
for (_name, meta) in &installed_agents.agents {
    if meta.marketplace == plugin_info.marketplace && meta.plugin == plugin_info.plugin {
        let (base, filename) = agent_hash_inputs(plugin_dir, &meta.source_path);
        let computed = crate::hash::hash_artifact(&base, std::slice::from_ref(&filename))?;
        if computed != meta.source_hash {
            return Ok(true);
        }
    }
}

// Native companions
for meta in installed_agents.native_companions.values() {
    if meta.marketplace == plugin_info.marketplace && meta.plugin == plugin_info.plugin {
        let scan_root = plugin_dir.join(meta.source_scan_root.as_str());
        let computed = crate::hash::hash_artifact(&scan_root, &meta.files)?;
        if computed != meta.source_hash {
            return Ok(true);
        }
    }
}

Ok(false)
```

The function body shrinks substantially — about 80 lines of legacy-fallback handling and threading deleted.

- [ ] **Step 6.5: Update `check_plugin_for_update` for new return shape**

In `crates/kiro-market-core/src/service/mod.rs` (around line 2310 where `check_plugin_for_update` calls `scan_plugin_for_content_drift`), update the call to drop the `manifest` argument and to handle the new `bool` return:

```rust
// BEFORE
let (content_drift, legacy_fallback) = Self::scan_plugin_for_content_drift(
    plugin_info,
    &plugin_dir,
    Some(&manifest),
    installed_skills,
    installed_steering,
    installed_agents,
)?;

// AFTER
let content_drift = Self::scan_plugin_for_content_drift(
    plugin_info,
    &plugin_dir,
    installed_skills,
    installed_steering,
    installed_agents,
)?;
```

Also remove the downstream `legacy_fallback` handling. Find any `if legacy_fallback { ... }` blocks in `check_plugin_for_update` and delete them — the flag no longer exists.

- [ ] **Step 6.6: Delete `hash_artifact_in_scan_paths`**

In `crates/kiro-market-core/src/service/mod.rs` (around line 2665-2714), delete the entire function body and its docstring. No callers remain after Task 5.

Verify:
```bash
grep -n "hash_artifact_in_scan_paths" crates/
```
Expected: no matches outside the function definition itself (which you're about to delete).

- [ ] **Step 6.7: Revert visibility of scan-path helpers**

In `crates/kiro-market-core/src/service/browse.rs` (lines 949 and 964):

```rust
// BEFORE
pub(super) fn agent_scan_paths_for_plugin(...)
pub(super) fn steering_scan_paths_for_plugin(...)

// AFTER
fn agent_scan_paths_for_plugin(...)
fn steering_scan_paths_for_plugin(...)
```

These were bumped to `pub(super)` for detection's use; detection no longer consumes them, so they can return to private. Verify:

```bash
grep -rn "agent_scan_paths_for_plugin\|steering_scan_paths_for_plugin" crates/
```

Expected: only intra-`browse.rs` callers. If anything else references them, leave the visibility as-is.

- [ ] **Step 6.8: Delete the three legacy-fallback tests**

In `crates/kiro-market-core/src/service/mod.rs`, delete:
- `detect_plugin_updates_legacy_fallback_source_hash_none` (around line 6091)
- `detect_plugin_updates_legacy_fallback_no_version_bump_returns_no_update` (around line 6145)
- `detect_plugin_updates_agent_legacy_fallback_source_hash_none` (around line 6447)

These all exercise the `legacy_fallback` flag that no longer exists.

- [ ] **Step 6.9: Run workspace tests + clippy + fmt + plan-lint**

```bash
cargo test --workspace 2>&1 | grep "test result\|FAIL" | tail -5 && \
  cargo clippy --workspace --tests -- -D warnings && \
  cargo fmt --all --check && \
  TETHYS_BIN=/home/dwalleck/repos/rivets/target/release/tethys cargo xtask plan-lint
```

Expected: all green. Test count drops by 3 (deleted tests). Net code reduction of ~150 lines.

- [ ] **Step 6.10: Commit**

```bash
git add -A && git commit -m "$(cat <<'EOF'
refactor(detect): delete probe helpers + legacy_fallback (now dead)

After Tasks 2-5 migrated all four detection loops to direct lookup, the
PR #96 probe-fallback machinery has zero callers. Sweep:

Schema:
- source_hash: Option<String> -> String on InstalledSkillMeta and
  InstalledAgentMeta. Symmetrises with InstalledSteeringMeta and
  InstalledNativeCompanionsMeta (already plain String).
- installed_hash: same tightening on the same two types.

Detection:
- scan_plugin_for_content_drift loses the manifest parameter (no
  longer needed to derive scan paths) and the legacy_fallback bool
  from its return tuple. Returns plain Result<bool, Error>.
- All four loop bodies simplified — no more "is this a legacy
  entry?" Option-matching, no more probe.
- check_plugin_for_update updated for the new signature; downstream
  legacy_fallback handling deleted.

Deletions:
- hash_artifact_in_scan_paths (~50 lines incl. docstring)
- agent_scan_paths_for_plugin / steering_scan_paths_for_plugin
  visibility reverted from pub(super) to private (no consumers
  outside browse.rs)
- detect_plugin_updates_legacy_fallback_source_hash_none
- detect_plugin_updates_legacy_fallback_no_version_bump_returns_no_update
- detect_plugin_updates_agent_legacy_fallback_source_hash_none

Net code reduction: ~150 lines. Detection now has one straight-line
recipe per artifact type, structurally symmetric across all four.

Per design docs/plans/2026-05-03-install-detect-symmetry-design.md.
EOF
)"
```

---

## Task 7: Bindings regen + deserialize-rejection tests

**Why.** Final commit pins the foundation contract: tracking files missing the new required fields fail to deserialize with a clear error. Bindings.ts regen propagates the schema changes to the FE.

**Files:**
- Modify: `crates/kiro-control-center/src/lib/bindings.ts` — regenerate
- Modify: `crates/kiro-market-core/src/project.rs` test module — add 4 deserialize-rejection tests

- [ ] **Step 7.1: Regenerate TS bindings**

```bash
cargo test -p kiro-control-center --lib -- --ignored
```

Expected: passes. `bindings.ts` updated in-place. Verify with:

```bash
git diff --stat crates/kiro-control-center/src/lib/bindings.ts
```

Expected: a few lines changed (new field on `InstalledSkillMeta` etc., and likely new `RelativePath` type alias if not already present).

- [ ] **Step 7.2: Add deserialize-rejection tests**

In `crates/kiro-market-core/src/project.rs` test module, add four tests pinning that legacy tracking files (without the new required fields) fail to load:

```rust
#[test]
fn load_installed_skills_rejects_legacy_entry_without_source_scan_root() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let project = KiroProject::new(tmp.path().to_path_buf());
    let kiro_dir = tmp.path().join(".kiro");
    std::fs::create_dir_all(&kiro_dir).expect("create .kiro");

    // Pre-B-schema entry: missing source_scan_root, missing source_hash,
    // missing installed_hash. Mirrors what an old tracking file looked
    // like before this commit landed.
    let legacy_json = br#"{
        "skills": {
            "alpha": {
                "marketplace": "mp",
                "plugin": "p",
                "version": "1.0",
                "installed_at": "2026-01-01T00:00:00Z"
            }
        }
    }"#;
    std::fs::write(kiro_dir.join("installed-skills.json"), legacy_json)
        .expect("write legacy tracking");

    let err = project
        .load_installed()
        .expect_err("legacy entry must fail to deserialize, not silently succeed");
    let msg = err.to_string();
    assert!(
        msg.contains("source_scan_root") || msg.contains("source_hash"),
        "error must mention the missing required field; got: {msg}"
    );
}

#[test]
fn load_installed_steering_rejects_legacy_entry_without_source_scan_root() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let project = KiroProject::new(tmp.path().to_path_buf());
    let kiro_dir = tmp.path().join(".kiro");
    std::fs::create_dir_all(&kiro_dir).expect("create .kiro");

    let legacy_json = br#"{
        "files": {
            "guide.md": {
                "marketplace": "mp",
                "plugin": "p",
                "version": "1.0",
                "installed_at": "2026-01-01T00:00:00Z",
                "source_hash": "blake3:0000000000000000000000000000000000000000000000000000000000000000",
                "installed_hash": "blake3:0000000000000000000000000000000000000000000000000000000000000000"
            }
        }
    }"#;
    std::fs::write(kiro_dir.join("installed-steering.json"), legacy_json)
        .expect("write legacy tracking");

    let err = project
        .load_installed_steering()
        .expect_err("legacy entry must fail");
    assert!(
        err.to_string().contains("source_scan_root"),
        "error must mention source_scan_root; got: {err}"
    );
}

#[test]
fn load_installed_agents_rejects_legacy_entry_without_source_path() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let project = KiroProject::new(tmp.path().to_path_buf());
    let kiro_dir = tmp.path().join(".kiro");
    std::fs::create_dir_all(&kiro_dir).expect("create .kiro");

    let legacy_json = br#"{
        "agents": {
            "reviewer": {
                "marketplace": "mp",
                "plugin": "p",
                "version": "1.0",
                "installed_at": "2026-01-01T00:00:00Z",
                "dialect": "native"
            }
        }
    }"#;
    std::fs::write(kiro_dir.join("installed-agents.json"), legacy_json)
        .expect("write legacy tracking");

    let err = project
        .load_installed_agents()
        .expect_err("legacy entry must fail");
    let msg = err.to_string();
    assert!(
        msg.contains("source_path") || msg.contains("source_hash"),
        "error must mention a missing required field; got: {msg}"
    );
}

#[test]
fn load_installed_native_companions_rejects_legacy_entry_without_source_scan_root() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let project = KiroProject::new(tmp.path().to_path_buf());
    let kiro_dir = tmp.path().join(".kiro");
    std::fs::create_dir_all(&kiro_dir).expect("create .kiro");

    // Native companions live in installed-agents.json under the
    // native_companions key; a missing source_scan_root on a companion
    // entry triggers the same deserialize failure.
    let legacy_json = br#"{
        "agents": {},
        "native_companions": {
            "p": {
                "marketplace": "mp",
                "plugin": "p",
                "version": "1.0",
                "installed_at": "2026-01-01T00:00:00Z",
                "files": ["prompts/reviewer.md"],
                "source_hash": "blake3:0000000000000000000000000000000000000000000000000000000000000000",
                "installed_hash": "blake3:0000000000000000000000000000000000000000000000000000000000000000"
            }
        }
    }"#;
    std::fs::write(kiro_dir.join("installed-agents.json"), legacy_json)
        .expect("write legacy tracking");

    let err = project
        .load_installed_agents()
        .expect_err("legacy companions entry must fail");
    assert!(
        err.to_string().contains("source_scan_root"),
        "error must mention source_scan_root; got: {err}"
    );
}
```

- [ ] **Step 7.3: Run the new tests**

```bash
cargo test -p kiro-market-core --lib load_installed_ -- --skip happy_path
```

Expected: 4 tests pass.

- [ ] **Step 7.4: Run full workspace + clippy + fmt + plan-lint**

```bash
cargo test --workspace 2>&1 | grep "test result\|FAIL" | tail -5 && \
  cargo clippy --workspace --tests -- -D warnings && \
  cargo fmt --all --check && \
  TETHYS_BIN=/home/dwalleck/repos/rivets/target/release/tethys cargo xtask plan-lint
```

Expected: all green. Test count up by 4 (deserialize-rejection tests).

- [ ] **Step 7.5: Commit**

```bash
git add -A && git commit -m "$(cat <<'EOF'
chore: regenerate bindings.ts + add deserialize-rejection tests

bindings.ts: picks up the new source_scan_root field on
InstalledSkillMeta/SteeringMeta/NativeCompanionsMeta, source_path
required (no longer Option) on InstalledAgentMeta, and source_hash/
installed_hash required (no longer Option) on InstalledSkillMeta and
InstalledAgentMeta.

Four deserialize-rejection tests pin the foundation contract: legacy
tracking files (written before this PR landed) fail to load with a
clear error mentioning the missing required field. This makes the
"no migration story" choice from the spec auditable — anyone reading
the test sees that legacy-entry rejection IS the contract, not an
accident.

Per design docs/plans/2026-05-03-install-detect-symmetry-design.md.
EOF
)"
```

---

## Wrap-up: PR readiness

- [ ] **Step W.1: Verify final state**

```bash
cd /home/dwalleck/repos/kiro-marketplace-cli-install-detect-symmetry
git log --oneline feat/phase-2a-update-detection..HEAD
```

Expected: 7 commits (1 design doc + Tasks 1-7's commits).

- [ ] **Step W.2: Final verification sweep**

```bash
cargo test --workspace 2>&1 | grep "test result" && \
  cargo clippy --workspace --tests -- -D warnings && \
  cargo fmt --all --check && \
  TETHYS_BIN=/home/dwalleck/repos/rivets/target/release/tethys cargo xtask plan-lint
```

Expected: all green. Test count delta from baseline (Step 0.2): +1 (skills regression in Task 3) +4 (deserialize-rejection in Task 7) -4 (legacy-fallback deletions in Tasks 2 + 6) = **+1 net**.

- [ ] **Step W.3: Push the branch**

```bash
git push -u origin fix/install-detect-symmetry
```

- [ ] **Step W.4: Open the PR**

```bash
gh pr create --base feat/phase-2a-update-detection --title "fix: install↔detect symmetry — closes #97 + closes scan-path bug pattern" --body "$(cat <<'EOF'
## Summary

Closes #97 by closing the structural pattern that made #97 the third instance of "detection has a hardcoded scan path; install honors `manifest.{...}`". After PR #96 closed steering and agents via probe-fallback helpers, this PR migrates all four `Installed*Meta` types to a uniform schema where install records the manifest scan-path it used, and detection looks it up directly.

**Depends on PR #96.** Stacked PR — base branch is `feat/phase-2a-update-detection`. Merge after #96 lands.

## Design

- Spec: `docs/plans/2026-05-03-install-detect-symmetry-design.md`
- Plan: `docs/plans/2026-05-03-install-detect-symmetry-plan.md`
- Origin issue: #97
- Predecessor for: #99 (umbrella for C-1 native-companion hash recipe, C-2 manifest path validation lift, C-3 ContentHash newtype)

## Schema changes (breaking)

- `InstalledSkillMeta` / `InstalledSteeringMeta` / `InstalledNativeCompanionsMeta`: new required field `source_scan_root: RelativePath`.
- `InstalledAgentMeta.source_path`: was `Option<RelativePath>`, now required `RelativePath`.
- `InstalledSkillMeta` / `InstalledAgentMeta`: `source_hash` and `installed_hash` tightened from `Option<String>` to `String` (now symmetric with the other two types).

**Pre-existing tracking files fail to deserialize.** Intentional, per the no-users assumption from the spec. Four deserialize-rejection tests pin this contract.

## Detection simplification

- `scan_plugin_for_content_drift` becomes one straight-line lookup per artifact type. Returns plain `Result<bool, Error>` (no more `legacy_fallback` tuple).
- Deleted: `hash_artifact_in_scan_paths` (PR #96 probe helper), `agent_hash_inputs` dialect-fallback branch, the I-N7 actionable-error branch, the `manifest` parameter on `scan_plugin_for_content_drift`.
- `agent_scan_paths_for_plugin` and `steering_scan_paths_for_plugin` reverted from `pub(super)` to private.

## Test plan

- [x] `cargo fmt --all --check` clean
- [x] `cargo clippy --workspace --tests -- -D warnings` clean
- [x] `cargo test --workspace` all green (~+1 test net: +1 skills regression, +4 deserialize-rejection, -4 legacy-fallback deletions)
- [x] `cargo xtask plan-lint` all 6 gates OK
- [x] `npm run check` clean (frontend types via regenerated bindings.ts)
EOF
)"
```

If `gh pr create` is unavailable in your environment, open the PR via the GitHub web UI using the same title and body. Set the base branch to `feat/phase-2a-update-detection`.

---

## Self-Review

After writing the plan, I checked it against the spec.

**Spec coverage:** Every section of the spec maps to one or more tasks:
- Schema changes (spec §"Schema changes") → Tasks 2 (agents), 3 (skills), 4 (steering), 5 (native companions), 6 (source_hash tightening). All four field changes accounted for.
- Detection simplification (spec §"Detection simplification") → Tasks 2 (agent loop + I-N7 deletion), 3 (skills loop), 4 (steering loop), 5 (native companions loop), 6 (deletions of helpers + legacy_fallback + manifest parameter).
- Install-time changes (spec §"Install-time changes") → Tasks 1 (shared helper), 2 (agents install), 3 (skills install + discover_skill_dirs refactor), 4 (steering install), 5 (native companions install).
- Tests (spec §"Tests") → Tests deleted in Tasks 2 (Copilot legacy-fallback) and 6 (3 legacy_fallback tests = 4 total). Tests reshaped via compiler-driven updates throughout. Tests added in Tasks 1 (3× normalization helper tests), 3 (skills regression), 7 (4 deserialize-rejection tests). New `install_translated_agent_fails_when_source_path_outside_plugin_dir` from spec is present in Task 2 Step 2.1 (as a precondition test of `from_path_under`).
- Migration (spec §"Migration") → No work needed (per the no-users assumption); pinned by Task 7 deserialize-rejection tests.
- PR strategy (spec §"PR strategy") → 6 commits → mapped to Tasks 2-7 (Task 1 adds a 7th foundational commit for the helper extraction; the spec's 6 became 7 because the spec rolled the helper into Task 2 and the plan separates them for clearer commit boundaries).

**Placeholder scan:** No "TBD"/"TODO"/"implement later"/"add appropriate error handling"/"similar to Task N" patterns. Code blocks present at every code step. Step 5.2's "verify the exact name" is a "do this lookup" instruction, not a placeholder — the executor must confirm the field name exists. Acceptable.

**Type consistency:**
- `DiscoveredSkill { scan_root: PathBuf, skill_dir: PathBuf }` defined in Task 3 Step 3.4; consumed in Step 3.6 (`discover_skills_for_plugin` return type) and Step 3.7 (install loop iteration). Same field names used everywhere.
- `RelativePath::from_path_under(path: &Path, base: &Path) -> Result<Self, ValidationError>` defined in Task 1 Step 1.4; consumed in Tasks 2.5, 3.7, 4.2, 5.2 with consistent argument order (`path` first, `base` second).
- `InstalledNativeCompanionsMeta.source_scan_root` typed as `RelativePath`, consistent with `InstalledSkillMeta` and `InstalledSteeringMeta` siblings.
- `agent_hash_inputs(plugin_dir: &Path, source_path: &RelativePath) -> (PathBuf, PathBuf)` — new signature in Task 2 Step 2.8 has 2 params; consumer in Task 2 Step 2.9 calls with 2 args. Consistent.

**Spec gap noted during review:** The spec's "Install-time changes → Translated agents" section says "specific variant choice deferred to implementation". The plan picks `AgentError::InstallFailed` with a synthesized `io::Error` (Task 2 Step 2.5). If a reviewer prefers a typed variant, that's a per-PR decision — flagged in the design doc as out of scope for this design's level of detail.

No spec gaps requiring new tasks.

---

## Execution Handoff

Plan complete and saved to `docs/plans/2026-05-03-install-detect-symmetry-plan.md`. Two execution options:

**1. Subagent-Driven (recommended)** — I dispatch a fresh subagent per task, review between tasks, fast iteration. Best for plans where each task has clean boundaries and the review-between-tasks gate catches drift early.

**2. Inline Execution** — Execute tasks in this session using `superpowers:executing-plans`, batch execution with checkpoints. Best when the plan is straightforward enough that you'd rather watch it land and steer in real time.

Which approach?

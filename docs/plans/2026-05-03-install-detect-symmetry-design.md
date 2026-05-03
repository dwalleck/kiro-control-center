# Install ↔ Detect Symmetry — Tracking Schema Foundation

> **Status:** design draft. Implementation plan to be written next via the `superpowers:writing-plans` skill once this design is approved.
> **Closes:** issue #97 (skills detection ignores `manifest.skills` custom scan paths).
> **Depends on:** PR #96 (Phase 2a update-detection backend). Stacked PR — base branch is `feat/phase-2a-update-detection`.
> **Predecessor for:** issue #99 (umbrella for C-1 / C-2 / C-3 structural follow-ups).

## Goal

Make every installed-artifact tracking entry self-contained at detect time: each `Installed*Meta` records the manifest scan-path it was installed from, so detection becomes a direct lookup against a known source location. No probing, no fallbacks, no per-artifact-type ad hoc recovery patterns.

This closes #97 (the skills instance of the bug) and the underlying structural pattern that made #97 the third instance after PR #96 closed steering and agents via probe-fallback helpers.

## Background

PR #96 (Phase 2a) closed two of three instances of "detection has a hardcoded scan path; install honors `manifest.{steering,agents}`":
- Steering: closed via `hash_artifact_in_scan_paths` — a probe helper that tries each configured manifest scan path until one resolves.
- Native companions: same probe helper.
- Translated agents: closed via `InstalledAgentMeta.source_path: Option<RelativePath>` (NC2) — install records the per-file source path; detection reads it directly.
- **Skills:** still hardcoded `plugin_dir.join("skills")`. Tracked at #97.

The asymmetry is real: agents have the cleanest design (lookup-not-probe via tracked field) because the dialect-fallback machinery forced install-side path tracking early. Steering and native-companions got probe-style fixes because their schema lacked anywhere to record the scan root. Skills got nothing.

This design closes all three remaining loops by giving every artifact type the agents-style treatment: a required field on the tracking metadata that records where install found the source, and a detection path that consults it directly.

The pattern is documented at the meta level in `docs/plan-review-checklist.md` Gate 6 ("Reference vs Transcription") — the original PR #96 plan transcribed default-config cache paths as literal strings instead of citing the install-side scan-path resolution. Gate 6 was added to catch the next instance of this shape; this design closes the underlying structural cause that made Gate 6 necessary.

## No-users assumption

This codebase has no production users. Tracking files in the wild are exclusively local development copies, expected to be regenerated. This unlocks two design choices that would otherwise be off the table:

1. **Required (not Option) tracking fields.** `source_scan_root: RelativePath` on the three new sites; `source_path: RelativePath` (no Option) on agents.
2. **No probe-fallback machinery.** PR #96's `hash_artifact_in_scan_paths` and the agents `agent_hash_inputs` dialect-fallback branch can be deleted outright.

Pre-existing tracking files written before this change will fail to deserialize (missing required field). Intentional. Users (= developers) regenerate by reinstalling the affected plugins.

## Schema changes

`crates/kiro-market-core/src/project.rs`:

```rust
pub struct InstalledSkillMeta {
    pub marketplace: MarketplaceName,
    pub plugin: PluginName,
    pub version: Option<String>,
    pub installed_at: DateTime<Utc>,
    pub source_hash: String,                  // CHANGED: was Option<String>
    pub installed_hash: String,               // CHANGED: was Option<String>
    pub source_scan_root: RelativePath,       // NEW: required
}

pub struct InstalledSteeringMeta {
    // ... unchanged ...
    pub source_scan_root: RelativePath,       // NEW: required
}

pub struct InstalledNativeCompanionsMeta {
    // ... unchanged ...
    pub source_scan_root: RelativePath,       // NEW: required
}

pub struct InstalledAgentMeta {
    // ... unchanged ...
    pub source_path: RelativePath,            // CHANGED: was Option<RelativePath>
    pub source_hash: String,                  // CHANGED: was Option<String>
    pub installed_hash: String,               // CHANGED: was Option<String>
}
```

**Naming convention.** Agents keeps `source_path` (full file path including filename, e.g. `agents/reviewer.md` or `custom/reviewer.agent.md`). The other three get `source_scan_root` (root-only, e.g. `skills` or `packs`). Names differ because semantics differ: agents need the filename (varies by dialect — `.md` / `.agent.md` / `.json`); the other three derive the file location from `source_scan_root` plus the existing tracking key (skill name, steering rel-path, or native companion `files` Vec).

**Storage format.** `RelativePath` serializes as a JSON string. No new format concerns. The newtype already enforces forward-slash separators (rejects backslashes at construction); on-disk format is consistently forward-slash regardless of the host OS. Detection's `Path::join` accepts forward-slash rels on both Windows and Unix.

**Multi-scan-root resolution at install.** When `manifest.{skills,steering,agents}` declares multiple scan paths and a file is found in more than one, install picks the first scan root that contains it (matches existing `discover_*_in_dirs` order). That choice is recorded in `source_scan_root`. Detection trusts what install stored and doesn't re-derive.

## Detection simplification

`scan_plugin_for_content_drift` (`service/mod.rs`) becomes straight-line — one path per artifact type, no probing, no fallbacks:

```rust
// Skills (was hardcoded plugin_dir.join("skills").join(name))
let skill_dir = plugin_dir.join(meta.source_scan_root.as_str()).join(name);
let computed = crate::hash::hash_dir_tree(&skill_dir)?;

// Steering (was hash_artifact_in_scan_paths probe-each-scan-path)
let scan_root = plugin_dir.join(meta.source_scan_root.as_str());
let computed = crate::hash::hash_artifact(&scan_root, std::slice::from_ref(rel_path))?;

// Native companions (was hash_artifact_in_scan_paths probe-each-scan-path)
let scan_root = plugin_dir.join(meta.source_scan_root.as_str());
let computed = crate::hash::hash_artifact(&scan_root, &meta.files)?;

// Translated agents (was agent_hash_inputs with dialect-fallback)
let full = plugin_dir.join(meta.source_path.as_str());
let parent = full.parent().unwrap_or(plugin_dir);
let filename = full.file_name().map(PathBuf::from).unwrap_or_else(|| meta.source_path.as_str().into());
let computed = crate::hash::hash_artifact(parent, std::slice::from_ref(&filename))?;
```

**Deletions.** All exclusively-legacy machinery goes:
- `hash_artifact_in_scan_paths` helper (~35 lines + its tests).
- `agent_hash_inputs`'s dialect-fallback branch (~10 lines).
- The I-N7 actionable-error branch in the agents loop (~30 lines, fired only when `source_path.is_none()`).
- The `manifest: Option<&PluginManifest>` parameter on `scan_plugin_for_content_drift` (manifest is no longer needed for scan-path resolution).
- The `legacy_fallback` boolean and its threading through `scan_plugin_for_content_drift`'s return tuple.

**Visibility cleanup.** `agent_scan_paths_for_plugin` and `steering_scan_paths_for_plugin` (`service/browse.rs`) were bumped to `pub(super)` for detection in PR #96. Detection no longer needs them; revert to `fn` (private to `browse.rs`) if no other consumer remains.

## Install-time changes

Each install path needs to populate the new field. Three of the four are easy because the install code already has the scan_root in scope; one needs a small refactor.

**Steering** (`install_plugin_steering`, `service/mod.rs:1540+`)
The `DiscoveredNativeFile` records returned by `discover_steering_files_in_dirs` already carry `scan_root: PathBuf`. Per-file install computes `scan_root.strip_prefix(plugin_dir)` → `RelativePath` via the shared normalization helper (below) and stores on `InstalledSteeringMeta.source_scan_root`. ~5 lines added.

**Native companions** (`install_native_companions_for_plugin`, `service/mod.rs:1880+`)
Single-scan-root invariant is enforced upstream by `multiple_companion_scan_roots`, so all companion files share one scan_root. Already passed via `NativeCompanionsInput.scan_root`. The persistence layer computes the relative path and stores on `InstalledNativeCompanionsMeta.source_scan_root`. ~3 lines.

**Translated agents** (`install_translated_agents_inner`, `service/mod.rs:1601+`)
Already populates `source_path: Option<RelativePath>` via `relative_source_path_for_tracking` (`service/mod.rs:2685+`). Change: drop the `Option` wrapper. The function's two existing failure modes (path not under `plugin_dir`, `RelativePath::new` rejection) currently warn and return `None`; promote both to typed `AgentError` install failures. Rationale: with the field required, install MUST be able to record the source path or the install can't safely complete — silent skip becomes a contract violation, hard error is the correct shape.

**Skills** (the actual refactor)
`discover_skill_dirs` (`plugin.rs:399`) returns `Vec<PathBuf>` of resolved skill directories — loses the scan_root context. Refactor to return:

```rust
pub struct DiscoveredSkill {
    pub scan_root: PathBuf,  // e.g. <plugin_dir>/skills/  or <plugin_dir>/packs/
    pub skill_dir: PathBuf,  // e.g. <plugin_dir>/skills/alpha/
}

pub fn discover_skill_dirs(plugin_root: &Path, skill_paths: &[&str]) -> Vec<DiscoveredSkill> {
    // Track which scan_root each skill came from instead of flattening.
}
```

Install iterates `Vec<DiscoveredSkill>`, copies the dir as today, and stores `scan_root.strip_prefix(plugin_dir)` (via the shared normalization helper) on `InstalledSkillMeta.source_scan_root`. The single existing caller (`discover_skills_for_plugin` in `browse.rs`) updates accordingly.

**Shared normalization helper.** The recipe for converting `PathBuf` → `RelativePath` with forward-slash normalization currently lives only in `relative_source_path_for_tracking`. After B it's needed at 4+ sites. Extract:

```rust
/// Convert a `Path` to a `RelativePath`, after stripping `plugin_dir`
/// and normalizing path separators to forward-slash. Returns Err if
/// the path is not under `plugin_dir` or cannot be expressed as a
/// valid `RelativePath`.
fn path_to_relative_under_plugin(
    path: &Path,
    plugin_dir: &Path,
) -> Result<RelativePath, RelativePathError>;
```

The forward-slash conversion is required because `RelativePath::new` rejects backslashes by construction (cross-platform portability of the wire format). On Windows, `Path::strip_prefix(...).to_string_lossy()` returns backslashes; without normalization, `RelativePath::new` fails. Pinned by the `relative_path_newtype_rejects_traversal_at_construction` test.

## Tests

**Deleted** (legacy-fallback paths that no longer exist):
- `detect_plugin_updates_legacy_fallback_source_hash_none`
- `detect_plugin_updates_legacy_fallback_no_version_bump_returns_no_update`
- `detect_plugin_updates_agent_legacy_fallback_source_hash_none`
- `detect_plugin_updates_copilot_agent_legacy_fallback`

**Reshaped** (intent intact; new field added to fixtures):
- `detect_plugin_updates_steering_with_custom_scan_path_no_false_drift` (added in PR #96 review-of-review)
- `detect_plugin_updates_native_companions_with_custom_scan_path_no_false_drift` (same)
- JSON-shape lock tests for `installed-{skills,steering,agents}.json` shapes

**Compiler-driven updates.** Every test that hand-constructs an `Installed*Meta` will fail to compile until the new required fields are supplied. Estimate: 15-25 sites across `service::tests` and `project::tests`. The compiler does the inventory.

**Added:**
- `detect_plugin_updates_skills_with_custom_scan_path_no_false_drift` — the regression test that closes #97. Direct sibling of the existing steering / native-companions regressions.
- `path_to_relative_under_plugin_normalizes_backslashes` — synthesises a `PathBuf` containing backslashes, runs through the shared helper, asserts the resulting `RelativePath` uses forward-slash. Tests Windows-native input on any OS.
- `load_installed_{skills,steering,agents,native_companions}_rejects_legacy_entry` (4 tests) — assert that a tracking file missing the new required field fails to deserialize with a clear error. Pins the "no legacy entries" foundation contract.
- `install_translated_agent_fails_when_source_path_outside_plugin_dir` — pins the warn-and-None → typed-error promotion for agents install.

Net test count change: +3 to +5 (4 deleted, 6-8 added).

## Migration

None. Pre-existing tracking files become invalid by design — see "No-users assumption" above. The deserialize-failure tests (`load_installed_*_rejects_legacy_entry`) document the contract: legacy entries are intentionally unparseable, with a clear error mentioning the missing field. Users (= developers) regenerate by reinstalling the affected plugins.

If users ever exist for this codebase, a migration story is in scope for that future work — not for this design.

## Out of scope

Deferred to issue #99:
- **C-1: Native-companion install/detect hash recipe.** Verify whether the bug documented on `detect_plugin_updates_copilot_agent_legacy_fallback`'s docstring exists; align if so. B closes the *location* asymmetry for native_companions but doesn't touch the *hash recipe* axis.
- **C-2: Lift `PluginManifest.{skills,steering,agents}: Vec<String>` to `Vec<RelativePath>`.** Parse-don't-validate at the manifest layer. Closes the rust-code-reviewer IMPORTANT 1 finding from PR #96 (asymmetric scan-path validation between install and detection).
- **C-3: `ContentHash` newtype.** Promotes `"blake3:" + hex` digests from `String` to a typed newtype. Independent axis from path tracking.

Other:
- **Backward compat / migration shims.** Explicitly out per the no-users assumption.
- **Cross-marketplace same-plugin idempotency** (`project.rs:2937-2956`, flagged in the PR #96 design as "may matter for Phase 2b") — separate axis.

## PR strategy

**Base branch.** `feat/phase-2a-update-detection` (PR #96). Stacked PR. Description includes "Depends on #96; merge after #96 lands." GitHub PR UI base set to PR #96's branch so the diff shows only B's changes.

**Six commits, each self-contained** (compiles + tests pass per commit, bisect-friendly):

1. `refactor(agents): tighten source_path to required, drop dialect-fallback`
2. `feat(skills): add InstalledSkillMeta.source_scan_root + use at detect time` (closes #97)
3. `feat(steering): add InstalledSteeringMeta.source_scan_root + use at detect time`
4. `feat(native-companions): add InstalledNativeCompanionsMeta.source_scan_root + use at detect time`
5. `refactor(detect): delete probe helpers + legacy_fallback (now dead)` — including tightening `source_hash: Option → String`
6. `chore: regenerate bindings.ts + add normalization helper test + deserialize-rejection tests`

**Estimated diff size:** 400-600 lines including tests. More deletes than adds on net (PR #96's probe helpers + the agents dialect-fallback go away).

**Review hooks.** Plan-lint should stay green throughout (no changes to error variants, classifiers, or external-error boundaries). `cargo test --workspace` + `cargo clippy --workspace --tests -- -D warnings` + `cargo fmt --all --check` per commit.

## Open questions

None at the time of writing. All Section 1-5 questions resolved during the brainstorm.

---

**Origin context.** Brainstormed via `superpowers:brainstorming` skill on 2026-05-03. Driver: user feedback on the immediate-fix approach to issue #97 ("Hold on, lets think about the right thing to do long term. I don't want to do any more quick fixes"). The brainstorm explicitly chose option (b) over (a) and option (α) over (β) — the most aggressive cleanup variants — because the no-users assumption removes the migration burden that would otherwise constrain the design.

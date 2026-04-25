# Stage 2: Native kiro-cli Agent Import Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

> **⚠️ READ BEFORE EXECUTING — REVISIT AFTER STAGE 1.**
>
> This plan was written before Stage 1 (the content-hash primitive) landed. Before starting Stage 2 implementation:
>
> 1. **Confirm the actual `kiro_market_core::hash` API matches what's used here.** Stage 2 calls `hash_artifact(base, &[...])`. If Stage 1's implementation diverged (e.g. different signature, different return shape), update the install logic in this plan accordingly.
> 2. **Verify `InstalledAgentMeta` actually has `source_hash` / `installed_hash` fields.** Stage 1 added them. If Stage 1 named the fields differently or chose a different shape (e.g. a single combined `hash` field), update Tasks 11–18 below.
> 3. **Re-read the design doc.** `docs/plans/2026-04-23-kiro-cli-native-plugin-import-design.md` is the source of truth — if anything in Stage 1 implementation revealed gaps in the spec (e.g. concurrent-install issues, unhandled error cases, performance problems), the spec MAY have been amended. Cross-reference.
> 4. **Check that `AgentDialect` is still `non_exhaustive`** (it was at design time). If anything changed about the enum, the `Native` variant addition (Task 3) needs adjusting.

**Goal:** Implement the native kiro-cli agent import path: plugins declaring `format: "kiro-cli"` get their JSON agents and companion files installed via a validate-and-copy pipeline that preserves Kiro's native schema verbatim.

**Architecture:** New `format: Option<PluginFormat>` field on `PluginManifest` drives a runtime dispatch in `MarketplaceService::install_plugin_agents`. The native path discovers `.json` files at scan-path roots and companion files in subdirs, parses each agent JSON via a new `parse_native_kiro_agent_file` function, and installs each agent + the plugin-wide companion bundle via two new `KiroProject` methods. Plugin-scoped companion ownership tracking via a new `native_companions` map on `InstalledAgents`. Translated path is unchanged.

**Tech Stack:** Rust (edition 2024), serde / serde_json, blake3 + hex (via Stage 1's `hash` module), thiserror, existing `validation` + `with_file_lock` primitives.

**Spec reference:** `docs/plans/2026-04-23-kiro-cli-native-plugin-import-design.md` § "Manifest Schema", § "Layer Contracts", § "Type Changes", § "Tracking Schema and Content Hashes" (`InstalledNativeCompanionsMeta`), § "Implementation Phasing — Stage 2".

---

## File Structure

**New files:**
- `crates/kiro-market-core/src/agent/parse_native.rs` — `parse_native_kiro_agent_file`, `NativeAgentBundle`, `NativeParseFailure`

**Modified files:**
- `crates/kiro-market-core/src/plugin.rs` — `format: Option<PluginFormat>`, `PluginFormat` enum
- `crates/kiro-market-core/src/agent/mod.rs` — `pub mod parse_native;`, re-exports
- `crates/kiro-market-core/src/agent/types.rs` — `AgentDialect::Native` variant
- `crates/kiro-market-core/src/agent/discover.rs` — `discover_native_kiro_agents_in_dirs`, `discover_native_companion_files`, `DiscoveredNativeFile`
- `crates/kiro-market-core/src/error.rs` — five new `AgentError` variants
- `crates/kiro-market-core/src/project.rs`:
  - `InstalledNativeCompanionsMeta` (new type)
  - `InstalledAgents` (add `native_companions: HashMap<String, InstalledNativeCompanionsMeta>`)
  - `KiroProject::install_native_agent` (new method)
  - `KiroProject::install_native_companions` (new method)
- `crates/kiro-market-core/src/service/browse.rs` — `PluginInstallContext::format`, resolver reads `manifest.format`
- `crates/kiro-market-core/src/service/mod.rs` — dispatch in `install_plugin_agents`, new `install_native_kiro_cli_agents_inner`
- `crates/kiro-market/src/commands/install.rs` — surface native agent + companion outcomes in CLI

---

## Task 1: Add `PluginFormat` enum and parse `format` field

**Files:**
- Modify: `crates/kiro-market-core/src/plugin.rs`

- [ ] **Step 1: Write the failing tests**

Append to the `#[cfg(test)] mod tests` block in `crates/kiro-market-core/src/plugin.rs` (or create one if not present):

```rust
#[test]
fn manifest_parses_format_kiro_cli() {
    let json = br#"{"name": "p", "format": "kiro-cli"}"#;
    let manifest = PluginManifest::from_json(json).unwrap();
    assert_eq!(manifest.format, Some(PluginFormat::KiroCli));
}

#[test]
fn manifest_format_absent_is_none() {
    let json = br#"{"name": "p"}"#;
    let manifest = PluginManifest::from_json(json).unwrap();
    assert!(manifest.format.is_none());
}

#[test]
fn manifest_unknown_format_value_fails_loudly() {
    let json = br#"{"name": "p", "format": "kiro-ide"}"#;
    let err = PluginManifest::from_json(json).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("kiro-ide") || msg.contains("unknown variant"),
        "error must mention the unknown variant; got: {msg}"
    );
}
```

- [ ] **Step 2: Run tests, verify they fail**

Run: `cargo test -p kiro-market-core --lib plugin::tests::manifest_parses_format`
Expected: FAIL — `cannot find type 'PluginFormat'`.

- [ ] **Step 3: Add `PluginFormat` and `format` field**

In `crates/kiro-market-core/src/plugin.rs`, immediately after the `PluginManifest` struct, add:

```rust
/// The plugin's native authoring format. Drives dispatch in
/// `MarketplaceService::install_plugin_agents`: `KiroCli` skips
/// parse-and-translate and validates-and-copies native JSON agents.
/// Absent means the plugin uses Claude / Copilot markdown agents that
/// require translation (the existing default flow).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum PluginFormat {
    KiroCli,
    // Future: KiroIde, etc.
}
```

Add the field to `PluginManifest`:

```rust
#[derive(Debug, Clone, Deserialize)]
pub struct PluginManifest {
    pub name: String,
    pub version: Option<String>,
    pub description: Option<String>,
    #[serde(default)]
    pub skills: Vec<String>,
    #[serde(default)]
    pub agents: Vec<String>,

    /// Authoring format for this plugin. See `PluginFormat`.
    #[serde(default)]
    pub format: Option<PluginFormat>,
}
```

- [ ] **Step 4: Run tests, verify they pass**

Run: `cargo test -p kiro-market-core --lib plugin::tests::manifest_parses_format`
Expected: All three tests PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/kiro-market-core/src/plugin.rs
git commit -m "feat(core): add PluginFormat enum + format field on PluginManifest"
```

---

## Task 2: Add `AgentDialect::Native` variant

**Files:**
- Modify: `crates/kiro-market-core/src/agent/types.rs`

- [ ] **Step 1: Write the failing test**

Append to the `#[cfg(test)] mod tests` block at the bottom of `crates/kiro-market-core/src/agent/types.rs`:

```rust
#[test]
fn agent_dialect_native_serializes_to_native() {
    let d = AgentDialect::Native;
    let json = serde_json::to_string(&d).unwrap();
    assert_eq!(json, "\"native\"");

    let round: AgentDialect = serde_json::from_str("\"native\"").unwrap();
    assert_eq!(round, AgentDialect::Native);
}
```

- [ ] **Step 2: Run test, verify it fails**

Run: `cargo test -p kiro-market-core --lib agent::types::tests::agent_dialect_native_serializes_to_native`
Expected: FAIL — `no variant 'Native' on type 'AgentDialect'`.

- [ ] **Step 3: Add the variant**

In `crates/kiro-market-core/src/agent/types.rs`, find `pub enum AgentDialect`. Add `Native`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum AgentDialect {
    Claude,
    Copilot,
    /// Plugin authored in Kiro's native JSON format. Installed via
    /// validate-and-copy (no parse-and-translate).
    Native,
}
```

- [ ] **Step 4: Run test, verify it passes**

Run: `cargo test -p kiro-market-core --lib agent::types::tests::agent_dialect_native_serializes_to_native`
Expected: PASS.

- [ ] **Step 5: Audit classifiers for missing arms**

Run: `grep -rn "AgentDialect::" crates/kiro-market-core/src/`
For every `match dialect { ... }` (NOT `match _ { ... }` with a wildcard), verify there's an explicit arm for `Native`. The `non_exhaustive` attribute means external consumers won't break, but internal classifiers per CLAUDE.md ("classifier functions enumerate every variant explicitly") should match every variant. Add `AgentDialect::Native => ...` arms wherever Claude/Copilot are handled and the right behavior for Native is meaningful.

- [ ] **Step 6: Run full crate tests**

Run: `cargo test -p kiro-market-core`
Expected: All tests pass.

- [ ] **Step 7: Commit**

```bash
git add crates/kiro-market-core/src/agent/types.rs crates/kiro-market-core/src/
git commit -m "feat(core): add AgentDialect::Native variant"
```

---

## Task 3: Add `DiscoveredNativeFile` and `discover_native_kiro_agents_in_dirs`

**Files:**
- Modify: `crates/kiro-market-core/src/agent/discover.rs`

- [ ] **Step 1: Write the failing tests**

Append to the `#[cfg(test)] mod tests` block in `crates/kiro-market-core/src/agent/discover.rs`:

```rust
#[test]
fn native_discovery_finds_json_files_at_scan_root() {
    let tmp = tempdir().unwrap();
    let agents = tmp.path().join("agents");
    fs::create_dir_all(&agents).unwrap();
    fs::write(agents.join("a.json"), b"{}").unwrap();
    fs::write(agents.join("b.json"), b"{}").unwrap();
    fs::write(agents.join("ignore.md"), b"---\nname: ignore\n---\n").unwrap();

    let found = discover_native_kiro_agents_in_dirs(
        tmp.path(),
        &["./agents/".to_string()],
    );

    let names: Vec<_> = found
        .iter()
        .map(|f| f.source.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert!(names.contains(&"a.json".to_string()));
    assert!(names.contains(&"b.json".to_string()));
    assert!(!names.contains(&"ignore.md".to_string()));
}

#[test]
fn native_discovery_excludes_readme_case_insensitive() {
    let tmp = tempdir().unwrap();
    let agents = tmp.path().join("agents");
    fs::create_dir_all(&agents).unwrap();
    fs::write(agents.join("README.json"), b"{}").unwrap();
    fs::write(agents.join("readme.json"), b"{}").unwrap();
    fs::write(agents.join("real.json"), b"{}").unwrap();

    let found = discover_native_kiro_agents_in_dirs(
        tmp.path(),
        &["./agents/".to_string()],
    );

    let names: Vec<_> = found
        .iter()
        .map(|f| f.source.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, vec!["real.json"]);
}

#[test]
fn native_discovery_rejects_path_traversal() {
    let tmp = tempdir().unwrap();
    let plugin = tmp.path().join("plugin");
    fs::create_dir_all(&plugin).unwrap();
    let escape = tmp.path().join("secrets");
    fs::create_dir_all(&escape).unwrap();
    fs::write(escape.join("loot.json"), b"{}").unwrap();

    let found = discover_native_kiro_agents_in_dirs(
        &plugin,
        &["../secrets/".to_string()],
    );

    assert!(found.is_empty(), "path traversal must be rejected");
}

#[cfg(unix)]
#[test]
fn native_discovery_skips_symlinks() {
    use std::os::unix::fs::symlink;
    let tmp = tempdir().unwrap();
    let agents = tmp.path().join("agents");
    fs::create_dir_all(&agents).unwrap();
    fs::write(agents.join("real.json"), b"{}").unwrap();

    let outside = tmp.path().join("outside.json");
    fs::write(&outside, b"{}").unwrap();
    symlink(&outside, agents.join("evil.json")).unwrap();

    let found = discover_native_kiro_agents_in_dirs(
        tmp.path(),
        &["./agents/".to_string()],
    );

    let names: Vec<_> = found
        .iter()
        .map(|f| f.source.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, vec!["real.json"]);
}

#[test]
fn native_discovery_returns_scan_root_for_dest_path_computation() {
    let tmp = tempdir().unwrap();
    let agents = tmp.path().join("agents");
    fs::create_dir_all(&agents).unwrap();
    fs::write(agents.join("a.json"), b"{}").unwrap();

    let found = discover_native_kiro_agents_in_dirs(
        tmp.path(),
        &["./agents/".to_string()],
    );

    assert_eq!(found.len(), 1);
    assert_eq!(found[0].scan_root, agents);
}
```

- [ ] **Step 2: Run tests, verify they fail**

Run: `cargo test -p kiro-market-core --lib agent::discover::tests::native_discovery`
Expected: FAIL — `cannot find type 'DiscoveredNativeFile'`.

- [ ] **Step 3: Add `DiscoveredNativeFile` and the discovery function**

Append to `crates/kiro-market-core/src/agent/discover.rs` (after the existing `discover_agents_in_dirs` function but before `#[cfg(test)] mod tests`):

```rust
/// A file produced by native discovery. Carries the source path along with
/// the resolved scan-root the file was discovered under, so the install
/// layer can compute destination-relative paths without re-doing the join.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredNativeFile {
    /// Absolute path to the source file inside the plugin.
    pub source: PathBuf,
    /// The resolved scan-path directory (e.g. `<plugin>/agents/`).
    pub scan_root: PathBuf,
}

/// Find native Kiro agent JSON candidates: `.json` files at the root of
/// each scan path. Mirrors the security model of `discover_agents_in_dirs`:
/// validates each scan path, refuses symlinks, excludes README/CONTRIBUTING/
/// CHANGELOG, non-recursive at the scan-path level.
#[must_use]
pub fn discover_native_kiro_agents_in_dirs(
    plugin_dir: &Path,
    scan_paths: &[String],
) -> Vec<DiscoveredNativeFile> {
    let mut out = Vec::new();
    for rel in scan_paths {
        if let Err(e) = crate::validation::validate_relative_path(rel) {
            warn!(
                path = %rel,
                error = %e,
                "skipping native agent scan path that fails validation"
            );
            continue;
        }
        let dir = plugin_dir.join(rel.trim_start_matches("./"));
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
            Err(e) => {
                warn!(
                    path = %dir.display(),
                    error = %e,
                    "failed to read native agent scan directory; skipping"
                );
                continue;
            }
        };
        for entry in entries {
            let entry = match entry {
                Ok(e) => e,
                Err(e) => {
                    warn!(
                        dir = %dir.display(),
                        error = %e,
                        "failed to read directory entry; skipping"
                    );
                    continue;
                }
            };
            let path = entry.path();
            let file_type = match fs::symlink_metadata(&path) {
                Ok(m) => m.file_type(),
                Err(e) => {
                    warn!(
                        path = %path.display(),
                        error = %e,
                        "failed to stat native agent candidate; skipping"
                    );
                    continue;
                }
            };
            if file_type.is_symlink() {
                debug!(
                    path = %path.display(),
                    "skipping symlink in native agent scan directory"
                );
                continue;
            }
            if !file_type.is_file() {
                continue;
            }
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if EXCLUDED_FILENAMES
                .iter()
                .any(|excluded| {
                    // Compare basenames case-insensitively. EXCLUDED_FILENAMES
                    // are .md filenames; we strip the extension before
                    // comparing so README.json is also excluded.
                    let stem_excl = std::path::Path::new(excluded)
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or(excluded);
                    let stem_name = std::path::Path::new(name)
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or(name);
                    stem_excl.eq_ignore_ascii_case(stem_name)
                })
            {
                continue;
            }
            if Path::new(name)
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("json"))
            {
                out.push(DiscoveredNativeFile {
                    source: path,
                    scan_root: dir.clone(),
                });
            }
        }
    }
    out
}
```

- [ ] **Step 4: Run tests, verify they pass**

Run: `cargo test -p kiro-market-core --lib agent::discover::tests::native_discovery`
Expected: All five tests PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/kiro-market-core/src/agent/discover.rs
git commit -m "feat(core): add discover_native_kiro_agents_in_dirs + DiscoveredNativeFile"
```

---

## Task 4: Add `discover_native_companion_files`

**Files:**
- Modify: `crates/kiro-market-core/src/agent/discover.rs`

- [ ] **Step 1: Write the failing tests**

Append to the `#[cfg(test)] mod tests` block:

```rust
#[test]
fn companion_discovery_finds_files_one_level_deep() {
    let tmp = tempdir().unwrap();
    let agents = tmp.path().join("agents");
    let prompts = agents.join("prompts");
    fs::create_dir_all(&prompts).unwrap();
    fs::write(prompts.join("a.md"), b"prompt a").unwrap();
    fs::write(prompts.join("b.md"), b"prompt b").unwrap();
    // A top-level .json (would be an agent, NOT a companion).
    fs::write(agents.join("agent.json"), b"{}").unwrap();

    let found = discover_native_companion_files(
        tmp.path(),
        &["./agents/".to_string()],
    );

    let names: Vec<_> = found
        .iter()
        .map(|f| f.source.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert!(names.contains(&"a.md".to_string()));
    assert!(names.contains(&"b.md".to_string()));
    assert!(!names.contains(&"agent.json".to_string()));
}

#[test]
fn companion_discovery_does_not_recurse_more_than_one_level() {
    let tmp = tempdir().unwrap();
    let agents = tmp.path().join("agents");
    let nested = agents.join("prompts").join("nested");
    fs::create_dir_all(&nested).unwrap();
    fs::write(agents.join("prompts/top.md"), b"top").unwrap();
    fs::write(nested.join("deep.md"), b"deep").unwrap();

    let found = discover_native_companion_files(
        tmp.path(),
        &["./agents/".to_string()],
    );

    let names: Vec<_> = found
        .iter()
        .map(|f| f.source.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert!(names.contains(&"top.md".to_string()));
    assert!(!names.contains(&"deep.md".to_string()));
}

#[test]
fn companion_discovery_skips_symlinks_in_subdir() {
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        let tmp = tempdir().unwrap();
        let prompts = tmp.path().join("agents/prompts");
        fs::create_dir_all(&prompts).unwrap();
        fs::write(prompts.join("real.md"), b"real").unwrap();
        let outside = tmp.path().join("outside.md");
        fs::write(&outside, b"outside").unwrap();
        symlink(&outside, prompts.join("evil.md")).unwrap();

        let found = discover_native_companion_files(
            tmp.path(),
            &["./agents/".to_string()],
        );

        let names: Vec<_> = found
            .iter()
            .map(|f| f.source.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["real.md"]);
    }
}

#[test]
fn companion_discovery_returns_empty_when_no_subdirs() {
    let tmp = tempdir().unwrap();
    let agents = tmp.path().join("agents");
    fs::create_dir_all(&agents).unwrap();
    fs::write(agents.join("only.json"), b"{}").unwrap();

    let found = discover_native_companion_files(
        tmp.path(),
        &["./agents/".to_string()],
    );

    assert!(found.is_empty());
}
```

- [ ] **Step 2: Run tests, verify they fail**

Run: `cargo test -p kiro-market-core --lib agent::discover::tests::companion_discovery`
Expected: FAIL — `cannot find function 'discover_native_companion_files'`.

- [ ] **Step 3: Implement `discover_native_companion_files`**

Append to `crates/kiro-market-core/src/agent/discover.rs` (after `discover_native_kiro_agents_in_dirs`):

```rust
/// Find companion file candidates: any regular (non-symlink) file inside
/// subdirectories of a scan path, exactly one level deep.
///
/// Plugin-wide — not attributed to any specific agent. The install layer
/// treats the result as one atomic bundle owned by the plugin.
///
/// `scan_paths` are the same agent scan paths used by
/// `discover_native_kiro_agents_in_dirs`. README/CONTRIBUTING/CHANGELOG are
/// excluded by basename (case-insensitive).
#[must_use]
pub fn discover_native_companion_files(
    plugin_dir: &Path,
    scan_paths: &[String],
) -> Vec<DiscoveredNativeFile> {
    let mut out = Vec::new();
    for rel in scan_paths {
        if let Err(e) = crate::validation::validate_relative_path(rel) {
            warn!(
                path = %rel,
                error = %e,
                "skipping native companion scan path that fails validation"
            );
            continue;
        }
        let scan_root = plugin_dir.join(rel.trim_start_matches("./"));
        let entries = match fs::read_dir(&scan_root) {
            Ok(entries) => entries,
            Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
            Err(e) => {
                warn!(
                    path = %scan_root.display(),
                    error = %e,
                    "failed to read native companion scan directory; skipping"
                );
                continue;
            }
        };
        for entry in entries {
            let entry = match entry {
                Ok(e) => e,
                Err(e) => {
                    warn!(
                        dir = %scan_root.display(),
                        error = %e,
                        "failed to read directory entry; skipping"
                    );
                    continue;
                }
            };
            let subdir = entry.path();
            let md = match fs::symlink_metadata(&subdir) {
                Ok(m) => m,
                Err(e) => {
                    warn!(
                        path = %subdir.display(),
                        error = %e,
                        "failed to stat companion subdir candidate; skipping"
                    );
                    continue;
                }
            };
            if md.file_type().is_symlink() || !md.file_type().is_dir() {
                continue;
            }
            // Walk one level into the subdir.
            let inner = match fs::read_dir(&subdir) {
                Ok(i) => i,
                Err(e) => {
                    warn!(
                        path = %subdir.display(),
                        error = %e,
                        "failed to read companion subdir; skipping"
                    );
                    continue;
                }
            };
            for inner_entry in inner {
                let inner_entry = match inner_entry {
                    Ok(e) => e,
                    Err(e) => {
                        warn!(
                            dir = %subdir.display(),
                            error = %e,
                            "failed to read companion entry; skipping"
                        );
                        continue;
                    }
                };
                let inner_path = inner_entry.path();
                let inner_md = match fs::symlink_metadata(&inner_path) {
                    Ok(m) => m,
                    Err(e) => {
                        warn!(
                            path = %inner_path.display(),
                            error = %e,
                            "failed to stat companion file; skipping"
                        );
                        continue;
                    }
                };
                if inner_md.file_type().is_symlink() {
                    debug!(
                        path = %inner_path.display(),
                        "skipping symlink in companion subdir"
                    );
                    continue;
                }
                if !inner_md.file_type().is_file() {
                    continue;
                }
                let Some(name) = inner_path.file_name().and_then(|n| n.to_str())
                else {
                    continue;
                };
                if EXCLUDED_FILENAMES
                    .iter()
                    .any(|excluded| excluded.eq_ignore_ascii_case(name))
                {
                    continue;
                }
                out.push(DiscoveredNativeFile {
                    source: inner_path,
                    scan_root: scan_root.clone(),
                });
            }
        }
    }
    out
}
```

- [ ] **Step 4: Run tests, verify they pass**

Run: `cargo test -p kiro-market-core --lib agent::discover::tests::companion_discovery`
Expected: All four tests PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/kiro-market-core/src/agent/discover.rs
git commit -m "feat(core): add discover_native_companion_files (plugin-scoped, 1-deep)"
```

---

## Task 5: Create `parse_native.rs` with `NativeAgentBundle` and `NativeParseFailure`

**Files:**
- Create: `crates/kiro-market-core/src/agent/parse_native.rs`
- Modify: `crates/kiro-market-core/src/agent/mod.rs`

- [ ] **Step 1: Create `parse_native.rs` skeleton**

Create `crates/kiro-market-core/src/agent/parse_native.rs`:

```rust
//! Parse native Kiro agent JSON files into `NativeAgentBundle` for the
//! validate-and-copy install path. This module deliberately does NOT
//! model the full Kiro agent schema — only the fields the install layer
//! acts on (`name`, `mcpServers`). The rest of the JSON is preserved as
//! `serde_json::Value` and copied verbatim.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::agent::types::McpServerConfig;
use crate::validation;

/// A parsed native Kiro agent ready for install.
#[derive(Debug, Clone)]
pub struct NativeAgentBundle {
    /// Absolute path to the source `.json` file.
    pub agent_json_source: PathBuf,
    /// The scan root (e.g. `<plugin>/agents/`) the JSON was discovered under.
    /// Used for computing destination-relative paths and for hashing.
    pub scan_root: PathBuf,
    /// Validated agent name (from JSON `name` field). Path-safe per
    /// `validation::validate_name`.
    pub name: String,
    /// MCP server entries from the JSON's `mcpServers` field. Empty if the
    /// field is absent or empty. Drives the `--accept-mcp` install gate.
    pub mcp_servers: BTreeMap<String, McpServerConfig>,
    /// The full parsed JSON, preserved for atomic copy-out at install time.
    /// Avoids re-reading the file from disk during the install rename.
    pub raw_json: serde_json::Value,
}

/// Failure modes for `parse_native_kiro_agent_file`. Mirrors the existing
/// `ParseFailure` for translated agents — structured variants instead of
/// free-form strings, so callers can branch on the semantic.
#[derive(Debug)]
pub enum NativeParseFailure {
    /// File could not be read (permission denied, racy delete, etc.).
    IoError(io::Error),
    /// File is not valid JSON.
    InvalidJson(serde_json::Error),
    /// JSON parsed but the required `name` field is missing.
    MissingName,
    /// `name` field is present but failed `validate_name`. Carries the
    /// validator's reason.
    InvalidName(String),
}

impl std::fmt::Display for NativeParseFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::IoError(e) => write!(f, "read failed: {e}"),
            Self::InvalidJson(e) => write!(f, "invalid JSON: {e}"),
            Self::MissingName => f.write_str("missing required `name` field"),
            Self::InvalidName(r) => write!(f, "invalid `name`: {r}"),
        }
    }
}

/// The minimal projection we need to read out of the JSON to validate +
/// classify the agent. Everything else stays in `raw_json`.
#[derive(Deserialize)]
struct NativeAgentProjection {
    name: Option<String>,
    #[serde(default, rename = "mcpServers")]
    mcp_servers: BTreeMap<String, McpServerConfig>,
}

/// Parse a candidate native Kiro agent JSON file.
///
/// Returns `Ok(NativeAgentBundle)` on success, `Err(NativeParseFailure)` for
/// structured failures the caller can route into `failed` outcomes.
pub fn parse_native_kiro_agent_file(
    json_path: &Path,
    scan_root: &Path,
) -> Result<NativeAgentBundle, NativeParseFailure> {
    let bytes = std::fs::read(json_path).map_err(NativeParseFailure::IoError)?;
    let raw_json: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(NativeParseFailure::InvalidJson)?;
    let projection: NativeAgentProjection = serde_json::from_slice(&bytes)
        .map_err(NativeParseFailure::InvalidJson)?;

    let name = projection.name.ok_or(NativeParseFailure::MissingName)?;
    validation::validate_name(&name)
        .map_err(|e| NativeParseFailure::InvalidName(e.to_string()))?;

    Ok(NativeAgentBundle {
        agent_json_source: json_path.to_path_buf(),
        scan_root: scan_root.to_path_buf(),
        name,
        mcp_servers: projection.mcp_servers,
        raw_json,
    })
}
```

- [ ] **Step 2: Wire into `agent/mod.rs`**

In `crates/kiro-market-core/src/agent/mod.rs`, add:

```rust
pub mod parse_native;
pub use parse_native::{parse_native_kiro_agent_file, NativeAgentBundle, NativeParseFailure};
```

- [ ] **Step 3: Write parser tests**

Append a `#[cfg(test)] mod tests` to `crates/kiro-market-core/src/agent/parse_native.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn write_json(dir: &Path, name: &str, body: &str) -> PathBuf {
        let p = dir.join(name);
        fs::write(&p, body).unwrap();
        p
    }

    #[test]
    fn parses_minimal_valid_kiro_agent() {
        let tmp = tempdir().unwrap();
        let p = write_json(
            tmp.path(),
            "rev.json",
            r#"{"name": "rev", "prompt": "..."}"#,
        );
        let b = parse_native_kiro_agent_file(&p, tmp.path()).unwrap();
        assert_eq!(b.name, "rev");
        assert!(b.mcp_servers.is_empty());
        assert_eq!(b.scan_root, tmp.path());
    }

    #[test]
    fn missing_name_returns_missing_name_failure() {
        let tmp = tempdir().unwrap();
        let p = write_json(tmp.path(), "x.json", r#"{"prompt": "hi"}"#);
        let err = parse_native_kiro_agent_file(&p, tmp.path()).unwrap_err();
        assert!(matches!(err, NativeParseFailure::MissingName));
    }

    #[test]
    fn invalid_name_returns_invalid_name_failure() {
        let tmp = tempdir().unwrap();
        let p = write_json(tmp.path(), "x.json", r#"{"name": "../evil"}"#);
        let err = parse_native_kiro_agent_file(&p, tmp.path()).unwrap_err();
        match err {
            NativeParseFailure::InvalidName(reason) => {
                assert!(!reason.is_empty(), "reason must not be empty");
            }
            other => panic!("expected InvalidName, got {other:?}"),
        }
    }

    #[test]
    fn malformed_json_returns_invalid_json_failure() {
        let tmp = tempdir().unwrap();
        let p = write_json(tmp.path(), "x.json", r#"{not json"#);
        let err = parse_native_kiro_agent_file(&p, tmp.path()).unwrap_err();
        assert!(matches!(err, NativeParseFailure::InvalidJson(_)));
    }

    #[test]
    fn extracts_mcp_servers_field() {
        let tmp = tempdir().unwrap();
        let p = write_json(
            tmp.path(),
            "with_mcp.json",
            r#"{
                "name": "x",
                "mcpServers": {
                    "tool": { "type": "stdio", "command": "echo", "args": ["hi"] }
                }
            }"#,
        );
        let b = parse_native_kiro_agent_file(&p, tmp.path()).unwrap();
        assert_eq!(b.mcp_servers.len(), 1);
        assert!(b.mcp_servers["tool"].is_stdio());
    }

    #[test]
    fn missing_file_returns_io_error() {
        let tmp = tempdir().unwrap();
        let nonexistent = tmp.path().join("nope.json");
        let err = parse_native_kiro_agent_file(&nonexistent, tmp.path())
            .unwrap_err();
        assert!(matches!(err, NativeParseFailure::IoError(_)));
    }
}
```

- [ ] **Step 4: Run tests, verify they pass**

Run: `cargo test -p kiro-market-core --lib agent::parse_native::tests`
Expected: All six tests PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/kiro-market-core/src/agent/parse_native.rs crates/kiro-market-core/src/agent/mod.rs
git commit -m "feat(core): add parse_native_kiro_agent_file + NativeAgentBundle"
```

---

## Task 6: Add five new `AgentError` variants

**Files:**
- Modify: `crates/kiro-market-core/src/error.rs`

- [ ] **Step 1: Write the failing test**

Append to (or create if absent) the `#[cfg(test)] mod tests` block in `crates/kiro-market-core/src/error.rs`:

```rust
#[test]
fn name_clash_with_other_plugin_renders_useful_message() {
    let err = AgentError::NameClashWithOtherPlugin {
        name: "code-reviewer".into(),
        owner: "other-plugin".into(),
    };
    let msg = err.to_string();
    assert!(msg.contains("code-reviewer"));
    assert!(msg.contains("other-plugin"));
    assert!(msg.contains("--force"));
}

#[test]
fn content_changed_requires_force_renders_useful_message() {
    let err = AgentError::ContentChangedRequiresForce {
        name: "x".into(),
    };
    let msg = err.to_string();
    assert!(msg.contains("x"));
    assert!(msg.contains("--force"));
}
```

- [ ] **Step 2: Run tests, verify they fail**

Run: `cargo test -p kiro-market-core --lib error::tests::name_clash`
Expected: FAIL — `no variant 'NameClashWithOtherPlugin' on type 'AgentError'`.

- [ ] **Step 3: Add variants to `AgentError`**

Find `pub enum AgentError` in `crates/kiro-market-core/src/error.rs`. Add five variants (placement: after the existing variants, in any order):

```rust
#[error("native agent JSON `{path}` failed to parse")]
NativeManifestParseFailed {
    path: PathBuf,
    #[source]
    source: serde_json::Error,
},

#[error("native agent at `{path}` is missing the required `name` field")]
NativeManifestMissingName { path: PathBuf },

#[error("native agent at `{path}` has an invalid `name`: {reason}")]
NativeManifestInvalidName { path: PathBuf, reason: String },

#[error(
    "native agent name `{name}` would clobber an agent owned by plugin \
     `{owner}`; pass --force to transfer ownership"
)]
NameClashWithOtherPlugin { name: String, owner: String },

#[error(
    "agent `{name}` content has changed since last install; \
     pass --force to overwrite"
)]
ContentChangedRequiresForce { name: String },
```

(Imports: ensure `use std::path::PathBuf;` is present at the top of the file. Add if missing.)

- [ ] **Step 4: Run tests, verify they pass**

Run: `cargo test -p kiro-market-core --lib error::tests::name_clash`
Expected: PASS.

- [ ] **Step 5: Audit `SkippedReason::from_plugin_error` and `remediation_hint`**

Run: `grep -rn "SkippedReason::from_plugin_error\|fn remediation_hint" crates/kiro-market-core/src/`
For each match, open the function and add explicit arms for the five new variants. Per CLAUDE.md, no `_ => default` is allowed — every variant gets an explicit decision.

Example for `from_plugin_error` (adapt to actual definition):

```rust
match err {
    // ... existing arms ...
    AgentError::NativeManifestParseFailed { .. }
    | AgentError::NativeManifestMissingName { .. }
    | AgentError::NativeManifestInvalidName { .. } => {
        Some(SkippedReason::AgentParseFailed { /* fields */ })
    }
    AgentError::NameClashWithOtherPlugin { .. } => None,
    AgentError::ContentChangedRequiresForce { .. } => None,
}
```

The exact mapping depends on what `SkippedReason` represents in the project — when in doubt, return `None` for the new variants (they're collision/parse failures that surface as `failed` entries, not `skipped`).

- [ ] **Step 6: Run full crate tests**

Run: `cargo test -p kiro-market-core`
Expected: All tests pass.

- [ ] **Step 7: Commit**

```bash
git add crates/kiro-market-core/src/
git commit -m "feat(core): add five native-agent variants to AgentError"
```

---

## Task 7: Extend `PluginInstallContext` with `format` field

**Files:**
- Modify: `crates/kiro-market-core/src/service/browse.rs`

- [ ] **Step 1: Find the existing `PluginInstallContext` struct**

Run: `grep -n "struct PluginInstallContext\|fn resolve_plugin_install_context_from_dir" crates/kiro-market-core/src/service/browse.rs`

Note the line numbers for both. The struct is what we extend; the function is what populates the new field.

- [ ] **Step 2: Write the failing test**

Append to the `#[cfg(test)] mod tests` block in `crates/kiro-market-core/src/service/browse.rs`:

```rust
#[test]
fn resolve_plugin_install_context_reads_format_kiro_cli() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("plugin.json"),
        br#"{"name": "p", "format": "kiro-cli"}"#,
    )
    .unwrap();

    let ctx = resolve_plugin_install_context_from_dir(tmp.path()).unwrap();
    assert_eq!(ctx.format, Some(crate::plugin::PluginFormat::KiroCli));
}

#[test]
fn resolve_plugin_install_context_format_absent_is_none() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("plugin.json"),
        br#"{"name": "p"}"#,
    )
    .unwrap();

    let ctx = resolve_plugin_install_context_from_dir(tmp.path()).unwrap();
    assert!(ctx.format.is_none());
}
```

- [ ] **Step 3: Run tests, verify they fail**

Run: `cargo test -p kiro-market-core --lib service::browse::tests::resolve_plugin_install_context_reads_format`
Expected: FAIL — `no field 'format' on type 'PluginInstallContext'`.

- [ ] **Step 4: Add the field to the struct**

Find `pub struct PluginInstallContext`. Add:

```rust
/// Authoring format for this plugin (drives native vs translated dispatch).
pub format: Option<crate::plugin::PluginFormat>,
```

- [ ] **Step 5: Update `resolve_plugin_install_context_from_dir` to populate it**

Find `resolve_plugin_install_context_from_dir`. The function reads the `PluginManifest` and constructs a `PluginInstallContext`. Add:

```rust
// In the constructor literal:
format: manifest.format,
```

- [ ] **Step 6: Run tests, verify they pass**

Run: `cargo test -p kiro-market-core --lib service::browse::tests::resolve_plugin_install_context_reads_format`
Expected: Both tests PASS.

- [ ] **Step 7: Commit**

```bash
git add crates/kiro-market-core/src/service/browse.rs
git commit -m "feat(core): add format field to PluginInstallContext"
```

---

## Task 8: Add `InstalledNativeCompanionsMeta` and `native_companions` map

**Files:**
- Modify: `crates/kiro-market-core/src/project.rs` (around line 52-71)

- [ ] **Step 1: Write the backward-compat test**

Append to the `#[cfg(test)] mod tests` block in `crates/kiro-market-core/src/project.rs`:

```rust
#[test]
fn installed_agents_loads_legacy_json_without_native_companions() {
    let legacy = br#"{
        "agents": {
            "x": {
                "marketplace": "m",
                "plugin": "p",
                "version": null,
                "installed_at": "2026-01-01T00:00:00Z",
                "dialect": "claude"
            }
        }
    }"#;

    let installed: InstalledAgents = serde_json::from_slice(legacy).unwrap();
    assert_eq!(installed.agents.len(), 1);
    assert!(installed.native_companions.is_empty());
}
```

- [ ] **Step 2: Run test, verify it fails**

Run: `cargo test -p kiro-market-core --lib installed_agents_loads_legacy_json_without_native_companions`
Expected: FAIL — `no field 'native_companions' on type 'InstalledAgents'`.

- [ ] **Step 3: Add `InstalledNativeCompanionsMeta`**

In `crates/kiro-market-core/src/project.rs`, after the `InstalledAgentMeta` struct definition (around line 64), add:

```rust
/// Tracking entry for a plugin's native companion file bundle.
///
/// Native plugins ship companion files (e.g. `prompts/`) that may be
/// referenced by multiple agents in the same plugin. Ownership is at the
/// plugin level (not per-agent), so this entry tracks the union of files
/// installed for one plugin's bundle.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledNativeCompanionsMeta {
    pub marketplace: String,
    pub plugin: String,
    pub version: Option<String>,
    pub installed_at: DateTime<Utc>,
    /// Relative paths under `.kiro/agents/` of every companion file owned
    /// by this plugin's bundle. Used for collision detection (cross-plugin
    /// path overlap) and for uninstall.
    pub files: Vec<PathBuf>,
    pub source_hash: String,
    pub installed_hash: String,
}
```

- [ ] **Step 4: Extend `InstalledAgents`**

Find `pub struct InstalledAgents` (around line 67) and add the field:

```rust
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InstalledAgents {
    pub agents: HashMap<String, InstalledAgentMeta>,
    /// Per-plugin companion file ownership (native plugins only).
    /// Defaults to empty for backward compat with legacy tracking files.
    #[serde(default)]
    pub native_companions: HashMap<String, InstalledNativeCompanionsMeta>,
}
```

- [ ] **Step 5: Run test, verify it passes**

Run: `cargo test -p kiro-market-core --lib installed_agents_loads_legacy_json_without_native_companions`
Expected: PASS.

- [ ] **Step 6: Run full crate tests, fix any breakage**

Run: `cargo test -p kiro-market-core`
Expected: All tests pass. Any test that compares `InstalledAgents` literally may need a `..Default::default()` to pick up the new field — fix any such test by adding `native_companions: HashMap::new()` or using `..Default::default()`.

- [ ] **Step 7: Commit**

```bash
git add crates/kiro-market-core/src/project.rs
git commit -m "feat(core): add InstalledNativeCompanionsMeta + native_companions map"
```

---

## Task 9: Define `InstalledNativeAgentOutcome` and `InstalledNativeCompanionsOutcome`

**Files:**
- Modify: `crates/kiro-market-core/src/project.rs`

These are the in-memory return types for the new install methods. Defined now so Tasks 10-15 can reference them.

- [ ] **Step 1: Add the outcome types**

Append to `crates/kiro-market-core/src/project.rs` (after the tracking type definitions, before the `KiroProject` impl block):

```rust
/// Per-call outcome of `install_native_agent`. Carries enough info for the
/// service layer to render a row in the install summary.
#[derive(Debug, Clone)]
pub struct InstalledNativeAgentOutcome {
    pub name: String,
    pub json_path: PathBuf,
    /// True if `--force` overwrote a tracked path (orphan or other plugin).
    pub forced_overwrite: bool,
    /// True if the install was a no-op because tracking matched
    /// `source_hash` exactly (idempotent reinstall).
    pub was_idempotent: bool,
    pub source_hash: String,
    pub installed_hash: String,
}

/// Per-call outcome of `install_native_companions`.
#[derive(Debug, Clone)]
pub struct InstalledNativeCompanionsOutcome {
    pub plugin: String,
    pub files: Vec<PathBuf>,
    pub forced_overwrite: bool,
    pub was_idempotent: bool,
    pub source_hash: String,
    pub installed_hash: String,
}
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo build -p kiro-market-core`
Expected: Builds. (No tests yet — these types are wired up in Tasks 10+.)

- [ ] **Step 3: Commit**

```bash
git add crates/kiro-market-core/src/project.rs
git commit -m "feat(core): add InstalledNative{Agent,Companions}Outcome types"
```

---

## Task 10: `KiroProject::install_native_agent` — happy path

**Files:**
- Modify: `crates/kiro-market-core/src/project.rs`

- [ ] **Step 1: Write the test**

Append to the `#[cfg(test)] mod tests` block. (Reuse the existing `KiroProject::new(tmp.path())` and `tempdir()` patterns.)

```rust
#[test]
fn install_native_agent_writes_json_with_dialect_native_and_hashes() {
    let tmp = tempdir().unwrap();
    let project = KiroProject::new(tmp.path()).unwrap();

    // Write a source agent JSON.
    let src_dir = tmp.path().join("source-agents");
    fs::create_dir_all(&src_dir).unwrap();
    let src_json = src_dir.join("rev.json");
    fs::write(
        &src_json,
        br#"{"name": "rev", "prompt": "You are a reviewer."}"#,
    )
    .unwrap();

    let bundle = crate::agent::parse_native_kiro_agent_file(&src_json, &src_dir)
        .unwrap();

    let source_hash = crate::hash::hash_artifact(
        &src_dir,
        &[std::path::PathBuf::from("rev.json")],
    )
    .unwrap();

    let outcome = project
        .install_native_agent(
            &bundle,
            "marketplace-x",
            "plugin-y",
            Some("0.1.0"),
            &source_hash,
            false,
        )
        .unwrap();

    assert_eq!(outcome.name, "rev");
    assert!(outcome.json_path.ends_with(".kiro/agents/rev.json"));
    assert!(!outcome.forced_overwrite);
    assert!(!outcome.was_idempotent);
    assert_eq!(outcome.source_hash, source_hash);
    assert!(outcome.installed_hash.starts_with("blake3:"));

    // The file landed.
    assert!(outcome.json_path.exists());

    // Tracking entry exists with dialect Native.
    let tracking = project.load_installed_agents().unwrap();
    let entry = tracking.agents.get("rev").expect("entry persisted");
    assert_eq!(entry.dialect, crate::agent::AgentDialect::Native);
    assert_eq!(entry.plugin, "plugin-y");
    assert_eq!(entry.marketplace, "marketplace-x");
    assert_eq!(entry.source_hash.as_deref(), Some(source_hash.as_str()));
    assert!(entry.installed_hash.is_some());
}
```

- [ ] **Step 2: Run test, verify it fails**

Run: `cargo test -p kiro-market-core --lib install_native_agent_writes_json_with_dialect_native_and_hashes`
Expected: FAIL — `no method named 'install_native_agent'`.

- [ ] **Step 3: Implement `install_native_agent`**

Add a new method on `impl KiroProject` (find the existing `impl KiroProject` block; place after `install_agent_force` for thematic grouping):

```rust
/// Install one native Kiro agent JSON.
///
/// Idempotent if `source_hash` matches the tracked entry. Otherwise, with
/// `force == false`, returns:
/// - `NameClashWithOtherPlugin` if another plugin already tracks an agent
///   with this name,
/// - `ContentChangedRequiresForce` if the same plugin has a different
///   `source_hash` tracked,
/// - `OrphanFileAtDestination` if the destination JSON exists with no
///   tracking entry,
/// - other I/O errors otherwise.
///
/// With `force == true`, conflicts are overwritten and ownership transfers
/// to the new plugin.
pub fn install_native_agent(
    &self,
    bundle: &crate::agent::NativeAgentBundle,
    marketplace: &str,
    plugin: &str,
    version: Option<&str>,
    source_hash: &str,
    force: bool,
) -> Result<InstalledNativeAgentOutcome, AgentError> {
    validation::validate_name(&bundle.name)?;

    let json_target = self.agents_dir().join(format!("{}.json", &bundle.name));

    crate::file_lock::with_file_lock(
        &self.agent_tracking_path(),
        || -> Result<InstalledNativeAgentOutcome, AgentError> {
            let mut installed = self.load_installed_agents()?;

            // Idempotency / collision check.
            let mut forced_overwrite = false;
            let mut was_idempotent = false;
            if let Some(existing) = installed.agents.get(&bundle.name) {
                if existing.plugin == plugin {
                    if existing.source_hash.as_deref() == Some(source_hash) {
                        // True idempotent reinstall.
                        was_idempotent = true;
                        return Ok(InstalledNativeAgentOutcome {
                            name: bundle.name.clone(),
                            json_path: json_target,
                            forced_overwrite: false,
                            was_idempotent: true,
                            source_hash: source_hash.to_string(),
                            installed_hash: existing
                                .installed_hash
                                .clone()
                                .unwrap_or_default(),
                        });
                    } else if !force {
                        return Err(AgentError::ContentChangedRequiresForce {
                            name: bundle.name.clone(),
                        });
                    } else {
                        forced_overwrite = true;
                    }
                } else if !force {
                    return Err(AgentError::NameClashWithOtherPlugin {
                        name: bundle.name.clone(),
                        owner: existing.plugin.clone(),
                    });
                } else {
                    forced_overwrite = true;
                }
            } else if json_target.exists() {
                if !force {
                    return Err(AgentError::OrphanFileAtDestination {
                        path: json_target.clone(),
                    });
                }
                forced_overwrite = true;
            }

            // Stage and rename.
            let staging = self.fresh_agent_staging_dir(&bundle.name);
            let staging_json = staging.join("agent.json");
            std::fs::create_dir_all(&staging)?;
            // Re-serialize the raw_json so we know exactly what bytes land.
            let pretty = serde_json::to_vec_pretty(&bundle.raw_json)?;
            if let Err(e) = std::fs::write(&staging_json, &pretty) {
                remove_staging_dir(&staging);
                return Err(e.into());
            }

            std::fs::create_dir_all(self.agents_dir())?;

            if force && json_target.exists() {
                if let Err(e) = std::fs::remove_file(&json_target)
                    && e.kind() != std::io::ErrorKind::NotFound
                {
                    remove_staging_dir(&staging);
                    return Err(e.into());
                }
            }

            if let Err(e) = std::fs::rename(&staging_json, &json_target) {
                remove_staging_dir(&staging);
                return Err(e.into());
            }
            // Best-effort cleanup of staging dir.
            let _ = std::fs::remove_dir_all(&staging);

            // Compute installed_hash over what landed.
            let installed_hash = crate::hash::hash_artifact(
                &self.agents_dir(),
                &[PathBuf::from(format!("{}.json", &bundle.name))],
            )?;

            // Update tracking.
            installed.agents.insert(
                bundle.name.clone(),
                InstalledAgentMeta {
                    marketplace: marketplace.to_string(),
                    plugin: plugin.to_string(),
                    version: version.map(String::from),
                    installed_at: chrono::Utc::now(),
                    dialect: crate::agent::AgentDialect::Native,
                    source_hash: Some(source_hash.to_string()),
                    installed_hash: Some(installed_hash.clone()),
                },
            );
            self.write_agent_tracking(&installed)?;

            Ok(InstalledNativeAgentOutcome {
                name: bundle.name.clone(),
                json_path: json_target,
                forced_overwrite,
                was_idempotent: false,
                source_hash: source_hash.to_string(),
                installed_hash,
            })
        },
    )
}
```

(Note: `_ = was_idempotent;` if Rust warns about the unused variable in the path that returns the idempotent outcome — it's set but only used in the no-op early return.)

- [ ] **Step 4: Run test, verify it passes**

Run: `cargo test -p kiro-market-core --lib install_native_agent_writes_json_with_dialect_native_and_hashes`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/kiro-market-core/src/project.rs
git commit -m "feat(core): KiroProject::install_native_agent (happy path)"
```

---

## Task 11: `install_native_agent` — idempotent reinstall

**Files:**
- Modify: `crates/kiro-market-core/src/project.rs` (test only — implementation already handles this)

- [ ] **Step 1: Write the test**

Append to the `#[cfg(test)] mod tests` block:

```rust
#[test]
fn install_native_agent_idempotent_when_source_hash_matches() {
    let tmp = tempdir().unwrap();
    let project = KiroProject::new(tmp.path()).unwrap();
    let src_dir = tmp.path().join("source");
    fs::create_dir_all(&src_dir).unwrap();
    let src_json = src_dir.join("rev.json");
    fs::write(&src_json, br#"{"name": "rev"}"#).unwrap();
    let bundle = crate::agent::parse_native_kiro_agent_file(&src_json, &src_dir)
        .unwrap();
    let source_hash = crate::hash::hash_artifact(
        &src_dir,
        &[PathBuf::from("rev.json")],
    )
    .unwrap();

    // First install.
    let first = project
        .install_native_agent(&bundle, "m", "p", None, &source_hash, false)
        .unwrap();
    assert!(!first.was_idempotent);

    // Second install with same source_hash — must be a no-op.
    let second = project
        .install_native_agent(&bundle, "m", "p", None, &source_hash, false)
        .unwrap();
    assert!(second.was_idempotent);
    assert!(!second.forced_overwrite);
}
```

- [ ] **Step 2: Run test**

Run: `cargo test -p kiro-market-core --lib install_native_agent_idempotent_when_source_hash_matches`
Expected: PASS (Task 10's implementation already handles this).

- [ ] **Step 3: Commit**

```bash
git add crates/kiro-market-core/src/project.rs
git commit -m "test(core): install_native_agent idempotent reinstall"
```

---

## Task 12: `install_native_agent` — content-changed requires force

**Files:**
- Modify: `crates/kiro-market-core/src/project.rs` (test only)

- [ ] **Step 1: Write the test**

```rust
#[test]
fn install_native_agent_content_changed_requires_force() {
    let tmp = tempdir().unwrap();
    let project = KiroProject::new(tmp.path()).unwrap();
    let src_dir = tmp.path().join("source");
    fs::create_dir_all(&src_dir).unwrap();
    let src_json = src_dir.join("rev.json");
    fs::write(&src_json, br#"{"name": "rev", "v": 1}"#).unwrap();
    let bundle = crate::agent::parse_native_kiro_agent_file(&src_json, &src_dir)
        .unwrap();
    let hash_v1 = crate::hash::hash_artifact(
        &src_dir,
        &[PathBuf::from("rev.json")],
    )
    .unwrap();

    project
        .install_native_agent(&bundle, "m", "p", None, &hash_v1, false)
        .unwrap();

    // Bump the source content.
    fs::write(&src_json, br#"{"name": "rev", "v": 2}"#).unwrap();
    let bundle_v2 =
        crate::agent::parse_native_kiro_agent_file(&src_json, &src_dir)
            .unwrap();
    let hash_v2 = crate::hash::hash_artifact(
        &src_dir,
        &[PathBuf::from("rev.json")],
    )
    .unwrap();
    assert_ne!(hash_v1, hash_v2);

    // Without --force: must fail.
    let err = project
        .install_native_agent(&bundle_v2, "m", "p", None, &hash_v2, false)
        .unwrap_err();
    match err {
        AgentError::ContentChangedRequiresForce { name } => {
            assert_eq!(name, "rev");
        }
        other => panic!("expected ContentChangedRequiresForce, got {other:?}"),
    }

    // With --force: succeeds.
    let outcome = project
        .install_native_agent(&bundle_v2, "m", "p", None, &hash_v2, true)
        .unwrap();
    assert!(outcome.forced_overwrite);
    assert_eq!(outcome.source_hash, hash_v2);
}
```

- [ ] **Step 2: Run test, verify it passes**

Run: `cargo test -p kiro-market-core --lib install_native_agent_content_changed_requires_force`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/kiro-market-core/src/project.rs
git commit -m "test(core): install_native_agent content-changed requires --force"
```

---

## Task 13: `install_native_agent` — cross-plugin name clash

**Files:**
- Modify: `crates/kiro-market-core/src/project.rs` (test only)

- [ ] **Step 1: Write the test**

```rust
#[test]
fn install_native_agent_cross_plugin_name_clash_fails_loudly() {
    let tmp = tempdir().unwrap();
    let project = KiroProject::new(tmp.path()).unwrap();
    let src_dir = tmp.path().join("source");
    fs::create_dir_all(&src_dir).unwrap();
    fs::write(src_dir.join("rev.json"), br#"{"name": "rev"}"#).unwrap();
    let bundle = crate::agent::parse_native_kiro_agent_file(
        &src_dir.join("rev.json"),
        &src_dir,
    )
    .unwrap();
    let h = crate::hash::hash_artifact(
        &src_dir,
        &[PathBuf::from("rev.json")],
    )
    .unwrap();

    // Plugin A installs first.
    project
        .install_native_agent(&bundle, "m", "plugin-a", None, &h, false)
        .unwrap();

    // Plugin B tries to install same agent name — fails.
    let err = project
        .install_native_agent(&bundle, "m", "plugin-b", None, &h, false)
        .unwrap_err();
    match err {
        AgentError::NameClashWithOtherPlugin { name, owner } => {
            assert_eq!(name, "rev");
            assert_eq!(owner, "plugin-a");
        }
        other => panic!("expected NameClashWithOtherPlugin, got {other:?}"),
    }

    // With --force: ownership transfers to plugin-b.
    let outcome = project
        .install_native_agent(&bundle, "m", "plugin-b", None, &h, true)
        .unwrap();
    assert!(outcome.forced_overwrite);

    let tracking = project.load_installed_agents().unwrap();
    let entry = tracking.agents.get("rev").unwrap();
    assert_eq!(entry.plugin, "plugin-b", "ownership must transfer");
}
```

- [ ] **Step 2: Run test, verify it passes**

Run: `cargo test -p kiro-market-core --lib install_native_agent_cross_plugin_name_clash_fails_loudly`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/kiro-market-core/src/project.rs
git commit -m "test(core): install_native_agent cross-plugin name clash + --force transfer"
```

---

## Task 14: `install_native_agent` — orphan-on-disk

**Files:**
- Modify: `crates/kiro-market-core/src/project.rs` (test only)

- [ ] **Step 1: Write the test**

```rust
#[test]
fn install_native_agent_orphan_at_destination_fails_loudly() {
    let tmp = tempdir().unwrap();
    let project = KiroProject::new(tmp.path()).unwrap();
    let src_dir = tmp.path().join("source");
    fs::create_dir_all(&src_dir).unwrap();
    fs::write(src_dir.join("rev.json"), br#"{"name": "rev"}"#).unwrap();
    let bundle = crate::agent::parse_native_kiro_agent_file(
        &src_dir.join("rev.json"),
        &src_dir,
    )
    .unwrap();
    let h = crate::hash::hash_artifact(
        &src_dir,
        &[PathBuf::from("rev.json")],
    )
    .unwrap();

    // Pre-create the destination file with no tracking.
    fs::create_dir_all(project.kiro_dir().join("agents")).unwrap();
    fs::write(
        project.kiro_dir().join("agents").join("rev.json"),
        b"orphan",
    )
    .unwrap();

    // Without --force: fails.
    let err = project
        .install_native_agent(&bundle, "m", "p", None, &h, false)
        .unwrap_err();
    assert!(matches!(err, AgentError::OrphanFileAtDestination { .. }));

    // With --force: succeeds, takes ownership.
    let outcome = project
        .install_native_agent(&bundle, "m", "p", None, &h, true)
        .unwrap();
    assert!(outcome.forced_overwrite);
}
```

- [ ] **Step 2: Run test, verify it passes**

Run: `cargo test -p kiro-market-core --lib install_native_agent_orphan_at_destination_fails_loudly`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/kiro-market-core/src/project.rs
git commit -m "test(core): install_native_agent orphan-at-destination fails + --force"
```

---

## Task 15: `KiroProject::install_native_companions` — happy path

**Files:**
- Modify: `crates/kiro-market-core/src/project.rs`

- [ ] **Step 1: Write the test**

```rust
#[test]
fn install_native_companions_copies_files_and_writes_tracking() {
    let tmp = tempdir().unwrap();
    let project = KiroProject::new(tmp.path()).unwrap();

    // Source companion files under a fake plugin's agents/ scan root.
    let scan_root = tmp.path().join("source-agents");
    let prompts = scan_root.join("prompts");
    fs::create_dir_all(&prompts).unwrap();
    fs::write(prompts.join("a.md"), b"prompt a").unwrap();
    fs::write(prompts.join("b.md"), b"prompt b").unwrap();

    let files = vec![
        crate::agent::DiscoveredNativeFile {
            source: prompts.join("a.md"),
            scan_root: scan_root.clone(),
        },
        crate::agent::DiscoveredNativeFile {
            source: prompts.join("b.md"),
            scan_root: scan_root.clone(),
        },
    ];

    let source_hash = crate::hash::hash_artifact(
        &scan_root,
        &[
            PathBuf::from("prompts/a.md"),
            PathBuf::from("prompts/b.md"),
        ],
    )
    .unwrap();

    let outcome = project
        .install_native_companions(
            &files,
            "marketplace-x",
            "plugin-y",
            Some("0.1.0"),
            &source_hash,
            false,
        )
        .unwrap();

    assert_eq!(outcome.plugin, "plugin-y");
    assert_eq!(outcome.files.len(), 2);
    assert!(!outcome.was_idempotent);

    // Files landed at the right destinations.
    let dest_a = project.kiro_dir().join("agents/prompts/a.md");
    let dest_b = project.kiro_dir().join("agents/prompts/b.md");
    assert!(dest_a.exists());
    assert!(dest_b.exists());
    assert_eq!(fs::read(&dest_a).unwrap(), b"prompt a");

    // Tracking entry exists.
    let tracking = project.load_installed_agents().unwrap();
    let entry = tracking
        .native_companions
        .get("plugin-y")
        .expect("native_companions entry written");
    assert_eq!(entry.plugin, "plugin-y");
    assert_eq!(entry.marketplace, "marketplace-x");
    assert_eq!(entry.files.len(), 2);
    assert_eq!(entry.source_hash, source_hash);
    assert!(entry.installed_hash.starts_with("blake3:"));
}
```

- [ ] **Step 2: Run test, verify it fails**

Run: `cargo test -p kiro-market-core --lib install_native_companions_copies_files_and_writes_tracking`
Expected: FAIL — `no method named 'install_native_companions'`.

- [ ] **Step 3: Implement `install_native_companions`**

Add to `impl KiroProject`:

```rust
/// Install a plugin's native companion file bundle as one atomic unit.
///
/// All files are validated against tracking BEFORE any writes. Cross-plugin
/// path overlap fails with `PathOwnedByOtherPlugin` unless `force == true`.
/// An empty `files` slice is a no-op (no tracking entry written).
pub fn install_native_companions(
    &self,
    files: &[crate::agent::DiscoveredNativeFile],
    marketplace: &str,
    plugin: &str,
    version: Option<&str>,
    source_hash: &str,
    force: bool,
) -> Result<InstalledNativeCompanionsOutcome, AgentError> {
    if files.is_empty() {
        return Ok(InstalledNativeCompanionsOutcome {
            plugin: plugin.to_string(),
            files: Vec::new(),
            forced_overwrite: false,
            was_idempotent: true,
            source_hash: source_hash.to_string(),
            installed_hash: source_hash.to_string(),
        });
    }

    // Compute (relative-from-scan-root) paths for each file.
    let mut entries: Vec<(PathBuf, PathBuf)> = Vec::with_capacity(files.len());
    for f in files {
        let rel = f
            .source
            .strip_prefix(&f.scan_root)
            .map_err(|_| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!(
                        "companion source `{}` is not under scan_root `{}`",
                        f.source.display(),
                        f.scan_root.display()
                    ),
                )
            })?
            .to_path_buf();
        entries.push((f.source.clone(), rel));
    }

    crate::file_lock::with_file_lock(
        &self.agent_tracking_path(),
        || -> Result<InstalledNativeCompanionsOutcome, AgentError> {
            let mut installed = self.load_installed_agents()?;

            // Idempotent / collision check.
            let mut forced_overwrite = false;
            if let Some(existing) = installed.native_companions.get(plugin) {
                if existing.source_hash == source_hash {
                    return Ok(InstalledNativeCompanionsOutcome {
                        plugin: plugin.to_string(),
                        files: existing
                            .files
                            .iter()
                            .map(|p| self.kiro_dir().join("agents").join(p))
                            .collect(),
                        forced_overwrite: false,
                        was_idempotent: true,
                        source_hash: source_hash.to_string(),
                        installed_hash: existing.installed_hash.clone(),
                    });
                } else if !force {
                    return Err(AgentError::ContentChangedRequiresForce {
                        name: format!("{plugin}/companions"),
                    });
                } else {
                    forced_overwrite = true;
                }
            }

            // Cross-plugin path conflict check.
            for (_src, rel) in &entries {
                for (other_plugin, other_meta) in &installed.native_companions {
                    if other_plugin == plugin {
                        continue;
                    }
                    if other_meta.files.contains(rel) {
                        if !force {
                            return Err(AgentError::PathOwnedByOtherPlugin {
                                path: self
                                    .kiro_dir()
                                    .join("agents")
                                    .join(rel),
                                owner: other_plugin.clone(),
                            });
                        }
                        forced_overwrite = true;
                    }
                }
                // Orphan check (file exists, not owned by any plugin).
                let dest = self.kiro_dir().join("agents").join(rel);
                if dest.exists() {
                    let owned_by_self = installed
                        .native_companions
                        .get(plugin)
                        .map(|e| e.files.contains(rel))
                        .unwrap_or(false);
                    let owned_by_other = installed
                        .native_companions
                        .iter()
                        .filter(|(p, _)| p.as_str() != plugin)
                        .any(|(_, m)| m.files.contains(rel));
                    if !owned_by_self && !owned_by_other {
                        if !force {
                            return Err(AgentError::OrphanFileAtDestination {
                                path: dest,
                            });
                        }
                        forced_overwrite = true;
                    }
                }
            }

            // Stage all files in a temp dir under the agents dir.
            let staging = self
                .agents_dir()
                .join(format!(".staging-companions-{plugin}"));
            // Best-effort cleanup of any prior staging dir.
            let _ = std::fs::remove_dir_all(&staging);
            std::fs::create_dir_all(&staging)?;
            for (src, rel) in &entries {
                let staged = staging.join(rel);
                if let Some(parent) = staged.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::copy(src, &staged)?;
            }

            // Promote: rename each staged file to its destination.
            // (Per-file rename rather than directory rename, because
            // companion subdirs may already exist with files from this same
            // plugin's prior install.)
            std::fs::create_dir_all(self.agents_dir())?;
            let mut placed: Vec<PathBuf> = Vec::with_capacity(entries.len());
            for (_src, rel) in &entries {
                let staged = staging.join(rel);
                let dest = self.kiro_dir().join("agents").join(rel);
                if let Some(parent) = dest.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                if dest.exists() {
                    std::fs::remove_file(&dest)?;
                }
                std::fs::rename(&staged, &dest)?;
                placed.push(dest);
            }
            let _ = std::fs::remove_dir_all(&staging);

            // Compute installed_hash over the placed files.
            let installed_paths: Vec<PathBuf> =
                entries.iter().map(|(_, rel)| rel.clone()).collect();
            let installed_hash =
                crate::hash::hash_artifact(&self.agents_dir(), &installed_paths)?;

            // Update tracking. Remove any orphaned ownership in OTHER
            // plugins' entries that conflicted (since --force just stole
            // those paths).
            if force {
                let conflicted_paths: std::collections::HashSet<&PathBuf> =
                    entries.iter().map(|(_, rel)| rel).collect();
                let other_plugins: Vec<String> = installed
                    .native_companions
                    .keys()
                    .filter(|p| p.as_str() != plugin)
                    .cloned()
                    .collect();
                for p in other_plugins {
                    if let Some(meta) = installed.native_companions.get_mut(&p) {
                        meta.files.retain(|f| !conflicted_paths.contains(f));
                    }
                }
                installed
                    .native_companions
                    .retain(|_, meta| !meta.files.is_empty());
            }

            installed.native_companions.insert(
                plugin.to_string(),
                InstalledNativeCompanionsMeta {
                    marketplace: marketplace.to_string(),
                    plugin: plugin.to_string(),
                    version: version.map(String::from),
                    installed_at: chrono::Utc::now(),
                    files: entries.iter().map(|(_, rel)| rel.clone()).collect(),
                    source_hash: source_hash.to_string(),
                    installed_hash: installed_hash.clone(),
                },
            );
            self.write_agent_tracking(&installed)?;

            Ok(InstalledNativeCompanionsOutcome {
                plugin: plugin.to_string(),
                files: placed,
                forced_overwrite,
                was_idempotent: false,
                source_hash: source_hash.to_string(),
                installed_hash,
            })
        },
    )
}
```

(Note: This implementation uses `AgentError::PathOwnedByOtherPlugin` and `AgentError::OrphanFileAtDestination`. If those existing variants don't already exist on `AgentError`, add them in a setup step before this task. They have the shape:

```rust
#[error("path `{path}` is owned by plugin `{owner}`; pass --force to transfer")]
PathOwnedByOtherPlugin { path: PathBuf, owner: String },

#[error(
    "file exists at `{path}` but has no tracking entry; \
     remove it manually or pass --force"
)]
OrphanFileAtDestination { path: PathBuf },
```

Add these alongside Task 6's variants if they're not already there.)

- [ ] **Step 4: Run test, verify it passes**

Run: `cargo test -p kiro-market-core --lib install_native_companions_copies_files_and_writes_tracking`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/kiro-market-core/src/
git commit -m "feat(core): KiroProject::install_native_companions (atomic bundle)"
```

---

## Task 16: `install_native_companions` — idempotent + content-changed + cross-plugin

**Files:**
- Modify: `crates/kiro-market-core/src/project.rs` (tests only)

- [ ] **Step 1: Write three tests**

```rust
#[test]
fn install_native_companions_idempotent_when_source_hash_matches() {
    let tmp = tempdir().unwrap();
    let project = KiroProject::new(tmp.path()).unwrap();
    let scan_root = tmp.path().join("src");
    fs::create_dir_all(scan_root.join("prompts")).unwrap();
    fs::write(scan_root.join("prompts/a.md"), b"a").unwrap();
    let files = vec![crate::agent::DiscoveredNativeFile {
        source: scan_root.join("prompts/a.md"),
        scan_root: scan_root.clone(),
    }];
    let h = crate::hash::hash_artifact(
        &scan_root,
        &[PathBuf::from("prompts/a.md")],
    )
    .unwrap();

    let first = project
        .install_native_companions(&files, "m", "p", None, &h, false)
        .unwrap();
    assert!(!first.was_idempotent);

    let second = project
        .install_native_companions(&files, "m", "p", None, &h, false)
        .unwrap();
    assert!(second.was_idempotent);
}

#[test]
fn install_native_companions_content_changed_requires_force() {
    let tmp = tempdir().unwrap();
    let project = KiroProject::new(tmp.path()).unwrap();
    let scan_root = tmp.path().join("src");
    fs::create_dir_all(scan_root.join("prompts")).unwrap();
    fs::write(scan_root.join("prompts/a.md"), b"v1").unwrap();
    let files = vec![crate::agent::DiscoveredNativeFile {
        source: scan_root.join("prompts/a.md"),
        scan_root: scan_root.clone(),
    }];
    let h_v1 = crate::hash::hash_artifact(
        &scan_root,
        &[PathBuf::from("prompts/a.md")],
    )
    .unwrap();
    project
        .install_native_companions(&files, "m", "p", None, &h_v1, false)
        .unwrap();

    fs::write(scan_root.join("prompts/a.md"), b"v2").unwrap();
    let h_v2 = crate::hash::hash_artifact(
        &scan_root,
        &[PathBuf::from("prompts/a.md")],
    )
    .unwrap();

    let err = project
        .install_native_companions(&files, "m", "p", None, &h_v2, false)
        .unwrap_err();
    assert!(matches!(err, AgentError::ContentChangedRequiresForce { .. }));

    let outcome = project
        .install_native_companions(&files, "m", "p", None, &h_v2, true)
        .unwrap();
    assert!(outcome.forced_overwrite);
}

#[test]
fn install_native_companions_cross_plugin_overlap_fails_loudly() {
    let tmp = tempdir().unwrap();
    let project = KiroProject::new(tmp.path()).unwrap();

    // Plugin A's source.
    let scan_a = tmp.path().join("a-src");
    fs::create_dir_all(scan_a.join("prompts")).unwrap();
    fs::write(scan_a.join("prompts/shared.md"), b"from-a").unwrap();
    let files_a = vec![crate::agent::DiscoveredNativeFile {
        source: scan_a.join("prompts/shared.md"),
        scan_root: scan_a.clone(),
    }];
    let h_a = crate::hash::hash_artifact(
        &scan_a,
        &[PathBuf::from("prompts/shared.md")],
    )
    .unwrap();
    project
        .install_native_companions(&files_a, "m", "plugin-a", None, &h_a, false)
        .unwrap();

    // Plugin B's source — same relative path, different content.
    let scan_b = tmp.path().join("b-src");
    fs::create_dir_all(scan_b.join("prompts")).unwrap();
    fs::write(scan_b.join("prompts/shared.md"), b"from-b").unwrap();
    let files_b = vec![crate::agent::DiscoveredNativeFile {
        source: scan_b.join("prompts/shared.md"),
        scan_root: scan_b.clone(),
    }];
    let h_b = crate::hash::hash_artifact(
        &scan_b,
        &[PathBuf::from("prompts/shared.md")],
    )
    .unwrap();

    let err = project
        .install_native_companions(&files_b, "m", "plugin-b", None, &h_b, false)
        .unwrap_err();
    assert!(matches!(err, AgentError::PathOwnedByOtherPlugin { .. }));

    // With --force: succeeds, tracking transfers ownership.
    let outcome = project
        .install_native_companions(&files_b, "m", "plugin-b", None, &h_b, true)
        .unwrap();
    assert!(outcome.forced_overwrite);

    let tracking = project.load_installed_agents().unwrap();
    // plugin-a should have lost the file (and its entry, since the file was
    // its only one).
    assert!(!tracking.native_companions.contains_key("plugin-a"));
    assert!(tracking.native_companions.contains_key("plugin-b"));
}
```

- [ ] **Step 2: Run tests, verify they pass**

Run: `cargo test -p kiro-market-core --lib install_native_companions_`
Expected: All three tests PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/kiro-market-core/src/project.rs
git commit -m "test(core): install_native_companions idempotent + collision + force"
```

---

## Task 17: MCP gate enforcement at the service layer

**Files:**
- Modify: `crates/kiro-market-core/src/service/mod.rs` (deferred — see Task 18)

This logic belongs in `install_native_kiro_cli_agents_inner` (Task 18). Skip this task as a standalone — the MCP check is implemented inline in Task 18 step 3.

---

## Task 18: `MarketplaceService::install_native_kiro_cli_agents_inner` and dispatch

**Files:**
- Modify: `crates/kiro-market-core/src/service/mod.rs`

This is the longest task in Stage 2 — it wires discovery + parsing + project-layer install into one orchestrator and adds the format-based dispatch.

- [ ] **Step 1: Write the failing test**

Append to the `#[cfg(test)] mod tests` block in `crates/kiro-market-core/src/service/mod.rs` (or `service/browse.rs` if that's where existing service tests live):

```rust
#[test]
fn install_plugin_agents_dispatches_to_native_when_format_kiro_cli() {
    use crate::plugin::PluginFormat;

    // Build a fake plugin dir with format: "kiro-cli" and one agent.
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("plugin.json"),
        br#"{"name": "p", "format": "kiro-cli"}"#,
    )
    .unwrap();
    let agents = tmp.path().join("agents");
    std::fs::create_dir_all(&agents).unwrap();
    std::fs::write(
        agents.join("rev.json"),
        br#"{"name": "rev", "prompt": "..."}"#,
    )
    .unwrap();

    // Minimal MarketplaceService for testing — use the test_support fixture.
    let svc = crate::service::test_support::test_marketplace_service();
    let project_root = tempfile::tempdir().unwrap();
    let project = crate::project::KiroProject::new(project_root.path()).unwrap();

    let ctx = crate::service::browse::resolve_plugin_install_context_from_dir(
        tmp.path(),
    )
    .unwrap();
    assert_eq!(ctx.format, Some(PluginFormat::KiroCli));

    let result = svc.install_plugin_agents(
        &project,
        "marketplace-x",
        &ctx,
        crate::service::AgentInstallOptions { force: false, accept_mcp: false },
    );

    // Native path should produce installed_agents (not skipped).
    assert_eq!(result.installed_agents.len(), 1);
    assert_eq!(result.installed_agents[0].name, "rev");
    assert!(result.installed_companions.is_none());  // no companion files
    assert!(result.failed.is_empty());

    // Tracking entry has dialect Native.
    let tracking = project.load_installed_agents().unwrap();
    assert_eq!(
        tracking.agents.get("rev").unwrap().dialect,
        crate::agent::AgentDialect::Native
    );
}
```

(Note: the exact API for the `MarketplaceService` test fixture (`test_marketplace_service()`) and the signature of `install_plugin_agents` may vary — adapt to what exists in `crates/kiro-market-core/src/service/test_support.rs` and the current `install_plugin_agents` shape per CLAUDE.md's `_impl` pattern guidance.)

- [ ] **Step 2: Run test, verify it fails**

Run: `cargo test -p kiro-market-core --lib install_plugin_agents_dispatches_to_native_when_format_kiro_cli`
Expected: FAIL — either `cannot find function 'install_plugin_agents'` or `installed_agents` field missing on result.

- [ ] **Step 3: Define `AgentInstallOptions` and `InstallAgentsResult` (if not already extended)**

In `crates/kiro-market-core/src/service/mod.rs`, ensure these exist (add or extend):

```rust
#[derive(Debug, Clone, Copy, Default)]
pub struct AgentInstallOptions {
    pub force: bool,
    pub accept_mcp: bool,
}

pub struct InstallAgentsResult {
    /// Per-agent native install outcomes (also covers translated path —
    /// the existing fields on the translated result are absorbed).
    pub installed_agents: Vec<crate::project::InstalledNativeAgentOutcome>,
    /// Per-plugin companion install outcome (native only). None if zero
    /// companion files were discovered for this plugin.
    pub installed_companions: Option<crate::project::InstalledNativeCompanionsOutcome>,
    /// Existing translated-agent path's "skipped" bucket. Empty for native.
    pub skipped: Vec<SkippedAgent>,
    pub failed: Vec<FailedAgent>,
    pub warnings: Vec<DiscoveryWarning>,
}

pub struct FailedAgent {
    pub name: Option<String>,
    pub source_path: std::path::PathBuf,
    pub error: crate::error::AgentError,
}
```

(`SkippedAgent` and `DiscoveryWarning` already exist for the translated path — reuse them.)

- [ ] **Step 4: Implement `install_native_kiro_cli_agents_inner`**

Add to `impl MarketplaceService`:

```rust
fn install_native_kiro_cli_agents_inner(
    &self,
    project: &crate::project::KiroProject,
    marketplace: &str,
    ctx: &crate::service::browse::PluginInstallContext,
    opts: AgentInstallOptions,
) -> InstallAgentsResult {
    let mut result = InstallAgentsResult {
        installed_agents: Vec::new(),
        installed_companions: None,
        skipped: Vec::new(),
        failed: Vec::new(),
        warnings: Vec::new(),
    };

    // Discovery.
    let agent_files = crate::agent::discover_native_kiro_agents_in_dirs(
        &ctx.plugin_dir,
        &ctx.agent_scan_paths,
    );
    let companion_files = crate::agent::discover_native_companion_files(
        &ctx.plugin_dir,
        &ctx.agent_scan_paths,
    );

    // Per-agent install.
    for f in &agent_files {
        let bundle = match crate::agent::parse_native_kiro_agent_file(
            &f.source,
            &f.scan_root,
        ) {
            Ok(b) => b,
            Err(parse_err) => {
                let err = match parse_err {
                    crate::agent::NativeParseFailure::IoError(e) => {
                        crate::error::AgentError::ManifestReadFailed {
                            path: f.source.clone(),
                            source: e,
                        }
                    }
                    crate::agent::NativeParseFailure::InvalidJson(e) => {
                        crate::error::AgentError::NativeManifestParseFailed {
                            path: f.source.clone(),
                            source: e,
                        }
                    }
                    crate::agent::NativeParseFailure::MissingName => {
                        crate::error::AgentError::NativeManifestMissingName {
                            path: f.source.clone(),
                        }
                    }
                    crate::agent::NativeParseFailure::InvalidName(reason) => {
                        crate::error::AgentError::NativeManifestInvalidName {
                            path: f.source.clone(),
                            reason,
                        }
                    }
                };
                result.failed.push(FailedAgent {
                    name: None,
                    source_path: f.source.clone(),
                    error: err,
                });
                continue;
            }
        };

        // MCP gate.
        let has_stdio = bundle
            .mcp_servers
            .values()
            .any(|s| s.is_stdio());
        if has_stdio && !opts.accept_mcp {
            result.failed.push(FailedAgent {
                name: Some(bundle.name.clone()),
                source_path: f.source.clone(),
                error: crate::error::AgentError::McpRequiresAccept {
                    name: bundle.name.clone(),
                },
            });
            continue;
        }

        // Compute source_hash for this single agent JSON.
        let rel = std::path::PathBuf::from(
            f.source.file_name().expect("agent file has a name"),
        );
        let source_hash = match crate::hash::hash_artifact(&f.scan_root, &[rel])
        {
            Ok(h) => h,
            Err(e) => {
                result.failed.push(FailedAgent {
                    name: Some(bundle.name.clone()),
                    source_path: f.source.clone(),
                    error: e.into(),
                });
                continue;
            }
        };

        // Project-layer install.
        match project.install_native_agent(
            &bundle,
            marketplace,
            &ctx.plugin_name,
            ctx.plugin_version.as_deref(),
            &source_hash,
            opts.force,
        ) {
            Ok(outcome) => result.installed_agents.push(outcome),
            Err(err) => result.failed.push(FailedAgent {
                name: Some(bundle.name.clone()),
                source_path: f.source.clone(),
                error: err,
            }),
        }
    }

    // Per-plugin companion install (after all agents — companion bundle
    // installs even if some agents failed).
    if !companion_files.is_empty() {
        // Compute companion source_hash. All companion files share the same
        // scan_root (it was the agent scan_path that found them).
        let scan_root = companion_files[0].scan_root.clone();
        let rel_paths: Vec<std::path::PathBuf> = companion_files
            .iter()
            .map(|f| {
                f.source
                    .strip_prefix(&f.scan_root)
                    .expect("companion source under scan_root")
                    .to_path_buf()
            })
            .collect();

        let source_hash = match crate::hash::hash_artifact(&scan_root, &rel_paths)
        {
            Ok(h) => h,
            Err(e) => {
                result.failed.push(FailedAgent {
                    name: None,
                    source_path: scan_root,
                    error: e.into(),
                });
                return result;
            }
        };

        match project.install_native_companions(
            &companion_files,
            marketplace,
            &ctx.plugin_name,
            ctx.plugin_version.as_deref(),
            &source_hash,
            opts.force,
        ) {
            Ok(outcome) => result.installed_companions = Some(outcome),
            Err(err) => result.failed.push(FailedAgent {
                name: None,
                source_path: scan_root,
                error: err,
            }),
        }
    }

    result
}
```

- [ ] **Step 5: Wire dispatch in `install_plugin_agents`**

Find the existing `pub fn install_plugin_agents` on `MarketplaceService`. Wrap its current body in a `match ctx.format`:

```rust
pub fn install_plugin_agents(
    &self,
    project: &crate::project::KiroProject,
    marketplace: &str,
    ctx: &crate::service::browse::PluginInstallContext,
    opts: AgentInstallOptions,
) -> InstallAgentsResult {
    match ctx.format {
        Some(crate::plugin::PluginFormat::KiroCli) => {
            self.install_native_kiro_cli_agents_inner(project, marketplace, ctx, opts)
        }
        None => {
            // EXISTING translated-agent body, renamed to
            // install_translated_agents_inner. Adapt the existing function
            // signature to take (&KiroProject, &str, &PluginInstallContext,
            // AgentInstallOptions) and return InstallAgentsResult.
            self.install_translated_agents_inner(project, marketplace, ctx, opts)
        }
    }
}
```

(If the existing function signature differs from `(&self, &KiroProject, &str, &PluginInstallContext, AgentInstallOptions)`, adapt the dispatch accordingly. The goal is for both branches to share the call signature so callers don't fork.)

- [ ] **Step 6: Add `AgentError::McpRequiresAccept` if not present**

Check whether `AgentError::McpRequiresAccept { name: String }` (or similar) exists. If not, add it alongside Task 6's variants. Update `SkippedReason::from_plugin_error` and any classifiers per CLAUDE.md.

- [ ] **Step 7: Run test, verify it passes**

Run: `cargo test -p kiro-market-core --lib install_plugin_agents_dispatches_to_native_when_format_kiro_cli`
Expected: PASS.

- [ ] **Step 8: Run all existing service tests**

Run: `cargo test -p kiro-market-core --lib service::`
Expected: All tests pass — translated path is unchanged, native path is added.

- [ ] **Step 9: Commit**

```bash
git add crates/kiro-market-core/src/service/ crates/kiro-market-core/src/error.rs
git commit -m "feat(core): install_native_kiro_cli_agents_inner + format dispatch"
```

---

## Task 19: CLI surfaces native agent + companion outcomes

**Files:**
- Modify: `crates/kiro-market/src/commands/install.rs`

- [ ] **Step 1: Locate the existing presenter for agent install results**

Run: `grep -n "InstallAgentsResult\|install_plugin_agents\|installed.*agent\|skipped.*agent" crates/kiro-market/src/commands/install.rs | head -20`

Note where the existing translated-agent results get rendered (typically a "X agents installed, Y skipped, Z failed" summary).

- [ ] **Step 2: Extend the renderer**

The existing renderer iterates `result.installed` (the old shape). After Task 18, the field is `result.installed_agents`. Update to handle both the per-agent rows AND a per-plugin companion row:

```rust
// In the function that renders InstallAgentsResult:

for outcome in &result.installed_agents {
    let suffix = if outcome.was_idempotent {
        " (unchanged)".dimmed()
    } else if outcome.forced_overwrite {
        " (forced)".yellow()
    } else {
        "".normal()
    };
    println!("  {} agent {}{}", "✓".green(), outcome.name, suffix);
}

if let Some(comp) = &result.installed_companions {
    let suffix = if comp.was_idempotent {
        " (unchanged)".dimmed()
    } else if comp.forced_overwrite {
        " (forced)".yellow()
    } else {
        "".normal()
    };
    println!(
        "  {} {} companion file(s){}",
        "✓".green(),
        comp.files.len(),
        suffix
    );
}

for failed in &result.failed {
    let name = failed.name.as_deref().unwrap_or("(unknown)");
    println!(
        "  {} agent {}: {}",
        "✗".red(),
        name,
        crate::error::error_full_chain(&failed.error)
    );
}
```

(The exact rendering style — colored crate, formatting, prefix glyphs — should match existing conventions in `install.rs`. The names of fields `installed_agents` / `installed_companions` / `failed` are what Task 18 defined; if existing presenter code uses different field names due to backward-compat fixups, reconcile.)

- [ ] **Step 3: Manually verify with a real plugin**

Run (this is a manual smoke test, not an automated test):

```bash
# Clone the starter-kit somewhere.
cd /tmp && git clone --depth 1 https://github.com/dwalleck/kiro-starter-kit.git
# In a fresh project dir:
mkdir -p /tmp/kiro-test-project && cd /tmp/kiro-test-project
# Add the marketplace and install the plugin via the CLI.
cargo run --bin kiro-market -- marketplace add /tmp/kiro-starter-kit --name kiro-starter-kit
cargo run --bin kiro-market -- install kiro-code-reviewer
# Expected output: ~6 "✓ agent X" lines + "✓ N companion file(s)" line.
ls /tmp/kiro-test-project/.kiro/agents/  # Should contain the .json files
ls /tmp/kiro-test-project/.kiro/agents/prompts/  # Should contain the .md files
```

Expected: All six reviewer agents and their prompts land. CLI shows installation summary.

- [ ] **Step 4: Commit**

```bash
git add crates/kiro-market/src/commands/install.rs
git commit -m "feat(cli): render native agent + companion install outcomes"
```

---

## Task 20: End-to-end integration test against starter-kit fixture

**Files:**
- Create: `crates/kiro-market-core/tests/integration_native_install.rs`

- [ ] **Step 1: Decide on the fixture strategy**

Two options:
- **(A) Inline fixture**: build a minimal "fake starter-kit" inside the test using `tempdir()`. Self-contained, no network, no external deps.
- **(B) Vendored fixture**: copy a snapshot of `dwalleck/kiro-starter-kit` into `crates/kiro-market-core/tests/fixtures/kiro-starter-kit/`. More realistic but requires periodic snapshot updates.

For Stage 2, pick **(A)** — the discovery + install logic doesn't care whether the source came from a real repo or a tempdir. The starter-kit's value is shape (agents/ + agents/prompts/ + multiple JSON files), which we can replicate in 30 lines.

- [ ] **Step 2: Write the integration test**

Create `crates/kiro-market-core/tests/integration_native_install.rs`:

```rust
//! End-to-end test of native kiro-cli plugin install, mirroring the
//! `dwalleck/kiro-starter-kit` layout (multiple JSON agents + a `prompts/`
//! companion subdirectory).

use std::fs;
use tempfile::tempdir;

use kiro_market_core::project::KiroProject;
use kiro_market_core::service::{
    browse::resolve_plugin_install_context_from_dir, AgentInstallOptions,
};

#[test]
fn end_to_end_native_plugin_with_agents_and_companions() {
    // Build a fake plugin dir mimicking kiro-starter-kit's layout.
    let plugin_dir = tempdir().unwrap();
    fs::write(
        plugin_dir.path().join("plugin.json"),
        br#"{"name": "fake-reviewers", "format": "kiro-cli"}"#,
    )
    .unwrap();
    let agents = plugin_dir.path().join("agents");
    let prompts = agents.join("prompts");
    fs::create_dir_all(&prompts).unwrap();

    for name in &["reviewer", "simplifier", "tester"] {
        let json = format!(
            r#"{{"name": "{name}", "prompt": "file://./prompts/{name}.md"}}"#
        );
        fs::write(agents.join(format!("{name}.json")), json).unwrap();
        fs::write(prompts.join(format!("{name}.md")), b"prompt body").unwrap();
    }

    // Set up a fresh project.
    let project_root = tempdir().unwrap();
    let project = KiroProject::new(project_root.path()).unwrap();

    let svc = kiro_market_core::service::test_support::test_marketplace_service();
    let ctx = resolve_plugin_install_context_from_dir(plugin_dir.path()).unwrap();

    let result = svc.install_plugin_agents(
        &project,
        "test-marketplace",
        &ctx,
        AgentInstallOptions { force: false, accept_mcp: false },
    );

    assert_eq!(result.installed_agents.len(), 3);
    assert!(result.failed.is_empty());
    assert!(result.installed_companions.is_some());
    assert_eq!(result.installed_companions.as_ref().unwrap().files.len(), 3);

    // Verify destinations.
    for name in &["reviewer", "simplifier", "tester"] {
        assert!(
            project_root
                .path()
                .join(".kiro/agents")
                .join(format!("{name}.json"))
                .exists(),
            "{name}.json must land at .kiro/agents/"
        );
        assert!(
            project_root
                .path()
                .join(".kiro/agents/prompts")
                .join(format!("{name}.md"))
                .exists(),
            "{name}.md must land at .kiro/agents/prompts/"
        );
    }

    // Idempotent reinstall — second call is a no-op for all three agents.
    let again = svc.install_plugin_agents(
        &project,
        "test-marketplace",
        &ctx,
        AgentInstallOptions { force: false, accept_mcp: false },
    );
    assert!(again.installed_agents.iter().all(|o| o.was_idempotent));
    assert!(
        again
            .installed_companions
            .as_ref()
            .unwrap()
            .was_idempotent
    );
}
```

- [ ] **Step 3: Run the integration test**

Run: `cargo test -p kiro-market-core --test integration_native_install`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/kiro-market-core/tests/integration_native_install.rs
git commit -m "test(core): end-to-end native install with agents + companions"
```

---

## Task 21: Final verification — full test suite + clippy + fmt

**Files:** none (verification only)

- [ ] **Step 1: Run the full test suite**

Run: `cargo test --workspace`
Expected: All tests pass.

- [ ] **Step 2: Run clippy**

Run: `cargo clippy --workspace --tests -- -D warnings`
Expected: No warnings. Address any pedantic lints that fire on the new code (e.g. `must_use_candidate`, `option_if_let_else`).

- [ ] **Step 3: Run fmt check**

Run: `cargo fmt --all --check`
Expected: No diff.

- [ ] **Step 4: Commit any cleanup**

```bash
# Only if previous steps required edits:
git add -u
git commit -m "style: address clippy + fmt for native agent import"
```

---

## Out-of-Plan Notes for Implementer

**Why companion bundles are plugin-scoped, not per-agent.** The user explicitly preferred "just copy everything in the agents directory" during brainstorming. Plugin-scoped ownership makes the model simpler: cross-plugin overlap fails, intra-plugin sharing is fine, uninstall is a single tracking entry per plugin. The downside (per-agent uninstall doesn't surgically remove only that agent's referenced companions) is captured in the spec's Out-of-Scope section.

**Why `install_native_agent` doesn't accept `companion_files` in its signature.** Companion install is a separate atomic operation per plugin. Coupling it to per-agent install would require either (a) wasteful re-install of the same companions for each agent, or (b) some "first agent in the batch installs the companions" coordination logic at the project layer that's harder to reason about than two separate methods called in sequence.

**Why `install_native_companions` accepts `&[DiscoveredNativeFile]` instead of `&CompanionBundle`.** Symmetry with the discovery layer's return type. There's no information `CompanionBundle` would carry beyond the slice, so introducing a wrapper type would be ceremony without benefit.

**Why `parse_native_kiro_agent_file` deserializes the JSON twice.** Once into `NativeAgentProjection` (typed access to the fields we need) and once into `serde_json::Value` (to preserve the raw bytes for atomic copy-out). The cost is one extra parse per agent file at install time — negligible compared to the I/O. The alternative — extracting fields by manual `Value` walking — is more code and error-prone.

**Why `_impl` pattern isn't introduced for new Tauri commands here.** Tauri commands for native install are explicitly out of scope for Stage 2 per the spec's "Out of Scope" section. When the Tauri side adopts the feature, follow `install_skills_impl` from `crates/kiro-control-center/src-tauri/src/commands/browse.rs` as the template.

**Why an integration test in `tests/` rather than just inline `mod tests`.** End-to-end tests that exercise the service + project + discovery + parser layers together are clearer when isolated from any single source file's `#[cfg(test)]` block. Mirrors the existing `crates/kiro-market-core/tests/` directory structure if it exists; if not, this is the first one.

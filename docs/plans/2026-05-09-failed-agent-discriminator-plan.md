# `FailedAgent` Tagged-Enum Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Convert `kiro_market_core::service::FailedAgent` from a struct with nullable name + dual-purpose `source_path` into a `#[serde(tag = "kind")]` discriminated enum with three variants (`agent` / `unparseable_agent` / `companion_bundle`). Eliminates the wire-format ambiguity that caused the kiro-control-center misdiagnosis where a directory-shaped `source_path` looked like a discovery bug.

**Architecture:** The new enum follows the `UpdateChangeSignal` precedent (`crates/kiro-market-core/src/service/mod.rs:632-642`) — a Rust enum with `#[serde(tag = "kind", rename_all = "snake_case")]` that specta projects to a discriminated TS union via `bindings.ts`. Construction sites pattern-match on context (does the name exist? is this a per-agent or bundle-level failure?) to choose the correct variant. One small classifier helper extracts conflict paths from typed `AgentError` returns at site 1962, enumerating every variant per CLAUDE.md's classifier rule.

**Tech Stack:** Rust 2024 (edition 1.85.0), `serde` + `serde_json`, `specta` (feature-gated), `thiserror`. TypeScript on the FE consuming `bindings.ts` (auto-generated).

**Spec:** `docs/plans/2026-05-09-failed-agent-discriminator-design.md`

**Behavior change (intentional):** This PR changes the wire-format JSON envelope for items in `result.failed`. Anything observing the raw JSON — log scrapers, browser devtools sessions, external CLI consumers reading `result.failed[i].name` — will see a structural change:

- Before: `{"name": null | string, "source_path": "<path>", "error": "<chain>"}`
- After: tagged enum with three variants discriminated by `kind`. Bundle failures no longer pretend to be agent failures with a directory-shaped `source_path`.

Existing FE only reads `agents.failed.length` (variant-independent), so the in-app UI is unchanged today. The wire-shape change is the *point* of the PR — preserving the old shape would defeat the purpose. Note this in the merge commit message so anyone bisecting wire-format expectations finds the change immediately.

---

## File Structure

| File | Role | Operation |
|------|------|-----------|
| `crates/kiro-market-core/src/service/mod.rs` | Enum definition, classifier helper, all 11 construction sites, wire-format test | Modify |
| `crates/kiro-control-center/src-tauri/src/commands/agents.rs` | One existing pattern-match on `result.failed[0].error` | Modify |
| `crates/kiro-control-center/src/lib/bindings.ts` | Auto-generated type definitions | Regenerate (machine-written) |
| `crates/kiro-control-center/src/lib/plugin-actions.ts` | Add forward-looking comment about discriminator-pushdown for inline-failure UI | Modify |

No new files. The enum, classifier helper, and tests all live in `service/mod.rs` next to the existing wire types per the project's "wire types co-located with service module" convention.

---

## Task 1: Write the failing wire-format test (RED)

This test pins the JSON shape for all three variants. It will fail to compile because the variants don't exist yet — that's the failing-test step.

**Files:**
- Modify: `crates/kiro-market-core/src/service/mod.rs` (append to existing `mod tests` at line ~2864)

- [ ] **Step 1.1: Add the test at the bottom of the existing `mod tests` block**

Locate the `mod tests` module in `crates/kiro-market-core/src/service/mod.rs` (starts around line 2864). Add this test inside it, near the existing `install_warning_*` tests for symmetry:

```rust
    /// Wire-format lock for `FailedAgent`. Pins the three-variant
    /// tagged-enum shape that crosses the FFI boundary via specta.
    /// `#[serde(tag = "kind", rename_all = "snake_case")]` produces
    /// `kind: "agent" | "unparseable_agent" | "companion_bundle"`
    /// in JSON. If a future change accidentally drops the tag,
    /// renames a variant, or adds/removes a field, this test fires
    /// before bindings.ts drift reaches the frontend.
    ///
    /// Asserts exact key-set per variant — the substitute for a
    /// round-trip test (we can't `from_value` because the typed
    /// `AgentError` doesn't `Deserialize`; a one-way `serialize_with`
    /// projects it to a string). Exact-key-set assertions catch the
    /// same drift a round-trip would: any added/removed/renamed field
    /// breaks the test.
    #[test]
    fn failed_agent_serializes_as_three_variant_tagged_enum() {
        use std::collections::BTreeSet;
        use std::path::PathBuf;
        use crate::error::AgentError;
        use crate::validation::PluginName;

        fn key_set(value: &serde_json::Value) -> BTreeSet<String> {
            value
                .as_object()
                .expect("variant serializes to a JSON object")
                .keys()
                .cloned()
                .collect()
        }

        // ---- Agent variant ------------------------------------------------
        let agent = FailedAgent::Agent {
            name: "reviewer".to_owned(),
            source_path: PathBuf::from("/src/reviewer.json"),
            error: AgentError::ContentChangedRequiresForce {
                name: "reviewer".to_owned(),
            },
        };
        let agent_json = serde_json::to_value(&agent).expect("serialize Agent");
        assert_eq!(agent_json["kind"], "agent");
        assert_eq!(agent_json["name"], "reviewer");
        assert_eq!(agent_json["source_path"], "/src/reviewer.json");
        assert!(agent_json["error"].is_string(), "error must serialize as string per FFI contract");
        assert_eq!(
            key_set(&agent_json),
            BTreeSet::from(["error".to_owned(), "kind".to_owned(), "name".to_owned(), "source_path".to_owned()]),
            "Agent variant key set drifted"
        );

        // ---- UnparseableAgent variant -------------------------------------
        let unparseable = FailedAgent::UnparseableAgent {
            source_path: PathBuf::from("/src/broken.json"),
            error: AgentError::NativeManifestParseFailed {
                path: PathBuf::from("/src/broken.json"),
                reason: "expected `,` or `}`".to_owned(),
            },
        };
        let unparseable_json = serde_json::to_value(&unparseable).expect("serialize UnparseableAgent");
        assert_eq!(unparseable_json["kind"], "unparseable_agent");
        assert_eq!(unparseable_json["source_path"], "/src/broken.json");
        assert!(unparseable_json["error"].is_string());
        assert_eq!(
            key_set(&unparseable_json),
            BTreeSet::from(["error".to_owned(), "kind".to_owned(), "source_path".to_owned()]),
            "UnparseableAgent variant key set drifted"
        );

        // ---- CompanionBundle variant: orphan case (length-1 conflicts) ----
        let bundle_orphan = FailedAgent::CompanionBundle {
            plugin: PluginName::new("myplugin").expect("valid plugin name"),
            conflicts: vec![PathBuf::from("prompts/code-reviewer.md")],
            error: AgentError::OrphanFileAtDestination {
                path: PathBuf::from("/dest/prompts/code-reviewer.md"),
            },
        };
        let bundle_orphan_json = serde_json::to_value(&bundle_orphan).expect("serialize CompanionBundle (orphan)");
        assert_eq!(bundle_orphan_json["kind"], "companion_bundle");
        assert_eq!(bundle_orphan_json["plugin"], "myplugin");
        let conflicts = bundle_orphan_json["conflicts"].as_array().expect("conflicts is array");
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0], "prompts/code-reviewer.md");
        assert!(bundle_orphan_json["error"].is_string());
        assert_eq!(
            key_set(&bundle_orphan_json),
            BTreeSet::from(["conflicts".to_owned(), "error".to_owned(), "kind".to_owned(), "plugin".to_owned()]),
            "CompanionBundle variant key set drifted"
        );

        // ---- CompanionBundle variant: pre-enumeration rejection (empty) ---
        // Multi-scan-root rejection fires BEFORE per-file collision
        // classification — `conflicts` is correctly empty, NOT absent.
        // Locking this guarantees serde renders `Vec::new()` as `[]`,
        // not as the field being omitted.
        let bundle_empty = FailedAgent::CompanionBundle {
            plugin: PluginName::new("myplugin").expect("valid plugin name"),
            conflicts: Vec::new(),
            error: AgentError::MultipleScanRootsNotSupported {
                roots: vec![PathBuf::from("agents"), PathBuf::from("other-agents")],
            },
        };
        let bundle_empty_json = serde_json::to_value(&bundle_empty).expect("serialize CompanionBundle (empty)");
        assert_eq!(bundle_empty_json["kind"], "companion_bundle");
        assert_eq!(bundle_empty_json["plugin"], "myplugin");
        assert_eq!(
            bundle_empty_json["conflicts"],
            serde_json::json!([]),
            "empty conflicts must serialize as `[]`, not be omitted"
        );
        assert!(bundle_empty_json["error"].is_string());
        assert_eq!(
            key_set(&bundle_empty_json),
            BTreeSet::from(["conflicts".to_owned(), "error".to_owned(), "kind".to_owned(), "plugin".to_owned()]),
            "CompanionBundle (empty conflicts) variant key set drifted"
        );
    }
```

- [ ] **Step 1.2: Run the test and confirm compile failure**

Run:
```
cargo test -p kiro-market-core --lib service::tests::failed_agent_serializes_as_three_variant_tagged_enum
```

Expected output: compilation errors mentioning `FailedAgent::Agent`, `FailedAgent::UnparseableAgent`, `FailedAgent::CompanionBundle` — each "no variant named X found for enum/struct `FailedAgent`". This is the failing-test signal; do NOT proceed to make it pass yet.

- [ ] **Step 1.3: Do NOT commit yet**

The crate doesn't compile. Tasks 2-5 will make it compile again. Commit only after the entire build is green.

---

## Task 2: Replace `FailedAgent` struct with the tagged enum

This is the type-definition change. After this step the crate has even more compile errors (every construction site breaks). Tasks 3-5 fix them.

**Files:**
- Modify: `crates/kiro-market-core/src/service/mod.rs:644-662`

- [ ] **Step 2.1: Replace the struct with the enum**

Find this block at lines 644-662:

```rust
/// An agent that failed to install, with the typed error.
///
/// `name` is `Some` once parsing has identified the agent; pre-parse
/// failures use `source_path` as the fallback identifier. `error` is the
/// typed [`AgentError`] so frontends can branch on cause without
/// substring-matching the rendered message; a custom `Serialize` impl
/// projects it to the chain string for the wire format.
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct FailedAgent {
    pub name: Option<String>,
    pub source_path: std::path::PathBuf,
    /// Typed error. `Serialize` renders it as a string via
    /// [`crate::error::error_full_chain`] so the wire shape stays string;
    /// in-process consumers can match on the typed variants directly.
    #[serde(serialize_with = "serialize_agent_error")]
    #[cfg_attr(feature = "specta", specta(type = String))]
    pub error: crate::error::AgentError,
}
```

Replace with:

```rust
/// A failure entry for one element of an agent install batch.
///
/// Three-variant tagged enum so the wire format distinguishes per-agent
/// failures (where `name` is known after parse), pre-parse failures
/// (where `source_path` is the only identifier), and bundle-level
/// failures (companion bundles are plugin-scoped, not agent-scoped).
///
/// JSON shape via `#[serde(tag = "kind", rename_all = "snake_case")]`:
/// - `{"kind": "agent", "name": "...", "source_path": "...", "error": "..."}`
/// - `{"kind": "unparseable_agent", "source_path": "...", "error": "..."}`
/// - `{"kind": "companion_bundle", "plugin": "...", "conflicts": [...], "error": "..."}`
///
/// Precedent: `UpdateChangeSignal` (this file) uses the same pattern.
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FailedAgent {
    /// A native or translated agent failed during install. Name is
    /// known because parsing succeeded — callers had a parsed
    /// `AgentDefinition` or `NativeAgentBundle` in scope when the
    /// failure occurred.
    Agent {
        name: String,
        source_path: std::path::PathBuf,
        #[serde(serialize_with = "serialize_agent_error")]
        #[cfg_attr(feature = "specta", specta(type = String))]
        error: crate::error::AgentError,
    },
    /// An agent file failed before parse, so no name is available.
    /// `source_path` is the only identifier the FE can show. The
    /// error variant inside carries the structured parse failure.
    UnparseableAgent {
        source_path: std::path::PathBuf,
        #[serde(serialize_with = "serialize_agent_error")]
        #[cfg_attr(feature = "specta", specta(type = String))]
        error: crate::error::AgentError,
    },
    /// A plugin's companion-file bundle (e.g. `agents/prompts/*.md`)
    /// failed atomically. The bundle is plugin-scoped, not agent-scoped,
    /// so neither `name` nor `source_path` (a per-agent concept) applies.
    ///
    /// `conflicts` enumerates destination paths that conflicted. Today
    /// the engine bails on first conflict, so length is 0 (rejection
    /// before per-file enumeration, e.g. `MultipleScanRootsNotSupported`)
    /// or 1 (orphan / cross-plugin). Forward-compatible with future
    /// "collect all conflicts" engine work without another wire migration.
    CompanionBundle {
        plugin: crate::validation::PluginName,
        conflicts: Vec<std::path::PathBuf>,
        #[serde(serialize_with = "serialize_agent_error")]
        #[cfg_attr(feature = "specta", specta(type = String))]
        error: crate::error::AgentError,
    },
}
```

- [ ] **Step 2.2: Add the classifier helper for site 1962**

Immediately after the enum definition (so it's findable next to the type it serves), add this helper. It must be exhaustive over `AgentError` per CLAUDE.md's classifier rule.

```rust
/// Project an `AgentError` returned by `KiroProject::install_native_companions`
/// into the `conflicts` list on `FailedAgent::CompanionBundle`.
///
/// Two variants carry destination paths that ARE conflicts:
/// `OrphanFileAtDestination` (orphan file with no tracking) and
/// `PathOwnedByOtherPlugin` (cross-plugin conflict). All other variants
/// either carry source paths (which don't belong in `conflicts`) or
/// describe non-conflict failures.
///
/// Exhaustive per CLAUDE.md's "Classifier functions over error enums
/// enumerate every variant" rule. A new `AgentError` variant should
/// force a compile error here, not silently default to empty.
fn companion_conflicts_from_error(err: &crate::error::AgentError) -> Vec<std::path::PathBuf> {
    use crate::error::AgentError;
    match err {
        AgentError::OrphanFileAtDestination { path } => vec![path.clone()],
        AgentError::PathOwnedByOtherPlugin { path, .. } => vec![path.clone()],
        AgentError::AlreadyInstalled { .. }
        | AgentError::NotInstalled { .. }
        | AgentError::ParseFailed { .. }
        | AgentError::NativeManifestParseFailed { .. }
        | AgentError::NativeManifestMissingName { .. }
        | AgentError::NativeManifestInvalidName { .. }
        | AgentError::ManifestReadFailed { .. }
        | AgentError::NameClashWithOtherPlugin { .. }
        | AgentError::ContentChangedRequiresForce { .. }
        | AgentError::MultipleScanRootsNotSupported { .. }
        | AgentError::SourceHardlinked { .. }
        | AgentError::InstallFailed { .. } => Vec::new(),
    }
}
```

- [ ] **Step 2.3: Add classifier unit tests**

The classifier `companion_conflicts_from_error` has 14 match arms (2 produce paths, 12 return empty). The compile-time exhaustiveness gate catches *missing* arms when `AgentError` grows a variant; behavioral tests catch *wrong routing* (e.g. someone moves `OrphanFileAtDestination` into the empty branch by accident). Three tests lock the routing contract.

Append to the existing `mod tests` block in `crates/kiro-market-core/src/service/mod.rs` (near the wire-format test from Task 1.1):

```rust
    /// Classifier routing test: orphan errors produce a length-1
    /// `conflicts` entry containing the destination path.
    #[test]
    fn companion_conflicts_from_error_orphan_returns_destination_path() {
        use std::path::PathBuf;
        use crate::error::AgentError;

        let err = AgentError::OrphanFileAtDestination {
            path: PathBuf::from("/dest/prompts/code-reviewer.md"),
        };
        let conflicts = companion_conflicts_from_error(&err);
        assert_eq!(conflicts, vec![PathBuf::from("/dest/prompts/code-reviewer.md")]);
    }

    /// Classifier routing test: cross-plugin path conflicts also
    /// produce a length-1 `conflicts` entry. The `owner` field is
    /// not consumed by the classifier — it survives in the typed
    /// error inside `FailedAgent::CompanionBundle.error`.
    #[test]
    fn companion_conflicts_from_error_path_owned_by_other_plugin_returns_destination_path() {
        use std::path::PathBuf;
        use crate::error::AgentError;

        let err = AgentError::PathOwnedByOtherPlugin {
            path: PathBuf::from("/dest/prompts/shared.md"),
            owner: "otherplugin".to_owned(),
        };
        let conflicts = companion_conflicts_from_error(&err);
        assert_eq!(conflicts, vec![PathBuf::from("/dest/prompts/shared.md")]);
    }

    /// Classifier routing test: errors that fire BEFORE per-file
    /// enumeration produce an empty conflicts list. `MultipleScanRootsNotSupported`
    /// is the canonical pre-enumeration case (rejection at discovery).
    /// This test pins the empty-conflicts branch so that a future
    /// "default to empty" wildcard regression (which CLAUDE.md
    /// prohibits) is also caught behaviorally.
    #[test]
    fn companion_conflicts_from_error_multi_scan_root_returns_empty() {
        use std::path::PathBuf;
        use crate::error::AgentError;

        let err = AgentError::MultipleScanRootsNotSupported {
            roots: vec![PathBuf::from("agents"), PathBuf::from("other-agents")],
        };
        let conflicts = companion_conflicts_from_error(&err);
        assert!(
            conflicts.is_empty(),
            "MultipleScanRootsNotSupported is a bundle-level rejection that fires \
             BEFORE per-file enumeration; conflicts must be empty, got: {conflicts:?}"
        );
    }
```

These tests will not run yet — the crate doesn't compile (construction sites still broken). They run in Task 6 along with the wire-format test.

- [ ] **Step 2.4: Run cargo check to confirm new compile errors at construction sites**

Run:
```
cargo check -p kiro-market-core
```

Expected: ~11 errors, each saying "expected struct `FailedAgent`, found enum variant" or "no field named `name` on enum `FailedAgent`" at the construction sites. This is normal — Tasks 3-5 fix them. The classifier function and its three new tests should NOT appear in the error list (they only depend on the new enum and `AgentError`, both of which exist).

---

## Task 3: Update construction sites in the translated-agent path

These sites construct `FailedAgent` from the translated (markdown) agent install path.

**Files:**
- Modify: `crates/kiro-market-core/src/service/mod.rs` lines 1648, 1727, 2778

- [ ] **Step 3.1: Site 1648 — translated parse failure → `UnparseableAgent`**

Locate the block around line 1648 (inside `install_translated_agents_inner`). The current code:

```rust
                Err(e) => {
                    // Install-layer variants (AlreadyInstalled/NotInstalled)
                    // shouldn't come from parse_agent_file, but we collect
                    // them as failures rather than crashing the batch.
                    result.failed.push(FailedAgent {
                        name: None,
                        source_path: path.clone(),
                        error: e,
                    });
                    continue;
                }
```

Replace with:

```rust
                Err(e) => {
                    // Install-layer variants (AlreadyInstalled/NotInstalled)
                    // shouldn't come from parse_agent_file, but we collect
                    // them as failures rather than crashing the batch.
                    // Pre-parse failure → `UnparseableAgent` (no name yet).
                    result.failed.push(FailedAgent::UnparseableAgent {
                        source_path: path.clone(),
                        error: e,
                    });
                    continue;
                }
```

- [ ] **Step 3.2: Site 1727 — per-translated-agent install failure → `Agent`**

Locate the block around line 1727. The current code:

```rust
                Err(e) => {
                    let agent_err = match e {
                        Error::Agent(agent_err) => agent_err,
                        other => crate::error::AgentError::InstallFailed {
                            path: path.clone(),
                            source: Box::new(other),
                        },
                    };
                    result.failed.push(FailedAgent {
                        name: Some(def.name),
                        source_path: path.clone(),
                        error: agent_err,
                    });
                }
```

Replace the `result.failed.push(...)` portion with:

```rust
                Err(e) => {
                    let agent_err = match e {
                        Error::Agent(agent_err) => agent_err,
                        other => crate::error::AgentError::InstallFailed {
                            path: path.clone(),
                            source: Box::new(other),
                        },
                    };
                    // Parse succeeded earlier → name is known.
                    result.failed.push(FailedAgent::Agent {
                        name: def.name,
                        source_path: path.clone(),
                        error: agent_err,
                    });
                }
```

(`def.name` was wrapped in `Some(...)` for the old `Option<String>` field; now it's the inner `String` directly.)

- [ ] **Step 3.3: Site 2778 — `required_source_path` helper → `Agent`**

Locate `required_source_path` (around line 2765). The current code:

```rust
fn required_source_path(
    path: &Path,
    plugin_dir: &Path,
    agent_name: String,
) -> Result<crate::validation::RelativePath, FailedAgent> {
    crate::validation::RelativePath::from_path_under(path, plugin_dir).map_err(|e| {
        let path_buf = path.to_path_buf();
        FailedAgent {
            name: Some(agent_name),
            source_path: path_buf.clone(),
            error: crate::error::AgentError::InstallFailed {
                path: path_buf,
                source: Box::new(crate::error::Error::Io(scan_root_invalid_io_err(
                    "discovered agent path",
                    path,
                    plugin_dir,
                ))),
            },
        }
    })
}
```

Replace the `FailedAgent { ... }` literal with:

```rust
        FailedAgent::Agent {
            name: agent_name,
            source_path: path_buf.clone(),
            error: crate::error::AgentError::InstallFailed {
                path: path_buf,
                source: Box::new(crate::error::Error::Io(scan_root_invalid_io_err(
                    "discovered agent path",
                    path,
                    plugin_dir,
                ))),
            },
        }
```

(`Some(agent_name)` → `agent_name`. Same `name` parameter, just unwrapped.)

- [ ] **Step 3.4: Verify intermediate compile state**

Run:
```
cargo check -p kiro-market-core
```

Expected: errors at lines 1812, 1841, 1854, 1895, 1772, 1921, 1939, 1962 only. Translated-path sites no longer error.

---

## Task 4: Update construction sites in the native per-agent path

**Files:**
- Modify: `crates/kiro-market-core/src/service/mod.rs` lines 1812, 1841, 1854, 1895

- [ ] **Step 4.1: Site 1812 — native parse failure → `UnparseableAgent`**

Locate the block around line 1812 (inside `install_one_native_agent`). Current:

```rust
        let bundle = match crate::agent::parse_native_kiro_agent_file(&file.source, &file.scan_root)
        {
            Ok(b) => b,
            Err(parse_err) => {
                result.failed.push(FailedAgent {
                    name: None,
                    source_path: file.source.clone(),
                    error: native_parse_failure_to_agent_error(&file.source, parse_err),
                });
                return;
            }
        };
```

Replace with:

```rust
        let bundle = match crate::agent::parse_native_kiro_agent_file(&file.source, &file.scan_root)
        {
            Ok(b) => b,
            Err(parse_err) => {
                // Parse failed → no `bundle.name` yet; route as UnparseableAgent.
                result.failed.push(FailedAgent::UnparseableAgent {
                    source_path: file.source.clone(),
                    error: native_parse_failure_to_agent_error(&file.source, parse_err),
                });
                return;
            }
        };
```

- [ ] **Step 4.2: Site 1841 — native-manifest-invalid-name → `Agent`**

Locate the block around line 1841. Current:

```rust
        let Some(filename) = file.source.file_name().map(std::path::PathBuf::from) else {
            result.failed.push(FailedAgent {
                name: Some(bundle.name.to_string()),
                source_path: file.source.clone(),
                error: crate::error::AgentError::NativeManifestInvalidName {
                    path: file.source.clone(),
                    reason: "discovered file has no file-name component".to_owned(),
                },
            });
            return;
        };
```

Replace with:

```rust
        let Some(filename) = file.source.file_name().map(std::path::PathBuf::from) else {
            result.failed.push(FailedAgent::Agent {
                name: bundle.name.to_string(),
                source_path: file.source.clone(),
                error: crate::error::AgentError::NativeManifestInvalidName {
                    path: file.source.clone(),
                    reason: "discovered file has no file-name component".to_owned(),
                },
            });
            return;
        };
```

- [ ] **Step 4.3: Site 1854 — native hash failure → `Agent`**

Locate the block around line 1854. Current:

```rust
        let source_hash = match crate::hash::hash_artifact(&file.scan_root, &[filename]) {
            Ok(h) => h,
            Err(e) => {
                result.failed.push(FailedAgent {
                    name: Some(bundle.name.to_string()),
                    source_path: file.source.clone(),
                    error: crate::error::AgentError::InstallFailed {
                        path: file.source.clone(),
                        source: Box::new(e.into()),
                    },
                });
                return;
            }
```

Replace with:

```rust
        let source_hash = match crate::hash::hash_artifact(&file.scan_root, &[filename]) {
            Ok(h) => h,
            Err(e) => {
                result.failed.push(FailedAgent::Agent {
                    name: bundle.name.to_string(),
                    source_path: file.source.clone(),
                    error: crate::error::AgentError::InstallFailed {
                        path: file.source.clone(),
                        source: Box::new(e.into()),
                    },
                });
                return;
            }
```

- [ ] **Step 4.4: Site 1895 — per-native-agent install failure → `Agent`**

Locate the block around line 1895 (the `match project.install_native_agent(...)` arm). Current:

```rust
            Err(err) => result.failed.push(FailedAgent {
                name: Some(bundle.name.to_string()),
                source_path: file.source.clone(),
                error: err,
            }),
```

Replace with:

```rust
            Err(err) => result.failed.push(FailedAgent::Agent {
                name: bundle.name.to_string(),
                source_path: file.source.clone(),
                error: err,
            }),
```

- [ ] **Step 4.5: Verify intermediate compile state**

Run:
```
cargo check -p kiro-market-core
```

Expected: errors only at lines 1772, 1921, 1939, 1962 (the four bundle-level sites left for Task 5).

---

## Task 5: Update construction sites in the companion-bundle path

These four sites all become `FailedAgent::CompanionBundle`. Site 1962 is the only one that uses the classifier helper from Task 2.2.

**Files:**
- Modify: `crates/kiro-market-core/src/service/mod.rs` lines 1772, 1921, 1939, 1962

- [ ] **Step 5.1: Site 1772 — multi-scan-root rejection → `CompanionBundle`**

Locate the block around line 1772 (inside `install_native_kiro_cli_agents_inner`). Current:

```rust
        if let Some(roots) = multiple_companion_scan_roots(&companion_files) {
            result.failed.push(FailedAgent {
                name: None,
                source_path: plugin_dir.to_path_buf(),
                error: crate::error::AgentError::MultipleScanRootsNotSupported { roots },
            });
            return result;
        }
```

Replace with:

```rust
        if let Some(roots) = multiple_companion_scan_roots(&companion_files) {
            // Bundle-level rejection BEFORE per-file enumeration —
            // `conflicts` is empty because we never got to the
            // collision-classification step.
            result.failed.push(FailedAgent::CompanionBundle {
                plugin: ctx.plugin.clone(),
                conflicts: Vec::new(),
                error: crate::error::AgentError::MultipleScanRootsNotSupported { roots },
            });
            return result;
        }
```

(`ctx.plugin` is `&PluginName`; clone to get an owned `PluginName`.)

- [ ] **Step 5.2: Site 1921 — strip-prefix discovery violation → `CompanionBundle`**

Locate the block around line 1921 (inside `install_native_companions_for_plugin`). Current:

```rust
            let Ok(rel) = f.source.strip_prefix(&f.scan_root) else {
                result.failed.push(FailedAgent {
                    name: None,
                    source_path: f.source.clone(),
                    error: crate::error::AgentError::InstallFailed {
                        path: f.source.clone(),
                        source: Box::new(crate::error::Error::Io(std::io::Error::other(
                            "discovered companion not under its declared scan_root",
                        ))),
                    },
                });
                return;
            };
```

Replace with:

```rust
            let Ok(rel) = f.source.strip_prefix(&f.scan_root) else {
                // Discovery contract violation (not a destination conflict).
                result.failed.push(FailedAgent::CompanionBundle {
                    plugin: ctx.plugin.clone(),
                    conflicts: Vec::new(),
                    error: crate::error::AgentError::InstallFailed {
                        path: f.source.clone(),
                        source: Box::new(crate::error::Error::Io(std::io::Error::other(
                            "discovered companion not under its declared scan_root",
                        ))),
                    },
                });
                return;
            };
```

- [ ] **Step 5.3: Site 1939 — companion bundle hash failure → `CompanionBundle`**

Locate the block around line 1939. Current:

```rust
        let source_hash = match crate::hash::hash_artifact(&scan_root, &rel_paths) {
            Ok(h) => h,
            Err(e) => {
                result.failed.push(FailedAgent {
                    name: None,
                    source_path: scan_root,
                    error: crate::error::AgentError::InstallFailed {
                        path: plugin_dir.to_path_buf(),
                        source: Box::new(e.into()),
                    },
                });
                return;
            }
        };
```

Replace with:

```rust
        let source_hash = match crate::hash::hash_artifact(&scan_root, &rel_paths) {
            Ok(h) => h,
            Err(e) => {
                // Bundle-level pre-promotion failure — no destination
                // conflicts because we never reached collision classification.
                result.failed.push(FailedAgent::CompanionBundle {
                    plugin: ctx.plugin.clone(),
                    conflicts: Vec::new(),
                    error: crate::error::AgentError::InstallFailed {
                        path: plugin_dir.to_path_buf(),
                        source: Box::new(e.into()),
                    },
                });
                return;
            }
        };
```

- [ ] **Step 5.4: Site 1962 — companion bundle install failure → `CompanionBundle` (uses classifier)**

Locate the block around line 1962. Current:

```rust
        }) {
            Ok(outcome) => result.installed_companions = Some(outcome),
            Err(err) => result.failed.push(FailedAgent {
                name: None,
                source_path: scan_root,
                error: err,
            }),
        }
```

Replace with:

```rust
        }) {
            Ok(outcome) => result.installed_companions = Some(outcome),
            Err(err) => {
                // The classifier extracts conflict paths from the typed
                // error (Orphan / PathOwnedByOtherPlugin carry destination
                // paths). All other variants → empty `conflicts`.
                let conflicts = companion_conflicts_from_error(&err);
                result.failed.push(FailedAgent::CompanionBundle {
                    plugin: ctx.plugin.clone(),
                    conflicts,
                    error: err,
                });
            }
        }
```

- [ ] **Step 5.5: Verify the full crate compiles**

Run:
```
cargo check -p kiro-market-core
```

Expected: 0 errors. The crate compiles. The wire-format test from Task 1 may now pass — check next.

---

## Task 6: Run the wire-format test (GREEN)

- [ ] **Step 6.1: Run the wire-format test and the three classifier tests**

The four new tests added by this plan are filterable by substring: the wire-format test is one match (`failed_agent_serializes_as_three_variant_tagged_enum`) and the three classifier tests share a prefix (`companion_conflicts_from_error_*`). Both filters in one cargo invocation:

```
cargo test -p kiro-market-core --lib failed_agent_serializes_as_three_variant_tagged_enum companion_conflicts_from_error
```

Expected: 4 passed. If the wire-format test fails, the assertion message indicates which JSON shape or key set drifted — re-read Task 2.1 carefully. If a classifier test fails, the routing in Task 2.2's helper is wrong — re-read which `AgentError` variants populate `conflicts` (only `OrphanFileAtDestination` and `PathOwnedByOtherPlugin`).

- [ ] **Step 6.2: Run the full kiro-market-core test suite**

Run:
```
cargo test -p kiro-market-core
```

Expected: most tests pass; some existing tests asserting old `FailedAgent { name, source_path, error }` shape will fail to compile or fail at runtime. Task 7 fixes them.

---

## Task 7: Fix existing test pattern-matches surfaced by the compiler

Compile errors in the test module will be of two kinds:
1. Construction patterns: `FailedAgent { name: Some(...), source_path: ..., error: ... }` — replace with `FailedAgent::Agent { name: ..., source_path: ..., error: ... }`.
2. Field access patterns: `result.failed[0].name` / `result.failed[0].source_path` / `result.failed[0].error` — replace with a `match` that destructures the variant.

**Files:**
- Modify: `crates/kiro-market-core/src/service/mod.rs` (test module)
- Modify: `crates/kiro-control-center/src-tauri/src/commands/agents.rs:421-427`

- [ ] **Step 7.1: Run the workspace tests to surface every broken test**

Run:
```
cargo test --workspace --no-run 2>&1 | head -200
```

Expected: a list of compile errors in test files. Each one points at the specific access pattern that needs updating.

- [ ] **Step 7.2: Apply the variant-aware match pattern to each broken test**

For every error like:

```rust
assert_eq!(result.failed[0].name, Some("reviewer".to_owned()));
```

Replace with:

```rust
match &result.failed[0] {
    FailedAgent::Agent { name, .. } => assert_eq!(name, "reviewer"),
    other => panic!("expected Agent variant, got {other:?}"),
}
```

For an error-only assertion like:

```rust
assert!(matches!(
    &result.failed[0].error,
    AgentError::ContentChangedRequiresForce { name } if name == "reviewer"
));
```

Replace with:

```rust
match &result.failed[0] {
    FailedAgent::Agent { error: AgentError::ContentChangedRequiresForce { name }, .. } if name == "reviewer" => {}
    other => panic!("expected Agent { error: ContentChangedRequiresForce, .. }, got {other:?}"),
}
```

- [ ] **Step 7.3: Specifically fix `commands/agents.rs:421-427`**

Locate `crates/kiro-control-center/src-tauri/src/commands/agents.rs:421-427`. Current:

```rust
        assert!(
            matches!(
                &result.failed[0].error,
                kiro_market_core::error::AgentError::ContentChangedRequiresForce { name }
                    if name == "reviewer"
            ),
            "wrong error variant: {:?}",
            result.failed[0].error
        );
```

Replace with:

```rust
        match &result.failed[0] {
            kiro_market_core::service::FailedAgent::Agent {
                error: kiro_market_core::error::AgentError::ContentChangedRequiresForce { name },
                ..
            } if name == "reviewer" => {}
            other => panic!(
                "expected FailedAgent::Agent {{ error: ContentChangedRequiresForce {{ name: \"reviewer\" }}, .. }}, \
                 got {other:?}"
            ),
        }
```

The downstream JSON-shape assertion at lines 429-444 still works as-is — `serde_json::to_value(&result)` produces a value whose `/failed/0/error` pointer resolves to a string (per the new Agent variant's `error` field having the same `serialize_with` attribute).

However, that test's assertion on `/failed/0/error` only finds the field if the JSON shape includes it at that path. Since the new tagged enum places `error` at `result.failed[0].error` regardless of variant, the existing JSON pointer continues to work. Confirm by re-running the test after the variant-aware match update.

- [ ] **Step 7.4: Iterate until `cargo test --workspace --no-run` is clean**

Run:
```
cargo test --workspace --no-run
```

If errors remain, repeat Step 7.2 for each. The compiler tells you exactly which file:line needs the variant match.

- [ ] **Step 7.5: Run the full test suite**

Run:
```
cargo test --workspace
```

Expected: all tests pass. If a runtime assertion fails (rare — usually caught by the compile errors in 7.1), inspect the panic message and update the test.

---

## Task 8: Lint, format, commit Rust changes

- [ ] **Step 8.1: Format**

Run:
```
cargo fmt --all
```

- [ ] **Step 8.2: Clippy**

Run:
```
cargo clippy --workspace --tests -- -D warnings
```

Expected: 0 warnings. If clippy flags the new code, fix it (likely candidates: unnecessary `clone()`, `Vec::new()` vs `vec![]` style — match the surrounding code).

- [ ] **Step 8.3: Confirm format check passes**

Run:
```
cargo fmt --all --check
```

Expected: 0 output (silent success).

- [ ] **Step 8.4: Stage and commit**

```bash
git add crates/kiro-market-core/src/service/mod.rs \
        crates/kiro-control-center/src-tauri/src/commands/agents.rs
git commit -m "$(cat <<'EOF'
refactor(service): make FailedAgent a tagged enum

Convert FailedAgent from a struct with nullable name + dual-purpose
source_path into a #[serde(tag = "kind")] enum with three variants:
agent, unparseable_agent, companion_bundle. Eliminates the wire-format
ambiguity that surfaced as a "directory mis-installed as a file"
misdiagnosis when companion-bundle failures used scan_root as
source_path.

Adds companion_conflicts_from_error classifier (exhaustive over
AgentError per CLAUDE.md classifier rule) to project orphan and
cross-plugin path conflicts into the new conflicts: Vec<PathBuf>
field. Other AgentError variants → empty conflicts.

11 construction sites updated; one downstream test pattern-match in
commands/agents.rs adjusted for the new shape. FE consumers only read
.length and need no changes today; bindings.ts regenerates in the
follow-up commit.

See docs/plans/2026-05-09-failed-agent-discriminator-design.md.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 9: Regenerate bindings.ts

- [ ] **Step 9.1: Run the bindings regen test**

Run:
```
cargo test -p kiro-control-center --lib -- --ignored
```

This is the recipe documented in CLAUDE.md. It writes the new `bindings.ts` based on the updated specta-derived types.

- [ ] **Step 9.2: Inspect the diff to confirm the new shape**

Run:
```
git diff crates/kiro-control-center/src/lib/bindings.ts
```

Expected pattern in the diff for `FailedAgent`:

```diff
-export type FailedAgent_Deserialize = {
-	name: string | null,
-	source_path: string,
-	error: string,
-};
+export type FailedAgent_Deserialize =
+	| { kind: "agent"; name: string; source_path: string; error: string }
+	| { kind: "unparseable_agent"; source_path: string; error: string }
+	| { kind: "companion_bundle"; plugin: string; conflicts: string[]; error: string };
```

(Specta may format the union slightly differently — multi-line, trailing commas, or comments preserved. The key check: each of the three `kind` discriminator strings appears, and the field sets per variant match the design doc.)

If the diff doesn't match (e.g. the discriminator is missing, or the shape is still a flat struct), the most likely cause is the `#[serde(tag = "kind", rename_all = "snake_case")]` attribute being dropped during the Task 2.1 edit — re-read the enum block in `service/mod.rs` and confirm the attribute is present.

- [ ] **Step 9.3: Run TypeScript checks**

Run from `crates/kiro-control-center/`:
```
npm run check
```

Expected: 0 errors. The existing FE consumers only read `.length` (`format.ts:232,234,246`, `plugin-actions.ts:146`), which is variant-independent. If TS errors appear, they indicate a consumer the grep missed — handle each by adding a discriminator-aware switch (see Task 10 for the pattern).

---

## Task 10: Add forward-looking comment to plugin-actions.ts

The existing `console.error` diagnostic logger at `plugin-actions.ts:130-148` documents that inline-failure UI is follow-on work (F3 in the spec). The new wire shape is what that UI should consume. Add a comment near it that pre-positions the discriminator-pushdown pattern for whoever picks F3 up.

**Files:**
- Modify: `crates/kiro-control-center/src/lib/plugin-actions.ts` near line 148 (after the `console.error` block closes)

- [ ] **Step 10.1: Read the existing comment block to preserve its tone**

Run:
```
cat crates/kiro-control-center/src/lib/plugin-actions.ts | sed -n '125,150p'
```

- [ ] **Step 10.2: Insert the forward-looking comment**

After the closing `}` of the `if (anyFailed) { console.error(...) }` block (around line 149), add:

```typescript
      // Per design 2026-05-09-failed-agent-discriminator-design.md (F3):
      // when the inline-failure UI ships, render `result.data.agents.failed`
      // by switching on the discriminator with an exhaustiveness guard:
      //
      //   switch (entry.kind) {
      //     case "agent": return renderAgent(entry.name, entry.source_path, entry.error);
      //     case "unparseable_agent": return renderUnparseable(entry.source_path, entry.error);
      //     case "companion_bundle": return renderBundle(entry.plugin, entry.conflicts, entry.error);
      //     default: { const _exhaustive: never = entry; throw new Error(`unhandled ${JSON.stringify(_exhaustive)}`); }
      //   }
      //
      // Pair the runtime switch with a value-position assert per CLAUDE.md
      // discriminator-pushdown discipline (see _PLUGIN_ACTION_VALUES + _AssertPluginActionExhaustive).
```

- [ ] **Step 10.3: Run TypeScript checks again**

Run from `crates/kiro-control-center/`:
```
npm run check
```

Expected: 0 errors. (The comment doesn't change runtime behavior.)

---

## Task 11: Final verification gates

Per CLAUDE.md "Pre-commit": run all four CI-enforced gates plus the FE checks before committing.

- [ ] **Step 11.1: Format check**

```
cargo fmt --all --check
```

Expected: 0 output.

- [ ] **Step 11.2: Workspace tests**

```
cargo test --workspace
```

Expected: all green.

- [ ] **Step 11.3: Clippy**

```
cargo clippy --workspace --tests -- -D warnings
```

Expected: 0 warnings.

- [ ] **Step 11.4: TypeScript check**

From `crates/kiro-control-center/`:
```
npm run check
```

Expected: 0 errors.

- [ ] **Step 11.5: Vitest**

From `crates/kiro-control-center/`:
```
npm run test:unit
```

Expected: all green.

---

## Task 12: Commit FE + bindings changes

- [ ] **Step 12.1: Stage and commit**

```bash
git add crates/kiro-control-center/src/lib/bindings.ts \
        crates/kiro-control-center/src/lib/plugin-actions.ts
git commit -m "$(cat <<'EOF'
chore(bindings): regenerate FailedAgent for tagged-enum wire shape

Regenerates bindings.ts after the kiro-market-core enum conversion.
The TS type for FailedAgent goes from a flat struct with nullable
name to a discriminated union of three variants. Existing FE
consumers only read .length and need no changes today.

Also adds a forward-looking comment in plugin-actions.ts pointing
at F3 (inline-failure UI) that documents the discriminator-pushdown
switch shape future consumers should adopt, per CLAUDE.md.

See docs/plans/2026-05-09-failed-agent-discriminator-design.md.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

- [ ] **Step 12.2: Final status check**

```
git status --short
```

Expected: only your pre-existing in-progress modifications appear (the ones unrelated to this PR). The `FailedAgent` work is fully committed.

---

## Done

The wire-format change is complete. Verify with one round-trip sanity check:

```
cargo run -p kiro-market -- list-marketplaces
```

(or any other command that exercises the install path) — if the CLI runs without panicking, the binary build is healthy. The actual install-with-orphan scenario is covered by the test from Task 1.

Spec follow-on items (F1-F5) are documented in `docs/plans/2026-05-09-failed-agent-discriminator-design.md` for future PRs.

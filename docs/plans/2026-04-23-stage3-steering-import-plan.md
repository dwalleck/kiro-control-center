# Stage 3: Steering Import Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

> **⚠️ READ BEFORE EXECUTING — REVISIT AFTER STAGES 1 AND 2.**
>
> This plan was written before Stages 1 and 2 landed. Before starting Stage 3 implementation:
>
> 1. **Confirm `kiro_market_core::hash::hash_artifact` exists with the expected signature** (Stage 1). All steering install logic depends on it.
> 2. **Confirm `InstalledNativeCompanionsMeta` and the `KiroProject::install_native_companions` pattern** (Stage 2). Steering tracking and install methods follow the same shape — if Stage 2 diverged from the design, mirror those divergences here for consistency.
> 3. **Re-check the `PluginInstallContext` shape** (Stage 2 added `format`). Stage 3 adds `steering_scan_paths`. If Stage 2 restructured the context (e.g. moved fields into a sub-struct), update Task 4 below.
> 4. **Re-read the design doc.** `docs/plans/2026-04-23-kiro-cli-native-plugin-import-design.md` is the source of truth — Stages 1 and 2 implementation may have surfaced spec gaps that triggered amendments. Cross-reference.
> 5. **Audit `SkippedReason::from_plugin_error` and any error classifiers** for `SteeringError` arms after Stage 2's pattern. Per CLAUDE.md, no `_ =>` defaults.

**Goal:** Add steering files as a third install target alongside skills and agents. Plugins ship `.md` files in a `steering/` directory; they land at `.kiro/steering/<filename>` with content-hash-aware tracking and the same fail-loudly collision policy.

**Architecture:** New `steering/` module mirrors the `agent/` layout's discovery shape. New `KiroProject::install_steering_file` method follows `install_native_agent`'s atomic-staging + collision-detection pattern. New `MarketplaceService::install_plugin_steering` orchestrator. New tracking file `installed-steering.json` keyed by relative-path-under-`.kiro/steering/`.

**Tech Stack:** Rust (edition 2024), serde / serde_json, blake3 + hex (via Stage 1's `hash` module), thiserror, existing `validation` + `with_file_lock` primitives.

**Spec reference:** `docs/plans/2026-04-23-kiro-cli-native-plugin-import-design.md` § "Manifest Schema" (steering field), § "Layer Contracts" (SteeringSource), § "Tracking Schema and Content Hashes" (`installed-steering.json`), § "Implementation Phasing — Stage 3".

---

## File Structure

**New files:**
- `crates/kiro-market-core/src/steering/mod.rs`
- `crates/kiro-market-core/src/steering/discover.rs` — `discover_steering_files_in_dirs`
- `crates/kiro-market-core/src/steering/types.rs` — `SteeringError`, `InstalledSteeringOutcome`, `InstallSteeringResult`, `SteeringInstallOptions`, `FailedSteeringFile`

**Modified files:**
- `crates/kiro-market-core/src/lib.rs` — `pub mod steering;`, `DEFAULT_STEERING_PATHS`
- `crates/kiro-market-core/src/plugin.rs` — `steering: Vec<String>` on `PluginManifest`
- `crates/kiro-market-core/src/error.rs` — `Error::Steering` arm
- `crates/kiro-market-core/src/project.rs` — `InstalledSteering`, `InstalledSteeringMeta`, `INSTALLED_STEERING_FILE`, `KiroProject::install_steering_file`, load/save helpers
- `crates/kiro-market-core/src/service/browse.rs` — `PluginInstallContext::steering_scan_paths`, resolver reads `manifest.steering`
- `crates/kiro-market-core/src/service/mod.rs` — `MarketplaceService::install_plugin_steering`
- `crates/kiro-market/src/commands/install.rs` — call `install_plugin_steering`, render results

---

## Task 1: Add `steering` field to `PluginManifest` and `DEFAULT_STEERING_PATHS`

**Files:**
- Modify: `crates/kiro-market-core/src/plugin.rs`
- Modify: `crates/kiro-market-core/src/lib.rs`

- [ ] **Step 1: Write the failing tests**

Append to `#[cfg(test)] mod tests` in `crates/kiro-market-core/src/plugin.rs`:

```rust
#[test]
fn manifest_parses_steering_paths() {
    let json = br#"{"name": "p", "steering": ["./guidance/", "./extras/"]}"#;
    let manifest = PluginManifest::from_json(json).unwrap();
    assert_eq!(manifest.steering, vec!["./guidance/", "./extras/"]);
}

#[test]
fn manifest_steering_absent_is_empty_vec() {
    let json = br#"{"name": "p"}"#;
    let manifest = PluginManifest::from_json(json).unwrap();
    assert!(manifest.steering.is_empty());
}
```

- [ ] **Step 2: Run tests, verify they fail**

Run: `cargo test -p kiro-market-core --lib plugin::tests::manifest_parses_steering_paths`
Expected: FAIL — `no field 'steering' on type 'PluginManifest'`.

- [ ] **Step 3: Add the field**

In `crates/kiro-market-core/src/plugin.rs`, find `pub struct PluginManifest`. Add the field after `format`:

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
    #[serde(default)]
    pub format: Option<PluginFormat>,

    /// Optional list of directories (relative to plugin root) to scan for
    /// steering files. Empty means use the default
    /// (`crate::DEFAULT_STEERING_PATHS`).
    #[serde(default)]
    pub steering: Vec<String>,
}
```

- [ ] **Step 4: Add `DEFAULT_STEERING_PATHS` to `lib.rs`**

In `crates/kiro-market-core/src/lib.rs`, find `pub const DEFAULT_AGENT_PATHS:` (or similar). Add a sibling:

```rust
/// Default scan paths for steering files when a plugin manifest declares
/// `steering: []` or omits the field entirely.
pub const DEFAULT_STEERING_PATHS: &[&str] = &["./steering/"];
```

- [ ] **Step 5: Run tests, verify they pass**

Run: `cargo test -p kiro-market-core --lib plugin::tests::manifest_parses_steering`
Expected: Both PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/kiro-market-core/src/plugin.rs crates/kiro-market-core/src/lib.rs
git commit -m "feat(core): add steering field to PluginManifest + DEFAULT_STEERING_PATHS"
```

---

## Task 2: Create `steering/` module skeleton with `SteeringError`

**Files:**
- Create: `crates/kiro-market-core/src/steering/mod.rs`
- Create: `crates/kiro-market-core/src/steering/types.rs`
- Modify: `crates/kiro-market-core/src/lib.rs`
- Modify: `crates/kiro-market-core/src/error.rs`

- [ ] **Step 1: Create `steering/types.rs`**

```rust
//! Public types for steering install. Mirrors the shape of the agent
//! types module: error enum + result/outcome structs.

use std::io;
use std::path::PathBuf;

use thiserror::Error;

/// Errors that can occur during steering install.
#[derive(Debug, Error)]
pub enum SteeringError {
    #[error("steering source `{path}` could not be read")]
    SourceReadFailed {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error(
        "steering file `{rel}` would clobber a file owned by plugin `{owner}`; \
         pass --force to transfer ownership"
    )]
    PathOwnedByOtherPlugin { rel: PathBuf, owner: String },

    #[error(
        "steering file exists at `{path}` but has no tracking entry; \
         remove it manually or pass --force"
    )]
    OrphanFileAtDestination { path: PathBuf },

    #[error(
        "steering file `{rel}` content has changed since last install; \
         pass --force to overwrite"
    )]
    ContentChangedRequiresForce { rel: PathBuf },

    #[error("steering tracking I/O failed at `{path}`")]
    TrackingIoFailed {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error(transparent)]
    Hash(#[from] crate::hash::HashError),

    #[error(transparent)]
    Io(#[from] io::Error),

    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

/// Per-call outcome of `KiroProject::install_steering_file`.
#[derive(Debug, Clone)]
pub struct InstalledSteeringOutcome {
    pub source: PathBuf,
    pub destination: PathBuf,
    /// True if `--force` overwrote a tracked path (orphan or other plugin).
    pub forced_overwrite: bool,
    /// True if the install was a no-op because tracking matched
    /// `source_hash` exactly (idempotent reinstall).
    pub was_idempotent: bool,
    pub source_hash: String,
    pub installed_hash: String,
}

/// Per-file failure entry in a steering install batch.
#[derive(Debug)]
pub struct FailedSteeringFile {
    pub source: PathBuf,
    pub error: SteeringError,
}

/// Aggregate result of `MarketplaceService::install_plugin_steering`.
#[derive(Debug)]
pub struct InstallSteeringResult {
    pub installed: Vec<InstalledSteeringOutcome>,
    pub failed: Vec<FailedSteeringFile>,
    pub warnings: Vec<crate::service::DiscoveryWarning>,
}

/// Options for `MarketplaceService::install_plugin_steering`.
#[derive(Debug, Clone, Copy, Default)]
pub struct SteeringInstallOptions {
    pub force: bool,
}
```

(Note: `crate::service::DiscoveryWarning` already exists per Stage 2's plan — if it doesn't, replace with whatever warning type the service layer uses for discovery issues.)

- [ ] **Step 2: Create `steering/mod.rs`**

```rust
//! Steering import: discover steering markdown files in a plugin and
//! install them into `.kiro/steering/` with content-hash-aware tracking.
//!
//! Steering is a peer install target alongside skills and agents — see
//! `docs/plans/2026-04-23-kiro-cli-native-plugin-import-design.md` for
//! the full design rationale.

pub mod discover;
pub mod types;

pub use discover::discover_steering_files_in_dirs;
pub use types::{
    FailedSteeringFile, InstallSteeringResult, InstalledSteeringOutcome,
    SteeringError, SteeringInstallOptions,
};
```

- [ ] **Step 3: Create empty `steering/discover.rs` placeholder**

```rust
//! Discovery for steering files. See `discover_steering_files_in_dirs`.

// Implementation lands in Task 3.
```

- [ ] **Step 4: Wire into `lib.rs`**

In `crates/kiro-market-core/src/lib.rs`:

```rust
pub mod steering;
```

- [ ] **Step 5: Wire `SteeringError` into top-level `Error`**

In `crates/kiro-market-core/src/error.rs`, add to `pub enum Error`:

```rust
#[error(transparent)]
Steering(#[from] crate::steering::SteeringError),
```

- [ ] **Step 6: Verify compilation**

Run: `cargo build -p kiro-market-core`
Expected: Build succeeds. The new module compiles even though `discover.rs` is empty.

- [ ] **Step 7: Commit**

```bash
git add crates/kiro-market-core/src/steering/ crates/kiro-market-core/src/lib.rs crates/kiro-market-core/src/error.rs
git commit -m "feat(core): scaffold steering module + SteeringError"
```

---

## Task 3: Implement `discover_steering_files_in_dirs`

**Files:**
- Modify: `crates/kiro-market-core/src/steering/discover.rs`

- [ ] **Step 1: Write the failing tests**

Replace the contents of `crates/kiro-market-core/src/steering/discover.rs` with:

```rust
//! Discovery for steering files.

use std::fs;
use std::io;
use std::path::Path;

use tracing::{debug, warn};

use crate::agent::DiscoveredNativeFile;

/// Filenames excluded from steering discovery (case-insensitive).
const EXCLUDED_FILENAMES: &[&str] = &["README.md", "CONTRIBUTING.md", "CHANGELOG.md"];

/// Find steering markdown candidates: `.md` files at the root of each scan
/// path. Mirrors `discover_native_kiro_agents_in_dirs` security model:
/// validates each scan path, refuses symlinks, excludes
/// README/CONTRIBUTING/CHANGELOG, non-recursive at the scan-path level.
#[must_use]
pub fn discover_steering_files_in_dirs(
    plugin_dir: &Path,
    scan_paths: &[String],
) -> Vec<DiscoveredNativeFile> {
    let mut out = Vec::new();
    for rel in scan_paths {
        if let Err(e) = crate::validation::validate_relative_path(rel) {
            warn!(
                path = %rel,
                error = %e,
                "skipping steering scan path that fails validation"
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
                    "failed to read steering scan directory; skipping"
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
                        "failed to stat steering candidate; skipping"
                    );
                    continue;
                }
            };
            if file_type.is_symlink() {
                debug!(
                    path = %path.display(),
                    "skipping symlink in steering scan directory"
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
                .any(|excluded| excluded.eq_ignore_ascii_case(name))
            {
                continue;
            }
            if Path::new(name)
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("md"))
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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn finds_md_files_at_steering_root() {
        let tmp = tempdir().unwrap();
        let steering = tmp.path().join("steering");
        fs::create_dir_all(&steering).unwrap();
        fs::write(steering.join("guide.md"), b"guide").unwrap();
        fs::write(steering.join("not.txt"), b"ignored").unwrap();

        let found = discover_steering_files_in_dirs(
            tmp.path(),
            &["./steering/".to_string()],
        );
        let names: Vec<_> = found
            .iter()
            .map(|f| f.source.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["guide.md"]);
    }

    #[test]
    fn returns_empty_when_directory_missing() {
        let tmp = tempdir().unwrap();
        let found = discover_steering_files_in_dirs(
            tmp.path(),
            &["./missing/".to_string()],
        );
        assert!(found.is_empty());
    }

    #[test]
    fn excludes_readme_case_insensitive() {
        let tmp = tempdir().unwrap();
        let steering = tmp.path().join("steering");
        fs::create_dir_all(&steering).unwrap();
        fs::write(steering.join("README.md"), b"readme").unwrap();
        fs::write(steering.join("readme.md"), b"lowercase").unwrap();
        fs::write(steering.join("real.md"), b"real").unwrap();

        let found = discover_steering_files_in_dirs(
            tmp.path(),
            &["./steering/".to_string()],
        );
        let names: Vec<_> = found
            .iter()
            .map(|f| f.source.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["real.md"]);
    }

    #[test]
    fn rejects_path_traversal() {
        let tmp = tempdir().unwrap();
        let plugin = tmp.path().join("plugin");
        fs::create_dir_all(&plugin).unwrap();
        let escape = tmp.path().join("escape");
        fs::create_dir_all(&escape).unwrap();
        fs::write(escape.join("loot.md"), b"loot").unwrap();

        let found = discover_steering_files_in_dirs(
            &plugin,
            &["../escape/".to_string()],
        );
        assert!(found.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn skips_symlinks() {
        use std::os::unix::fs::symlink;
        let tmp = tempdir().unwrap();
        let steering = tmp.path().join("steering");
        fs::create_dir_all(&steering).unwrap();
        fs::write(steering.join("real.md"), b"real").unwrap();

        let outside = tmp.path().join("outside.md");
        fs::write(&outside, b"outside").unwrap();
        symlink(&outside, steering.join("evil.md")).unwrap();

        let found = discover_steering_files_in_dirs(
            tmp.path(),
            &["./steering/".to_string()],
        );
        let names: Vec<_> = found
            .iter()
            .map(|f| f.source.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["real.md"]);
    }

    #[test]
    fn carries_scan_root_for_destination_computation() {
        let tmp = tempdir().unwrap();
        let steering = tmp.path().join("steering");
        fs::create_dir_all(&steering).unwrap();
        fs::write(steering.join("a.md"), b"a").unwrap();

        let found = discover_steering_files_in_dirs(
            tmp.path(),
            &["./steering/".to_string()],
        );
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].scan_root, steering);
    }
}
```

- [ ] **Step 2: Run tests, verify they pass**

Run: `cargo test -p kiro-market-core --lib steering::discover::tests`
Expected: All six tests PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/kiro-market-core/src/steering/discover.rs
git commit -m "feat(core): discover_steering_files_in_dirs with security primitives"
```

---

## Task 4: Extend `PluginInstallContext` with `steering_scan_paths`

**Files:**
- Modify: `crates/kiro-market-core/src/service/browse.rs`

- [ ] **Step 1: Write the failing test**

Append to the `#[cfg(test)] mod tests` block in `crates/kiro-market-core/src/service/browse.rs`:

```rust
#[test]
fn resolve_plugin_install_context_uses_default_steering_when_absent() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("plugin.json"),
        br#"{"name": "p"}"#,
    )
    .unwrap();
    let ctx = resolve_plugin_install_context_from_dir(tmp.path()).unwrap();
    assert_eq!(
        ctx.steering_scan_paths,
        crate::DEFAULT_STEERING_PATHS
            .iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>(),
    );
}

#[test]
fn resolve_plugin_install_context_uses_manifest_steering_when_declared() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("plugin.json"),
        br#"{"name": "p", "steering": ["./guide/", "./extras/"]}"#,
    )
    .unwrap();
    let ctx = resolve_plugin_install_context_from_dir(tmp.path()).unwrap();
    assert_eq!(ctx.steering_scan_paths, vec!["./guide/", "./extras/"]);
}
```

- [ ] **Step 2: Run tests, verify they fail**

Run: `cargo test -p kiro-market-core --lib service::browse::tests::resolve_plugin_install_context_uses_default_steering`
Expected: FAIL — `no field 'steering_scan_paths' on type 'PluginInstallContext'`.

- [ ] **Step 3: Add the field and resolver logic**

Find `pub struct PluginInstallContext`. Add the field:

```rust
pub steering_scan_paths: Vec<String>,
```

Find `resolve_plugin_install_context_from_dir`. The function reads the `PluginManifest` and constructs a `PluginInstallContext`. Add (immediately after the analogous `agent_scan_paths` resolution):

```rust
let steering_scan_paths = if manifest.steering.is_empty() {
    crate::DEFAULT_STEERING_PATHS
        .iter()
        .map(|s| (*s).to_string())
        .collect()
} else {
    manifest.steering.clone()
};
```

And in the constructor literal:

```rust
steering_scan_paths,
```

- [ ] **Step 4: Run tests, verify they pass**

Run: `cargo test -p kiro-market-core --lib service::browse::tests::resolve_plugin_install_context_uses`
Expected: Both PASS.

- [ ] **Step 5: Run full crate tests**

Run: `cargo test -p kiro-market-core`
Expected: All tests pass.

- [ ] **Step 6: Commit**

```bash
git add crates/kiro-market-core/src/service/browse.rs
git commit -m "feat(core): add steering_scan_paths to PluginInstallContext"
```

---

## Task 5: Add `InstalledSteering`, `InstalledSteeringMeta`, `INSTALLED_STEERING_FILE`

**Files:**
- Modify: `crates/kiro-market-core/src/project.rs`

- [ ] **Step 1: Write the backward-compat test**

Append to the `#[cfg(test)] mod tests` block in `crates/kiro-market-core/src/project.rs`:

```rust
#[test]
fn installed_steering_loads_legacy_empty_object() {
    // Old projects without any steering install: file may not exist, or
    // may be `{}`. Both must deserialize to an empty wrapper.
    let from_empty: InstalledSteering = serde_json::from_slice(b"{}").unwrap();
    assert!(from_empty.files.is_empty());
}

#[test]
fn installed_steering_round_trips_through_serde() {
    let mut steering = InstalledSteering::default();
    steering.files.insert(
        std::path::PathBuf::from("review-process.md"),
        InstalledSteeringMeta {
            marketplace: "m".into(),
            plugin: "p".into(),
            version: Some("0.1.0".into()),
            installed_at: chrono::Utc::now(),
            source_hash: "blake3:abc".into(),
            installed_hash: "blake3:abc".into(),
        },
    );
    let bytes = serde_json::to_vec(&steering).unwrap();
    let back: InstalledSteering = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(back.files.len(), 1);
}
```

- [ ] **Step 2: Run tests, verify they fail**

Run: `cargo test -p kiro-market-core --lib installed_steering_loads_legacy_empty_object`
Expected: FAIL — `cannot find type 'InstalledSteering'`.

- [ ] **Step 3: Add the tracking types and constant**

Append to `crates/kiro-market-core/src/project.rs` (near the existing `InstalledAgents` definition, around line 67):

```rust
/// Tracking entry for one installed steering file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledSteeringMeta {
    pub marketplace: String,
    pub plugin: String,
    pub version: Option<String>,
    pub installed_at: DateTime<Utc>,
    pub source_hash: String,
    pub installed_hash: String,
}

/// On-disk structure of `installed-steering.json`. Map key is the file's
/// relative path under `.kiro/steering/`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InstalledSteering {
    #[serde(default)]
    pub files: HashMap<PathBuf, InstalledSteeringMeta>,
}
```

Add the constant near the existing `INSTALLED_AGENTS_FILE` constant:

```rust
/// Name of the steering tracking file inside `.kiro/`.
const INSTALLED_STEERING_FILE: &str = "installed-steering.json";
```

- [ ] **Step 4: Run tests, verify they pass**

Run: `cargo test -p kiro-market-core --lib installed_steering`
Expected: Both PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/kiro-market-core/src/project.rs
git commit -m "feat(core): InstalledSteering + InstalledSteeringMeta tracking types"
```

---

## Task 6: Add steering load/save helpers on `KiroProject`

**Files:**
- Modify: `crates/kiro-market-core/src/project.rs`

- [ ] **Step 1: Write the test**

Append to `#[cfg(test)] mod tests`:

```rust
#[test]
fn load_installed_steering_returns_default_when_file_missing() {
    let tmp = tempdir().unwrap();
    let project = KiroProject::new(tmp.path()).unwrap();
    let installed = project.load_installed_steering().unwrap();
    assert!(installed.files.is_empty());
}

#[test]
fn load_installed_steering_round_trips_through_disk() {
    let tmp = tempdir().unwrap();
    let project = KiroProject::new(tmp.path()).unwrap();

    let mut to_save = InstalledSteering::default();
    to_save.files.insert(
        PathBuf::from("guide.md"),
        InstalledSteeringMeta {
            marketplace: "m".into(),
            plugin: "p".into(),
            version: None,
            installed_at: chrono::Utc::now(),
            source_hash: "blake3:abc".into(),
            installed_hash: "blake3:abc".into(),
        },
    );
    project.write_steering_tracking(&to_save).unwrap();

    let loaded = project.load_installed_steering().unwrap();
    assert_eq!(loaded.files.len(), 1);
    assert!(loaded.files.contains_key(std::path::Path::new("guide.md")));
}
```

- [ ] **Step 2: Run tests, verify they fail**

Run: `cargo test -p kiro-market-core --lib load_installed_steering`
Expected: FAIL — `no method named 'load_installed_steering'`.

- [ ] **Step 3: Implement the helpers**

Add to `impl KiroProject`:

```rust
/// The `.kiro/steering/` directory.
pub fn steering_dir(&self) -> PathBuf {
    self.kiro_dir().join("steering")
}

/// Path to the steering tracking file.
fn steering_tracking_path(&self) -> PathBuf {
    self.kiro_dir().join(INSTALLED_STEERING_FILE)
}

/// Load the installed-steering tracking file, returning a default empty
/// wrapper if the file does not exist.
///
/// # Errors
///
/// I/O or JSON parse failures.
pub fn load_installed_steering(&self) -> crate::error::Result<InstalledSteering> {
    let path = self.steering_tracking_path();
    match fs::read(&path) {
        Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            debug!(
                path = %path.display(),
                "steering tracking file not found, returning default"
            );
            Ok(InstalledSteering::default())
        }
        Err(e) => Err(e.into()),
    }
}

/// Persist the steering tracking file atomically.
///
/// # Errors
///
/// I/O or JSON serialisation failures.
pub fn write_steering_tracking(
    &self,
    installed: &InstalledSteering,
) -> crate::error::Result<()> {
    let path = self.steering_tracking_path();
    fs::create_dir_all(self.kiro_dir())?;
    let bytes = serde_json::to_vec_pretty(installed)?;
    // Atomic write via temp file + rename.
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, &bytes)?;
    fs::rename(&tmp, &path)?;
    Ok(())
}
```

- [ ] **Step 4: Run tests, verify they pass**

Run: `cargo test -p kiro-market-core --lib load_installed_steering`
Expected: Both PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/kiro-market-core/src/project.rs
git commit -m "feat(core): load_installed_steering + write_steering_tracking helpers"
```

---

## Task 7: `KiroProject::install_steering_file` — happy path

**Files:**
- Modify: `crates/kiro-market-core/src/project.rs`

- [ ] **Step 1: Write the test**

Append to `#[cfg(test)] mod tests`:

```rust
#[test]
fn install_steering_file_writes_to_kiro_steering_with_hashes() {
    let tmp = tempdir().unwrap();
    let project = KiroProject::new(tmp.path()).unwrap();

    let scan_root = tmp.path().join("source-steering");
    fs::create_dir_all(&scan_root).unwrap();
    let src = scan_root.join("guide.md");
    fs::write(&src, b"# Steering Guide\n\nbody").unwrap();

    let source_hash = crate::hash::hash_artifact(
        &scan_root,
        &[PathBuf::from("guide.md")],
    )
    .unwrap();

    let discovered = crate::agent::DiscoveredNativeFile {
        source: src.clone(),
        scan_root: scan_root.clone(),
    };

    let outcome = project
        .install_steering_file(
            &discovered,
            "marketplace-x",
            "plugin-y",
            Some("0.1.0"),
            &source_hash,
            false,
        )
        .unwrap();

    let dest = project.steering_dir().join("guide.md");
    assert_eq!(outcome.destination, dest);
    assert!(dest.exists());
    assert_eq!(fs::read(&dest).unwrap(), b"# Steering Guide\n\nbody");
    assert_eq!(outcome.source_hash, source_hash);
    assert!(outcome.installed_hash.starts_with("blake3:"));
    assert!(!outcome.was_idempotent);

    // Tracking entry exists.
    let tracking = project.load_installed_steering().unwrap();
    let entry = tracking
        .files
        .get(std::path::Path::new("guide.md"))
        .expect("tracking entry written");
    assert_eq!(entry.plugin, "plugin-y");
    assert_eq!(entry.marketplace, "marketplace-x");
}
```

- [ ] **Step 2: Run test, verify it fails**

Run: `cargo test -p kiro-market-core --lib install_steering_file_writes_to_kiro_steering_with_hashes`
Expected: FAIL — `no method named 'install_steering_file'`.

- [ ] **Step 3: Implement `install_steering_file`**

Add to `impl KiroProject`:

```rust
/// Install one steering file. Same idempotency / collision rules as
/// native agent install: matching `source_hash` is a no-op, mismatch
/// requires `force == true`, cross-plugin or orphan paths require
/// `force == true` to overwrite.
pub fn install_steering_file(
    &self,
    source: &crate::agent::DiscoveredNativeFile,
    marketplace: &str,
    plugin: &str,
    version: Option<&str>,
    source_hash: &str,
    force: bool,
) -> Result<crate::steering::InstalledSteeringOutcome, crate::steering::SteeringError> {
    let rel_path = source
        .source
        .strip_prefix(&source.scan_root)
        .map_err(|_| {
            crate::steering::SteeringError::SourceReadFailed {
                path: source.source.clone(),
                source: std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "steering source not under scan_root",
                ),
            }
        })?
        .to_path_buf();
    let dest = self.steering_dir().join(&rel_path);

    crate::file_lock::with_file_lock(
        &self.steering_tracking_path(),
        || -> Result<crate::steering::InstalledSteeringOutcome, crate::steering::SteeringError> {
            let mut installed = self.load_installed_steering().map_err(|e| {
                crate::steering::SteeringError::TrackingIoFailed {
                    path: self.steering_tracking_path(),
                    source: std::io::Error::other(e.to_string()),
                }
            })?;

            // Idempotency / collision check.
            let mut forced_overwrite = false;
            if let Some(existing) = installed.files.get(&rel_path) {
                if existing.plugin == plugin {
                    if existing.source_hash == source_hash {
                        return Ok(crate::steering::InstalledSteeringOutcome {
                            source: source.source.clone(),
                            destination: dest,
                            forced_overwrite: false,
                            was_idempotent: true,
                            source_hash: source_hash.to_string(),
                            installed_hash: existing.installed_hash.clone(),
                        });
                    } else if !force {
                        return Err(
                            crate::steering::SteeringError::ContentChangedRequiresForce {
                                rel: rel_path.clone(),
                            },
                        );
                    } else {
                        forced_overwrite = true;
                    }
                } else if !force {
                    return Err(
                        crate::steering::SteeringError::PathOwnedByOtherPlugin {
                            rel: rel_path.clone(),
                            owner: existing.plugin.clone(),
                        },
                    );
                } else {
                    forced_overwrite = true;
                }
            } else if dest.exists() {
                if !force {
                    return Err(
                        crate::steering::SteeringError::OrphanFileAtDestination {
                            path: dest.clone(),
                        },
                    );
                }
                forced_overwrite = true;
            }

            // Stage and rename.
            std::fs::create_dir_all(self.steering_dir())?;
            let staging = self
                .steering_dir()
                .join(format!(".staging-{}", uuid_or_pid()));
            std::fs::write(&staging, std::fs::read(&source.source)?)?;
            if dest.exists() {
                std::fs::remove_file(&dest)?;
            }
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::rename(&staging, &dest)?;

            // Compute installed_hash over what landed.
            let installed_hash = crate::hash::hash_artifact(
                &self.steering_dir(),
                &[rel_path.clone()],
            )?;

            installed.files.insert(
                rel_path,
                InstalledSteeringMeta {
                    marketplace: marketplace.to_string(),
                    plugin: plugin.to_string(),
                    version: version.map(String::from),
                    installed_at: chrono::Utc::now(),
                    source_hash: source_hash.to_string(),
                    installed_hash: installed_hash.clone(),
                },
            );
            self.write_steering_tracking(&installed).map_err(|e| {
                crate::steering::SteeringError::TrackingIoFailed {
                    path: self.steering_tracking_path(),
                    source: std::io::Error::other(e.to_string()),
                }
            })?;

            Ok(crate::steering::InstalledSteeringOutcome {
                source: source.source.clone(),
                destination: dest,
                forced_overwrite,
                was_idempotent: false,
                source_hash: source_hash.to_string(),
                installed_hash,
            })
        },
    )
}
```

(Note: `uuid_or_pid()` is a helper for generating a unique staging filename. If the project doesn't already have such a helper, use `std::process::id().to_string()` or similar — the goal is just to avoid a collision if two installs race. The file-lock around the operation should prevent races, but a unique staging name is defensive.)

- [ ] **Step 4: Run test, verify it passes**

Run: `cargo test -p kiro-market-core --lib install_steering_file_writes_to_kiro_steering_with_hashes`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/kiro-market-core/src/project.rs
git commit -m "feat(core): KiroProject::install_steering_file (happy path)"
```

---

## Task 8: `install_steering_file` — idempotent + content-changed + cross-plugin + orphan

**Files:**
- Modify: `crates/kiro-market-core/src/project.rs` (tests only)

- [ ] **Step 1: Write the four collision tests**

Append to `#[cfg(test)] mod tests`:

```rust
#[test]
fn install_steering_idempotent_when_source_hash_matches() {
    let tmp = tempdir().unwrap();
    let project = KiroProject::new(tmp.path()).unwrap();
    let scan = tmp.path().join("src");
    fs::create_dir_all(&scan).unwrap();
    fs::write(scan.join("a.md"), b"a").unwrap();
    let h = crate::hash::hash_artifact(&scan, &[PathBuf::from("a.md")]).unwrap();
    let d = crate::agent::DiscoveredNativeFile {
        source: scan.join("a.md"),
        scan_root: scan.clone(),
    };

    let first = project
        .install_steering_file(&d, "m", "p", None, &h, false)
        .unwrap();
    assert!(!first.was_idempotent);

    let second = project
        .install_steering_file(&d, "m", "p", None, &h, false)
        .unwrap();
    assert!(second.was_idempotent);
}

#[test]
fn install_steering_content_changed_requires_force() {
    let tmp = tempdir().unwrap();
    let project = KiroProject::new(tmp.path()).unwrap();
    let scan = tmp.path().join("src");
    fs::create_dir_all(&scan).unwrap();
    fs::write(scan.join("a.md"), b"v1").unwrap();
    let h_v1 = crate::hash::hash_artifact(&scan, &[PathBuf::from("a.md")]).unwrap();
    let d = crate::agent::DiscoveredNativeFile {
        source: scan.join("a.md"),
        scan_root: scan.clone(),
    };
    project
        .install_steering_file(&d, "m", "p", None, &h_v1, false)
        .unwrap();

    fs::write(scan.join("a.md"), b"v2").unwrap();
    let h_v2 = crate::hash::hash_artifact(&scan, &[PathBuf::from("a.md")]).unwrap();
    let err = project
        .install_steering_file(&d, "m", "p", None, &h_v2, false)
        .unwrap_err();
    assert!(matches!(
        err,
        crate::steering::SteeringError::ContentChangedRequiresForce { .. }
    ));

    let outcome = project
        .install_steering_file(&d, "m", "p", None, &h_v2, true)
        .unwrap();
    assert!(outcome.forced_overwrite);
}

#[test]
fn install_steering_cross_plugin_clash_fails_loudly() {
    let tmp = tempdir().unwrap();
    let project = KiroProject::new(tmp.path()).unwrap();

    let scan_a = tmp.path().join("a-src");
    fs::create_dir_all(&scan_a).unwrap();
    fs::write(scan_a.join("shared.md"), b"from-a").unwrap();
    let h_a =
        crate::hash::hash_artifact(&scan_a, &[PathBuf::from("shared.md")]).unwrap();
    let d_a = crate::agent::DiscoveredNativeFile {
        source: scan_a.join("shared.md"),
        scan_root: scan_a.clone(),
    };
    project
        .install_steering_file(&d_a, "m", "plugin-a", None, &h_a, false)
        .unwrap();

    let scan_b = tmp.path().join("b-src");
    fs::create_dir_all(&scan_b).unwrap();
    fs::write(scan_b.join("shared.md"), b"from-b").unwrap();
    let h_b =
        crate::hash::hash_artifact(&scan_b, &[PathBuf::from("shared.md")]).unwrap();
    let d_b = crate::agent::DiscoveredNativeFile {
        source: scan_b.join("shared.md"),
        scan_root: scan_b.clone(),
    };

    let err = project
        .install_steering_file(&d_b, "m", "plugin-b", None, &h_b, false)
        .unwrap_err();
    match err {
        crate::steering::SteeringError::PathOwnedByOtherPlugin { rel, owner } => {
            assert_eq!(rel, PathBuf::from("shared.md"));
            assert_eq!(owner, "plugin-a");
        }
        other => panic!("expected PathOwnedByOtherPlugin, got {other:?}"),
    }

    let outcome = project
        .install_steering_file(&d_b, "m", "plugin-b", None, &h_b, true)
        .unwrap();
    assert!(outcome.forced_overwrite);

    let tracking = project.load_installed_steering().unwrap();
    let entry = tracking
        .files
        .get(std::path::Path::new("shared.md"))
        .unwrap();
    assert_eq!(entry.plugin, "plugin-b", "ownership must transfer");
}

#[test]
fn install_steering_orphan_at_destination_fails_loudly() {
    let tmp = tempdir().unwrap();
    let project = KiroProject::new(tmp.path()).unwrap();
    fs::create_dir_all(project.steering_dir()).unwrap();
    fs::write(project.steering_dir().join("orphan.md"), b"orphan").unwrap();

    let scan = tmp.path().join("src");
    fs::create_dir_all(&scan).unwrap();
    fs::write(scan.join("orphan.md"), b"new").unwrap();
    let h =
        crate::hash::hash_artifact(&scan, &[PathBuf::from("orphan.md")]).unwrap();
    let d = crate::agent::DiscoveredNativeFile {
        source: scan.join("orphan.md"),
        scan_root: scan.clone(),
    };

    let err = project
        .install_steering_file(&d, "m", "p", None, &h, false)
        .unwrap_err();
    assert!(matches!(
        err,
        crate::steering::SteeringError::OrphanFileAtDestination { .. }
    ));

    let outcome = project
        .install_steering_file(&d, "m", "p", None, &h, true)
        .unwrap();
    assert!(outcome.forced_overwrite);
}
```

- [ ] **Step 2: Run tests, verify they pass**

Run: `cargo test -p kiro-market-core --lib install_steering_`
Expected: All four tests PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/kiro-market-core/src/project.rs
git commit -m "test(core): install_steering_file collision policy + --force"
```

---

## Task 9: `MarketplaceService::install_plugin_steering`

**Files:**
- Modify: `crates/kiro-market-core/src/service/mod.rs`

- [ ] **Step 1: Write the test**

Append to the `#[cfg(test)] mod tests` block in `crates/kiro-market-core/src/service/mod.rs`:

```rust
#[test]
fn install_plugin_steering_discovers_and_installs_all_files() {
    use crate::steering::SteeringInstallOptions;

    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("plugin.json"),
        br#"{"name": "p"}"#,
    )
    .unwrap();
    let steering = tmp.path().join("steering");
    std::fs::create_dir_all(&steering).unwrap();
    std::fs::write(steering.join("alpha.md"), b"alpha").unwrap();
    std::fs::write(steering.join("beta.md"), b"beta").unwrap();

    let svc = crate::service::test_support::test_marketplace_service();
    let project_root = tempfile::tempdir().unwrap();
    let project = crate::project::KiroProject::new(project_root.path()).unwrap();

    let ctx = crate::service::browse::resolve_plugin_install_context_from_dir(
        tmp.path(),
    )
    .unwrap();

    let result = svc.install_plugin_steering(
        &project,
        "marketplace-x",
        &ctx,
        SteeringInstallOptions { force: false },
    );

    assert_eq!(result.installed.len(), 2);
    assert!(result.failed.is_empty());

    assert!(project_root.path().join(".kiro/steering/alpha.md").exists());
    assert!(project_root.path().join(".kiro/steering/beta.md").exists());

    // Idempotent reinstall.
    let again = svc.install_plugin_steering(
        &project,
        "marketplace-x",
        &ctx,
        SteeringInstallOptions { force: false },
    );
    assert!(again.installed.iter().all(|o| o.was_idempotent));
}
```

- [ ] **Step 2: Run test, verify it fails**

Run: `cargo test -p kiro-market-core --lib install_plugin_steering_discovers_and_installs_all_files`
Expected: FAIL — `no method named 'install_plugin_steering'`.

- [ ] **Step 3: Implement `install_plugin_steering`**

Add to `impl MarketplaceService`:

```rust
pub fn install_plugin_steering(
    &self,
    project: &crate::project::KiroProject,
    marketplace: &str,
    ctx: &crate::service::browse::PluginInstallContext,
    opts: crate::steering::SteeringInstallOptions,
) -> crate::steering::InstallSteeringResult {
    let mut result = crate::steering::InstallSteeringResult {
        installed: Vec::new(),
        failed: Vec::new(),
        warnings: Vec::new(),
    };

    let files = crate::steering::discover_steering_files_in_dirs(
        &ctx.plugin_dir,
        &ctx.steering_scan_paths,
    );

    for f in &files {
        let rel = match f.source.strip_prefix(&f.scan_root) {
            Ok(p) => p.to_path_buf(),
            Err(_) => {
                result.failed.push(crate::steering::FailedSteeringFile {
                    source: f.source.clone(),
                    error: crate::steering::SteeringError::SourceReadFailed {
                        path: f.source.clone(),
                        source: std::io::Error::new(
                            std::io::ErrorKind::InvalidInput,
                            "source not under scan_root",
                        ),
                    },
                });
                continue;
            }
        };

        let source_hash =
            match crate::hash::hash_artifact(&f.scan_root, &[rel.clone()]) {
                Ok(h) => h,
                Err(e) => {
                    result.failed.push(crate::steering::FailedSteeringFile {
                        source: f.source.clone(),
                        error: e.into(),
                    });
                    continue;
                }
            };

        match project.install_steering_file(
            f,
            marketplace,
            &ctx.plugin_name,
            ctx.plugin_version.as_deref(),
            &source_hash,
            opts.force,
        ) {
            Ok(outcome) => result.installed.push(outcome),
            Err(err) => {
                result.failed.push(crate::steering::FailedSteeringFile {
                    source: f.source.clone(),
                    error: err,
                })
            }
        }
    }

    result
}
```

- [ ] **Step 4: Run test, verify it passes**

Run: `cargo test -p kiro-market-core --lib install_plugin_steering_discovers_and_installs_all_files`
Expected: PASS.

- [ ] **Step 5: Run full crate tests**

Run: `cargo test -p kiro-market-core`
Expected: All tests pass.

- [ ] **Step 6: Commit**

```bash
git add crates/kiro-market-core/src/service/mod.rs
git commit -m "feat(core): MarketplaceService::install_plugin_steering"
```

---

## Task 10: CLI install command calls `install_plugin_steering`

**Files:**
- Modify: `crates/kiro-market/src/commands/install.rs`

- [ ] **Step 1: Find the existing install flow**

Run: `grep -n "install_plugin_skills\|install_plugin_agents" crates/kiro-market/src/commands/install.rs`
Note the line where these are called sequentially.

- [ ] **Step 2: Add the steering call after `install_plugin_agents`**

Add (after the existing `install_plugin_agents` invocation):

```rust
let steering_result = svc.install_plugin_steering(
    &project,
    &marketplace_name,
    &ctx,
    kiro_market_core::steering::SteeringInstallOptions { force },
);
```

(Variable names — `svc`, `project`, `marketplace_name`, `ctx`, `force` — depend on the existing function's signature. Adapt to whatever's in scope at the call site.)

- [ ] **Step 3: Render the steering result**

After the existing agent-result rendering, add:

```rust
for outcome in &steering_result.installed {
    let suffix = if outcome.was_idempotent {
        " (unchanged)".dimmed()
    } else if outcome.forced_overwrite {
        " (forced)".yellow()
    } else {
        "".normal()
    };
    let rel = outcome
        .destination
        .strip_prefix(project.kiro_dir().join("steering"))
        .unwrap_or(&outcome.destination);
    println!("  {} steering {}{}", "✓".green(), rel.display(), suffix);
}

for failed in &steering_result.failed {
    println!(
        "  {} steering {}: {}",
        "✗".red(),
        failed.source.display(),
        kiro_market_core::error::error_full_chain(&failed.error)
    );
}
```

(Adapt rendering style — colored crate, prefix glyphs — to match existing presenter conventions.)

- [ ] **Step 4: Update exit-code logic**

Find where the install command computes its exit code (typically based on `result.failed.is_empty()` for the agent result). Extend to also consider `steering_result.failed`:

```rust
let exit_code = if !agent_result.failed.is_empty() || !steering_result.failed.is_empty() {
    1
} else {
    0
};
```

- [ ] **Step 5: Manually verify against the starter-kit**

```bash
# Re-use the test project from Stage 2's manual smoke test, or:
cd /tmp/kiro-test-project
rm -rf .kiro  # fresh state
cargo run --bin kiro-market -- install kiro-code-reviewer
# Expected output: 6 "✓ agent X" lines + "✓ N companion file(s)" line
#                  + 1 "✓ steering review-process.md" line.
ls .kiro/steering/  # Should contain review-process.md
```

- [ ] **Step 6: Commit**

```bash
git add crates/kiro-market/src/commands/install.rs
git commit -m "feat(cli): wire install_plugin_steering into install command"
```

---

## Task 11: End-to-end integration test (full starter-kit shape)

**Files:**
- Modify: `crates/kiro-market-core/tests/integration_native_install.rs` (extend Stage 2's test) OR
- Create: `crates/kiro-market-core/tests/integration_steering.rs`

If Stage 2's test file exists, extend it. Otherwise, create a new file.

- [ ] **Step 1: Extend the integration test**

In `crates/kiro-market-core/tests/integration_native_install.rs` (or new file), add:

```rust
#[test]
fn end_to_end_native_plugin_with_agents_companions_and_steering() {
    use kiro_market_core::project::KiroProject;
    use kiro_market_core::service::{
        browse::resolve_plugin_install_context_from_dir, AgentInstallOptions,
    };
    use kiro_market_core::steering::SteeringInstallOptions;
    use std::fs;
    use tempfile::tempdir;

    // Plugin layout: agents/, agents/prompts/, steering/.
    let plugin_dir = tempdir().unwrap();
    fs::write(
        plugin_dir.path().join("plugin.json"),
        br#"{"name": "fake-reviewers", "format": "kiro-cli"}"#,
    )
    .unwrap();
    let agents = plugin_dir.path().join("agents");
    let prompts = agents.join("prompts");
    let steering = plugin_dir.path().join("steering");
    fs::create_dir_all(&prompts).unwrap();
    fs::create_dir_all(&steering).unwrap();

    for name in &["reviewer", "tester"] {
        fs::write(
            agents.join(format!("{name}.json")),
            format!(
                r#"{{"name": "{name}", "prompt": "file://./prompts/{name}.md", "resources": ["file://.kiro/steering/process.md"]}}"#
            ),
        )
        .unwrap();
        fs::write(prompts.join(format!("{name}.md")), b"prompt body").unwrap();
    }
    fs::write(steering.join("process.md"), b"# Process\n\nshared rules").unwrap();

    let project_root = tempdir().unwrap();
    let project = KiroProject::new(project_root.path()).unwrap();
    let svc = kiro_market_core::service::test_support::test_marketplace_service();
    let ctx = resolve_plugin_install_context_from_dir(plugin_dir.path()).unwrap();

    // Install agents + companions.
    let agent_result = svc.install_plugin_agents(
        &project,
        "test-marketplace",
        &ctx,
        AgentInstallOptions { force: false, accept_mcp: false },
    );
    assert_eq!(agent_result.installed_agents.len(), 2);
    assert!(agent_result.failed.is_empty());

    // Install steering.
    let steering_result = svc.install_plugin_steering(
        &project,
        "test-marketplace",
        &ctx,
        SteeringInstallOptions { force: false },
    );
    assert_eq!(steering_result.installed.len(), 1);
    assert!(steering_result.failed.is_empty());

    // Verify all destinations.
    assert!(project_root.path().join(".kiro/agents/reviewer.json").exists());
    assert!(project_root.path().join(".kiro/agents/tester.json").exists());
    assert!(project_root.path().join(".kiro/agents/prompts/reviewer.md").exists());
    assert!(project_root.path().join(".kiro/agents/prompts/tester.md").exists());
    assert!(project_root.path().join(".kiro/steering/process.md").exists());

    // Idempotent reinstall of all three.
    let agents_again = svc.install_plugin_agents(
        &project,
        "test-marketplace",
        &ctx,
        AgentInstallOptions { force: false, accept_mcp: false },
    );
    assert!(agents_again.installed_agents.iter().all(|o| o.was_idempotent));

    let steering_again = svc.install_plugin_steering(
        &project,
        "test-marketplace",
        &ctx,
        SteeringInstallOptions { force: false },
    );
    assert!(steering_again.installed.iter().all(|o| o.was_idempotent));
}
```

- [ ] **Step 2: Run the integration test**

Run: `cargo test -p kiro-market-core --test integration_native_install end_to_end_native_plugin_with_agents_companions_and_steering`
(or `--test integration_steering` if you created a new file)
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/kiro-market-core/tests/
git commit -m "test(core): end-to-end native plugin install with steering"
```

---

## Task 12: Final verification — full test suite + clippy + fmt

**Files:** none (verification only)

- [ ] **Step 1: Run the full test suite**

Run: `cargo test --workspace`
Expected: All tests pass.

- [ ] **Step 2: Run clippy**

Run: `cargo clippy --workspace --tests -- -D warnings`
Expected: No warnings.

- [ ] **Step 3: Run fmt check**

Run: `cargo fmt --all --check`
Expected: No diff.

- [ ] **Step 4: Verify against the real starter-kit one final time**

```bash
cd /tmp && rm -rf kiro-starter-kit && git clone --depth 1 https://github.com/dwalleck/kiro-starter-kit.git
mkdir -p /tmp/kiro-final-test && cd /tmp/kiro-final-test
rm -rf .kiro
cargo run --bin kiro-market -- marketplace add /tmp/kiro-starter-kit --name kiro-starter-kit
cargo run --bin kiro-market -- install kiro-code-reviewer
ls .kiro/agents/
ls .kiro/agents/prompts/
ls .kiro/steering/
cat .kiro/installed-steering.json
```

Expected:
- 6 agent JSONs at `.kiro/agents/`
- 6 prompts at `.kiro/agents/prompts/`
- 1 steering file at `.kiro/steering/review-process.md`
- `installed-steering.json` exists with one entry

- [ ] **Step 5: Commit any cleanup**

```bash
# Only if previous steps required edits:
git add -u
git commit -m "style: address clippy + fmt for steering import"
```

---

## Out-of-Plan Notes for Implementer

**Why steering uses `DiscoveredNativeFile` instead of a dedicated `SteeringSource` type.** The fields are identical (`source: PathBuf`, `scan_root: PathBuf`). Reusing `DiscoveredNativeFile` keeps the discovery layer's wire shape consistent across native agent + companion + steering contexts. If steering ever needs additional metadata not relevant to native discovery, a dedicated newtype can be introduced as a follow-up.

**Why `install_steering_file` is one-file-at-a-time, not bundle-style like companions.** Steering files have no shared identity — each filename IS the unique key under `.kiro/steering/`. There's no plugin-wide "steering bundle" concept. Per-file install gives finer-grained collision detection (one bad file doesn't fail the rest) at the cost of slightly more tracking-file writes per plugin install. The trade-off is the right one — steering authors are encouraged to ship a small number of files, and the per-file granularity makes failure messages more useful.

**Why the steering tracking key is the relative path under `.kiro/steering/` instead of a synthetic name.** The filename IS the user-facing identity. A future `kiro-market remove-steering review-process.md` reads naturally; `kiro-market remove-steering some-uuid` would not. Path-as-key also makes cross-plugin overlap detection trivial — same destination = same key in the tracking map.

**Why no MCP gate for steering.** Steering files are markdown documents read into agent context — no execution semantics, no subprocess spawning, no network. The MCP gate is specific to agent install paths where `mcpServers` may bring stdio/http transports.

**Why the staging filename uses `uuid_or_pid()`.** Defense in depth on top of the file lock. The lock prevents two installers from racing on the same tracking file, but a unique staging name protects against any leftover staging file from a prior crash that may have shared a name. If `uuid` isn't already a dep, use `std::process::id()` plus an atomic counter.

**Why `install_plugin_steering` doesn't have an idempotent-skip bucket like translated agents do.** Steering install's "idempotent" outcome IS a successful install (`was_idempotent: true` on the outcome). The translated-agent path's `skipped` bucket is a legacy of its name-collision-as-skip semantics — the new design treats idempotency as success, not skip. Steering follows the new semantics.

# Stage 1: Content-Hash Primitive Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a deterministic blake3-based content-hash primitive (`hash_artifact`, `hash_dir_tree`) and populate `source_hash` / `installed_hash` fields on every existing skill and translated-agent install path.

**Architecture:** New `kiro_market_core::hash` module provides the primitive. Existing `InstalledSkillMeta` and `InstalledAgentMeta` get two `Option<String>` hash fields (backward-compatible via `#[serde(default)]`). `install_skill_from_dir` and `install_agent_inner` (the translated agent path) compute and persist hashes during install. No new install paths, no new tracking files, no behavior change for callers — pure foundational work that Stages 2 and 3 build on.

**Tech Stack:** Rust (edition 2024), blake3 (~10× faster than SHA-256), hex (digest encoding), thiserror (typed errors), existing serde / serde_json / chrono.

**Spec reference:** `docs/plans/2026-04-23-kiro-cli-native-plugin-import-design.md` § "Tracking Schema and Content Hashes" and § "Implementation Phasing — Stage 1".

---

## File Structure

**New files:**
- `crates/kiro-market-core/src/hash.rs` — `hash_artifact`, `hash_dir_tree`, `HashError`

**Modified files:**
- `Cargo.toml` (workspace) — add blake3 + hex to `[workspace.dependencies]`
- `crates/kiro-market-core/Cargo.toml` — declare blake3 + hex
- `crates/kiro-market-core/src/lib.rs` — `pub mod hash;`
- `crates/kiro-market-core/src/error.rs` — `HashError` arm on top-level `Error`
- `crates/kiro-market-core/src/project.rs`:
  - `InstalledSkillMeta` (line ~32) — add `source_hash` / `installed_hash` fields
  - `InstalledAgentMeta` (line ~52) — add `source_hash` / `installed_hash` fields
  - `install_skill_from_dir` (line ~331) — compute and persist hashes
  - `write_skill_dir` (line ~669) — receive computed hashes via meta
  - `install_agent_inner` (line ~470) — compute and persist hashes

---

## Task 1: Add blake3 + hex to workspace dependencies

**Files:**
- Modify: `Cargo.toml` (workspace root)

- [ ] **Step 1: Add to `[workspace.dependencies]`**

Open `Cargo.toml` at the workspace root. Find the `[workspace.dependencies]` section. Add (alphabetically with existing entries):

```toml
blake3 = "1.5"
hex = "0.4"
```

- [ ] **Step 2: Verify the workspace resolves**

Run: `cargo metadata --format-version=1 --no-deps > /dev/null`
Expected: exits 0 with no errors.

- [ ] **Step 3: No commit yet**

Combined with Task 2's crate-level dep declaration into a single commit.

---

## Task 2: Declare blake3 + hex on `kiro-market-core`

**Files:**
- Modify: `crates/kiro-market-core/Cargo.toml`

- [ ] **Step 1: Add to `[dependencies]`**

In `crates/kiro-market-core/Cargo.toml`, after `chrono = { workspace = true }`:

```toml
blake3 = { workspace = true }
hex = { workspace = true }
```

- [ ] **Step 2: Verify the crate compiles**

Run: `cargo build -p kiro-market-core`
Expected: Build succeeds. The new deps download but are not yet used.

- [ ] **Step 3: Commit**

```bash
git add Cargo.toml crates/kiro-market-core/Cargo.toml
git commit -m "$(cat <<'EOF'
chore(deps): add blake3 + hex for content-hash primitive

blake3 chosen for ~10× SHA-256 speedup on small files and keyed-MAC
support for future signed manifests. hex for digest encoding.
Used by the upcoming kiro_market_core::hash module (Stage 1 of native
kiro-cli plugin import design).
EOF
)"
```

---

## Task 3: Create `hash` module skeleton with `HashError`

**Files:**
- Create: `crates/kiro-market-core/src/hash.rs`
- Modify: `crates/kiro-market-core/src/lib.rs`
- Modify: `crates/kiro-market-core/src/error.rs`

- [ ] **Step 1: Create `crates/kiro-market-core/src/hash.rs`**

```rust
//! Deterministic content-hash primitive for installed artifacts.
//!
//! `hash_artifact` produces a stable hex-encoded blake3 digest over a set
//! of files rooted at a base directory. Used by skill / agent / steering
//! install paths to populate `source_hash` (what was in the marketplace)
//! and `installed_hash` (what landed in the project), enabling idempotent
//! reinstall and future drift detection.
//!
//! See `docs/plans/2026-04-23-kiro-cli-native-plugin-import-design.md`
//! § "Tracking Schema and Content Hashes" for the design rationale.

use std::io;
use std::path::{Path, PathBuf};

use thiserror::Error;

/// Errors that can occur while computing an artifact hash.
#[derive(Debug, Error)]
pub enum HashError {
    /// A file listed in `relative_paths` could not be read.
    #[error("failed to read `{path}` while hashing")]
    ReadFailed {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    /// Walking a directory tree (used by `hash_dir_tree`) failed.
    #[error("failed to walk directory `{path}` while hashing")]
    WalkFailed {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

// Implementations land in subsequent tasks.
```

- [ ] **Step 2: Wire into `lib.rs`**

In `crates/kiro-market-core/src/lib.rs`, find the `pub mod` declarations (look for `pub mod agent;` or similar). Add:

```rust
pub mod hash;
```

- [ ] **Step 3: Wire `HashError` into top-level `Error`**

In `crates/kiro-market-core/src/error.rs`, find the `pub enum Error` definition. Add a new arm (after the existing `Agent` / `Skill` / etc. arms — placement doesn't matter for behavior but keep variants alphabetical if the existing list is):

```rust
#[error(transparent)]
Hash(#[from] crate::hash::HashError),
```

- [ ] **Step 4: Verify it compiles**

Run: `cargo build -p kiro-market-core`
Expected: Build succeeds. No warnings about unused module (the module body is empty besides `HashError` but it's referenced by `Error::Hash`).

- [ ] **Step 5: Commit**

```bash
git add crates/kiro-market-core/src/hash.rs crates/kiro-market-core/src/lib.rs crates/kiro-market-core/src/error.rs
git commit -m "feat(core): add hash module skeleton with HashError type"
```

---

## Task 4: Implement `hash_artifact` (happy path)

**Files:**
- Modify: `crates/kiro-market-core/src/hash.rs`
- Test: `crates/kiro-market-core/src/hash.rs` (inline `#[cfg(test)] mod tests`)

- [ ] **Step 1: Write the failing test**

Append to `crates/kiro-market-core/src/hash.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn hash_artifact_returns_blake3_prefixed_hex_for_single_file() {
        let tmp = tempdir().unwrap();
        let base = tmp.path();
        fs::write(base.join("a.txt"), b"hello").unwrap();

        let h = hash_artifact(base, &[PathBuf::from("a.txt")]).unwrap();

        assert!(h.starts_with("blake3:"), "got: {h}");
        // 32-byte blake3 → 64 hex chars + "blake3:" prefix = 71 chars.
        assert_eq!(h.len(), "blake3:".len() + 64);
    }
}
```

- [ ] **Step 2: Run test, verify it fails**

Run: `cargo test -p kiro-market-core --lib hash::tests::hash_artifact_returns_blake3_prefixed_hex_for_single_file`
Expected: FAIL — `cannot find function 'hash_artifact' in module 'super'`.

- [ ] **Step 3: Implement `hash_artifact`**

Add to `crates/kiro-market-core/src/hash.rs` (between `HashError` and the `tests` module):

```rust
/// Deterministic content hash over `relative_paths` rooted at `base`.
///
/// Sorts paths internally so input order does not affect the result.
/// For each path, feeds `relative_path_bytes || 0x00 || file_bytes || 0x00`
/// into the hasher. The NUL separators prevent file-rename collisions
/// (`a/b` + content `XY` would otherwise collide with `a` + content `b\0XY`).
///
/// Returns `"blake3:" + hex_encoded_digest`. The `"blake3:"` prefix lets a
/// future migration to a different algorithm be schema-compatible.
///
/// # Errors
///
/// Returns `HashError::ReadFailed` if any file in `relative_paths` cannot be
/// read.
pub fn hash_artifact(
    base: &Path,
    relative_paths: &[PathBuf],
) -> Result<String, HashError> {
    let mut sorted: Vec<&PathBuf> = relative_paths.iter().collect();
    sorted.sort();

    let mut hasher = blake3::Hasher::new();
    for rel in sorted {
        let abs = base.join(rel);
        let bytes = std::fs::read(&abs).map_err(|e| HashError::ReadFailed {
            path: abs.clone(),
            source: e,
        })?;
        // Use the platform-agnostic Unicode form of the relative path.
        // PathBuf::to_string_lossy is acceptable here because `relative_paths`
        // come from controlled sources (discovery layer or directory walks
        // we did ourselves) and we want the hash to be stable across OSes
        // for the same logical layout.
        let path_str = rel.to_string_lossy();
        hasher.update(path_str.as_bytes());
        hasher.update(&[0u8]);
        hasher.update(&bytes);
        hasher.update(&[0u8]);
    }
    let digest = hasher.finalize();
    Ok(format!("blake3:{}", hex::encode(digest.as_bytes())))
}
```

- [ ] **Step 4: Run test, verify it passes**

Run: `cargo test -p kiro-market-core --lib hash::tests::hash_artifact_returns_blake3_prefixed_hex_for_single_file`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/kiro-market-core/src/hash.rs
git commit -m "feat(core): implement hash_artifact happy path with blake3"
```

---

## Task 5: `hash_artifact` is order-independent (sort-internal)

**Files:**
- Modify: `crates/kiro-market-core/src/hash.rs` (add test only)

- [ ] **Step 1: Write the failing test**

Append to the `tests` module in `crates/kiro-market-core/src/hash.rs`:

```rust
#[test]
fn hash_artifact_is_order_independent() {
    let tmp = tempdir().unwrap();
    let base = tmp.path();
    fs::write(base.join("a.txt"), b"alpha").unwrap();
    fs::write(base.join("b.txt"), b"beta").unwrap();

    let h_ab = hash_artifact(
        base,
        &[PathBuf::from("a.txt"), PathBuf::from("b.txt")],
    )
    .unwrap();
    let h_ba = hash_artifact(
        base,
        &[PathBuf::from("b.txt"), PathBuf::from("a.txt")],
    )
    .unwrap();

    assert_eq!(h_ab, h_ba, "input order must not affect hash");
}
```

- [ ] **Step 2: Run test**

Run: `cargo test -p kiro-market-core --lib hash::tests::hash_artifact_is_order_independent`
Expected: PASS — implementation already sorts, so this is a regression-prevention test.

- [ ] **Step 3: Commit**

```bash
git add crates/kiro-market-core/src/hash.rs
git commit -m "test(core): hash_artifact is order-independent (regression guard)"
```

---

## Task 6: `hash_artifact` NUL separator prevents rename collisions

**Files:**
- Modify: `crates/kiro-market-core/src/hash.rs` (add test only)

- [ ] **Step 1: Write the test**

Append to the `tests` module:

```rust
#[test]
fn hash_artifact_distinguishes_rename_collisions() {
    // Without NUL separators, "a" + "bXY" would collide with "ab" + "XY"
    // (concatenation makes them identical streams). The NUL terminators
    // make them distinct.
    let tmp = tempdir().unwrap();
    let base = tmp.path();

    // Layout 1: file "a" with content "bXY"
    fs::write(base.join("a"), b"bXY").unwrap();
    let h1 = hash_artifact(base, &[PathBuf::from("a")]).unwrap();

    // Layout 2: file "ab" with content "XY" — different layout, would
    // collide without NUL separation.
    let tmp2 = tempdir().unwrap();
    let base2 = tmp2.path();
    fs::write(base2.join("ab"), b"XY").unwrap();
    let h2 = hash_artifact(base2, &[PathBuf::from("ab")]).unwrap();

    assert_ne!(
        h1, h2,
        "NUL separator must distinguish rename-collision layouts"
    );
}
```

- [ ] **Step 2: Run test**

Run: `cargo test -p kiro-market-core --lib hash::tests::hash_artifact_distinguishes_rename_collisions`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/kiro-market-core/src/hash.rs
git commit -m "test(core): hash_artifact NUL separator prevents rename collisions"
```

---

## Task 7: `hash_artifact` returns `ReadFailed` for missing files

**Files:**
- Modify: `crates/kiro-market-core/src/hash.rs` (add test only)

- [ ] **Step 1: Write the test**

Append to the `tests` module:

```rust
#[test]
fn hash_artifact_returns_read_failed_for_missing_file() {
    let tmp = tempdir().unwrap();
    let base = tmp.path();
    // Don't create the file.

    let err =
        hash_artifact(base, &[PathBuf::from("missing.txt")]).unwrap_err();

    match err {
        HashError::ReadFailed { path, source } => {
            assert!(path.ends_with("missing.txt"), "got: {}", path.display());
            assert_eq!(source.kind(), std::io::ErrorKind::NotFound);
        }
        other => panic!("expected ReadFailed, got: {other:?}"),
    }
}
```

- [ ] **Step 2: Run test**

Run: `cargo test -p kiro-market-core --lib hash::tests::hash_artifact_returns_read_failed_for_missing_file`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/kiro-market-core/src/hash.rs
git commit -m "test(core): hash_artifact reports ReadFailed for missing files"
```

---

## Task 8: `hash_artifact` distinguishes CRLF vs LF (no normalization)

**Files:**
- Modify: `crates/kiro-market-core/src/hash.rs` (add test only)

- [ ] **Step 1: Write the test**

Append to the `tests` module:

```rust
#[test]
fn hash_artifact_distinguishes_crlf_vs_lf() {
    let tmp_lf = tempdir().unwrap();
    fs::write(tmp_lf.path().join("a.txt"), b"line1\nline2\n").unwrap();
    let h_lf = hash_artifact(tmp_lf.path(), &[PathBuf::from("a.txt")]).unwrap();

    let tmp_crlf = tempdir().unwrap();
    fs::write(tmp_crlf.path().join("a.txt"), b"line1\r\nline2\r\n").unwrap();
    let h_crlf =
        hash_artifact(tmp_crlf.path(), &[PathBuf::from("a.txt")]).unwrap();

    assert_ne!(
        h_lf, h_crlf,
        "hash must NOT normalize line endings — different bytes are different content"
    );
}
```

- [ ] **Step 2: Run test**

Run: `cargo test -p kiro-market-core --lib hash::tests::hash_artifact_distinguishes_crlf_vs_lf`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/kiro-market-core/src/hash.rs
git commit -m "test(core): hash_artifact does not normalize line endings"
```

---

## Task 9: Implement `hash_dir_tree`

**Files:**
- Modify: `crates/kiro-market-core/src/hash.rs`

- [ ] **Step 1: Write the failing test**

Append to the `tests` module:

```rust
#[test]
fn hash_dir_tree_produces_stable_hash_over_tree() {
    let tmp = tempdir().unwrap();
    let root = tmp.path();
    fs::create_dir_all(root.join("sub")).unwrap();
    fs::write(root.join("top.md"), b"top content").unwrap();
    fs::write(root.join("sub/nested.md"), b"nested content").unwrap();

    let h1 = hash_dir_tree(root).unwrap();
    let h2 = hash_dir_tree(root).unwrap();

    assert_eq!(h1, h2, "same tree must produce same hash");
    assert!(h1.starts_with("blake3:"));
}

#[test]
fn hash_dir_tree_changes_when_content_changes() {
    let tmp = tempdir().unwrap();
    let root = tmp.path();
    fs::write(root.join("a.md"), b"v1").unwrap();
    let h1 = hash_dir_tree(root).unwrap();

    fs::write(root.join("a.md"), b"v2").unwrap();
    let h2 = hash_dir_tree(root).unwrap();

    assert_ne!(h1, h2, "content change must change the tree hash");
}

#[cfg(unix)]
#[test]
fn hash_dir_tree_skips_symlinks() {
    use std::os::unix::fs::symlink;

    let tmp = tempdir().unwrap();
    let root = tmp.path();
    fs::write(root.join("real.md"), b"real").unwrap();

    let outside = tempdir().unwrap();
    fs::write(outside.path().join("secret.md"), b"secret").unwrap();
    symlink(outside.path().join("secret.md"), root.join("link.md")).unwrap();

    // Hash with symlink present
    let h_with_link = hash_dir_tree(root).unwrap();

    // Remove the symlink and re-hash
    fs::remove_file(root.join("link.md")).unwrap();
    let h_without = hash_dir_tree(root).unwrap();

    assert_eq!(
        h_with_link, h_without,
        "symlinks must not contribute to the directory hash"
    );
}
```

- [ ] **Step 2: Run tests, verify they fail**

Run: `cargo test -p kiro-market-core --lib hash::tests::hash_dir_tree`
Expected: FAIL — `cannot find function 'hash_dir_tree' in module 'super'`.

- [ ] **Step 3: Implement `hash_dir_tree`**

Add to `crates/kiro-market-core/src/hash.rs` (between `hash_artifact` and the `tests` module):

```rust
/// Hash an entire directory tree by walking it and feeding each regular
/// file (non-recursively-following symlinks) through `hash_artifact`.
///
/// Symlinks are skipped — they would otherwise let a malicious source dir
/// pull arbitrary file contents into the hash via paths outside `root`.
///
/// # Errors
///
/// - `HashError::WalkFailed` if directory traversal fails.
/// - `HashError::ReadFailed` if a file in the tree cannot be read.
pub fn hash_dir_tree(root: &Path) -> Result<String, HashError> {
    let mut relative_paths: Vec<PathBuf> = Vec::new();
    walk_collect(root, root, &mut relative_paths)?;
    hash_artifact(root, &relative_paths)
}

/// Recursive helper for `hash_dir_tree`. Collects relative paths of regular
/// files (not symlinks, not directories) under `current` into `out`.
fn walk_collect(
    root: &Path,
    current: &Path,
    out: &mut Vec<PathBuf>,
) -> Result<(), HashError> {
    let entries = std::fs::read_dir(current).map_err(|e| HashError::WalkFailed {
        path: current.to_path_buf(),
        source: e,
    })?;
    for entry in entries {
        let entry = entry.map_err(|e| HashError::WalkFailed {
            path: current.to_path_buf(),
            source: e,
        })?;
        let path = entry.path();
        // Use symlink_metadata so we don't follow symlinks.
        let md = std::fs::symlink_metadata(&path).map_err(|e| {
            HashError::WalkFailed {
                path: path.clone(),
                source: e,
            }
        })?;
        let ft = md.file_type();
        if ft.is_symlink() {
            continue;
        }
        if ft.is_dir() {
            walk_collect(root, &path, out)?;
        } else if ft.is_file() {
            // Strip the root prefix to get a relative path.
            let rel = path
                .strip_prefix(root)
                .expect("walk_collect only produces paths under root")
                .to_path_buf();
            out.push(rel);
        }
        // Skip other file types (sockets, FIFOs, devices) silently.
    }
    Ok(())
}
```

- [ ] **Step 4: Run tests, verify they pass**

Run: `cargo test -p kiro-market-core --lib hash::tests`
Expected: All hash::tests tests PASS (including the new `hash_dir_tree` ones).

- [ ] **Step 5: Commit**

```bash
git add crates/kiro-market-core/src/hash.rs
git commit -m "feat(core): implement hash_dir_tree with symlink skip"
```

---

## Task 10: Add hash fields to `InstalledSkillMeta` (backward-compat)

**Files:**
- Modify: `crates/kiro-market-core/src/project.rs` (around line 32)

- [ ] **Step 1: Write the backward-compat test**

Find the existing `#[cfg(test)] mod tests { ... }` block at the bottom of `crates/kiro-market-core/src/project.rs`. Append a new test:

```rust
#[test]
fn installed_skill_meta_loads_legacy_json_without_hash_fields() {
    // Old tracking files (pre-Stage-1) lack source_hash / installed_hash.
    // The new schema must deserialize them with both fields = None.
    let legacy = br#"{
        "marketplace": "m",
        "plugin": "p",
        "version": "1.0.0",
        "installed_at": "2026-01-01T00:00:00Z"
    }"#;

    let meta: InstalledSkillMeta = serde_json::from_slice(legacy).unwrap();

    assert_eq!(meta.marketplace, "m");
    assert_eq!(meta.plugin, "p");
    assert!(meta.source_hash.is_none());
    assert!(meta.installed_hash.is_none());
}
```

- [ ] **Step 2: Run test, verify it fails**

Run: `cargo test -p kiro-market-core --lib installed_skill_meta_loads_legacy_json_without_hash_fields`
Expected: FAIL — `no field 'source_hash' on type InstalledSkillMeta`.

- [ ] **Step 3: Add fields to `InstalledSkillMeta`**

Find `pub struct InstalledSkillMeta` (around line 32) and add two fields after `installed_at`:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledSkillMeta {
    pub marketplace: String,
    pub plugin: String,
    pub version: Option<String>,
    pub installed_at: DateTime<Utc>,

    /// Tree-hash of the skill source as it existed in the marketplace at
    /// install time. `None` for entries written before Stage 1 of the
    /// native-kiro-import work landed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_hash: Option<String>,

    /// Tree-hash of the skill as it was copied into the project. `None`
    /// for entries written before Stage 1 landed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installed_hash: Option<String>,
}
```

- [ ] **Step 4: Run test, verify it passes**

Run: `cargo test -p kiro-market-core --lib installed_skill_meta_loads_legacy_json_without_hash_fields`
Expected: PASS.

- [ ] **Step 5: Run full crate tests, fix any breakage**

Run: `cargo test -p kiro-market-core`
Expected: All tests pass. If any existing test constructed an `InstalledSkillMeta` without the new fields, the test still compiles (the fields default to `None`). If a test does `meta == InstalledSkillMeta { ... }` literally, you may need to add `source_hash: None, installed_hash: None,` — fix any such test by adding the explicit `None` defaults.

- [ ] **Step 6: Commit**

```bash
git add crates/kiro-market-core/src/project.rs
git commit -m "feat(core): add source_hash + installed_hash to InstalledSkillMeta"
```

---

## Task 11: Add hash fields to `InstalledAgentMeta` (backward-compat)

**Files:**
- Modify: `crates/kiro-market-core/src/project.rs` (around line 52)

- [ ] **Step 1: Write the backward-compat test**

Append to the `#[cfg(test)] mod tests` block:

```rust
#[test]
fn installed_agent_meta_loads_legacy_json_without_hash_fields() {
    let legacy = br#"{
        "marketplace": "m",
        "plugin": "p",
        "version": "0.1.0",
        "installed_at": "2026-01-01T00:00:00Z",
        "dialect": "claude"
    }"#;

    let meta: InstalledAgentMeta = serde_json::from_slice(legacy).unwrap();

    assert_eq!(meta.dialect, AgentDialect::Claude);
    assert!(meta.source_hash.is_none());
    assert!(meta.installed_hash.is_none());
}
```

- [ ] **Step 2: Run test, verify it fails**

Run: `cargo test -p kiro-market-core --lib installed_agent_meta_loads_legacy_json_without_hash_fields`
Expected: FAIL — `no field 'source_hash'`.

- [ ] **Step 3: Add fields to `InstalledAgentMeta`**

Find `pub struct InstalledAgentMeta` (around line 52) and add the same two fields after `dialect`:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledAgentMeta {
    pub marketplace: String,
    pub plugin: String,
    pub version: Option<String>,
    pub installed_at: DateTime<Utc>,
    pub dialect: AgentDialect,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_hash: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installed_hash: Option<String>,
}
```

- [ ] **Step 4: Run test, verify it passes**

Run: `cargo test -p kiro-market-core --lib installed_agent_meta_loads_legacy_json_without_hash_fields`
Expected: PASS.

- [ ] **Step 5: Run full crate tests, fix any breakage**

Run: `cargo test -p kiro-market-core`
Expected: All tests pass. Same struct-literal fix-up rule as Task 10 if any test constructs `InstalledAgentMeta` directly.

- [ ] **Step 6: Commit**

```bash
git add crates/kiro-market-core/src/project.rs
git commit -m "feat(core): add source_hash + installed_hash to InstalledAgentMeta"
```

---

## Task 12: `install_skill_from_dir` populates source + installed hashes

**Files:**
- Modify: `crates/kiro-market-core/src/project.rs` (around line 331 + line 669)

- [ ] **Step 1: Write the test**

Append to the `#[cfg(test)] mod tests` block:

```rust
#[test]
fn install_skill_from_dir_populates_source_and_installed_hashes() {
    let tmp = tempdir().unwrap();
    let project = KiroProject::new(tmp.path()).unwrap();

    // Create a tiny source skill directory.
    let skill_src = tmp.path().join("source");
    fs::create_dir_all(&skill_src).unwrap();
    fs::write(skill_src.join("SKILL.md"), b"# test skill\n\nbody").unwrap();

    let meta = InstalledSkillMeta {
        marketplace: "m".into(),
        plugin: "p".into(),
        version: Some("1.0.0".into()),
        installed_at: chrono::Utc::now(),
        source_hash: None,
        installed_hash: None,
    };

    project.install_skill_from_dir("test", &skill_src, meta).unwrap();

    let installed = project.load_installed_skills().unwrap();
    let entry = installed.skills.get("test").expect("entry persisted");

    let src_hash = entry.source_hash.as_ref().expect("source_hash populated");
    let inst_hash = entry
        .installed_hash
        .as_ref()
        .expect("installed_hash populated");

    assert!(src_hash.starts_with("blake3:"));
    assert!(inst_hash.starts_with("blake3:"));
    // Source and installed contents are identical (we just copied), so the
    // hashes match.
    assert_eq!(src_hash, inst_hash);
}
```

- [ ] **Step 2: Run test, verify it fails**

Run: `cargo test -p kiro-market-core --lib install_skill_from_dir_populates_source_and_installed_hashes`
Expected: FAIL — the assertion `source_hash populated` panics because the meta is persisted with the `None` values the caller passed in.

- [ ] **Step 3: Update `install_skill_from_dir` to compute hashes**

Replace `install_skill_from_dir` (around line 331) with:

```rust
pub fn install_skill_from_dir(
    &self,
    name: &str,
    source_dir: &Path,
    meta: InstalledSkillMeta,
) -> crate::error::Result<()> {
    validation::validate_name(name)?;
    let source_hash = crate::hash::hash_dir_tree(source_dir)?;
    self.write_skill_dir(name, source_dir, meta, false, source_hash)
}
```

Replace `install_skill_from_dir_force` similarly:

```rust
pub fn install_skill_from_dir_force(
    &self,
    name: &str,
    source_dir: &Path,
    meta: InstalledSkillMeta,
) -> crate::error::Result<()> {
    validation::validate_name(name)?;
    let source_hash = crate::hash::hash_dir_tree(source_dir)?;
    self.write_skill_dir(name, source_dir, meta, true, source_hash)
}
```

- [ ] **Step 4: Update `write_skill_dir` to accept and persist hashes**

Find `fn write_skill_dir` (around line 669). Update its signature to accept `source_hash: String`:

```rust
fn write_skill_dir(
    &self,
    name: &str,
    source_dir: &Path,
    mut meta: InstalledSkillMeta,
    force: bool,
    source_hash: String,
) -> crate::error::Result<()> {
```

Inside `write_skill_dir`, AFTER the rename-into-place succeeds (and before the tracking write), compute `installed_hash` from the destination directory and stuff both into `meta`:

```rust
// (existing code that places content into self.skills_dir().join(name))

// Compute installed_hash AFTER the rename-into-place so we hash the bytes
// that actually landed in the project.
let installed_dir = self.skills_dir().join(name);
let installed_hash = crate::hash::hash_dir_tree(&installed_dir)?;

meta.source_hash = Some(source_hash);
meta.installed_hash = Some(installed_hash);

// (existing code that writes the tracking entry)
```

The exact placement depends on the existing structure of `write_skill_dir` — find the `installed.skills.insert(name.to_string(), meta);` line (or equivalent) and insert the hash assignments immediately before it.

- [ ] **Step 5: Run test, verify it passes**

Run: `cargo test -p kiro-market-core --lib install_skill_from_dir_populates_source_and_installed_hashes`
Expected: PASS.

- [ ] **Step 6: Run full crate tests, fix any breakage**

Run: `cargo test -p kiro-market-core`
Expected: All tests pass. Existing `install_skill_from_dir_*` tests still work because they pass `meta` with `source_hash: None`, and the install layer overrides those Nones with computed hashes — assertions that don't look at hash fields are unaffected.

- [ ] **Step 7: Commit**

```bash
git add crates/kiro-market-core/src/project.rs
git commit -m "feat(core): populate source/installed hashes during skill install"
```

---

## Task 13: `install_agent_inner` (translated path) populates hashes

**Files:**
- Modify: `crates/kiro-market-core/src/project.rs` (around line 470)

- [ ] **Step 1: Write the test**

Append to the `#[cfg(test)] mod tests` block. The existing test helpers like `write_agent` and `sample_agent_meta` (search for them at `project.rs:877+`) are reusable. Pattern:

```rust
#[test]
fn install_agent_translated_populates_source_and_installed_hashes() {
    let tmp = tempdir().unwrap();
    let project = KiroProject::new(tmp.path()).unwrap();

    let source_md = write_agent(tmp.path(), "rev", "You are a reviewer.");
    let def = crate::agent::AgentDefinition {
        name: "rev".into(),
        description: None,
        prompt_body: "You are a reviewer.".into(),
        model: None,
        source_tools: vec![],
        mcp_servers: std::collections::BTreeMap::new(),
        dialect: crate::agent::AgentDialect::Claude,
    };
    let mapped: Vec<crate::agent::tools::MappedTool> = vec![];
    let mut meta = sample_agent_meta();
    meta.source_hash = None;
    meta.installed_hash = None;

    project
        .install_agent(&def, &mapped, meta)
        .expect("install succeeds");

    let installed = project.load_installed_agents().unwrap();
    let entry = installed.agents.get("rev").expect("entry persisted");

    let src = entry.source_hash.as_ref().expect("source_hash set");
    let inst = entry.installed_hash.as_ref().expect("installed_hash set");
    assert!(src.starts_with("blake3:"));
    assert!(inst.starts_with("blake3:"));
    // Translated path: source bytes (raw .md) differ from installed bytes
    // (emitted .json + prompt body), so the two hashes ARE different here
    // — unlike the skill case where source and dest are identical copies.
    assert_ne!(src, inst);

    // Sanity: re-hashing the source file directly matches the recorded
    // source_hash.
    let recomputed_src = crate::hash::hash_artifact(
        source_md.parent().unwrap(),
        &[std::path::PathBuf::from(
            source_md.file_name().unwrap(),
        )],
    )
    .unwrap();
    assert_eq!(src, &recomputed_src);
}
```

- [ ] **Step 2: Run test, verify it fails**

Run: `cargo test -p kiro-market-core --lib install_agent_translated_populates_source_and_installed_hashes`
Expected: FAIL — `expected source_hash set` panics.

- [ ] **Step 3: Update `install_agent_inner` to compute hashes**

Find `fn install_agent_inner` (around line 470). The function currently takes `def: &AgentDefinition` and writes `<name>.json` + `prompts/<name>.md`. It needs:

1. **Source hash**: hash the file the agent was parsed from. The current `AgentDefinition` doesn't carry a source path — but `install_agent` is called from the service layer which DOES know the source path. Add a new optional parameter to `install_agent` and `install_agent_force` for the source path:

```rust
pub fn install_agent(
    &self,
    def: &AgentDefinition,
    mapped_tools: &[MappedTool],
    meta: InstalledAgentMeta,
    source_path: Option<&Path>,  // NEW
) -> crate::error::Result<()> {
    self.install_agent_inner(def, mapped_tools, meta, false, source_path)
}

pub fn install_agent_force(
    &self,
    def: &AgentDefinition,
    mapped_tools: &[MappedTool],
    meta: InstalledAgentMeta,
    source_path: Option<&Path>,  // NEW
) -> crate::error::Result<()> {
    self.install_agent_inner(def, mapped_tools, meta, true, source_path)
}
```

(The `Option<&Path>` is for backward-compat: callers that don't have a source path — e.g., synthetic test agents — pass `None` and `source_hash` stays `None`.)

2. **Update `install_agent_inner` signature**:

```rust
fn install_agent_inner(
    &self,
    def: &AgentDefinition,
    mapped_tools: &[MappedTool],
    mut meta: InstalledAgentMeta,
    force: bool,
    source_path: Option<&Path>,
) -> crate::error::Result<()> {
```

3. **Compute source_hash before the file lock acquisition** (`source_path.is_some()` only):

Insert near the top of `install_agent_inner` (before the `with_file_lock` block):

```rust
let source_hash = match source_path {
    Some(p) => {
        let parent = p.parent().ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("source path `{}` has no parent dir", p.display()),
            )
        })?;
        let filename = p.file_name().ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("source path `{}` has no file name", p.display()),
            )
        })?;
        Some(crate::hash::hash_artifact(
            parent,
            &[std::path::PathBuf::from(filename)],
        )?)
    }
    None => None,
};
```

4. **Compute installed_hash AFTER the renames** (after both `rename(staging_json, json_target)` and `rename(staging_prompt, prompt_target)` succeed, before the tracking write):

```rust
// Compute installed_hash over the two files we just placed.
let agents_root = self.agents_dir();
let json_rel = std::path::PathBuf::from(format!("{}.json", def.name));
let prompt_rel = std::path::PathBuf::from(format!("prompts/{}.md", def.name));
let installed_hash = crate::hash::hash_artifact(
    &agents_root,
    &[json_rel, prompt_rel],
)?;

meta.source_hash = source_hash;
meta.installed_hash = Some(installed_hash);

// (existing code that calls installed.agents.insert(...) follows)
```

5. **Update all `self.install_agent(...)` and `install_agent_force(...)` callers** in tests AND in the service layer (`crates/kiro-market-core/src/service/`) to pass the source path. For tests that synthesize an `AgentDefinition` from thin air, pass `None`. For service callers that have the source path, pass `Some(&path)`.

A quick way to find callers: `grep -rn "install_agent\b\|install_agent_force\b" crates/`.

- [ ] **Step 4: Run test, verify it passes**

Run: `cargo test -p kiro-market-core --lib install_agent_translated_populates_source_and_installed_hashes`
Expected: PASS.

- [ ] **Step 5: Run full workspace tests, fix any breakage**

Run: `cargo test --workspace`
Expected: All tests pass after updating callers in step 3.5.

- [ ] **Step 6: Commit**

```bash
git add crates/kiro-market-core/src/project.rs crates/kiro-market-core/src/service/ crates/kiro-market/src/
git commit -m "feat(core): populate source/installed hashes during translated agent install"
```

---

## Task 14: Final verification — full test suite + clippy + fmt

**Files:** none (verification only)

- [ ] **Step 1: Run the full test suite**

Run: `cargo test --workspace`
Expected: All tests pass.

- [ ] **Step 2: Run clippy with the project's strict settings**

Run: `cargo clippy --workspace --tests -- -D warnings`
Expected: No warnings. If `clippy::pedantic` flags something in the new hash module (e.g., `must_use_candidate`), fix it inline:
- Mark `hash_artifact` and `hash_dir_tree` `#[must_use]` if pedantic asks.
- Use `&Path` instead of `PathBuf` parameters where clippy suggests.

- [ ] **Step 3: Run fmt check**

Run: `cargo fmt --all --check`
Expected: No diff. (The PostToolUse hook should have auto-formatted, but verify.)

- [ ] **Step 4: Commit any clippy/fmt fixes (if any)**

```bash
# Only if the previous steps required edits:
git add -u
git commit -m "style: address clippy + fmt for hash module"
```

If no edits were needed, skip this step.

---

## Out-of-Plan Notes for Implementer

**Why no Tauri or CLI changes in this stage.** Stage 1 is pure foundational work. The hash fields are populated in tracking but no UI surface consumes them yet. A future drift-check command (out of scope for this design) is what reads them. CLI output and Tauri commands are unchanged.

**Why `Option<&Path>` on `install_agent` instead of required `&Path`.** The translated install path always has a source path (the parsed .md file). The native install path (Stage 2) has it too. But test fixtures sometimes synthesize an `AgentDefinition` from thin air — `Option` keeps those tests working without a fake source file.

**Why `hash_dir_tree` for skills but `hash_artifact` for agents.** Skills are directory trees (the install copies wholesale, source ≡ destination). Agents are single-file or two-file artifacts where the source list is known in advance — `hash_artifact` with an explicit path list is faster and avoids walking the agents directory (which has files from many other plugins).

**Why blake3 (not SHA-256).** Per the design doc: ~10× faster on small files, supports keyed-MAC for future signed manifests. The `"blake3:"` prefix on every hash makes a future migration to a different algorithm a schema-compatible change (deserializers can match on the prefix to pick the verifier).

**TOCTOU note.** Between `hash_dir_tree(source_dir)` and the actual file copy, the source could in principle change. We accept this race: the user installed what was on disk at copy time, and the `installed_hash` (computed AFTER the copy) is the source of truth for what landed. If they differ, the marketplace cache moved under us — surfacing this is future drift-check work, not Stage 1.

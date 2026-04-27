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
#[non_exhaustive]
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
pub fn hash_artifact(base: &Path, relative_paths: &[PathBuf]) -> Result<String, HashError> {
    let mut sorted: Vec<&PathBuf> = relative_paths.iter().collect();
    sorted.sort();

    let mut hasher = blake3::Hasher::new();
    for rel in sorted {
        let abs = base.join(rel);
        // Defense-in-depth: re-check the file type immediately before reading
        // to close the TOCTOU window between `walk_collect` (which checks
        // `symlink_metadata` and skips symlinks) and this read (which would
        // otherwise follow a symlink if an attacker swapped a regular file
        // for a symlink between the two steps). Matches the security
        // guarantee stated in this module's top-level doc comment.
        let md = std::fs::symlink_metadata(&abs).map_err(|e| HashError::ReadFailed {
            path: abs.clone(),
            source: e,
        })?;
        // Use is_reparse_or_symlink (not is_symlink) so Windows directory
        // junctions and other reparse-point flavors are also caught — a
        // junction substituted between walk and read would otherwise slip
        // past `is_symlink()` (which returns false for
        // IO_REPARSE_TAG_MOUNT_POINT) and let the read traverse outside
        // the install boundary.
        if crate::platform::is_reparse_or_symlink(&md) {
            return Err(HashError::ReadFailed {
                path: abs.clone(),
                source: std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "symlink or reparse point appeared between walk and read",
                ),
            });
        }
        let bytes = std::fs::read(&abs).map_err(|e| HashError::ReadFailed {
            path: abs.clone(),
            source: e,
        })?;
        // Use the platform-agnostic forward-slash form of the relative
        // path. `PathBuf::to_string_lossy()` returns native separators
        // (`\` on Windows, `/` on Unix); we normalize to `/` so the hash
        // captures the logical layout, not the host's path conventions.
        // `relative_paths` come from controlled sources (discovery layer
        // or directory walks we did ourselves), so non-UTF-8 handling via
        // `to_string_lossy` is acceptable.
        let path_str = rel.to_string_lossy();
        let normalized = path_str.replace('\\', "/");
        hasher.update(normalized.as_bytes());
        hasher.update(&[0u8]);
        hasher.update(&bytes);
        hasher.update(&[0u8]);
    }
    let digest = hasher.finalize();
    Ok(format!("blake3:{}", hex::encode(digest.as_bytes())))
}

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
fn walk_collect(root: &Path, current: &Path, out: &mut Vec<PathBuf>) -> Result<(), HashError> {
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
        let md = std::fs::symlink_metadata(&path).map_err(|e| HashError::WalkFailed {
            path: path.clone(),
            source: e,
        })?;
        // is_reparse_or_symlink catches Windows directory junctions
        // (which is_symlink misses) — a junction inside the source
        // tree would otherwise let the walker traverse outside `root`.
        if crate::platform::is_reparse_or_symlink(&md) {
            continue;
        }
        let ft = md.file_type();
        if ft.is_dir() {
            walk_collect(root, &path, out)?;
        } else if ft.is_file() {
            // Strip the root prefix to get a relative path.
            // `path` is always under `root` because `read_dir(current)` returns
            // entries inside `current`, which itself was reached by recursive descent
            // from `root`. This invariant cannot be violated by directory traversal alone.
            // We still propagate as WalkFailed rather than panicking, per CLAUDE.md
            // zero-tolerance on `.expect()` in production code.
            let rel = path
                .strip_prefix(root)
                .map_err(|_| HashError::WalkFailed {
                    path: path.clone(),
                    source: std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "walk produced a path outside root",
                    ),
                })?
                .to_path_buf();
            out.push(rel);
        }
        // Skip other file types (sockets, FIFOs, devices) silently.
    }
    Ok(())
}

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

    #[test]
    fn hash_artifact_is_order_independent() {
        let tmp = tempdir().unwrap();
        let base = tmp.path();
        fs::write(base.join("a.txt"), b"alpha").unwrap();
        fs::write(base.join("b.txt"), b"beta").unwrap();

        let h_ab = hash_artifact(base, &[PathBuf::from("a.txt"), PathBuf::from("b.txt")]).unwrap();
        let h_ba = hash_artifact(base, &[PathBuf::from("b.txt"), PathBuf::from("a.txt")]).unwrap();

        assert_eq!(h_ab, h_ba, "input order must not affect hash");
    }

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

    #[test]
    fn hash_artifact_returns_read_failed_for_missing_file() {
        let tmp = tempdir().unwrap();
        let base = tmp.path();
        // Don't create the file.

        let err = hash_artifact(base, &[PathBuf::from("missing.txt")]).unwrap_err();

        match err {
            HashError::ReadFailed { path, source } => {
                assert!(path.ends_with("missing.txt"), "got: {}", path.display());
                assert_eq!(source.kind(), std::io::ErrorKind::NotFound);
            }
            other => panic!("expected ReadFailed, got: {other:?}"),
        }
    }

    #[test]
    fn hash_artifact_distinguishes_crlf_vs_lf() {
        let tmp_lf = tempdir().unwrap();
        fs::write(tmp_lf.path().join("a.txt"), b"line1\nline2\n").unwrap();
        let h_lf = hash_artifact(tmp_lf.path(), &[PathBuf::from("a.txt")]).unwrap();

        let tmp_crlf = tempdir().unwrap();
        fs::write(tmp_crlf.path().join("a.txt"), b"line1\r\nline2\r\n").unwrap();
        let h_crlf = hash_artifact(tmp_crlf.path(), &[PathBuf::from("a.txt")]).unwrap();

        assert_ne!(
            h_lf, h_crlf,
            "hash must NOT normalize line endings — different bytes are different content"
        );
    }

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

    #[test]
    fn hash_artifact_uses_forward_slash_canonical_path_form() {
        // Golden-value test: pins the bytes fed into blake3 to the
        // forward-slash canonical form of the relative path. A Windows
        // regression that re-introduces `rel.to_string_lossy()` without
        // normalization would flip to backslash and this test would
        // fail on Windows. The expected value is computed by hand to
        // make the invariant explicit.
        let tmp = tempdir().unwrap();
        let base = tmp.path();
        std::fs::create_dir_all(base.join("sub")).unwrap();
        fs::write(base.join("sub/nested.md"), b"content").unwrap();

        let h = hash_artifact(base, &[PathBuf::from("sub/nested.md")]).unwrap();

        let mut hasher = blake3::Hasher::new();
        hasher.update(b"sub/nested.md");
        hasher.update(&[0u8]);
        hasher.update(b"content");
        hasher.update(&[0u8]);
        let expected = format!("blake3:{}", hex::encode(hasher.finalize().as_bytes()));

        assert_eq!(
            h, expected,
            "hash must use forward-slash canonical path form in the hasher input"
        );
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
}

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
}

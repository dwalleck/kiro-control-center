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

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Canonical content hash for installed artifacts.
///
/// Stored on disk as the string `"blake3:" + 64 ASCII hex chars`. The
/// type wraps a `String` with a private inner field so the only ways
/// to construct one are (a) `BlakeHash::new` (validates the format),
/// (b) the artifact-hash producers in this module ([`hash_artifact`]
/// / [`hash_dir_tree`], which build the canonical form by
/// construction), or (c) the `placeholder` constructor used during
/// install scaffolding (and in test fixtures that don't care about the
/// actual content).
///
/// The `Deserialize` impl routes through `new`, so a tracking file
/// containing `"source_hash": ""` (or any other malformed value) fails
/// to load instead of being silently accepted as a sentinel.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(transparent)]
pub struct BlakeHash(String);

const BLAKE_HASH_PREFIX: &str = "blake3:";
const BLAKE_HASH_HEX_LEN: usize = 64;
/// Inner string for [`BlakeHash::placeholder`]. Centralised so a future
/// change to `BLAKE_HASH_PREFIX` / `BLAKE_HASH_HEX_LEN` can't desync the
/// placeholder from `validate_blake_hash` — the
/// `placeholder_satisfies_blake_hash_validation` test pins this.
const BLAKE_HASH_PLACEHOLDER_HEX: &str =
    "0000000000000000000000000000000000000000000000000000000000000000";

impl BlakeHash {
    /// Construct a `BlakeHash` from a string, validating the canonical
    /// `"blake3:" + 64 hex` format. The hex payload is normalised to
    /// lowercase so two `BlakeHash` values that represent the same
    /// content always compare equal, even if one was authored in
    /// uppercase (e.g. by a hand-edited tracking file or an external
    /// producer). Lowercase is canonical because `blake3::Hash::Display`
    /// and `from_blake3_digest` both emit lowercase.
    ///
    /// # Errors
    ///
    /// Returns [`BlakeHashParseError`] if `value` doesn't start with
    /// `"blake3:"`, isn't exactly 64 hex chars after the prefix, or
    /// contains non-ASCII-hex characters.
    pub fn new(value: impl Into<String>) -> Result<Self, BlakeHashParseError> {
        let mut value = value.into();
        validate_blake_hash(&value)?;
        // Validation accepts mixed case; equality is case-sensitive on
        // the inner String. Normalise post-validation so the canonical
        // form is the only representation that ever lands in `Self`.
        value.make_ascii_lowercase();
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Build a `BlakeHash` directly from a finalized blake3 digest.
    /// Crate-internal because the caller holds the raw digest bytes —
    /// no string parsing involved, format invariant holds by
    /// construction. Uses `blake3::Hash`'s `Display` impl (lowercase
    /// hex) so the produced value is in canonical form and round-trips
    /// through `Deserialize` without re-normalisation.
    pub(crate) fn from_blake3_digest(digest: &blake3::Hash) -> Self {
        Self(format!("{BLAKE_HASH_PREFIX}{digest}"))
    }
}

impl std::fmt::Display for BlakeHash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for BlakeHash {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Self::new(s).map_err(serde::de::Error::custom)
    }
}

/// Production visibility for [`BlakeHash::placeholder`]. In-crate scaffolding
/// sites (skill/agent/companion install paths) construct the meta value
/// before the real hash is known, then overwrite both hash fields against
/// the staged content before committing to tracking. Keeping this
/// `pub(crate)` in production builds prevents external crates from
/// minting a content-meaningless hash that could later reach disk.
#[cfg(not(any(test, feature = "test-support")))]
impl BlakeHash {
    pub(crate) fn placeholder() -> Self {
        Self(format!("{BLAKE_HASH_PREFIX}{BLAKE_HASH_PLACEHOLDER_HEX}"))
    }
}

/// Test-build visibility for [`BlakeHash::placeholder`]. The Tauri crate's
/// dev-dependencies activate `test-support`, so test fixtures across
/// crates can build meta values without rigging up a real source file.
/// The placeholder value is well-formed under the `BlakeHash` invariant
/// (it round-trips through `Deserialize`), so leaking one into a
/// production tracking file would cause loud content-drift on the next
/// install rather than silent corruption — but production code should
/// still go through the in-crate scaffolding sites that overwrite
/// before persistence.
#[cfg(any(test, feature = "test-support"))]
impl BlakeHash {
    /// A canonical-but-content-meaningless `BlakeHash` for two patterns:
    ///
    /// 1. **Install scaffolding**: an `InstalledSkillMeta` /
    ///    `InstalledAgentMeta` / `InstalledNativeCompanionsMeta` is
    ///    constructed before its hashes are known, then the hash
    ///    fields are overwritten against the staged content before
    ///    the entry is committed to tracking. The placeholder is
    ///    well-formed (it will round-trip through `Deserialize`) but
    ///    must never reach disk — the install-finalisation step is
    ///    responsible for the overwrite. **Do not branch on equality
    ///    with `placeholder()` to detect "unset"** — the all-zeros
    ///    value is computationally indistinguishable from a real hash.
    /// 2. **Test fixtures** that synthesise a meta value from thin air
    ///    and don't exercise drift detection. Tests asserting drift
    ///    behaviour should compute a real hash via [`hash_artifact`]
    ///    instead.
    #[must_use]
    pub fn placeholder() -> Self {
        Self(format!("{BLAKE_HASH_PREFIX}{BLAKE_HASH_PLACEHOLDER_HEX}"))
    }
}

/// Validation failure for [`BlakeHash::new`].
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("invalid blake3 content hash `{value}`: {reason}")]
pub struct BlakeHashParseError {
    pub value: String,
    pub reason: &'static str,
}

fn validate_blake_hash(value: &str) -> Result<(), BlakeHashParseError> {
    let Some(hex) = value.strip_prefix(BLAKE_HASH_PREFIX) else {
        return Err(BlakeHashParseError {
            value: value.to_owned(),
            reason: "missing `blake3:` prefix",
        });
    };
    if hex.len() != BLAKE_HASH_HEX_LEN {
        return Err(BlakeHashParseError {
            value: value.to_owned(),
            reason: "hex payload must be exactly 64 chars",
        });
    }
    if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(BlakeHashParseError {
            value: value.to_owned(),
            reason: "hex payload must be ASCII hex digits",
        });
    }
    Ok(())
}

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
/// Returns a [`BlakeHash`] in canonical `"blake3:" + hex_encoded_digest` form.
/// The `"blake3:"` prefix lets a future migration to a different algorithm
/// be schema-compatible.
///
/// # Errors
///
/// Returns `HashError::ReadFailed` if any file in `relative_paths` cannot be
/// read.
pub fn hash_artifact(base: &Path, relative_paths: &[PathBuf]) -> Result<BlakeHash, HashError> {
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
    Ok(BlakeHash::from_blake3_digest(&hasher.finalize()))
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
pub fn hash_dir_tree(root: &Path) -> Result<BlakeHash, HashError> {
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

        assert!(h.as_str().starts_with("blake3:"), "got: {h}");
        // 32-byte blake3 → 64 hex chars + "blake3:" prefix = 71 chars.
        assert_eq!(h.as_str().len(), "blake3:".len() + 64);
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
        assert!(h1.as_str().starts_with("blake3:"));
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
            h.as_str(),
            expected,
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

    #[test]
    fn blake_hash_new_accepts_canonical_form() {
        let canonical = format!("blake3:{}", "a".repeat(64));
        let h = BlakeHash::new(canonical.clone()).expect("canonical form must parse");
        assert_eq!(h.as_str(), canonical);
    }

    #[test]
    fn blake_hash_new_rejects_empty_string() {
        let err = BlakeHash::new(String::new()).expect_err("empty string must be rejected");
        assert_eq!(err.reason, "missing `blake3:` prefix");
    }

    #[test]
    fn blake_hash_new_rejects_missing_prefix() {
        let err = BlakeHash::new("a".repeat(64)).expect_err("missing prefix must be rejected");
        assert_eq!(err.reason, "missing `blake3:` prefix");
    }

    #[test]
    fn blake_hash_new_rejects_short_payload() {
        let err = BlakeHash::new(format!("blake3:{}", "a".repeat(63)))
            .expect_err("63-char payload must be rejected");
        assert_eq!(err.reason, "hex payload must be exactly 64 chars");
    }

    #[test]
    fn blake_hash_new_rejects_long_payload() {
        let err = BlakeHash::new(format!("blake3:{}", "a".repeat(65)))
            .expect_err("65-char payload must be rejected");
        assert_eq!(err.reason, "hex payload must be exactly 64 chars");
    }

    #[test]
    fn blake_hash_new_rejects_non_hex_chars() {
        // 'g' is not a hex digit; everything else is.
        let mut payload = "a".repeat(63);
        payload.push('g');
        let err =
            BlakeHash::new(format!("blake3:{payload}")).expect_err("non-hex char must be rejected");
        assert_eq!(err.reason, "hex payload must be ASCII hex digits");
    }

    #[test]
    fn blake_hash_serializes_as_plain_string() {
        let h = BlakeHash::placeholder();
        let json = serde_json::to_string(&h).unwrap();
        assert_eq!(json, format!("\"{}\"", h.as_str()));
    }

    #[test]
    fn blake_hash_deserialize_routes_through_new() {
        // A tracking file with `"source_hash": ""` must fail to load
        // rather than being silently accepted as a sentinel.
        let err = serde_json::from_str::<BlakeHash>("\"\"")
            .expect_err("empty string must fail deserialize");
        assert!(
            err.to_string().contains("missing `blake3:` prefix"),
            "deserialize error must surface the validation reason; got: {err}"
        );
    }

    #[test]
    fn blake_hash_deserialize_accepts_canonical_form() {
        let canonical = format!("blake3:{}", "f".repeat(64));
        let json = format!("\"{canonical}\"");
        let h: BlakeHash = serde_json::from_str(&json).unwrap();
        assert_eq!(h.as_str(), canonical);
    }

    /// Two `BlakeHash` values authored in different cases but representing
    /// the same content must compare equal. `validate_blake_hash` accepts
    /// mixed case, so without lowercase normalisation in `new` an
    /// uppercase-authored entry from a hand-edited tracking file would
    /// fail to match the lowercase output of `from_blake3_digest`,
    /// producing spurious drift on every detection pass.
    #[test]
    fn blake_hash_normalises_case_so_equality_is_canonical() {
        let upper = BlakeHash::new(format!("blake3:{}", "ABCDEF1234567890".repeat(4))).unwrap();
        let lower = BlakeHash::new(format!("blake3:{}", "abcdef1234567890".repeat(4))).unwrap();
        assert_eq!(upper, lower, "case-different inputs must compare equal");
        assert_eq!(
            upper.as_str(),
            format!("blake3:{}", "abcdef1234567890".repeat(4)),
            "stored form must be lowercase canonical"
        );
    }

    /// `placeholder()` and `validate_blake_hash` must agree on the format —
    /// otherwise a future change to `BLAKE_HASH_PREFIX` /
    /// `BLAKE_HASH_HEX_LEN` could desync the placeholder constant from the
    /// validator without any caller noticing (until a `Deserialize`
    /// round-trip on a placeholder-bearing tracking file blew up at load
    /// time).
    #[test]
    fn placeholder_satisfies_blake_hash_validation() {
        let p = BlakeHash::placeholder();
        BlakeHash::new(p.as_str().to_owned()).expect("placeholder must satisfy validation");
    }

    #[test]
    fn hash_artifact_returns_blake_hash_round_trips_through_serde() {
        // hash_artifact's BlakeHash output must satisfy BlakeHash's own
        // validation invariant. Belt-and-braces check that the
        // `from_blake3_digest` constructor stays in sync with `validate_blake_hash`.
        let tmp = tempdir().unwrap();
        let base = tmp.path();
        fs::write(base.join("a.txt"), b"hello").unwrap();

        let h = hash_artifact(base, &[PathBuf::from("a.txt")]).unwrap();
        let json = serde_json::to_string(&h).unwrap();
        let h2: BlakeHash = serde_json::from_str(&json).unwrap();
        assert_eq!(h, h2);
    }
}

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
use std::path::PathBuf;

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

// Implementations land in subsequent tasks.

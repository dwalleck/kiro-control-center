//! Public types for steering install. Mirrors the shape of the agent
//! types module: error enum + warning enum + result/outcome structs.
//!
//! Per S3-1 the infrastructure variants (`HashFailed`,
//! `StagingWriteFailed`, `DestinationDirFailed`, `TrackingMalformed`)
//! carry a `path: PathBuf` so a top-level "no such file or directory"
//! never surfaces without context. Per S3-8 the per-file outcome reuses
//! the workspace-shared [`InstallOutcomeKind`] rather than introducing
//! a parallel enum.

use std::io;
use std::path::PathBuf;

use serde::Serialize;
use thiserror::Error;

use crate::project::InstallOutcomeKind;
use crate::service::InstallMode;

/// Bundled non-source-specific install identity threaded through the
/// per-file steering install chain. Mirrors
/// [`crate::service::AgentInstallContext`] (no `accept_mcp` because
/// steering files have no execution semantics — see plan rationale).
///
/// `Copy` because every field is already a cheap reference / primitive.
#[derive(Debug, Clone, Copy)]
pub struct SteeringInstallContext<'a> {
    pub mode: InstallMode,
    pub marketplace: &'a str,
    pub plugin: &'a str,
    pub version: Option<&'a str>,
}

/// Errors that can occur during steering install.
#[derive(Debug, Error)]
#[non_exhaustive]
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

    #[error("hash computation failed at `{path}`")]
    HashFailed {
        path: PathBuf,
        #[source]
        source: crate::hash::HashError,
    },

    #[error("steering staging file `{path}` could not be written")]
    StagingWriteFailed {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("steering destination directory `{path}` could not be prepared")]
    DestinationDirFailed {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("steering tracking JSON malformed at `{path}`")]
    TrackingMalformed {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
}

/// Per-call outcome of `KiroProject::install_steering_file`.
///
/// The `kind` field uses the workspace-shared [`InstallOutcomeKind`] so
/// presenters can match exhaustively over the same 3-variant enum used
/// by `InstalledNativeAgentOutcome` and `InstalledNativeCompanionsOutcome`.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct InstalledSteeringOutcome {
    pub source: PathBuf,
    pub destination: PathBuf,
    pub kind: InstallOutcomeKind,
    pub source_hash: String,
    pub installed_hash: String,
}

/// Per-file failure entry in a steering install batch.
#[derive(Debug)]
pub struct FailedSteeringFile {
    pub source: PathBuf,
    pub error: SteeringError,
}

/// Non-fatal issues raised during steering install. Discovery-time
/// problems (invalid scan paths, skipped candidates) get surfaced as
/// warnings rather than errors so the batch keeps making progress.
#[derive(Clone, Debug, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[non_exhaustive]
pub enum SteeringWarning {
    /// A steering scan path was declared but failed validation
    /// (path-traversal, absolute, non-utf-8 component, etc.).
    ScanPathInvalid { path: PathBuf, reason: String },
    /// A discovered candidate looked like steering but was excluded at
    /// discovery time (README-style markdown skipped, symlink refused).
    Skipped { path: PathBuf, reason: String },
}

impl std::fmt::Display for SteeringWarning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ScanPathInvalid { path, reason } => {
                write!(f, "skipped scan path {}: {}", path.display(), reason)
            }
            Self::Skipped { path, reason } => write!(
                f,
                "skipped steering candidate {}: {}",
                path.display(),
                reason
            ),
        }
    }
}

/// Aggregate result of `MarketplaceService::install_plugin_steering`.
#[derive(Debug, Default)]
pub struct InstallSteeringResult {
    pub installed: Vec<InstalledSteeringOutcome>,
    pub failed: Vec<FailedSteeringFile>,
    pub warnings: Vec<SteeringWarning>,
}

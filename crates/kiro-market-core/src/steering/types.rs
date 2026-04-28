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
use std::path::{Path, PathBuf};

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
    #[non_exhaustive]
    #[error("steering source `{path}` could not be read")]
    SourceReadFailed {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    /// Source file is a hardlink (Unix `nlink > 1`). See
    /// [`crate::agent::parse_native::NativeParseFailure::HardlinkRefused`]
    /// for the canonical threat-model statement; the steering install
    /// fires the same defense at the staging boundary so a hostile
    /// manifest can't exfiltrate inode contents into `.kiro/steering/`.
    #[non_exhaustive]
    #[error("refusing hardlinked steering source at `{path}` (nlink={nlink})")]
    SourceHardlinked { path: PathBuf, nlink: u64 },

    #[non_exhaustive]
    #[error(
        "steering file `{rel}` would clobber a file owned by plugin `{owner}`; \
         pass --force to transfer ownership"
    )]
    PathOwnedByOtherPlugin { rel: PathBuf, owner: String },

    #[non_exhaustive]
    #[error(
        "steering file exists at `{path}` but has no tracking entry; \
         remove it manually or pass --force"
    )]
    OrphanFileAtDestination { path: PathBuf },

    #[non_exhaustive]
    #[error(
        "steering file `{rel}` content has changed since last install; \
         pass --force to overwrite"
    )]
    ContentChangedRequiresForce { rel: PathBuf },

    #[non_exhaustive]
    #[error("steering tracking I/O failed at `{path}`")]
    TrackingIoFailed {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[non_exhaustive]
    #[error("hash computation failed at `{path}`")]
    HashFailed {
        path: PathBuf,
        #[source]
        source: crate::hash::HashError,
    },

    #[non_exhaustive]
    #[error("steering staging file `{path}` could not be written")]
    StagingWriteFailed {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[non_exhaustive]
    #[error("steering destination directory `{path}` could not be prepared")]
    DestinationDirFailed {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    /// Steering tracking JSON failed to parse. `reason` carries the full
    /// `serde_json` error chain materialized at the adapter boundary
    /// (`tracking_malformed` constructor) — the source type does not leak
    /// through the public API. Mirrors `AgentError::NativeManifestParseFailed`
    /// per CLAUDE.md "map external errors at the adapter boundary".
    #[non_exhaustive]
    #[error("steering tracking JSON malformed at `{path}`: {reason}")]
    TrackingMalformed { path: PathBuf, reason: String },
}

/// Construct a [`SteeringError::TrackingMalformed`] from a `serde_json::Error`,
/// materializing the full source chain into the variant's `reason` field at
/// the adapter boundary.
///
/// This is the only in-tree constructor for the variant; every call site
/// goes through it so the documented invariant — `reason` always carries
/// the materialized chain rather than an arbitrary string — is structural,
/// not just prose. The enum is `#[non_exhaustive]`, so external crates
/// cannot bypass this constructor with a struct literal.
#[must_use]
pub(crate) fn tracking_malformed(path: PathBuf, err: &serde_json::Error) -> SteeringError {
    SteeringError::TrackingMalformed {
        path,
        reason: crate::error::error_full_chain(err),
    }
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
///
/// `error` stays typed in-process so consumers can match on
/// [`SteeringError`] variants. The wire format projects it to a string
/// via [`error_full_chain`], mirroring the precedent set by
/// [`crate::service::FailedAgent`] / `serialize_agent_error` —
/// [`SteeringError`] carries `io::Error` / `HashError` payloads that
/// don't implement `Serialize`, and the serialized chain stays stable
/// across variant additions.
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct FailedSteeringFile {
    pub source: PathBuf,
    #[serde(serialize_with = "serialize_steering_error")]
    #[cfg_attr(feature = "specta", specta(type = String))]
    pub error: SteeringError,
}

/// Serialize a [`SteeringError`] as the rendered chain produced by
/// [`crate::error::error_full_chain`]. Mirrors
/// [`crate::service::serialize_agent_error`] — the typed variants carry
/// `io::Error` / `HashError` payloads that don't implement `Serialize`,
/// so the wire format projects through the chain string instead.
fn serialize_steering_error<S: serde::Serializer>(
    err: &SteeringError,
    serializer: S,
) -> std::result::Result<S::Ok, S::Error> {
    serializer.serialize_str(&crate::error::error_full_chain(err))
}

/// Non-fatal issues raised during steering discovery. Surface
/// actionable signals only — by-design exclusions (README-style files,
/// symlinks refused for security) stay as `tracing::debug!` so the
/// CLI doesn't spam users with normal product behaviour.
///
/// Per the original S3-2 amendment this enum was scoped wider; the
/// `Skipped` variant was retired during PR-64 review when it became
/// clear surfacing every README would teach users to ignore warnings,
/// and that symlink/junction refusals are by-design security behaviour
/// rather than actionable feedback for plugin authors.
///
/// The `reason: String` payloads on both variants are pre-rendered;
/// upgrading them to typed payloads (`ValidationError` / `io::Error`)
/// is tracked at
/// <https://github.com/dwalleck/kiro-control-center/issues/66>.
#[derive(Clone, Debug, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[non_exhaustive]
pub enum SteeringWarning {
    /// A steering scan path declared in the manifest failed validation
    /// (path-traversal, absolute, embedded NUL, non-utf-8 component).
    /// `path` carries the raw manifest value — almost always a typo
    /// worth surfacing to the plugin author. The validation rejection
    /// is also logged at `tracing::warn!` for operators.
    ScanPathInvalid { path: PathBuf, reason: String },
    /// A steering scan directory exists but couldn't be read
    /// (permission denied, I/O error). Distinct from `NotFound` —
    /// missing directories are a silent no-op since plugins commonly
    /// declare `./steering/` without authoring any files. This variant
    /// fires only for system-level failures the user can act on.
    ScanDirUnreadable { path: PathBuf, reason: String },
}

/// Wrapper for safe terminal rendering of paths from untrusted manifests.
/// Replaces ASCII control bytes (`0x00..0x20`, `0x7f`) and the U+202E /
/// U+202D RTL-override codepoints with `\x{NN}` / `\u{NNNN}` escapes so a
/// malicious manifest can't inject ANSI escape sequences (clear screen,
/// hide cursor, etc.) or display reordering tricks via warning render.
struct SafeForTerminal<'a>(&'a Path);

impl std::fmt::Display for SafeForTerminal<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use std::fmt::Write;
        for ch in self.0.to_string_lossy().chars() {
            let cp = ch as u32;
            if cp < 0x20 || cp == 0x7f {
                write!(f, "\\x{cp:02x}")?;
            } else if matches!(cp, 0x202d | 0x202e | 0x2066..=0x2069) {
                // Bidirectional override / isolate codepoints.
                write!(f, "\\u{{{cp:04x}}}")?;
            } else {
                f.write_char(ch)?;
            }
        }
        Ok(())
    }
}

impl std::fmt::Display for SteeringWarning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ScanPathInvalid { path, reason } => {
                write!(f, "skipped scan path {}: {}", SafeForTerminal(path), reason)
            }
            Self::ScanDirUnreadable { path, reason } => write!(
                f,
                "could not read steering scan directory {}: {}",
                SafeForTerminal(path),
                reason
            ),
        }
    }
}

/// Aggregate result of `MarketplaceService::install_plugin_steering`.
#[derive(Debug, Default, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct InstallSteeringResult {
    pub installed: Vec<InstalledSteeringOutcome>,
    pub failed: Vec<FailedSteeringFile>,
    pub warnings: Vec<SteeringWarning>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tracking_malformed_renders_path_and_reason() {
        use std::error::Error as _;
        let parse_err = serde_json::from_str::<serde_json::Value>("{not").unwrap_err();
        let err = tracking_malformed(PathBuf::from(".kiro/installed-steering.json"), &parse_err);
        let rendered = err.to_string();
        assert!(
            rendered.contains("installed-steering.json"),
            "path missing: {rendered}"
        );
        // The serde_json line/column survives the materialization.
        assert!(rendered.contains("line 1"), "reason missing: {rendered}");
        // Wire-format contract: the variant exposes no `source()` chain,
        // so downstream FFI surfaces (Tauri, CLI text) cannot accidentally
        // re-introduce the `serde_json::Error` type by walking `.source()`.
        // Re-introducing `#[source]` would silently break this assertion.
        // Mirrors the same lock at
        // `crate::error::tests::native_manifest_parse_failed_renders_path_and_reason`.
        assert!(
            err.source().is_none(),
            "TrackingMalformed must not expose a source chain — \
             reason: String is the only carrier of the materialized serde_json detail"
        );
    }
}

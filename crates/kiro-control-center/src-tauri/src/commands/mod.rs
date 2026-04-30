pub mod agents;
pub mod browse;
pub mod installed;
pub mod kiro_settings;
pub mod marketplaces;
pub mod plugins;
pub mod settings;
pub mod steering;

use kiro_market_core::cache::CacheDir;
use kiro_market_core::git::GixCliBackend;
use kiro_market_core::service::MarketplaceService;

use crate::error::{CommandError, ErrorType};

/// Construct a [`MarketplaceService`] for read-side and install-only
/// command handlers. Centralized here so every `#[tauri::command]` wrapper
/// resolves the cache directory and `GitBackend` the same way; previously
/// the body was duplicated in every command file.
///
/// All current callers are read-only or install-only; the [`GixCliBackend`]
/// is unused on every code path, so the default backend is fine. If a
/// command grows that needs a different backend, take the service as a
/// parameter on the `_impl` instead of branching here.
pub(in crate::commands) fn make_service() -> Result<MarketplaceService, CommandError> {
    let cache = CacheDir::default_location().ok_or_else(|| {
        CommandError::new(
            "could not determine data directory; is $HOME set?",
            ErrorType::IoError,
        )
    })?;
    Ok(MarketplaceService::new(cache, GixCliBackend::default()))
}

/// Fail-fast validation of a Tauri-supplied `project_path`, returning
/// the canonical absolute path on success.
///
/// Rejects:
///
/// - an empty / whitespace-only string — frontend default-construction
///   would otherwise silently write to `./.kiro/...` relative to the
///   Tauri process cwd instead of the user's project,
/// - a path that cannot be `stat`ed (does not exist, permission
///   denied, ...),
/// - a top-level symlink — defense-in-depth: the install layer in
///   `kiro-market-core` validates per-content path entries, but the
///   project root itself going through a symlink chain widens the trust
///   boundary unnecessarily,
/// - a path with no `.kiro/` subdirectory under the canonical root.
///
/// Returning the canonical [`PathBuf`] solves a separate problem too:
/// `with_file_lock` keys derive from the path the caller hands in, so
/// `/proj`, `/proj/./`, and `/proj/../proj` would all acquire distinct
/// locks even though they alias to the same project on disk. Forcing
/// every caller to thread the canonicalised result into
/// [`kiro_market_core::project::KiroProject::new`] keeps the
/// cross-call mutex coherent.
pub(in crate::commands) fn validate_kiro_project_path(
    project_path: &str,
) -> Result<std::path::PathBuf, CommandError> {
    if project_path.trim().is_empty() {
        return Err(CommandError::new(
            "project_path must not be empty",
            ErrorType::Validation,
        ));
    }
    let raw = std::path::Path::new(project_path);
    let metadata = std::fs::symlink_metadata(raw).map_err(|e| {
        CommandError::new(
            format!("project_path `{project_path}` could not be read: {e}"),
            ErrorType::Validation,
        )
    })?;
    if metadata.file_type().is_symlink() {
        return Err(CommandError::new(
            format!("project_path `{project_path}` is a symlink (refused)"),
            ErrorType::Validation,
        ));
    }
    let canonical = std::fs::canonicalize(raw).map_err(|e| {
        CommandError::new(
            format!("project_path `{project_path}` could not be canonicalized: {e}"),
            ErrorType::Validation,
        )
    })?;
    if !canonical.join(".kiro").is_dir() {
        return Err(CommandError::new(
            format!(
                "project_path `{}` is not a Kiro project (missing `.kiro/` directory)",
                canonical.display()
            ),
            ErrorType::Validation,
        ));
    }
    Ok(canonical)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    /// A real Kiro project directory must round-trip through the
    /// validator and come back as a canonical absolute path.
    #[test]
    fn validate_kiro_project_path_succeeds_for_real_project_dir() {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::create_dir_all(dir.path().join(".kiro")).expect("create .kiro");
        let canonical = validate_kiro_project_path(dir.path().to_str().expect("utf-8"))
            .expect("real project must validate");
        assert!(
            canonical.is_absolute(),
            "validator must return an absolute path, got: {canonical:?}",
        );
        assert!(
            canonical.join(".kiro").is_dir(),
            ".kiro/ must be reachable under the returned canonical path",
        );
    }

    /// Empty / whitespace-only strings short-circuit before any FS
    /// access — the validator must not call `stat` on `""`.
    #[test]
    fn validate_kiro_project_path_rejects_empty_and_whitespace() {
        for raw in ["", "   ", "\t"] {
            let err = validate_kiro_project_path(raw).expect_err("must reject");
            assert_eq!(err.error_type, ErrorType::Validation);
        }
    }

    /// A top-level symlink — even one pointing at a valid Kiro project
    /// — is refused. Defense-in-depth: the install layer's per-content
    /// path checks don't see the project root itself, so we close that
    /// gap at the IPC boundary.
    #[cfg(unix)]
    #[test]
    fn validate_kiro_project_path_refuses_symlink_to_valid_kiro_project() {
        let dir = tempfile::tempdir().expect("tempdir");
        let real = dir.path().join("real");
        fs::create_dir_all(real.join(".kiro")).expect("create real/.kiro");
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&real, &link).expect("create symlink");

        let err = validate_kiro_project_path(link.to_str().expect("utf-8"))
            .expect_err("symlinked project root must be refused");
        assert_eq!(err.error_type, ErrorType::Validation);
        assert!(
            err.message.contains("symlink"),
            "error must mention symlink, got: {}",
            err.message,
        );
    }

    /// Three syntactically distinct paths that alias to the same
    /// directory must canonicalize to a single `PathBuf`. Without this
    /// guarantee, `with_file_lock` would acquire distinct keys for the
    /// same project and the cross-call mutex would silently fail.
    #[test]
    fn validate_kiro_project_path_returns_canonical_path_normalizing_dot_segments() {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::create_dir_all(dir.path().join(".kiro")).expect("create .kiro");
        let base = dir.path().to_str().expect("utf-8").to_owned();
        let with_dot = format!("{base}/.");
        // `<dir>/sub/..` resolves back to `<dir>` after canonicalization.
        let sub = dir.path().join("sub");
        fs::create_dir_all(&sub).expect("create sub");
        let with_dotdot = format!("{base}/sub/..");

        let a = validate_kiro_project_path(&base).expect("plain path");
        let b = validate_kiro_project_path(&with_dot).expect("./ suffix");
        let c = validate_kiro_project_path(&with_dotdot).expect("sub/.. round-trip");

        assert_eq!(
            a, b,
            "trailing `.` must canonicalize to the same path: {a:?} vs {b:?}",
        );
        assert_eq!(
            a, c,
            "`sub/..` must canonicalize to the same path: {a:?} vs {c:?}",
        );
    }

    /// Nonexistent path returns Validation (canonicalize fails on a
    /// path the FS can't `stat`).
    #[test]
    fn validate_kiro_project_path_rejects_nonexistent_path() {
        let dir = tempfile::tempdir().expect("tempdir");
        let bogus = dir.path().join("does-not-exist");
        let err = validate_kiro_project_path(bogus.to_str().expect("utf-8"))
            .expect_err("nonexistent path must error");
        assert_eq!(err.error_type, ErrorType::Validation);
    }

    /// A real directory without `.kiro/` is rejected. Pins the
    /// "missing .kiro" branch separately from the symlink and
    /// nonexistent-path branches.
    #[test]
    fn validate_kiro_project_path_rejects_directory_without_kiro_subdir() {
        let dir = tempfile::tempdir().expect("tempdir");
        let err = validate_kiro_project_path(dir.path().to_str().expect("utf-8"))
            .expect_err("missing .kiro/ must error");
        assert_eq!(err.error_type, ErrorType::Validation);
        assert!(
            err.message.contains(".kiro"),
            "error must mention .kiro/, got: {}",
            err.message,
        );
    }
}

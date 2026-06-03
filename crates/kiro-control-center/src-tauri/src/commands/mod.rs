pub mod agents;
pub mod agents_authoring;
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

/// Reject an empty `names` slice at the IPC boundary. Empty `names`
/// is structurally ambiguous with `InstallFilter::All` at the core
/// layer: `filter_matches` returns false for every item and
/// `surface_unmatched_names` sees no misses to surface — net result
/// is a silent Ok with empty installed/failed. Callers (drawer
/// applyDrawerDiff) already short-circuit on empty diffs; this
/// rejection is defensive against future / non-drawer callers.
pub(in crate::commands) fn reject_empty_names(
    names: &[String],
    command: &str,
) -> Result<(), CommandError> {
    if names.is_empty() {
        return Err(CommandError::new(
            format!("{command}: names list must not be empty"),
            ErrorType::Validation,
        ));
    }
    Ok(())
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

/// Byte cap for draft agent JSON payloads accepted at the IPC
/// boundary. Generously sized for any reasonable hand-authored agent
/// (typical files are under 10 KiB) while bounding the memory
/// footprint a renderer can ask the backend to allocate via a single
/// `create_user_agent` / `save_user_agent` call.
///
/// Kept equal to `kiro_market_core`'s `USER_AGENT_READ_BYTE_CAP` (the
/// read-side bound in `read_user_agent_json`) by intent: the read cap
/// must be >= this write cap or a draft saved at exactly the write cap
/// would be rejected on reload, breaking the authoring round-trip. The
/// two are separate constants (distinct crates, `usize` vs `u64`) equal
/// by value, not by definition — change them together.
pub(in crate::commands) const DRAFT_JSON_BYTE_CAP: usize = 1024 * 1024;

/// Fail-fast IPC-boundary guard for the `draft_json` payload accepted
/// by `create_user_agent` and `save_user_agent`. Caps the byte length
/// and parses the payload as JSON so a compromised or buggy renderer
/// cannot:
///
/// - DoS the backend by writing arbitrarily-large files into
///   `.kiro/agents/` (and through that, fill the disk on the user's
///   machine), or
/// - persist non-JSON bytes that the list endpoint's `serde_json`
///   parse then silently skips — leaving a file invisible to the UI
///   that wrote it.
///
/// The parse runs `serde_json::from_slice::<serde_json::Value>` which
/// only validates well-formedness, not schema conformance. Schema-level
/// checks (required fields, field types) live deeper in core; here we
/// only assert "this is JSON" because anything else means the renderer
/// is broken or hostile.
pub(in crate::commands) fn validate_draft_json_payload(
    draft_json: &str,
) -> Result<(), CommandError> {
    if draft_json.len() > DRAFT_JSON_BYTE_CAP {
        return Err(CommandError::new(
            format!(
                "draft_json exceeds {DRAFT_JSON_BYTE_CAP}-byte cap (got {} bytes)",
                draft_json.len()
            ),
            ErrorType::Validation,
        ));
    }
    serde_json::from_slice::<serde_json::Value>(draft_json.as_bytes()).map_err(|e| {
        CommandError::new(
            format!("draft_json is not valid JSON: {e}"),
            ErrorType::ParseError,
        )
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    /// The `{command}:` prefix is the only reason `reject_empty_names`
    /// takes a `command` parameter — without pinning it, a refactor
    /// could silently drop the prefix and make debug logs less useful
    /// while leaving the per-command "must not be empty" assertions
    /// in `commands/agents.rs` and `commands/steering.rs` green.
    #[test]
    fn reject_empty_names_includes_command_prefix() {
        let err = reject_empty_names(&[], "install_agents").expect_err("empty must reject");
        assert_eq!(err.error_type, ErrorType::Validation);
        assert!(
            err.message.starts_with("install_agents:"),
            "error must start with command prefix, got: {}",
            err.message,
        );
    }

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

    // -----------------------------------------------------------------------
    // validate_draft_json_payload
    // -----------------------------------------------------------------------

    #[test]
    fn validate_draft_json_payload_accepts_valid_object() {
        validate_draft_json_payload(r#"{"name": "ok", "tools": []}"#)
            .expect("valid object payload must pass");
    }

    /// Non-object roots (array, scalar) are still valid JSON, so the
    /// IPC-boundary guard accepts them — schema-level shape checks
    /// live in core. This pins the "well-formedness only" contract.
    #[test]
    fn validate_draft_json_payload_accepts_any_valid_json() {
        validate_draft_json_payload("[]").expect("array root is valid JSON");
        validate_draft_json_payload("null").expect("null is valid JSON");
        validate_draft_json_payload("42").expect("number is valid JSON");
    }

    #[test]
    fn validate_draft_json_payload_rejects_malformed_json() {
        let err =
            validate_draft_json_payload("{ not json").expect_err("malformed JSON must reject");
        assert_eq!(err.error_type, ErrorType::ParseError);
        assert!(
            err.message.contains("not valid JSON"),
            "error must mention JSON validity, got: {}",
            err.message,
        );
    }

    /// Defense-in-depth: a renderer that pastes (or programmatically
    /// submits) bytes beyond the cap must be refused at the wrapper
    /// before any filesystem work.
    #[test]
    fn validate_draft_json_payload_rejects_oversized_input() {
        // Build a well-formed JSON object whose total byte length
        // exceeds the cap by padding a string field. The cap check
        // must fire BEFORE the parse check; otherwise a hostile
        // renderer could spend the parser's time on a huge but
        // well-formed payload.
        let padding = "x".repeat(DRAFT_JSON_BYTE_CAP);
        let oversized = format!(r#"{{"name": "{padding}"}}"#);
        assert!(oversized.len() > DRAFT_JSON_BYTE_CAP);

        let err =
            validate_draft_json_payload(&oversized).expect_err("oversized payload must reject");
        assert_eq!(err.error_type, ErrorType::Validation);
        assert!(
            err.message.contains("cap"),
            "error must mention the byte cap, got: {}",
            err.message,
        );
    }

    /// A payload that is BOTH oversized AND malformed must surface
    /// the size error, not the parse error — the cap check is
    /// cheaper and runs first by design.
    #[test]
    fn validate_draft_json_payload_size_check_runs_before_parse() {
        let oversized_malformed = "x".repeat(DRAFT_JSON_BYTE_CAP + 1);
        let err = validate_draft_json_payload(&oversized_malformed)
            .expect_err("oversized payload must reject");
        assert_eq!(
            err.error_type,
            ErrorType::Validation,
            "size check (Validation) must fire before parse check (ParseError)",
        );
    }
}

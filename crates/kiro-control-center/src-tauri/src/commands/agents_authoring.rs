//! User-authored agent CRUD commands.
//!
//! **PROJECT-ONLY** — none of the wrappers in this module construct or
//! accept a [`kiro_market_core::service::MarketplaceService`]. Per
//! CLAUDE.md "Tauri command handlers", project-only commands inline
//! the body in the wrapper (no `_impl(svc, ...)` pattern).

use tracing::debug;

use kiro_market_core::project::KiroProject;
use kiro_market_core::user_agent::{SaveOutcome, UserAgentRow};

use crate::commands::{validate_draft_json_payload, validate_kiro_project_path};
use crate::error::CommandError;

/// List every JSON-parseable agent in `.kiro/agents/` for the given
/// project. Auto-creates the directory if absent.
#[tauri::command]
#[specta::specta]
pub async fn list_user_agents(project_path: String) -> Result<Vec<UserAgentRow>, CommandError> {
    let project_root = validate_kiro_project_path(&project_path)?;
    let project = KiroProject::new(project_root);
    let rows = project.list_user_agents().map_err(CommandError::from)?;
    debug!(count = rows.len(), "listed user agents");
    Ok(rows)
}

/// Read the raw JSON content of a user-authored agent for the editor's
/// edit-mode load. Returns the file's bytes as a UTF-8 string,
/// suitable for round-tripping back through [`save_user_agent`] after
/// the user makes edits.
///
/// Companion to [`list_user_agents`] (which only returns summary
/// fields). The editor's prompt / tools / MCP / resources / hooks /
/// advanced sections need the full in-file shape — `UserAgentRow`'s
/// counts can't reconstruct it. Without this command edit mode would
/// have to start from a synthetic empty draft, and saving would
/// silently truncate the agent.
#[tauri::command]
#[specta::specta]
pub async fn load_user_agent_json(
    name: String,
    project_path: String,
) -> Result<String, CommandError> {
    let project_root = validate_kiro_project_path(&project_path)?;
    let project = KiroProject::new(project_root);
    let json = project
        .read_user_agent_json(&name)
        .map_err(CommandError::from)?;
    debug!(agent = %name, bytes = json.len(), "user agent JSON loaded");
    Ok(json)
}

/// Atomically create a new user-authored agent at
/// `.kiro/agents/<name>.json`. Rejects existing-file collisions before
/// writing.
///
/// `draft_json` is the agent JSON as a UTF-8 string; the wrapper
/// passes its bytes directly to the core write path. No re-serialization.
#[tauri::command]
#[specta::specta]
pub async fn create_user_agent(
    name: String,
    draft_json: String,
    project_path: String,
) -> Result<(), CommandError> {
    let project_root = validate_kiro_project_path(&project_path)?;
    validate_draft_json_payload(&draft_json)?;
    let project = KiroProject::new(project_root);
    project
        .create_user_agent(&name, draft_json.as_bytes())
        .map_err(CommandError::from)?;
    debug!(agent = %name, "user agent created");
    Ok(())
}

/// Save an edited user-authored agent. Handles three orthogonal
/// shapes — in-place edit, rename, and optional detach from
/// marketplace tracking — under a single file lock.
///
/// `from_name` is the filename stem of the agent being edited.
/// `draft_name` is the post-edit name (may equal `from_name` for
/// in-place; differ for rename). `detach=true` drops the
/// `InstalledAgents` entry for `from_name` if present.
#[tauri::command]
#[specta::specta]
pub async fn save_user_agent(
    from_name: String,
    draft_name: String,
    draft_json: String,
    detach: bool,
    project_path: String,
) -> Result<SaveOutcome, CommandError> {
    let project_root = validate_kiro_project_path(&project_path)?;
    validate_draft_json_payload(&draft_json)?;
    let project = KiroProject::new(project_root);
    let outcome = project
        .save_user_agent(&from_name, &draft_name, draft_json.as_bytes(), detach)
        .map_err(CommandError::from)?;
    debug!(
        from = %from_name,
        to = %draft_name,
        detach,
        orphan = ?outcome.orphan_left_behind,
        "user agent saved"
    );
    Ok(outcome)
}

/// Delete a user-visible agent. Tracking-aware: agents with marketplace
/// lineage take the full `remove_agent` path (file lock + tracking
/// update + rollback on unlink failure); user-authored agents take a
/// direct `fs::remove_file` that is idempotent on `NotFound`.
#[tauri::command]
#[specta::specta]
pub async fn delete_user_agent(name: String, project_path: String) -> Result<(), CommandError> {
    let project_root = validate_kiro_project_path(&project_path)?;
    let project = KiroProject::new(project_root);
    project
        .delete_user_agent(&name)
        .map_err(CommandError::from)?;
    debug!(agent = %name, "user agent deleted");
    Ok(())
}

/// Duplicate a user-visible agent. Walks `<source>-copy`,
/// `<source>-copy-2`, ... finding the smallest unused name. The
/// duplicate is always user-authored even if the source has
/// marketplace lineage.
///
/// Returns the new agent's name as a string so the UI can navigate
/// to the duplicate or refresh the list.
#[tauri::command]
#[specta::specta]
pub async fn duplicate_user_agent(
    source_name: String,
    project_path: String,
) -> Result<String, CommandError> {
    let project_root = validate_kiro_project_path(&project_path)?;
    let project = KiroProject::new(project_root);
    let new_name = project
        .duplicate_user_agent(&source_name)
        .map_err(CommandError::from)?;
    debug!(source = %source_name, new = %new_name, "user agent duplicated");
    Ok(new_name)
}

#[cfg(test)]
mod tests {
    use super::*;

    // The wrappers are thin pass-throughs around `KiroProject` methods
    // whose own test coverage is comprehensive (see `project.rs` unit
    // tests). These tests confirm the wrapper plumbing — project_path
    // validation + error mapping — without re-testing the core
    // semantics.

    #[tokio::test]
    async fn list_user_agents_rejects_invalid_project_path() {
        let err = list_user_agents(String::new())
            .await
            .expect_err("empty project_path must fail");
        // The exact ErrorType comes from validate_kiro_project_path.
        // We assert there's an error, not the specific kind — the
        // validator is the contract.
        let _ = err;
    }

    #[tokio::test]
    async fn delete_user_agent_idempotent_via_wrapper() {
        let dir = tempfile::tempdir().expect("tempdir");
        // Set up a minimal .kiro/ so validate_kiro_project_path accepts.
        std::fs::create_dir(dir.path().join(".kiro")).expect("mk .kiro");
        let path = dir.path().to_string_lossy().to_string();

        // No agents/ subdir, no file — must be Ok(()) (idempotent
        // on `NotFound`).
        delete_user_agent("ghost".to_string(), path.clone())
            .await
            .expect("idempotent delete via wrapper");
    }

    #[tokio::test]
    async fn list_user_agents_on_empty_project_returns_empty_list() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir(dir.path().join(".kiro")).expect("mk .kiro");
        let path = dir.path().to_string_lossy().to_string();

        let rows = list_user_agents(path).await.expect("list ok");
        assert!(rows.is_empty());
    }

    /// `load_user_agent_json` happy + missing path. Wrapper plumbing
    /// only — full read semantics are pinned by the core
    /// `read_user_agent_json_*` rstests in `project.rs`.
    #[tokio::test]
    async fn load_user_agent_json_returns_bytes_and_typed_not_found() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path().join(".kiro/agents")).expect("mk .kiro/agents");
        std::fs::write(
            dir.path().join(".kiro/agents/alpha.json"),
            br#"{"name":"alpha"}"#,
        )
        .expect("seed");
        let path = dir.path().to_string_lossy().to_string();

        // Happy path: round-trip bytes verbatim.
        let got = load_user_agent_json("alpha".to_string(), path.clone())
            .await
            .expect("load ok");
        assert_eq!(got, r#"{"name":"alpha"}"#);

        // Missing file: typed NotFound (so the editor can branch on
        // `error_type === "not_found"`, not substring-match the message).
        let err = load_user_agent_json("ghost".to_string(), path)
            .await
            .expect_err("missing must error");
        assert_eq!(err.error_type, crate::error::ErrorType::NotFound);
    }

    /// `create_user_agent`'s wrapper must reject malformed JSON BEFORE
    /// touching the filesystem. Pins that `validate_draft_json_payload`
    /// is wired in — a future refactor that drops the call would let
    /// non-JSON bytes land in `.kiro/agents/<name>.json` where the list
    /// endpoint would then silently skip them.
    #[tokio::test]
    async fn create_user_agent_rejects_malformed_draft_json_at_wrapper() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path().join(".kiro/agents")).expect("mk .kiro/agents");
        let path = dir.path().to_string_lossy().to_string();

        let err = create_user_agent("victim".to_string(), "{ not valid json".to_string(), path)
            .await
            .expect_err("malformed draft_json must be refused at the wrapper");
        assert_eq!(err.error_type, crate::error::ErrorType::ParseError);
        // And no file landed on disk.
        assert!(
            !dir.path().join(".kiro/agents/victim.json").exists(),
            "wrapper rejection must happen before any FS write",
        );
    }

    /// `save_user_agent`'s wrapper must reject malformed JSON BEFORE
    /// touching the filesystem. Same shape as the create_user_agent
    /// test above; pins the same wiring on the save path.
    #[tokio::test]
    async fn save_user_agent_rejects_malformed_draft_json_at_wrapper() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path().join(".kiro/agents")).expect("mk .kiro/agents");
        std::fs::write(
            dir.path().join(".kiro/agents/existing.json"),
            br#"{"name": "existing"}"#,
        )
        .expect("seed existing agent");
        let path = dir.path().to_string_lossy().to_string();
        let pre_bytes =
            std::fs::read(dir.path().join(".kiro/agents/existing.json")).expect("read pre-state");

        let err = save_user_agent(
            "existing".to_string(),
            "existing".to_string(),
            "{ not valid json".to_string(),
            false,
            path,
        )
        .await
        .expect_err("malformed draft_json must be refused at the wrapper");
        assert_eq!(err.error_type, crate::error::ErrorType::ParseError);
        // And the existing file was not touched.
        let post_bytes =
            std::fs::read(dir.path().join(".kiro/agents/existing.json")).expect("read post-state");
        assert_eq!(
            pre_bytes, post_bytes,
            "wrapper rejection must happen before any FS write",
        );
    }
}

//! User-authored agent CRUD commands for the
//! `Workflows > Agents` view in Control Center.
//!
//! **PROJECT-ONLY** — none of the wrappers in this module construct or
//! accept a [`kiro_market_core::service::MarketplaceService`]. Per
//! CLAUDE.md "Tauri command handlers", project-only commands inline
//! the body in the wrapper (no `_impl(svc, ...)` pattern).
//!
//! Design claim C7 in `.agents-view/design-slice-1.md`. The CI gate
//! tracked at rivets-6g6r will enforce the no-MarketplaceService
//! invariant; until then, the manual grep
//! `grep -E '(make_service|MarketplaceService)' agents_authoring.rs`
//! is the fence. Both should return no matches against this file.

use tracing::debug;

use kiro_market_core::project::KiroProject;
use kiro_market_core::user_agent::UserAgentRow;

use crate::commands::validate_kiro_project_path;
use crate::error::CommandError;

/// List every JSON-parseable agent in `.kiro/agents/` for the given
/// project. Auto-creates the directory if absent.
///
/// Slice S8 wrapper around
/// [`KiroProject::list_user_agents`] (slice S3, design claim C1+C2).
#[tauri::command]
#[specta::specta]
pub async fn list_user_agents(project_path: String) -> Result<Vec<UserAgentRow>, CommandError> {
    let project_root = validate_kiro_project_path(&project_path)?;
    let project = KiroProject::new(project_root);
    let rows = project.list_user_agents().map_err(CommandError::from)?;
    debug!(count = rows.len(), "listed user agents");
    Ok(rows)
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
) -> Result<(), CommandError> {
    let project_root = validate_kiro_project_path(&project_path)?;
    let project = KiroProject::new(project_root);
    project
        .save_user_agent(&from_name, &draft_name, draft_json.as_bytes(), detach)
        .map_err(CommandError::from)?;
    debug!(
        from = %from_name,
        to = %draft_name,
        detach,
        "user agent saved"
    );
    Ok(())
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

    // The wrappers are thin pass-throughs around KiroProject methods
    // whose own test coverage is comprehensive (S3-S7 stress fixtures).
    // These tests confirm the wrapper plumbing — project_path
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

        // No agents/ subdir, no file — must be Ok(()) per S6 case 3.
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
}

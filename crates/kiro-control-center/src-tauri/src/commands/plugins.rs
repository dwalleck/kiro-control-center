//! Plugin-level commands for the Tauri frontend.
//!
//! Four commands live here:
//!
//! - [`install_plugin`] — orchestrator wrapper around
//!   [`MarketplaceService::install_plugin`] (Task 1's core method). Carries
//!   `accept_mcp: bool` per A-18 so the user's MCP opt-in flows through to
//!   the agent install. Splits into `_impl(svc, ...)` per CLAUDE.md's
//!   service-consuming-commands rule.
//! - [`list_installed_plugins`] — aggregator wrapper around
//!   [`KiroProject::installed_plugins`] (Task 2). No `_impl` per A-19; the
//!   command operates on `KiroProject` only and follows the
//!   [`crate::commands::installed::list_installed_skills`] precedent of
//!   inlining the body in the wrapper.
//! - [`remove_plugin`] — cascade wrapper around
//!   [`KiroProject::remove_plugin`] (Task 3). No `_impl` per A-19; same
//!   project-only-read shape as `list_installed_plugins`.
//! - [`detect_plugin_updates`] — update-detection wrapper around
//!   [`MarketplaceService::detect_plugin_updates`] (Phase 2a). Splits
//!   into [`detect_plugin_updates_impl`] per the service-consuming
//!   convention so the body is testable without a Tauri runtime.

use kiro_market_core::project::{InstalledPluginsView, KiroProject, RemovePluginResult};
use kiro_market_core::service::{
    DetectUpdatesResult, InstallMode, InstallPluginResult, MarketplaceService,
};
use kiro_market_core::validation::{MarketplaceName, PluginName};

use crate::commands::{make_service, validate_kiro_project_path};
use crate::error::CommandError;

/// Install every skill, steering file, and agent declared by a plugin
/// into the active project's `.kiro/` tree in one call.
///
/// The wrapper exists only to construct a [`MarketplaceService`] from
/// process globals and translate the FFI `force: bool` into an
/// [`InstallMode`]; the install itself runs in [`install_plugin_impl`]
/// so it can be tested without a Tauri runtime.
///
/// `accept_mcp` is the per-call MCP opt-in gate (A-18). The Phase 1
/// frontend hardcodes `false` at the call site until a user-toggle UI
/// lands; the parameter is plumbed through so a later PR can flip it
/// without touching this signature.
#[tauri::command]
#[specta::specta]
pub async fn install_plugin(
    marketplace: String,
    plugin: String,
    force: bool,
    accept_mcp: bool,
    project_path: String,
) -> Result<InstallPluginResult, CommandError> {
    let svc = make_service()?;
    install_plugin_impl(
        &svc,
        &marketplace,
        &plugin,
        InstallMode::from(force),
        accept_mcp,
        &project_path,
    )
}

fn install_plugin_impl(
    svc: &MarketplaceService,
    marketplace: &str,
    plugin: &str,
    mode: InstallMode,
    accept_mcp: bool,
    project_path: &str,
) -> Result<InstallPluginResult, CommandError> {
    let project_root = validate_kiro_project_path(project_path)?;
    let marketplace = MarketplaceName::new(marketplace)?;
    let plugin = PluginName::new(plugin)?;
    let project = KiroProject::new(project_root);
    svc.install_plugin(&project, &marketplace, &plugin, mode, accept_mcp)
        .map_err(CommandError::from)
}

fn detect_plugin_updates_impl(
    svc: &MarketplaceService,
    project_path: &str,
) -> Result<DetectUpdatesResult, CommandError> {
    let project_root = validate_kiro_project_path(project_path)?;
    let project = KiroProject::new(project_root);
    svc.detect_plugin_updates(&project)
        .map_err(CommandError::from)
}

/// Scan installed plugins for available updates by comparing each
/// project tracking entry's recorded `version` and `source_hash`
/// against the corresponding plugin in the marketplace cache. Reads
/// from local cache only — callers that want fresh data run
/// `update_marketplaces` first.
///
/// Returns a [`DetectUpdatesResult`] split into three vecs:
/// - `updates`: plugins with an available update (typed
///   `change_signal` distinguishes manifest version bump from
///   content drift without version bump).
/// - `failures`: plugins the scan couldn't check, with a typed
///   `kind: PluginUpdateFailureKind` for FE branching (Rule 42).
/// - `partial_load_warnings`: tracking files that failed to load
///   (corrupt JSON etc.) — the other tracking files still
///   contribute and the scan continues.
///
/// Splits into [`detect_plugin_updates_impl`] per the
/// service-consuming-command convention so the body is testable
/// without a Tauri runtime.
///
/// # Errors
///
/// Returns `CommandError::Validation` on an invalid `project_path`.
/// Per-plugin scan failures land in
/// [`DetectUpdatesResult::failures`] (a typed entry, not an
/// `Err(CommandError)`) so a single bad plugin doesn't abort the
/// whole scan.
#[tauri::command]
#[specta::specta]
pub async fn detect_plugin_updates(
    project_path: String,
) -> Result<DetectUpdatesResult, CommandError> {
    let svc = make_service()?;
    detect_plugin_updates_impl(&svc, &project_path)
}

/// List every installed plugin in the project, aggregated by
/// `(marketplace, plugin)` pair across the three tracking files.
///
/// No `_impl` split (A-19): the body is inline in the wrapper because
/// the command consumes `KiroProject` only — there's no
/// `MarketplaceService` to thread through. Mirrors the existing
/// [`crate::commands::installed::list_installed_skills`] shape; tests
/// exercise the wrapper directly via `#[tokio::test]`.
#[tauri::command]
#[specta::specta]
pub async fn list_installed_plugins(
    project_path: String,
) -> Result<InstalledPluginsView, CommandError> {
    let project_root = validate_kiro_project_path(&project_path)?;
    let project = KiroProject::new(project_root);
    project.installed_plugins().map_err(CommandError::from)
}

/// Remove every skill, steering file, and agent for a given
/// `(marketplace, plugin)` pair from the project, returning per-content
/// `removed: Vec<String>` lists plus per-content `failures` vecs.
///
/// No `_impl` split (A-19): same rationale as
/// [`list_installed_plugins`] above — `KiroProject`-only read/write,
/// no service.
#[tauri::command]
#[specta::specta]
pub async fn remove_plugin(
    marketplace: String,
    plugin: String,
    project_path: String,
) -> Result<RemovePluginResult, CommandError> {
    let project_root = validate_kiro_project_path(&project_path)?;
    let marketplace = MarketplaceName::new(marketplace)?;
    let plugin = PluginName::new(plugin)?;
    let project = KiroProject::new(project_root);
    project
        .remove_plugin(&marketplace, &plugin)
        .map_err(CommandError::from)
}

#[cfg(test)]
mod tests {
    //! `_impl`-level tests for [`install_plugin_impl`] and
    //! [`detect_plugin_updates_impl`] plus wrapper-level tests for
    //! [`list_installed_plugins`] and [`remove_plugin`] (A-19: the
    //! latter two have no `_impl`, so the wrapper IS the unit). The
    //! `#[tauri::command]` attribute on `install_plugin` /
    //! `detect_plugin_updates` is a thin serde shim and is not
    //! re-exercised here; see `commands/browse.rs::tests` for the
    //! canonical pattern.

    use std::fs;

    use kiro_market_core::project::KiroProject;
    use kiro_market_core::service::test_support::{
        make_kiro_project, make_plugin_with_skills, relative_path_entry,
        seed_marketplace_with_registry, temp_service,
    };

    use crate::error::ErrorType;

    use super::*;

    /// Write a `plugin.json` carrying the given JSON body into a plugin
    /// directory. Mirrors the inline helpers in `agents.rs::tests` and
    /// `steering.rs::tests` but without forcing `format: kiro-cli`, so
    /// the install resolves through the translated path (which is what
    /// [`install_plugin_impl`] tests want for the happy path).
    fn write_plugin_manifest(plugin_dir: &std::path::Path, body: &[u8]) {
        fs::create_dir_all(plugin_dir).expect("plugin dir");
        fs::write(plugin_dir.join("plugin.json"), body).expect("write plugin.json");
    }

    /// Write a steering markdown file under `<plugin_dir>/steering/`.
    fn write_steering_file(plugin_dir: &std::path::Path, rel: &str, body: &str) {
        let p = plugin_dir.join(rel);
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).expect("create steering parent");
        }
        fs::write(&p, body).expect("write steering file");
    }

    /// Write a translated-path agent file (markdown + YAML frontmatter).
    /// Without the frontmatter fence the translated install path silently
    /// skips the file — matches `agents.rs::tests::write_translated_agent_file`.
    fn write_translated_agent_file(
        plugin_dir: &std::path::Path,
        rel: &str,
        name: &str,
        body: &str,
    ) {
        let p = plugin_dir.join(rel);
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).expect("create agent parent");
        }
        let content = format!("---\nname: {name}\ndescription: Test agent\n---\n{body}");
        fs::write(&p, content).expect("write agent file");
    }

    // -----------------------------------------------------------------------
    // install_plugin_impl
    // -----------------------------------------------------------------------

    /// Happy path: a plugin declaring one skill, one steering file, and
    /// one translated agent must populate all three sub-results in one
    /// orchestrator call. Without this, a regression that dropped (say)
    /// the agent install branch from `MarketplaceService::install_plugin`
    /// would survive the per-content tests in `browse.rs`, `steering.rs`,
    /// and `agents.rs` because each of those only exercises one branch.
    #[test]
    fn install_plugin_impl_orchestrates_skills_steering_and_agents() {
        let (dir, svc) = temp_service();
        let entries = vec![relative_path_entry("myplugin", "plugins/myplugin")];
        let marketplace_path = seed_marketplace_with_registry(dir.path(), &svc, "mp1", &entries);
        let plugin_dir = marketplace_path.join("plugins/myplugin");
        // No `plugin.json` => translated path with default scan roots
        // for both steering and agents.
        make_plugin_with_skills(&marketplace_path, "myplugin", &["alpha"]);
        write_steering_file(&plugin_dir, "steering/guide.md", "# guide\n");
        write_translated_agent_file(
            &plugin_dir,
            "agents/reviewer.md",
            "reviewer",
            "You are a reviewer.\n",
        );
        let project_path = make_kiro_project(dir.path());

        let result = install_plugin_impl(
            &svc,
            "mp1",
            "myplugin",
            InstallMode::New,
            false,
            &project_path,
        )
        .expect("happy path");

        assert_eq!(result.plugin, "myplugin");
        assert_eq!(
            result.skills.installed,
            vec!["alpha".to_string()],
            "skills sub-result must populate, got: {:?}",
            result.skills,
        );
        assert!(
            result.skills.failed.is_empty(),
            "no skill failures expected: {:?}",
            result.skills.failed
        );
        assert_eq!(
            result.steering.installed.len(),
            1,
            "steering sub-result must populate, got: {:?}",
            result.steering,
        );
        assert!(
            result.steering.failed.is_empty(),
            "no steering failures expected: {:?}",
            result.steering.failed
        );
        assert_eq!(
            result.agents.installed,
            vec!["reviewer".to_string()],
            "agents sub-result must populate, got: {:?}",
            result.agents,
        );
        assert!(
            result.agents.failed.is_empty(),
            "no agent failures expected: {:?}",
            result.agents.failed
        );

        let project_root = std::path::PathBuf::from(&project_path);
        assert!(
            project_root.join(".kiro/skills/alpha/SKILL.md").exists(),
            "skill must land under .kiro/skills/"
        );
        assert!(
            project_root.join(".kiro/steering/guide.md").exists(),
            "steering must land under .kiro/steering/"
        );
        assert!(
            project_root.join(".kiro/agents/reviewer.json").exists(),
            "agent must land under .kiro/agents/"
        );
    }

    /// Plugin manifest with `version: "4.5.6"` must thread through
    /// `install_plugin`'s shared `ctx.version` into all three sub-results'
    /// tracking entries. Without this, a regression that hardcoded
    /// `version: None` on (say) the steering branch — while skills and
    /// agents still wrote the version — would be invisible: each
    /// per-content test only checks one branch.
    #[test]
    fn install_plugin_impl_threads_version_through_all_three_branches() {
        let (dir, svc) = temp_service();
        let entries = vec![relative_path_entry("myplugin", "plugins/myplugin")];
        let marketplace_path = seed_marketplace_with_registry(dir.path(), &svc, "mp1", &entries);
        let plugin_dir = marketplace_path.join("plugins/myplugin");
        write_plugin_manifest(&plugin_dir, br#"{"name": "myplugin", "version": "4.5.6"}"#);
        make_plugin_with_skills(&marketplace_path, "myplugin", &["alpha"]);
        write_steering_file(&plugin_dir, "steering/guide.md", "# guide\n");
        write_translated_agent_file(&plugin_dir, "agents/reviewer.md", "reviewer", "Body.\n");
        let project_path = make_kiro_project(dir.path());

        let result = install_plugin_impl(
            &svc,
            "mp1",
            "myplugin",
            InstallMode::New,
            false,
            &project_path,
        )
        .expect("install with manifest version");

        assert_eq!(result.version.as_deref(), Some("4.5.6"));

        let project = KiroProject::new(std::path::PathBuf::from(&project_path));
        let installed_skills = project.load_installed().expect("load installed skills");
        assert_eq!(
            installed_skills
                .skills
                .get("alpha")
                .and_then(|m| m.version.as_deref()),
            Some("4.5.6"),
            "skills tracking must carry the manifest version"
        );

        let steering = project.load_installed_steering().expect("load steering");
        assert_eq!(
            steering
                .files
                .get(std::path::Path::new("guide.md"))
                .and_then(|m| m.version.as_deref()),
            Some("4.5.6"),
            "steering tracking must carry the manifest version"
        );

        let agents = project.load_installed_agents().expect("load agents");
        assert_eq!(
            agents
                .agents
                .get("reviewer")
                .and_then(|m| m.version.as_deref()),
            Some("4.5.6"),
            "agents tracking must carry the manifest version"
        );
    }

    /// Unknown plugin must surface as `ErrorType::NotFound` from the
    /// preamble (`resolve_plugin_install_context` inside
    /// `MarketplaceService::install_plugin`). Mirrors the same assertion
    /// in `steering.rs::tests` and `agents.rs::tests`.
    #[test]
    fn install_plugin_impl_returns_not_found_for_unknown_plugin() {
        let (dir, svc) = temp_service();
        let entries = vec![relative_path_entry("real-plugin", "plugins/real-plugin")];
        let _ = seed_marketplace_with_registry(dir.path(), &svc, "mp1", &entries);
        let project_path = make_kiro_project(dir.path());

        let err = install_plugin_impl(
            &svc,
            "mp1",
            "does-not-exist",
            InstallMode::New,
            false,
            &project_path,
        )
        .expect_err("unknown plugin must error");

        assert_eq!(err.error_type, ErrorType::NotFound);
    }

    /// `KiroProject::new` is infallible, so `install_plugin_impl` must
    /// fail-fast on an empty `project_path` before the orchestrator
    /// starts writing under `./.kiro/`. Pins the
    /// `validate_kiro_project_path` call.
    #[test]
    fn install_plugin_impl_empty_project_path_returns_validation() {
        let (_dir, svc) = temp_service();
        let err = install_plugin_impl(&svc, "mp1", "myplugin", InstallMode::New, false, "")
            .expect_err("empty project_path must error");
        assert_eq!(err.error_type, ErrorType::Validation);
        assert!(
            err.message.contains("project_path"),
            "error message must name the offending field, got: {}",
            err.message
        );
    }

    // FE-supplied `marketplace = "../etc/passwd"` would otherwise reach
    // `cache::marketplace_path(marketplace)` and force an FS access at
    // `<registries_dir>/../etc/passwd` before the registry layer's own
    // checks fire. The IPC-boundary `MarketplaceName::new` constructor
    // (which routes through `validate_name`) rejects it before the
    // service ever runs.
    #[test]
    fn install_plugin_impl_rejects_traversal_in_marketplace() {
        let (dir, svc) = temp_service();
        let project_path = make_kiro_project(dir.path());
        let err = install_plugin_impl(
            &svc,
            "../etc/passwd",
            "myplugin",
            InstallMode::New,
            false,
            &project_path,
        )
        .expect_err("traversal in marketplace must error");
        assert_eq!(err.error_type, ErrorType::Validation);
    }

    /// NUL bytes truncate C-string conversions in syscalls; the
    /// IPC-boundary `PluginName::new` constructor (which routes through
    /// `validate_name`) must reject them before they reach
    /// `cache::plugin_registry_path`.
    #[test]
    fn install_plugin_impl_rejects_nul_byte_in_plugin() {
        let (dir, svc) = temp_service();
        let project_path = make_kiro_project(dir.path());
        let err = install_plugin_impl(
            &svc,
            "mp1",
            "evil\0plugin",
            InstallMode::New,
            false,
            &project_path,
        )
        .expect_err("NUL byte in plugin name must error");
        assert_eq!(err.error_type, ErrorType::Validation);
    }

    // -----------------------------------------------------------------------
    // list_installed_plugins
    // -----------------------------------------------------------------------

    /// A fresh `.kiro/` project with no installs returns an empty vec
    /// (not an error). The aggregator must tolerate missing tracking
    /// files via the underlying `load_installed*` methods' "treat absent
    /// as empty" behavior.
    #[tokio::test]
    async fn list_installed_plugins_empty_for_fresh_project() {
        let dir = tempfile::tempdir().expect("tempdir");
        let project_path = make_kiro_project(dir.path());

        let view = list_installed_plugins(project_path)
            .await
            .expect("aggregator on fresh project");

        assert!(
            view.plugins.is_empty(),
            "fresh project must aggregate to an empty list, got: {view:?}"
        );
        assert!(
            view.partial_load_warnings.is_empty(),
            "fresh project: no warnings expected, got: {:?}",
            view.partial_load_warnings
        );
    }

    /// After [`install_plugin_impl`] seeds all three content types,
    /// `list_installed_plugins` must surface a single
    /// `(marketplace, plugin)` row whose counts reflect the install.
    /// Locks the aggregator's grouping contract end-to-end through the
    /// Tauri command boundary.
    #[tokio::test]
    async fn list_installed_plugins_aggregates_counts_after_install() {
        let (dir, svc) = temp_service();
        let entries = vec![relative_path_entry("myplugin", "plugins/myplugin")];
        let marketplace_path = seed_marketplace_with_registry(dir.path(), &svc, "mp1", &entries);
        let plugin_dir = marketplace_path.join("plugins/myplugin");
        make_plugin_with_skills(&marketplace_path, "myplugin", &["alpha"]);
        write_steering_file(&plugin_dir, "steering/guide.md", "# guide\n");
        write_translated_agent_file(&plugin_dir, "agents/reviewer.md", "reviewer", "Body.\n");
        let project_path = make_kiro_project(dir.path());

        install_plugin_impl(
            &svc,
            "mp1",
            "myplugin",
            InstallMode::New,
            false,
            &project_path,
        )
        .expect("seed via install_plugin_impl");

        let view = list_installed_plugins(project_path)
            .await
            .expect("aggregator after install");

        assert_eq!(
            view.plugins.len(),
            1,
            "expected one plugin row, got: {view:?}"
        );
        let row = &view.plugins[0];
        assert_eq!(row.marketplace, "mp1");
        assert_eq!(row.plugin, "myplugin");
        assert_eq!(row.skill_count, 1);
        assert_eq!(row.steering_count, 1);
        assert_eq!(row.agent_count, 1);
        assert_eq!(row.installed_skills, vec!["alpha".to_string()]);
        assert_eq!(row.installed_agents, vec!["reviewer".to_string()]);
        assert!(view.partial_load_warnings.is_empty());
    }

    /// An empty `project_path` must surface as `ErrorType::Validation`,
    /// same as the install command. Without this, a frontend default-
    /// constructed empty string would silently land at `./.kiro/...`
    /// relative to the Tauri process cwd.
    #[tokio::test]
    async fn list_installed_plugins_empty_project_path_returns_validation() {
        let err = list_installed_plugins(String::new())
            .await
            .expect_err("empty project_path must error");
        assert_eq!(err.error_type, ErrorType::Validation);
    }

    // -----------------------------------------------------------------------
    // remove_plugin
    // -----------------------------------------------------------------------

    /// Removing a `(marketplace, plugin)` pair that was never installed
    /// returns empty per-content `removed` lists (not an error). The
    /// cascade's "filter-then-remove" shape naturally produces empty
    /// vecs when no tracking entries match.
    #[tokio::test]
    async fn remove_plugin_returns_zeros_for_nonexistent_pair() {
        let dir = tempfile::tempdir().expect("tempdir");
        let project_path = make_kiro_project(dir.path());

        let result = remove_plugin(
            "mp-absent".to_string(),
            "plugin-absent".to_string(),
            project_path,
        )
        .await
        .expect("remove on empty project must succeed");

        assert!(result.skills.removed.is_empty());
        assert!(result.steering.removed.is_empty());
        assert!(result.agents.removed.is_empty());
        assert!(result.skills.failures.is_empty());
        assert!(result.steering.failures.is_empty());
        assert!(result.agents.failures.is_empty());
    }

    /// After [`install_plugin_impl`] seeds all three content types,
    /// `remove_plugin` must report per-content removed lists matching
    /// what was installed AND the tracking files must reflect the
    /// removal.
    #[tokio::test]
    async fn remove_plugin_returns_expected_counts_after_install() {
        let (dir, svc) = temp_service();
        let entries = vec![relative_path_entry("myplugin", "plugins/myplugin")];
        let marketplace_path = seed_marketplace_with_registry(dir.path(), &svc, "mp1", &entries);
        let plugin_dir = marketplace_path.join("plugins/myplugin");
        make_plugin_with_skills(&marketplace_path, "myplugin", &["alpha"]);
        write_steering_file(&plugin_dir, "steering/guide.md", "# guide\n");
        write_translated_agent_file(&plugin_dir, "agents/reviewer.md", "reviewer", "Body.\n");
        let project_path = make_kiro_project(dir.path());

        install_plugin_impl(
            &svc,
            "mp1",
            "myplugin",
            InstallMode::New,
            false,
            &project_path,
        )
        .expect("seed via install_plugin_impl");

        let result = remove_plugin(
            "mp1".to_string(),
            "myplugin".to_string(),
            project_path.clone(),
        )
        .await
        .expect("cascade remove");

        assert_eq!(result.skills.removed, vec!["alpha"]);
        assert_eq!(result.steering.removed, vec!["guide.md"]);
        assert_eq!(result.agents.removed, vec!["reviewer"]);
        assert!(result.skills.failures.is_empty());
        assert!(result.steering.failures.is_empty());
        assert!(result.agents.failures.is_empty());

        let view_after = list_installed_plugins(project_path)
            .await
            .expect("aggregator after remove");
        assert!(
            view_after.plugins.is_empty(),
            "all tracking entries must be gone after remove_plugin, got: {view_after:?}"
        );
    }

    /// An empty `project_path` must surface as `ErrorType::Validation`,
    /// same as the install / list commands.
    #[tokio::test]
    async fn remove_plugin_empty_project_path_returns_validation() {
        let err = remove_plugin("mp1".to_string(), "myplugin".to_string(), String::new())
            .await
            .expect_err("empty project_path must error");
        assert_eq!(err.error_type, ErrorType::Validation);
    }

    // -----------------------------------------------------------------------
    // detect_plugin_updates_impl
    // -----------------------------------------------------------------------

    #[test]
    fn detect_plugin_updates_impl_happy_path() {
        let (dir, svc) = temp_service();
        let project_path = make_kiro_project(dir.path());
        let result = detect_plugin_updates_impl(&svc, &project_path)
            .expect("scan succeeds with empty project");
        assert!(result.updates.is_empty());
        assert!(result.failures.is_empty());
    }

    #[test]
    fn detect_plugin_updates_impl_rejects_invalid_project_path() {
        let (_dir, svc) = temp_service();
        let result = detect_plugin_updates_impl(&svc, "/nonexistent/path/to/project");
        let err = result.expect_err("invalid project path must be rejected");
        assert_eq!(err.error_type, ErrorType::Validation);
    }
}

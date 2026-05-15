//! Plugin-agents install command for the Tauri frontend.
//!
//! Mirrors [`crate::commands::steering::install_plugin_steering`]: a thin
//! `#[tauri::command]` wrapper builds the [`MarketplaceService`] from
//! process globals, then delegates to a private `_impl` whose body is
//! unit-testable against [`kiro_market_core::service::test_support`]
//! fixtures without a Tauri runtime.
//!
//! Agent installs differ from steering in two parameters: the FFI carries
//! `accept_mcp: bool` (the per-call MCP opt-in gate documented on
//! [`AgentInstallContext::accept_mcp`]), and the resolver hands us
//! [`PluginInstallContext::format`] (`Some(KiroCli)` for native plugins,
//! `None` for translated). Otherwise the wrapper / `_impl` split is
//! identical.

use kiro_market_core::project::KiroProject;
use kiro_market_core::service::{
    AgentInstallContext, InstallAgentsResult, InstallFilter, InstallMode, MarketplaceService,
};
use kiro_market_core::validation::{AgentName, MarketplaceName, PluginName};

use crate::commands::{make_service, reject_empty_names, validate_kiro_project_path};
use crate::error::{CommandError, ErrorType};

/// Install every agent declared by a plugin into the active project's
/// `.kiro/agents/` directory.
///
/// The wrapper exists only to construct a [`MarketplaceService`] from
/// process globals and translate the FFI `force: bool` into an
/// [`InstallMode`]; the install itself runs in
/// [`install_plugin_agents_impl`] so it can be tested without a Tauri
/// runtime.
#[tauri::command]
#[specta::specta]
pub async fn install_plugin_agents(
    marketplace: String,
    plugin: String,
    force: bool,
    accept_mcp: bool,
    project_path: String,
) -> Result<InstallAgentsResult, CommandError> {
    let svc = make_service()?;
    install_plugin_agents_impl(
        &svc,
        &marketplace,
        &plugin,
        InstallMode::from(force),
        accept_mcp,
        &project_path,
    )
}

fn install_plugin_agents_impl(
    svc: &MarketplaceService,
    marketplace: &str,
    plugin: &str,
    mode: InstallMode,
    accept_mcp: bool,
    project_path: &str,
) -> Result<InstallAgentsResult, CommandError> {
    let project_root = validate_kiro_project_path(project_path)?;
    let marketplace = MarketplaceName::new(marketplace)?;
    let plugin = PluginName::new(plugin)?;
    let ctx = svc
        .resolve_plugin_install_context(&marketplace, &plugin)
        .map_err(CommandError::from)?;
    let project = KiroProject::new(project_root);

    Ok(MarketplaceService::install_plugin_agents(
        &project,
        &ctx.plugin_dir,
        &ctx.agent_scan_paths,
        ctx.format,
        &InstallFilter::All,
        AgentInstallContext {
            mode,
            accept_mcp,
            marketplace: &marketplace,
            plugin: &plugin,
            version: ctx.version.as_deref(),
        },
    ))
}

/// Per-agent install: drop into a plugin's discovered agent set and
/// install only the named agents. Mirrors
/// [`crate::commands::steering::install_steering_files`] for agents —
/// same `_impl(svc, ...)` testable shape and same wire shape as the
/// whole-plugin install.
///
/// Names are the **parsed** agent identity (frontmatter `name` for
/// translated dialects, JSON `name` field for native), NOT source
/// filenames — pinned by the slice-A2 fence
/// `install_plugin_agents_names_filter_joins_on_parsed_name`.
/// Unmatched names surface as `FailedAgent::RequestedButNotFound`
/// inside `result.failed`.
#[tauri::command]
#[specta::specta]
pub async fn install_agents(
    marketplace: String,
    plugin: String,
    names: Vec<String>,
    force: bool,
    accept_mcp: bool,
    project_path: String,
) -> Result<InstallAgentsResult, CommandError> {
    let svc = make_service()?;
    install_agents_impl(
        &svc,
        &marketplace,
        &plugin,
        &names,
        InstallMode::from(force),
        accept_mcp,
        &project_path,
    )
}

fn install_agents_impl(
    svc: &MarketplaceService,
    marketplace: &str,
    plugin: &str,
    names: &[String],
    mode: InstallMode,
    accept_mcp: bool,
    project_path: &str,
) -> Result<InstallAgentsResult, CommandError> {
    reject_empty_names(names, "install_agents")?;
    let project_root = validate_kiro_project_path(project_path)?;
    let marketplace = MarketplaceName::new(marketplace)?;
    let plugin = PluginName::new(plugin)?;
    let ctx = svc
        .resolve_plugin_install_context(&marketplace, &plugin)
        .map_err(CommandError::from)?;
    let project = KiroProject::new(project_root);

    Ok(MarketplaceService::install_plugin_agents(
        &project,
        &ctx.plugin_dir,
        &ctx.agent_scan_paths,
        ctx.format,
        &InstallFilter::Names(names),
        AgentInstallContext {
            mode,
            accept_mcp,
            marketplace: &marketplace,
            plugin: &plugin,
            version: ctx.version.as_deref(),
        },
    ))
}

/// Remove one agent from the active project. The `name` is the parsed
/// agent identity (matches the catalog's `AgentItemInfo.name`).
/// Routes through `AgentName::new` at the IPC boundary so a malformed
/// name is rejected before reaching `KiroProject::remove_agent`'s
/// filesystem call. Mirrors
/// [`crate::commands::installed::remove_skill`] — no plugin context
/// needed because agent names are unique under `.kiro/agents/`.
#[tauri::command]
#[specta::specta]
pub async fn remove_agent(name: String, project_path: String) -> Result<(), CommandError> {
    remove_agent_impl(&name, &project_path)
}

fn remove_agent_impl(name: &str, project_path: &str) -> Result<(), CommandError> {
    let project_root = validate_kiro_project_path(project_path)?;
    // Parse-don't-validate: route the FFI string through AgentName::new
    // so a malformed name (path traversal, NUL byte, empty) is rejected
    // here, not after a project-level validation pass that the FE
    // can't easily distinguish from "agent not installed."
    let validated = AgentName::new(name).map_err(|e| {
        CommandError::new(
            format!("invalid agent name `{name}`: {e}"),
            ErrorType::Validation,
        )
    })?;
    let project = KiroProject::new(project_root);
    project
        .remove_agent(validated.as_str())
        .map_err(CommandError::from)
}

#[cfg(test)]
mod tests {
    //! `_impl`-level tests. The `#[tauri::command]` wrapper is a thin
    //! serde shim and is not re-exercised here; see
    //! `commands/browse.rs::tests` for the canonical pattern.

    use std::fs;

    use kiro_market_core::service::test_support::{
        make_kiro_project, relative_path_entry, seed_marketplace_with_registry, temp_service,
    };

    use crate::error::ErrorType;

    use super::*;

    /// Write a translated-path agent file (markdown + YAML frontmatter
    /// `name`/`description`). The translated install path treats files
    /// without frontmatter as non-agents and silently skips them, so the
    /// frontmatter fence is required for the file to actually install.
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

    /// Write a native-format `plugin.json` declaring `format: kiro-cli`
    /// plus a single agent JSON under `agents/`. The native install path
    /// is the only one whose collision matrix surfaces
    /// `ContentChangedRequiresForce`; the translated path's `New` mode
    /// returns `AlreadyInstalled` (which becomes `skipped`, not `failed`)
    /// regardless of whether the source bytes changed. Mirroring the
    /// steering test's `force_mode_overwrites_changed_source` /
    /// `new_mode_surfaces_content_changed_in_failed` semantics therefore
    /// requires the native path.
    fn write_native_plugin(plugin_dir: &std::path::Path, name: &str, body: &[u8]) {
        fs::create_dir_all(plugin_dir).expect("plugin dir");
        fs::write(
            plugin_dir.join("plugin.json"),
            format!(r#"{{"name": "{name}", "format": "kiro-cli"}}"#),
        )
        .expect("write native plugin.json");
        let agents = plugin_dir.join("agents");
        fs::create_dir_all(&agents).expect("agents dir");
        fs::write(agents.join(format!("{name}.json")), body).expect("write native agent json");
    }

    #[test]
    fn install_plugin_agents_impl_installs_default_path_files() {
        let (dir, svc) = temp_service();
        let entries = vec![relative_path_entry("myplugin", "plugins/myplugin")];
        let marketplace_path = seed_marketplace_with_registry(dir.path(), &svc, "mp1", &entries);
        let plugin_dir = marketplace_path.join("plugins/myplugin");
        fs::create_dir_all(&plugin_dir).expect("plugin dir");
        // No `plugin.json` => translated path with the default
        // `./agents/` scan root, mirroring the steering happy-path test
        // which has no manifest either.
        write_translated_agent_file(
            &plugin_dir,
            "agents/reviewer.md",
            "reviewer",
            "You are a reviewer.\n",
        );
        let project_path = make_kiro_project(dir.path());

        let result = install_plugin_agents_impl(
            &svc,
            "mp1",
            "myplugin",
            InstallMode::New,
            false,
            &project_path,
        )
        .expect("happy path");

        assert_eq!(
            result.installed,
            vec!["reviewer".to_string()],
            "expected one installed agent named `reviewer`, got: installed={:?}, failed={:?}, warnings={:?}",
            result.installed,
            result.failed,
            result.warnings,
        );
        assert!(
            result.failed.is_empty(),
            "no failures expected: {:?}",
            result.failed
        );
        // A regression that surfaces a spurious warning on the happy path
        // (e.g. the README-skip warning leaking through, or an unmapped
        // tool warning when the agent declares no tools) would otherwise
        // be invisible — none of the other assertions touch `warnings`.
        assert!(
            result.warnings.is_empty(),
            "happy path must produce no warnings, got: {:?}",
            result.warnings
        );
        assert!(
            std::path::PathBuf::from(&project_path)
                .join(".kiro/agents/reviewer.json")
                .exists(),
            "agent JSON must land under the requested project_path"
        );
        assert!(
            std::path::PathBuf::from(&project_path)
                .join(".kiro/agents/prompts/reviewer.md")
                .exists(),
            "translated-path prompt body must land under prompts/"
        );
    }

    #[test]
    fn install_plugin_agents_impl_returns_not_found_for_unknown_plugin() {
        let (dir, svc) = temp_service();
        let entries = vec![relative_path_entry("real-plugin", "plugins/real-plugin")];
        let _ = seed_marketplace_with_registry(dir.path(), &svc, "mp1", &entries);
        let project_path = make_kiro_project(dir.path());

        let err = install_plugin_agents_impl(
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

    #[test]
    fn install_plugin_agents_impl_threads_resolved_version_into_install_ctx() {
        let (dir, svc) = temp_service();
        let entries = vec![relative_path_entry("myplugin", "plugins/myplugin")];
        let marketplace_path = seed_marketplace_with_registry(dir.path(), &svc, "mp1", &entries);
        let plugin_dir = marketplace_path.join("plugins/myplugin");
        fs::create_dir_all(&plugin_dir).expect("plugin dir");
        fs::write(
            plugin_dir.join("plugin.json"),
            br#"{"name": "myplugin", "version": "3.1.4"}"#,
        )
        .expect("write plugin.json");
        write_translated_agent_file(
            &plugin_dir,
            "agents/guide.md",
            "guide",
            "You are a guide.\n",
        );
        let project_path = make_kiro_project(dir.path());

        let result = install_plugin_agents_impl(
            &svc,
            "mp1",
            "myplugin",
            InstallMode::New,
            false,
            &project_path,
        )
        .expect("install with manifest version");

        assert_eq!(result.installed, vec!["guide".to_string()]);
        let project = KiroProject::new(std::path::PathBuf::from(&project_path));
        let tracking = project
            .load_installed_agents()
            .expect("load installed agents");
        let entry = tracking
            .agents
            .get("guide")
            .expect("guide should be tracked under .kiro/agents/");
        // The `version: "3.1.4"` from plugin.json must thread into the
        // tracking record — same contract as
        // `install_plugin_steering_impl_threads_resolved_version_into_install_ctx`
        // for the steering path.
        assert_eq!(entry.version.as_deref(), Some("3.1.4"));
        assert_eq!(entry.marketplace, "mp1");
        assert_eq!(entry.plugin, "myplugin");
    }

    /// Mirrors `install_plugin_steering_impl_force_mode_overwrites_changed_source`
    /// for the agents path. Uses the **native** install path (`format: kiro-cli`)
    /// because that's the only agent path whose collision matrix surfaces
    /// `ContentChangedRequiresForce` — the translated path returns
    /// `AlreadyInstalled` (skipped, not failed) regardless of source bytes.
    #[test]
    fn install_plugin_agents_impl_force_mode_overwrites_changed_source() {
        use kiro_market_core::project::InstallOutcomeKind;

        let (dir, svc) = temp_service();
        let entries = vec![relative_path_entry("myplugin", "plugins/myplugin")];
        let marketplace_path = seed_marketplace_with_registry(dir.path(), &svc, "mp1", &entries);
        let plugin_dir = marketplace_path.join("plugins/myplugin");
        write_native_plugin(
            &plugin_dir,
            "myplugin",
            br#"{"name":"reviewer","prompt":"v1"}"#,
        );
        let project_path = make_kiro_project(dir.path());

        let first = install_plugin_agents_impl(
            &svc,
            "mp1",
            "myplugin",
            InstallMode::New,
            false,
            &project_path,
        )
        .expect("first install");
        assert_eq!(
            first.installed,
            vec!["reviewer".to_string()],
            "first install must succeed, got: installed={:?}, failed={:?}",
            first.installed,
            first.failed,
        );

        // Bump the source bytes — same plugin, same agent name, different
        // hash. New mode would now refuse with ContentChangedRequiresForce
        // (covered by the next test); Force mode must overwrite.
        fs::write(
            plugin_dir.join("agents/myplugin.json"),
            br#"{"name":"reviewer","prompt":"v2"}"#,
        )
        .expect("rewrite agent v2");

        let forced = install_plugin_agents_impl(
            &svc,
            "mp1",
            "myplugin",
            InstallMode::Force,
            false,
            &project_path,
        )
        .expect("force re-install");

        assert_eq!(
            forced.installed,
            vec!["reviewer".to_string()],
            "force must re-install the changed agent, got: installed={:?}, failed={:?}",
            forced.installed,
            forced.failed,
        );
        assert_eq!(
            forced.installed_native.len(),
            1,
            "native path must populate installed_native, got: {:?}",
            forced.installed_native
        );
        assert!(
            matches!(
                forced.installed_native[0].kind,
                InstallOutcomeKind::ForceOverwrote
            ),
            "force outcome must be ForceOverwrote, got: {:?}",
            forced.installed_native[0].kind,
        );
        let installed_json = fs::read_to_string(
            std::path::PathBuf::from(&project_path).join(".kiro/agents/reviewer.json"),
        )
        .expect("read installed agent json");
        assert!(
            installed_json.contains("\"v2\""),
            "force overwrite must replace the bytes on disk, got: {installed_json}"
        );
    }

    /// Companion to the force test above — without `force`, a same-plugin
    /// reinstall whose source bytes have changed must surface
    /// `ContentChangedRequiresForce` in `failed` (not silently no-op into
    /// `skipped` or `installed`). Wire-format lock at the end pins the
    /// custom `Serialize` projection on `FailedAgent` (typed enum →
    /// string `error` on the FFI boundary).
    #[test]
    fn install_plugin_agents_impl_new_mode_surfaces_content_changed_in_failed() {
        let (dir, svc) = temp_service();
        let entries = vec![relative_path_entry("myplugin", "plugins/myplugin")];
        let marketplace_path = seed_marketplace_with_registry(dir.path(), &svc, "mp1", &entries);
        let plugin_dir = marketplace_path.join("plugins/myplugin");
        write_native_plugin(
            &plugin_dir,
            "myplugin",
            br#"{"name":"reviewer","prompt":"v1"}"#,
        );
        let project_path = make_kiro_project(dir.path());

        install_plugin_agents_impl(
            &svc,
            "mp1",
            "myplugin",
            InstallMode::New,
            false,
            &project_path,
        )
        .expect("first install");

        fs::write(
            plugin_dir.join("agents/myplugin.json"),
            br#"{"name":"reviewer","prompt":"v2"}"#,
        )
        .expect("rewrite agent v2");

        let result = install_plugin_agents_impl(
            &svc,
            "mp1",
            "myplugin",
            InstallMode::New,
            false,
            &project_path,
        )
        .expect("second install (should not error at the impl level)");

        assert!(
            result.installed.is_empty(),
            "InstallMode::New must NOT overwrite a changed source, got: {:?}",
            result.installed
        );
        assert!(
            result.skipped.is_empty(),
            "changed source is not idempotent — must NOT land in skipped, got: {:?}",
            result.skipped
        );
        assert_eq!(
            result.failed.len(),
            1,
            "expected one failure with ContentChangedRequiresForce, got: {:?}",
            result.failed
        );
        match &result.failed[0] {
            kiro_market_core::service::FailedAgent::Agent {
                error: kiro_market_core::error::AgentError::ContentChangedRequiresForce { name },
                ..
            } if name == "reviewer" => {}
            other => panic!(
                "expected FailedAgent::Agent {{ error: ContentChangedRequiresForce {{ name: \"reviewer\" }}, .. }}, \
                 got {other:?}"
            ),
        }

        let json = serde_json::to_value(&result).expect("InstallAgentsResult serializes");
        let failed_error = json
            .pointer("/failed/0/error")
            .expect("/failed/0/error must exist in wire format");
        assert!(
            failed_error.is_string(),
            "FailedAgent.error must serialize as string (FFI contract), \
             got non-string: {failed_error:?}"
        );
        let rendered = failed_error
            .as_str()
            .expect("error rendered as string per the FFI contract");
        assert!(
            rendered.contains("reviewer"),
            "rendered error must mention the agent name, got: {rendered}"
        );
    }

    // -----------------------------------------------------------------------
    // install_plugin_agents_impl — IPC-boundary newtype rejection (I2 from
    // PR #95 8-reviewer review). Mirrors `install_plugin_impl` tests in
    // `commands/plugins.rs::tests:368-401` so each install `_impl` carries
    // symmetric adversarial coverage; otherwise the `MarketplaceName::new`
    // / `PluginName::new` guard is only exercised on one of four install
    // paths.
    // -----------------------------------------------------------------------

    /// FE-supplied `marketplace = "../etc/passwd"` would otherwise reach
    /// `cache::marketplace_path(marketplace)` via the
    /// `resolve_plugin_install_context` lookup and force an FS read at
    /// `<registries_dir>/../etc/passwd`. The IPC-boundary
    /// `MarketplaceName::new` constructor rejects it before the service
    /// ever runs.
    #[test]
    fn install_plugin_agents_impl_rejects_traversal_in_marketplace() {
        let (dir, svc) = temp_service();
        let project_path = make_kiro_project(dir.path());
        let err = install_plugin_agents_impl(
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
    /// IPC-boundary `PluginName::new` constructor must reject them
    /// before they reach `cache::plugin_registry_path`.
    #[test]
    fn install_plugin_agents_impl_rejects_nul_byte_in_plugin() {
        let (dir, svc) = temp_service();
        let project_path = make_kiro_project(dir.path());
        let err = install_plugin_agents_impl(
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
    // install_agents_impl + remove_agent_impl (kiro-zx73 slices A5+A6)
    // -----------------------------------------------------------------------

    /// Tauri-side fence on per-agent install. Plugin ships two
    /// markdown agents; install one by parsed name. Asserts only that
    /// one lands in tracking + on disk; the other is NOT installed.
    /// The bug class — implementer wires the filter parameter but
    /// the catalog → drawer → command pipeline drops it — fails
    /// this test.
    #[test]
    fn install_agents_impl_installs_only_named_agent() {
        let (dir, svc) = temp_service();
        let entries = vec![relative_path_entry("myplugin", "plugins/myplugin")];
        let marketplace_path = seed_marketplace_with_registry(dir.path(), &svc, "mp1", &entries);
        let plugin_dir = marketplace_path.join("plugins/myplugin");
        let agents = plugin_dir.join("agents");
        fs::create_dir_all(&agents).expect("create agents dir");
        fs::write(
            agents.join("alpha.md"),
            "---\nname: alpha\ndescription: a\n---\nbody-a",
        )
        .expect("write alpha");
        fs::write(
            agents.join("beta.md"),
            "---\nname: beta\ndescription: b\n---\nbody-b",
        )
        .expect("write beta");
        let project_path = make_kiro_project(dir.path());

        let names = vec!["alpha".to_string()];
        let result = install_agents_impl(
            &svc,
            "mp1",
            "myplugin",
            &names,
            InstallMode::New,
            false,
            &project_path,
        )
        .expect("install one agent");

        assert_eq!(result.installed, vec!["alpha".to_string()]);
        assert!(result.failed.is_empty(), "got {:?}", result.failed);
        // Translated agents install as `.json` (parsed-and-converted),
        // not `.md`. Mirrors what install_plugin_agents_impl_installs_default_path_files
        // asserts at the destination level.
        let dest = std::path::PathBuf::from(&project_path).join(".kiro/agents");
        assert!(dest.join("alpha.json").exists(), "alpha.json must land");
        assert!(!dest.join("beta.json").exists(), "beta MUST NOT install");
    }

    /// Tauri-side fence on the **native** (`kiro-cli`) per-agent install
    /// path. Mirrors `install_agents_impl_installs_only_named_agent`
    /// (which exercises the translated markdown path) for the native
    /// JSON path. The bug class — a future refactor that drops the
    /// `InstallFilter::Names` branch in the native install loop would
    /// silently no-op every native filter request while leaving the
    /// translated path's filter test green — fails this test.
    #[test]
    fn install_agents_impl_native_path_filters_by_parsed_json_name() {
        let (dir, svc) = temp_service();
        let entries = vec![relative_path_entry("myplugin", "plugins/myplugin")];
        let marketplace_path = seed_marketplace_with_registry(dir.path(), &svc, "mp1", &entries);
        let plugin_dir = marketplace_path.join("plugins/myplugin");
        // write_native_plugin creates plugin.json (format: kiro-cli) +
        // one agent JSON at `agents/<name>.json`. Add a second native
        // agent so the filter actually has to choose between two.
        write_native_plugin(&plugin_dir, "myplugin", br#"{"name":"alpha","prompt":"a"}"#);
        fs::write(
            plugin_dir.join("agents/beta.json"),
            br#"{"name":"beta","prompt":"b"}"#,
        )
        .expect("write beta native agent");
        let project_path = make_kiro_project(dir.path());

        let names = vec!["alpha".to_string()];
        let result = install_agents_impl(
            &svc,
            "mp1",
            "myplugin",
            &names,
            InstallMode::New,
            false,
            &project_path,
        )
        .expect("install one native agent");

        assert_eq!(result.installed, vec!["alpha".to_string()]);
        assert!(result.failed.is_empty(), "got {:?}", result.failed);
        let dest = std::path::PathBuf::from(&project_path).join(".kiro/agents");
        assert!(dest.join("alpha.json").exists(), "alpha.json must land");
        assert!(
            !dest.join("beta.json").exists(),
            "beta MUST NOT install when filter selects alpha only",
        );
    }

    /// Empty `names` collapses to a silent Ok at the core layer because
    /// `filter_matches` returns false for every discovered agent.
    /// Reject at the IPC boundary so a future / non-drawer caller
    /// doesn't silently no-op. Paired with the analogous fence in
    /// `commands/steering.rs` and the core-side rstest in
    /// `service::tests::filter_matches_names_single_equivalence`.
    #[test]
    fn install_agents_impl_rejects_empty_names() {
        let (dir, svc) = temp_service();
        let project_path = make_kiro_project(dir.path());
        let err = install_agents_impl(
            &svc,
            "mp1",
            "myplugin",
            &[],
            InstallMode::New,
            false,
            &project_path,
        )
        .expect_err("empty names list must error");
        assert_eq!(err.error_type, ErrorType::Validation);
        assert!(
            err.message.contains("names list must not be empty"),
            "got {:?}",
            err.message
        );
    }

    /// Round-trip: install an agent then remove it via the new
    /// `remove_agent_impl`. Verifies the install_agents → remove_agent
    /// pipeline the drawer's diff math will produce.
    #[test]
    fn remove_agent_impl_unlinks_installed_agent() {
        let (dir, svc) = temp_service();
        let entries = vec![relative_path_entry("myplugin", "plugins/myplugin")];
        let marketplace_path = seed_marketplace_with_registry(dir.path(), &svc, "mp1", &entries);
        let plugin_dir = marketplace_path.join("plugins/myplugin");
        let agents = plugin_dir.join("agents");
        fs::create_dir_all(&agents).expect("create agents dir");
        fs::write(
            agents.join("rev.md"),
            "---\nname: rev\ndescription: r\n---\nbody",
        )
        .expect("write rev");
        let project_path = make_kiro_project(dir.path());

        let names = vec!["rev".to_string()];
        install_agents_impl(
            &svc,
            "mp1",
            "myplugin",
            &names,
            InstallMode::New,
            false,
            &project_path,
        )
        .expect("install");
        let dest = std::path::PathBuf::from(&project_path).join(".kiro/agents/rev.json");
        assert!(dest.exists(), "install precondition");

        remove_agent_impl("rev", &project_path).expect("remove");
        assert!(!dest.exists(), "agent file unlinked after remove");

        let project = KiroProject::new(std::path::PathBuf::from(&project_path));
        let tracking = project.load_installed_agents().expect("load tracking");
        assert!(
            !tracking.agents.contains_key("rev"),
            "tracking row must be removed alongside the file"
        );
    }

    /// Drawer Apply can re-call install_agents against unchanged
    /// agents — the user toggles, undoes, hits Apply. Second call must
    /// be idempotent: nothing in `failed`, the agent surfaces as
    /// `skipped` (already-installed semantic). Companion to
    /// install_plugin_agents_impl_new_mode_surfaces_content_changed_in_failed
    /// which covers the changed-source case; this is the unchanged
    /// twin.
    #[test]
    fn install_agents_impl_idempotent_reinstall_is_noop() {
        let (dir, svc) = temp_service();
        let entries = vec![relative_path_entry("myplugin", "plugins/myplugin")];
        let marketplace_path = seed_marketplace_with_registry(dir.path(), &svc, "mp1", &entries);
        let plugin_dir = marketplace_path.join("plugins/myplugin");
        let agents = plugin_dir.join("agents");
        fs::create_dir_all(&agents).expect("create agents dir");
        fs::write(
            agents.join("alpha.md"),
            "---\nname: alpha\ndescription: a\n---\nbody-a",
        )
        .expect("write alpha");
        let project_path = make_kiro_project(dir.path());
        let names = vec!["alpha".to_string()];

        let r1 = install_agents_impl(
            &svc,
            "mp1",
            "myplugin",
            &names,
            InstallMode::New,
            false,
            &project_path,
        )
        .expect("first install");
        assert_eq!(r1.installed, vec!["alpha".to_string()]);
        assert!(r1.failed.is_empty(), "{:?}", r1.failed);

        let r2 = install_agents_impl(
            &svc,
            "mp1",
            "myplugin",
            &names,
            InstallMode::New,
            false,
            &project_path,
        )
        .expect("second install must not error");
        assert!(
            r2.failed.is_empty(),
            "idempotent reinstall must not fail, got {:?}",
            r2.failed
        );
        assert_eq!(
            r2.skipped,
            vec!["alpha".to_string()],
            "second install must report alpha as skipped (already installed)",
        );
        assert!(
            r2.installed.is_empty(),
            "no new install on idempotent path, got {:?}",
            r2.installed,
        );
    }

    /// IPC-boundary hardening: a malformed agent name must be rejected
    /// before reaching `KiroProject::remove_agent`'s filesystem call.
    /// `AgentName::new` rejects NUL bytes, path separators, and other
    /// shapes that would otherwise navigate outside `.kiro/agents/`.
    #[test]
    fn remove_agent_impl_rejects_path_traversal_in_name() {
        let (dir, _svc) = temp_service();
        let project_path = make_kiro_project(dir.path());
        let err =
            remove_agent_impl("../etc/passwd", &project_path).expect_err("traversal must error");
        assert_eq!(err.error_type, ErrorType::Validation);
    }
}

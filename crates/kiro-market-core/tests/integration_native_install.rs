//! End-to-end integration test for the native kiro-cli plugin install
//! pipeline.
//!
//! Builds an inline starter-kit-shaped fixture (multiple JSON agents +
//! a `prompts/` companion subdirectory) under a tempdir, runs the full
//! `MarketplaceService::install_plugin_agents` pipeline against a
//! tempdir-rooted Kiro project, and asserts file landing, tracking
//! shape, and idempotent reinstall semantics.
//!
//! Lives in `tests/` rather than inline `#[cfg(test)] mod tests` because
//! it exercises the service + project + discovery + parser layers
//! together — this is the integration seam, not a unit boundary.

use std::fs;
use std::path::{Path, PathBuf};

use kiro_market_core::agent::AgentDialect;
use kiro_market_core::plugin::PluginFormat;
use kiro_market_core::project::{InstallOutcomeKind, KiroProject};
use kiro_market_core::service::test_support::temp_service;
use kiro_market_core::service::{
    AgentInstallContext, InstallAgentsResult, InstallMode, MarketplaceService,
};
use kiro_market_core::steering::{InstallSteeringResult, SteeringInstallContext};
use rstest::{fixture, rstest};
use tempfile::{TempDir, tempdir};

/// Integration-test harness: tempdir-rooted plugin staging area + a Kiro
/// project + a `MarketplaceService` from
/// [`kiro_market_core::service::test_support::temp_service`]. That helper
/// uses a `PanicOnNetworkBackend` so any accidental network reach in the
/// install path panics loudly rather than silently performing a clone —
/// matching the security posture of the unit tests.
///
/// Reaching `test_support` from an integration test requires the
/// `test-support` feature to be active for this crate's compilation
/// unit; the self-cycle dev-dep in `Cargo.toml` activates it.
///
/// Owns three tempdirs (`plugin_root`, `project_root`, `_svc_dir`) so
/// the harness's lifetime keeps every artifact alive for the test.
struct IntegrationHarness {
    plugin_root: TempDir,
    project_root: TempDir,
    project: KiroProject,
    _svc_dir: TempDir,
}

impl IntegrationHarness {
    /// Install `plugin_dir` (already staged on disk) under `(marketplace,
    /// plugin)`. Looks up the install context once and threads its `format`.
    fn install(
        &self,
        plugin_dir: &Path,
        marketplace: &str,
        plugin: &str,
        mode: InstallMode,
    ) -> (Option<PluginFormat>, InstallAgentsResult) {
        let ctx = MarketplaceService::resolve_plugin_install_context_from_dir(plugin_dir)
            .expect("resolve plugin install context");
        let install_ctx = AgentInstallContext {
            mode,
            accept_mcp: false, // fixtures never carry MCP servers
            marketplace,
            plugin,
            version: None,
        };
        let result = MarketplaceService::install_plugin_agents(
            &self.project,
            plugin_dir,
            &ctx.agent_scan_paths,
            ctx.format,
            install_ctx,
        );
        (ctx.format, result)
    }

    /// Install steering files for `plugin_dir` under `(marketplace, plugin)`.
    /// Reads the steering scan paths from the resolved install context so
    /// the manifest's declared steering paths drive the scan.
    fn install_steering(
        &self,
        plugin_dir: &Path,
        marketplace: &str,
        plugin: &str,
        mode: InstallMode,
    ) -> InstallSteeringResult {
        let ctx = MarketplaceService::resolve_plugin_install_context_from_dir(plugin_dir)
            .expect("resolve plugin install context");
        let steering_ctx = SteeringInstallContext {
            mode,
            marketplace,
            plugin,
            version: None,
        };
        MarketplaceService::install_plugin_steering(
            &self.project,
            plugin_dir,
            &ctx.steering_scan_paths,
            steering_ctx,
        )
    }
}

#[fixture]
fn harness() -> IntegrationHarness {
    let plugin_root = tempdir().expect("plugin tempdir");
    let project_root = tempdir().expect("project tempdir");
    let project = KiroProject::new(project_root.path().to_path_buf());
    let (svc_dir, _svc) = temp_service();
    IntegrationHarness {
        plugin_root,
        project_root,
        project,
        _svc_dir: svc_dir,
    }
}

/// Stage a starter-kit-shaped plugin tree under `<plugin_root>/<plugin-name>/`:
///
/// ```text
/// <plugin-name>/
///   plugin.json                # format: "kiro-cli"
///   agents/
///     <name>.json              # one per name in `agent_names`
///     prompts/
///       <name>.md              # one per name in `agent_names`
/// ```
fn stage_starter_kit_plugin(
    plugin_root: &Path,
    plugin_name: &str,
    agent_names: &[&str],
) -> PathBuf {
    let plugin_dir = plugin_root.join(plugin_name);
    let agents = plugin_dir.join("agents");
    let prompts = agents.join("prompts");
    fs::create_dir_all(&prompts).expect("create prompts dir");
    fs::write(
        plugin_dir.join("plugin.json"),
        format!(r#"{{"name":"{plugin_name}","format":"kiro-cli"}}"#),
    )
    .expect("write plugin.json");
    for name in agent_names {
        let json = format!(r#"{{"name":"{name}","prompt":"file://./prompts/{name}.md"}}"#);
        fs::write(agents.join(format!("{name}.json")), json).expect("write agent json");
        fs::write(prompts.join(format!("{name}.md")), b"prompt body").expect("write prompt");
    }
    plugin_dir
}

/// Stage a single-agent translated plugin (no `format` field) under
/// `<plugin_root>/<plugin-name>/`. Used to verify the new dispatcher
/// doesn't change behaviour for existing plugins.
fn stage_translated_plugin(plugin_root: &Path, plugin_name: &str, agent_name: &str) -> PathBuf {
    let plugin_dir = plugin_root.join(plugin_name);
    let agents = plugin_dir.join("agents");
    fs::create_dir_all(&agents).expect("create agents dir");
    fs::write(
        plugin_dir.join("plugin.json"),
        format!(r#"{{"name":"{plugin_name}"}}"#),
    )
    .expect("write plugin.json");
    fs::write(
        agents.join(format!("{agent_name}.md")),
        format!("---\nname: {agent_name}\n---\nYou are a reviewer.\n"),
    )
    .expect("write agent md");
    plugin_dir
}

/// Assert the post-install on-disk + tracking state for the starter-kit
/// fixture: every agent JSON and prompt file landed, and tracking
/// records every agent with dialect `Native` plus the companion bundle.
fn assert_starter_kit_landed(
    project_root: &Path,
    project: &KiroProject,
    plugin_name: &str,
    agent_names: &[&str],
) {
    for name in agent_names {
        let agent_dest = project_root
            .join(".kiro/agents")
            .join(format!("{name}.json"));
        let prompt_dest = project_root
            .join(".kiro/agents/prompts")
            .join(format!("{name}.md"));
        assert!(
            agent_dest.exists(),
            "{name}.json must land at .kiro/agents/"
        );
        assert!(
            prompt_dest.exists(),
            "{name}.md must land at .kiro/agents/prompts/"
        );
    }
    let tracking = project.load_installed_agents().expect("load tracking");
    for name in agent_names {
        let entry = tracking
            .agents
            .get(*name)
            .unwrap_or_else(|| panic!("agent `{name}` must be tracked"));
        assert_eq!(entry.dialect, AgentDialect::Native);
        assert_eq!(entry.plugin, plugin_name);
    }
    let companion_entry = tracking
        .native_companions
        .get(plugin_name)
        .expect("companion entry tracked");
    assert_eq!(companion_entry.files.len(), agent_names.len());
}

#[rstest]
fn end_to_end_native_plugin_with_agents_and_companions(harness: IntegrationHarness) {
    let agent_names = ["reviewer", "simplifier", "tester"];
    let plugin_dir =
        stage_starter_kit_plugin(harness.plugin_root.path(), "fake-reviewers", &agent_names);

    let (format, result) = harness.install(
        &plugin_dir,
        "test-marketplace",
        "fake-reviewers",
        InstallMode::New,
    );
    assert_eq!(format, Some(PluginFormat::KiroCli));

    assert!(result.failed.is_empty(), "no failures: {:?}", result.failed);
    assert_eq!(result.installed.len(), 3);
    assert_eq!(result.installed_native.len(), 3);
    let companions = result
        .installed_companions
        .as_ref()
        .expect("companion outcome present");
    assert_eq!(companions.files.len(), 3);
    assert_eq!(companions.kind, InstallOutcomeKind::Installed);

    assert_starter_kit_landed(
        harness.project_root.path(),
        &harness.project,
        "fake-reviewers",
        &agent_names,
    );

    // Idempotent reinstall — every agent and the companion bundle must
    // round-trip as a no-op.
    let (_, again) = harness.install(
        &plugin_dir,
        "test-marketplace",
        "fake-reviewers",
        InstallMode::New,
    );
    assert!(again.failed.is_empty(), "no failures: {:?}", again.failed);
    assert!(
        again.installed.is_empty(),
        "idempotent reinstall must not list anything in `installed`: {:?}",
        again.installed
    );
    assert_eq!(again.skipped.len(), 3);
    assert!(
        again
            .installed_native
            .iter()
            .all(|o| o.kind == InstallOutcomeKind::Idempotent),
        "every native outcome must be idempotent on reinstall"
    );
    assert_eq!(
        again
            .installed_companions
            .as_ref()
            .expect("companion outcome on reinstall")
            .kind,
        InstallOutcomeKind::Idempotent,
        "companion bundle must be idempotent on reinstall"
    );
}

#[rstest]
fn end_to_end_translated_plugin_unaffected_by_native_dispatch(harness: IntegrationHarness) {
    // A plugin without `format` in plugin.json takes the translated path.
    // The new dispatcher field defaulting to None must not change behavior
    // for existing plugins.
    let plugin_dir =
        stage_translated_plugin(harness.plugin_root.path(), "translated-plugin", "rev");

    let (format, result) = harness.install(&plugin_dir, "m", "translated-plugin", InstallMode::New);
    assert!(format.is_none(), "translated plugin has no format field");

    assert!(result.failed.is_empty(), "no failures: {:?}", result.failed);
    assert_eq!(result.installed, vec!["rev".to_string()]);
    assert!(
        result.installed_native.is_empty(),
        "translated path must NOT populate installed_native"
    );
    assert!(
        result.installed_companions.is_none(),
        "translated path must NOT populate installed_companions"
    );

    let tracking = harness.project.load_installed_agents().expect("load");
    assert_eq!(
        tracking.agents.get("rev").unwrap().dialect,
        AgentDialect::Claude
    );
}

/// Stage a starter-kit-shaped plugin tree with both agents/companions
/// AND a steering directory:
///
/// ```text
/// <plugin-name>/
///   plugin.json                # format: "kiro-cli"
///   agents/
///     <name>.json
///     prompts/<name>.md
///   steering/
///     <steering-file>          # e.g. process.md
/// ```
fn stage_starter_kit_plugin_with_steering(
    plugin_root: &Path,
    plugin_name: &str,
    agent_names: &[&str],
    steering_files: &[(&str, &[u8])],
) -> PathBuf {
    let plugin_dir = stage_starter_kit_plugin(plugin_root, plugin_name, agent_names);
    let steering = plugin_dir.join("steering");
    fs::create_dir_all(&steering).expect("create steering dir");
    for (name, body) in steering_files {
        fs::write(steering.join(name), body).expect("write steering file");
    }
    plugin_dir
}

#[rstest]
fn end_to_end_native_plugin_with_agents_companions_and_steering(harness: IntegrationHarness) {
    let agent_names = ["reviewer", "tester"];
    let steering_files: &[(&str, &[u8])] = &[("process.md", b"# Process\n\nshared rules\n")];
    let plugin_dir = stage_starter_kit_plugin_with_steering(
        harness.plugin_root.path(),
        "fake-reviewers",
        &agent_names,
        steering_files,
    );

    // Agents + companions install first.
    let (format, agent_result) = harness.install(
        &plugin_dir,
        "test-marketplace",
        "fake-reviewers",
        InstallMode::New,
    );
    assert_eq!(format, Some(PluginFormat::KiroCli));
    assert!(
        agent_result.failed.is_empty(),
        "no agent failures: {:?}",
        agent_result.failed
    );
    assert_eq!(agent_result.installed.len(), 2);

    // Steering install runs in its own pass.
    let steering_result = harness.install_steering(
        &plugin_dir,
        "test-marketplace",
        "fake-reviewers",
        InstallMode::New,
    );
    assert!(
        steering_result.failed.is_empty(),
        "no steering failures: {:?}",
        steering_result.failed
    );
    assert_eq!(steering_result.installed.len(), 1);
    assert_eq!(
        steering_result.installed[0].kind,
        InstallOutcomeKind::Installed
    );

    // All three destinations present.
    assert_starter_kit_landed(
        harness.project_root.path(),
        &harness.project,
        "fake-reviewers",
        &agent_names,
    );
    assert!(
        harness
            .project_root
            .path()
            .join(".kiro/steering/process.md")
            .exists(),
        "steering file must land at .kiro/steering/"
    );

    // Tracking entry for the steering file.
    let steering_tracking = harness
        .project
        .load_installed_steering()
        .expect("load steering tracking");
    let entry = steering_tracking
        .files
        .get(std::path::Path::new("process.md"))
        .expect("steering tracking entry written");
    assert_eq!(entry.plugin, "fake-reviewers");
    assert_eq!(entry.marketplace, "test-marketplace");

    // Idempotent reinstall of all three install targets.
    let (_, agents_again) = harness.install(
        &plugin_dir,
        "test-marketplace",
        "fake-reviewers",
        InstallMode::New,
    );
    assert!(
        agents_again
            .installed_native
            .iter()
            .all(|o| o.kind == InstallOutcomeKind::Idempotent),
        "every native agent outcome must be idempotent on reinstall"
    );

    let steering_again = harness.install_steering(
        &plugin_dir,
        "test-marketplace",
        "fake-reviewers",
        InstallMode::New,
    );
    assert!(
        steering_again
            .installed
            .iter()
            .all(|o| o.kind == InstallOutcomeKind::Idempotent),
        "every steering outcome must be idempotent on reinstall: {:?}",
        steering_again.installed
    );
}

#[rstest]
fn end_to_end_steering_multi_scan_root_installs_all_files(harness: IntegrationHarness) {
    // S3-11: multi-scan-root steering is supported. A plugin manifest
    // declaring `steering: ["./a/", "./b/"]` with distinct .md files
    // in each must install all of them — no upstream rejection like
    // the companion bundle's MultipleScanRootsNotSupported.
    let plugin_name = "multi-root";
    let plugin_dir = harness.plugin_root.path().join(plugin_name);
    fs::create_dir_all(plugin_dir.join("a")).unwrap();
    fs::create_dir_all(plugin_dir.join("b")).unwrap();
    fs::write(plugin_dir.join("a/alpha.md"), b"alpha body").unwrap();
    fs::write(plugin_dir.join("b/beta.md"), b"beta body").unwrap();
    fs::write(
        plugin_dir.join("plugin.json"),
        format!(r#"{{"name":"{plugin_name}","steering":["./a/","./b/"]}}"#),
    )
    .unwrap();

    let result = harness.install_steering(&plugin_dir, "m", plugin_name, InstallMode::New);

    assert!(
        result.failed.is_empty(),
        "no failures expected: {:?}",
        result.failed
    );
    assert_eq!(result.installed.len(), 2);
    assert!(
        harness
            .project_root
            .path()
            .join(".kiro/steering/alpha.md")
            .exists()
    );
    assert!(
        harness
            .project_root
            .path()
            .join(".kiro/steering/beta.md")
            .exists()
    );
}

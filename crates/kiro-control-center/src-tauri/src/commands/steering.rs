//! Steering install command for the Tauri frontend.
//!
//! Mirrors the shape of [`crate::commands::browse::install_skills`]:
//! a thin `#[tauri::command]` wrapper builds the [`MarketplaceService`]
//! from process globals, then delegates to a private `_impl` whose body
//! is unit-testable against [`kiro_market_core::service::test_support`]
//! fixtures without a Tauri runtime.

use std::path::PathBuf;

use kiro_market_core::project::KiroProject;
use kiro_market_core::service::{InstallMode, MarketplaceService};
use kiro_market_core::steering::{InstallSteeringResult, SteeringInstallContext};

use crate::commands::make_service;
use crate::error::CommandError;

/// Install every steering file declared by a plugin into the active
/// project's `.kiro/steering/` directory.
///
/// The wrapper exists only to construct a [`MarketplaceService`] from
/// process globals and translate the FFI `force: bool` into an
/// [`InstallMode`]; the install itself runs in
/// [`install_plugin_steering_impl`] so it can be tested without a Tauri
/// runtime.
#[tauri::command]
#[specta::specta]
pub async fn install_plugin_steering(
    marketplace: String,
    plugin: String,
    force: bool,
    project_path: String,
) -> Result<InstallSteeringResult, CommandError> {
    let svc = make_service()?;
    install_plugin_steering_impl(
        &svc,
        &marketplace,
        &plugin,
        InstallMode::from(force),
        &project_path,
    )
}

fn install_plugin_steering_impl(
    svc: &MarketplaceService,
    marketplace: &str,
    plugin: &str,
    mode: InstallMode,
    project_path: &str,
) -> Result<InstallSteeringResult, CommandError> {
    let ctx = svc
        .resolve_plugin_install_context(marketplace, plugin)
        .map_err(CommandError::from)?;
    let project = KiroProject::new(PathBuf::from(project_path));

    let install_ctx = SteeringInstallContext {
        mode,
        marketplace,
        plugin,
        version: ctx.version.as_deref(),
    };

    Ok(MarketplaceService::install_plugin_steering(
        &project,
        &ctx.plugin_dir,
        &ctx.steering_scan_paths,
        install_ctx,
    ))
}

#[cfg(test)]
mod tests {
    //! `_impl`-level tests. The `#[tauri::command]` wrapper is a thin
    //! serde shim and is not re-exercised here; see
    //! `commands/browse.rs::tests` for the canonical pattern.

    use std::fs;

    use kiro_market_core::service::test_support::{
        relative_path_entry, seed_marketplace_with_registry, temp_service,
    };

    use crate::error::ErrorType;

    use super::*;

    fn make_kiro_project(dir: &std::path::Path) -> String {
        let project_path = dir.join("kproj");
        fs::create_dir_all(project_path.join(".kiro")).expect("create .kiro dir");
        project_path.to_str().expect("utf-8 path").to_owned()
    }

    fn write_steering_file(plugin_dir: &std::path::Path, rel: &str, body: &str) {
        let p = plugin_dir.join(rel);
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).expect("create steering parent");
        }
        fs::write(&p, body).expect("write steering file");
    }

    #[test]
    fn install_plugin_steering_impl_installs_default_path_files() {
        let (dir, svc) = temp_service();
        let entries = vec![relative_path_entry("myplugin", "plugins/myplugin")];
        let marketplace_path = seed_marketplace_with_registry(dir.path(), &svc, "mp1", &entries);
        let plugin_dir = marketplace_path.join("plugins/myplugin");
        fs::create_dir_all(&plugin_dir).expect("plugin dir");
        // No plugin.json -> falls back to DEFAULT_STEERING_PATHS = ["./steering/"].
        write_steering_file(&plugin_dir, "steering/code-style.md", "# style\n");
        let project_path = make_kiro_project(dir.path());

        let result =
            install_plugin_steering_impl(&svc, "mp1", "myplugin", InstallMode::New, &project_path)
                .expect("happy path");

        assert_eq!(
            result.installed.len(),
            1,
            "expected one install, got: {:?}",
            result.installed
        );
        assert!(
            result.failed.is_empty(),
            "no failures expected: {:?}",
            result.failed
        );
        assert!(
            std::path::PathBuf::from(&project_path)
                .join(".kiro/steering/code-style.md")
                .exists(),
            "steering file must land under the requested project_path"
        );
    }

    #[test]
    fn install_plugin_steering_impl_returns_not_found_for_unknown_plugin() {
        let (dir, svc) = temp_service();
        let entries = vec![relative_path_entry("real-plugin", "plugins/real-plugin")];
        let _ = seed_marketplace_with_registry(dir.path(), &svc, "mp1", &entries);
        let project_path = make_kiro_project(dir.path());

        let err = install_plugin_steering_impl(
            &svc,
            "mp1",
            "does-not-exist",
            InstallMode::New,
            &project_path,
        )
        .expect_err("unknown plugin must error");

        assert_eq!(err.error_type, ErrorType::NotFound);
    }

    #[test]
    fn install_plugin_steering_impl_threads_resolved_version_into_install_ctx() {
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
        write_steering_file(&plugin_dir, "steering/guide.md", "# guide\n");
        let project_path = make_kiro_project(dir.path());

        let result =
            install_plugin_steering_impl(&svc, "mp1", "myplugin", InstallMode::New, &project_path)
                .expect("install with manifest version");

        assert_eq!(result.installed.len(), 1);
        let project = KiroProject::new(std::path::PathBuf::from(&project_path));
        let tracking = project
            .load_installed_steering()
            .expect("load installed steering");
        let entry = tracking
            .files
            .get(std::path::Path::new("guide.md"))
            .expect("guide.md should be tracked under .kiro/steering/");
        // The `version: "3.1.4"` from plugin.json must thread into the
        // tracking record —
        // `install_skills_impl_threads_resolved_version_into_install_result`
        // (browse.rs) locks the same contract for the skills path.
        assert_eq!(entry.version.as_deref(), Some("3.1.4"));
        assert_eq!(entry.marketplace, "mp1");
        assert_eq!(entry.plugin, "myplugin");
    }

    /// Mirrors `install_skills_impl_force_mode_overwrites_existing_install`
    /// (browse.rs) for the steering path. The `force: bool` parameter is the
    /// primary user-facing control; without this test, a regression where
    /// `InstallMode::from(force)` returned `New` unconditionally would
    /// silently pass the rest of the suite.
    #[test]
    fn install_plugin_steering_impl_force_mode_overwrites_changed_source() {
        use kiro_market_core::project::InstallOutcomeKind;

        let (dir, svc) = temp_service();
        let entries = vec![relative_path_entry("myplugin", "plugins/myplugin")];
        let marketplace_path = seed_marketplace_with_registry(dir.path(), &svc, "mp1", &entries);
        let plugin_dir = marketplace_path.join("plugins/myplugin");
        fs::create_dir_all(&plugin_dir).expect("plugin dir");
        write_steering_file(&plugin_dir, "steering/guide.md", "# v1\n");
        let project_path = make_kiro_project(dir.path());

        let first =
            install_plugin_steering_impl(&svc, "mp1", "myplugin", InstallMode::New, &project_path)
                .expect("first install");
        assert_eq!(first.installed.len(), 1, "first install must succeed");

        write_steering_file(&plugin_dir, "steering/guide.md", "# v2\n");

        let forced = install_plugin_steering_impl(
            &svc,
            "mp1",
            "myplugin",
            InstallMode::Force,
            &project_path,
        )
        .expect("force re-install");

        assert_eq!(
            forced.installed.len(),
            1,
            "force must re-install the changed file, got: installed={:?}, failed={:?}",
            forced.installed,
            forced.failed
        );
        assert!(
            matches!(forced.installed[0].kind, InstallOutcomeKind::ForceOverwrote),
            "force outcome must be ForceOverwrote, got: {:?}",
            forced.installed[0].kind
        );
        let installed_bytes = fs::read_to_string(
            std::path::PathBuf::from(&project_path).join(".kiro/steering/guide.md"),
        )
        .expect("read installed steering file");
        assert_eq!(installed_bytes, "# v2\n");
    }

    /// Companion to the force test above — without `force`, a changed
    /// source must surface `ContentChangedRequiresForce` in `failed`
    /// (not silently no-op into `installed`). Also exercises
    /// `serialize_steering_error` end-to-end: serializing the result
    /// must produce a JSON `error: string` for the failure.
    #[test]
    fn install_plugin_steering_impl_new_mode_surfaces_content_changed_in_failed() {
        let (dir, svc) = temp_service();
        let entries = vec![relative_path_entry("myplugin", "plugins/myplugin")];
        let marketplace_path = seed_marketplace_with_registry(dir.path(), &svc, "mp1", &entries);
        let plugin_dir = marketplace_path.join("plugins/myplugin");
        fs::create_dir_all(&plugin_dir).expect("plugin dir");
        write_steering_file(&plugin_dir, "steering/guide.md", "# v1\n");
        let project_path = make_kiro_project(dir.path());

        install_plugin_steering_impl(&svc, "mp1", "myplugin", InstallMode::New, &project_path)
            .expect("first install");

        write_steering_file(&plugin_dir, "steering/guide.md", "# v2\n");

        let result =
            install_plugin_steering_impl(&svc, "mp1", "myplugin", InstallMode::New, &project_path)
                .expect("second install (should not error at the impl level)");

        assert!(
            result.installed.is_empty(),
            "InstallMode::New must NOT overwrite a changed source, got: {:?}",
            result.installed
        );
        assert_eq!(
            result.failed.len(),
            1,
            "expected one failure with ContentChangedRequiresForce, got: {:?}",
            result.failed
        );
        assert!(
            matches!(
                &result.failed[0].error,
                kiro_market_core::steering::SteeringError::ContentChangedRequiresForce { .. }
            ),
            "wrong error variant: {:?}",
            result.failed[0].error
        );

        let json = serde_json::to_value(&result).expect("InstallSteeringResult serializes");
        let failed_error = json
            .pointer("/failed/0/error")
            .expect("/failed/0/error must exist in wire format");
        assert!(
            failed_error.is_string(),
            "FailedSteeringFile.error must serialize as string (FFI contract), \
             got non-string: {failed_error:?}"
        );
        let rendered = failed_error
            .as_str()
            .expect("error rendered as string per the FFI contract");
        assert!(
            rendered.contains("guide.md"),
            "rendered error must mention the path, got: {rendered}"
        );
    }
}

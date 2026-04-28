//! Steering install command for the Tauri frontend.
//!
//! Mirrors the shape of [`crate::commands::browse::install_skills`]:
//! a thin `#[tauri::command]` wrapper builds the [`MarketplaceService`]
//! from process globals, then delegates to a private `_impl` whose body
//! is unit-testable against [`kiro_market_core::service::test_support`]
//! fixtures without a Tauri runtime.

use std::path::PathBuf;

use kiro_market_core::cache::CacheDir;
use kiro_market_core::git::GixCliBackend;
use kiro_market_core::project::KiroProject;
use kiro_market_core::service::{InstallMode, MarketplaceService};
use kiro_market_core::steering::{InstallSteeringResult, SteeringInstallContext};

use crate::error::{CommandError, ErrorType};

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

fn make_service() -> Result<MarketplaceService, CommandError> {
    let cache = CacheDir::default_location().ok_or_else(|| {
        CommandError::new(
            "could not determine data directory; is $HOME set?",
            ErrorType::IoError,
        )
    })?;
    Ok(MarketplaceService::new(cache, GixCliBackend::default()))
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
        // tracking record — `install_skills_impl_threads_resolved_version`
        // (browse.rs) locks the same contract for the skills path.
        assert_eq!(entry.version.as_deref(), Some("3.1.4"));
        assert_eq!(entry.marketplace, "mp1");
        assert_eq!(entry.plugin, "myplugin");
    }
}

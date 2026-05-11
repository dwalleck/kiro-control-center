//! Test fixtures for exercising [`MarketplaceService`] in downstream
//! crates' unit tests.
//!
//! Available when the `test-support` feature is enabled or when
//! running the crate's own tests. Downstream crates (Tauri, CLI)
//! activate the feature via a `dev-dependencies` override on
//! `kiro-market-core` so the fixtures land only in test builds.

// Fails the build if `test-support` is enabled in a release, non-test
// context. Catches the specific misuse of adding
// `kiro-market-core = { features = ["test-support"] }` under a downstream
// crate's `[dependencies]` (not `[dev-dependencies]`), which would ship
// `PanicOnNetworkBackend` into release binaries. `cargo test --release`
// on downstream crates is the one false positive; those callers should
// gate their release-test setup behind a local feature.
#[cfg(all(feature = "test-support", not(any(test, debug_assertions))))]
compile_error!(
    "The `test-support` feature is for test dev-dependencies only. \
     Enabling it from `[dependencies]` or the default feature list would ship \
     `PanicOnNetworkBackend` into release binaries, where any browse-side \
     git operation would panic."
);

use std::path::{Path, PathBuf};

use tempfile::{TempDir, tempdir};

use crate::cache::CacheDir;
use crate::error::GitError;
use crate::git::{CloneOptions, GitBackend};
use crate::marketplace::{PluginEntry, PluginSource};
use crate::service::MarketplaceService;
use crate::validation::RelativePath;

/// A [`GitBackend`] that panics on any network operation. Browse and
/// listing tests never clone — reaching a network call means a bug in
/// the code under test, not a missing fixture.
#[derive(Clone, Copy, Debug, Default)]
pub struct PanicOnNetworkBackend;

impl GitBackend for PanicOnNetworkBackend {
    fn clone_repo(&self, _url: &str, _dest: &Path, _opts: &CloneOptions) -> Result<(), GitError> {
        panic!("browse-side tests must not clone");
    }

    fn pull_repo(&self, _path: &Path) -> Result<(), GitError> {
        panic!("browse-side tests must not pull");
    }

    fn verify_sha(&self, _path: &Path, _expected: &str) -> Result<(), GitError> {
        Ok(())
    }
}

/// Build a [`MarketplaceService`] backed by a temporary cache directory
/// and the panic-on-network git backend. The returned [`TempDir`] must
/// outlive the service — drop it last.
///
/// # Panics
///
/// Panics if the temporary directory cannot be created or its cache
/// subdirectories cannot be initialized. Test infrastructure only.
#[must_use]
pub fn temp_service() -> (TempDir, MarketplaceService) {
    let dir = tempdir().expect("tempdir");
    let cache = CacheDir::with_root(dir.path().to_path_buf());
    cache.ensure_dirs().expect("ensure_dirs");
    let svc = MarketplaceService::new(cache, PanicOnNetworkBackend);
    (dir, svc)
}

/// Build a plugin directory tree with `skills/<name>/SKILL.md` files
/// under `<root>/plugins/<plugin_name>/skills/`, matching the default
/// skill-discovery layout. Does not write a `plugin.json` — callers
/// that need one write it explicitly.
///
/// # Panics
///
/// Panics if any directory or file creation fails. Test infrastructure
/// only — callers should pass a freshly created temp directory.
pub fn make_plugin_with_skills(root: &Path, plugin_name: &str, skill_names: &[&str]) {
    let skills_root = root.join("plugins").join(plugin_name).join("skills");
    std::fs::create_dir_all(&skills_root).expect("create skills dir");
    for name in skill_names {
        let dir = skills_root.join(name);
        std::fs::create_dir_all(&dir).expect("create skill dir");
        std::fs::write(
            dir.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: test\n---\n"),
        )
        .expect("write SKILL.md");
    }
}

/// Construct a [`PluginEntry`] with a [`PluginSource::RelativePath`]
/// source. The relative path is validated through [`RelativePath::new`].
///
/// # Panics
///
/// Panics if `rel` is not a valid relative path. Callers are tests
/// passing known-good literals.
#[must_use]
pub fn relative_path_entry(name: &str, rel: &str) -> PluginEntry {
    PluginEntry {
        name: name.into(),
        description: None,
        source: PluginSource::RelativePath(RelativePath::new(rel).expect("valid relative path")),
    }
}

/// Build a `.kiro/`-rooted project directory under `dir` and return its
/// path as a UTF-8 string ready to pass through the Tauri FFI.
///
/// Mirrors the inline helper that previously lived in three Tauri
/// `commands/*::tests` modules (`steering.rs`, `browse.rs`, `agents.rs`).
/// Hoisted here so new command files (`plugins.rs`) can reuse the same
/// project-shape contract that
/// [`crate::commands::validate_kiro_project_path`](../../../../kiro-control-center/src-tauri/src/commands/mod.rs)
/// expects.
///
/// The created path is `<dir>/kproj/` with a `.kiro/` subdirectory; the
/// returned `String` points at `<dir>/kproj/`.
///
/// # Panics
///
/// Panics if directory creation fails or the path is not valid UTF-8
/// (the latter can't happen on `tempdir()`-rooted callers but is asserted
/// for symmetry with the original inline helpers). Test infrastructure only.
#[must_use]
pub fn make_kiro_project(dir: &Path) -> String {
    let project_path = dir.join("kproj");
    std::fs::create_dir_all(project_path.join(".kiro")).expect("create .kiro dir");
    project_path.to_str().expect("utf-8 path").to_owned()
}

/// Seed a marketplace's plugin registry directly to disk, bypassing
/// the real `marketplace.json` + fetch flow. Returns the marketplace
/// root path as a convenience for tests that then place plugin
/// directories under it; tests that only need the registry can bind
/// the return to `_`.
///
/// Reconstructs a sibling [`CacheDir`] pointing at the same root the
/// service was built with — [`CacheDir`] is stateless, so this is a
/// safe end-run around the service's private cache field without
/// exposing it.
///
/// # Panics
///
/// Panics if the marketplace directory cannot be created or the
/// registry cannot be written. Test infrastructure only.
#[must_use]
pub fn seed_marketplace_with_registry(
    cache_root: &Path,
    svc: &MarketplaceService,
    marketplace_name: &str,
    entries: &[PluginEntry],
) -> PathBuf {
    let marketplace_path = svc.marketplace_path(marketplace_name);
    std::fs::create_dir_all(&marketplace_path).expect("create marketplace root");
    let cache = CacheDir::with_root(cache_root.to_path_buf());
    cache
        .write_plugin_registry(marketplace_name, entries)
        .expect("write plugin registry");
    marketplace_path
}

/// Test helper: construct a [`MarketplaceName`](crate::validation::MarketplaceName)
/// from a string literal, panicking on validation failure. Test fixtures pass
/// `"mp"`, `"plug-a"`, etc. — values controlled by the test author, not user
/// input. A fixture failure is a bug, not an error to handle.
///
/// # Panics
///
/// Panics if `s` is not a valid marketplace name. Test infrastructure only —
/// callers pass known-good literals.
#[cfg(any(test, feature = "test-support"))]
#[must_use]
pub fn mp(s: &str) -> crate::validation::MarketplaceName {
    crate::validation::MarketplaceName::new(s)
        .unwrap_or_else(|e| panic!("test fixture: invalid marketplace name {s:?}: {e}"))
}

/// Test helper: construct a [`PluginName`](crate::validation::PluginName) from
/// a string literal. See [`mp`] for the contract.
///
/// # Panics
///
/// Panics if `s` is not a valid plugin name. Test infrastructure only —
/// callers pass known-good literals.
#[cfg(any(test, feature = "test-support"))]
#[must_use]
pub fn pn(s: &str) -> crate::validation::PluginName {
    crate::validation::PluginName::new(s)
        .unwrap_or_else(|e| panic!("test fixture: invalid plugin name {s:?}: {e}"))
}

/// Test helper: construct an [`AgentName`](crate::validation::AgentName) from a
/// string literal. Sibling to [`mp`] / [`pn`] — same fixture-only contract.
///
/// # Panics
///
/// Panics if `s` is not a valid agent name. Test infrastructure only —
/// callers pass known-good literals.
#[cfg(any(test, feature = "test-support"))]
#[must_use]
pub fn agent_name(s: &str) -> crate::validation::AgentName {
    crate::validation::AgentName::new(s)
        .unwrap_or_else(|e| panic!("test fixture: invalid agent name {s:?}: {e}"))
}

/// Construct an [`AgentInstallContext`](crate::service::AgentInstallContext)
/// with the defaults every test cares about: `InstallMode::New`, MCP gate
/// closed, no version pin. Override individual fields via struct-update
/// syntax (`AgentInstallContext { accept_mcp: true, ..default_install_ctx(&m, &p) }`).
///
/// Bind `marketplace` and `plugin` first so the borrows outlive the
/// returned struct:
///
/// ```ignore
/// let market = mp("mp");
/// let plug = pn("p");
/// let ctx = default_install_ctx(&market, &plug);
/// ```
///
/// The struct is `Copy`, so the caller can pass it by value into the
/// service entry points without a clone.
#[cfg(any(test, feature = "test-support"))]
#[must_use]
pub fn default_install_ctx<'a>(
    marketplace: &'a crate::validation::MarketplaceName,
    plugin: &'a crate::validation::PluginName,
) -> crate::service::AgentInstallContext<'a> {
    crate::service::AgentInstallContext {
        mode: crate::service::InstallMode::New,
        accept_mcp: false,
        marketplace,
        plugin,
        version: None,
    }
}

/// Build a native (`format: "kiro-cli"`) plugin source tree on disk:
/// writes `<root>/<scan>/<agent_name>.json` and, when `companion` is
/// `Some(rel)`, writes the companion body at `<root>/<scan>/<rel>` and
/// references it from the agent JSON's `prompt` field via `file://./<rel>`.
/// Returns the scan-root path so callers can drop additional files (a
/// second agent, a sibling companion) into the same scan.
///
/// `companion = None` produces an agent with an empty `prompt` field;
/// most native install paths reject such an agent at parse, so the `None`
/// variant is useful only for tests that want to exercise pre-validation
/// rejection (e.g. multi-scan-root rejection, which fires before parse).
///
/// Replaces three near-identical inlined fixtures in
/// `service::tests::install_plugin_agents_native_*` (the orphan,
/// cross-plugin, and multi-scan-root tests each rebuilt the shape).
///
/// # Panics
///
/// Panics on any filesystem failure. Test infrastructure only — callers
/// pass freshly created temp directories.
#[cfg(any(test, feature = "test-support"))]
// Returns the scan dir for callers that need it (e.g. to write extra
// fixture files); callers that just want the side effect can discard
// the value, so opt out of `must_use_candidate` here.
#[allow(clippy::must_use_candidate)]
pub fn make_native_plugin_dir(
    root: &Path,
    scan: &str,
    agent_name: &str,
    companion: Option<&str>,
) -> PathBuf {
    let scan_dir = root.join(scan);
    std::fs::create_dir_all(&scan_dir).expect("create native scan dir");
    let prompt = companion
        .map(|rel| format!("file://./{rel}"))
        .unwrap_or_default();
    std::fs::write(
        scan_dir.join(format!("{agent_name}.json")),
        format!(r#"{{"name":"{agent_name}","prompt":"{prompt}"}}"#),
    )
    .expect("write native agent json");
    if let Some(rel) = companion {
        let companion_path = scan_dir.join(rel);
        if let Some(parent) = companion_path.parent() {
            std::fs::create_dir_all(parent).expect("create companion parent dir");
        }
        std::fs::write(&companion_path, b"prompt body").expect("write companion body");
    }
    scan_dir
}

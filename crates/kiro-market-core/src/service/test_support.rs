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

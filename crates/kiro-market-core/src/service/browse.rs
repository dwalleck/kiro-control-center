//! Browse-side service methods: enumerate skills across marketplaces and
//! plugins, cross-referenced with the target project's installed set.
//!
//! Frontends (CLI, Tauri) remain thin wrappers — they decide how to
//! construct the [`MarketplaceService`] and how to frame errors, but
//! they do not duplicate the enumeration loop or the per-skill
//! frontmatter-parsing logic.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;
use tracing::{debug, warn};

use crate::error::{Error, PluginError};
use crate::marketplace::{PluginEntry, PluginSource};
use crate::plugin::{PluginManifest, discover_skill_dirs};
use crate::project::InstalledSkills;
use crate::service::MarketplaceService;
use crate::skill::parse_frontmatter;

// ---------------------------------------------------------------------------
// Response types
// ---------------------------------------------------------------------------

/// Information about a single skill, cross-referenced with the target
/// project's installed set.
///
/// `installed` is a point-in-time snapshot — the project's
/// `.kiro/installed.json` at the moment the listing was built. Callers
/// that want a live view must re-query.
#[derive(Clone, Debug, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct SkillInfo {
    pub name: String,
    pub description: String,
    pub plugin: String,
    pub marketplace: String,
    pub installed: bool,
}

/// Result of a marketplace-wide skill listing. The bulk path continues
/// past per-plugin errors (missing directory, malformed manifest) to
/// preserve the partial listing; `skipped` records those errors so the
/// frontend can show a warning rather than silently dropping plugins.
#[derive(Clone, Debug, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct BulkSkillsResult {
    pub skills: Vec<SkillInfo>,
    pub skipped: Vec<SkippedPlugin>,
}

/// A plugin that was excluded from a bulk listing, with the reason.
#[derive(Clone, Debug, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct SkippedPlugin {
    pub name: String,
    pub reason: String,
}

// ---------------------------------------------------------------------------
// Service methods
// ---------------------------------------------------------------------------

impl MarketplaceService {
    /// Resolve a plugin's on-disk location, local-only. Returns
    /// [`PluginError::RemoteSourceNotLocal`] for structured sources
    /// rather than cloning them — browse and list paths never want
    /// network I/O.
    ///
    /// Distinct from [`MarketplaceService::resolve_plugin_dir`], which
    /// clones remote sources on demand. Callers that can't tolerate a
    /// clone (enumerations, counts, read-only listings) use this
    /// method; callers that expect the directory to exist one way or
    /// another (install, update) use the cloning variant.
    ///
    /// # Errors
    ///
    /// - [`Error::Plugin`] ([`PluginError::DirectoryMissing`]) if a
    ///   `RelativePath` points to a missing directory.
    /// - [`Error::Plugin`] ([`PluginError::RemoteSourceNotLocal`]) if
    ///   the source is structured (GitHub / Git URL / Git subdir).
    pub fn resolve_local_plugin_dir(
        &self,
        entry: &PluginEntry,
        marketplace_path: &Path,
    ) -> Result<PathBuf, Error> {
        match &entry.source {
            PluginSource::RelativePath(rel) => {
                // `rel` is a validated `RelativePath` — no traversal
                // check needed. `symlink_metadata` refuses to follow
                // symlinks, matching the hardening in
                // `resolve_plugin_dir`.
                let resolved = marketplace_path.join(rel);
                let is_real_dir = fs::symlink_metadata(&resolved).is_ok_and(|m| m.is_dir());
                if !is_real_dir {
                    return Err(PluginError::DirectoryMissing { path: resolved }.into());
                }
                Ok(resolved)
            }
            PluginSource::Structured(_) => Err(PluginError::RemoteSourceNotLocal {
                plugin: entry.name.clone(),
            }
            .into()),
        }
    }

    /// List every skill defined by a single plugin, cross-referenced
    /// with the project's installed set.
    ///
    /// Per-skill errors inside a working plugin (unreadable `SKILL.md`,
    /// malformed frontmatter) are skipped silently with a `warn`. A
    /// plugin-level error (missing directory, malformed manifest,
    /// remote source) propagates — callers who selected this plugin
    /// explicitly should see a real error rather than an empty list.
    ///
    /// # Errors
    ///
    /// - [`Error::Marketplace`] / [`Error::Plugin`] from
    ///   [`Self::list_plugin_entries`] (unknown marketplace,
    ///   corrupt manifest).
    /// - [`Error::Plugin`] ([`PluginError::NotFound`]) if `plugin`
    ///   does not appear in the marketplace.
    /// - [`Error::Plugin`] ([`PluginError::DirectoryMissing`] /
    ///   [`PluginError::InvalidManifest`] /
    ///   [`PluginError::RemoteSourceNotLocal`]) for plugin-level
    ///   resolution failures.
    pub fn list_skills_for_plugin(
        &self,
        marketplace: &str,
        plugin: &str,
        installed: &InstalledSkills,
    ) -> Result<Vec<SkillInfo>, Error> {
        let marketplace_path = self.marketplace_path(marketplace);
        let plugin_entries = self.list_plugin_entries(marketplace)?;

        let plugin_entry = plugin_entries
            .iter()
            .find(|p| p.name == plugin)
            .ok_or_else(|| {
                Error::Plugin(PluginError::NotFound {
                    plugin: plugin.to_owned(),
                    marketplace: marketplace.to_owned(),
                })
            })?;

        let mut out: Vec<SkillInfo> = Vec::new();
        collect_skills_for_plugin_into(
            self,
            plugin_entry,
            &marketplace_path,
            marketplace,
            installed,
            &mut out,
        )?;
        Ok(out)
    }

    /// List every skill across every plugin in a marketplace,
    /// cross-referenced with the project's installed set.
    ///
    /// Plugin-level errors (missing directory, malformed manifest,
    /// remote source) are folded into [`BulkSkillsResult::skipped`]
    /// so a single bad plugin doesn't hide its siblings. Per-skill
    /// errors inside a working plugin are still warned-and-skipped,
    /// matching [`Self::list_skills_for_plugin`].
    ///
    /// The `skills` and `skipped` vectors are pre-allocated with the
    /// plugin count as a baseline — `skills` usually grows past it
    /// (multiple skills per plugin) and `skipped` is bounded above
    /// by it, so this avoids the first few reallocations in the
    /// common case.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Marketplace`] / [`Error::Plugin`] from
    /// [`Self::list_plugin_entries`] when the marketplace is unknown
    /// or its manifest is corrupt. Non-plugin-level errors during
    /// iteration propagate; plugin-level errors go to `skipped`.
    pub fn list_all_skills(
        &self,
        marketplace: &str,
        installed: &InstalledSkills,
    ) -> Result<BulkSkillsResult, Error> {
        let marketplace_path = self.marketplace_path(marketplace);
        let plugin_entries = self.list_plugin_entries(marketplace)?;

        let mut skills: Vec<SkillInfo> = Vec::with_capacity(plugin_entries.len());
        let mut skipped: Vec<SkippedPlugin> = Vec::with_capacity(plugin_entries.len());

        for plugin_entry in &plugin_entries {
            match collect_skills_for_plugin_into(
                self,
                plugin_entry,
                &marketplace_path,
                marketplace,
                installed,
                &mut skills,
            ) {
                Ok(()) => {}
                Err(err) if is_plugin_level_skip(&err) => {
                    let reason = err.to_string();
                    warn!(
                        plugin = %plugin_entry.name,
                        error = %reason,
                        "skipping plugin in bulk skill listing"
                    );
                    skipped.push(SkippedPlugin {
                        name: plugin_entry.name.clone(),
                        reason,
                    });
                }
                Err(other) => return Err(other),
            }
        }

        Ok(BulkSkillsResult { skills, skipped })
    }
}

/// Is this error one that the bulk path should fold into `skipped`
/// rather than propagate? `true` for plugin-level resolution failures
/// (missing directory, malformed manifest, remote source); `false`
/// for anything else — corrupt marketplace state, unexpected I/O
/// errors, etc., which must surface so callers see them.
fn is_plugin_level_skip(err: &Error) -> bool {
    matches!(
        err,
        Error::Plugin(
            PluginError::DirectoryMissing { .. }
                | PluginError::InvalidManifest { .. }
                | PluginError::RemoteSourceNotLocal { .. }
        )
    )
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Append every skill defined by `plugin_entry` to `out`, cross-referenced
/// against `installed`. Plugin-level errors (missing dir, malformed
/// manifest, remote source) propagate as `Err`; per-skill errors
/// (unreadable `SKILL.md`, malformed frontmatter) are logged and skipped.
///
/// Shared between the per-plugin and bulk public entry points so the
/// per-skill skip philosophy and plugin-level error classification live
/// in exactly one place.
pub(super) fn collect_skills_for_plugin_into(
    service: &MarketplaceService,
    plugin_entry: &PluginEntry,
    marketplace_path: &Path,
    marketplace_name: &str,
    installed: &InstalledSkills,
    out: &mut Vec<SkillInfo>,
) -> Result<(), Error> {
    let plugin_dir = service.resolve_local_plugin_dir(plugin_entry, marketplace_path)?;
    let plugin_manifest = load_plugin_manifest(&plugin_dir)?;
    let skill_dirs = discover_skills_for_plugin(&plugin_dir, plugin_manifest.as_ref());
    out.reserve(skill_dirs.len());

    for skill_dir in &skill_dirs {
        let skill_md_path = skill_dir.join("SKILL.md");
        let content = match fs::read_to_string(&skill_md_path) {
            Ok(c) => c,
            Err(e) => {
                warn!(
                    path = %skill_md_path.display(),
                    error = %e,
                    "failed to read SKILL.md, skipping"
                );
                continue;
            }
        };

        let (frontmatter, _body_offset) = match parse_frontmatter(&content) {
            Ok(result) => result,
            Err(e) => {
                warn!(
                    path = %skill_md_path.display(),
                    error = %e,
                    "failed to parse SKILL.md frontmatter, skipping"
                );
                continue;
            }
        };

        let is_installed = installed.skills.contains_key(&frontmatter.name);
        out.push(SkillInfo {
            name: frontmatter.name,
            description: frontmatter.description,
            plugin: plugin_entry.name.clone(),
            marketplace: marketplace_name.to_owned(),
            installed: is_installed,
        });
    }

    Ok(())
}

/// Resolve the skill-discovery paths for a plugin. Uses
/// `manifest.skills` when the manifest specifies any, otherwise falls
/// back to [`crate::DEFAULT_SKILL_PATHS`]. The manifest-empty-list case
/// also falls back — an empty `skills` field means "no custom paths",
/// not "no skills."
fn discover_skills_for_plugin(
    plugin_dir: &Path,
    manifest: Option<&PluginManifest>,
) -> Vec<PathBuf> {
    let skill_paths: Vec<&str> = if let Some(m) = manifest.filter(|m| !m.skills.is_empty()) {
        m.skills.iter().map(String::as_str).collect()
    } else {
        crate::DEFAULT_SKILL_PATHS.to_vec()
    };

    discover_skill_dirs(plugin_dir, &skill_paths)
}

/// Load a `plugin.json` from the given directory.
///
/// Returns `Ok(None)` if the file is genuinely missing (not an error —
/// the plugin uses defaults) and `Err(PluginError::InvalidManifest)` if
/// the file exists but could not be parsed. I/O errors other than
/// `NotFound` propagate as [`Error::Io`].
fn load_plugin_manifest(plugin_dir: &Path) -> Result<Option<PluginManifest>, Error> {
    let manifest_path = plugin_dir.join("plugin.json");
    let bytes = match fs::read(&manifest_path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            debug!(
                path = %manifest_path.display(),
                "plugin.json not found, using defaults"
            );
            return Ok(None);
        }
        Err(e) => {
            warn!(
                path = %manifest_path.display(),
                error = %e,
                "failed to read plugin.json"
            );
            return Err(Error::Io(e));
        }
    };

    match PluginManifest::from_json(&bytes) {
        Ok(manifest) => {
            debug!(name = %manifest.name, "loaded plugin manifest");
            Ok(Some(manifest))
        }
        Err(e) => {
            warn!(
                path = %manifest_path.display(),
                error = %e,
                "plugin.json is malformed"
            );
            Err(PluginError::InvalidManifest {
                path: manifest_path,
                reason: e.to_string(),
            }
            .into())
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::path::Path;

    use tempfile::tempdir;

    use super::*;
    use crate::cache::CacheDir;
    use crate::error::GitError;
    use crate::git::{CloneOptions, GitBackend};
    use crate::marketplace::{PluginSource, StructuredSource};
    use crate::validation::RelativePath;

    // -----------------------------------------------------------------------
    // Test fixtures
    // -----------------------------------------------------------------------

    /// A `GitBackend` that panics on any network operation — browse-side
    /// tests never clone, so any call means a bug in the code under test.
    #[derive(Default)]
    struct PanicOnNetworkBackend;

    impl GitBackend for PanicOnNetworkBackend {
        fn clone_repo(
            &self,
            _url: &str,
            _dest: &Path,
            _opts: &CloneOptions,
        ) -> Result<(), GitError> {
            panic!("browse-side tests must not clone");
        }

        fn pull_repo(&self, _path: &Path) -> Result<(), GitError> {
            panic!("browse-side tests must not pull");
        }

        fn verify_sha(&self, _path: &Path, _expected: &str) -> Result<(), GitError> {
            Ok(())
        }
    }

    fn temp_service() -> (tempfile::TempDir, MarketplaceService) {
        let dir = tempdir().expect("tempdir");
        let cache = CacheDir::with_root(dir.path().to_path_buf());
        cache.ensure_dirs().expect("ensure_dirs");
        let svc = MarketplaceService::new(cache, PanicOnNetworkBackend);
        (dir, svc)
    }

    /// Build a plugin directory tree with `skills/<name>/SKILL.md` files
    /// under `<root>/plugins/<plugin_name>/skills/`, matching the
    /// default skill-discovery layout.
    fn make_plugin_with_skills(root: &Path, plugin_name: &str, skill_names: &[&str]) {
        let skills_root = root.join("plugins").join(plugin_name).join("skills");
        fs::create_dir_all(&skills_root).expect("create skills dir");
        for name in skill_names {
            let dir = skills_root.join(name);
            fs::create_dir_all(&dir).expect("create skill dir");
            fs::write(
                dir.join("SKILL.md"),
                format!("---\nname: {name}\ndescription: test\n---\n"),
            )
            .expect("write SKILL.md");
        }
    }

    fn relative_path_entry(name: &str, rel: &str) -> PluginEntry {
        PluginEntry {
            name: name.into(),
            description: None,
            source: PluginSource::RelativePath(RelativePath::new(rel).unwrap()),
        }
    }

    // -----------------------------------------------------------------------
    // resolve_local_plugin_dir
    // -----------------------------------------------------------------------

    #[test]
    fn resolve_local_plugin_dir_relative_path_exists() {
        let (dir, svc) = temp_service();
        let marketplace_path = dir.path().join("marketplace");
        let plugin_dir = marketplace_path.join("plugins/my-plugin");
        fs::create_dir_all(&plugin_dir).expect("create plugin dir");

        let entry = relative_path_entry("my-plugin", "plugins/my-plugin");
        let resolved = svc
            .resolve_local_plugin_dir(&entry, &marketplace_path)
            .expect("happy path");
        assert_eq!(resolved, plugin_dir);
    }

    #[test]
    fn resolve_local_plugin_dir_missing_returns_directory_missing() {
        let (dir, svc) = temp_service();
        let marketplace_path = dir.path().join("marketplace");
        fs::create_dir_all(&marketplace_path).expect("create marketplace root");

        let entry = relative_path_entry("ghost", "plugins/ghost");
        let err = svc
            .resolve_local_plugin_dir(&entry, &marketplace_path)
            .expect_err("missing dir must error");
        assert!(
            matches!(err, Error::Plugin(PluginError::DirectoryMissing { .. })),
            "expected DirectoryMissing, got: {err:?}"
        );
    }

    #[test]
    fn resolve_local_plugin_dir_structured_returns_remote_source_not_local() {
        let (dir, svc) = temp_service();
        let marketplace_path = dir.path().join("marketplace");

        let entry = PluginEntry {
            name: "remote".into(),
            description: None,
            source: PluginSource::Structured(StructuredSource::GitHub {
                repo: "owner/repo".into(),
                git_ref: None,
                sha: None,
            }),
        };

        let err = svc
            .resolve_local_plugin_dir(&entry, &marketplace_path)
            .expect_err("structured source must refuse local resolution");
        assert!(
            matches!(err, Error::Plugin(PluginError::RemoteSourceNotLocal { .. })),
            "expected RemoteSourceNotLocal, got: {err:?}"
        );
    }

    // -----------------------------------------------------------------------
    // collect_skills_for_plugin_into (helper-level tests)
    // -----------------------------------------------------------------------

    #[test]
    fn collect_skills_for_plugin_into_happy_path() {
        let (dir, svc) = temp_service();
        make_plugin_with_skills(dir.path(), "good", &["alpha", "beta"]);
        let entry = relative_path_entry("good", "plugins/good");

        let mut out: Vec<SkillInfo> = Vec::new();
        let installed = InstalledSkills::default();
        collect_skills_for_plugin_into(&svc, &entry, dir.path(), "mp1", &installed, &mut out)
            .expect("happy path");

        assert_eq!(out.len(), 2);
        assert!(out.iter().any(|s| s.name == "alpha"));
        assert!(out.iter().any(|s| s.name == "beta"));
        assert!(
            out.iter()
                .all(|s| s.plugin == "good" && s.marketplace == "mp1")
        );
        assert!(out.iter().all(|s| !s.installed));
    }

    #[test]
    fn collect_skills_for_plugin_into_missing_dir_errors() {
        let (dir, svc) = temp_service();
        let entry = relative_path_entry("ghost", "plugins/ghost");

        let mut out: Vec<SkillInfo> = Vec::new();
        let installed = InstalledSkills::default();
        let err =
            collect_skills_for_plugin_into(&svc, &entry, dir.path(), "mp1", &installed, &mut out)
                .expect_err("missing dir must propagate");

        assert!(
            matches!(err, Error::Plugin(PluginError::DirectoryMissing { .. })),
            "expected DirectoryMissing, got: {err:?}"
        );
        assert!(out.is_empty());
    }

    #[test]
    fn collect_skills_for_plugin_into_malformed_manifest_errors() {
        let (dir, svc) = temp_service();
        let plugin_dir = dir.path().join("plugins").join("broken");
        fs::create_dir_all(&plugin_dir).expect("create plugin dir");
        fs::write(plugin_dir.join("plugin.json"), "{ not valid json").expect("write manifest");
        let entry = relative_path_entry("broken", "plugins/broken");

        let mut out: Vec<SkillInfo> = Vec::new();
        let installed = InstalledSkills::default();
        let err =
            collect_skills_for_plugin_into(&svc, &entry, dir.path(), "mp1", &installed, &mut out)
                .expect_err("malformed manifest must propagate");

        assert!(
            matches!(err, Error::Plugin(PluginError::InvalidManifest { .. })),
            "expected InvalidManifest, got: {err:?}"
        );
        assert!(out.is_empty());
    }

    #[test]
    fn collect_skills_for_plugin_into_skips_bad_frontmatter_and_continues() {
        let (dir, svc) = temp_service();
        let skills_dir = dir.path().join("plugins").join("mixed").join("skills");
        fs::create_dir_all(skills_dir.join("good-skill")).expect("create skill dir");
        fs::create_dir_all(skills_dir.join("bad-skill")).expect("create skill dir");
        fs::write(
            skills_dir.join("good-skill").join("SKILL.md"),
            "---\nname: good-skill\ndescription: works\n---\n",
        )
        .expect("write good skill");
        // Missing closing `---` makes frontmatter parsing fail.
        fs::write(
            skills_dir.join("bad-skill").join("SKILL.md"),
            "---\nname: bad\n",
        )
        .expect("write bad skill");
        let entry = relative_path_entry("mixed", "plugins/mixed");

        let mut out: Vec<SkillInfo> = Vec::new();
        let installed = InstalledSkills::default();
        collect_skills_for_plugin_into(&svc, &entry, dir.path(), "mp1", &installed, &mut out)
            .expect("per-skill errors should not propagate");

        assert_eq!(out.len(), 1, "bad frontmatter should be skipped, good kept");
        assert_eq!(out[0].name, "good-skill");
    }

    // -----------------------------------------------------------------------
    // list_skills_for_plugin (public API integration)
    // -----------------------------------------------------------------------

    #[test]
    fn list_skills_for_plugin_unknown_marketplace_errors() {
        let (_dir, svc) = temp_service();
        let installed = InstalledSkills::default();
        let err = svc
            .list_skills_for_plugin("does-not-exist", "foo", &installed)
            .expect_err("unknown marketplace must error");

        // MarketplaceError::NotFound or similar — the exact variant is
        // an implementation detail of list_plugin_entries; we only
        // guarantee the top-level Error::Marketplace shape here.
        assert!(
            matches!(err, Error::Marketplace(_)),
            "expected Error::Marketplace, got: {err:?}"
        );
    }

    // -----------------------------------------------------------------------
    // list_all_skills (bulk public API)
    // -----------------------------------------------------------------------

    /// Build a plugin-registry-backed marketplace so the bulk path can
    /// enumerate entries without a real `marketplace.json`.
    ///
    /// Reconstructs a sibling `CacheDir` pointing at the same root the
    /// service was built with — `CacheDir` is stateless, so this is a
    /// safe end-run around the service's private cache field without
    /// exposing it.
    fn seed_marketplace_with_registry(
        cache_root: &Path,
        svc: &MarketplaceService,
        marketplace_name: &str,
        entries: &[PluginEntry],
    ) -> PathBuf {
        let marketplace_path = svc.marketplace_path(marketplace_name);
        fs::create_dir_all(&marketplace_path).expect("create marketplace root");
        let cache = CacheDir::with_root(cache_root.to_path_buf());
        cache
            .write_plugin_registry(marketplace_name, entries)
            .expect("write plugin registry");
        marketplace_path
    }

    #[test]
    fn list_all_skills_happy_path_enumerates_across_plugins() {
        let (dir, svc) = temp_service();
        let entries = vec![
            relative_path_entry("alpha-plug", "plugins/alpha-plug"),
            relative_path_entry("beta-plug", "plugins/beta-plug"),
        ];
        let marketplace_path = seed_marketplace_with_registry(dir.path(), &svc, "mp1", &entries);
        make_plugin_with_skills(&marketplace_path, "alpha-plug", &["skill-a1", "skill-a2"]);
        make_plugin_with_skills(&marketplace_path, "beta-plug", &["skill-b1"]);

        let installed = InstalledSkills::default();
        let result = svc.list_all_skills("mp1", &installed).expect("happy path");

        assert_eq!(result.skills.len(), 3);
        assert!(result.skipped.is_empty());
        assert!(result.skills.iter().any(|s| s.name == "skill-a1"));
        assert!(result.skills.iter().any(|s| s.name == "skill-a2"));
        assert!(result.skills.iter().any(|s| s.name == "skill-b1"));
    }

    #[test]
    fn list_all_skills_skips_one_broken_plugin_keeps_the_rest() {
        let (dir, svc) = temp_service();
        let entries = vec![
            relative_path_entry("good", "plugins/good"),
            relative_path_entry("ghost", "plugins/ghost"),
        ];
        let marketplace_path = seed_marketplace_with_registry(dir.path(), &svc, "mp1", &entries);
        make_plugin_with_skills(&marketplace_path, "good", &["alpha"]);
        // Deliberately do not create `plugins/ghost` — it must land in
        // `skipped` rather than cause the whole bulk call to fail.

        let installed = InstalledSkills::default();
        let result = svc
            .list_all_skills("mp1", &installed)
            .expect("bulk call must succeed despite one broken plugin");

        assert_eq!(result.skills.len(), 1);
        assert_eq!(result.skills[0].name, "alpha");
        assert_eq!(result.skipped.len(), 1);
        assert_eq!(result.skipped[0].name, "ghost");
        assert!(
            result.skipped[0].reason.contains("does not exist"),
            "skipped reason should name the failure mode, got: {}",
            result.skipped[0].reason
        );
    }
}

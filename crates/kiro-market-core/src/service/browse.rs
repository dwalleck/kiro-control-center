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
    /// - [`Error::Plugin`] ([`PluginError::NotADirectory`]) if the path
    ///   exists but is a regular file (or other non-directory).
    /// - [`Error::Plugin`] ([`PluginError::SymlinkRefused`]) if the path
    ///   is a symlink — refused rather than followed as a security
    ///   measure.
    /// - [`Error::Plugin`] ([`PluginError::DirectoryUnreadable`]) if
    ///   stat'ing the path fails (permission denied, I/O error, etc.).
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
                // `resolve_plugin_dir`. Metadata outcomes split into
                // five arms: is_dir success, symlink → SymlinkRefused
                // (security refusal), non-directory → NotADirectory
                // (shape mismatch), NotFound → DirectoryMissing, and
                // other I/O → DirectoryUnreadable carrying the
                // underlying io::Error via #[source]. Splitting
                // NotFound from the catch-all ensures a permissions
                // problem surfaces as "could not access" with
                // ErrorKind preserved, not as a misleading "does not
                // exist."
                let resolved = marketplace_path.join(rel);
                match fs::symlink_metadata(&resolved) {
                    Ok(m) if m.file_type().is_symlink() => {
                        Err(PluginError::SymlinkRefused { path: resolved }.into())
                    }
                    Ok(m) if m.is_dir() => Ok(resolved),
                    Ok(_) => Err(PluginError::NotADirectory { path: resolved }.into()),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                        Err(PluginError::DirectoryMissing { path: resolved }.into())
                    }
                    Err(e) => Err(PluginError::DirectoryUnreadable {
                        path: resolved,
                        source: e,
                    }
                    .into()),
                }
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
    /// - [`Error::Marketplace`] / [`Error::Plugin`] / [`Error::Io`] /
    ///   [`Error::Json`] from [`Self::list_plugin_entries`] (unknown
    ///   marketplace, corrupt or unreadable registry).
    /// - [`Error::Plugin`] ([`PluginError::NotFound`]) if `plugin`
    ///   does not appear in the marketplace.
    /// - [`Error::Plugin`] ([`PluginError::DirectoryMissing`] /
    ///   [`PluginError::DirectoryUnreadable`] /
    ///   [`PluginError::InvalidManifest`] /
    ///   [`PluginError::ManifestReadFailed`] /
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
    /// Returns [`Error::Marketplace`] / [`Error::Plugin`] /
    /// [`Error::Io`] / [`Error::Json`] from
    /// [`Self::list_plugin_entries`] when the marketplace is unknown
    /// or its registry is corrupt / unreadable. Non-plugin-level
    /// errors during iteration propagate; plugin-level errors (see
    /// [`is_plugin_level_skip`]) go to `skipped`.
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
/// (missing, not-a-directory, symlinked, or unreadable directory;
/// malformed or unreadable manifest; remote source); `false` for
/// anything else — corrupt marketplace state, unexpected I/O errors
/// outside the per-plugin scope, etc., which must surface so callers
/// see them.
fn is_plugin_level_skip(err: &Error) -> bool {
    matches!(
        err,
        Error::Plugin(
            PluginError::DirectoryMissing { .. }
                | PluginError::NotADirectory { .. }
                | PluginError::SymlinkRefused { .. }
                | PluginError::DirectoryUnreadable { .. }
                | PluginError::InvalidManifest { .. }
                | PluginError::ManifestReadFailed { .. }
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
fn collect_skills_for_plugin_into(
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
                    marketplace = %marketplace_name,
                    plugin = %plugin_entry.name,
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
                    marketplace = %marketplace_name,
                    plugin = %plugin_entry.name,
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
/// Returns:
/// - `Ok(Some(manifest))` on success.
/// - `Ok(None)` when the file is genuinely absent (`NotFound`) or when
///   it is a symlink — a symlinked `plugin.json` inside an untrusted
///   cloned repository could point at arbitrary host files, so it is
///   treated as absent with a `warn!`. Matches the hardening in
///   `crate::commands::install::load_plugin_manifest` in the CLI crate.
/// - `Err(PluginError::InvalidManifest)` if the file exists but could
///   not be parsed.
/// - `Err(PluginError::ManifestReadFailed)` for any other read or stat
///   failure (permission denied, transient I/O, etc.). Classified as
///   plugin-level so bulk listings skip the plugin rather than aborting.
fn load_plugin_manifest(plugin_dir: &Path) -> Result<Option<PluginManifest>, Error> {
    let manifest_path = plugin_dir.join("plugin.json");

    // Refuse to follow symlinks. plugin_dir lives inside a cloned
    // (untrusted) repository; a symlinked plugin.json could leak host
    // file contents through the InvalidManifest error path's `reason`
    // field (which includes serde's parse error over the target bytes).
    match fs::symlink_metadata(&manifest_path) {
        Ok(m) if m.file_type().is_symlink() => {
            warn!(
                path = %manifest_path.display(),
                "plugin.json is a symlink, refusing to follow; treating as missing"
            );
            return Ok(None);
        }
        Ok(_) => {}
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
                "failed to stat plugin.json"
            );
            return Err(PluginError::ManifestReadFailed {
                path: manifest_path,
                source: e,
            }
            .into());
        }
    }

    let bytes = match fs::read(&manifest_path) {
        Ok(b) => b,
        Err(e) => {
            warn!(
                path = %manifest_path.display(),
                error = %e,
                "failed to read plugin.json"
            );
            return Err(PluginError::ManifestReadFailed {
                path: manifest_path,
                source: e,
            }
            .into());
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

    // -----------------------------------------------------------------------
    // Symlink-refusal regression tests (plugin dir + plugin.json)
    // -----------------------------------------------------------------------

    /// Regression guard: `resolve_local_plugin_dir` uses
    /// `symlink_metadata` combined with an explicit `is_symlink()`
    /// check rather than `Path::exists()`, so a symlink at the plugin
    /// path is classified as [`PluginError::SymlinkRefused`] rather
    /// than traversed. This test fails if the symlink arm is replaced
    /// by `Path::exists()` (which would follow the link) or by a
    /// weaker shape check (which would let the symlink fall through
    /// to [`PluginError::NotADirectory`] and hide the security
    /// semantic). Mirrors `resolve_plugin_dir_refuses_symlinked_relative_path`
    /// for the cloning sibling in `service/mod.rs`.
    #[cfg(unix)]
    #[test]
    fn resolve_local_plugin_dir_refuses_symlinked_relative_path() {
        let (dir, svc) = temp_service();
        let marketplace_path = dir.path().join("marketplace");
        fs::create_dir_all(&marketplace_path).expect("create marketplace root");

        let outside = dir.path().join("outside-marketplace");
        fs::create_dir_all(&outside).expect("create outside target");

        let link_path = marketplace_path.join("plugins").join("escape");
        fs::create_dir_all(link_path.parent().expect("plugins dir parent"))
            .expect("create plugins dir");
        std::os::unix::fs::symlink(&outside, &link_path).expect("create symlink");

        let entry = relative_path_entry("escape", "plugins/escape");
        let err = svc
            .resolve_local_plugin_dir(&entry, &marketplace_path)
            .expect_err("symlinked plugin directory must be refused");
        assert!(
            matches!(err, Error::Plugin(PluginError::SymlinkRefused { .. })),
            "expected SymlinkRefused for symlink, got: {err:?}"
        );
    }

    /// Regression guard: `load_plugin_manifest` treats a symlinked
    /// `plugin.json` as absent (matching the CLI-side
    /// `kiro_market::commands::install::load_plugin_manifest` and the
    /// agent discovery hardening). A symlinked manifest inside a cloned
    /// repo could leak host file contents through the `InvalidManifest`
    /// error path, which embeds the serde parse error over the target
    /// bytes.
    #[cfg(unix)]
    #[test]
    fn load_plugin_manifest_refuses_symlinked_manifest() {
        let tmp = tempdir().expect("tempdir");
        let plugin_dir = tmp.path().join("plugin");
        fs::create_dir_all(&plugin_dir).expect("create plugin dir");

        // A "sensitive" target with valid-looking JSON so we can tell
        // absence from "parsed but wrong."
        let sensitive = tmp.path().join("secrets.json");
        fs::write(&sensitive, br#"{"name":"leaked","version":"1.0"}"#).expect("write target");

        std::os::unix::fs::symlink(&sensitive, plugin_dir.join("plugin.json"))
            .expect("create symlink");

        let result = load_plugin_manifest(&plugin_dir).expect("symlink must be Ok(None)");
        assert!(
            result.is_none(),
            "symlinked plugin.json must be treated as absent, got: {result:?}"
        );
    }

    // -----------------------------------------------------------------------
    // resolve_local_plugin_dir: Unreadable vs Missing classification
    // -----------------------------------------------------------------------

    /// Regression guard: a regular file sitting at the plugin path
    /// must classify as [`PluginError::NotADirectory`] rather than
    /// [`PluginError::DirectoryMissing`] (which would mislead users
    /// into thinking the path is absent) or
    /// [`PluginError::DirectoryUnreadable`] (which implies an I/O
    /// failure and loses the structural semantic). Pins the four-way
    /// split on `resolve_local_plugin_dir`.
    #[test]
    fn resolve_local_plugin_dir_file_path_returns_not_a_directory() {
        let (dir, svc) = temp_service();
        let marketplace_path = dir.path().join("marketplace");
        fs::create_dir_all(marketplace_path.join("plugins")).expect("create plugins dir");
        fs::write(
            marketplace_path.join("plugins").join("not-a-dir"),
            b"this is a regular file",
        )
        .expect("write file");

        let entry = relative_path_entry("not-a-dir", "plugins/not-a-dir");
        let err = svc
            .resolve_local_plugin_dir(&entry, &marketplace_path)
            .expect_err("regular file must not resolve as a plugin directory");
        assert!(
            matches!(err, Error::Plugin(PluginError::NotADirectory { .. })),
            "expected NotADirectory for non-directory, got: {err:?}"
        );
    }

    // -----------------------------------------------------------------------
    // is_plugin_level_skip covers the new plugin-level variants
    // -----------------------------------------------------------------------

    /// Regression guard: the bulk path relies on this classifier to
    /// decide skip-vs-propagate. Before the fix, `ManifestReadFailed`
    /// propagated as an `Error::Io` that slipped past the `matches!`,
    /// aborting the entire listing on one unreadable `plugin.json`.
    #[rstest::rstest]
    #[case::directory_missing(Error::Plugin(PluginError::DirectoryMissing {
        path: "/tmp/x".into(),
    }))]
    #[case::not_a_directory(Error::Plugin(PluginError::NotADirectory {
        path: "/tmp/x".into(),
    }))]
    #[case::symlink_refused(Error::Plugin(PluginError::SymlinkRefused {
        path: "/tmp/x".into(),
    }))]
    #[case::directory_unreadable(Error::Plugin(PluginError::DirectoryUnreadable {
        path: "/tmp/x".into(),
        source: std::io::Error::from(std::io::ErrorKind::PermissionDenied),
    }))]
    #[case::invalid_manifest(Error::Plugin(PluginError::InvalidManifest {
        path: "/tmp/x/plugin.json".into(),
        reason: "missing name".into(),
    }))]
    #[case::manifest_read_failed(Error::Plugin(PluginError::ManifestReadFailed {
        path: "/tmp/x/plugin.json".into(),
        source: std::io::Error::from(std::io::ErrorKind::PermissionDenied),
    }))]
    #[case::remote_source_not_local(Error::Plugin(PluginError::RemoteSourceNotLocal {
        plugin: "remote-plug".into(),
    }))]
    fn is_plugin_level_skip_accepts_plugin_level_variants(#[case] err: Error) {
        assert!(
            is_plugin_level_skip(&err),
            "expected bulk-path skip for: {err:?}"
        );
    }

    #[test]
    fn is_plugin_level_skip_rejects_non_plugin_errors() {
        let io_err = Error::Io(std::io::Error::other("disk full"));
        assert!(
            !is_plugin_level_skip(&io_err),
            "generic I/O errors must propagate, not skip"
        );
    }

    // -----------------------------------------------------------------------
    // list_skills_for_plugin: happy path + NotFound branch + installed
    // -----------------------------------------------------------------------

    /// Single installed-skill fixture so the cross-reference branch
    /// `installed.skills.contains_key(&frontmatter.name) == true` gets
    /// exercised. All production `SkillInfo.installed` consumers depend
    /// on this being correct; historically every test used
    /// `InstalledSkills::default()`, so only the `false` branch was
    /// covered.
    fn installed_with(skill_name: &str, plugin: &str, marketplace: &str) -> InstalledSkills {
        use std::collections::HashMap;

        use chrono::Utc;

        use crate::project::InstalledSkillMeta;

        let mut skills = HashMap::new();
        skills.insert(
            skill_name.to_owned(),
            InstalledSkillMeta {
                marketplace: marketplace.to_owned(),
                plugin: plugin.to_owned(),
                version: None,
                installed_at: Utc::now(),
            },
        );
        InstalledSkills { skills }
    }

    #[test]
    fn list_skills_for_plugin_happy_path() {
        let (dir, svc) = temp_service();
        let entries = vec![relative_path_entry("alpha", "plugins/alpha")];
        let marketplace_path = seed_marketplace_with_registry(dir.path(), &svc, "mp1", &entries);
        make_plugin_with_skills(&marketplace_path, "alpha", &["skill-a"]);

        let installed = InstalledSkills::default();
        let skills = svc
            .list_skills_for_plugin("mp1", "alpha", &installed)
            .expect("happy path");

        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "skill-a");
        assert_eq!(skills[0].plugin, "alpha");
        assert_eq!(skills[0].marketplace, "mp1");
        assert!(!skills[0].installed);
    }

    #[test]
    fn list_skills_for_plugin_unknown_plugin_errors() {
        let (dir, svc) = temp_service();
        let entries = vec![relative_path_entry("alpha", "plugins/alpha")];
        let marketplace_path = seed_marketplace_with_registry(dir.path(), &svc, "mp1", &entries);
        make_plugin_with_skills(&marketplace_path, "alpha", &["skill-a"]);

        let installed = InstalledSkills::default();
        let err = svc
            .list_skills_for_plugin("mp1", "does-not-exist", &installed)
            .expect_err("unknown plugin must error");

        assert!(
            matches!(
                err,
                Error::Plugin(PluginError::NotFound { ref plugin, .. })
                    if plugin == "does-not-exist"
            ),
            "expected PluginError::NotFound, got: {err:?}"
        );
    }

    #[test]
    fn list_skills_for_plugin_marks_installed_skills_true() {
        let (dir, svc) = temp_service();
        let entries = vec![relative_path_entry("alpha", "plugins/alpha")];
        let marketplace_path = seed_marketplace_with_registry(dir.path(), &svc, "mp1", &entries);
        make_plugin_with_skills(&marketplace_path, "alpha", &["already-installed", "fresh"]);

        let installed = installed_with("already-installed", "alpha", "mp1");
        let skills = svc
            .list_skills_for_plugin("mp1", "alpha", &installed)
            .expect("happy path");

        let marked_installed: Vec<_> = skills.iter().filter(|s| s.installed).collect();
        assert_eq!(marked_installed.len(), 1);
        assert_eq!(marked_installed[0].name, "already-installed");
        assert!(
            skills.iter().any(|s| s.name == "fresh" && !s.installed),
            "fresh skill should not be marked installed"
        );
    }

    // -----------------------------------------------------------------------
    // list_all_skills: additional skip branches + installed cross-ref
    // -----------------------------------------------------------------------

    /// Bulk path must fold a plugin with an unparseable `plugin.json`
    /// into `skipped`. Previously only the `DirectoryMissing` skip
    /// branch was covered; a narrowed classifier could pass CI without
    /// this.
    #[test]
    fn list_all_skills_skips_plugin_with_invalid_manifest() {
        let (dir, svc) = temp_service();
        let entries = vec![
            relative_path_entry("good", "plugins/good"),
            relative_path_entry("broken", "plugins/broken"),
        ];
        let marketplace_path = seed_marketplace_with_registry(dir.path(), &svc, "mp1", &entries);
        make_plugin_with_skills(&marketplace_path, "good", &["alpha"]);
        let broken_dir = marketplace_path.join("plugins").join("broken");
        fs::create_dir_all(&broken_dir).expect("create broken plugin dir");
        fs::write(broken_dir.join("plugin.json"), "{ not valid json")
            .expect("write malformed manifest");

        let installed = InstalledSkills::default();
        let result = svc
            .list_all_skills("mp1", &installed)
            .expect("bulk call must succeed with one broken plugin");

        assert_eq!(result.skills.len(), 1);
        assert_eq!(result.skills[0].name, "alpha");
        assert_eq!(result.skipped.len(), 1);
        assert_eq!(result.skipped[0].name, "broken");
        // TODO(#30): replace substring match with a pattern match on a
        // typed SkippedPlugin.reason enum once that refactor lands. The
        // current Display-string assertion is fragile to rewording.
        assert!(
            result.skipped[0].reason.contains("invalid plugin manifest"),
            "skipped reason should name the manifest failure, got: {}",
            result.skipped[0].reason
        );
    }

    /// Bulk path must fold a plugin whose source is remote into
    /// `skipped`, not propagate. Without this, listing a marketplace
    /// that mixes local and remote plugins would abort on the first
    /// remote entry.
    #[test]
    fn list_all_skills_skips_plugin_with_remote_source() {
        let (dir, svc) = temp_service();
        let local = relative_path_entry("local", "plugins/local");
        let remote = PluginEntry {
            name: "remote".into(),
            description: None,
            source: PluginSource::Structured(StructuredSource::GitHub {
                repo: "owner/repo".into(),
                git_ref: None,
                sha: None,
            }),
        };
        let marketplace_path =
            seed_marketplace_with_registry(dir.path(), &svc, "mp1", &[local, remote]);
        make_plugin_with_skills(&marketplace_path, "local", &["local-skill"]);

        let installed = InstalledSkills::default();
        let result = svc
            .list_all_skills("mp1", &installed)
            .expect("bulk call must succeed with one remote plugin");

        assert_eq!(result.skills.len(), 1);
        assert_eq!(result.skills[0].name, "local-skill");
        assert_eq!(result.skipped.len(), 1);
        assert_eq!(result.skipped[0].name, "remote");
        // TODO(#30): replace substring match with a pattern match on a
        // typed SkippedPlugin.reason enum once that refactor lands. The
        // current Display-string assertion is fragile to rewording.
        assert!(
            result.skipped[0].reason.contains("remote source"),
            "skipped reason should name the remote-source failure, got: {}",
            result.skipped[0].reason
        );
    }

    #[test]
    fn list_all_skills_marks_installed_skills_true() {
        let (dir, svc) = temp_service();
        let entries = vec![relative_path_entry("alpha", "plugins/alpha")];
        let marketplace_path = seed_marketplace_with_registry(dir.path(), &svc, "mp1", &entries);
        make_plugin_with_skills(&marketplace_path, "alpha", &["installed", "fresh"]);

        let installed = installed_with("installed", "alpha", "mp1");
        let result = svc.list_all_skills("mp1", &installed).expect("happy path");

        let marked: Vec<_> = result.skills.iter().filter(|s| s.installed).collect();
        assert_eq!(marked.len(), 1);
        assert_eq!(marked[0].name, "installed");
    }
}

//! Scan a plugin directory for agent markdown files.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use tracing::{debug, warn};

/// Files commonly found in `agents/` directories that are documentation,
/// not agents. Compared case-insensitively so `readme.md` is also excluded.
const EXCLUDED_FILENAMES: &[&str] = &["README.md", "CONTRIBUTING.md", "CHANGELOG.md"];

/// Find agent markdown files inside `plugin_dir` according to `scan_paths`.
///
/// `scan_paths` are relative to `plugin_dir`. Each entry is first validated
/// by [`crate::validation::validate_relative_path`] so a malicious plugin
/// manifest cannot escape the plugin root via absolute paths or `..`
/// components. Invalid entries are skipped with a `warn!`. This matches
/// the skill discovery guard in `plugin::discover_skill_dirs`.
///
/// Files whose extension is `md` (case-insensitive) are included; the
/// caller uses `detect_dialect` at parse time to route to the right
/// parser. Scans are non-recursive: only direct children of each scan
/// directory are considered. This avoids grabbing nested `prompts/*.md`
/// or editor backup files.
///
/// README / CONTRIBUTING / CHANGELOG are excluded by filename so plugins
/// can keep docs in their `agents/` directory without producing
/// parse-failure warnings. Other non-agent `.md` files (e.g. ad-hoc notes)
/// will still be picked up, parsed, and surfaced as `AgentParseFailed`
/// warnings — the service layer demotes the `MissingFrontmatter` flavor
/// specifically via a variant match on `ParseFailure`.
///
/// `read_dir` failures are handled narrowly: `NotFound` silently yields
/// an empty list (the scan dir may legitimately not exist), but every
/// other I/O error is logged via `warn!` so a permission-denied or
/// filesystem error cannot present as "no agents here".
#[must_use]
pub fn discover_agents_in_dirs(plugin_dir: &Path, scan_paths: &[String]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for rel in scan_paths {
        if let Err(e) = crate::validation::validate_relative_path(rel) {
            warn!(
                path = %rel,
                error = %e,
                "skipping agent scan path that fails validation"
            );
            continue;
        }
        let dir = plugin_dir.join(rel.trim_start_matches("./"));
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
            Err(e) => {
                warn!(
                    path = %dir.display(),
                    error = %e,
                    "failed to read agent scan directory; skipping"
                );
                continue;
            }
        };
        for entry in entries {
            let entry = match entry {
                Ok(e) => e,
                Err(e) => {
                    warn!(
                        dir = %dir.display(),
                        error = %e,
                        "failed to read directory entry; skipping"
                    );
                    continue;
                }
            };
            let path = entry.path();
            // Use symlink_metadata (does NOT follow symlinks) so a malicious
            // plugin cannot smuggle in an agent path that reads arbitrary
            // files via parse_agent_file. Matches project::copy_dir_recursive.
            let metadata = match fs::symlink_metadata(&path) {
                Ok(m) => m,
                Err(e) => {
                    warn!(
                        path = %path.display(),
                        error = %e,
                        "failed to stat agent candidate; skipping"
                    );
                    continue;
                }
            };
            if crate::platform::is_reparse_or_symlink(&metadata) {
                debug!(
                    path = %path.display(),
                    "skipping symlink or reparse point in agent scan directory"
                );
                continue;
            }
            if !metadata.file_type().is_file() {
                continue;
            }
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if EXCLUDED_FILENAMES
                .iter()
                .any(|excluded| excluded.eq_ignore_ascii_case(name))
            {
                continue;
            }
            if Path::new(name)
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("md"))
            {
                out.push(path);
            }
        }
    }
    out
}

/// A file produced by native discovery. Carries the source path along
/// with the resolved scan-root the file was discovered under, so the
/// install layer can compute destination-relative paths without
/// re-doing the join.
///
/// `#[non_exhaustive]` blocks external crates from constructing
/// arbitrary instances via struct literals. Production producers are
/// the discover functions in this module; tests in this crate
/// construct directly. Cross-crate consumers of this type only ever
/// receive instances from those producers.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct DiscoveredNativeFile {
    /// Absolute path to the source file inside the plugin.
    pub source: PathBuf,
    /// The resolved scan-path directory (e.g. `<plugin>/agents/`).
    pub scan_root: PathBuf,
}

/// Find native Kiro agent JSON candidates: `.json` files at the root of
/// each scan path. Mirrors the security model of [`discover_agents_in_dirs`]:
/// validates each scan path, refuses symlinks, excludes README/CONTRIBUTING/
/// CHANGELOG, non-recursive at the scan-path level.
#[must_use]
pub fn discover_native_kiro_agents_in_dirs(
    plugin_dir: &Path,
    scan_paths: &[String],
) -> Vec<DiscoveredNativeFile> {
    let mut out = Vec::new();
    for rel in scan_paths {
        if let Err(e) = crate::validation::validate_relative_path(rel) {
            warn!(
                path = %rel,
                error = %e,
                "skipping native agent scan path that fails validation"
            );
            continue;
        }
        let dir = plugin_dir.join(rel.trim_start_matches("./"));
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
            Err(e) => {
                warn!(
                    path = %dir.display(),
                    error = %e,
                    "failed to read native agent scan directory; skipping"
                );
                continue;
            }
        };
        for entry in entries {
            let entry = match entry {
                Ok(e) => e,
                Err(e) => {
                    warn!(
                        dir = %dir.display(),
                        error = %e,
                        "failed to read directory entry; skipping"
                    );
                    continue;
                }
            };
            let path = entry.path();
            let metadata = match fs::symlink_metadata(&path) {
                Ok(m) => m,
                Err(e) => {
                    warn!(
                        path = %path.display(),
                        error = %e,
                        "failed to stat native agent candidate; skipping"
                    );
                    continue;
                }
            };
            if crate::platform::is_reparse_or_symlink(&metadata) {
                debug!(
                    path = %path.display(),
                    "skipping symlink or reparse point in native agent scan directory"
                );
                continue;
            }
            if !metadata.file_type().is_file() {
                continue;
            }
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            // README/CONTRIBUTING/CHANGELOG with .json extension are excluded
            // case-insensitively by stem to mirror the .md exclusion.
            if EXCLUDED_FILENAMES.iter().any(|excluded| {
                let stem_excl = Path::new(excluded)
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or(excluded);
                let stem_name = Path::new(name)
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or(name);
                stem_excl.eq_ignore_ascii_case(stem_name)
            }) {
                continue;
            }
            if Path::new(name)
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("json"))
            {
                out.push(DiscoveredNativeFile {
                    source: path,
                    scan_root: dir.clone(),
                });
            }
        }
    }
    out
}

/// Find companion file candidates: any regular (non-symlink) file inside
/// subdirectories of a scan path, exactly one level deep.
///
/// Plugin-wide — not attributed to any specific agent. The install layer
/// treats the result as one atomic bundle owned by the plugin.
///
/// `scan_paths` are the same agent scan paths used by
/// [`discover_native_kiro_agents_in_dirs`]. README/CONTRIBUTING/CHANGELOG
/// are excluded by basename (case-insensitive).
#[must_use]
pub fn discover_native_companion_files(
    plugin_dir: &Path,
    scan_paths: &[String],
) -> Vec<DiscoveredNativeFile> {
    let mut out = Vec::new();
    for rel in scan_paths {
        if let Err(e) = crate::validation::validate_relative_path(rel) {
            warn!(
                path = %rel,
                error = %e,
                "skipping native companion scan path that fails validation"
            );
            continue;
        }
        let scan_root = plugin_dir.join(rel.trim_start_matches("./"));
        let entries = match fs::read_dir(&scan_root) {
            Ok(entries) => entries,
            Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
            Err(e) => {
                warn!(
                    path = %scan_root.display(),
                    error = %e,
                    "failed to read native companion scan directory; skipping"
                );
                continue;
            }
        };
        for entry in entries {
            let entry = match entry {
                Ok(e) => e,
                Err(e) => {
                    warn!(
                        dir = %scan_root.display(),
                        error = %e,
                        "failed to read directory entry; skipping"
                    );
                    continue;
                }
            };
            let subdir = entry.path();
            let md = match fs::symlink_metadata(&subdir) {
                Ok(m) => m,
                Err(e) => {
                    warn!(
                        path = %subdir.display(),
                        error = %e,
                        "failed to stat companion subdir candidate; skipping"
                    );
                    continue;
                }
            };
            if crate::platform::is_reparse_or_symlink(&md) || !md.file_type().is_dir() {
                continue;
            }
            collect_companion_subdir_files(&subdir, &scan_root, &mut out);
        }
    }
    out
}

/// Walk the immediate children of `subdir` and append regular non-symlink
/// files (excluding README/CONTRIBUTING/CHANGELOG) to `out` as
/// [`DiscoveredNativeFile`] entries rooted at `scan_root`.
fn collect_companion_subdir_files(
    subdir: &Path,
    scan_root: &Path,
    out: &mut Vec<DiscoveredNativeFile>,
) {
    let inner = match fs::read_dir(subdir) {
        Ok(i) => i,
        Err(e) => {
            warn!(
                path = %subdir.display(),
                error = %e,
                "failed to read companion subdir; skipping"
            );
            return;
        }
    };
    for inner_entry in inner {
        let inner_entry = match inner_entry {
            Ok(e) => e,
            Err(e) => {
                warn!(
                    dir = %subdir.display(),
                    error = %e,
                    "failed to read companion entry; skipping"
                );
                continue;
            }
        };
        let inner_path = inner_entry.path();
        let inner_md = match fs::symlink_metadata(&inner_path) {
            Ok(m) => m,
            Err(e) => {
                warn!(
                    path = %inner_path.display(),
                    error = %e,
                    "failed to stat companion file; skipping"
                );
                continue;
            }
        };
        if crate::platform::is_reparse_or_symlink(&inner_md) {
            debug!(
                path = %inner_path.display(),
                "skipping symlink or reparse point in companion subdir"
            );
            continue;
        }
        if !inner_md.file_type().is_file() {
            continue;
        }
        let Some(name) = inner_path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if EXCLUDED_FILENAMES
            .iter()
            .any(|excluded| excluded.eq_ignore_ascii_case(name))
        {
            continue;
        }
        out.push(DiscoveredNativeFile {
            source: inner_path,
            scan_root: scan_root.to_path_buf(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn discover_finds_both_md_and_agent_md() {
        let tmp = tempdir().unwrap();
        let agents = tmp.path().join("agents");
        fs::create_dir_all(&agents).unwrap();
        fs::write(agents.join("claude.md"), "---\nname: c\n---\n").unwrap();
        fs::write(agents.join("copilot.agent.md"), "---\nname: o\n---\n").unwrap();
        fs::write(agents.join("notes.txt"), "ignored").unwrap();

        let found = discover_agents_in_dirs(tmp.path(), &["./agents/".to_string()]);
        let names: Vec<_> = found
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        assert!(names.contains(&"claude.md".to_string()));
        assert!(names.contains(&"copilot.agent.md".to_string()));
        assert!(!names.contains(&"notes.txt".to_string()));
    }

    #[test]
    fn discover_returns_empty_when_directory_missing() {
        let tmp = tempdir().unwrap();
        let found = discover_agents_in_dirs(tmp.path(), &["./nope/".to_string()]);
        assert!(found.is_empty());
    }

    #[test]
    fn discover_excludes_readme_and_contributing_case_insensitive() {
        let tmp = tempdir().unwrap();
        let agents = tmp.path().join("agents");
        fs::create_dir_all(&agents).unwrap();
        fs::write(agents.join("README.md"), "# README").unwrap();
        fs::write(agents.join("CONTRIBUTING.md"), "# Contrib").unwrap();
        fs::write(agents.join("CHANGELOG.md"), "# Changelog").unwrap();
        fs::write(agents.join("readme.md"), "# lowercase readme").unwrap();
        fs::write(agents.join("real.md"), "---\nname: r\n---\n").unwrap();

        let found = discover_agents_in_dirs(tmp.path(), &["./agents/".to_string()]);
        let names: Vec<_> = found
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        assert_eq!(names, vec!["real.md"]);
    }

    #[test]
    fn discover_accepts_uppercase_md_extension() {
        let tmp = tempdir().unwrap();
        let agents = tmp.path().join("agents");
        fs::create_dir_all(&agents).unwrap();
        fs::write(agents.join("Uppercase.MD"), "---\nname: u\n---\n").unwrap();
        fs::write(agents.join("normal.md"), "---\nname: n\n---\n").unwrap();

        let found = discover_agents_in_dirs(tmp.path(), &["./agents/".to_string()]);
        assert_eq!(found.len(), 2, "both extensions should be matched");
    }

    #[test]
    fn discover_does_not_recurse_into_subdirectories() {
        // Prevents accidentally picking up nested `agents/prompts/*.md` or
        // backup directories.
        let tmp = tempdir().unwrap();
        let agents = tmp.path().join("agents");
        let nested = agents.join("archived");
        fs::create_dir_all(&nested).unwrap();
        fs::write(agents.join("top.md"), "---\nname: t\n---\n").unwrap();
        fs::write(nested.join("deep.md"), "---\nname: d\n---\n").unwrap();

        let found = discover_agents_in_dirs(tmp.path(), &["./agents/".to_string()]);
        let names: Vec<_> = found
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        assert_eq!(names, vec!["top.md"]);
    }

    #[test]
    fn discover_handles_relative_path_without_leading_dot_slash() {
        let tmp = tempdir().unwrap();
        let agents = tmp.path().join("agents");
        fs::create_dir_all(&agents).unwrap();
        fs::write(agents.join("x.md"), "---\nname: x\n---\n").unwrap();

        // Caller passes bare "agents/" (no ./), should still work.
        let found = discover_agents_in_dirs(tmp.path(), &["agents/".to_string()]);
        assert_eq!(found.len(), 1);
    }

    #[test]
    fn discover_rejects_path_traversal_in_scan_paths() {
        // A malicious plugin manifest claims agents live outside the plugin
        // root. The returned list must be empty; the warn! fires but does
        // not fail the call.
        let tmp = tempdir().unwrap();
        let plugin = tmp.path().join("plugin");
        fs::create_dir_all(&plugin).unwrap();

        // Prime a directory next to the plugin that would otherwise be
        // readable.
        let escape = tmp.path().join("secrets");
        fs::create_dir_all(&escape).unwrap();
        fs::write(escape.join("loot.md"), "---\nname: loot\n---\n").unwrap();

        let found = discover_agents_in_dirs(&plugin, &["../secrets/".to_string()]);
        assert!(
            found.is_empty(),
            "path traversal must not escape plugin root: {found:?}"
        );
    }

    #[test]
    fn discover_rejects_absolute_scan_paths() {
        let tmp = tempdir().unwrap();
        let plugin = tmp.path().join("plugin");
        fs::create_dir_all(&plugin).unwrap();

        let found = discover_agents_in_dirs(&plugin, &["/etc/".to_string()]);
        assert!(found.is_empty(), "absolute path must be rejected");
    }

    #[test]
    fn discover_scans_valid_paths_alongside_rejected_ones() {
        // Mixed list: one bad, one good. The good one still works.
        let tmp = tempdir().unwrap();
        let plugin = tmp.path().join("plugin");
        let agents = plugin.join("agents");
        fs::create_dir_all(&agents).unwrap();
        fs::write(agents.join("legit.md"), "---\nname: ok\n---\n").unwrap();

        let found = discover_agents_in_dirs(
            &plugin,
            &["../../etc/".to_string(), "./agents/".to_string()],
        );
        let names: Vec<_> = found
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        assert_eq!(names, vec!["legit.md"]);
    }

    #[cfg(unix)]
    #[test]
    fn discover_skips_symlinked_agent_files() {
        // A malicious plugin could drop a symlink `agents/evil.md -> /etc/passwd`.
        // discover_agents_in_dirs must refuse to surface it as an agent file.
        let tmp = tempdir().unwrap();
        let agents = tmp.path().join("agents");
        fs::create_dir_all(&agents).unwrap();
        fs::write(agents.join("legit.md"), "---\nname: ok\n---\n").unwrap();

        let target = tmp.path().join("secret.md");
        fs::write(&target, "---\nname: secret\n---\nclassified\n").unwrap();
        std::os::unix::fs::symlink(&target, agents.join("evil.md")).unwrap();

        let found = discover_agents_in_dirs(tmp.path(), &["./agents/".to_string()]);
        let names: Vec<_> = found
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        assert_eq!(
            names,
            vec!["legit.md"],
            "symlinked agent must not appear in discovery output"
        );
    }

    #[test]
    fn discover_scans_multiple_paths() {
        let tmp = tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("a")).unwrap();
        fs::create_dir_all(tmp.path().join("b")).unwrap();
        fs::write(tmp.path().join("a").join("x.md"), "---\nname: x\n---\n").unwrap();
        fs::write(tmp.path().join("b").join("y.md"), "---\nname: y\n---\n").unwrap();

        let found = discover_agents_in_dirs(tmp.path(), &["./a/".to_string(), "./b/".to_string()]);
        assert_eq!(found.len(), 2);
    }

    // -------------------------------------------------------------------
    // Native discovery
    // -------------------------------------------------------------------

    #[test]
    fn native_discovery_finds_json_files_at_scan_root() {
        let tmp = tempdir().unwrap();
        let agents = tmp.path().join("agents");
        fs::create_dir_all(&agents).unwrap();
        fs::write(agents.join("a.json"), b"{}").unwrap();
        fs::write(agents.join("b.json"), b"{}").unwrap();
        fs::write(agents.join("ignore.md"), b"---\nname: ignore\n---\n").unwrap();

        let found = discover_native_kiro_agents_in_dirs(tmp.path(), &["./agents/".to_string()]);

        let names: Vec<_> = found
            .iter()
            .map(|f| f.source.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert!(names.contains(&"a.json".to_string()));
        assert!(names.contains(&"b.json".to_string()));
        assert!(!names.contains(&"ignore.md".to_string()));
    }

    #[test]
    fn native_discovery_excludes_readme_case_insensitive() {
        let tmp = tempdir().unwrap();
        let agents = tmp.path().join("agents");
        fs::create_dir_all(&agents).unwrap();
        fs::write(agents.join("README.json"), b"{}").unwrap();
        fs::write(agents.join("readme.json"), b"{}").unwrap();
        fs::write(agents.join("real.json"), b"{}").unwrap();

        let found = discover_native_kiro_agents_in_dirs(tmp.path(), &["./agents/".to_string()]);

        let names: Vec<_> = found
            .iter()
            .map(|f| f.source.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["real.json"]);
    }

    #[test]
    fn native_discovery_rejects_path_traversal() {
        let tmp = tempdir().unwrap();
        let plugin = tmp.path().join("plugin");
        fs::create_dir_all(&plugin).unwrap();
        let escape = tmp.path().join("secrets");
        fs::create_dir_all(&escape).unwrap();
        fs::write(escape.join("loot.json"), b"{}").unwrap();

        let found = discover_native_kiro_agents_in_dirs(&plugin, &["../secrets/".to_string()]);

        assert!(found.is_empty(), "path traversal must be rejected");
    }

    #[cfg(unix)]
    #[test]
    fn native_discovery_skips_symlinks() {
        use std::os::unix::fs::symlink;
        let tmp = tempdir().unwrap();
        let agents = tmp.path().join("agents");
        fs::create_dir_all(&agents).unwrap();
        fs::write(agents.join("real.json"), b"{}").unwrap();

        let outside = tmp.path().join("outside.json");
        fs::write(&outside, b"{}").unwrap();
        symlink(&outside, agents.join("evil.json")).unwrap();

        let found = discover_native_kiro_agents_in_dirs(tmp.path(), &["./agents/".to_string()]);

        let names: Vec<_> = found
            .iter()
            .map(|f| f.source.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["real.json"]);
    }

    #[cfg(windows)]
    #[test]
    fn companion_discovery_skips_directory_junctions() {
        // Path::is_symlink() returns false for Windows directory junctions
        // (they're IO_REPARSE_TAG_MOUNT_POINT, not IO_REPARSE_TAG_SYMLINK),
        // so the broader is_reparse_or_symlink() check has to catch them.
        // A junction at agents/escape pointing outside the plugin tree
        // would otherwise let collect_companion_subdir_files walk into
        // arbitrary host directories and surface their files as
        // companion candidates.
        let tmp = tempdir().unwrap();
        let agents = tmp.path().join("agents");
        fs::create_dir_all(&agents).unwrap();
        // Create a real companion subdir with a real file.
        let real_subdir = agents.join("prompts");
        fs::create_dir_all(&real_subdir).unwrap();
        fs::write(real_subdir.join("a.md"), b"real").unwrap();

        // Junction target lives outside `agents/` and contains a file
        // the attacker wants surfaced as a companion.
        let outside = tmp.path().join("outside");
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("smuggled.md"), b"smuggled").unwrap();
        let junction_path = agents.join("escape");
        junction::create(&outside, &junction_path).expect("create junction");

        let found = discover_native_companion_files(tmp.path(), &["./agents/".to_string()]);

        let names: Vec<_> = found
            .iter()
            .map(|f| f.source.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        // Only the real prompt should appear. The junction must be
        // refused at the outer subdir loop, so its `smuggled.md` never
        // even reaches collect_companion_subdir_files.
        assert_eq!(names, vec!["a.md"]);
    }

    #[test]
    fn native_discovery_returns_scan_root_for_dest_path_computation() {
        let tmp = tempdir().unwrap();
        let agents = tmp.path().join("agents");
        fs::create_dir_all(&agents).unwrap();
        fs::write(agents.join("a.json"), b"{}").unwrap();

        let found = discover_native_kiro_agents_in_dirs(tmp.path(), &["./agents/".to_string()]);

        assert_eq!(found.len(), 1);
        assert_eq!(found[0].scan_root, agents);
    }

    // -------------------------------------------------------------------
    // Companion discovery
    // -------------------------------------------------------------------

    #[test]
    fn companion_discovery_finds_files_one_level_deep() {
        let tmp = tempdir().unwrap();
        let agents = tmp.path().join("agents");
        let prompts = agents.join("prompts");
        fs::create_dir_all(&prompts).unwrap();
        fs::write(prompts.join("a.md"), b"prompt a").unwrap();
        fs::write(prompts.join("b.md"), b"prompt b").unwrap();
        // A top-level .json (would be an agent, NOT a companion).
        fs::write(agents.join("agent.json"), b"{}").unwrap();

        let found = discover_native_companion_files(tmp.path(), &["./agents/".to_string()]);

        let names: Vec<_> = found
            .iter()
            .map(|f| f.source.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert!(names.contains(&"a.md".to_string()));
        assert!(names.contains(&"b.md".to_string()));
        assert!(!names.contains(&"agent.json".to_string()));
    }

    #[test]
    fn companion_discovery_does_not_recurse_more_than_one_level() {
        let tmp = tempdir().unwrap();
        let agents = tmp.path().join("agents");
        let nested = agents.join("prompts").join("nested");
        fs::create_dir_all(&nested).unwrap();
        fs::write(agents.join("prompts/top.md"), b"top").unwrap();
        fs::write(nested.join("deep.md"), b"deep").unwrap();

        let found = discover_native_companion_files(tmp.path(), &["./agents/".to_string()]);

        let names: Vec<_> = found
            .iter()
            .map(|f| f.source.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert!(names.contains(&"top.md".to_string()));
        assert!(!names.contains(&"deep.md".to_string()));
    }

    #[cfg(unix)]
    #[test]
    fn companion_discovery_skips_symlinks_in_subdir() {
        use std::os::unix::fs::symlink;
        let tmp = tempdir().unwrap();
        let prompts = tmp.path().join("agents/prompts");
        fs::create_dir_all(&prompts).unwrap();
        fs::write(prompts.join("real.md"), b"real").unwrap();
        let outside = tmp.path().join("outside.md");
        fs::write(&outside, b"outside").unwrap();
        symlink(&outside, prompts.join("evil.md")).unwrap();

        let found = discover_native_companion_files(tmp.path(), &["./agents/".to_string()]);

        let names: Vec<_> = found
            .iter()
            .map(|f| f.source.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["real.md"]);
    }

    #[test]
    fn companion_discovery_returns_empty_when_no_subdirs() {
        let tmp = tempdir().unwrap();
        let agents = tmp.path().join("agents");
        fs::create_dir_all(&agents).unwrap();
        fs::write(agents.join("only.json"), b"{}").unwrap();

        let found = discover_native_companion_files(tmp.path(), &["./agents/".to_string()]);
        assert!(found.is_empty());
    }
}

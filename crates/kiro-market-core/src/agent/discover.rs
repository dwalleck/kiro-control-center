//! Scan a plugin directory for agent markdown files.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use tracing::warn;

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
            if !path.is_file() {
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
}

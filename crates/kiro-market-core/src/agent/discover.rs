//! Scan a plugin directory for agent markdown files.

use std::fs;
use std::path::{Path, PathBuf};

/// Files commonly found in `agents/` directories that are documentation,
/// not agents. Excluded by name so plugins can keep READMEs alongside
/// their agent files without producing install-time warnings.
const EXCLUDED_FILENAMES: &[&str] = &["README.md", "CONTRIBUTING.md", "CHANGELOG.md"];

/// Find agent markdown files inside `plugin_dir` according to `scan_paths`.
///
/// `scan_paths` are relative to `plugin_dir`. Files ending in `.md` or
/// `.agent.md` are included; the caller uses `detect_dialect` at parse time
/// to route to the right parser. Scans are non-recursive: only direct
/// children of each scan directory are considered. This avoids grabbing
/// nested `prompts/*.md` or editor backup files.
///
/// Files listed in [`EXCLUDED_FILENAMES`] are skipped so shared
/// documentation doesn't surface as parse-failure warnings. Other
/// non-agent `.md` files (e.g. ad-hoc notes a plugin author drops in)
/// will still be picked up, parsed, and surfaced as `AgentParseFailed`
/// warnings. The service layer further demotes the "no frontmatter fence"
/// flavor of parse error to a debug log so it doesn't spam the user.
#[must_use]
pub fn discover_agents_in_dirs(plugin_dir: &Path, scan_paths: &[String]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for rel in scan_paths {
        let dir = plugin_dir.join(rel.trim_start_matches("./"));
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if EXCLUDED_FILENAMES.contains(&name) {
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
    fn discover_excludes_readme_and_contributing() {
        let tmp = tempdir().unwrap();
        let agents = tmp.path().join("agents");
        fs::create_dir_all(&agents).unwrap();
        fs::write(agents.join("README.md"), "# README").unwrap();
        fs::write(agents.join("CONTRIBUTING.md"), "# Contrib").unwrap();
        fs::write(agents.join("CHANGELOG.md"), "# Changelog").unwrap();
        fs::write(agents.join("real.md"), "---\nname: r\n---\n").unwrap();

        let found = discover_agents_in_dirs(tmp.path(), &["./agents/".to_string()]);
        let names: Vec<_> = found
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        assert_eq!(names, vec!["real.md"]);
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

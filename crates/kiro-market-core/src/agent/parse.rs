//! Dialect detection and top-level parser dispatch.

use std::fs;
use std::path::Path;

use crate::error::AgentError;

use super::types::{AgentDefinition, AgentDialect, ParseFailure};
use super::{parse_claude_agent, parse_copilot_agent};

/// Detect the source dialect from a filename.
///
/// Filenames ending in `.agent.md` are treated as Copilot; everything else
/// as Claude. The `.agent.md` double-extension is the Copilot community
/// convention (see `awesome-copilot/agents/`).
#[must_use]
pub fn detect_dialect(path: &Path) -> AgentDialect {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    if name.ends_with(".agent.md") {
        AgentDialect::Copilot
    } else {
        AgentDialect::Claude
    }
}

/// Read and parse an agent file, dispatching to the correct dialect parser.
///
/// All failures — I/O, YAML, missing frontmatter, missing name — are
/// returned as [`AgentError::ParseFailed`] carrying a structured
/// [`ParseFailure`] so callers can switch on the variant (e.g. to demote
/// `MissingFrontmatter` to a debug log) without string matching.
///
/// # Errors
///
/// Always [`AgentError::ParseFailed`]; inspect its `failure` field for the
/// specific failure mode.
pub fn parse_agent_file(path: &Path) -> Result<AgentDefinition, AgentError> {
    let content = fs::read_to_string(path).map_err(|e| AgentError::ParseFailed {
        path: path.to_path_buf(),
        failure: ParseFailure::IoError(e.to_string()),
    })?;
    let dialect = detect_dialect(path);
    let result = match dialect {
        AgentDialect::Claude => parse_claude_agent(&content),
        AgentDialect::Copilot => parse_copilot_agent(&content),
    };
    result.map_err(|failure| AgentError::ParseFailed {
        path: path.to_path_buf(),
        failure,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn detects_copilot_by_agent_md_suffix() {
        assert_eq!(
            detect_dialect(Path::new("foo.agent.md")),
            AgentDialect::Copilot
        );
        assert_eq!(
            detect_dialect(Path::new("/a/b/c.agent.md")),
            AgentDialect::Copilot
        );
    }

    #[test]
    fn detects_claude_for_plain_md() {
        assert_eq!(
            detect_dialect(Path::new("reviewer.md")),
            AgentDialect::Claude
        );
    }

    #[test]
    fn parse_agent_file_dispatches_by_dialect() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("sample.agent.md");
        std::fs::write(&path, "---\nname: sample\n---\nbody\n").unwrap();
        let def = parse_agent_file(&path).expect("parse");
        assert_eq!(def.dialect, AgentDialect::Copilot);
        assert_eq!(def.name, "sample");
    }

    #[test]
    fn parse_agent_file_missing_name_is_typed() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("no-name.md");
        std::fs::write(&path, "---\ndescription: x\n---\nbody\n").unwrap();
        let err = parse_agent_file(&path).unwrap_err();
        assert!(matches!(
            err,
            AgentError::ParseFailed {
                failure: ParseFailure::MissingName,
                ..
            }
        ));
    }

    #[test]
    fn parse_agent_file_missing_frontmatter_is_typed() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("readme.md");
        std::fs::write(&path, "# just a readme\n").unwrap();
        let err = parse_agent_file(&path).unwrap_err();
        assert!(matches!(
            err,
            AgentError::ParseFailed {
                failure: ParseFailure::MissingFrontmatter,
                ..
            }
        ));
    }

    #[test]
    fn parse_agent_file_invalid_yaml_is_typed() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("broken.md");
        std::fs::write(&path, "---\nname: [unclosed\n---\nbody\n").unwrap();
        let err = parse_agent_file(&path).unwrap_err();
        assert!(matches!(
            err,
            AgentError::ParseFailed {
                failure: ParseFailure::InvalidYaml(_),
                ..
            }
        ));
    }

    #[test]
    fn parse_agent_file_unreadable_is_io_error() {
        let path = Path::new("/nonexistent/agent.md");
        let err = parse_agent_file(path).unwrap_err();
        assert!(matches!(
            err,
            AgentError::ParseFailed {
                failure: ParseFailure::IoError(_),
                ..
            }
        ));
    }
}

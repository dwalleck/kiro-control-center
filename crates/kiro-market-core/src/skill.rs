//! Parsing of `SKILL.md` files.
//!
//! A skill file uses YAML frontmatter delimited by `---` fences, followed by
//! free-form Markdown content. This module extracts and validates the
//! frontmatter and records where the body begins.

use serde::Deserialize;
use thiserror::Error;

/// Errors that can occur while parsing `SKILL.md` frontmatter.
#[derive(Debug, Error)]
pub enum ParseError {
    /// The file does not start with a `---` frontmatter fence.
    #[error("missing opening `---` frontmatter fence")]
    MissingFrontmatter,

    /// The opening `---` fence was found but no closing fence follows.
    #[error("unclosed frontmatter: missing closing `---` fence")]
    UnclosedFrontmatter,

    /// The YAML between the fences could not be parsed.
    #[error("invalid YAML in frontmatter: {0}")]
    InvalidYaml(String),
}

/// Parsed YAML frontmatter from a `SKILL.md` file.
#[derive(Debug, Clone, Deserialize)]
pub struct SkillFrontmatter {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub invocable: bool,
}

/// Parse YAML frontmatter from a `SKILL.md` file.
///
/// Returns the parsed [`SkillFrontmatter`] together with the **byte offset**
/// where the body (content after the closing `---`) begins. The caller can use
/// `&content[offset..]` to obtain the body.
///
/// # Errors
///
/// Returns a [`ParseError`] if the frontmatter is missing, unclosed, or
/// contains invalid YAML.
pub fn parse_frontmatter(content: &str) -> Result<(SkillFrontmatter, usize), ParseError> {
    let trimmed = content.trim_start();
    let leading_whitespace = content.len() - trimmed.len();

    if !trimmed.starts_with("---") {
        return Err(ParseError::MissingFrontmatter);
    }

    // Skip past the opening fence and its newline.
    let after_open = &trimmed[3..];
    let after_open = after_open
        .strip_prefix('\n')
        .unwrap_or(after_open.strip_prefix("\r\n").unwrap_or(after_open));

    let Some(close_pos) = after_open.find("\n---") else {
        return Err(ParseError::UnclosedFrontmatter);
    };

    let yaml_block = &after_open[..close_pos];

    let frontmatter: SkillFrontmatter =
        serde_yaml::from_str(yaml_block).map_err(|e| ParseError::InvalidYaml(e.to_string()))?;

    // Calculate the byte offset where the body starts (after closing `---` and its newline).
    let close_fence_start =
        leading_whitespace + 3 + (trimmed.len() - 3 - after_open.len()) + close_pos;
    // Skip "\n---"
    let after_close_fence = close_fence_start + 4;
    // Skip optional newline after the closing fence.
    let body_offset = if content.as_bytes().get(after_close_fence) == Some(&b'\r') {
        if content.as_bytes().get(after_close_fence + 1) == Some(&b'\n') {
            after_close_fence + 2
        } else {
            after_close_fence + 1
        }
    } else if content.as_bytes().get(after_close_fence) == Some(&b'\n') {
        after_close_fence + 1
    } else {
        after_close_fence
    };

    Ok((frontmatter, body_offset))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid_frontmatter() {
        let content = "---\nname: my-skill\ndescription: A useful skill\n---\nBody text here.\n";

        let (fm, offset) = parse_frontmatter(content).expect("should parse");
        assert_eq!(fm.name, "my-skill");
        assert_eq!(fm.description, "A useful skill");
        assert!(!fm.invocable);
        assert_eq!(&content[offset..], "Body text here.\n");
    }

    #[test]
    fn parse_with_invocable_field() {
        let content =
            "---\nname: cmd-skill\ndescription: Runs commands\ninvocable: true\n---\n# Usage\n";

        let (fm, offset) = parse_frontmatter(content).expect("should parse");
        assert_eq!(fm.name, "cmd-skill");
        assert!(fm.invocable);
        assert_eq!(&content[offset..], "# Usage\n");
    }

    #[test]
    fn reject_missing_opening_fence() {
        let content = "name: no-fence\ndescription: Bad\n---\n";

        let err = parse_frontmatter(content).expect_err("should fail");
        assert!(
            matches!(err, ParseError::MissingFrontmatter),
            "expected MissingFrontmatter, got {err:?}"
        );
    }

    #[test]
    fn reject_unclosed_fence() {
        let content = "---\nname: orphan\ndescription: No closing fence\n";

        let err = parse_frontmatter(content).expect_err("should fail");
        assert!(
            matches!(err, ParseError::UnclosedFrontmatter),
            "expected UnclosedFrontmatter, got {err:?}"
        );
    }

    #[test]
    fn reject_invalid_yaml() {
        let content = "---\n: [invalid yaml\n---\n";

        let err = parse_frontmatter(content).expect_err("should fail");
        assert!(
            matches!(err, ParseError::InvalidYaml(_)),
            "expected InvalidYaml, got {err:?}"
        );
    }

    #[test]
    fn parse_crlf_frontmatter_body_offset() {
        let content = "---\r\nname: s\r\ndescription: d\r\n---\r\nBody\r\n";

        let (fm, offset) = parse_frontmatter(content).expect("should parse");
        assert_eq!(fm.name, "s");
        assert!(
            content[offset..].starts_with("Body"),
            "body should start with `Body`, got {:?}",
            &content[offset..]
        );
    }

    #[test]
    fn parse_no_trailing_newline_after_close_fence() {
        let content = "---\nname: s\ndescription: d\n---";

        let (_fm, offset) = parse_frontmatter(content).expect("should parse");
        assert_eq!(
            &content[offset..],
            "",
            "body offset should point to empty string"
        );
    }

    #[test]
    fn invocable_non_bool_yields_invalid_yaml() {
        let content = "---\nname: s\ndescription: d\ninvocable: maybe\n---\n";

        let err = parse_frontmatter(content).expect_err("should fail");
        assert!(
            matches!(err, ParseError::InvalidYaml(_)),
            "expected InvalidYaml, got {err:?}"
        );
    }
}

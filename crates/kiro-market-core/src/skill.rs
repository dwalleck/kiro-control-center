//! Parsing and merging of `SKILL.md` files.
//!
//! A skill file uses YAML frontmatter delimited by `---` fences, followed by
//! free-form Markdown content. This module extracts and validates the
//! frontmatter, records where the body begins, and supports merging companion
//! `.md` files into a single output.

use std::path::Path;

use pulldown_cmark::{Event, Parser, Tag};
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

/// A fully parsed skill with both metadata and content.
#[derive(Debug, Clone)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub content: String,
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

/// Extract relative `.md` link destinations from Markdown content.
///
/// Only links whose `dest_url`:
/// - ends with `.md`
/// - does **not** start with `http://`, `https://`, or `/`
///
/// are returned. This identifies companion files that live alongside a
/// `SKILL.md` and can be merged into it.
#[must_use]
pub fn extract_relative_md_links(markdown: &str) -> Vec<String> {
    let parser = Parser::new(markdown);
    let mut links = Vec::new();

    for event in parser {
        if let Event::Start(Tag::Link { dest_url, .. }) = event {
            let url: &str = &dest_url;
            let has_md_ext = Path::new(url)
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("md"));

            if has_md_ext
                && !url.starts_with("http://")
                && !url.starts_with("https://")
                && !url.starts_with('/')
            {
                links.push(url.to_owned());
            }
        }
    }

    links
}

/// Merge companion files into a `SKILL.md` document.
///
/// For each relative `.md` link found in `skill_content` that has a matching
/// entry in `companions` (keyed by relative path), the companion file content
/// is appended after a separator comment.
///
/// If `companions` is empty the original content is returned unchanged.
///
/// # Errors
///
/// Returns [`ParseError`] if the frontmatter in `skill_content` is invalid.
pub fn merge_skill(skill_content: &str, companions: &[(&str, &str)]) -> Result<String, ParseError> {
    if companions.is_empty() {
        return Ok(skill_content.to_owned());
    }

    // Validate frontmatter so we fail fast on malformed input.
    let (_fm, body_offset) = parse_frontmatter(skill_content)?;
    let body = &skill_content[body_offset..];

    let referenced_links = extract_relative_md_links(body);

    let mut merged = skill_content.to_owned();

    for link in &referenced_links {
        if let Some(&(_path, content)) = companions.iter().find(|(p, _)| *p == link.as_str()) {
            merged.push_str("\n\n---\n<!-- Merged from ");
            merged.push_str(link);
            merged.push_str(" -->\n");
            merged.push_str(content);
        }
    }

    Ok(merged)
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

    // -----------------------------------------------------------------------
    // extract_relative_md_links
    // -----------------------------------------------------------------------

    #[test]
    fn extract_relative_md_links_from_mixed_content() {
        let markdown = r"
Check [external](https://example.com/docs.md) and [image](logo.png).
Also see [type mapping](references/type-mapping.md) and
[error guide](references/error-guide.md).
And a [root link](/absolute/path.md).
";
        let links = extract_relative_md_links(markdown);
        assert_eq!(
            links,
            vec!["references/type-mapping.md", "references/error-guide.md",]
        );
    }

    #[test]
    fn extract_relative_md_links_returns_empty_for_plain_text() {
        let markdown = "No links here, just plain text with a .md mention.";
        let links = extract_relative_md_links(markdown);
        assert!(links.is_empty());
    }

    // -----------------------------------------------------------------------
    // merge_skill
    // -----------------------------------------------------------------------

    #[test]
    fn merge_skill_with_one_companion() {
        let skill = "\
---
name: test-skill
description: A test
---
See [ref](companion.md) for details.
";
        let companion_content = "# Companion\nExtra details here.";
        let companions = [("companion.md", companion_content)];

        let merged = merge_skill(skill, &companions).expect("should merge");

        assert!(merged.starts_with(skill));
        assert!(merged.contains("<!-- Merged from companion.md -->"));
        assert!(merged.contains(companion_content));
    }

    #[test]
    fn merge_skill_with_no_companions_returns_original() {
        let skill = "\
---
name: standalone
description: No companions
---
Just the body.
";
        let merged = merge_skill(skill, &[]).expect("should succeed");
        assert_eq!(merged, skill);
    }

    #[test]
    fn merge_skill_with_multiple_companions() {
        let skill = "\
---
name: multi
description: Multiple refs
---
Read [types](references/types.md) and [errors](references/errors.md).
";
        let companions = [
            ("references/types.md", "# Types\nType info."),
            ("references/errors.md", "# Errors\nError info."),
        ];

        let merged = merge_skill(skill, &companions).expect("should merge");

        assert!(merged.contains("<!-- Merged from references/types.md -->"));
        assert!(merged.contains("# Types\nType info."));
        assert!(merged.contains("<!-- Merged from references/errors.md -->"));
        assert!(merged.contains("# Errors\nError info."));

        let types_pos = merged
            .find("<!-- Merged from references/types.md -->")
            .expect("types marker");
        let errors_pos = merged
            .find("<!-- Merged from references/errors.md -->")
            .expect("errors marker");
        assert!(
            types_pos < errors_pos,
            "types should appear before errors (document order)"
        );
    }
}

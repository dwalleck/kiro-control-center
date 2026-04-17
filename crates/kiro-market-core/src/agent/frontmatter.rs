//! Dialect-agnostic YAML frontmatter splitter. Shared by `parse_claude`
//! and `parse_copilot`.

use super::types::ParseFailure;

/// Split `---`-fenced YAML frontmatter from the body. Returns `(yaml, body)`.
///
/// # Errors
///
/// Returns [`ParseFailure::MissingFrontmatter`] when the opening fence is
/// absent, or [`ParseFailure::UnclosedFrontmatter`] when only the opening
/// fence is present.
pub(super) fn split_frontmatter(content: &str) -> Result<(&str, &str), ParseFailure> {
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        return Err(ParseFailure::MissingFrontmatter);
    }
    let after_open = trimmed[3..]
        .strip_prefix('\n')
        .or_else(|| trimmed[3..].strip_prefix("\r\n"))
        .unwrap_or(&trimmed[3..]);
    let Some(close_pos) = after_open.find("\n---") else {
        return Err(ParseFailure::UnclosedFrontmatter);
    };
    let yaml = &after_open[..close_pos];
    // After `\n---`, strip any run of `\r` / `\n`. The GitHub frontmatter
    // convention puts a blank line between the closing fence and the body,
    // and some editors add a trailing `\r` before the newline.
    let body = after_open[close_pos + 4..].trim_start_matches(['\r', '\n']);
    Ok((yaml, body))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_simple_frontmatter() {
        let (yaml, body) = split_frontmatter("---\nname: a\n---\nbody\n").unwrap();
        assert_eq!(yaml, "name: a");
        assert_eq!(body, "body\n");
    }

    #[test]
    fn missing_open_fence_returns_missing_frontmatter() {
        let err = split_frontmatter("body\n").unwrap_err();
        assert_eq!(err, ParseFailure::MissingFrontmatter);
    }

    #[test]
    fn missing_close_fence_returns_unclosed_frontmatter() {
        let err = split_frontmatter("---\nname: a\nbody\n").unwrap_err();
        assert_eq!(err, ParseFailure::UnclosedFrontmatter);
    }

    #[test]
    fn handles_crlf_line_endings() {
        let (yaml, body) = split_frontmatter("---\r\nname: a\r\n---\r\nbody\r\n").unwrap();
        assert!(yaml.contains("name: a"));
        assert!(body.starts_with("body"));
    }

    #[test]
    fn consumes_multiple_blank_lines_before_body() {
        let (_yaml, body) = split_frontmatter("---\nname: a\n---\n\n\nhello\n").unwrap();
        assert!(body.starts_with("hello"));
    }

    #[test]
    fn tolerates_leading_whitespace_before_fence() {
        let (yaml, _body) = split_frontmatter("\n\n---\nname: a\n---\nx\n").unwrap();
        assert_eq!(yaml, "name: a");
    }
}

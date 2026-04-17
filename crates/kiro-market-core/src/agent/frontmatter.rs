//! Dialect-agnostic YAML frontmatter splitter.
//!
//! Both Claude (`*.md`) and Copilot (`*.agent.md`) agents use the same
//! `---`-fenced frontmatter convention, so the fence-handling logic lives
//! in one place rather than being duplicated (or worse, reached into via
//! a `pub(super)` shim) between the two parsers.

/// Split `---`-fenced YAML frontmatter from the body. Returns `(yaml, body)`.
///
/// # Errors
///
/// Returns a human-readable error if the opening or closing fence is missing.
pub(super) fn split_frontmatter(content: &str) -> Result<(&str, &str), String> {
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        return Err("missing opening `---` frontmatter fence".into());
    }
    let after_open = trimmed[3..]
        .strip_prefix('\n')
        .or_else(|| trimmed[3..].strip_prefix("\r\n"))
        .unwrap_or(&trimmed[3..]);
    let Some(close_pos) = after_open.find("\n---") else {
        return Err("unclosed frontmatter: missing closing `---` fence".into());
    };
    let yaml = &after_open[..close_pos];
    // After `\n---`, consume any number of blank lines (CRLF or LF). Most
    // editors (and the GitHub frontmatter convention) put a blank line
    // between the closing fence and the body, so callers should not have to
    // strip it themselves.
    let mut body = &after_open[close_pos + 4..];
    loop {
        if let Some(rest) = body.strip_prefix("\r\n") {
            body = rest;
        } else if let Some(rest) = body.strip_prefix('\n') {
            body = rest;
        } else {
            break;
        }
    }
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
    fn missing_open_fence_errors() {
        let err = split_frontmatter("body\n").unwrap_err();
        assert!(err.contains("opening"));
    }

    #[test]
    fn missing_close_fence_errors() {
        let err = split_frontmatter("---\nname: a\nbody\n").unwrap_err();
        assert!(err.contains("closing") || err.contains("unclosed"));
    }

    #[test]
    fn handles_crlf_line_endings() {
        let (yaml, body) = split_frontmatter("---\r\nname: a\r\n---\r\nbody\r\n").unwrap();
        assert!(yaml.contains("name: a"));
        assert!(body.starts_with("body"));
    }

    #[test]
    fn tolerates_leading_whitespace_before_fence() {
        let (yaml, _body) = split_frontmatter("\n\n---\nname: a\n---\nx\n").unwrap();
        assert_eq!(yaml, "name: a");
    }
}

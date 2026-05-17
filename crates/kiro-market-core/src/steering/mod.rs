//! Steering import: discover steering markdown files in a plugin and
//! install them into `.kiro/steering/` with content-hash-aware tracking.
//!
//! Steering is a peer install target alongside skills and agents — see
//! `docs/plans/2026-04-23-kiro-cli-native-plugin-import-design.md` for
//! the full design rationale.

pub mod discover;
pub mod types;

pub use discover::discover_steering_files_in_dirs;
pub(crate) use types::tracking_malformed;
pub use types::{
    FailedSteeringFile, InstallSteeringResult, InstalledSteeringOutcome, SteeringError,
    SteeringInstallContext, SteeringWarning,
};

use tracing::warn;

/// Strip YAML frontmatter from markdown content if a well-formed `---`
/// fence pair is present at the start. Returns the input unchanged when
/// no frontmatter is detected.
///
/// A fence is `---` *alone on a line* — leading dashes that are part of
/// a longer token (`----` thematic break, `--- trailing text`) are not
/// treated as fences, so the body of a non-frontmatter file with `---`
/// horizontal rules is never accidentally truncated.
///
/// Steering files sourced from Copilot `instructions/` directories carry
/// frontmatter (`description`, `applyTo`) that Kiro doesn't interpret.
/// Stripping it avoids installing misleading metadata into
/// `.kiro/steering/`.
///
/// Non-UTF-8 input and openers without a matching closer are returned
/// unchanged but are logged at `tracing::warn!` so operators notice
/// likely-malformed authoring rather than seeing a silent no-op install.
#[must_use]
pub fn strip_yaml_frontmatter(content: &[u8]) -> &[u8] {
    let Ok(s) = std::str::from_utf8(content) else {
        warn!(
            len = content.len(),
            "steering source is not valid UTF-8; frontmatter stripping skipped"
        );
        return content;
    };
    // Strip only ASCII whitespace and an optional UTF-8 BOM — not arbitrary
    // Unicode whitespace (which would eat NBSP, ideographic space, etc.).
    let trimmed = s.trim_start_matches(['\u{FEFF}', ' ', '\t', '\r', '\n']);
    // The opener must be `---` followed by `\n` or `\r\n`. A lone `---`
    // at EOF (with no body) is an unclosed opener and falls through below.
    let Some(after_open) = trimmed
        .strip_prefix("---\n")
        .or_else(|| trimmed.strip_prefix("---\r\n"))
    else {
        return content;
    };
    // The closer must be a line whose content (after stripping trailing
    // CR/space/tab) is exactly `---`. Iterating line-by-line avoids the
    // false-positive class where `find("\n---")` matches `\n--- text`
    // or a `\n----` thematic break in the body.
    let mut byte_pos: usize = 0;
    for line in after_open.split_inclusive('\n') {
        let trimmed_end = line.trim_end_matches([' ', '\t', '\r', '\n']);
        if trimmed_end == "---" {
            let body_start = byte_pos + line.len();
            // Strip at most one blank line (single \n or \r\n
            // separator) between the closing fence and body content.
            // Preserves intentional leading blank lines in the body
            // that would be eaten by an unbounded trim.
            let body = &after_open[body_start..];
            let body = body
                .strip_prefix('\n')
                .or_else(|| body.strip_prefix("\r\n"))
                .unwrap_or(body);
            return body.as_bytes();
        }
        byte_pos += line.len();
    }
    warn!(
        len = content.len(),
        "steering source has an opening `---` fence with no matching closer; returning bytes unchanged"
    );
    content
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_frontmatter() {
        let input = b"---\ndescription: test\napplyTo: \"**/*.cs\"\n---\n\n# Body\n";
        assert_eq!(strip_yaml_frontmatter(input), b"# Body\n");
    }

    #[test]
    fn returns_unchanged_without_frontmatter() {
        let input = b"# Just a heading\nSome content\n";
        assert_eq!(strip_yaml_frontmatter(input), input);
    }

    #[test]
    fn returns_unchanged_with_unclosed_frontmatter() {
        let input = b"---\nname: broken\nno closing fence\n";
        assert_eq!(strip_yaml_frontmatter(input), input);
    }

    #[test]
    fn handles_crlf() {
        let input = b"---\r\nkey: val\r\n---\r\n\r\nBody\r\n";
        assert_eq!(strip_yaml_frontmatter(input), b"Body\r\n");
    }

    #[test]
    fn preserves_multiple_leading_newlines() {
        // Only the first blank line after the closing fence is a separator;
        // additional blank lines are body content and must be preserved.
        let input = b"---\nkey: v\n---\n\n\n\n# Body\n";
        assert_eq!(strip_yaml_frontmatter(input), b"\n\n# Body\n");
    }

    #[test]
    fn preserves_multiple_leading_crlf_blank_lines() {
        // Same as above but with CRLF line endings. Only the first CRLF
        // after the closer is stripped.
        let input = b"---\r\nkey: v\r\n---\r\n\r\n\r\n\r\n# Body\r\n";
        assert_eq!(strip_yaml_frontmatter(input), b"\r\n\r\n# Body\r\n");
    }

    #[test]
    fn body_immediately_after_closer_with_no_blank_line() {
        // When body content starts on the line immediately after `---`,
        // nothing should be stripped.
        let input = b"---\nkey: v\n---\n# Title\n\nContent\n";
        assert_eq!(strip_yaml_frontmatter(input), b"# Title\n\nContent\n");
    }

    #[test]
    fn does_not_strip_four_dash_thematic_break() {
        // A line of `----` is a markdown thematic break, not a YAML opener.
        // The whole document must be returned unchanged.
        let input = b"----\n# Title\n----\n\nBody\n";
        assert_eq!(strip_yaml_frontmatter(input), input);
    }

    #[test]
    fn does_not_treat_dashed_text_line_as_closer() {
        // A line starting with `---` but carrying trailing text is not a
        // valid closer. The real closer further down must still be found.
        let input = b"---\nkey: v\n--- not a fence\n---\nBody\n";
        assert_eq!(strip_yaml_frontmatter(input), b"Body\n");
    }

    #[test]
    fn dashed_text_line_with_no_real_closer_returns_unchanged() {
        // If the only `\n---<text>` in the file isn't a real closer, the
        // file is treated as having no frontmatter and returned unchanged.
        let input = b"---\nkey: v\n--- not a fence\nBody\n";
        assert_eq!(strip_yaml_frontmatter(input), input);
    }

    #[test]
    fn handles_frontmatter_only_file_with_no_body() {
        let input = b"---\nkey: v\n---\n";
        assert_eq!(strip_yaml_frontmatter(input), b"");
    }

    #[test]
    fn handles_closer_without_trailing_newline() {
        // EOF immediately after the closer should still strip cleanly.
        let input = b"---\nkey: v\n---";
        assert_eq!(strip_yaml_frontmatter(input), b"");
    }

    #[test]
    fn handles_empty_input() {
        assert_eq!(strip_yaml_frontmatter(b""), b"");
    }

    #[test]
    fn handles_utf8_bom_before_frontmatter() {
        // UTF-8 BOM (EF BB BF) before the opener should be stripped during
        // detection so files saved by editors that add a BOM still strip.
        let input = b"\xef\xbb\xbf---\nkey: v\n---\n\nBody\n";
        assert_eq!(strip_yaml_frontmatter(input), b"Body\n");
    }

    #[test]
    fn does_not_strip_indented_closer() {
        // YAML's closing fence must be at column 0; an indented `---` is
        // body content, not a fence. The file is therefore unclosed and
        // returned unchanged.
        let input = b"---\nkey: v\n  ---\nBody\n";
        assert_eq!(strip_yaml_frontmatter(input), input);
    }

    #[test]
    fn non_utf8_input_returns_unchanged() {
        // Non-UTF-8 bytes are a likely-bug signal; the function returns
        // input unchanged and (separately) logs at warn level.
        let input: &[u8] = b"\xff\xfe---\nkey\n---\nBody\n";
        assert_eq!(strip_yaml_frontmatter(input), input);
    }
}

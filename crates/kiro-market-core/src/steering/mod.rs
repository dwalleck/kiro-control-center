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

use std::path::Path;

use tracing::warn;

/// Discriminated outcome of [`strip_yaml_frontmatter`]. Echo variants
/// carry the original input slice so [`Self::bytes_to_install`] returns
/// the canonical bytes by construction — callers cannot pass the wrong
/// slice back.
#[derive(Debug)]
#[must_use = "dropping a StripOutcome silently discards any anomaly warning; call \
              .anomaly_warning() and route the result into InstallSteeringResult.warnings"]
#[non_exhaustive]
pub(crate) enum StripOutcome<'a> {
    /// Well-formed frontmatter was removed. `body` is a strict suffix
    /// of the input, borrowed without copying.
    Stripped {
        body: &'a [u8],
    },
    NoFrontmatter {
        input: &'a [u8],
    },
    NonUtf8 {
        input: &'a [u8],
    },
    UnclosedFence {
        input: &'a [u8],
    },
}

impl<'a> StripOutcome<'a> {
    pub(crate) fn bytes_to_install(&self) -> &'a [u8] {
        match self {
            Self::Stripped { body, .. } => body,
            Self::NoFrontmatter { input }
            | Self::NonUtf8 { input }
            | Self::UnclosedFence { input } => input,
        }
    }

    pub(crate) fn anomaly_warning(&self, source_path: &Path) -> Option<SteeringWarning> {
        match self {
            Self::NonUtf8 { .. } => Some(SteeringWarning::SourceNotUtf8 {
                path: source_path.to_path_buf(),
            }),
            Self::UnclosedFence { .. } => Some(SteeringWarning::UnclosedFrontmatter {
                path: source_path.to_path_buf(),
            }),
            Self::Stripped { .. } | Self::NoFrontmatter { .. } => None,
        }
    }
}

/// Strip YAML frontmatter when a well-formed `---` fence pair leads the input.
///
/// A fence is `---` *alone on a line* — leading dashes that are part of
/// a longer token (`----` thematic break, `--- trailing text`) are not
/// treated as fences, so a non-frontmatter file with `---` horizontal
/// rules is never accidentally truncated. Copilot `instructions/`
/// sources carry frontmatter Kiro doesn't interpret; stripping it
/// keeps misleading metadata out of `.kiro/steering/`.
///
/// Anomalous outcomes ([`StripOutcome::NonUtf8`], [`StripOutcome::UnclosedFence`])
/// also log at `tracing::warn!` with a `len` field for operators.
pub(crate) fn strip_yaml_frontmatter(content: &[u8]) -> StripOutcome<'_> {
    let Ok(s) = std::str::from_utf8(content) else {
        warn!(
            len = content.len(),
            "steering source is not valid UTF-8; frontmatter stripping skipped"
        );
        return StripOutcome::NonUtf8 { input: content };
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
        return StripOutcome::NoFrontmatter { input: content };
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
            return StripOutcome::Stripped {
                body: body.as_bytes(),
            };
        }
        byte_pos += line.len();
    }
    warn!(
        len = content.len(),
        "steering source has an opening `---` fence with no matching closer; returning bytes unchanged"
    );
    StripOutcome::UnclosedFence { input: content }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[track_caller]
    fn assert_stripped(input: &[u8], expected_body: &[u8]) {
        let outcome = strip_yaml_frontmatter(input);
        match &outcome {
            StripOutcome::Stripped { body, .. } => {
                assert_eq!(*body, expected_body, "stripped body bytes differ");
            }
            other => panic!("expected Stripped, got {other:?}"),
        }
        assert_eq!(
            outcome.bytes_to_install(),
            expected_body,
            "bytes_to_install must match the body on Stripped",
        );
    }

    #[derive(Debug, Clone, Copy, PartialEq)]
    enum EchoKind {
        NoFrontmatter,
        NonUtf8,
        UnclosedFence,
    }

    #[track_caller]
    fn assert_echoes(input: &[u8], expected: EchoKind) {
        let outcome = strip_yaml_frontmatter(input);
        let got = match outcome {
            StripOutcome::NoFrontmatter { .. } => Some(EchoKind::NoFrontmatter),
            StripOutcome::NonUtf8 { .. } => Some(EchoKind::NonUtf8),
            StripOutcome::UnclosedFence { .. } => Some(EchoKind::UnclosedFence),
            StripOutcome::Stripped { .. } => None,
        };
        assert_eq!(
            got,
            Some(expected),
            "expected echo {expected:?}, got {outcome:?}"
        );
        assert_eq!(
            outcome.bytes_to_install(),
            input,
            "bytes_to_install must echo the original on non-Stripped variants",
        );
    }

    #[test]
    fn strips_frontmatter() {
        let input = b"---\ndescription: test\napplyTo: \"**/*.cs\"\n---\n\n# Body\n";
        assert_stripped(input, b"# Body\n");
    }

    #[test]
    fn returns_unchanged_without_frontmatter() {
        let input = b"# Just a heading\nSome content\n";
        assert_echoes(input, EchoKind::NoFrontmatter);
    }

    #[tracing_test::traced_test]
    #[test]
    fn returns_unchanged_with_unclosed_frontmatter() {
        let input = b"---\nname: broken\nno closing fence\n";
        assert_echoes(input, EchoKind::UnclosedFence);
        // CLAUDE.md Rule 35: the warn arm is contract, not decoration.
        // A regression that removed the `warn!` call would otherwise
        // pass — the function returns the input bytes either way.
        assert!(logs_contain("no matching closer"));
    }

    #[test]
    fn handles_crlf() {
        let input = b"---\r\nkey: val\r\n---\r\n\r\nBody\r\n";
        assert_stripped(input, b"Body\r\n");
    }

    #[test]
    fn preserves_multiple_leading_newlines() {
        // Only the first blank line after the closing fence is a separator;
        // additional blank lines are body content and must be preserved.
        let input = b"---\nkey: v\n---\n\n\n\n# Body\n";
        assert_stripped(input, b"\n\n# Body\n");
    }

    #[test]
    fn preserves_multiple_leading_crlf_blank_lines() {
        // Same as above but with CRLF line endings. Only the first CRLF
        // after the closer is stripped.
        let input = b"---\r\nkey: v\r\n---\r\n\r\n\r\n\r\n# Body\r\n";
        assert_stripped(input, b"\r\n\r\n# Body\r\n");
    }

    #[test]
    fn body_immediately_after_closer_with_no_blank_line() {
        // When body content starts on the line immediately after `---`,
        // nothing should be stripped.
        let input = b"---\nkey: v\n---\n# Title\n\nContent\n";
        assert_stripped(input, b"# Title\n\nContent\n");
    }

    #[test]
    fn does_not_strip_four_dash_thematic_break() {
        // A line of `----` is a markdown thematic break, not a YAML opener.
        // The whole document must be returned unchanged. Locks the
        // refusal class: if closer-matching downgrades from exact
        // `trim_end_matches(...) == "---"` to a `starts_with("---")`
        // shape (the pre-rewrite shape), this test fails.
        let input = b"----\n# Title\n----\n\nBody\n";
        assert_echoes(input, EchoKind::NoFrontmatter);
    }

    #[test]
    fn does_not_treat_dashed_text_line_as_closer() {
        // A line starting with `---` but carrying trailing text is not a
        // valid closer. The real closer further down must still be found.
        let input = b"---\nkey: v\n--- not a fence\n---\nBody\n";
        assert_stripped(input, b"Body\n");
    }

    #[test]
    fn dashed_text_line_with_no_real_closer_returns_unchanged() {
        // If the only `\n---<text>` in the file isn't a real closer, the
        // file is treated as having no frontmatter and returned unchanged
        // — falls through to `UnclosedFence` because the opener was
        // valid but no closer line was found.
        let input = b"---\nkey: v\n--- not a fence\nBody\n";
        assert_echoes(input, EchoKind::UnclosedFence);
    }

    #[test]
    fn handles_frontmatter_only_file_with_no_body() {
        assert_stripped(b"---\nkey: v\n---\n", b"");
    }

    #[test]
    fn handles_adjacent_open_and_close_fences_with_body() {
        // Empty frontmatter (opener and closer on adjacent lines).
        // The pre-rewrite bug class was `find("\n---")` requiring a
        // preceding newline, which missed this shape entirely.
        // Pinned so the line-by-line scan can't silently regress to
        // that pattern.
        assert_stripped(b"---\n---\nBody\n", b"Body\n");
    }

    #[test]
    fn handles_adjacent_open_and_close_fences_with_body_crlf() {
        assert_stripped(b"---\r\n---\r\nBody\r\n", b"Body\r\n");
    }

    #[test]
    fn handles_closer_without_trailing_newline() {
        // EOF immediately after the closer should still strip cleanly.
        assert_stripped(b"---\nkey: v\n---", b"");
    }

    #[test]
    fn handles_empty_input() {
        // Empty input has no opener — falls through to NoFrontmatter.
        assert_echoes(b"", EchoKind::NoFrontmatter);
    }

    #[test]
    fn handles_utf8_bom_before_frontmatter() {
        // UTF-8 BOM (EF BB BF) before the opener should be stripped during
        // detection so files saved by editors that add a BOM still strip.
        assert_stripped(b"\xef\xbb\xbf---\nkey: v\n---\n\nBody\n", b"Body\n");
    }

    #[test]
    fn does_not_strip_indented_closer() {
        // YAML's closing fence must be at column 0; an indented `---` is
        // body content, not a fence. The file is therefore unclosed.
        let input = b"---\nkey: v\n  ---\nBody\n";
        assert_echoes(input, EchoKind::UnclosedFence);
    }

    #[tracing_test::traced_test]
    #[test]
    fn non_utf8_input_returns_unchanged() {
        // Non-UTF-8 bytes are a likely-bug signal; the function returns
        // input unchanged and (separately) logs at warn level. The
        // captured-log assertion locks the warn contract — see the
        // companion test `returns_unchanged_with_unclosed_frontmatter`.
        let input: &[u8] = b"\xff\xfe---\nkey\n---\nBody\n";
        assert_echoes(input, EchoKind::NonUtf8);
        assert!(logs_contain("not valid UTF-8"));
    }

    #[test]
    fn anomaly_warning_routes_to_typed_steering_warning() {
        use std::path::PathBuf;

        let stripped = StripOutcome::Stripped { body: b"" };
        assert!(stripped.anomaly_warning(Path::new("x.md")).is_none());
        assert!(
            StripOutcome::NoFrontmatter { input: b"" }
                .anomaly_warning(Path::new("x.md"))
                .is_none()
        );

        let utf8 = StripOutcome::NonUtf8 { input: b"" }.anomaly_warning(Path::new("bin.md"));
        assert!(matches!(
            utf8,
            Some(SteeringWarning::SourceNotUtf8 { ref path }) if path == &PathBuf::from("bin.md")
        ));
        let unclosed =
            StripOutcome::UnclosedFence { input: b"" }.anomaly_warning(Path::new("oops.md"));
        assert!(matches!(
            unclosed,
            Some(SteeringWarning::UnclosedFrontmatter { ref path }) if path == &PathBuf::from("oops.md")
        ));
    }
}

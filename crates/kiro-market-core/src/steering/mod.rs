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

/// Outcome of [`strip_yaml_frontmatter`]. Discriminates between the
/// four cases the caller actually cares about:
///
/// - [`StripOutcome::Stripped`] — a well-formed frontmatter block was
///   removed; `body` is the post-frontmatter byte slice borrowed from
///   the input.
/// - [`StripOutcome::NoFrontmatter`] — input did not begin with a
///   `---` opener; install the bytes verbatim.
/// - [`StripOutcome::NonUtf8`] — input was not valid UTF-8; install
///   the bytes verbatim. Anomalous: the caller should surface a
///   [`SteeringWarning::SourceNotUtf8`] so the user notices their
///   steering file isn't actually text.
/// - [`StripOutcome::UnclosedFence`] — input had an opening `---`
///   fence with no matching closer; install the bytes verbatim.
///   Anomalous: the caller should surface a
///   [`SteeringWarning::UnclosedFrontmatter`] — almost certainly an
///   authoring slip.
///
/// The lifetime parameter binds [`StripOutcome::Stripped::body`] to
/// the input slice so the caller reads the result without copying.
#[derive(Debug)]
#[non_exhaustive]
pub enum StripOutcome<'a> {
    Stripped { body: &'a [u8] },
    NoFrontmatter,
    NonUtf8,
    UnclosedFence,
}

impl<'a> StripOutcome<'a> {
    /// Bytes the caller should install. For [`StripOutcome::Stripped`]
    /// the post-frontmatter body; for every other variant the original
    /// input slice. Encodes the lenient install policy: even malformed
    /// sources land on disk, paired with a typed warning to surface
    /// the issue.
    #[must_use]
    pub fn bytes_to_install(&self, original: &'a [u8]) -> &'a [u8] {
        match self {
            Self::Stripped { body } => body,
            Self::NoFrontmatter | Self::NonUtf8 | Self::UnclosedFence => original,
        }
    }

    /// Map an anomalous outcome to a typed [`SteeringWarning`].
    /// Returns `None` for the non-anomalous variants ([`Stripped`],
    /// [`NoFrontmatter`]). Exhaustive over all variants — adding a
    /// new [`StripOutcome`] arm forces a compile-time decision here.
    ///
    /// [`Stripped`]: StripOutcome::Stripped
    /// [`NoFrontmatter`]: StripOutcome::NoFrontmatter
    #[must_use]
    pub fn anomaly_warning(&self, source_path: &Path) -> Option<SteeringWarning> {
        match self {
            Self::NonUtf8 => Some(SteeringWarning::SourceNotUtf8 {
                path: source_path.to_path_buf(),
            }),
            Self::UnclosedFence => Some(SteeringWarning::UnclosedFrontmatter {
                path: source_path.to_path_buf(),
            }),
            Self::Stripped { .. } | Self::NoFrontmatter => None,
        }
    }
}

/// Strip YAML frontmatter from markdown content if a well-formed `---`
/// fence pair is present at the start.
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
/// The return value is a [`StripOutcome`] discriminating between
/// stripped, no-frontmatter, non-UTF-8, and unclosed-opener cases.
/// The anomalous variants ([`StripOutcome::NonUtf8`] and
/// [`StripOutcome::UnclosedFence`]) also log at `tracing::warn!` with
/// a `len` field for operators who watch the structured stream.
/// Callers route the anomalies into [`SteeringWarning`]s via
/// [`StripOutcome::anomaly_warning`] so they reach
/// `InstallSteeringResult.warnings` and surface in the UI/CLI.
#[must_use]
pub fn strip_yaml_frontmatter(content: &[u8]) -> StripOutcome<'_> {
    let Ok(s) = std::str::from_utf8(content) else {
        warn!(
            len = content.len(),
            "steering source is not valid UTF-8; frontmatter stripping skipped"
        );
        return StripOutcome::NonUtf8;
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
        return StripOutcome::NoFrontmatter;
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
    StripOutcome::UnclosedFence
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: assert the outcome is `Stripped` with the given body
    /// bytes. Routes through `bytes_to_install` so the lenient install
    /// invariant (`bytes_to_install` returns the body slice on
    /// `Stripped`) also gets pinned by every byte-content test.
    #[track_caller]
    fn assert_stripped(input: &[u8], expected_body: &[u8]) {
        let outcome = strip_yaml_frontmatter(input);
        match &outcome {
            StripOutcome::Stripped { body } => {
                assert_eq!(*body, expected_body, "stripped body bytes differ");
            }
            other => panic!("expected Stripped, got {other:?}"),
        }
        assert_eq!(
            outcome.bytes_to_install(input),
            expected_body,
            "bytes_to_install must match the body on Stripped",
        );
    }

    /// Helper: assert the outcome is one of the echo variants with the
    /// expected discriminant, and that `bytes_to_install` returns the
    /// original input slice (the lenient install policy).
    #[track_caller]
    fn assert_echoes(input: &[u8], expected: &StripOutcome<'_>) {
        let outcome = strip_yaml_frontmatter(input);
        // Match by discriminant — for echo variants there's no payload
        // to compare, but we want exact-variant equality.
        let got_kind = std::mem::discriminant(&outcome);
        let want_kind = std::mem::discriminant(expected);
        assert_eq!(
            got_kind, want_kind,
            "expected {expected:?}, got {outcome:?}",
        );
        assert_eq!(
            outcome.bytes_to_install(input),
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
        assert_echoes(input, &StripOutcome::NoFrontmatter);
    }

    #[tracing_test::traced_test]
    #[test]
    fn returns_unchanged_with_unclosed_frontmatter() {
        let input = b"---\nname: broken\nno closing fence\n";
        assert_echoes(input, &StripOutcome::UnclosedFence);
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
        assert_echoes(input, &StripOutcome::NoFrontmatter);
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
        assert_echoes(input, &StripOutcome::UnclosedFence);
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
        assert_echoes(b"", &StripOutcome::NoFrontmatter);
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
        assert_echoes(input, &StripOutcome::UnclosedFence);
    }

    #[tracing_test::traced_test]
    #[test]
    fn non_utf8_input_returns_unchanged() {
        // Non-UTF-8 bytes are a likely-bug signal; the function returns
        // input unchanged and (separately) logs at warn level. The
        // captured-log assertion locks the warn contract — see the
        // companion test `returns_unchanged_with_unclosed_frontmatter`.
        let input: &[u8] = b"\xff\xfe---\nkey\n---\nBody\n";
        assert_echoes(input, &StripOutcome::NonUtf8);
        assert!(logs_contain("not valid UTF-8"));
    }

    #[test]
    fn anomaly_warning_routes_to_typed_steering_warning() {
        use std::path::PathBuf;

        // The Stripped and NoFrontmatter cases produce no warning.
        let stripped = StripOutcome::Stripped { body: b"" };
        assert!(stripped.anomaly_warning(Path::new("x.md")).is_none());
        assert!(
            StripOutcome::NoFrontmatter
                .anomaly_warning(Path::new("x.md"))
                .is_none()
        );

        // The anomalous cases produce typed SteeringWarnings carrying
        // the source path. A regression that maps NonUtf8 to the
        // UnclosedFrontmatter variant (or vice versa) would flip the
        // user-facing message and this test catches it.
        let utf8 = StripOutcome::NonUtf8.anomaly_warning(Path::new("bin.md"));
        assert!(matches!(
            utf8,
            Some(SteeringWarning::SourceNotUtf8 { ref path }) if path == &PathBuf::from("bin.md")
        ));
        let unclosed = StripOutcome::UnclosedFence.anomaly_warning(Path::new("oops.md"));
        assert!(matches!(
            unclosed,
            Some(SteeringWarning::UnclosedFrontmatter { ref path }) if path == &PathBuf::from("oops.md")
        ));
    }
}

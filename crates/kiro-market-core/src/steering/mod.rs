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

/// Strip YAML frontmatter (`---` fenced block) from markdown content if
/// present. Returns the body unchanged when no frontmatter is detected.
///
/// Steering files sourced from Copilot `instructions/` directories carry
/// frontmatter (`description`, `applyTo`) that Kiro doesn't interpret.
/// Stripping it avoids installing misleading metadata into
/// `.kiro/steering/`.
#[must_use]
pub fn strip_yaml_frontmatter(content: &[u8]) -> &[u8] {
    let Ok(s) = std::str::from_utf8(content) else {
        return content;
    };
    let trimmed = s.trim_start();
    if !trimmed.starts_with("---") {
        return content;
    }
    let after_open = trimmed[3..]
        .strip_prefix('\n')
        .or_else(|| trimmed[3..].strip_prefix("\r\n"))
        .unwrap_or(&trimmed[3..]);
    let Some(close_pos) = after_open.find("\n---") else {
        return content;
    };
    // Skip past `\n---` and any trailing newlines after the closing fence.
    let body = after_open[close_pos + 4..].trim_start_matches(['\r', '\n']);
    // Return a subslice of the original content bytes.
    let offset = body.as_ptr() as usize - content.as_ptr() as usize;
    &content[offset..]
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
}

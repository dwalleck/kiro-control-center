//! Domain error types for kiro-market-core.
//!
//! Errors are organised into thematic groups ([`MarketplaceError`],
//! [`PluginError`], [`SkillError`], [`AgentError`], [`GitError`]) and a
//! top-level [`Error`] enum that unifies them via `From` conversions.

use std::path::PathBuf;

use thiserror::Error;

use crate::agent::ParseFailure;

// ---------------------------------------------------------------------------
// Marketplace errors
// ---------------------------------------------------------------------------

/// Errors related to marketplace operations.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum MarketplaceError {
    /// The requested marketplace was not found.
    #[error("marketplace not found: {name}")]
    NotFound { name: String },

    /// A marketplace with this name is already registered.
    #[error("marketplace already registered: {name}")]
    AlreadyRegistered { name: String },

    /// The marketplace manifest could not be parsed.
    #[error("invalid marketplace manifest: {reason}")]
    InvalidManifest { reason: String },

    /// No `marketplace.json` and no `plugin.json` files found via scan.
    #[error("no plugins found in {path}")]
    NoPluginsFound { path: PathBuf },

    /// The user supplied an `http://` URL but the
    /// [`crate::service::InsecureHttpPolicy`] is set to `Reject`
    /// (the default). http traffic is unauthenticated and trivially
    /// MITM'd; a network attacker who substitutes the marketplace
    /// contents gets arbitrary code execution via skills, agents, and
    /// MCP servers, and the cache persists so a one-time MITM is a
    /// long-term backdoor. The error names the caller-facing knob
    /// (the `--allow-insecure-http` CLI flag, mapping to the `Allow`
    /// variant) so the remediation is discoverable from the message.
    #[error(
        "refusing to add insecure http:// marketplace `{url}`; \
         use https:// (or pass --allow-insecure-http to opt in to MITM risk)"
    )]
    InsecureSource { url: String },
}

// ---------------------------------------------------------------------------
// Plugin errors
// ---------------------------------------------------------------------------

/// Errors related to plugin operations.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum PluginError {
    /// The requested plugin was not found inside its marketplace.
    #[error("plugin `{plugin}` not found in marketplace `{marketplace}`")]
    NotFound { plugin: String, marketplace: String },

    /// The plugin manifest could not be parsed.
    #[error("invalid plugin manifest: {reason}")]
    InvalidManifest { reason: String },

    /// No `plugin.json` exists at the expected location.
    #[error("plugin manifest not found at {path}")]
    ManifestNotFound { path: PathBuf },

    /// The plugin declares no skills.
    #[error("plugin `{name}` has no skills")]
    NoSkills { name: String },

    /// The plugin directory referenced by a `RelativePath` source does not
    /// exist on disk. Typically means the marketplace manifest points at a
    /// directory that was never committed or was deleted.
    #[error("plugin directory does not exist: {path}")]
    DirectoryMissing { path: PathBuf },
}

// ---------------------------------------------------------------------------
// Skill errors
// ---------------------------------------------------------------------------

/// Errors related to skill operations.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum SkillError {
    /// The skill is already installed in the target project.
    #[error("skill `{name}` is already installed")]
    AlreadyInstalled { name: String },

    /// The skill is not installed in the target project.
    #[error("skill `{name}` is not installed")]
    NotInstalled { name: String },

    /// No `SKILL.md` was found for the skill.
    #[error("SKILL.md not found at {path}")]
    SkillMdNotFound { path: PathBuf },
}

// ---------------------------------------------------------------------------
// Agent errors
// ---------------------------------------------------------------------------

/// Errors related to agent operations.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum AgentError {
    /// The agent is already installed in the target project.
    #[error("agent `{name}` is already installed")]
    AlreadyInstalled { name: String },

    /// The agent is not installed in the target project.
    #[error("agent `{name}` is not installed")]
    NotInstalled { name: String },

    /// The source file could not be parsed. Inspect `failure` for the
    /// specific stage (missing frontmatter, invalid YAML, missing name,
    /// I/O error) — callers switch on the variant rather than
    /// substring-matching this Display.
    #[error("failed to parse agent at {path}: {failure}")]
    ParseFailed {
        path: PathBuf,
        failure: ParseFailure,
    },
}

// ---------------------------------------------------------------------------
// Git errors
// ---------------------------------------------------------------------------

/// Errors related to Git operations.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum GitError {
    /// Cloning a remote repository failed.
    #[error("failed to clone {url}")]
    CloneFailed {
        url: String,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// Pulling updates into an existing clone failed.
    #[error("failed to pull in {path}")]
    PullFailed {
        path: PathBuf,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// Opening an existing repository failed.
    #[error("failed to open repository at {path}")]
    OpenFailed {
        path: PathBuf,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// The checked-out commit SHA does not match the expected pinned SHA.
    #[error("SHA mismatch: expected {expected}, got {actual}")]
    ShaMismatch { expected: String, actual: String },

    /// A pinned-SHA value supplied by the user (e.g. via the marketplace
    /// manifest's `sha` field) is structurally invalid. Caught at the
    /// boundary so a typo never reaches `git` only to silently match a
    /// short-prefix collision. The typed [`InvalidShaReason`] lets
    /// callers branch on cause (e.g. "too short → suggest 7+ chars" vs
    /// "non-hex → highlight the bad character") rather than parsing the
    /// rendered message.
    #[error("invalid SHA `{value}`: {reason}")]
    InvalidSha {
        value: String,
        reason: InvalidShaReason,
    },

    /// The `git` command-line tool was not found in `$PATH`.
    #[error("the 'git' command-line tool is required but was not found in PATH")]
    GitNotFound,

    /// A `git` subprocess failed to launch (not a missing binary).
    #[error("git command failed in {dir}")]
    GitCommandFailed {
        dir: PathBuf,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// A `git` subprocess could not authenticate to the remote. Almost
    /// always means an HTTPS clone needs credentials (a credential
    /// helper, a personal access token, or an SSH switch). Translated
    /// from the `fatal: could not read Username/Password ...` family of
    /// libcurl/git errors so users see "the repo needs auth" instead
    /// of "no such device or address" — those raw errors are the result
    /// of the deliberate `Stdio::null()` we set on the child to prevent
    /// credential prompts from stalling CI.
    #[error(
        "authentication required for `{url}` — configure a credential \
         helper (`git config --global credential.helper ...`), provide a \
         personal access token in the URL, or switch to SSH"
    )]
    AuthenticationRequired { url: String },
}

/// Why a user-supplied SHA prefix failed structural validation. Carried
/// by [`GitError::InvalidSha`]; consumers can `match` on the cause
/// (rather than substring-match the rendered message) to surface
/// targeted remediation in the UI.
///
/// `#[non_exhaustive]` so a future stricter rule (e.g. "looks like a
/// branch name") is additive.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum InvalidShaReason {
    /// Shorter than the minimum acceptable prefix length. Carries the
    /// observed length and the minimum so the UI can render
    /// "needs at least N hex chars".
    TooShort { actual: usize, min: usize },
    /// Longer than any known SHA hash output (40 chars for SHA-1,
    /// 64 chars for SHA-256). Probably a paste mistake, not a real SHA.
    TooLong { actual: usize, max: usize },
    /// Contains a character outside `[0-9a-fA-F]`. Carries the byte
    /// offset of the first offending character so the UI can underline
    /// the typo.
    NonHex { at: usize, byte: u8 },
}

impl std::fmt::Display for InvalidShaReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooShort { actual, min } => write!(
                f,
                "SHA prefix must be at least {min} hex characters (got {actual})"
            ),
            Self::TooLong { actual, max } => write!(
                f,
                "SHA prefix must be at most {max} hex characters (got {actual})"
            ),
            Self::NonHex { byte, .. } => {
                write!(f, "non-hex character `{}`", *byte as char)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Validation errors
// ---------------------------------------------------------------------------

/// Errors related to path / name validation.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ValidationError {
    /// A name used as a directory component contains unsafe characters.
    #[error("invalid name `{name}`: {reason}")]
    InvalidName { name: String, reason: String },

    /// A relative path contains components that would escape its root.
    #[error("invalid relative path `{path}`: {reason}")]
    InvalidRelativePath { path: String, reason: String },
}

// ---------------------------------------------------------------------------
// Top-level unified error
// ---------------------------------------------------------------------------

/// Unified error type for the kiro-market-core crate.
///
/// Provides `From` conversions for each domain error group as well as common
/// infrastructure errors (`io::Error`, `serde_json::Error`).
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum Error {
    #[error(transparent)]
    Marketplace(#[from] MarketplaceError),

    #[error(transparent)]
    Plugin(#[from] PluginError),

    #[error(transparent)]
    Skill(#[from] SkillError),

    #[error(transparent)]
    Agent(#[from] AgentError),

    #[error(transparent)]
    Git(#[from] GitError),

    #[error(transparent)]
    Validation(#[from] ValidationError),

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

/// Convenience alias for results using the crate-level [`Error`].
pub type Result<T> = std::result::Result<T, Error>;

// ---------------------------------------------------------------------------
// Source-chain helpers
// ---------------------------------------------------------------------------

/// Walk the `Error::source()` chain of `err` and return the joined messages,
/// **excluding** the top-level error's own Display.
///
/// Use this when constructing a *new* error that will wrap `err` and add its
/// own context — including the top-level Display would duplicate that
/// context. (E.g. a `CloneFailed { url, source: chain_of_inner_clone_failed }`
/// would otherwise emit "failed to clone X: failed to clone X: <root>".)
#[must_use]
pub fn error_source_chain(err: &(dyn std::error::Error + 'static)) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut source = err.source();
    while let Some(cause) = source {
        parts.push(cause.to_string());
        source = cause.source();
    }
    parts.join(": ")
}

/// Walk the full chain of `err` including its top-level Display. Use this
/// for *terminal* error reporting (logs, user-facing messages) where the
/// caller is not wrapping the error further.
#[must_use]
pub fn error_full_chain(err: &(dyn std::error::Error + 'static)) -> String {
    let mut detail = err.to_string();
    let chain = error_source_chain(err);
    if !chain.is_empty() {
        detail.push_str(": ");
        detail.push_str(&chain);
    }
    detail
}

#[cfg(test)]
mod tests {
    use std::io;

    use rstest::rstest;

    use super::*;

    // -----------------------------------------------------------------------
    // Display formatting
    // -----------------------------------------------------------------------

    #[rstest]
    #[case::marketplace_not_found(
        MarketplaceError::NotFound { name: "acme".into() },
        "marketplace not found: acme"
    )]
    #[case::marketplace_already_registered(
        MarketplaceError::AlreadyRegistered { name: "acme".into() },
        "marketplace already registered: acme"
    )]
    #[case::marketplace_invalid_manifest(
        MarketplaceError::InvalidManifest { reason: "bad json".into() },
        "invalid marketplace manifest: bad json"
    )]
    #[case::no_plugins_found(
        MarketplaceError::NoPluginsFound { path: PathBuf::from("/tmp/repo") },
        "no plugins found in /tmp/repo"
    )]
    fn marketplace_error_display(#[case] err: MarketplaceError, #[case] expected: &str) {
        assert_eq!(err.to_string(), expected);
    }

    #[rstest]
    #[case::plugin_not_found(
        PluginError::NotFound { plugin: "dotnet".into(), marketplace: "ms".into() },
        "plugin `dotnet` not found in marketplace `ms`"
    )]
    #[case::plugin_invalid_manifest(
        PluginError::InvalidManifest { reason: "missing name".into() },
        "invalid plugin manifest: missing name"
    )]
    #[case::plugin_manifest_not_found(
        PluginError::ManifestNotFound { path: PathBuf::from("/tmp/plugin.json") },
        "plugin manifest not found at /tmp/plugin.json"
    )]
    #[case::plugin_no_skills(
        PluginError::NoSkills { name: "empty".into() },
        "plugin `empty` has no skills"
    )]
    fn plugin_error_display(#[case] err: PluginError, #[case] expected: &str) {
        assert_eq!(err.to_string(), expected);
    }

    #[rstest]
    #[case::skill_already_installed(
        SkillError::AlreadyInstalled { name: "rust-check".into() },
        "skill `rust-check` is already installed"
    )]
    #[case::skill_not_installed(
        SkillError::NotInstalled { name: "missing-skill".into() },
        "skill `missing-skill` is not installed"
    )]
    #[case::skill_md_not_found(
        SkillError::SkillMdNotFound { path: PathBuf::from("skills/rust/SKILL.md") },
        "SKILL.md not found at skills/rust/SKILL.md"
    )]
    fn skill_error_display(#[case] err: SkillError, #[case] expected: &str) {
        assert_eq!(err.to_string(), expected);
    }

    #[rstest]
    #[case::invalid_name(
        ValidationError::InvalidName { name: "../escape".into(), reason: "contains `..`".into() },
        "invalid name `../escape`: contains `..`"
    )]
    #[case::invalid_relative_path(
        ValidationError::InvalidRelativePath { path: "../secret.md".into(), reason: "contains `..` component".into() },
        "invalid relative path `../secret.md`: contains `..` component"
    )]
    fn validation_error_display(#[case] err: ValidationError, #[case] expected: &str) {
        assert_eq!(err.to_string(), expected);
    }

    #[rstest]
    #[case::agent_already_installed(
        AgentError::AlreadyInstalled { name: "reviewer".into() },
        "agent `reviewer` is already installed"
    )]
    #[case::agent_not_installed(
        AgentError::NotInstalled { name: "missing".into() },
        "agent `missing` is not installed"
    )]
    #[case::agent_parse_invalid_yaml(
        AgentError::ParseFailed {
            path: PathBuf::from("a.md"),
            failure: ParseFailure::InvalidYaml("bad yaml".into()),
        },
        "failed to parse agent at a.md: invalid YAML: bad yaml"
    )]
    #[case::agent_parse_missing_name(
        AgentError::ParseFailed {
            path: PathBuf::from("a.md"),
            failure: ParseFailure::MissingName,
        },
        "failed to parse agent at a.md: missing required `name` field"
    )]
    #[case::agent_parse_missing_frontmatter(
        AgentError::ParseFailed {
            path: PathBuf::from("readme.md"),
            failure: ParseFailure::MissingFrontmatter,
        },
        "failed to parse agent at readme.md: missing opening `---` frontmatter fence"
    )]
    fn agent_error_display(#[case] err: AgentError, #[case] expected: &str) {
        assert_eq!(err.to_string(), expected);
    }

    #[test]
    fn git_clone_failed_display() {
        let err = GitError::CloneFailed {
            url: "https://github.com/x/y.git".into(),
            source: "network timeout".to_owned().into(),
        };
        assert_eq!(
            err.to_string(),
            "failed to clone https://github.com/x/y.git"
        );
    }

    #[test]
    fn git_pull_failed_display() {
        let err = GitError::PullFailed {
            path: PathBuf::from("/tmp/repo"),
            source: "merge conflict".to_owned().into(),
        };
        assert_eq!(err.to_string(), "failed to pull in /tmp/repo");
    }

    #[test]
    fn git_open_failed_display() {
        let err = GitError::OpenFailed {
            path: PathBuf::from("/tmp/nope"),
            source: "not a repository".to_owned().into(),
        };
        assert_eq!(err.to_string(), "failed to open repository at /tmp/nope");
    }

    #[test]
    fn git_sha_mismatch_display() {
        let err = GitError::ShaMismatch {
            expected: "abc1234".into(),
            actual: "def5678".into(),
        };
        assert_eq!(
            err.to_string(),
            "SHA mismatch: expected abc1234, got def5678"
        );
    }

    #[test]
    fn git_not_found_display() {
        let err = GitError::GitNotFound;
        assert_eq!(
            err.to_string(),
            "the 'git' command-line tool is required but was not found in PATH"
        );
    }

    #[test]
    fn git_command_failed_display() {
        let err = GitError::GitCommandFailed {
            dir: PathBuf::from("/tmp/repo"),
            source: "permission denied".to_owned().into(),
        };
        assert_eq!(err.to_string(), "git command failed in /tmp/repo");
    }

    #[test]
    fn git_command_failed_has_source() {
        use std::error::Error as _;
        let err = GitError::GitCommandFailed {
            dir: PathBuf::from("/tmp"),
            source: "permission denied".to_owned().into(),
        };
        let source = err.source().expect("should have a source");
        assert!(source.to_string().contains("permission denied"));
    }

    // -----------------------------------------------------------------------
    // From conversions
    // -----------------------------------------------------------------------

    #[test]
    fn from_marketplace_error() {
        let inner = MarketplaceError::NotFound {
            name: "test".into(),
        };
        let err: Error = inner.into();
        assert!(matches!(err, Error::Marketplace(_)));
    }

    #[test]
    fn from_plugin_error() {
        let inner = PluginError::NoSkills {
            name: "test".into(),
        };
        let err: Error = inner.into();
        assert!(matches!(err, Error::Plugin(_)));
    }

    #[test]
    fn from_skill_error() {
        let inner = SkillError::AlreadyInstalled {
            name: "test".into(),
        };
        let err: Error = inner.into();
        assert!(matches!(err, Error::Skill(_)));
    }

    #[test]
    fn from_not_installed_error() {
        let inner = SkillError::NotInstalled {
            name: "missing".into(),
        };
        let err: Error = inner.into();
        assert!(matches!(err, Error::Skill(SkillError::NotInstalled { .. })));
        assert!(
            err.to_string().contains("not installed"),
            "display should contain 'not installed', got: {err}"
        );
    }

    #[test]
    fn from_git_error() {
        let inner = GitError::CloneFailed {
            url: "https://example.com".into(),
            source: "fail".to_owned().into(),
        };
        let err: Error = inner.into();
        assert!(matches!(err, Error::Git(_)));
    }

    #[test]
    fn from_validation_error() {
        let inner = ValidationError::InvalidName {
            name: "../bad".into(),
            reason: "contains `..`".into(),
        };
        let err: Error = inner.into();
        assert!(matches!(err, Error::Validation(_)));
    }

    #[test]
    fn from_agent_error() {
        let inner = AgentError::NotInstalled { name: "x".into() };
        let err: Error = inner.into();
        assert!(matches!(err, Error::Agent(_)));
    }

    #[test]
    fn from_io_error() {
        let inner = io::Error::new(io::ErrorKind::NotFound, "gone");
        let err: Error = inner.into();
        assert!(matches!(err, Error::Io(_)));
    }

    #[test]
    fn from_serde_json_error() {
        let inner = serde_json::from_str::<String>("not json").unwrap_err();
        let err: Error = inner.into();
        assert!(matches!(err, Error::Json(_)));
    }

    // -----------------------------------------------------------------------
    // Source chain verification
    // -----------------------------------------------------------------------

    #[test]
    fn git_clone_failed_has_source() {
        use std::error::Error as _;
        let err = GitError::CloneFailed {
            url: "https://x.com/r.git".into(),
            source: "timeout".to_owned().into(),
        };
        let source = err.source().expect("should have a source");
        assert!(source.to_string().contains("timeout"));
    }

    #[test]
    fn git_pull_failed_has_source() {
        use std::error::Error as _;
        let err = GitError::PullFailed {
            path: PathBuf::from("/tmp"),
            source: "conflict".to_owned().into(),
        };
        let source = err.source().expect("should have a source");
        assert!(source.to_string().contains("conflict"));
    }

    #[test]
    fn git_open_failed_has_source() {
        use std::error::Error as _;
        let err = GitError::OpenFailed {
            path: PathBuf::from("/tmp"),
            source: "bad repo".to_owned().into(),
        };
        let source = err.source().expect("should have a source");
        assert!(source.to_string().contains("bad repo"));
    }

    // -----------------------------------------------------------------------
    // Source-chain helpers
    // -----------------------------------------------------------------------

    #[test]
    fn error_source_chain_skips_top_level_display() {
        // CloneFailed Display says "failed to clone X"; we want only the
        // root cause back, not the URL again.
        let inner: Box<dyn std::error::Error + Send + Sync> = "TLS handshake failed".into();
        let err = GitError::CloneFailed {
            url: "https://example.com/repo.git".into(),
            source: inner,
        };
        let chain = error_source_chain(&err);
        assert_eq!(chain, "TLS handshake failed");
        assert!(
            !chain.contains("https://example.com/repo.git"),
            "URL must not appear in source-only chain: {chain}"
        );
    }

    #[test]
    fn error_full_chain_includes_top_level_display() {
        let inner: Box<dyn std::error::Error + Send + Sync> = "stderr: bad".into();
        let err = GitError::CloneFailed {
            url: "https://example.com/repo.git".into(),
            source: inner,
        };
        let full = error_full_chain(&err);
        assert!(
            full.contains("https://example.com/repo.git"),
            "full chain must include top-level Display: {full}"
        );
        assert!(
            full.contains("stderr: bad"),
            "full chain must include source: {full}"
        );
    }

    #[test]
    fn nested_clone_failed_does_not_triplicate_url_when_source_chain_used() {
        // Simulates the dual-failure path: gix and CLI both fail, the outer
        // CloneFailed wraps a composed source containing both. When using
        // error_source_chain (NOT to_string), the URL must appear only in
        // the OUTER CloneFailed's Display — never inside the source.
        let url = "https://example.com/r.git";
        let gix_err = GitError::CloneFailed {
            url: url.into(),
            source: "gix root".to_owned().into(),
        };
        let cli_err = GitError::CloneFailed {
            url: url.into(),
            source: "cli root".to_owned().into(),
        };
        let combined = format!(
            "gix: {}; system git: {}",
            error_source_chain(&gix_err),
            error_source_chain(&cli_err)
        );
        let outer = GitError::CloneFailed {
            url: url.into(),
            source: combined.into(),
        };
        let full = error_full_chain(&outer);

        // URL appears exactly once (in the outer Display).
        let occurrences = full.matches(url).count();
        assert_eq!(
            occurrences, 1,
            "URL should appear exactly once in fully-rendered chain, got {occurrences} in: {full}"
        );
        // Both root causes must still be present.
        assert!(full.contains("gix root"), "missing gix root in: {full}");
        assert!(full.contains("cli root"), "missing cli root in: {full}");
    }

    #[test]
    fn error_source_chain_walks_multiple_levels() {
        let leaf = std::io::Error::other("permission denied");
        let middle = GitError::PullFailed {
            path: PathBuf::from("/tmp/repo"),
            source: Box::new(leaf),
        };
        let outer = GitError::OpenFailed {
            path: PathBuf::from("/tmp/repo"),
            source: Box::new(middle),
        };
        let chain = error_source_chain(&outer);
        assert!(chain.contains("failed to pull"), "chain: {chain}");
        assert!(chain.contains("permission denied"), "chain: {chain}");
    }
}

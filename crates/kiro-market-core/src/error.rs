//! Domain error types for kiro-market-core.
//!
//! Errors are organised into thematic groups ([`MarketplaceError`],
//! [`PluginError`], [`SkillError`], [`GitError`]) and a top-level [`Error`]
//! enum that unifies them via `From` conversions.

use std::path::PathBuf;

use thiserror::Error;

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

    /// No `marketplace.json` exists at the expected location.
    #[error("marketplace manifest not found at {path}")]
    ManifestNotFound { path: PathBuf },
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

    /// No `SKILL.md` was found for the skill.
    #[error("SKILL.md not found at {path}")]
    SkillMdNotFound { path: PathBuf },

    /// Merging the skill into the target project failed.
    #[error("failed to merge skill `{skill}` into {path}: {reason}")]
    MergeFailed {
        skill: String,
        path: PathBuf,
        reason: String,
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
        source: git2::Error,
    },

    /// Pulling updates into an existing clone failed.
    #[error("failed to pull in {path}")]
    PullFailed {
        path: PathBuf,
        #[source]
        source: git2::Error,
    },

    /// Opening an existing repository failed.
    #[error("failed to open repository at {path}")]
    OpenFailed {
        path: PathBuf,
        #[source]
        source: git2::Error,
    },
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
    #[case::marketplace_manifest_not_found(
        MarketplaceError::ManifestNotFound { path: PathBuf::from("/tmp/mp.json") },
        "marketplace manifest not found at /tmp/mp.json"
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
    #[case::skill_md_not_found(
        SkillError::SkillMdNotFound { path: PathBuf::from("skills/rust/SKILL.md") },
        "SKILL.md not found at skills/rust/SKILL.md"
    )]
    #[case::skill_merge_failed(
        SkillError::MergeFailed {
            skill: "go-lint".into(),
            path: PathBuf::from(".kiro/skills"),
            reason: "conflict".into(),
        },
        "failed to merge skill `go-lint` into .kiro/skills: conflict"
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

    #[test]
    fn git_clone_failed_display() {
        let git_err = git2::Error::from_str("network timeout");
        let err = GitError::CloneFailed {
            url: "https://github.com/x/y.git".into(),
            source: git_err,
        };
        assert_eq!(
            err.to_string(),
            "failed to clone https://github.com/x/y.git"
        );
    }

    #[test]
    fn git_pull_failed_display() {
        let git_err = git2::Error::from_str("merge conflict");
        let err = GitError::PullFailed {
            path: PathBuf::from("/tmp/repo"),
            source: git_err,
        };
        assert_eq!(err.to_string(), "failed to pull in /tmp/repo");
    }

    #[test]
    fn git_open_failed_display() {
        let git_err = git2::Error::from_str("not a repository");
        let err = GitError::OpenFailed {
            path: PathBuf::from("/tmp/nope"),
            source: git_err,
        };
        assert_eq!(err.to_string(), "failed to open repository at /tmp/nope");
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
    fn from_git_error() {
        let inner = GitError::CloneFailed {
            url: "https://example.com".into(),
            source: git2::Error::from_str("fail"),
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
        let git_err = git2::Error::from_str("timeout");
        let err = GitError::CloneFailed {
            url: "https://x.com/r.git".into(),
            source: git_err,
        };
        let source = err.source().expect("should have a source");
        assert!(source.downcast_ref::<git2::Error>().is_some());
    }

    #[test]
    fn git_pull_failed_has_source() {
        use std::error::Error as _;
        let git_err = git2::Error::from_str("conflict");
        let err = GitError::PullFailed {
            path: PathBuf::from("/tmp"),
            source: git_err,
        };
        let source = err.source().expect("should have a source");
        assert!(source.downcast_ref::<git2::Error>().is_some());
    }

    #[test]
    fn git_open_failed_has_source() {
        use std::error::Error as _;
        let git_err = git2::Error::from_str("bad repo");
        let err = GitError::OpenFailed {
            path: PathBuf::from("/tmp"),
            source: git_err,
        };
        let source = err.source().expect("should have a source");
        assert!(source.downcast_ref::<git2::Error>().is_some());
    }
}

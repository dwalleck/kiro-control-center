use kiro_market_core::error::{Error as CoreError, MarketplaceError, PluginError, SkillError};
use serde::Serialize;
use tracing::warn;

/// Machine-readable error classification for frontend conditional logic.
///
/// Serialized as snake_case strings and exported to TypeScript via specta.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum ErrorType {
    NotFound,
    AlreadyExists,
    Validation,
    GitError,
    IoError,
    ParseError,
    Unknown,
}

/// Structured error response for Tauri commands.
///
/// Provides a human-readable message alongside a machine-readable error type,
/// enabling the frontend to make type-safe decisions about error handling and
/// display appropriate messages without string parsing.
#[derive(Debug, Clone, Serialize, specta::Type)]
pub struct CommandError {
    pub message: String,
    pub error_type: ErrorType,
}

impl CommandError {
    pub fn new(message: impl Into<String>, error_type: ErrorType) -> Self {
        Self {
            message: message.into(),
            error_type,
        }
    }
}

impl From<CoreError> for CommandError {
    #[allow(clippy::match_same_arms)]
    fn from(err: CoreError) -> Self {
        let error_type = match &err {
            CoreError::Marketplace(MarketplaceError::NotFound { .. }) => ErrorType::NotFound,
            CoreError::Marketplace(MarketplaceError::AlreadyRegistered { .. }) => {
                ErrorType::AlreadyExists
            }
            CoreError::Marketplace(MarketplaceError::NoPluginsFound { .. }) => ErrorType::NotFound,
            CoreError::Marketplace(_) => ErrorType::ParseError,
            CoreError::Skill(SkillError::AlreadyInstalled { .. }) => ErrorType::AlreadyExists,
            CoreError::Skill(SkillError::NotInstalled { .. }) => ErrorType::NotFound,
            CoreError::Skill(SkillError::SkillMdNotFound { .. }) => ErrorType::NotFound,
            CoreError::Skill(_) => ErrorType::Unknown,
            CoreError::Validation(_) => ErrorType::Validation,
            CoreError::Git(_) => ErrorType::GitError,
            CoreError::Io(_) => ErrorType::IoError,
            CoreError::Json(_) => ErrorType::ParseError,
            CoreError::Plugin(PluginError::NotFound { .. }) => ErrorType::NotFound,
            CoreError::Plugin(PluginError::ManifestNotFound { .. }) => ErrorType::NotFound,
            CoreError::Plugin(PluginError::InvalidManifest { .. }) => ErrorType::ParseError,
            CoreError::Plugin(PluginError::NoSkills { .. }) => ErrorType::Validation,
            CoreError::Plugin(_) => {
                warn!("unmapped Plugin error variant, defaulting to Unknown");
                ErrorType::Unknown
            }
            _ => {
                warn!("unmapped CoreError variant, defaulting to Unknown");
                ErrorType::Unknown
            }
        };

        let message = err.to_string();
        warn!(
            error_type = ?error_type,
            error = %message,
            "command failed"
        );

        Self {
            message,
            error_type,
        }
    }
}

impl From<String> for CommandError {
    fn from(message: String) -> Self {
        Self {
            message,
            error_type: ErrorType::Unknown,
        }
    }
}

/// Allow `with_file_lock`'s `E: From<io::Error>` bound to be satisfied
/// directly. Lock-acquisition I/O failures (timeout, missing parent, etc.)
/// surface as `IoError` to the frontend so they can be distinguished from
/// validation or parse errors.
impl From<std::io::Error> for CommandError {
    fn from(e: std::io::Error) -> Self {
        Self {
            message: e.to_string(),
            error_type: ErrorType::IoError,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use kiro_market_core::error::{GitError, PluginError, SkillError, ValidationError};
    use rstest::rstest;

    use super::*;

    // -----------------------------------------------------------------------
    // From<CoreError> → ErrorType mapping
    // -----------------------------------------------------------------------

    #[rstest]
    #[case::marketplace_not_found(
        CoreError::Marketplace(MarketplaceError::NotFound { name: "acme".into() }),
        ErrorType::NotFound
    )]
    #[case::marketplace_already_registered(
        CoreError::Marketplace(MarketplaceError::AlreadyRegistered { name: "acme".into() }),
        ErrorType::AlreadyExists
    )]
    #[case::marketplace_no_plugins_found(
        CoreError::Marketplace(MarketplaceError::NoPluginsFound { path: PathBuf::from("/tmp/repo") }),
        ErrorType::NotFound
    )]
    #[case::marketplace_invalid_manifest(
        CoreError::Marketplace(MarketplaceError::InvalidManifest { reason: "bad json".into() }),
        ErrorType::ParseError
    )]
    #[case::skill_already_installed(
        CoreError::Skill(SkillError::AlreadyInstalled { name: "rust-check".into() }),
        ErrorType::AlreadyExists
    )]
    #[case::skill_not_installed(
        CoreError::Skill(SkillError::NotInstalled { name: "missing".into() }),
        ErrorType::NotFound
    )]
    #[case::skill_md_not_found(
        CoreError::Skill(SkillError::SkillMdNotFound { path: PathBuf::from("skills/SKILL.md") }),
        ErrorType::NotFound
    )]
    #[case::validation_invalid_name(
        CoreError::Validation(ValidationError::InvalidName {
            name: "../escape".into(),
            reason: "contains `..`".into(),
        }),
        ErrorType::Validation
    )]
    #[case::git_clone_failed(
        CoreError::Git(GitError::CloneFailed {
            url: "https://github.com/x/y.git".into(),
            source: "network timeout".to_owned().into(),
        }),
        ErrorType::GitError
    )]
    #[case::io_error(
        CoreError::Io(std::io::Error::new(std::io::ErrorKind::NotFound, "gone")),
        ErrorType::IoError
    )]
    #[case::plugin_not_found(
        CoreError::Plugin(PluginError::NotFound {
            plugin: "dotnet".into(),
            marketplace: "ms".into(),
        }),
        ErrorType::NotFound
    )]
    #[case::plugin_manifest_not_found(
        CoreError::Plugin(PluginError::ManifestNotFound {
            path: PathBuf::from("/tmp/plugin.json"),
        }),
        ErrorType::NotFound
    )]
    #[case::plugin_invalid_manifest(
        CoreError::Plugin(PluginError::InvalidManifest {
            path: PathBuf::from("/tmp/plugin.json"),
            reason: "missing name".into(),
        }),
        ErrorType::ParseError
    )]
    #[case::plugin_no_skills(
        CoreError::Plugin(PluginError::NoSkills { name: "empty".into() }),
        ErrorType::Validation
    )]
    fn core_error_maps_to_error_type(#[case] core_err: CoreError, #[case] expected: ErrorType) {
        let cmd_err = CommandError::from(core_err);
        assert_eq!(cmd_err.error_type, expected);
    }

    #[test]
    fn json_error_maps_to_parse_error() {
        let json_err = serde_json::from_str::<String>("not json").unwrap_err();
        let core_err = CoreError::Json(json_err);
        let cmd_err = CommandError::from(core_err);
        assert_eq!(cmd_err.error_type, ErrorType::ParseError);
    }

    // -----------------------------------------------------------------------
    // From<CoreError> preserves message
    // -----------------------------------------------------------------------

    #[test]
    fn core_error_message_is_preserved() {
        let core_err = CoreError::Marketplace(MarketplaceError::NotFound {
            name: "test-marketplace".into(),
        });
        let expected_msg = core_err.to_string();
        let cmd_err = CommandError::from(core_err);
        assert_eq!(cmd_err.message, expected_msg);
    }

    // -----------------------------------------------------------------------
    // From<String> → ErrorType::Unknown
    // -----------------------------------------------------------------------

    #[test]
    fn string_error_maps_to_unknown() {
        let cmd_err = CommandError::from("something went wrong".to_owned());
        assert_eq!(cmd_err.error_type, ErrorType::Unknown);
        assert_eq!(cmd_err.message, "something went wrong");
    }

    // -----------------------------------------------------------------------
    // CommandError::new
    // -----------------------------------------------------------------------

    #[test]
    fn command_error_new_sets_fields() {
        let err = CommandError::new("boom", ErrorType::Validation);
        assert_eq!(err.message, "boom");
        assert_eq!(err.error_type, ErrorType::Validation);
    }
}

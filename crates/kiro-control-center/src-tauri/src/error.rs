use kiro_market_core::error::{
    Error as CoreError, MarketplaceError, PluginError, SkillError,
};
use serde::Serialize;

/// Machine-readable error classification for frontend conditional logic.
///
/// Serialized as snake_case strings and exported to TypeScript via specta.
#[derive(Debug, Clone, Serialize, specta::Type)]
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
    fn from(err: CoreError) -> Self {
        let error_type = match &err {
            CoreError::Marketplace(MarketplaceError::NotFound { .. }) => ErrorType::NotFound,
            CoreError::Marketplace(MarketplaceError::AlreadyRegistered { .. }) => {
                ErrorType::AlreadyExists
            }
            CoreError::Marketplace(_) => ErrorType::ParseError,
            CoreError::Skill(SkillError::AlreadyInstalled { .. }) => ErrorType::AlreadyExists,
            CoreError::Skill(SkillError::NotInstalled { .. }) => ErrorType::NotFound,
            CoreError::Skill(_) => ErrorType::NotFound,
            CoreError::Validation(_) => ErrorType::Validation,
            CoreError::Git(_) => ErrorType::GitError,
            CoreError::Io(_) => ErrorType::IoError,
            CoreError::Json(_) => ErrorType::ParseError,
            CoreError::Plugin(PluginError::NotFound { .. }) => ErrorType::NotFound,
            CoreError::Plugin(_) => ErrorType::NotFound,
            _ => ErrorType::Unknown,
        };

        Self {
            message: err.to_string(),
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

//! Path and name validation utilities.
//!
//! These functions guard against path traversal attacks where untrusted input
//! (marketplace manifests, plugin.json, SKILL.md frontmatter)
//! could escape intended directories via `..` segments or path separators.

use std::path::Path;

use crate::error::ValidationError;

/// Validate that a name is safe to use as a single directory component.
///
/// Rejects names that are empty, contain path separators (`/`, `\`),
/// contain `..` sequences, or are exactly `.`.
///
/// # Errors
///
/// Returns [`ValidationError::InvalidName`] if the name is unsafe.
pub fn validate_name(name: &str) -> Result<(), ValidationError> {
    if name.is_empty() {
        return Err(ValidationError::InvalidName {
            name: name.to_owned(),
            reason: "name must not be empty".into(),
        });
    }

    if name.contains('/') || name.contains('\\') {
        return Err(ValidationError::InvalidName {
            name: name.to_owned(),
            reason: "contains path separator".into(),
        });
    }

    if name.contains("..") {
        return Err(ValidationError::InvalidName {
            name: name.to_owned(),
            reason: "contains `..`".into(),
        });
    }

    if name == "." {
        return Err(ValidationError::InvalidName {
            name: name.to_owned(),
            reason: "must not be `.`".into(),
        });
    }

    Ok(())
}

/// Validate that a relative path does not escape its root via `..` components.
///
/// Also rejects absolute paths (starting with `/` or `\`).
///
/// # Errors
///
/// Returns [`ValidationError::InvalidRelativePath`] if the path is unsafe.
pub fn validate_relative_path(path: &str) -> Result<(), ValidationError> {
    if path.is_empty() {
        return Err(ValidationError::InvalidRelativePath {
            path: path.to_owned(),
            reason: "path must not be empty".into(),
        });
    }

    if path.starts_with('/') || path.starts_with('\\') {
        return Err(ValidationError::InvalidRelativePath {
            path: path.to_owned(),
            reason: "must not be an absolute path".into(),
        });
    }

    // Check each component for `..`.
    for component in Path::new(path).components() {
        if matches!(component, std::path::Component::ParentDir) {
            return Err(ValidationError::InvalidRelativePath {
                path: path.to_owned(),
                reason: "contains `..` component".into(),
            });
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // validate_name
    // -----------------------------------------------------------------------

    #[test]
    fn validate_name_accepts_simple_names() {
        assert!(validate_name("rust-check").is_ok());
        assert!(validate_name("my_plugin").is_ok());
        assert!(validate_name("dotnet-skills-2024").is_ok());
    }

    #[test]
    fn validate_name_rejects_empty() {
        let err = validate_name("").unwrap_err();
        assert!(
            matches!(err, ValidationError::InvalidName { .. }),
            "expected InvalidName, got {err:?}"
        );
    }

    #[test]
    fn validate_name_rejects_forward_slash() {
        assert!(validate_name("../escape").is_err());
        assert!(validate_name("sub/dir").is_err());
    }

    #[test]
    fn validate_name_rejects_backslash() {
        assert!(validate_name("sub\\dir").is_err());
        assert!(validate_name("..\\escape").is_err());
    }

    #[test]
    fn validate_name_rejects_dotdot() {
        assert!(validate_name("..").is_err());
        assert!(validate_name("name..suffix").is_err());
    }

    #[test]
    fn validate_name_rejects_single_dot() {
        assert!(validate_name(".").is_err());
    }

    #[test]
    fn validate_name_accepts_dotfiles() {
        // Names like ".hidden" are fine -- they are valid directory names.
        assert!(validate_name(".hidden").is_ok());
    }

    // -----------------------------------------------------------------------
    // validate_relative_path
    // -----------------------------------------------------------------------

    #[test]
    fn validate_relative_path_accepts_simple_paths() {
        assert!(validate_relative_path("references/types.md").is_ok());
        assert!(validate_relative_path("companion.md").is_ok());
        assert!(validate_relative_path("./skills/").is_ok());
    }

    #[test]
    fn validate_relative_path_rejects_parent_traversal() {
        assert!(validate_relative_path("../escape.md").is_err());
        assert!(validate_relative_path("sub/../../escape.md").is_err());
    }

    #[test]
    fn validate_relative_path_rejects_absolute() {
        assert!(validate_relative_path("/etc/passwd").is_err());
    }

    #[test]
    fn validate_relative_path_rejects_empty() {
        assert!(validate_relative_path("").is_err());
    }

    #[test]
    fn validate_relative_path_accepts_current_dir_prefix() {
        assert!(validate_relative_path("./skills/tunit").is_ok());
    }

    #[test]
    fn validate_relative_path_rejects_backslash_absolute() {
        let err = validate_relative_path("\\windows\\path").unwrap_err();
        assert!(
            matches!(err, ValidationError::InvalidRelativePath { .. }),
            "expected InvalidRelativePath, got {err:?}"
        );
    }
}

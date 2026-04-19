//! Path and name validation utilities.
//!
//! These functions guard against path traversal attacks where untrusted input
//! (marketplace manifests, plugin.json, SKILL.md frontmatter)
//! could escape intended directories via `..` segments or path separators.

use std::path::Path;

use serde::{Deserialize, Deserializer, Serialize};

use crate::error::ValidationError;

/// A string that has been validated as a safe relative path.
///
/// Construction goes through [`RelativePath::new`], which applies
/// [`validate_relative_path`] — so holding a `RelativePath` is a static
/// guarantee that the inner string is non-empty, contains no `..`
/// components, no NUL bytes, and is not an absolute path.
///
/// The newtype replaces a plain `String` in the manifest data model
/// (`PluginSource::RelativePath`, `StructuredSource::GitSubdir.path`) so
/// downstream code never needs to re-validate. `Deserialize` calls
/// `new` internally, so `serde_json::from_slice::<Marketplace>(…)`
/// rejects traversal at parse time.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(transparent)]
pub struct RelativePath(String);

impl RelativePath {
    /// Construct a `RelativePath` from any string-like value, validating it.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError::InvalidRelativePath`] if the input fails
    /// [`validate_relative_path`].
    pub fn new(value: impl Into<String>) -> Result<Self, ValidationError> {
        let value = value.into();
        validate_relative_path(&value)?;
        Ok(Self(value))
    }

    /// View the validated path as a `&str`.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Consume the newtype and return the inner `String`.
    #[must_use]
    pub fn into_inner(self) -> String {
        self.0
    }
}

impl AsRef<str> for RelativePath {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl AsRef<Path> for RelativePath {
    fn as_ref(&self) -> &Path {
        Path::new(&self.0)
    }
}

impl std::fmt::Display for RelativePath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl PartialEq<str> for RelativePath {
    fn eq(&self, other: &str) -> bool {
        self.0 == other
    }
}

impl PartialEq<&str> for RelativePath {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

impl<'de> Deserialize<'de> for RelativePath {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Self::new(s).map_err(serde::de::Error::custom)
    }
}

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

    // NUL bytes can truncate C-string conversions inside syscalls on some
    // platforms, so reject them at the validation boundary.
    if path.contains('\0') {
        return Err(ValidationError::InvalidRelativePath {
            path: path.to_owned(),
            reason: "contains NUL byte".into(),
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

/// Serde adapter that deserialises a `String` and rejects anything
/// [`validate_relative_path`] would reject, raising a custom serde error.
///
/// Use as `#[serde(deserialize_with = "deserialize_relative_path")]` on any
/// manifest field that is later joined to a trusted base directory.
///
/// # Errors
///
/// Returns a serde error if the underlying string deserialises but fails
/// relative-path validation.
pub fn deserialize_relative_path<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    validate_relative_path(&s).map_err(serde::de::Error::custom)?;
    Ok(s)
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
    fn validate_relative_path_rejects_nul_byte() {
        let err = validate_relative_path("skills/\0injected").unwrap_err();
        assert!(
            matches!(err, ValidationError::InvalidRelativePath { .. }),
            "expected InvalidRelativePath, got {err:?}"
        );
        assert!(
            err.to_string().contains("NUL"),
            "error should mention NUL: {err}"
        );
    }

    #[test]
    fn validate_relative_path_rejects_backslash_absolute() {
        let err = validate_relative_path("\\windows\\path").unwrap_err();
        assert!(
            matches!(err, ValidationError::InvalidRelativePath { .. }),
            "expected InvalidRelativePath, got {err:?}"
        );
    }

    // -----------------------------------------------------------------------
    // deserialize_relative_path
    // -----------------------------------------------------------------------

    #[derive(Debug, serde::Deserialize)]
    struct Wrapper {
        #[serde(deserialize_with = "deserialize_relative_path")]
        path: String,
    }

    #[test]
    fn deserialize_relative_path_accepts_safe_paths() {
        let w: Wrapper = serde_json::from_str(r#"{"path":"./skills/test"}"#).expect("parse");
        assert_eq!(w.path, "./skills/test");
    }

    #[test]
    fn deserialize_relative_path_rejects_parent_traversal() {
        let err = serde_json::from_str::<Wrapper>(r#"{"path":"../../etc"}"#)
            .expect_err("should reject traversal");
        assert!(
            err.to_string().contains("..") || err.to_string().contains("path"),
            "error should mention path/..: {err}"
        );
    }

    #[test]
    fn deserialize_relative_path_rejects_absolute_unix() {
        let err = serde_json::from_str::<Wrapper>(r#"{"path":"/etc/passwd"}"#)
            .expect_err("should reject absolute path");
        assert!(err.to_string().contains("absolute"), "got: {err}");
    }

    #[test]
    fn deserialize_relative_path_rejects_absolute_windows() {
        let err = serde_json::from_str::<Wrapper>(r#"{"path":"\\windows\\system32"}"#)
            .expect_err("should reject backslash-absolute path");
        assert!(err.to_string().contains("absolute"), "got: {err}");
    }

    #[test]
    fn deserialize_relative_path_rejects_empty() {
        let err = serde_json::from_str::<Wrapper>(r#"{"path":""}"#)
            .expect_err("should reject empty path");
        assert!(err.to_string().contains("empty"), "got: {err}");
    }
}

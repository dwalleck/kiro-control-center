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

/// Names reserved by Windows for legacy device handles. Trying to create
/// a file or directory with one of these names (with or without extension)
/// fails on Windows in interesting ways: the OS short-circuits the path to
/// the device, so opening `CON.txt` returns a console handle, and a folder
/// called `NUL/` is unwritable. Reject them at the validator regardless of
/// platform so the cache layout works the same on every host.
const WINDOWS_RESERVED_NAMES: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Validate that a name is safe to use as a single directory component on
/// every platform we support.
///
/// Rejects names that:
/// - are empty
/// - contain a path separator (`/`, `\`) — would split into multiple components
/// - contain `..` — `Path::components` would surface a parent-dir component
/// - are exactly `.` — refers to the current directory
/// - contain a NUL byte — truncates C-string conversions in syscalls
/// - contain any other ASCII control character (0x01..=0x1F, 0x7F) — these
///   render as garbled or invisible bytes in logs and shells, and several
///   filesystems reject them outright
/// - have leading or trailing ASCII whitespace — leading whitespace makes
///   the directory look empty in shell listings; trailing whitespace and
///   trailing dots are silently stripped by NTFS, which would alias two
///   apparently distinct names to the same on-disk directory
/// - match a Windows reserved device name (CON, PRN, AUX, NUL, COM1-9,
///   LPT1-9), comparison case-insensitive and applied to both the bare
///   name and the stem-before-extension. The OS reserves these regardless
///   of extension, so `nul.txt` is rejected too. This matters even on
///   Unix because the marketplace cache may be mounted/synced to a
///   Windows host.
///
/// Internal whitespace (e.g. `Terraform Agent`) is permitted because real
/// Copilot agents use it; only the leading and trailing positions are
/// rejected.
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

    // Control character rejection. NUL is called out separately for a
    // clearer error message; everything else (BEL, BS, VT, ESC, DEL, …)
    // collapses into the generic case so the user knows the byte index.
    if let Some((idx, ch)) = name
        .char_indices()
        .find(|&(_, c)| c == '\0' || c.is_control())
    {
        let reason = if ch == '\0' {
            "contains NUL byte".to_owned()
        } else {
            format!(
                "contains control character U+{:04X} at byte {idx}",
                ch as u32
            )
        };
        return Err(ValidationError::InvalidName {
            name: name.to_owned(),
            reason,
        });
    }

    if name.chars().next().is_some_and(|c| c.is_ascii_whitespace()) {
        return Err(ValidationError::InvalidName {
            name: name.to_owned(),
            reason: "must not start with whitespace".into(),
        });
    }
    if name
        .chars()
        .next_back()
        .is_some_and(|c| c.is_ascii_whitespace())
    {
        return Err(ValidationError::InvalidName {
            name: name.to_owned(),
            reason: "must not end with whitespace".into(),
        });
    }

    // NTFS strips trailing dots when creating files, which would silently
    // alias `foo.` and `foo` to the same on-disk directory. Reject so the
    // cache layout is unambiguous across platforms.
    if name.ends_with('.') && name != "." {
        return Err(ValidationError::InvalidName {
            name: name.to_owned(),
            reason: "must not end with `.`".into(),
        });
    }

    // Windows-reserved device names. Compare both the bare name and the
    // stem-before-first-`.` so `CON`, `con`, `Con.txt`, `con.tar.gz` are
    // all rejected. Case-insensitive on ASCII because the reserved table
    // is ASCII.
    let stem = name.split('.').next().unwrap_or(name);
    let is_reserved = |candidate: &str| {
        WINDOWS_RESERVED_NAMES
            .iter()
            .any(|reserved| reserved.eq_ignore_ascii_case(candidate))
    };
    if is_reserved(name) || is_reserved(stem) {
        return Err(ValidationError::InvalidName {
            name: name.to_owned(),
            reason: format!(
                "matches a Windows reserved device name (`{stem}`); rename to avoid conflicts"
            ),
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

    // Reject any backslash anywhere in the path. `Path::components` treats
    // `\` as a literal on Unix but as a separator on Windows, so a string
    // like `sub\..\..\etc` would pass the `..` check on Unix yet traverse
    // on Windows. Rejecting `\` at the boundary makes validation
    // platform-independent. Legitimate relative paths in this codebase use
    // forward slashes (see `DiscoveredPlugin::as_relative_path_string`).
    if path.contains('\\') {
        return Err(ValidationError::InvalidRelativePath {
            path: path.to_owned(),
            reason: "contains backslash (use `/` as a separator)".into(),
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
    use rstest::rstest;

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

    #[test]
    fn validate_name_accepts_internal_whitespace() {
        // "Terraform Agent" is a real Copilot agent name. Internal spaces
        // must keep working even though leading/trailing whitespace is
        // rejected — otherwise we'd break every Copilot multi-word agent.
        assert!(validate_name("Terraform Agent").is_ok());
    }

    #[test]
    fn validate_name_rejects_nul_byte() {
        let err = validate_name("foo\0bar").unwrap_err();
        assert!(
            matches!(&err, ValidationError::InvalidName { reason, .. } if reason.contains("NUL")),
            "expected NUL-specific reason, got {err:?}"
        );
    }

    #[rstest]
    #[case::newline("foo\nbar")]
    #[case::bell("alert\x07")]
    #[case::tab("tab\there")]
    #[case::del("del\x7Fend")]
    fn validate_name_rejects_other_control_characters(#[case] raw: &str) {
        let err = validate_name(raw).unwrap_err();
        assert!(
            matches!(&err, ValidationError::InvalidName { reason, .. } if reason.contains("control character")),
            "expected control-character reason for {raw:?}, got {err:?}"
        );
    }

    #[rstest]
    // Leading whitespace creates folders that look empty in `ls`.
    // Trailing whitespace is silently stripped by NTFS, aliasing two
    // distinct names to the same on-disk directory. Tab / newline are
    // covered by the control-character check, which fires first; the
    // remaining ASCII-whitespace cases are leading/trailing space.
    #[case::leading(" leading")]
    #[case::trailing("trailing ")]
    fn validate_name_rejects_leading_and_trailing_space(#[case] raw: &str) {
        let err = validate_name(raw).unwrap_err();
        assert!(
            matches!(&err, ValidationError::InvalidName { reason, .. } if reason.contains("whitespace")),
            "expected whitespace rejection for {raw:?}, got {err:?}"
        );
    }

    #[test]
    fn validate_name_rejects_trailing_dot() {
        // NTFS strips trailing dots — "foo." and "foo" would alias.
        let err = validate_name("foo.").unwrap_err();
        assert!(
            matches!(&err, ValidationError::InvalidName { reason, .. } if reason.contains("end with `.`")),
            "got {err:?}"
        );
    }

    #[rstest]
    #[case::con_upper("CON")]
    #[case::con_lower("con")]
    #[case::prn("PRN")]
    #[case::aux("AUX")]
    #[case::nul_upper("NUL")]
    #[case::nul_lower("nul")]
    #[case::com1("COM1")]
    #[case::lpt9_lower("lpt9")]
    #[case::con_with_ext("Con.txt")]
    #[case::nul_double_ext("nul.tar.gz")]
    fn validate_name_rejects_windows_reserved_names(#[case] reserved: &str) {
        let err = validate_name(reserved).unwrap_err();
        assert!(
            matches!(&err, ValidationError::InvalidName { reason, .. } if reason.contains("Windows reserved")),
            "expected Windows-reserved rejection for {reserved:?}, got {err:?}"
        );
    }

    #[test]
    fn validate_name_accepts_names_that_merely_share_prefix_with_reserved() {
        // "console", "auxiliary", "command" are NOT Windows reserved —
        // only the exact device names CON, AUX, COM1 etc. are. Don't
        // over-reject.
        assert!(validate_name("console").is_ok());
        assert!(validate_name("auxiliary").is_ok());
        assert!(validate_name("command-runner").is_ok());
        assert!(validate_name("nullable").is_ok());
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

    #[test]
    fn validate_relative_path_rejects_embedded_backslash() {
        // Regression for a Unix/Windows asymmetry: `Path::components` treats
        // `\` as a literal on Unix, so without explicit rejection a string
        // like `sub\..\..\etc` would pass the `..` check on Unix but
        // traverse on Windows or in a shell. The validator must reject any
        // embedded backslash regardless of platform.
        let err = validate_relative_path("sub\\..\\..\\etc").unwrap_err();
        assert!(
            matches!(err, ValidationError::InvalidRelativePath { .. }),
            "expected InvalidRelativePath, got {err:?}"
        );
        assert!(
            err.to_string().contains("backslash"),
            "error should mention backslash: {err}"
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

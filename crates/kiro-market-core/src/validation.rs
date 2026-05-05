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

    /// Construct a `RelativePath` from a value the caller has already
    /// validated upstream. `pub(crate)` so external callers cannot bypass
    /// `validate_relative_path`.
    ///
    /// **Caller contract:** `value` must already satisfy
    /// `validate_relative_path` — non-empty, no leading `/` or `\`, no
    /// embedded `\` or NUL, no `..` component. The current caller —
    /// [`crate::plugin::DiscoveredPlugin::as_relative_path`] — is sound
    /// only because [`crate::plugin::try_read_plugin`] runs
    /// `validate_relative_path` against the assembled path before
    /// constructing the `DiscoveredPlugin`. Adding a new internal caller
    /// requires re-establishing this argument; the `debug_assert!` below
    /// catches a contract violation in tests.
    pub(crate) fn from_internal_unchecked(value: String) -> Self {
        debug_assert!(
            validate_relative_path(&value).is_ok(),
            "from_internal_unchecked called with invalid path: {value:?}"
        );
        Self(value)
    }

    /// The conventional `agents/` scan-root used by the translated
    /// agent install path when the source-side scan root cannot be
    /// recovered from `meta.source_path` (legacy `agent.md` discovered
    /// without a configured scan path, hand-synthesised test fixtures,
    /// etc.). Replaces the previous `from_internal_unchecked("agents".to_owned())`
    /// site at `KiroProject::install_agent_inner`: the
    /// unchecked-constructor pattern was sound but anonymous —
    /// every reader had to re-derive that `"agents"` is a constant
    /// string. A named `agents_root()` makes the intent explicit and
    /// removes one audited-by-convention site.
    #[must_use]
    pub fn agents_root() -> Self {
        Self::from_internal_unchecked("agents".to_owned())
    }

    /// Convert a `Path` to a [`RelativePath`] by stripping `base` and
    /// normalising path separators to forward-slash. Returns `Err` if
    /// `path` is not under `base` or the resulting relative path fails
    /// [`validate_relative_path`].
    ///
    /// Forward-slash conversion is required because [`RelativePath::new`]
    /// rejects backslashes for cross-platform portability of the wire
    /// format. On Windows, `Path::strip_prefix(...).to_string_lossy()`
    /// returns backslashes; without normalisation, `RelativePath::new`
    /// fails. The recipe (`Components::Normal` + `join("/")`) drops any
    /// platform-specific prefix or root components, producing a
    /// purely-forward-slash representation regardless of the host OS.
    ///
    /// Used by install-side code to record where in the plugin tree an
    /// artifact came from, so detection can look it up directly without
    /// probing each configured manifest scan path.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError::InvalidRelativePath`] if `path` is
    /// not under `base`, or whatever [`validate_relative_path`] returns
    /// if the normalised string fails its checks.
    pub fn from_path_under(path: &Path, base: &Path) -> Result<Self, ValidationError> {
        let rel = path
            .strip_prefix(base)
            .map_err(|_| ValidationError::InvalidRelativePath {
                path: path.display().to_string(),
                reason: format!("path is not under base directory `{}`", base.display()),
            })?;
        // Two-pass normalisation. On Windows-native paths, `Components`
        // already splits on `\` and `Normal` strings have no embedded
        // backslashes — `join("/")` is enough. On Unix interpreting a
        // Windows-shaped path (or any synthetic input), `\` is NOT a
        // path separator, so a string like `"agents\reviewer.md"` lands
        // as ONE `Normal` component and we'd hand `RelativePath::new` a
        // backslash it rejects. The explicit `.replace('\\', '/')`
        // closes that gap; harmless when components already split.
        //
        // Components are converted via `to_str()`,
        // not `to_string_lossy()`. Lossy conversion silently substitutes
        // `U+FFFD` for invalid UTF-8 sequences, so a non-UTF-8 OsStr
        // component would round-trip as a valid-looking `RelativePath`
        // that doesn't correspond to anything on disk — detection-side
        // hashing later misses with `NotFound` and the user sees a
        // misleading "missing file" error. Failing the construction
        // here surfaces the real cause at the parse boundary.
        // Match each component explicitly rather than `_ => None`. Only
        // `CurDir` (`.`) is silently discarded; `ParentDir` (`..`),
        // `Prefix` (Windows drive letters), and `RootDir` (a leading
        // separator) are structurally illegal under a relative-path
        // contract and must error rather than silently disappear into the
        // filter. Without this gate a hand-built path lexically containing
        // `..` after `strip_prefix` would round-trip as a "clean"
        // `RelativePath` that doesn't represent the input.
        let parts: Vec<&str> = rel
            .components()
            .filter_map(|c| match c {
                std::path::Component::Normal(s) => Some(Ok(s)),
                std::path::Component::CurDir => None,
                std::path::Component::ParentDir => {
                    Some(Err(ValidationError::InvalidRelativePath {
                        path: path.display().to_string(),
                        reason: "path contains a `..` component".to_owned(),
                    }))
                }
                std::path::Component::Prefix(_) | std::path::Component::RootDir => {
                    Some(Err(ValidationError::InvalidRelativePath {
                        path: path.display().to_string(),
                        reason: "path contains an absolute or prefix component after \
                                 strip_prefix"
                            .to_owned(),
                    }))
                }
            })
            .map(|c| {
                c.and_then(|s| {
                    s.to_str()
                        .ok_or_else(|| ValidationError::InvalidRelativePath {
                            path: path.display().to_string(),
                            reason: "path contains a non-UTF-8 component".to_owned(),
                        })
                })
            })
            .collect::<Result<_, _>>()?;
        let rel_str = parts.join("/").replace('\\', "/");
        // `path == base` produces an empty rel string (Components yields
        // zero Normal entries), which `RelativePath::new` rejects.
        // Substitute "." so the call still succeeds and downstream
        // `plugin_dir.join(".").join(name)` resolves correctly. Closes
        // the bare-path skill at plugin root regression.
        let normalised = if rel_str.is_empty() {
            ".".to_owned()
        } else {
            rel_str
        };
        Self::new(normalised)
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

/// A string that has been validated as a safe agent / skill / plugin name.
///
/// Construction goes through [`AgentName::new`], which applies
/// [`validate_name`] — so holding an `AgentName` is a static guarantee
/// that the inner string passed name validation (non-empty, no path
/// separators, no `..`, no NUL, no Windows reserved names, etc.).
///
/// The newtype replaces a plain `String` for the validated `name` field
/// of [`crate::agent::parse_native::NativeAgentBundle`] so downstream
/// install code never needs to re-validate. `Deserialize` calls `new`
/// internally, so any future serializable type that embeds an
/// `AgentName` rejects bad names at parse time.
///
/// The native-agent projection (`NativeAgentProjection`) deliberately
/// keeps a raw `Option<String>` for the wire-format `name` field so the
/// post-parse conversion can route into distinct
/// [`crate::agent::parse_native::NativeParseFailure`] variants
/// (`MissingName` vs `InvalidName(reason)` vs `InvalidJson`) — this
/// granularity is part of the contract surfaced via
/// `service::native_parse_failure_to_agent_error`.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(transparent)]
pub struct AgentName(String);

impl AgentName {
    /// Construct an `AgentName` from any string-like value, validating it.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError::InvalidName`] if the input fails
    /// [`validate_name`].
    pub fn new(value: impl Into<String>) -> Result<Self, ValidationError> {
        let value = value.into();
        validate_name(&value)?;
        Ok(Self(value))
    }

    /// View the validated name as a `&str`.
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

impl TryFrom<&str> for AgentName {
    type Error = ValidationError;
    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl TryFrom<String> for AgentName {
    type Error = ValidationError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl AsRef<str> for AgentName {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for AgentName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl PartialEq<str> for AgentName {
    fn eq(&self, other: &str) -> bool {
        self.0 == other
    }
}

impl PartialEq<&str> for AgentName {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

impl<'de> Deserialize<'de> for AgentName {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Self::new(s).map_err(serde::de::Error::custom)
    }
}

/// Validated marketplace name. Routes through [`validate_name`] at construction
/// — non-empty, no NUL/control bytes, no path-traversal, no Windows-reserved
/// names. The `serde(transparent)` representation keeps the JSON wire format
/// byte-identical to a plain string; `Deserialize` is routed through `new` so
/// `serde_json::from_slice` rejects malformed names at parse time.
///
/// Deliberately does NOT derive `Default` — `MarketplaceName::default()` would
/// return `MarketplaceName(String::new())` which `validate_name` rejects.
/// Matches the existing `RelativePath` / `AgentName` / `GitRef` precedent.
///
/// `Ord`/`PartialOrd` are derived for use as `BTreeMap` keys in
/// `installed_plugins`'s aggregator (see `project.rs`). Lexicographic ordering
/// on the inner string is well-defined and semantically equivalent to
/// `String`'s ordering. Deviates from `RelativePath` / `AgentName` / `GitRef`
/// (which don't derive `Ord`) because none of those types are used as map keys
/// today.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(transparent)]
pub struct MarketplaceName(String);

impl MarketplaceName {
    /// Construct after validation.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError::InvalidName`] if the input fails
    /// [`validate_name`].
    pub fn new(value: impl Into<String>) -> Result<Self, ValidationError> {
        let value = value.into();
        validate_name(&value)?;
        Ok(MarketplaceName(value))
    }

    /// View the validated name as a `&str`.
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

impl TryFrom<&str> for MarketplaceName {
    type Error = ValidationError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl TryFrom<String> for MarketplaceName {
    type Error = ValidationError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl AsRef<str> for MarketplaceName {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for MarketplaceName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.0, f)
    }
}

impl PartialEq<str> for MarketplaceName {
    fn eq(&self, other: &str) -> bool {
        self.0 == other
    }
}

impl PartialEq<&str> for MarketplaceName {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

impl<'de> Deserialize<'de> for MarketplaceName {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

/// Validated plugin name. Same shape as [`MarketplaceName`]; see that type's
/// documentation for the construction, serde, and Ord-derive contracts.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(transparent)]
pub struct PluginName(String);

impl PluginName {
    /// Construct after validation.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError::InvalidName`] if the input fails
    /// [`validate_name`].
    pub fn new(value: impl Into<String>) -> Result<Self, ValidationError> {
        let value = value.into();
        validate_name(&value)?;
        Ok(PluginName(value))
    }

    /// View the validated name as a `&str`.
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

impl TryFrom<&str> for PluginName {
    type Error = ValidationError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl TryFrom<String> for PluginName {
    type Error = ValidationError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl AsRef<str> for PluginName {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for PluginName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.0, f)
    }
}

impl PartialEq<str> for PluginName {
    fn eq(&self, other: &str) -> bool {
        self.0 == other
    }
}

impl PartialEq<&str> for PluginName {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

impl<'de> Deserialize<'de> for PluginName {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
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

    // -- AgentName -----------------------------------------------------

    #[test]
    fn agent_name_new_accepts_valid() {
        let n = AgentName::new("rev").expect("valid name");
        assert_eq!(n.as_str(), "rev");
    }

    #[test]
    fn agent_name_new_rejects_traversal() {
        let err = AgentName::new("../evil").expect_err("must reject");
        assert!(err.to_string().contains(".."), "got: {err}");
    }

    #[test]
    fn agent_name_new_rejects_empty() {
        AgentName::new("").expect_err("must reject empty");
    }

    #[test]
    fn agent_name_accessors_round_trip() {
        let n = AgentName::new("rev").expect("valid name");
        assert_eq!(n.as_str(), "rev");
        assert_eq!(<AgentName as AsRef<str>>::as_ref(&n), "rev");
        assert_eq!(format!("{n}"), "rev");
        assert_eq!(n.clone().into_inner(), String::from("rev"));
    }

    #[test]
    fn agent_name_partial_eq_against_str_and_ref_str() {
        let n = AgentName::new("rev").expect("valid name");
        // Both impls compile and evaluate — locks the ergonomic surface
        // that downstream `assert_eq!(bundle.name, "rev")` relies on.
        assert!(n == *"rev");
        assert!(n == "rev");
    }

    #[derive(Debug, serde::Deserialize)]
    struct AgentNameWrapper {
        name: AgentName,
    }

    #[test]
    fn deserialize_agent_name_accepts_valid() {
        let w: AgentNameWrapper = serde_json::from_str(r#"{"name":"rev"}"#).expect("parse");
        assert_eq!(w.name, "rev");
    }

    #[test]
    fn deserialize_agent_name_rejects_traversal() {
        let err = serde_json::from_str::<AgentNameWrapper>(r#"{"name":"../evil"}"#)
            .expect_err("must reject");
        assert!(
            err.to_string().contains("..") || err.to_string().contains("name"),
            "got: {err}"
        );
    }

    #[test]
    fn deserialize_agent_name_rejects_empty() {
        serde_json::from_str::<AgentNameWrapper>(r#"{"name":""}"#).expect_err("must reject empty");
    }

    #[test]
    fn deserialize_agent_name_rejects_path_separator() {
        serde_json::from_str::<AgentNameWrapper>(r#"{"name":"sub/dir"}"#)
            .expect_err("must reject path separator");
    }

    #[test]
    fn serialize_agent_name_is_transparent_string() {
        // Locks the `#[serde(transparent)]` choice — without it the wire
        // format becomes `{"0":"rev"}` and any future `AgentName`-bearing
        // wire-format type silently shifts shape. Removing
        // `#[serde(transparent)]` would break this assertion.
        let n = AgentName::new("rev").expect("valid name");
        assert_eq!(serde_json::to_string(&n).expect("ser"), r#""rev""#);
    }

    #[test]
    fn agent_name_round_trips_through_serde_json() {
        let original = AgentName::new("rev").expect("valid name");
        let wire = serde_json::to_string(&original).expect("ser");
        let parsed: AgentName = serde_json::from_str(&wire).expect("de");
        assert_eq!(parsed, original);
    }

    // ──── MarketplaceName ────────────────────────────────────────────────

    #[test]
    fn marketplace_name_new_accepts_valid() {
        let name = MarketplaceName::new("kiro-starter-kit").expect("valid");
        assert_eq!(name.as_str(), "kiro-starter-kit");
    }

    #[test]
    fn marketplace_name_new_rejects_empty() {
        assert!(MarketplaceName::new("").is_err());
    }

    #[test]
    fn marketplace_name_new_rejects_traversal() {
        assert!(MarketplaceName::new("../etc").is_err());
        assert!(MarketplaceName::new("..").is_err());
    }

    #[test]
    fn marketplace_name_new_rejects_nul_byte() {
        assert!(MarketplaceName::new("foo\0bar").is_err());
    }

    #[test]
    fn marketplace_name_partial_eq_against_str_and_ref_str() {
        let name = MarketplaceName::new("mp").expect("valid");
        assert_eq!(name, *"mp");
        assert_eq!(name, "mp");
    }

    #[test]
    fn marketplace_name_accessors_round_trip() {
        let name = MarketplaceName::new("mp").expect("valid");
        let s = name.clone().into_inner();
        assert_eq!(s, "mp");
        assert_eq!(name.as_str(), "mp");
    }

    #[derive(Debug, serde::Deserialize)]
    struct MarketplaceNameWrapper {
        name: MarketplaceName,
    }

    #[test]
    fn deserialize_marketplace_name_accepts_valid() {
        let w: MarketplaceNameWrapper =
            serde_json::from_str(r#"{"name":"kiro-starter-kit"}"#).expect("valid");
        assert_eq!(w.name.as_str(), "kiro-starter-kit");
    }

    #[test]
    fn deserialize_marketplace_name_rejects_traversal() {
        let result: Result<MarketplaceNameWrapper, _> =
            serde_json::from_str(r#"{"name":"../etc"}"#);
        assert!(result.is_err());
    }

    #[test]
    fn deserialize_marketplace_name_rejects_empty() {
        let result: Result<MarketplaceNameWrapper, _> = serde_json::from_str(r#"{"name":""}"#);
        assert!(result.is_err());
    }

    #[test]
    fn serialize_marketplace_name_is_transparent_string() {
        let name = MarketplaceName::new("mp").expect("valid");
        let json = serde_json::to_string(&name).expect("serialize");
        assert_eq!(json, r#""mp""#);
    }

    #[test]
    fn marketplace_name_round_trips_through_serde_json() {
        let name = MarketplaceName::new("kiro-starter-kit").expect("valid");
        let json = serde_json::to_string(&name).expect("serialize");
        let parsed: MarketplaceName = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed, name);
    }

    #[test]
    fn marketplace_name_ord_is_lexicographic_on_inner() {
        let a = MarketplaceName::new("alpha").expect("valid");
        let b = MarketplaceName::new("bravo").expect("valid");
        assert!(a < b);
    }

    #[test]
    fn marketplace_name_ord_matches_inner_string_ord() {
        let a = MarketplaceName::new("alpha").expect("valid");
        let b = MarketplaceName::new("bravo").expect("valid");
        assert_eq!(
            a.cmp(&b),
            a.as_str().cmp(b.as_str()),
            "MarketplaceName::cmp must match inner String::cmp byte-for-byte"
        );
    }

    // ──── PluginName ────────────────────────────────────────────────────

    #[test]
    fn plugin_name_new_accepts_valid() {
        let name = PluginName::new("kiro-code-reviewer").expect("valid");
        assert_eq!(name.as_str(), "kiro-code-reviewer");
    }

    #[test]
    fn plugin_name_new_rejects_empty() {
        assert!(PluginName::new("").is_err());
    }

    #[test]
    fn plugin_name_new_rejects_traversal() {
        assert!(PluginName::new("../etc").is_err());
    }

    #[test]
    fn plugin_name_new_rejects_nul_byte() {
        assert!(PluginName::new("foo\0bar").is_err());
    }

    #[test]
    fn plugin_name_partial_eq_against_str_and_ref_str() {
        let name = PluginName::new("p").expect("valid");
        assert_eq!(name, *"p");
        assert_eq!(name, "p");
    }

    #[derive(Debug, serde::Deserialize)]
    struct PluginNameWrapper {
        name: PluginName,
    }

    #[test]
    fn deserialize_plugin_name_accepts_valid() {
        let w: PluginNameWrapper =
            serde_json::from_str(r#"{"name":"kiro-code-reviewer"}"#).expect("valid");
        assert_eq!(w.name.as_str(), "kiro-code-reviewer");
    }

    #[test]
    fn deserialize_plugin_name_rejects_traversal() {
        let result: Result<PluginNameWrapper, _> = serde_json::from_str(r#"{"name":"../etc"}"#);
        assert!(result.is_err());
    }

    #[test]
    fn serialize_plugin_name_is_transparent_string() {
        let name = PluginName::new("p").expect("valid");
        let json = serde_json::to_string(&name).expect("serialize");
        assert_eq!(json, r#""p""#);
    }

    // The five tests below mirror the corresponding `marketplace_name_*`
    // tests. PluginName previously lacked parallel
    // accessor / empty-deserialize / round-trip / Ord coverage, which
    // would let a regression slip through one type but not the other.
    // The Ord lock in particular is load-bearing: if `PluginName` ever
    // becomes a `BTreeMap` key (it isn't today, but `MarketplaceName`
    // already is), an inconsistent comparator would silently misorder
    // entries. Locking the contract symmetrically prevents that.

    #[test]
    fn plugin_name_accessors_round_trip() {
        let name = PluginName::new("p").expect("valid");
        let s = name.clone().into_inner();
        assert_eq!(s, "p");
        assert_eq!(name.as_str(), "p");
    }

    #[test]
    fn deserialize_plugin_name_rejects_empty() {
        let result: Result<PluginNameWrapper, _> = serde_json::from_str(r#"{"name":""}"#);
        assert!(result.is_err());
    }

    #[test]
    fn plugin_name_round_trips_through_serde_json() {
        let name = PluginName::new("kiro-code-reviewer").expect("valid");
        let json = serde_json::to_string(&name).expect("serialize");
        let parsed: PluginName = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed, name);
    }

    #[test]
    fn plugin_name_ord_is_lexicographic_on_inner() {
        let a = PluginName::new("alpha").expect("valid");
        let b = PluginName::new("bravo").expect("valid");
        assert!(a < b);
    }

    #[test]
    fn plugin_name_ord_matches_inner_string_ord() {
        let a = PluginName::new("alpha").expect("valid");
        let b = PluginName::new("bravo").expect("valid");
        assert_eq!(
            a.cmp(&b),
            a.as_str().cmp(b.as_str()),
            "PluginName::cmp must match inner String::cmp byte-for-byte"
        );
    }

    // -----------------------------------------------------------------
    // RelativePath::from_path_under (install↔detect symmetry foundation)
    // -----------------------------------------------------------------

    #[test]
    fn from_path_under_normalizes_backslashes() {
        use std::path::PathBuf;
        // Synthesise a path with backslash components. PathBuf is just
        // bytes underneath, so this works on any platform — tests the
        // Windows-native input shape without requiring Windows CI.
        let plugin_dir = PathBuf::from("/tmp/plugin");
        let source = PathBuf::from("/tmp/plugin/agents\\reviewer.md");
        let rel = RelativePath::from_path_under(&source, &plugin_dir)
            .expect("backslash path under plugin_dir should normalize");
        assert_eq!(rel.as_str(), "agents/reviewer.md");
    }

    #[test]
    fn from_path_under_rejects_path_outside_base() {
        use std::path::PathBuf;
        let plugin_dir = PathBuf::from("/tmp/plugin");
        let outside = PathBuf::from("/etc/passwd");
        assert!(
            RelativePath::from_path_under(&outside, &plugin_dir).is_err(),
            "path not under plugin_dir must error, not silently produce a bogus rel"
        );
    }

    #[test]
    fn from_path_under_round_trips_simple_unix_path() {
        use std::path::PathBuf;
        let plugin_dir = PathBuf::from("/tmp/plugin");
        let source = PathBuf::from("/tmp/plugin/agents/reviewer.md");
        let rel = RelativePath::from_path_under(&source, &plugin_dir).expect("valid input");
        assert_eq!(rel.as_str(), "agents/reviewer.md");
    }

    /// A manifest declaring `skills: ["my-skill"]`
    /// (bare path, no `./skills/` directory) makes `discover_skill_dirs`
    /// set `scan_root = candidate.parent() = plugin_root`, then install
    /// calls `from_path_under(scan_root=plugin_root, plugin_dir=plugin_root)`.
    /// Previously this errored with empty-rel; install pushed `FailedSkill`
    /// for a skill that pre-PR would have installed cleanly. Now
    /// returns `RelativePath("." )` so detection can use
    /// `plugin_dir.join(".").join(name)` to resolve back to the right
    /// directory.
    #[test]
    fn from_path_under_returns_dot_when_path_equals_base() {
        use std::path::PathBuf;
        let plugin_dir = PathBuf::from("/tmp/plugin");
        let rel = RelativePath::from_path_under(&plugin_dir, &plugin_dir)
            .expect("path == base must produce a valid sentinel, not error");
        assert_eq!(rel.as_str(), ".");
    }

    /// A path component containing invalid UTF-8 must
    /// produce `ValidationError::InvalidRelativePath { reason: "...non-UTF-8..." }`,
    /// not a `to_string_lossy`-substituted U+FFFD that round-trips
    /// through validation but doesn't match anything on disk. Unix-only
    /// because Windows `OsStr` is WTF-16 and constructing an invalid
    /// component takes a different path; the production code is
    /// platform-agnostic — a Unix-only regression test is sufficient
    /// since it pins the same `to_str()` call site on every target.
    #[cfg(unix)]
    #[test]
    fn from_path_under_rejects_non_utf8_component() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;
        use std::path::{Path, PathBuf};

        // 0xFF is invalid as a UTF-8 start byte; the resulting OsStr is
        // a valid Unix filename but has no UTF-8 representation.
        let bad_component = OsStr::from_bytes(&[b'a', 0xFF, b'b']);
        let base = PathBuf::from("/plugins/foo");
        let path = base.join(Path::new(bad_component));

        let err = RelativePath::from_path_under(&path, &base)
            .expect_err("non-UTF-8 component must error, not lossy-substitute");
        match err {
            crate::error::ValidationError::InvalidRelativePath { reason, .. } => {
                assert!(
                    reason.contains("non-UTF-8"),
                    "reason should call out non-UTF-8, got: {reason}"
                );
            }
            other => panic!("expected InvalidRelativePath, got {other:?}"),
        }
    }

    /// `..` component in the post-`strip_prefix` path must error rather
    /// than silently disappear into the component filter. Without this
    /// gate a hand-built path lexically containing `..` would round-trip
    /// as a "clean" `RelativePath` that doesn't represent the input —
    /// the precondition for a path-traversal smuggling bug.
    ///
    /// Constructs the relative path directly via `Path::new("..")` and
    /// passes a base of `""` so `strip_prefix` is a no-op and the `..`
    /// reaches the component-classification arm.
    #[test]
    fn from_path_under_rejects_parent_dir_component() {
        use std::path::Path;

        let err = RelativePath::from_path_under(Path::new("../etc/passwd"), Path::new(""))
            .expect_err("ParentDir component must error, not silently disappear");
        match err {
            crate::error::ValidationError::InvalidRelativePath { reason, .. } => {
                assert!(
                    reason.contains(".."),
                    "reason should call out the `..` component, got: {reason}"
                );
            }
            other => panic!("expected InvalidRelativePath, got {other:?}"),
        }
    }

    /// `RootDir` (a leading separator) is structurally illegal under a
    /// relative-path contract. Pre-S-4 the `_ => None` filter silently
    /// dropped it; this test pins the explicit error so a future
    /// refactor can't collapse the arm back into a silent discard.
    /// `Prefix` (Windows drive letters) shares the same arm and is
    /// covered by the same regression by construction.
    #[cfg(unix)]
    #[test]
    fn from_path_under_rejects_absolute_path() {
        use std::path::Path;

        let err = RelativePath::from_path_under(Path::new("/etc/passwd"), Path::new(""))
            .expect_err("absolute path must error, not silently disappear");
        match err {
            crate::error::ValidationError::InvalidRelativePath { reason, .. } => {
                assert!(
                    reason.contains("absolute") || reason.contains("prefix"),
                    "reason should call out the absolute/prefix component, got: {reason}"
                );
            }
            other => panic!("expected InvalidRelativePath, got {other:?}"),
        }
    }
}

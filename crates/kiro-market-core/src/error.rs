//! Domain error types for kiro-market-core.
//!
//! Errors are organised into thematic groups ([`MarketplaceError`],
//! [`PluginError`], [`SkillError`], [`AgentError`], [`GitError`]) and a
//! top-level [`Error`] enum that unifies them via `From` conversions.

use std::io;
use std::path::PathBuf;

use thiserror::Error;

use crate::agent::ParseFailure;
use crate::marketplace::StructuredSource;

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

    /// The plugin manifest could not be parsed. Carries `path` so error
    /// rendering names the offending file — without it, bulk listings
    /// over many plugins reduce to "invalid plugin manifest: missing name"
    /// with no way to tell which plugin is broken.
    #[error("invalid plugin manifest at {path}: {reason}")]
    InvalidManifest { path: PathBuf, reason: String },

    /// No `plugin.json` exists at the expected location.
    #[error("plugin manifest not found at {path}")]
    ManifestNotFound { path: PathBuf },

    /// The plugin declares no skills. Carries `path` for parity with
    /// [`Self::InvalidManifest`] and [`Self::ManifestReadFailed`] so
    /// bulk-listing callers that surface the error alongside a plugin
    /// name can also point at the plugin directory in the message.
    #[error("plugin `{name}` at {path} has no skills")]
    NoSkills { name: String, path: PathBuf },

    /// The plugin directory referenced by a `RelativePath` source does not
    /// exist on disk. Typically means the marketplace manifest points at a
    /// directory that was never committed or was deleted.
    #[error("plugin directory does not exist: {path}")]
    DirectoryMissing { path: PathBuf },

    /// A plugin path exists on disk but is not a directory — e.g. a regular
    /// file sitting at the expected location. Distinct from
    /// [`Self::DirectoryMissing`] (path doesn't exist) and
    /// [`Self::DirectoryUnreadable`] (stat itself failed) so callers can
    /// branch on the semantic rather than substring-matching a reason
    /// string. Classified as plugin-level so bulk listings skip this
    /// plugin rather than aborting the whole listing.
    #[error("plugin path exists but is not a directory: {path}")]
    NotADirectory { path: PathBuf },

    /// A plugin path is a symbolic link. Following it could escape the
    /// marketplace tree or point at arbitrary host files inside an
    /// untrusted cloned repository, so it is refused rather than
    /// traversed. Distinct from [`Self::NotADirectory`] on semantic
    /// grounds (security refusal vs shape mismatch) — both currently
    /// map to `ErrorType::Validation` at the Tauri boundary, but the
    /// split lets security-audit logs and future UI surfaces
    /// distinguish the two. Classified as plugin-level.
    #[error("refusing to follow symlinked plugin path: {path}")]
    SymlinkRefused { path: PathBuf },

    /// The plugin directory exists but stat'ing it failed (permission
    /// denied, I/O error, etc.). Distinct from [`Self::DirectoryMissing`]
    /// (path doesn't exist), [`Self::NotADirectory`] (path exists but is
    /// not a directory), and [`Self::SymlinkRefused`] (security refusal).
    /// The underlying [`io::Error`] is carried via `#[source]`, so
    /// [`io::ErrorKind`] is preserved and `error_full_chain` surfaces it
    /// in terminal output. Classified as plugin-level.
    #[error("could not access plugin directory at {path}")]
    DirectoryUnreadable {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    /// The plugin manifest file exists but could not be read (permission
    /// denied, transient I/O, stat failure, etc.). Distinct from
    /// [`Self::InvalidManifest`] (file exists but can't be parsed) and
    /// [`Self::ManifestNotFound`] (file doesn't exist at all). Carries
    /// the underlying [`io::Error`] via `#[source]` so [`io::ErrorKind`]
    /// is preserved for callers that want to branch on the failure mode.
    /// Classified as plugin-level so bulk listings skip this plugin
    /// rather than aborting on a single bad `plugin.json`.
    #[error("could not read plugin manifest at {path}")]
    ManifestReadFailed {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    /// A caller asked for a local filesystem path to a plugin whose source
    /// is remote (GitHub / Git URL / Git subdir). Resolving it would
    /// require a clone, which the caller explicitly did not request.
    /// Distinct from [`Self::DirectoryMissing`] so the UI can offer the
    /// right remediation ("clone this remote plugin" vs "the local copy
    /// is broken").
    ///
    /// `plugin_source` carries the [`StructuredSource`] so callers that
    /// surface this error to a user can render provider-specific
    /// remediation (e.g. a GitHub-clone hint vs a plain git-URL hint)
    /// without having to refetch the plugin entry. The field is named
    /// `plugin_source` rather than `source` because thiserror treats a
    /// field literally named `source` as the `std::error::Error::source()`
    /// implementation and requires its type to implement `Error` — which
    /// [`StructuredSource`] deliberately does not. The wire-format
    /// projection in [`crate::service::browse::SkippedReason::RemoteSourceNotLocal`]
    /// uses the natural name `source` on the frontend-facing side.
    ///
    /// The Display intentionally does NOT embed remediation text —
    /// remediation is surface-dependent (CLI vs UI) and is exposed via
    /// [`PluginError::remediation_hint`] so each frontend picks its own
    /// phrasing.
    #[error("plugin `{plugin}` uses a remote source and is not available locally")]
    RemoteSourceNotLocal {
        plugin: String,
        plugin_source: StructuredSource,
    },
}

/// Which user-facing surface is rendering an error. Used by
/// [`PluginError::remediation_hint`] to pick surface-appropriate
/// remediation text — CLI hints reference CLI commands, UI hints
/// reference UI affordances. Kept in core so error variants and their
/// remediation stay colocated; consumer crates pick the variant that
/// matches where the error is about to be shown.
///
/// Closed two-variant semantic (no `#[non_exhaustive]`): these are
/// every rendering surface in this workspace. If a new surface ever
/// lands (e.g. a web UI), adding a variant is an intentional breaking
/// change that should force every `match Surface { ... }` to decide
/// what the new surface's remediation text is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Surface {
    /// Terminal / CLI output. Remediation may reference CLI flags or
    /// commands.
    Cli,
    /// Tauri desktop UI. Remediation may reference UI navigation or
    /// buttons.
    Ui,
}

impl PluginError {
    /// A surface-appropriate remediation hint for this error, or `None`
    /// if the variant is self-explanatory / no actionable next step
    /// exists at this layer.
    ///
    /// The hint is deliberately NOT embedded in [`Display`] — a CLI
    /// sentence ("use `kiro-market add` to clone it") renders as
    /// misleading noise in the Tauri UI (which has no CLI), and a UI
    /// sentence ("open the marketplace detail page") renders as
    /// misleading noise in the CLI. Returning `Option<String>` also
    /// lets callers decide how to compose the hint with the error
    /// message (a trailing paragraph in the CLI, an inline badge in the
    /// UI, etc.) rather than baking the composition into `Display`.
    #[must_use]
    pub fn remediation_hint(&self, surface: Surface) -> Option<String> {
        // Every variant is enumerated explicitly (no `_ => None`) so a
        // new `PluginError` variant that *should* have a remediation
        // forces a compile error here rather than silently defaulting
        // to `None`. This mirrors the sibling classifier
        // [`crate::service::browse::SkippedReason::from_plugin_error`],
        // which enumerates for the same reason — the two classifications
        // cannot drift.
        match self {
            Self::RemoteSourceNotLocal { plugin, .. } => Some(match surface {
                // `kiro-market install` uses the cloning resolver
                // (`resolve_plugin_dir`), whereas `list`/`info`/`search`
                // use `resolve_local_plugin_dir` — the non-cloning
                // variant that produces this error. Installing is the
                // user-facing remediation that triggers the clone.
                Surface::Cli => {
                    format!("run `kiro-market install {plugin}@<marketplace>` to clone it locally")
                }
                Surface::Ui => {
                    "open the plugin's detail page in the marketplace to clone it".to_owned()
                }
            }),
            Self::NotFound { .. }
            | Self::InvalidManifest { .. }
            | Self::ManifestNotFound { .. }
            | Self::NoSkills { .. }
            | Self::DirectoryMissing { .. }
            | Self::NotADirectory { .. }
            | Self::SymlinkRefused { .. }
            | Self::DirectoryUnreadable { .. }
            | Self::ManifestReadFailed { .. } => None,
        }
    }
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

    // -----------------------------------------------------------------
    // Native-import parsing failures.
    // Mirror translated-path `ParseFailed` but with structured payloads
    // matching the JSON parse pipeline (no frontmatter / YAML stages).
    // -----------------------------------------------------------------
    /// Native agent JSON file failed to parse. `reason` carries the full
    /// `serde_json` error chain materialized at the adapter boundary
    /// (`service::native_parse_failure_to_agent_error`) — the source
    /// type does not leak through the public API.
    #[error("native agent JSON `{path}` failed to parse: {reason}")]
    NativeManifestParseFailed { path: PathBuf, reason: String },

    /// Native agent JSON parsed but is missing the required `name` field.
    #[error("native agent at `{path}` is missing the required `name` field")]
    NativeManifestMissingName { path: PathBuf },

    /// Native agent JSON has a `name` that failed validation
    /// (path-unsafe characters, empty, etc.).
    #[error("native agent at `{path}` has an invalid `name`: {reason}")]
    NativeManifestInvalidName { path: PathBuf, reason: String },

    /// Native agent JSON manifest read failed (permission denied,
    /// transient I/O, etc.). Parallels [`PluginError::ManifestReadFailed`].
    #[error("could not read native agent manifest at {path}")]
    ManifestReadFailed {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    // -----------------------------------------------------------------
    // Cross-plugin / collision errors.
    // -----------------------------------------------------------------
    /// Native agent install would clobber an agent owned by another plugin.
    /// Without `--force`, ownership is preserved by the existing owner.
    #[error(
        "native agent name `{name}` would clobber an agent owned by plugin \
         `{owner}`; pass --force to transfer ownership"
    )]
    NameClashWithOtherPlugin { name: String, owner: String },

    /// Companion file path is owned by another plugin's bundle.
    /// Without `--force`, the existing owner keeps the path.
    #[error("path `{path}` is owned by plugin `{owner}`; pass --force to transfer")]
    PathOwnedByOtherPlugin { path: PathBuf, owner: String },

    /// File exists at the destination but no plugin has tracking for it
    /// (orphan from a manual install or a previous uninstall that left
    /// stray files). Without `--force`, refused to avoid silent overwrite.
    #[error(
        "file exists at `{path}` but has no tracking entry; \
         remove it manually or pass --force"
    )]
    OrphanFileAtDestination { path: PathBuf },

    /// Same plugin reinstalled the same agent / companion bundle, but the
    /// source content has changed since the last install. Without
    /// `--force`, the install is refused so the user explicitly opts in
    /// to overwriting prior content.
    #[error(
        "agent `{name}` content has changed since last install; \
         pass --force to overwrite"
    )]
    ContentChangedRequiresForce { name: String },

    /// A native plugin declares multiple agent scan roots. v1 supports a
    /// single scan root only — companion ownership tracking would otherwise
    /// have to disambiguate which scan root each companion belongs to,
    /// expanding the tracking schema. Out of scope for v1.
    #[error("native plugin spans multiple agent scan roots; v1 supports a single scan root only")]
    MultipleScanRootsNotSupported { roots: Vec<PathBuf> },

    /// A native companion source file is a hardlink (Unix `nlink > 1`).
    /// The other path(s) sharing the inode could be sensitive host
    /// files; the install refuses rather than `fs::copy` inode contents
    /// into `.kiro/agents/`. Mirrors
    /// [`crate::agent::parse_native::NativeParseFailure::HardlinkRefused`]
    /// (the canonical statement of the threat model) and
    /// `SteeringError::SourceHardlinked`.
    #[error("refusing hardlinked native companion at `{path}` (nlink={nlink})")]
    SourceHardlinked { path: PathBuf, nlink: u64 },

    // -----------------------------------------------------------------
    // Catch-all for non-AgentError infrastructure failures that surface
    // through the install pipeline.
    // -----------------------------------------------------------------
    /// An infrastructure error (I/O, hash, JSON) bubbled up through an
    /// install attempt. Used at the boundary where a top-level [`Error`]
    /// must be wrapped into a per-agent failure entry without losing the
    /// underlying source chain.
    #[error("install failed at `{path}`")]
    InstallFailed {
        path: PathBuf,
        #[source]
        source: Box<crate::error::Error>,
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
    Steering(#[from] crate::steering::SteeringError),

    #[error(transparent)]
    Git(#[from] GitError),

    #[error(transparent)]
    Validation(#[from] ValidationError),

    #[error(transparent)]
    Hash(#[from] crate::hash::HashError),

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
        PluginError::InvalidManifest {
            path: PathBuf::from("/tmp/plugin.json"),
            reason: "missing name".into(),
        },
        "invalid plugin manifest at /tmp/plugin.json: missing name"
    )]
    #[case::plugin_manifest_not_found(
        PluginError::ManifestNotFound { path: PathBuf::from("/tmp/plugin.json") },
        "plugin manifest not found at /tmp/plugin.json"
    )]
    #[case::plugin_no_skills(
        PluginError::NoSkills {
            name: "empty".into(),
            path: PathBuf::from("/tmp/plugins/empty"),
        },
        "plugin `empty` at /tmp/plugins/empty has no skills"
    )]
    #[case::plugin_directory_missing(
        PluginError::DirectoryMissing { path: PathBuf::from("/tmp/plugins/x") },
        "plugin directory does not exist: /tmp/plugins/x"
    )]
    #[case::plugin_not_a_directory(
        PluginError::NotADirectory { path: PathBuf::from("/tmp/plugins/x") },
        "plugin path exists but is not a directory: /tmp/plugins/x"
    )]
    #[case::plugin_symlink_refused(
        PluginError::SymlinkRefused { path: PathBuf::from("/tmp/plugins/escape") },
        "refusing to follow symlinked plugin path: /tmp/plugins/escape"
    )]
    #[case::plugin_directory_unreadable(
        PluginError::DirectoryUnreadable {
            path: PathBuf::from("/tmp/plugins/x"),
            source: io::Error::from(io::ErrorKind::PermissionDenied),
        },
        "could not access plugin directory at /tmp/plugins/x"
    )]
    #[case::plugin_manifest_read_failed(
        PluginError::ManifestReadFailed {
            path: PathBuf::from("/tmp/plugins/x/plugin.json"),
            source: io::Error::from(io::ErrorKind::PermissionDenied),
        },
        "could not read plugin manifest at /tmp/plugins/x/plugin.json"
    )]
    #[case::plugin_remote_source_not_local(
        PluginError::RemoteSourceNotLocal {
            plugin: "acme".into(),
            plugin_source: StructuredSource::GitHub {
                repo: "owner/repo".into(),
                git_ref: None,
                sha: None,
            },
        },
        "plugin `acme` uses a remote source and is not available locally"
    )]
    fn plugin_error_display(#[case] err: PluginError, #[case] expected: &str) {
        assert_eq!(err.to_string(), expected);
    }

    /// Regression guard: the Display of `RemoteSourceNotLocal` must NOT
    /// embed the "use the CLI to clone it first" remediation any longer.
    /// That sentence renders as misleading noise in the Tauri UI (which
    /// has no CLI). Surface-appropriate remediation now lives on
    /// [`PluginError::remediation_hint`]. If this test fails, somebody
    /// put the CLI hint back in the format string — break out
    /// `remediation_hint` instead.
    #[test]
    fn remote_source_not_local_display_has_no_cli_hint() {
        let err = PluginError::RemoteSourceNotLocal {
            plugin: "acme".into(),
            plugin_source: StructuredSource::GitHub {
                repo: "owner/repo".into(),
                git_ref: None,
                sha: None,
            },
        };
        let display = err.to_string();
        assert!(
            !display.to_lowercase().contains("cli"),
            "Display must be surface-neutral, got: {display}"
        );
        assert!(
            !display.to_lowercase().contains("clone"),
            "remediation verbs belong in remediation_hint, not Display: {display}"
        );
    }

    #[test]
    fn remediation_hint_remote_source_distinguishes_surfaces() {
        let err = PluginError::RemoteSourceNotLocal {
            plugin: "acme".into(),
            plugin_source: StructuredSource::GitHub {
                repo: "owner/repo".into(),
                git_ref: None,
                sha: None,
            },
        };
        let cli = err
            .remediation_hint(Surface::Cli)
            .expect("CLI hint must be present for RemoteSourceNotLocal");
        let ui = err
            .remediation_hint(Surface::Ui)
            .expect("UI hint must be present for RemoteSourceNotLocal");
        assert_ne!(cli, ui, "CLI and UI hints must differ");
        // Each hint must only reference its own surface's vocabulary —
        // swapping them would be the whole point of bug.
        assert!(
            cli.to_lowercase().contains("cli") || cli.to_lowercase().contains("kiro-market"),
            "CLI hint should reference CLI vocabulary, got: {cli}"
        );
        // Pin the specific CLI subcommand. `kiro-market install` is the
        // remediation because it uses the cloning resolver; previously
        // this hint said `kiro-market add` which does not exist as a
        // subcommand (flagged by gemini-code-assist on PR #35). The
        // `install` substring catches a regression back to any
        // non-existent command.
        assert!(
            cli.contains("kiro-market install"),
            "CLI hint must reference `kiro-market install` (the cloning \
             remediation), got: {cli}"
        );
        // The hint must interpolate the plugin name so the user can
        // copy-paste it verbatim. "acme" is the plugin name seeded in
        // the fixture above.
        assert!(
            cli.contains("acme"),
            "CLI hint must interpolate the plugin name from the error \
             payload, got: {cli}"
        );
        assert!(
            !ui.to_lowercase().contains("cli") && !ui.contains("kiro-market"),
            "UI hint must not reference CLI commands, got: {ui}"
        );
    }

    /// Remediation hints are only defined for variants where the caller
    /// can take an actionable next step. Every other plugin-level variant
    /// returns `None` so consumers don't render empty trailing hints.
    #[rstest]
    #[case::not_found(PluginError::NotFound {
        plugin: "p".into(),
        marketplace: "m".into(),
    })]
    #[case::invalid_manifest(PluginError::InvalidManifest {
        path: PathBuf::from("/tmp/plugin.json"),
        reason: "bad json".into(),
    })]
    #[case::directory_missing(PluginError::DirectoryMissing {
        path: PathBuf::from("/tmp/plugins/ghost"),
    })]
    #[case::not_a_directory(PluginError::NotADirectory {
        path: PathBuf::from("/tmp/plugins/file"),
    })]
    #[case::symlink_refused(PluginError::SymlinkRefused {
        path: PathBuf::from("/tmp/plugins/link"),
    })]
    #[case::no_skills(PluginError::NoSkills {
        name: "empty".into(),
        path: PathBuf::from("/tmp/plugins/empty"),
    })]
    fn remediation_hint_returns_none_for_variants_without_actionable_step(
        #[case] err: PluginError,
    ) {
        assert!(
            err.remediation_hint(Surface::Cli).is_none(),
            "CLI hint must be None for variant without remediation"
        );
        assert!(
            err.remediation_hint(Surface::Ui).is_none(),
            "UI hint must be None for variant without remediation"
        );
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
    #[case::native_manifest_parse_failed(
        AgentError::NativeManifestParseFailed {
            path: PathBuf::from("rev.json"),
            reason: "expected `,` or `}` at line 3 column 1".into(),
        },
        "native agent JSON `rev.json` failed to parse: expected `,` or `}` at line 3 column 1"
    )]
    fn agent_error_display(#[case] err: AgentError, #[case] expected: &str) {
        assert_eq!(err.to_string(), expected);
    }

    /// Locks the wire-format contract that
    /// [`AgentError::NativeManifestParseFailed`] does not expose a
    /// `source()` chain. The `reason: String` field is the only carrier
    /// of materialized `serde_json` detail; re-introducing
    /// `#[source] serde_json::Error` would silently break this assertion
    /// AND would be caught at lint time by
    /// `cargo xtask plan-lint --gate gate-4-external-error-boundary`.
    ///
    /// Replaces the deleted `native_manifest_parse_failed_renders_path_and_reason`
    /// test that exercised the now-removed
    /// `error::native_manifest_parse_failed` constructor — the contract
    /// still applies even though the constructor is gone, since
    /// `service::native_parse_failure_to_agent_error` now produces this
    /// variant directly from `NativeParseFailure::InvalidJson { reason }`.
    #[test]
    fn native_manifest_parse_failed_exposes_no_source_chain() {
        use std::error::Error as _;
        let err = AgentError::NativeManifestParseFailed {
            path: PathBuf::from("rev.json"),
            reason: "stub".into(),
        };
        assert!(
            err.source().is_none(),
            "NativeManifestParseFailed must not expose a source chain — \
             reason: String is the only carrier of materialized serde_json detail"
        );
    }

    #[test]
    fn name_clash_with_other_plugin_renders_useful_message() {
        let err = AgentError::NameClashWithOtherPlugin {
            name: "code-reviewer".into(),
            owner: "other-plugin".into(),
        };
        let msg = err.to_string();
        assert!(msg.contains("code-reviewer"));
        assert!(msg.contains("other-plugin"));
        assert!(msg.contains("--force"));
    }

    #[test]
    fn content_changed_requires_force_renders_useful_message() {
        let err = AgentError::ContentChangedRequiresForce { name: "x".into() };
        let msg = err.to_string();
        assert!(msg.contains('x'));
        assert!(msg.contains("--force"));
    }

    #[test]
    fn path_owned_by_other_plugin_renders_useful_message() {
        let err = AgentError::PathOwnedByOtherPlugin {
            path: PathBuf::from("prompts/shared.md"),
            owner: "plugin-a".into(),
        };
        let msg = err.to_string();
        assert!(msg.contains("prompts/shared.md"));
        assert!(msg.contains("plugin-a"));
        assert!(msg.contains("--force"));
    }

    #[test]
    fn orphan_file_at_destination_mentions_force() {
        let err = AgentError::OrphanFileAtDestination {
            path: PathBuf::from(".kiro/agents/rev.json"),
        };
        let msg = err.to_string();
        assert!(msg.contains(".kiro/agents/rev.json"));
        assert!(msg.contains("--force"));
    }

    #[test]
    fn install_failed_carries_source_chain() {
        use std::error::Error as _;
        let inner = io::Error::from(io::ErrorKind::PermissionDenied);
        let err = AgentError::InstallFailed {
            path: PathBuf::from(".kiro/agents/x.json"),
            source: Box::new(crate::error::Error::Io(inner)),
        };
        let display = err.to_string();
        assert!(display.contains(".kiro/agents/x.json"));
        let source = err.source().expect("source chain populated");
        assert!(source.to_string().to_lowercase().contains("permission"));
    }

    #[test]
    fn multiple_scan_roots_not_supported_renders() {
        let err = AgentError::MultipleScanRootsNotSupported {
            roots: vec![PathBuf::from("./agents/"), PathBuf::from("./extra/")],
        };
        assert!(err.to_string().contains("multiple agent scan roots"));
    }

    #[test]
    fn manifest_read_failed_carries_io_source() {
        use std::error::Error as _;
        let err = AgentError::ManifestReadFailed {
            path: PathBuf::from("rev.json"),
            source: io::Error::from(io::ErrorKind::PermissionDenied),
        };
        assert!(err.to_string().contains("rev.json"));
        assert!(err.source().is_some());
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
            path: PathBuf::from("/tmp/plugins/test"),
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

    /// Regression guard: `#[source] source: io::Error` on `DirectoryUnreadable`
    /// and `ManifestReadFailed` must survive wrapping through
    /// `Error::Plugin(#[error(transparent)])` so `error_full_chain` at the
    /// Tauri boundary renders the variant Display AND the underlying
    /// `io::Error`. The Round 2 refactor shortened these variants' Display
    /// to drop the reason suffix on the premise that the source chain
    /// carries it — this test pins that contract. If `#[source]` is dropped
    /// or `Error::Plugin` loses `#[error(transparent)]`, this fails before
    /// the Tauri layer does.
    #[rstest]
    #[case::directory_unreadable(
        PluginError::DirectoryUnreadable {
            path: PathBuf::from("/tmp/plugins/x"),
            source: io::Error::from(io::ErrorKind::PermissionDenied),
        },
        "could not access plugin directory at /tmp/plugins/x"
    )]
    #[case::manifest_read_failed(
        PluginError::ManifestReadFailed {
            path: PathBuf::from("/tmp/plugins/x/plugin.json"),
            source: io::Error::from(io::ErrorKind::PermissionDenied),
        },
        "could not read plugin manifest at /tmp/plugins/x/plugin.json"
    )]
    fn plugin_error_full_chain_preserves_io_source_through_wrapping(
        #[case] variant: PluginError,
        #[case] expected_display: &str,
    ) {
        let err: Error = variant.into();
        let chain = error_full_chain(&err);
        assert!(
            chain.contains(expected_display),
            "chain missing variant Display: {chain}"
        );
        assert!(
            chain.contains("permission denied"),
            "chain missing io::Error source: {chain}"
        );
        assert!(
            chain.contains(&format!("{expected_display}: permission denied")),
            "chain must have Display + source joined by `: `: {chain}"
        );
    }
}

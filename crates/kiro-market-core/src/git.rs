//! Git operations for cloning and updating marketplace repositories.
//!
//! Cloning tries `gix` first (fast, in-process) and falls back to the
//! system `git` CLI when `gix` fails — for example when `curl-sys` was
//! compiled without TLS support, or on corporate networks where the
//! system git has proxy/certificate configuration that `gix` cannot
//! access. Pull and checkout always use the system `git` CLI.
//! Errors are mapped into domain-specific [`GitError`] variants.

use std::num::NonZeroU32;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::AtomicBool;

use gix::progress::Discard;
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

use crate::error::{GitError, error_source_chain};

// ---------------------------------------------------------------------------
// Git backend trait
// ---------------------------------------------------------------------------

/// A validated git reference (branch, tag, or commit SHA).
///
/// Constructing a `GitRef` runs the validation that previously lived
/// inside `checkout_ref_if_needed`, so by the time the value reaches the
/// git backend it is already proven safe to feed to `git checkout`.
/// Parse-don't-validate: an instance in hand is proof that the ref does
/// not start with `-` (which would let an attacker smuggle a `git`
/// option through the checkout argument list) and is non-empty.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct GitRef(String);

impl GitRef {
    /// Construct a `GitRef`, rejecting empty values and refs that start
    /// with `-` (potential argument injection vector).
    ///
    /// # Errors
    ///
    /// Returns [`crate::error::ValidationError::InvalidName`] if the input
    /// is empty or begins with `-`.
    pub fn new(value: impl Into<String>) -> Result<Self, crate::error::ValidationError> {
        let value = value.into();
        if value.is_empty() {
            return Err(crate::error::ValidationError::InvalidName {
                name: value,
                reason: "git ref must not be empty".into(),
            });
        }
        if value.starts_with('-') {
            return Err(crate::error::ValidationError::InvalidName {
                name: value,
                reason: "git ref must not start with '-' (would be parsed as a git flag)".into(),
            });
        }
        Ok(Self(value))
    }

    /// The underlying ref string, ready to be passed to `git checkout`.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for GitRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<&str> for GitRef {
    type Error = crate::error::ValidationError;
    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl TryFrom<String> for GitRef {
    type Error = crate::error::ValidationError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

/// Options for cloning a repository.
///
/// When `git_ref` is `None`, the implementation should use a shallow clone
/// (depth 1) to reduce transfer size. When `git_ref` is `Some`, a full
/// clone is performed followed by a checkout of the specified ref.
#[derive(Clone, Debug, Default)]
pub struct CloneOptions {
    /// Branch, tag, or SHA to check out after cloning. Validated at
    /// construction via [`GitRef::new`].
    pub git_ref: Option<GitRef>,
}

/// Trait abstracting git operations for testability and backend swapping.
///
/// Implementations must be `Send + Sync` to support sharing across async
/// Tauri command handlers via `Arc` or `Box`.
pub trait GitBackend: Send + Sync {
    /// Clone a remote repository into `dest`.
    ///
    /// # Errors
    ///
    /// Returns [`GitError::CloneFailed`] if the clone or checkout fails.
    fn clone_repo(&self, url: &str, dest: &Path, opts: &CloneOptions) -> Result<(), GitError>;

    /// Pull (fast-forward only) the default branch.
    ///
    /// # Errors
    ///
    /// Returns [`GitError::OpenFailed`] if the path is not a valid repository,
    /// or [`GitError::PullFailed`] if the pull fails.
    fn pull_repo(&self, path: &Path) -> Result<(), GitError>;

    /// Verify the HEAD commit matches the expected SHA prefix.
    ///
    /// # Errors
    ///
    /// Returns [`GitError::ShaMismatch`] if the SHA does not match.
    /// Returns [`GitError::OpenFailed`] if the repository cannot be read.
    fn verify_sha(&self, path: &Path, expected_sha: &str) -> Result<(), GitError>;
}

// ---------------------------------------------------------------------------
// Gix + CLI backend
// ---------------------------------------------------------------------------

/// Git backend using `gix` for clone/open and the system `git` CLI for
/// pull and ref checkout.
///
/// SSH connect-timeout protection is applied when no custom `GIT_SSH_COMMAND`
/// or `GIT_SSH` is configured. `GIT_TERMINAL_PROMPT=0` prevents interactive
/// prompts from hanging non-interactive contexts.
#[derive(Debug)]
pub struct GixCliBackend {
    ssh_connect_timeout: u32,
}

impl Default for GixCliBackend {
    fn default() -> Self {
        Self {
            ssh_connect_timeout: SSH_CONNECT_TIMEOUT_SECS,
        }
    }
}

/// Default SSH connect timeout (seconds), applied via `GIT_SSH_COMMAND`
/// **only when** neither `GIT_SSH_COMMAND` nor `GIT_SSH` is already set
/// in the environment. Prevents indefinite hangs when SSH port 22 is
/// firewalled, while leaving custom SSH wrappers (e.g. plink, jump-host
/// configs) untouched.
const SSH_CONNECT_TIMEOUT_SECS: u32 = 30;

impl GixCliBackend {
    /// Run a `git` command with SSH connect-timeout protection.
    ///
    /// Sets `GIT_SSH_COMMAND` with a configurable `ConnectTimeout` to prevent
    /// indefinite hangs when SSH port 22 is firewalled. Detects a missing
    /// `git` binary and returns [`GitError::GitNotFound`].
    fn run_git(&self, args: &[&str], dir: &Path) -> Result<std::process::Output, GitError> {
        let mut cmd = Command::new("git");
        cmd.args(args)
            .current_dir(dir)
            .env("GIT_TERMINAL_PROMPT", "0");

        // Only set SSH timeout when no custom SSH configuration exists.
        // GIT_SSH_COMMAND takes precedence over GIT_SSH in git's resolution;
        // setting it when GIT_SSH points to plink would silently override it.
        if std::env::var_os("GIT_SSH_COMMAND").is_none() && std::env::var_os("GIT_SSH").is_none() {
            cmd.env(
                "GIT_SSH_COMMAND",
                format!("ssh -o ConnectTimeout={}", self.ssh_connect_timeout),
            );
        }

        cmd.output().map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => GitError::GitNotFound,
            _ => GitError::GitCommandFailed {
                dir: dir.to_path_buf(),
                source: Box::new(e),
            },
        })
    }

    /// Clone using the `gix` library (fast, in-process, no subprocess).
    fn clone_with_gix(&self, url: &str, dest: &Path, opts: &CloneOptions) -> Result<(), GitError> {
        let mut prepare = gix::prepare_clone(url, dest).map_err(|e| clone_failed(url, e))?;

        if opts.git_ref.is_none() {
            // Shallow clone, depth = 1. NonZeroU32::new(1).unwrap() reads
            // more clearly here than `NonZeroU32::MIN`, which would force
            // the reader to remember that NonZeroU32's minimum is 1.
            let depth = NonZeroU32::new(1).expect("1 is non-zero");
            prepare = prepare.with_shallow(gix::remote::fetch::Shallow::DepthAtRemote(depth));
        }

        let (mut checkout, _outcome) = prepare
            .fetch_then_checkout(Discard, &AtomicBool::new(false))
            .map_err(|e| clone_failed(url, e))?;

        let (_repo, _outcome) = checkout
            .main_worktree(Discard, &AtomicBool::new(false))
            .map_err(|e| clone_failed(url, e))?;

        // Checkout-after-clone may fail; clean up the populated dest so a
        // retry is not blocked by "destination path already exists".
        if let Err(e) = self.checkout_ref_if_needed(url, dest, opts) {
            cleanup_failed_clone_dest(dest);
            return Err(e);
        }

        Ok(())
    }

    /// Clone using the system `git` CLI.
    ///
    /// Falls back to this when `gix` fails (e.g. missing TLS backend on
    /// Windows, corporate proxy issues, or unsupported transport).
    fn clone_with_cli(&self, url: &str, dest: &Path, opts: &CloneOptions) -> Result<(), GitError> {
        let mut args = vec!["clone"];
        if opts.git_ref.is_none() {
            args.extend(["--depth", "1"]);
        }
        args.push(url);
        let dest_str = dest.to_string_lossy();
        args.push(&dest_str);

        debug!(url, dest = %dest.display(), "cloning via system git CLI");

        // run_git's current_dir must already exist.
        let work_dir = dest.parent().ok_or_else(|| {
            clone_failed(
                url,
                format!(
                    "destination path '{}' has no parent directory",
                    dest.display()
                ),
            )
        })?;
        let output = self
            .run_git(&args, work_dir)
            .map_err(|e| clone_failed(url, e))?;

        if !output.status.success() {
            let detail = git_error_detail(&output);
            // `git clone` may have created a partial dest before failing.
            cleanup_failed_clone_dest(dest);
            return Err(clone_failed(url, detail));
        }

        // Checkout failure leaves the cloned tree in `dest`; remove it so a
        // retry is not blocked by "destination path already exists".
        if let Err(e) = self.checkout_ref_if_needed(url, dest, opts) {
            cleanup_failed_clone_dest(dest);
            return Err(e);
        }

        Ok(())
    }

    /// Check out a specific git ref if one was requested. The ref's
    /// shape was validated at [`GitRef`] construction, so this only has
    /// to handle the subprocess call.
    fn checkout_ref_if_needed(
        &self,
        url: &str,
        dest: &Path,
        opts: &CloneOptions,
    ) -> Result<(), GitError> {
        let Some(refname) = opts.git_ref.as_ref() else {
            return Ok(());
        };

        let output = self
            .run_git(&["checkout", refname.as_str()], dest)
            .map_err(|e| clone_failed(url, e))?;

        if !output.status.success() {
            let detail = git_error_detail(&output);
            return Err(clone_failed(url, detail));
        }

        Ok(())
    }
}

/// Construct a [`GitError::CloneFailed`] from a URL and an error source.
///
/// Centralises the repeated `map_err` closure that appeared in
/// `clone_with_gix`, `clone_with_cli`, and `checkout_ref_if_needed`.
fn clone_failed(url: &str, e: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> GitError {
    GitError::CloneFailed {
        url: url.to_owned(),
        source: e.into(),
    }
}

/// Combine a gix failure and a system-git failure into a single
/// [`GitError::CloneFailed`] preserving both root causes. Uses
/// [`error_source_chain`] (not `to_string`) on the inner errors so the
/// URL appears only once in the final rendered message.
fn combine_clone_errors(url: &str, gix_err: &GitError, cli_err: &GitError) -> GitError {
    let gix_detail = error_source_chain(gix_err);
    let cli_detail = error_source_chain(cli_err);
    clone_failed(url, format!("gix: {gix_detail}; system git: {cli_detail}"))
}

/// Combine a gix failure with a cleanup failure into a single
/// [`GitError::CloneFailed`]. Used when gix left a partial clone behind
/// and the cleanup itself errored, blocking the CLI fallback.
fn combine_clone_and_cleanup_errors(
    url: &str,
    gix_err: &GitError,
    cleanup_err: &std::io::Error,
) -> GitError {
    clone_failed(
        url,
        format!(
            "gix: {}; cleanup failed: {cleanup_err}",
            error_source_chain(gix_err)
        ),
    )
}

/// Best-effort removal of a clone destination after a clone or checkout
/// failure. A leftover dest dir will fail any future `git clone` with
/// "destination path already exists" — wiping it lets the caller retry.
fn cleanup_failed_clone_dest(dest: &Path) {
    match std::fs::remove_dir_all(dest) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => {
            warn!(
                path = %dest.display(),
                error = %e,
                "failed to clean up clone destination after failure"
            );
        }
    }
}

/// Extract a useful error message from a failed git command.
///
/// Prefers stderr, falls back to stdout, and ultimately includes the exit
/// code if both are empty.
fn git_error_detail(output: &std::process::Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !stderr.trim().is_empty() {
        return stderr.trim().to_owned();
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    if !stdout.trim().is_empty() {
        return stdout.trim().to_owned();
    }
    format!("git exited with {}", output.status)
}

impl GitBackend for GixCliBackend {
    fn clone_repo(&self, url: &str, dest: &Path, opts: &CloneOptions) -> Result<(), GitError> {
        debug!(
            url,
            dest = %dest.display(),
            git_ref = opts.git_ref.as_ref().map(GitRef::as_str),
            "cloning repository"
        );

        match self.clone_with_gix(url, dest, opts) {
            Ok(()) => Ok(()),
            Err(gix_err) => {
                warn!(
                    url,
                    error = %gix_err,
                    "gix clone failed, falling back to system git CLI"
                );
                // Clean up any partial gix clone before retrying.
                // If cleanup fails, the CLI clone will fail on a non-empty
                // directory, so we must bail out with both errors.
                if dest.exists()
                    && let Err(cleanup_err) = std::fs::remove_dir_all(dest)
                {
                    warn!(
                        path = %dest.display(),
                        error = %cleanup_err,
                        "failed to clean up partial gix clone"
                    );
                    return Err(combine_clone_and_cleanup_errors(
                        url,
                        &gix_err,
                        &cleanup_err,
                    ));
                }
                self.clone_with_cli(url, dest, opts)
                    .map_err(|cli_err| combine_clone_errors(url, &gix_err, &cli_err))
            }
        }
    }

    fn pull_repo(&self, path: &Path) -> Result<(), GitError> {
        debug!(path = %path.display(), "pulling repository");

        // Verify it's actually a git repo first (preserves the OpenFailed error).
        let _repo = gix::open(path).map_err(|e| GitError::OpenFailed {
            path: path.to_path_buf(),
            source: Box::new(e),
        })?;

        let output =
            self.run_git(&["pull", "--ff-only"], path)
                .map_err(|e| GitError::PullFailed {
                    path: path.to_path_buf(),
                    source: Box::new(e),
                })?;

        if !output.status.success() {
            let detail = git_error_detail(&output);
            return Err(GitError::PullFailed {
                path: path.to_path_buf(),
                source: detail.into(),
            });
        }

        Ok(())
    }

    fn verify_sha(&self, path: &Path, expected_sha: &str) -> Result<(), GitError> {
        if expected_sha.is_empty() {
            return Err(GitError::ShaMismatch {
                expected: "(empty)".to_owned(),
                actual: "(not checked)".to_owned(),
            });
        }

        let repo = gix::open(path).map_err(|e| GitError::OpenFailed {
            path: path.to_path_buf(),
            source: Box::new(e),
        })?;

        let head_id = repo.head_id().map_err(|e| GitError::OpenFailed {
            path: path.to_path_buf(),
            source: Box::new(e),
        })?;

        let actual_sha = head_id.to_string();

        if actual_sha.starts_with(expected_sha) {
            Ok(())
        } else {
            Err(GitError::ShaMismatch {
                expected: expected_sha.to_owned(),
                actual: actual_sha,
            })
        }
    }
}

/// Which transport protocol to use when cloning from a shorthand host
/// reference (e.g. `owner/repo`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "cli", derive(clap::ValueEnum))]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "lowercase")]
pub enum GitProtocol {
    /// Clone via HTTPS (works through firewalls, uses credential helpers).
    #[default]
    Https,
    /// Clone via SSH (uses SSH agent / keys).
    Ssh,
}

/// Convert a GitHub `owner/repo` shorthand into a clone URL using the
/// specified protocol.
#[must_use]
pub fn github_repo_to_url(repo: &str, protocol: GitProtocol) -> String {
    match protocol {
        GitProtocol::Https => format!("https://github.com/{repo}.git"),
        GitProtocol::Ssh => format!("git@github.com:{repo}.git"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::test_utils::path_to_file_url;

    /// Create a local git repository with a single commit for testing.
    fn create_local_repo(dir: &Path) {
        let run = |args: &[&str]| {
            let output = Command::new("git")
                .args(args)
                .current_dir(dir)
                .env("GIT_AUTHOR_NAME", "Test")
                .env("GIT_AUTHOR_EMAIL", "test@example.com")
                .env("GIT_COMMITTER_NAME", "Test")
                .env("GIT_COMMITTER_EMAIL", "test@example.com")
                .output()
                .expect("git command should run");
            assert!(
                output.status.success(),
                "git {args:?} failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        };
        run(&["init"]);
        std::fs::write(dir.join("hello.txt"), "Hello, world!").expect("write file");
        run(&["add", "hello.txt"]);
        run(&[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-m",
            "initial commit",
        ]);
    }

    #[test]
    fn clone_local_repo() {
        let origin_dir = tempfile::tempdir().expect("tempdir");
        create_local_repo(origin_dir.path());

        let clone_dir = tempfile::tempdir().expect("tempdir");
        let dest = clone_dir.path().join("cloned");

        let url = path_to_file_url(origin_dir.path());
        let git = GixCliBackend::default();
        let opts = CloneOptions::default();
        git.clone_repo(&url, &dest, &opts)
            .expect("clone should succeed");

        let content = std::fs::read_to_string(dest.join("hello.txt")).expect("read hello.txt");
        assert_eq!(content, "Hello, world!");
    }

    #[test]
    fn clone_nonexistent_url_returns_error() {
        let dest_dir = tempfile::tempdir().expect("tempdir");
        let dest = dest_dir.path().join("bad-clone");

        let git = GixCliBackend::default();
        let opts = CloneOptions::default();
        let Err(err) = git.clone_repo("file:///nonexistent/repo", &dest, &opts) else {
            panic!("clone should fail for nonexistent URL");
        };

        assert!(
            matches!(err, GitError::CloneFailed { .. }),
            "expected CloneFailed, got {err:?}"
        );
    }

    #[test]
    fn clone_combines_both_errors_when_both_backends_fail() {
        // Real-world dual-failure path: a URL nothing can clone. The
        // outer CloneFailed must carry BOTH "gix:" and "system git:"
        // detail prefixes so a user (and the test) can see that both
        // attempts were tried and what each said.
        let dest_dir = tempfile::tempdir().expect("tempdir");
        let dest = dest_dir.path().join("bad-clone");
        let url = "file:///definitely/does/not/exist/xyz123";

        let git = GixCliBackend::default();
        let err = git
            .clone_repo(url, &dest, &CloneOptions::default())
            .expect_err("nonexistent URL must fail both backends");

        let full = crate::error::error_full_chain(&err);
        assert!(
            full.contains("gix:"),
            "combined error must mention gix attempt: {full}"
        );
        assert!(
            full.contains("system git:"),
            "combined error must mention system git attempt: {full}"
        );

        // Our nesting must not add duplicate "failed to clone" prefixes.
        // Inner library errors may mention the URL on their own — that's
        // outside our control — but our outer wrapper should contribute
        // exactly one "failed to clone" header.
        let prefix_count = full.matches("failed to clone").count();
        assert_eq!(
            prefix_count, 1,
            "outer wrapper should contribute exactly one 'failed to clone' \
             (more would mean to_string was used where error_source_chain belongs); got {prefix_count} in: {full}"
        );
    }

    #[test]
    fn combine_clone_errors_preserves_root_causes_without_url_duplication() {
        // Direct unit test for the error-composition function used by the
        // dual-backend fallback. Constructs the inputs explicitly so the
        // test does not depend on filesystem state or git version.
        let url = "https://example.com/r.git";
        let gix_err = GitError::CloneFailed {
            url: url.into(),
            source: "gix root cause".to_owned().into(),
        };
        let cli_err = GitError::CloneFailed {
            url: url.into(),
            source: "stderr: cli root cause".to_owned().into(),
        };

        let combined = combine_clone_errors(url, &gix_err, &cli_err);

        let full = crate::error::error_full_chain(&combined);
        assert!(full.contains("gix root cause"), "lost gix root: {full}");
        assert!(full.contains("cli root cause"), "lost cli root: {full}");
        assert_eq!(
            full.matches(url).count(),
            1,
            "URL must not be duplicated: {full}"
        );
    }

    #[test]
    fn combine_clone_and_cleanup_errors_includes_both() {
        // Verifies the cleanup-failure branch's error composition: when
        // gix fails AND removing the partial clone also fails, the user
        // needs to see both pieces of information.
        let url = "https://example.com/r.git";
        let gix_err = GitError::CloneFailed {
            url: url.into(),
            source: "gix transport error".to_owned().into(),
        };
        let cleanup_err =
            std::io::Error::new(std::io::ErrorKind::PermissionDenied, "cannot remove");

        let combined = combine_clone_and_cleanup_errors(url, &gix_err, &cleanup_err);
        let full = crate::error::error_full_chain(&combined);

        assert!(
            full.contains("gix transport error"),
            "missing gix detail: {full}"
        );
        assert!(
            full.contains("cleanup failed"),
            "missing cleanup tag: {full}"
        );
        assert!(
            full.contains("cannot remove"),
            "missing cleanup detail: {full}"
        );
    }

    #[test]
    fn clone_repo_with_git_ref_checks_out_branch() {
        let origin_dir = tempfile::tempdir().expect("tempdir");
        create_local_repo(origin_dir.path());

        // Create a branch in the origin.
        let run = |args: &[&str]| {
            let output = Command::new("git")
                .args(args)
                .current_dir(origin_dir.path())
                .env("GIT_AUTHOR_NAME", "Test")
                .env("GIT_AUTHOR_EMAIL", "test@example.com")
                .env("GIT_COMMITTER_NAME", "Test")
                .env("GIT_COMMITTER_EMAIL", "test@example.com")
                .output()
                .expect("git command should run");
            assert!(
                output.status.success(),
                "git {args:?} failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        };
        run(&["checkout", "-b", "feature-branch"]);
        std::fs::write(origin_dir.path().join("feature.txt"), "feature work").expect("write");
        run(&["add", "feature.txt"]);
        run(&[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-m",
            "feature commit",
        ]);

        // Clone with git_ref pointing to the branch.
        let clone_dir = tempfile::tempdir().expect("tempdir");
        let dest = clone_dir.path().join("cloned");
        let url = path_to_file_url(origin_dir.path());

        let git = GixCliBackend::default();
        let opts = CloneOptions {
            git_ref: Some(GitRef::new("feature-branch").expect("valid ref")),
        };
        git.clone_repo(&url, &dest, &opts)
            .expect("clone with ref should succeed");

        assert!(
            dest.join("feature.txt").exists(),
            "feature.txt should exist on checked-out branch"
        );
    }

    #[test]
    fn clone_repo_with_invalid_git_ref_returns_error() {
        let origin_dir = tempfile::tempdir().expect("tempdir");
        create_local_repo(origin_dir.path());

        let clone_dir = tempfile::tempdir().expect("tempdir");
        let dest = clone_dir.path().join("cloned");
        let url = path_to_file_url(origin_dir.path());

        let git = GixCliBackend::default();
        let opts = CloneOptions {
            git_ref: Some(GitRef::new("nonexistent-branch").expect("structurally valid")),
        };
        let err = git
            .clone_repo(&url, &dest, &opts)
            .expect_err("should fail for nonexistent ref");

        assert!(
            matches!(err, GitError::CloneFailed { .. }),
            "expected CloneFailed, got {err:?}"
        );
    }

    #[test]
    fn git_ref_rejects_dash_prefixed_value_at_construction() {
        // Dash-prefix rejection now lives in the type, not in the backend.
        // This proves the parse-don't-validate boundary: an invalid ref
        // cannot be constructed and thus cannot reach `git checkout`.
        let err = GitRef::new("--orphan=malicious").expect_err("should reject dash-prefixed");
        assert!(
            matches!(err, crate::error::ValidationError::InvalidName { .. }),
            "expected InvalidName, got {err:?}"
        );
    }

    #[test]
    fn git_ref_rejects_empty_value_at_construction() {
        let err = GitRef::new("").expect_err("should reject empty");
        assert!(
            matches!(err, crate::error::ValidationError::InvalidName { .. }),
            "expected InvalidName, got {err:?}"
        );
    }

    #[test]
    fn git_ref_try_from_str_accepts_valid_ref() {
        let r: GitRef = "v1.2.3".try_into().expect("tag is valid");
        assert_eq!(r.as_str(), "v1.2.3");
    }

    #[test]
    fn clone_failure_cleans_up_dest_so_retry_can_proceed() {
        // After a checkout-after-clone failure, dest must be removed so a
        // subsequent clone is not blocked by "destination path already exists".
        let origin_dir = tempfile::tempdir().expect("tempdir");
        create_local_repo(origin_dir.path());

        let clone_dir = tempfile::tempdir().expect("tempdir");
        let dest = clone_dir.path().join("cloned");
        let url = path_to_file_url(origin_dir.path());
        let git = GixCliBackend::default();

        // First attempt: bad ref. Clone succeeds, checkout fails.
        let _err = git
            .clone_repo(
                &url,
                &dest,
                &CloneOptions {
                    git_ref: Some(GitRef::new("nonexistent-branch").expect("structurally valid")),
                },
            )
            .expect_err("first attempt should fail");

        assert!(
            !dest.exists(),
            "dest should have been cleaned up after checkout failure, found leftover at {}",
            dest.display()
        );

        // Second attempt with no ref must now succeed (would fail with
        // "destination already exists" if cleanup didn't happen).
        git.clone_repo(&url, &dest, &CloneOptions::default())
            .expect("retry after cleanup should succeed");
        assert!(
            dest.join("hello.txt").exists(),
            "retry should populate dest"
        );
    }

    #[test]
    fn github_repo_to_url_https() {
        assert_eq!(
            github_repo_to_url("owner/repo", GitProtocol::Https),
            "https://github.com/owner/repo.git"
        );
    }

    #[test]
    fn github_repo_to_url_ssh() {
        assert_eq!(
            github_repo_to_url("owner/repo", GitProtocol::Ssh),
            "git@github.com:owner/repo.git"
        );
    }

    #[test]
    fn git_protocol_default_is_https() {
        assert_eq!(GitProtocol::default(), GitProtocol::Https);
    }

    #[test]
    fn git_protocol_serde_roundtrip() {
        assert_eq!(
            serde_json::to_string(&GitProtocol::Https).expect("serialize"),
            "\"https\""
        );
        assert_eq!(
            serde_json::to_string(&GitProtocol::Ssh).expect("serialize"),
            "\"ssh\""
        );
        assert_eq!(
            serde_json::from_str::<GitProtocol>("\"https\"").expect("deserialize"),
            GitProtocol::Https
        );
        assert_eq!(
            serde_json::from_str::<GitProtocol>("\"ssh\"").expect("deserialize"),
            GitProtocol::Ssh
        );
    }

    #[test]
    fn verify_sha_matches_full_sha() {
        let dir = tempfile::tempdir().expect("tempdir");
        create_local_repo(dir.path());

        let repo = gix::open(dir.path()).expect("open repo");
        let head_sha = repo.head_id().expect("head_id").to_string();

        let git = GixCliBackend::default();
        git.verify_sha(dir.path(), &head_sha)
            .expect("full SHA should match");
    }

    #[test]
    fn verify_sha_matches_prefix() {
        let dir = tempfile::tempdir().expect("tempdir");
        create_local_repo(dir.path());

        let repo = gix::open(dir.path()).expect("open repo");
        let head_sha = repo.head_id().expect("head_id").to_string();
        let prefix = &head_sha[..7];

        let git = GixCliBackend::default();
        git.verify_sha(dir.path(), prefix)
            .expect("7-char prefix should match");
    }

    #[test]
    fn verify_sha_rejects_wrong_sha() {
        let dir = tempfile::tempdir().expect("tempdir");
        create_local_repo(dir.path());

        let git = GixCliBackend::default();
        let err = git
            .verify_sha(dir.path(), "0000000deadbeef")
            .expect_err("should reject wrong SHA");

        assert!(
            matches!(err, GitError::ShaMismatch { .. }),
            "expected ShaMismatch, got {err:?}"
        );
    }

    #[test]
    fn verify_sha_rejects_empty_expected() {
        let dir = tempfile::tempdir().expect("tempdir");
        create_local_repo(dir.path());

        let git = GixCliBackend::default();
        let err = git
            .verify_sha(dir.path(), "")
            .expect_err("should reject empty SHA");

        assert!(
            matches!(err, GitError::ShaMismatch { .. }),
            "expected ShaMismatch, got {err:?}"
        );
    }

    #[test]
    fn verify_sha_rejects_expected_longer_than_actual_prefix() {
        let dir = tempfile::tempdir().expect("tempdir");
        create_local_repo(dir.path());

        let repo = gix::open(dir.path()).expect("open repo");
        let head_sha = repo.head_id().expect("head_id").to_string();

        // Append extra characters to the actual SHA so it can never be a valid prefix.
        let too_long = format!("{head_sha}extra");
        let git = GixCliBackend::default();
        let err = git
            .verify_sha(dir.path(), &too_long)
            .expect_err("should reject overly long expected SHA");

        assert!(
            matches!(err, GitError::ShaMismatch { .. }),
            "expected ShaMismatch, got {err:?}"
        );
    }

    #[test]
    fn pull_repo_on_non_repo_returns_error() {
        let dir = tempfile::tempdir().expect("tempdir");

        let git = GixCliBackend::default();
        let err = git
            .pull_repo(dir.path())
            .expect_err("should fail on non-repo");

        assert!(
            matches!(err, GitError::OpenFailed { .. }),
            "expected OpenFailed, got {err:?}"
        );
    }

    #[test]
    fn pull_repo_fetches_new_commits() {
        // Create a "remote" repo with one commit.
        let origin_dir = tempfile::tempdir().expect("tempdir");
        create_local_repo(origin_dir.path());

        // Clone it locally.
        let clone_dir = tempfile::tempdir().expect("tempdir");
        let dest = clone_dir.path().join("cloned");
        let url = path_to_file_url(origin_dir.path());
        let git = GixCliBackend::default();
        let opts = CloneOptions::default();
        git.clone_repo(&url, &dest, &opts)
            .expect("clone should succeed");

        // Add a second commit to the origin.
        let run = |args: &[&str], dir: &Path| {
            let output = Command::new("git")
                .args(args)
                .current_dir(dir)
                .env("GIT_AUTHOR_NAME", "Test")
                .env("GIT_AUTHOR_EMAIL", "test@example.com")
                .env("GIT_COMMITTER_NAME", "Test")
                .env("GIT_COMMITTER_EMAIL", "test@example.com")
                .output()
                .expect("git command should run");
            assert!(
                output.status.success(),
                "git {args:?} failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        };
        std::fs::write(origin_dir.path().join("second.txt"), "second").expect("write");
        run(&["add", "second.txt"], origin_dir.path());
        run(
            &[
                "-c",
                "commit.gpgsign=false",
                "commit",
                "-m",
                "second commit",
            ],
            origin_dir.path(),
        );

        // Pull into the clone -- the new file should appear.
        git.pull_repo(&dest).expect("pull should succeed");

        assert!(
            dest.join("second.txt").exists(),
            "second.txt should exist after pull"
        );
        let content = std::fs::read_to_string(dest.join("second.txt")).expect("read");
        assert_eq!(content, "second");
    }
}

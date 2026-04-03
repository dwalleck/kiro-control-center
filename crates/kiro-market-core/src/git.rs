//! Git operations for cloning and updating marketplace repositories.
//!
//! Uses `gix` for clone and repository inspection, and shells out to the
//! `git` CLI for operations that require working-tree updates (pull,
//! checkout). Errors are mapped into domain-specific [`GitError`] variants.

use std::num::NonZeroU32;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::AtomicBool;

use gix::progress::Discard;
use serde::{Deserialize, Serialize};
use tracing::debug;

use crate::error::GitError;

/// Default SSH connect timeout in seconds applied via `GIT_SSH_COMMAND`.
const SSH_CONNECT_TIMEOUT_SECS: u32 = 30;

/// Run a `git` command with SSH connect-timeout protection.
///
/// Sets `GIT_SSH_COMMAND` with a 30-second `ConnectTimeout` to prevent
/// indefinite hangs when SSH port 22 is firewalled. Detects a missing
/// `git` binary and returns [`GitError::GitNotFound`].
fn run_git(args: &[&str], dir: &Path) -> Result<std::process::Output, GitError> {
    let ssh_cmd = format!("ssh -o ConnectTimeout={SSH_CONNECT_TIMEOUT_SECS}");

    Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_SSH_COMMAND", &ssh_cmd)
        .output()
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => GitError::GitNotFound,
            _ => GitError::GitCommandFailed {
                dir: dir.to_path_buf(),
                source: Box::new(e),
            },
        })
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

/// Clone a remote Git repository into `dest`.
///
/// Uses `gix` for the clone operation. When `git_ref` is `None`, a shallow
/// clone (depth 1) is used to reduce transfer size. When `git_ref` is
/// provided, a full clone is performed followed by a `git checkout` of the
/// specified branch, tag, or SHA (requires the `git` CLI in `$PATH`).
///
/// # Errors
///
/// Returns [`GitError::CloneFailed`] if the clone or checkout fails.
pub fn clone_repo(url: &str, dest: &Path, git_ref: Option<&str>) -> Result<(), GitError> {
    debug!(url, dest = %dest.display(), git_ref, "cloning repository");

    let map_err = |e: Box<dyn std::error::Error + Send + Sync>| GitError::CloneFailed {
        url: url.to_owned(),
        source: e,
    };

    let mut prepare = gix::prepare_clone(url, dest).map_err(|e| map_err(Box::new(e)))?;

    if git_ref.is_none() {
        let depth = NonZeroU32::MIN;
        prepare = prepare.with_shallow(gix::remote::fetch::Shallow::DepthAtRemote(depth));
    }

    let (mut checkout, _outcome) = prepare
        .fetch_then_checkout(Discard, &AtomicBool::new(false))
        .map_err(|e| map_err(Box::new(e)))?;

    let (_repo, _outcome) = checkout
        .main_worktree(Discard, &AtomicBool::new(false))
        .map_err(|e| map_err(Box::new(e)))?;

    if let Some(refname) = git_ref {
        if refname.starts_with('-') {
            return Err(map_err(
                format!("invalid git ref: '{refname}' must not start with '-'").into(),
            ));
        }

        let output = run_git(&["checkout", refname], dest)?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(map_err(stderr.trim().to_owned().into()));
        }
    }

    Ok(())
}

/// Verify the current HEAD of a repository matches the expected SHA prefix.
///
/// Both full and abbreviated SHAs are supported: the check passes if the
/// actual commit SHA starts with the expected string.
///
/// # Errors
///
/// Returns [`GitError::ShaMismatch`] if the actual commit SHA does not match.
/// Returns [`GitError::OpenFailed`] if the repository cannot be read.
pub fn verify_sha(path: &Path, expected_sha: &str) -> Result<(), GitError> {
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

/// Pull the default branch using `git pull --ff-only`.
///
/// Opens the repository at `path` with `gix` to verify it is valid,
/// then runs `git pull --ff-only` to fetch and fast-forward the local
/// branch.
///
/// # Errors
///
/// Returns [`GitError::OpenFailed`] if the path is not a valid repository,
/// or [`GitError::PullFailed`] if the pull fails.
pub fn pull_repo(path: &Path) -> Result<(), GitError> {
    debug!(path = %path.display(), "pulling repository");

    // Verify it's actually a git repo first (preserves the OpenFailed error).
    let _repo = gix::open(path).map_err(|e| GitError::OpenFailed {
        path: path.to_path_buf(),
        source: Box::new(e),
    })?;

    let output = run_git(&["pull", "--ff-only"], path)?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(GitError::PullFailed {
            path: path.to_path_buf(),
            source: stderr.trim().to_owned().into(),
        });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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

        let url = format!("file://{}", origin_dir.path().display());
        clone_repo(&url, &dest, None).expect("clone should succeed");

        let content = std::fs::read_to_string(dest.join("hello.txt")).expect("read hello.txt");
        assert_eq!(content, "Hello, world!");
    }

    #[test]
    fn clone_nonexistent_url_returns_error() {
        let dest_dir = tempfile::tempdir().expect("tempdir");
        let dest = dest_dir.path().join("bad-clone");

        let err = match clone_repo("file:///nonexistent/repo", &dest, None) {
            Err(e) => e,
            Ok(()) => panic!("clone should fail for nonexistent URL"),
        };

        assert!(
            matches!(err, GitError::CloneFailed { .. }),
            "expected CloneFailed, got {err:?}"
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
        run(&["-c", "commit.gpgsign=false", "commit", "-m", "feature commit"]);

        // Clone with git_ref pointing to the branch.
        let clone_dir = tempfile::tempdir().expect("tempdir");
        let dest = clone_dir.path().join("cloned");
        let url = format!("file://{}", origin_dir.path().display());

        clone_repo(&url, &dest, Some("feature-branch")).expect("clone with ref should succeed");

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
        let url = format!("file://{}", origin_dir.path().display());

        let err = clone_repo(&url, &dest, Some("nonexistent-branch"))
            .expect_err("should fail for nonexistent ref");

        assert!(
            matches!(err, GitError::CloneFailed { .. }),
            "expected CloneFailed, got {err:?}"
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

        verify_sha(dir.path(), &head_sha).expect("full SHA should match");
    }

    #[test]
    fn verify_sha_matches_prefix() {
        let dir = tempfile::tempdir().expect("tempdir");
        create_local_repo(dir.path());

        let repo = gix::open(dir.path()).expect("open repo");
        let head_sha = repo.head_id().expect("head_id").to_string();
        let prefix = &head_sha[..7];

        verify_sha(dir.path(), prefix).expect("7-char prefix should match");
    }

    #[test]
    fn verify_sha_rejects_wrong_sha() {
        let dir = tempfile::tempdir().expect("tempdir");
        create_local_repo(dir.path());

        let err = verify_sha(dir.path(), "0000000deadbeef").expect_err("should reject wrong SHA");

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
        let err =
            verify_sha(dir.path(), &too_long).expect_err("should reject overly long expected SHA");

        assert!(
            matches!(err, GitError::ShaMismatch { .. }),
            "expected ShaMismatch, got {err:?}"
        );
    }

    #[test]
    fn pull_repo_on_non_repo_returns_error() {
        let dir = tempfile::tempdir().expect("tempdir");

        let err = pull_repo(dir.path()).expect_err("should fail on non-repo");

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
        let url = format!("file://{}", origin_dir.path().display());
        clone_repo(&url, &dest, None).expect("clone should succeed");

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
            &["-c", "commit.gpgsign=false", "commit", "-m", "second commit"],
            origin_dir.path(),
        );

        // Pull into the clone — the new file should appear.
        pull_repo(&dest).expect("pull should succeed");

        assert!(
            dest.join("second.txt").exists(),
            "second.txt should exist after pull"
        );
        let content = std::fs::read_to_string(dest.join("second.txt")).expect("read");
        assert_eq!(content, "second");
    }
}

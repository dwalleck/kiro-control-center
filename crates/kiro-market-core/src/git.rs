//! Git operations for cloning and updating marketplace repositories.
//!
//! Uses [`git2`] for all Git interactions and maps errors into
//! domain-specific [`GitError`] variants.

use std::path::Path;

use git2::{Cred, FetchOptions, RemoteCallbacks, Repository};
use serde::{Deserialize, Serialize};
use tracing::debug;

use crate::error::GitError;

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

/// Default timeout (in milliseconds) for the initial TCP connection to a
/// git server. Prevents infinite hangs when SSH port 22 is firewalled.
///
/// Binary crates should call
/// `git2::opts::set_server_connect_timeout_in_milliseconds` with this
/// value at startup.
pub const CONNECT_TIMEOUT_MS: i32 = 30_000;

/// Build fetch options with credential callbacks for SSH agent and git
/// credential helpers.
fn build_fetch_options<'a>() -> FetchOptions<'a> {
    let mut callbacks = RemoteCallbacks::new();
    callbacks.credentials(|url, username_from_url, allowed_types| {
        if allowed_types.contains(git2::CredentialType::SSH_KEY)
            && let Some(username) = username_from_url
        {
            return Cred::ssh_key_from_agent(username);
        }
        if allowed_types.contains(git2::CredentialType::USER_PASS_PLAINTEXT) {
            return Cred::credential_helper(&git2::Config::open_default()?, url, username_from_url);
        }
        if allowed_types.contains(git2::CredentialType::DEFAULT) {
            return Cred::default();
        }
        Err(git2::Error::from_str("no credentials available"))
    });

    let mut fetch_options = FetchOptions::new();
    fetch_options.remote_callbacks(callbacks);
    fetch_options
}

/// Clone a remote Git repository into `dest`.
///
/// Uses SSH agent and git credential helpers for authentication when
/// available. If `git_ref` is provided the working tree is checked out to
/// that ref (branch, tag, or commit SHA) after the clone completes.
///
/// # Errors
///
/// Returns [`GitError::CloneFailed`] if the clone or checkout fails.
pub fn clone_repo(url: &str, dest: &Path, git_ref: Option<&str>) -> Result<Repository, GitError> {
    debug!(url, dest = %dest.display(), "cloning repository");

    let fetch_options = build_fetch_options();
    let repo = git2::build::RepoBuilder::new()
        .fetch_options(fetch_options)
        .clone(url, dest)
        .map_err(|source| GitError::CloneFailed {
            url: url.to_owned(),
            source,
        })?;

    if let Some(refname) = git_ref {
        checkout_ref(&repo, refname).map_err(|source| GitError::CloneFailed {
            url: url.to_owned(),
            source,
        })?;
    }

    Ok(repo)
}

/// Verify the current HEAD of a repository matches the expected SHA prefix.
///
/// Both full and abbreviated SHAs are supported: the check passes if the
/// actual commit SHA starts with the expected string.
///
/// # Errors
///
/// Returns [`GitError::ShaMismatch`] if the actual commit SHA does not match.
pub fn verify_sha(repo: &Repository, expected_sha: &str) -> Result<(), GitError> {
    let head = repo.head().map_err(|source| GitError::OpenFailed {
        path: repo
            .workdir()
            .unwrap_or_else(|| Path::new("<bare>"))
            .to_path_buf(),
        source,
    })?;

    let actual_oid = head.target().ok_or_else(|| GitError::OpenFailed {
        path: repo
            .workdir()
            .unwrap_or_else(|| Path::new("<bare>"))
            .to_path_buf(),
        source: git2::Error::from_str("HEAD does not point to a commit"),
    })?;

    let actual_str = actual_oid.to_string();

    if !actual_str.starts_with(expected_sha) {
        return Err(GitError::ShaMismatch {
            expected: expected_sha.to_owned(),
            actual: actual_str,
        });
    }

    Ok(())
}

/// Pull (fetch + fast-forward) the default branch from `origin`.
///
/// Opens the repository at `path`, fetches from the `origin` remote, and
/// attempts a fast-forward merge of the current branch to the fetched head.
///
/// # Errors
///
/// Returns [`GitError::OpenFailed`] if the path is not a valid repository,
/// or [`GitError::PullFailed`] if the fetch or fast-forward fails.
pub fn pull_repo(path: &Path) -> Result<(), GitError> {
    debug!(path = %path.display(), "pulling repository");

    let repo = Repository::open(path).map_err(|source| GitError::OpenFailed {
        path: path.to_path_buf(),
        source,
    })?;

    let mut remote = repo
        .find_remote("origin")
        .map_err(|source| GitError::PullFailed {
            path: path.to_path_buf(),
            source,
        })?;

    let mut fetch_options = build_fetch_options();
    remote
        .fetch(&[] as &[&str], Some(&mut fetch_options), None)
        .map_err(|source| GitError::PullFailed {
            path: path.to_path_buf(),
            source,
        })?;

    let fetch_head = repo
        .find_reference("FETCH_HEAD")
        .map_err(|source| GitError::PullFailed {
            path: path.to_path_buf(),
            source,
        })?;

    let fetch_commit = repo
        .reference_to_annotated_commit(&fetch_head)
        .map_err(|source| GitError::PullFailed {
            path: path.to_path_buf(),
            source,
        })?;

    let (analysis, _) =
        repo.merge_analysis(&[&fetch_commit])
            .map_err(|source| GitError::PullFailed {
                path: path.to_path_buf(),
                source,
            })?;

    if analysis.is_up_to_date() {
        debug!(path = %path.display(), "already up to date");
        return Ok(());
    }

    if analysis.is_fast_forward() {
        let head_ref = repo.head().map_err(|source| GitError::PullFailed {
            path: path.to_path_buf(),
            source,
        })?;

        let refname = head_ref.name().ok_or_else(|| GitError::PullFailed {
            path: path.to_path_buf(),
            source: git2::Error::from_str("HEAD ref name could not be resolved"),
        })?;

        let target_oid = fetch_commit.id();

        repo.find_reference(refname)
            .and_then(|mut r| r.set_target(target_oid, "kiro-market: fast-forward pull"))
            .map_err(|source| GitError::PullFailed {
                path: path.to_path_buf(),
                source,
            })?;

        repo.set_head(refname)
            .map_err(|source| GitError::PullFailed {
                path: path.to_path_buf(),
                source,
            })?;

        repo.checkout_head(Some(git2::build::CheckoutBuilder::default().force()))
            .map_err(|source| GitError::PullFailed {
                path: path.to_path_buf(),
                source,
            })?;

        debug!(path = %path.display(), "fast-forwarded to {}", target_oid);
    } else {
        return Err(GitError::PullFailed {
            path: path.to_path_buf(),
            source: git2::Error::from_str("cannot fast-forward; manual merge required"),
        });
    }

    Ok(())
}

/// Check out a named ref (branch, tag, or commit SHA) in the given repository.
fn checkout_ref(repo: &Repository, refname: &str) -> Result<(), git2::Error> {
    // Try as a direct OID first (for commit SHA), then fall back to revparse.
    let object = repo.revparse_single(refname)?;

    repo.checkout_tree(
        &object,
        Some(git2::build::CheckoutBuilder::default().force()),
    )?;

    // If it resolves to a branch or tag reference, set HEAD symbolically.
    if let Ok(reference) = repo.find_reference(&format!("refs/remotes/origin/{refname}")) {
        repo.set_head(
            reference
                .name()
                .ok_or_else(|| git2::Error::from_str("non-UTF-8 reference name"))?,
        )?;
    } else {
        // Detached HEAD for tags or direct SHAs.
        repo.set_head_detached(object.id())?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    /// Create a bare-bones local Git repository with a single committed file.
    fn create_local_repo(dir: &Path) -> Repository {
        let repo = Repository::init(dir).expect("init should succeed");

        // Configure a dummy author for the commit.
        let sig = git2::Signature::now("Test", "test@example.com").expect("signature");

        let file_path = dir.join("hello.txt");
        fs::write(&file_path, "Hello, world!").expect("write file");

        let mut index = repo.index().expect("index");
        index.add_path(Path::new("hello.txt")).expect("add_path");
        index.write().expect("write index");

        let tree_oid = index.write_tree().expect("write_tree");
        let tree = repo.find_tree(tree_oid).expect("find_tree");

        repo.commit(Some("HEAD"), &sig, &sig, "initial commit", &tree, &[])
            .expect("commit");

        // Drop `tree` before moving `repo` out of this function.
        drop(tree);

        repo
    }

    #[test]
    fn clone_local_repo() {
        let source_dir = tempfile::tempdir().expect("tempdir");
        let dest_dir = tempfile::tempdir().expect("tempdir");
        let dest = dest_dir.path().join("cloned");

        create_local_repo(source_dir.path());

        // file:// URLs: on Unix paths start with /, so file:// + /path works.
        // On Windows, paths start with C:\, need file:///C:/path with forward slashes.
        let path_str = source_dir.path().to_string_lossy().replace('\\', "/");
        let url = if path_str.starts_with('/') {
            format!("file://{path_str}")
        } else {
            format!("file:///{path_str}")
        };
        let repo = clone_repo(&url, &dest, None).expect("clone should succeed");

        assert!(dest.join("hello.txt").exists(), "cloned file should exist");
        assert!(repo.head().is_ok(), "cloned repo should have a valid HEAD");

        let content = fs::read_to_string(dest.join("hello.txt")).expect("read");
        assert_eq!(content, "Hello, world!");
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
    fn clone_nonexistent_url_returns_error() {
        let dest_dir = tempfile::tempdir().expect("tempdir");
        let dest = dest_dir.path().join("bad-clone");

        let err = match clone_repo("file:///nonexistent/repo", &dest, None) {
            Err(e) => e,
            Ok(_) => panic!("clone should fail for nonexistent URL"),
        };

        assert!(
            matches!(err, GitError::CloneFailed { .. }),
            "expected CloneFailed, got {err:?}"
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
    fn verify_sha_matches_full_sha() {
        let dir = tempfile::tempdir().expect("tempdir");
        let repo = create_local_repo(dir.path());

        let head_oid = repo.head().expect("HEAD").target().expect("target");
        let full_sha = head_oid.to_string();

        verify_sha(&repo, &full_sha).expect("full SHA should match");
    }

    #[test]
    fn verify_sha_matches_prefix() {
        let dir = tempfile::tempdir().expect("tempdir");
        let repo = create_local_repo(dir.path());

        let head_oid = repo.head().expect("HEAD").target().expect("target");
        let prefix = &head_oid.to_string()[..7];

        verify_sha(&repo, prefix).expect("SHA prefix should match");
    }

    #[test]
    fn verify_sha_rejects_wrong_sha() {
        let dir = tempfile::tempdir().expect("tempdir");
        let repo = create_local_repo(dir.path());

        let err = verify_sha(&repo, "0000000deadbeef").expect_err("should fail on wrong SHA");

        assert!(
            matches!(err, GitError::ShaMismatch { .. }),
            "expected ShaMismatch, got {err:?}"
        );
    }

    #[test]
    fn verify_sha_rejects_expected_longer_than_actual_prefix() {
        // Regression: the old bidirectional check would pass if the expected
        // SHA *started with* the actual SHA (backwards logic). This test
        // constructs an expected string that begins with the real prefix but
        // has wrong trailing characters.
        let dir = tempfile::tempdir().expect("tempdir");
        let repo = create_local_repo(dir.path());

        let head_oid = repo.head().expect("HEAD").target().expect("target");
        let actual_str = head_oid.to_string();
        let prefix = &actual_str[..7];

        // Build a fake expected that starts with the real prefix but diverges.
        let fake_expected = format!("{prefix}ffffffffffffffffffffffffffffffff0");

        let err = verify_sha(&repo, &fake_expected)
            .expect_err("should reject when expected extends actual with wrong chars");

        assert!(
            matches!(err, GitError::ShaMismatch { .. }),
            "expected ShaMismatch, got {err:?}"
        );
    }
}

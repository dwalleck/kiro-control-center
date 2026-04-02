//! Git operations for cloning and updating marketplace repositories.
//!
//! Uses [`git2`] for all Git interactions and maps errors into
//! domain-specific [`GitError`] variants.

use std::path::Path;

use git2::Repository;
use tracing::debug;

use crate::error::GitError;

/// Convert a GitHub `owner/repo` shorthand into a full HTTPS clone URL.
#[must_use]
pub fn github_repo_to_url(repo: &str) -> String {
    format!("https://github.com/{repo}.git")
}

/// Clone a remote Git repository into `dest`.
///
/// If `git_ref` is provided the working tree is checked out to that ref
/// (branch, tag, or commit SHA) after the clone completes.
///
/// # Errors
///
/// Returns [`GitError::CloneFailed`] if the clone or checkout fails.
pub fn clone_repo(url: &str, dest: &Path, git_ref: Option<&str>) -> Result<Repository, GitError> {
    debug!(url, dest = %dest.display(), "cloning repository");

    let repo = Repository::clone(url, dest).map_err(|source| GitError::CloneFailed {
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

    remote
        .fetch(&[] as &[&str], None, None)
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

        let url = format!("file://{}", source_dir.path().display());
        let repo = clone_repo(&url, &dest, None).expect("clone should succeed");

        assert!(dest.join("hello.txt").exists(), "cloned file should exist");
        assert!(repo.head().is_ok(), "cloned repo should have a valid HEAD");

        let content = fs::read_to_string(dest.join("hello.txt")).expect("read");
        assert_eq!(content, "Hello, world!");
    }

    #[test]
    fn github_repo_to_url_formats_correctly() {
        assert_eq!(
            github_repo_to_url("owner/repo"),
            "https://github.com/owner/repo.git"
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
}

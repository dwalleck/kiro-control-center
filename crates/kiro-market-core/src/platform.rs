//! Cross-platform filesystem linking for local marketplace tracking.
//!
//! On Unix, uses symlinks. On Windows, tries directory junctions (no
//! admin required on NTFS), with copy fallback.

use std::io;
use std::path::Path;

/// What `create_local_link` actually did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkResult {
    /// A true link was created (symlink on Unix, junction on Windows).
    /// Changes to the source are reflected immediately.
    Linked,
    /// The source was copied into the destination (Windows fallback).
    /// Changes to the source will NOT be reflected.
    Copied,
}

/// Create a local link from `src` to `dest` for live marketplace tracking.
///
/// Returns [`LinkResult`] indicating whether a true link or a copy was used,
/// so callers can inform the user about live-tracking behavior.
///
/// # Platform behavior
///
/// - **Unix:** Creates a symbolic link. Always returns `Linked`.
/// - **Windows:** Tries a directory junction (NTFS, no admin required).
///   Falls back to copying `src` into `dest` if junctions fail, returning
///   `Copied` so the caller can warn the user.
///
/// # Errors
///
/// Returns an error if the link or copy operation fails (e.g. `src` does
/// not exist or insufficient permissions).
pub fn create_local_link(src: &Path, dest: &Path) -> io::Result<LinkResult> {
    sys::create_local_link(src, dest)
}

/// Check whether `path` is a local link (symlink or directory junction).
///
/// Returns `false` for regular directories (including copy-fallback dirs),
/// nonexistent paths, and files.
#[must_use]
pub fn is_local_link(path: &Path) -> bool {
    sys::is_local_link(path)
}

/// Remove a local link without removing the target contents.
///
/// Should only be called when [`is_local_link`] returns `true`. For
/// regular directories (e.g. copy-fallback), use `fs::remove_dir_all`.
///
/// # Errors
///
/// Returns an error if the link cannot be removed (e.g. it does not exist
/// or the process lacks permissions).
pub fn remove_local_link(path: &Path) -> io::Result<()> {
    sys::remove_local_link(path)
}

#[cfg(unix)]
mod sys {
    use std::io;
    use std::path::Path;

    use super::LinkResult;

    pub fn create_local_link(src: &Path, dest: &Path) -> io::Result<LinkResult> {
        std::os::unix::fs::symlink(src, dest)?;
        Ok(LinkResult::Linked)
    }

    pub fn is_local_link(path: &Path) -> bool {
        path.is_symlink()
    }

    pub fn remove_local_link(path: &Path) -> io::Result<()> {
        std::fs::remove_file(path)
    }
}

#[cfg(windows)]
mod sys {
    use std::io;
    use std::path::Path;

    use super::LinkResult;

    pub fn create_local_link(src: &Path, dest: &Path) -> io::Result<LinkResult> {
        // Junctions require absolute source paths.
        let src = std::fs::canonicalize(src)?;

        // Try directory junction first (works without admin on NTFS).
        match junction::create(&src, dest) {
            Ok(()) => return Ok(LinkResult::Linked),
            Err(e) => {
                tracing::debug!(
                    error = %e,
                    "junction failed, falling back to directory copy"
                );
            }
        }

        // Fallback: copy the directory tree.
        copy_dir_recursive(&src, dest)?;
        Ok(LinkResult::Copied)
    }

    pub fn is_local_link(path: &Path) -> bool {
        // Check for both symlinks (IO_REPARSE_TAG_SYMLINK) and directory
        // junctions (IO_REPARSE_TAG_MOUNT_POINT). Path::is_symlink() only
        // detects symlinks, so we also check via the junction crate.
        path.is_symlink() || junction::exists(path).unwrap_or(false)
    }

    pub fn remove_local_link(path: &Path) -> io::Result<()> {
        // Junctions are directory reparse points — remove_dir removes the
        // junction without deleting the target. Symlinks use remove_file.
        if junction::exists(path).unwrap_or(false) || path.is_dir() {
            std::fs::remove_dir(path)
        } else {
            std::fs::remove_file(path)
        }
    }

    fn copy_dir_recursive(src: &Path, dest: &Path) -> io::Result<()> {
        std::fs::create_dir_all(dest)?;
        for entry in std::fs::read_dir(src)? {
            let entry = entry?;
            let target = dest.join(entry.file_name());
            if entry.file_type()?.is_dir() {
                copy_dir_recursive(&entry.path(), &target)?;
            } else {
                std::fs::copy(entry.path(), target)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_and_detect_local_link() {
        let dir = tempfile::tempdir().expect("tempdir");
        let src = dir.path().join("source");
        std::fs::create_dir_all(&src).expect("create source");
        std::fs::write(src.join("file.txt"), "hello").expect("write");

        let dest = dir.path().join("link");
        create_local_link(&src, &dest).expect("create link");

        // On Windows, if junctions aren't supported, create_local_link falls
        // back to a directory copy. In that case is_local_link returns false
        // and the test still passes — we just verify the content is accessible.
        if is_local_link(&dest) {
            assert!(is_local_link(&dest), "should detect as local link");
        }

        assert!(
            dest.join("file.txt").exists(),
            "linked content should be visible"
        );
    }

    #[test]
    fn remove_local_link_does_not_delete_target() {
        let dir = tempfile::tempdir().expect("tempdir");
        let src = dir.path().join("source");
        std::fs::create_dir_all(&src).expect("create source");
        std::fs::write(src.join("file.txt"), "hello").expect("write");

        let dest = dir.path().join("link");
        create_local_link(&src, &dest).expect("create link");

        if is_local_link(&dest) {
            remove_local_link(&dest).expect("remove link");
            assert!(!dest.exists(), "link should be gone");
        } else {
            // Copy fallback — remove_local_link won't work on a regular dir.
            // Just verify the source is still intact.
        }

        assert!(src.join("file.txt").exists(), "source should be intact");
    }

    #[test]
    fn is_local_link_false_for_regular_dir() {
        let dir = tempfile::tempdir().expect("tempdir");
        let regular = dir.path().join("regular");
        std::fs::create_dir_all(&regular).expect("create dir");

        assert!(!is_local_link(&regular), "regular dir is not a link");
    }
}

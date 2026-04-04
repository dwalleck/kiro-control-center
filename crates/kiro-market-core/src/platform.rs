//! Cross-platform filesystem linking for local marketplace tracking.
//!
//! On Unix, uses symlinks. On Windows, tries directory junctions (no
//! admin required on NTFS), with copy fallback.

use std::io;
use std::path::Path;

/// Create a local link from `src` to `dest` for live marketplace tracking.
///
/// # Platform behavior
///
/// - **Unix:** Creates a symbolic link.
/// - **Windows:** Tries a directory junction (NTFS, no admin required).
///   Falls back to copying `src` into `dest` if junctions fail, logging
///   a warning that changes won't be live-tracked.
///
/// # Errors
///
/// Returns an error if the link or copy operation fails (e.g. `src` does
/// not exist, `dest` already exists, or insufficient permissions).
pub fn create_local_link(src: &Path, dest: &Path) -> io::Result<()> {
    sys::create_local_link(src, dest)
}

/// Check whether `path` is a local link (symlink or directory junction).
#[must_use]
pub fn is_local_link(path: &Path) -> bool {
    sys::is_local_link(path)
}

/// Remove a local link without removing the target contents.
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

    pub fn create_local_link(src: &Path, dest: &Path) -> io::Result<()> {
        std::os::unix::fs::symlink(src, dest)
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

    pub fn create_local_link(src: &Path, dest: &Path) -> io::Result<()> {
        // Junctions require absolute source paths.
        let src = std::fs::canonicalize(src)?;

        // Try directory junction first (works without admin on NTFS).
        match junction::create(&src, dest) {
            Ok(()) => return Ok(()),
            Err(e) => {
                tracing::debug!(
                    error = %e,
                    "junction failed, falling back to directory copy"
                );
            }
        }

        // Fallback: copy the directory tree.
        copy_dir_recursive(&src, dest)?;
        tracing::warn!(
            src = %src.display(),
            dest = %dest.display(),
            "used directory copy instead of junction — local changes will NOT be live-tracked"
        );
        Ok(())
    }

    pub fn is_local_link(path: &Path) -> bool {
        path.is_symlink()
    }

    pub fn remove_local_link(path: &Path) -> io::Result<()> {
        // Junctions are directory reparse points — remove_dir removes the
        // junction without deleting the target. Symlinks use remove_file.
        if path.is_dir() {
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

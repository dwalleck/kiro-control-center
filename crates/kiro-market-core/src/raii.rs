//! RAII helpers shared across the crate.
//!
//! Currently exposes [`DirCleanupGuard`], a single-shot directory remover
//! used by the marketplace add/remove flow (`service::add` retargets it
//! across the temp→final rename) and the Windows local-link copy fallback
//! (`platform::sys::create_local_link` on Windows). Both call sites
//! previously carried near-identical `path + defused: bool + Drop` shells;
//! collapsing them here keeps the cleanup invariant in one place so a
//! future fix to ordering, log severity, or `NotFound` handling applies
//! once.

use std::fs;
use std::path::PathBuf;

use tracing::warn;

/// Removes a directory tree on `Drop` unless explicitly defused.
///
/// Lifecycle, by intended call site:
///
/// ```text
/// let mut guard = DirCleanupGuard::new(temp_dir, "marketplace temp directory");
/// // ... clone, scan, validate ...
/// fs::rename(&temp_dir, &final_dir)?;
/// guard.retarget(final_dir.clone());  // armed → final_dir
/// // ... register; on rollback explicitly remove + defuse ...
/// guard.defuse();                     // disarmed
/// ```
///
/// Errors during cleanup are logged at `warn!` (with the `label` field for
/// tracability) rather than propagated — the guard runs from `Drop` where
/// returning errors is impossible. The label appears in the warning so
/// users debugging a leftover directory can grep their logs and find the
/// originating call site without having to thread a path back through the
/// stack.
///
/// `NotFound` on the cleanup is silently swallowed: a successful flow
/// often unlinks the temp dir as part of its work (atomic rename) and
/// the guard's job is then "make sure nothing is left," not "make sure
/// something WAS there."
pub(crate) struct DirCleanupGuard {
    path: PathBuf,
    label: &'static str,
    defused: bool,
}

impl DirCleanupGuard {
    /// Construct an armed guard. `label` appears in any cleanup-failure
    /// warning to make leftover directories grep-able to their owner.
    pub(crate) fn new(path: PathBuf, label: &'static str) -> Self {
        Self {
            path,
            label,
            defused: false,
        }
    }

    /// Re-point the guard at a new on-disk location.
    ///
    /// Use after an atomic rename has moved the temp directory into its
    /// final place. The previous path is no longer the guard's
    /// responsibility — it has either ceased to exist (the rename
    /// consumed it) or the caller has taken ownership of cleaning it up.
    pub(crate) fn retarget(&mut self, new_path: PathBuf) {
        self.path = new_path;
    }

    /// Prevent cleanup on drop (call after the operation has succeeded
    /// or after the caller has handled cleanup explicitly along an error
    /// path).
    pub(crate) fn defuse(&mut self) {
        self.defused = true;
    }
}

impl Drop for DirCleanupGuard {
    fn drop(&mut self) {
        if self.defused {
            return;
        }
        if let Err(e) = fs::remove_dir_all(&self.path)
            && e.kind() != std::io::ErrorKind::NotFound
        {
            warn!(
                path = %self.path.display(),
                label = %self.label,
                error = %e,
                "failed to clean up directory — remove it manually"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drop_removes_armed_target() {
        let dir = tempfile::tempdir().expect("tempdir");
        let target = dir.path().join("victim");
        fs::create_dir_all(&target).expect("mkdir");

        {
            let _guard = DirCleanupGuard::new(target.clone(), "test");
        }
        assert!(!target.exists(), "armed guard must remove target on drop");
    }

    #[test]
    fn defuse_prevents_removal() {
        let dir = tempfile::tempdir().expect("tempdir");
        let target = dir.path().join("survivor");
        fs::create_dir_all(&target).expect("mkdir");

        {
            let mut guard = DirCleanupGuard::new(target.clone(), "test");
            guard.defuse();
        }
        assert!(target.exists(), "defused guard must NOT remove target");
    }

    #[test]
    fn retarget_moves_cleanup_focus() {
        let dir = tempfile::tempdir().expect("tempdir");
        let original = dir.path().join("original");
        let renamed = dir.path().join("renamed");
        fs::create_dir_all(&original).expect("mkdir original");
        // Simulate the post-rename state: original is gone, renamed exists.
        fs::rename(&original, &renamed).expect("rename");
        assert!(!original.exists());
        assert!(renamed.exists());

        {
            let mut guard = DirCleanupGuard::new(original.clone(), "test");
            guard.retarget(renamed.clone());
        }
        assert!(
            !renamed.exists(),
            "guard should have cleaned up the new path"
        );
    }

    #[test]
    fn drop_swallows_not_found_silently() {
        // The successful path often consumes the temp dir before drop
        // runs (atomic rename moves it). A NotFound on drop is expected
        // and must not warn or panic.
        let dir = tempfile::tempdir().expect("tempdir");
        let nonexistent = dir.path().join("never-existed");

        let _guard = DirCleanupGuard::new(nonexistent, "test");
        // Drop runs at end of scope; if it panics or surfaces an error,
        // this test fails. The implicit success is the assertion.
    }
}

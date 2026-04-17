//! Advisory file locking for concurrent marketplace operations.
//!
//! Uses [`fs4`] exclusive advisory locks to serialise read-modify-write cycles
//! on shared JSON files (`installed-skills.json`, `known_marketplaces.json`, etc.).

use std::fs::{self, OpenOptions};
use std::io;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use fs4::fs_std::FileExt;

/// Maximum time to wait for the lock before giving up.
const LOCK_TIMEOUT: Duration = Duration::from_secs(10);

/// How long to sleep between lock-acquisition retries.
const LOCK_RETRY_INTERVAL: Duration = Duration::from_millis(100);

/// Returns the `.lock` sibling path for a given file path.
///
/// # Errors
///
/// Returns an `io::Error` with `ErrorKind::InvalidInput` if `path` has no
/// file-name component (e.g. a bare root path like `/`). Returning `Result`
/// rather than panicking lets callers — including those handling user input
/// downstream of validation — surface the failure cleanly.
pub fn lock_path_for(path: &Path) -> io::Result<PathBuf> {
    let name = path.file_name().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "cannot derive lock path: '{}' has no file-name component",
                path.display()
            ),
        )
    })?;
    let mut lock_name = name.to_os_string();
    lock_name.push(".lock");
    Ok(path.with_file_name(lock_name))
}

/// Acquires an exclusive advisory lock on a `.lock` sibling of `path`, then
/// runs the closure `f` while the lock is held.
///
/// The lock file and any missing parent directories are created automatically.
/// The lock is released when the file handle is dropped — including on a
/// panic from `f`. Panics from `f` are caught with [`catch_unwind`], a warning
/// is emitted (since the locked file may now contain partial writes), and the
/// panic is then resumed so callers see the original failure.
///
/// # Errors
///
/// Returns `io::Error` with `ErrorKind::TimedOut` if the lock cannot be
/// acquired within [`LOCK_TIMEOUT`]. Returns `ErrorKind::InvalidInput` if
/// `path` has no file-name component. Otherwise, propagates any I/O error
/// from lock-file creation or errors returned by the closure.
pub fn with_file_lock<T, E>(path: &Path, f: impl FnOnce() -> Result<T, E>) -> Result<T, E>
where
    E: From<io::Error>,
{
    let lock_path = lock_path_for(path)?;

    if let Some(parent) = lock_path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }

    let file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)?;

    let start = Instant::now();
    let mut first_attempt = true;

    loop {
        match file.try_lock_exclusive() {
            Ok(true) => break,
            Ok(false) => {}
            Err(e) => return Err(e.into()),
        }

        if start.elapsed() >= LOCK_TIMEOUT {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!("timed out waiting for lock on {}", lock_path.display()),
            )
            .into());
        }

        if first_attempt {
            tracing::warn!(
                target = %path.display(),
                lock = %lock_path.display(),
                "waiting for file lock, another process may be running"
            );
            first_attempt = false;
        }

        std::thread::sleep(LOCK_RETRY_INTERVAL);
    }

    // Catch panics so the lock is reliably released. Without this the OS
    // still drops the file (releasing the lock), but no warning is emitted
    // about possible mid-write corruption inside the locked region.
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(result) => result,
        Err(panic_payload) => {
            tracing::warn!(
                target = %path.display(),
                "closure panicked while holding file lock; the locked file may contain partial writes"
            );
            // Lock release happens on `file` Drop during stack unwinding.
            resume_unwind(panic_payload);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Barrier};

    use tempfile::TempDir;

    use super::*;

    #[test]
    fn lock_path_for_appends_lock_extension() {
        let input = Path::new("/tmp/installed-skills.json");
        let expected = PathBuf::from("/tmp/installed-skills.json.lock");
        assert_eq!(lock_path_for(input).expect("file name present"), expected);
    }

    #[test]
    fn lock_path_for_returns_invalid_input_on_root_path() {
        let err = lock_path_for(Path::new("/")).expect_err("root has no file name");
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        assert!(
            err.to_string().contains("file-name component"),
            "message should hint at the cause: {err}"
        );
    }

    #[test]
    fn with_file_lock_creates_lock_file_and_runs_closure() {
        let dir = TempDir::new().unwrap();
        let target = dir.path().join("state.json");

        let result: Result<&str, io::Error> = with_file_lock(&target, || Ok("done"));

        assert_eq!(result.unwrap(), "done");
        assert!(
            lock_path_for(&target).expect("file name present").exists(),
            "lock file should exist"
        );
    }

    #[test]
    fn with_file_lock_propagates_closure_error() {
        let dir = TempDir::new().unwrap();
        let target = dir.path().join("state.json");

        let result: Result<(), io::Error> = with_file_lock(&target, || {
            Err(io::Error::new(io::ErrorKind::InvalidData, "bad data"))
        });

        let err = result.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert_eq!(err.to_string(), "bad data");
    }

    #[test]
    fn with_file_lock_creates_parent_directories() {
        let dir = TempDir::new().unwrap();
        let target = dir.path().join("a").join("b").join("c").join("state.json");

        let result: Result<u32, io::Error> = with_file_lock(&target, || Ok(42));

        assert_eq!(result.unwrap(), 42);
        assert!(
            lock_path_for(&target).expect("file name present").exists(),
            "lock file should exist in nested directory"
        );
    }

    #[test]
    fn with_file_lock_releases_lock_when_closure_panics() {
        // After a panicking closure, the OS lock should still be released
        // (via Drop on the file handle during unwind), so a subsequent
        // acquire on the SAME path succeeds. This pins the catch_unwind +
        // resume_unwind contract: panic surfaces, but lock state is clean.
        let dir = TempDir::new().unwrap();
        let target = dir.path().join("state.json");
        let target_clone = target.clone();

        let panic_outcome = std::panic::catch_unwind(move || {
            let _: Result<(), io::Error> =
                with_file_lock(&target_clone, || -> Result<(), io::Error> {
                    panic!("simulated mid-write panic");
                });
        });
        assert!(
            panic_outcome.is_err(),
            "panic from closure should propagate to caller"
        );

        // Lock must be free now: a second acquire on the same path succeeds
        // immediately (no LOCK_TIMEOUT delay).
        let start = std::time::Instant::now();
        let result: Result<(), io::Error> = with_file_lock(&target, || Ok(()));
        let elapsed = start.elapsed();

        result.expect("second acquire should succeed");
        assert!(
            elapsed < std::time::Duration::from_secs(1),
            "second acquire should not wait for timeout, took {elapsed:?}"
        );
    }

    #[test]
    fn with_file_lock_serializes_concurrent_access() {
        // NOTE: This tests thread-level serialization, not cross-process locking.
        let dir = TempDir::new().unwrap();
        let target = dir.path().join("counter.json");
        let counter_path = dir.path().join("counter.txt");

        // Initialise counter file to "0".
        fs::write(&counter_path, "0").unwrap();

        let num_threads = 2;
        let increments_per_thread = 50;
        let barrier = Arc::new(Barrier::new(num_threads));

        let handles: Vec<_> = (0..num_threads)
            .map(|_| {
                let barrier = Arc::clone(&barrier);
                let target = target.clone();
                let counter_path = counter_path.clone();

                std::thread::spawn(move || {
                    barrier.wait();
                    for _ in 0..increments_per_thread {
                        let result: Result<(), io::Error> = with_file_lock(&target, || {
                            let val: u64 = fs::read_to_string(&counter_path)?
                                .trim()
                                .parse()
                                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
                            fs::write(&counter_path, (val + 1).to_string())?;
                            Ok(())
                        });
                        result.expect("lock + increment should succeed");
                    }
                })
            })
            .collect();

        for handle in handles {
            handle.join().expect("thread should not panic");
        }

        let final_value: u64 = fs::read_to_string(&counter_path)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        assert_eq!(
            final_value,
            u64::try_from(num_threads * increments_per_thread).unwrap(),
            "all increments should be serialised"
        );
    }
}

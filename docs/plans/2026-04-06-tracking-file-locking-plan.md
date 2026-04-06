# Tracking File Locking — Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add advisory file locking to `installed-skills.json` and `known_marketplaces.json` to prevent concurrent CLI/Tauri processes from clobbering each other's writes.

**Architecture:** Add an `fs4` dependency for cross-platform advisory file locking. Create a `with_file_lock` helper that acquires an exclusive lock on a `.lock` sibling file with a timeout. Wrap the four existing read-modify-write sites in `project.rs` and `cache.rs`, expanding lock scope to cover both filesystem mutations and tracking updates.

**Tech Stack:** Rust, `fs4` crate for cross-platform `flock`

**Review fixes applied:**
- Lock scope expanded to cover entire operations (filesystem + tracking), not just tracking updates
- Added timeout (10s) with `try_lock_exclusive` and user-facing "waiting for lock" message
- `lock_path_for` uses `expect()` instead of `unwrap_or_default()` for path safety
- `move` closures to avoid unnecessary `.clone()` allocations
- `OpenOptions` instead of `File::create` to avoid unnecessary truncation
- Removed misleading "released file lock" debug log (lock releases on drop)
- Windows dirty-read caveat documented as accepted risk
- `.lock` files added to `.gitignore`

---

### Task 1: Add `fs4` dependency and `file_lock` module

**Files:**
- Modify: `Cargo.toml` (workspace)
- Modify: `crates/kiro-market-core/Cargo.toml`
- Create: `crates/kiro-market-core/src/file_lock.rs`
- Modify: `crates/kiro-market-core/src/lib.rs`

**Step 1: Add `fs4` to workspace dependencies**

In `Cargo.toml` (workspace root), add after the `# Platform` section:

```toml
# File locking
fs4 = "0.13"
```

In `crates/kiro-market-core/Cargo.toml`, add to `[dependencies]`:

```toml
fs4 = { workspace = true }
```

**Step 2: Create the `file_lock` module**

Create `crates/kiro-market-core/src/file_lock.rs`:

```rust
//! Advisory file locking for read-modify-write operations.
//!
//! Uses a `.lock` sibling file with OS-level advisory locks (`flock` on
//! Unix, `LockFile` on Windows) to prevent concurrent processes from
//! clobbering each other's writes to shared JSON files.
//!
//! **Windows caveat:** read-only calls (`load_installed`,
//! `load_known_marketplaces`) are not locked. On Linux/macOS the atomic
//! rename used by `atomic_write` is safe for concurrent readers, but on
//! Windows NTFS this is not guaranteed. This is accepted as a low-risk
//! edge case — the worst outcome is a transient read error, not data loss.

use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use fs4::fs_std::FileExt;
use tracing::debug;

/// How long to wait for a lock before giving up.
const LOCK_TIMEOUT: Duration = Duration::from_secs(10);

/// How long to sleep between lock attempts.
const LOCK_RETRY_INTERVAL: Duration = Duration::from_millis(100);

/// Execute a fallible closure while holding an exclusive advisory lock.
///
/// Creates a `.lock` sibling file next to `path` (e.g.
/// `installed-skills.json.lock`), acquires an exclusive lock with a
/// timeout, runs `f`, and releases the lock when the file handle is
/// dropped.
///
/// If the lock cannot be acquired within [`LOCK_TIMEOUT`], returns an
/// I/O error. A diagnostic message is logged after the first failed
/// attempt so the user knows what is happening.
///
/// # Errors
///
/// Returns an I/O error if the lock file cannot be created, the lock
/// times out, or `f` returns an error.
pub fn with_file_lock<T, E>(path: &Path, f: impl FnOnce() -> Result<T, E>) -> Result<T, E>
where
    E: From<io::Error>,
{
    let lock_path = lock_path_for(path);

    if let Some(parent) = lock_path.parent() {
        fs::create_dir_all(parent).map_err(E::from)?;
    }

    let lock_file = OpenOptions::new()
        .write(true)
        .create(true)
        .open(&lock_path)
        .map_err(E::from)?;

    acquire_lock_with_timeout(&lock_file, &lock_path).map_err(E::from)?;

    debug!(path = %lock_path.display(), "acquired file lock");

    // Lock is released when `lock_file` is dropped at the end of this
    // function, after `f()` returns.
    f()
}

/// Try to acquire an exclusive lock with a timeout.
///
/// Uses `try_lock_exclusive` in a retry loop. Logs a "waiting" message
/// after the first failed attempt so the user knows why the process is
/// blocking.
fn acquire_lock_with_timeout(file: &File, lock_path: &Path) -> io::Result<()> {
    // Fast path: try once without sleeping.
    if file.try_lock_exclusive().is_ok() {
        return Ok(());
    }

    debug!(
        path = %lock_path.display(),
        "lock held by another process, waiting..."
    );
    eprintln!(
        "Waiting for lock on {}...",
        lock_path.display()
    );

    let start = Instant::now();
    loop {
        thread::sleep(LOCK_RETRY_INTERVAL);

        if file.try_lock_exclusive().is_ok() {
            return Ok(());
        }

        if start.elapsed() >= LOCK_TIMEOUT {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!(
                    "timed out waiting for lock on {} after {}s",
                    lock_path.display(),
                    LOCK_TIMEOUT.as_secs()
                ),
            ));
        }
    }
}

/// Return the `.lock` sibling path for a given file.
///
/// Appends `.lock` to the full file name (not just the extension), so
/// `installed-skills.json` becomes `installed-skills.json.lock`.
///
/// # Panics
///
/// Panics if `path` has no file name component (e.g. root path `/`).
/// Callers always pass paths constructed from known-good components.
fn lock_path_for(path: &Path) -> std::path::PathBuf {
    let mut lock_name = path
        .file_name()
        .expect("lock target path must have a file name component")
        .to_os_string();
    lock_name.push(".lock");
    path.with_file_name(lock_name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lock_path_for_appends_lock_extension() {
        let path = Path::new("/home/user/.kiro/installed-skills.json");
        let lock = lock_path_for(path);
        assert_eq!(
            lock,
            Path::new("/home/user/.kiro/installed-skills.json.lock")
        );
    }

    #[test]
    #[should_panic(expected = "file name component")]
    fn lock_path_for_panics_on_root_path() {
        lock_path_for(Path::new("/"));
    }

    #[test]
    fn with_file_lock_creates_lock_file_and_runs_closure() {
        let dir = tempfile::tempdir().expect("tempdir");
        let data_path = dir.path().join("data.json");

        let result: io::Result<i32> = with_file_lock(&data_path, || Ok(42));

        assert_eq!(result.expect("should succeed"), 42);
        assert!(
            lock_path_for(&data_path).exists(),
            "lock file should exist on disk"
        );
    }

    #[test]
    fn with_file_lock_propagates_closure_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let data_path = dir.path().join("data.json");

        let result: io::Result<()> = with_file_lock(&data_path, || {
            Err(io::Error::new(io::ErrorKind::Other, "inner error"))
        });

        let err = result.expect_err("should propagate error");
        assert_eq!(err.to_string(), "inner error");
    }

    #[test]
    fn with_file_lock_creates_parent_directories() {
        let dir = tempfile::tempdir().expect("tempdir");
        let data_path = dir.path().join("nested").join("dir").join("data.json");

        let result: io::Result<()> = with_file_lock(&data_path, || Ok(()));

        result.expect("should succeed even with missing parent dirs");
        assert!(lock_path_for(&data_path).exists());
    }

    #[test]
    fn with_file_lock_serializes_concurrent_access() {
        use std::io::Write;
        use std::sync::{Arc, Barrier};

        // NOTE: This test validates locking across threads within one
        // process. Cross-process locking relies on the same OS flock
        // mechanism but is not tested here — it would require spawning
        // a child process with std::process::Command.

        let dir = tempfile::tempdir().expect("tempdir");
        let data_path = dir.path().join("counter.json");
        let counter_path = dir.path().join("counter.txt");

        fs::write(&counter_path, "0").expect("init");

        let barrier = Arc::new(Barrier::new(2));
        let mut handles = vec![];

        for _ in 0..2 {
            let dp = data_path.clone();
            let cp = counter_path.clone();
            let b = Arc::clone(&barrier);

            handles.push(thread::spawn(move || {
                b.wait();
                for _ in 0..50 {
                    let _: io::Result<()> = with_file_lock(&dp, || {
                        let val: i32 = fs::read_to_string(&cp)
                            .expect("read")
                            .trim()
                            .parse()
                            .expect("parse");
                        let mut f = File::create(&cp).expect("create");
                        write!(f, "{}", val + 1).expect("write");
                        Ok(())
                    });
                }
            }));
        }

        for h in handles {
            h.join().expect("thread join");
        }

        let final_val: i32 = fs::read_to_string(&counter_path)
            .expect("read")
            .trim()
            .parse()
            .expect("parse");
        assert_eq!(final_val, 100, "concurrent increments should not be lost");
    }
}
```

**Step 3: Register the module**

In `crates/kiro-market-core/src/lib.rs`, add:

```rust
pub mod file_lock;
```

**Step 4: Run tests**

Run: `cargo test -p kiro-market-core file_lock`
Expected: PASS (all 6 tests including concurrency and panic test)

**Step 5: Commit**

```bash
git add Cargo.toml Cargo.lock crates/kiro-market-core/Cargo.toml \
       crates/kiro-market-core/src/file_lock.rs \
       crates/kiro-market-core/src/lib.rs
git commit -m "feat(core): add with_file_lock helper using fs4 advisory locks"
```

---

### Task 2: Wrap `project.rs` operations with file lock (full scope)

**Files:**
- Modify: `crates/kiro-market-core/src/project.rs`

The lock must cover the **entire operation** — existence check, filesystem
mutation, and tracking update — not just the tracking update. This prevents
TOCTOU races where two processes both check `dir.exists()` before either
acquires the lock.

**Step 1: Wrap `write_skill` with lock covering the full tracking update**

`write_skill` is called after `install_skill` / `install_skill_force` have
already done their existence checks and filesystem setup. The tracking
update is the only concurrent-sensitive part here since directory creation
and file writing are idempotent. However, for `install_skill` the
`dir.exists()` check needs to be inside the lock to prevent two processes
from both passing the check and then both writing.

Refactor `install_skill` to move the existence check inside the lock:

```rust
pub fn install_skill(
    &self,
    name: &str,
    content: &str,
    meta: InstalledSkillMeta,
) -> crate::error::Result<()> {
    validation::validate_name(name)?;

    crate::file_lock::with_file_lock(&self.tracking_path(), move || {
        let dir = self.skill_dir(name);

        if dir.exists() {
            return Err(SkillError::AlreadyInstalled {
                name: name.to_owned(),
            }
            .into());
        }

        fs::create_dir_all(&dir)?;
        crate::cache::atomic_write(&dir.join(SKILL_MD), content.as_bytes())?;

        let mut installed = self.load_installed()?;
        installed.skills.insert(name.to_owned(), meta);
        self.write_tracking(&installed)
    })
}
```

**Step 2: Refactor `install_skill_force` to lock the full operation**

```rust
pub fn install_skill_force(
    &self,
    name: &str,
    content: &str,
    meta: InstalledSkillMeta,
) -> crate::error::Result<()> {
    validation::validate_name(name)?;

    crate::file_lock::with_file_lock(&self.tracking_path(), move || {
        let dir = self.skill_dir(name);

        if dir.exists() {
            debug!(name, "removing existing skill directory for force install");
            fs::remove_dir_all(&dir)?;
        }

        fs::create_dir_all(&dir)?;
        crate::cache::atomic_write(&dir.join(SKILL_MD), content.as_bytes())?;

        let mut installed = self.load_installed()?;
        installed.skills.insert(name.to_owned(), meta);
        self.write_tracking(&installed)
    })
}
```

**Step 3: Refactor `remove_skill` to lock the full operation**

```rust
pub fn remove_skill(&self, name: &str) -> crate::error::Result<()> {
    validation::validate_name(name)?;

    crate::file_lock::with_file_lock(&self.tracking_path(), move || {
        let dir = self.skill_dir(name);

        if !dir.exists() {
            return Err(SkillError::NotInstalled {
                name: name.to_owned(),
            }
            .into());
        }

        fs::remove_dir_all(&dir)?;

        let mut installed = self.load_installed()?;
        installed.skills.remove(name);
        self.write_tracking(&installed)
    })?;

    debug!(name, "skill removed");
    Ok(())
}
```

**Step 4: Remove the now-unused `write_skill` helper**

Since `install_skill` and `install_skill_force` now inline the filesystem
and tracking operations inside the lock closure, `write_skill` is dead code.
Remove it.

**Step 5: Run tests**

Run: `cargo test -p kiro-market-core`
Expected: All existing tests pass unchanged.

Run: `cargo clippy --workspace -- -D warnings`
Expected: Clean.

**Step 6: Commit**

```bash
git add crates/kiro-market-core/src/project.rs
git commit -m "feat(core): lock installed-skills.json for entire install/remove operations"
```

---

### Task 3: Wrap `cache.rs` operations with file lock (full scope)

**Files:**
- Modify: `crates/kiro-market-core/src/cache.rs`

**Step 1: Wrap `add_known_marketplace` with lock**

Use a `move` closure to avoid unnecessary `.clone()` on `entry`:

```rust
pub fn add_known_marketplace(&self, entry: KnownMarketplace) -> crate::error::Result<()> {
    validation::validate_name(&entry.name)?;

    crate::file_lock::with_file_lock(&self.registry_path(), move || {
        let mut entries = self.load_known_marketplaces()?;

        if entries.iter().any(|e| e.name == entry.name) {
            return Err(MarketplaceError::AlreadyRegistered { name: entry.name }.into());
        }

        entries.push(entry);
        self.write_registry(&entries)
    })
}
```

**Step 2: Wrap `remove_known_marketplace` with lock**

```rust
pub fn remove_known_marketplace(&self, name: &str) -> crate::error::Result<()> {
    crate::file_lock::with_file_lock(&self.registry_path(), move || {
        let mut entries = self.load_known_marketplaces()?;
        let before_len = entries.len();
        entries.retain(|e| e.name != name);

        if entries.len() == before_len {
            return Err(MarketplaceError::NotFound {
                name: name.to_owned(),
            }
            .into());
        }

        self.write_registry(&entries)
    })
}
```

**Step 3: Run tests**

Run: `cargo test -p kiro-market-core`
Expected: All existing tests pass.

Run: `cargo test --workspace`
Expected: Full suite passes.

Run: `cargo clippy --workspace -- -D warnings`
Expected: Clean.

**Step 4: Commit**

```bash
git add crates/kiro-market-core/src/cache.rs
git commit -m "feat(core): lock known_marketplaces.json for entire add/remove operations"
```

---

### Task 4: Add `.gitignore` for lock files and verify end-to-end

**Files:**
- Modify or create: `.gitignore` (if lock files would appear in tracked directories)

**Step 1: Check if `.lock` files could appear in tracked directories**

The two lock files are:
- `~/.local/share/kiro-market/known_marketplaces.json.lock` — in the user
  data directory, never in a git repo.
- `<project>/.kiro/installed-skills.json.lock` — inside the project's
  `.kiro/` directory, which may be git-tracked.

If `.kiro/` is tracked, add to the project's `.gitignore`:

```
# Advisory lock files from kiro-market
*.json.lock
```

If `.kiro/` is already in `.gitignore`, no action needed.

**Step 2: Run the full CI check**

```bash
cargo build --workspace
cargo test --workspace
cargo clippy --workspace -- -D warnings
```

Expected: All green.

**Step 3: Commit**

```bash
git add -A
git commit -m "chore: add .gitignore for lock files, verify end-to-end"
```

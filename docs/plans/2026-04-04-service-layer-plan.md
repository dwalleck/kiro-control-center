# Service Layer Refactor — Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Eliminate duplicated marketplace logic between CLI and Tauri by introducing a `GitBackend` trait, a `platform` module for cross-platform local links, and a `MarketplaceService` that owns all marketplace operations.

**Architecture:** The `GitBackend` trait abstracts git operations (clone/pull/verify). `GixCliBackend` implements it using the current gix+CLI hybrid. A `platform` module provides cross-platform `create_local_link`/`is_local_link`/`remove_local_link` functions. `MarketplaceService` stores the backend as `Box<dyn GitBackend>` (not a generic parameter — vtable cost is negligible relative to git I/O, and it simplifies all handler signatures). The service owns the marketplace lifecycle (add/remove/update/list), replacing the duplicated logic in both frontends. CLI and Tauri handlers become thin wrappers that call the service and format output.

**Tech Stack:** Rust 1.85, gix 0.81, thiserror, junction crate (Windows-only)

---

### Task 1: Define `GitBackend` trait and `CloneOptions`

**Files:**
- Modify: `crates/kiro-market-core/src/git.rs`

**Step 1: Add the trait and options struct**

At the top of `git.rs` (after the `use` block, before `SSH_CONNECT_TIMEOUT_SECS`), add:

```rust
/// Options for cloning a repository.
#[derive(Clone, Debug, Default)]
pub struct CloneOptions {
    /// Branch, tag, or SHA to check out after cloning.
    /// When `None`, a shallow clone (depth 1) is used to reduce transfer size.
    /// When `Some`, a full clone is performed followed by a checkout of the ref.
    pub git_ref: Option<String>,
}

/// Trait abstracting git operations for testability and backend swapping.
///
/// Implementations must be `Send + Sync` to support sharing across async
/// Tauri command handlers via `Arc`.
pub trait GitBackend: Send + Sync {
    /// Clone a remote repository into `dest`.
    fn clone_repo(&self, url: &str, dest: &Path, opts: &CloneOptions) -> Result<(), GitError>;

    /// Pull (fast-forward only) the default branch.
    fn pull_repo(&self, path: &Path) -> Result<(), GitError>;

    /// Verify the HEAD commit matches the expected SHA prefix.
    fn verify_sha(&self, path: &Path, expected_sha: &str) -> Result<(), GitError>;
}
```

**Design note:** `shallow` is not a separate field — it's derived from `git_ref` inside the implementation. When `git_ref` is `None`, clone is shallow. This avoids exposing a knob nobody turns independently.

**Step 2: Run tests**

Run: `cargo test -p kiro-market-core`
Expected: All existing tests pass (trait is just a definition, nothing uses it yet).

**Step 3: Commit**

```
refactor: define GitBackend trait and CloneOptions

Introduces the trait boundary that separates git operation contracts
from their implementation. No behavioral changes yet.
```

---

### Task 2: Implement `GixCliBackend`

**Files:**
- Modify: `crates/kiro-market-core/src/git.rs`

**Step 1: Create the struct and move free functions into `impl GitBackend`**

Add the struct after the trait definition:

```rust
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
```

Move `run_git` and `git_error_detail` into `impl GixCliBackend` as private methods (change `fn run_git(args, dir)` to `fn run_git(&self, args, dir)` and use `self.ssh_connect_timeout`).

Implement the trait:

```rust
impl GitBackend for GixCliBackend {
    fn clone_repo(&self, url: &str, dest: &Path, opts: &CloneOptions) -> Result<(), GitError> {
        // Move current clone_repo body here, using opts.git_ref and opts.shallow
    }

    fn pull_repo(&self, path: &Path) -> Result<(), GitError> {
        // Move current pull_repo body here
    }

    fn verify_sha(&self, path: &Path, expected_sha: &str) -> Result<(), GitError> {
        // Move current verify_sha body here
    }
}
```

**Step 2: Keep the old free functions as deprecated wrappers**

Replace the existing free functions with wrappers that call `GixCliBackend::default()`:

```rust
/// Clone a remote Git repository into `dest`.
///
/// Deprecated: use `GixCliBackend::default().clone_repo()` or a
/// `MarketplaceService` instead.
pub fn clone_repo(url: &str, dest: &Path, git_ref: Option<&str>) -> Result<(), GitError> {
    let opts = CloneOptions {
        git_ref: git_ref.map(ToOwned::to_owned),
    };
    GixCliBackend::default().clone_repo(url, dest, &opts)
}

pub fn pull_repo(path: &Path) -> Result<(), GitError> {
    GixCliBackend::default().pull_repo(path)
}

pub fn verify_sha(path: &Path, expected_sha: &str) -> Result<(), GitError> {
    GixCliBackend::default().verify_sha(path, expected_sha)
}
```

This keeps all existing callers working without changes. The wrappers get removed in Task 8.

**Step 3: Fix tests to use GixCliBackend directly**

Update the test module: tests should call `GixCliBackend::default().clone_repo(...)` etc. instead of the free functions. Both should work, but testing through the trait validates the implementation.

**Step 4: Run tests**

Run: `cargo test -p kiro-market-core`
Run: `cargo clippy --workspace -- -D warnings`
Expected: All pass.

**Step 5: Commit**

```
refactor: implement GixCliBackend behind GitBackend trait

Moves clone/pull/verify logic into GixCliBackend. Old free functions
remain as deprecated wrappers to avoid breaking callers. Tests updated
to exercise the trait implementation directly.
```

---

### Task 3: Add `platform` module for cross-platform local links

**Files:**
- Create: `crates/kiro-market-core/src/platform.rs`
- Modify: `crates/kiro-market-core/src/lib.rs`
- Modify: `crates/kiro-market-core/Cargo.toml` (add `junction` dependency)

**Step 1: Add the junction dependency (Windows-only)**

In workspace `Cargo.toml`, add under `[workspace.dependencies]`:

```toml
junction = "1"
```

In `crates/kiro-market-core/Cargo.toml`, add under `[dependencies]`:

```toml
[target.'cfg(windows)'.dependencies]
junction = { workspace = true }
```

**Step 2: Create `platform.rs`**

```rust
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
pub fn create_local_link(src: &Path, dest: &Path) -> io::Result<()> {
    sys::create_local_link(src, dest)
}

/// Check whether `path` is a local link (symlink or directory junction).
pub fn is_local_link(path: &Path) -> bool {
    sys::is_local_link(path)
}

/// Remove a local link without removing the target contents.
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
        // Try directory junction first (works without admin on NTFS).
        match junction::create(src, dest) {
            Ok(()) => return Ok(()),
            Err(e) => {
                tracing::debug!(
                    error = %e,
                    "junction failed, falling back to directory copy"
                );
            }
        }

        // Fallback: copy the directory tree.
        copy_dir_recursive(src, dest)?;
        tracing::warn!(
            src = %src.display(),
            dest = %dest.display(),
            "used directory copy instead of junction — local changes will NOT be live-tracked"
        );
        Ok(())
    }

    pub fn is_local_link(path: &Path) -> bool {
        // is_symlink() returns true for both symlinks and junctions on Windows.
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
```

**Step 3: Register the module in `lib.rs`**

Add `pub mod platform;` after `pub mod plugin;`.

**Step 4: Add platform module tests**

Add a `#[cfg(test)] mod tests` at the bottom of `platform.rs`:

```rust
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

        assert!(is_local_link(&dest), "dest should be detected as a link");
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
        remove_local_link(&dest).expect("remove link");

        assert!(!dest.exists(), "link should be gone");
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
```

**Important:** On Windows, the junction fallback may trigger `copy_dir_recursive` instead of creating a true link. The `is_local_link` test will return `false` for copies, which means `create_and_detect_local_link` may fail on some Windows configurations (non-NTFS, no junction support). Add a note to the test acknowledging this and skip if `is_local_link` returns false after creation:

```rust
    // On Windows, if junctions aren't supported, create_local_link falls
    // back to a directory copy. In that case is_local_link returns false
    // and the test still passes — we just verify the content is accessible.
```

**Step 5: Handle junction preconditions on Windows**

The `junction::create` function requires:
- `src` must be an absolute path
- `dest` must not already exist

Add canonicalization of `src` before calling `junction::create` in the Windows implementation:

```rust
    pub fn create_local_link(src: &Path, dest: &Path) -> io::Result<()> {
        // Junctions require absolute source paths.
        let src = std::fs::canonicalize(src)?;
        // ...
    }
```

**Step 6: Run tests**

Run: `cargo test -p kiro-market-core platform`
Run: `cargo clippy --workspace -- -D warnings`
Expected: All pass.

**Step 7: Commit**

```
feat: add platform module for cross-platform local marketplace links

Unix uses symlinks, Windows uses directory junctions with copy
fallback. Includes unit tests for link creation, detection, and
removal. Replaces scattered #[cfg(unix)] blocks in UI-layer code.
```

---

### Task 4: Add `MarketplaceService` with `add` method

**Files:**
- Create: `crates/kiro-market-core/src/service.rs`
- Modify: `crates/kiro-market-core/src/lib.rs`

This is the largest task. The `add` method contains the most complex logic.

**Step 1: Create `service.rs` with result types and the `add` method**

```rust
//! Marketplace lifecycle operations.
//!
//! [`MarketplaceService`] owns the add/remove/update/list logic that was
//! previously duplicated between the CLI and Tauri frontends.

use std::fs;
use std::path::PathBuf;

use tracing::{debug, warn};

use crate::cache::{CacheDir, KnownMarketplace, MarketplaceSource};
use crate::error::Error;
use crate::git::{GitBackend, GitProtocol};
use crate::marketplace::Marketplace;
use crate::{platform, validation};

// ---------------------------------------------------------------------------
// Result types
// ---------------------------------------------------------------------------

/// Result of adding a new marketplace.
#[derive(Clone, Debug, serde::Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct MarketplaceAddResult {
    pub name: String,
    pub plugins: Vec<PluginBasicInfo>,
}

/// Basic information about a plugin within a marketplace.
#[derive(Clone, Debug, serde::Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct PluginBasicInfo {
    pub name: String,
    pub description: Option<String>,
}

/// Result of updating one or more marketplaces.
#[derive(Clone, Debug, Default, serde::Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct UpdateResult {
    pub updated: Vec<String>,
    pub failed: Vec<FailedUpdate>,
    pub skipped: Vec<String>,
}

/// A marketplace that failed to update, with the reason.
#[derive(Clone, Debug, serde::Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct FailedUpdate {
    pub name: String,
    pub error: String,
}

**Design note:** `serde::Serialize` is unconditional (Tauri needs it for IPC). Only `specta::Type` is feature-gated.

// ---------------------------------------------------------------------------
// Service
// ---------------------------------------------------------------------------

/// Manages the marketplace lifecycle: add, remove, update, list.
///
/// Uses `Box<dyn GitBackend>` rather than a generic parameter to keep
/// handler signatures clean. The vtable cost is negligible relative to
/// git I/O.
pub struct MarketplaceService {
    cache: CacheDir,
    git: Box<dyn GitBackend>,
}

impl MarketplaceService {
    pub fn new(cache: CacheDir, git: impl GitBackend + 'static) -> Self {
        Self { cache, git: Box::new(git) }
    }

    /// Add a new marketplace source.
    ///
    /// 1. Detect source type (GitHub, git URL, local path).
    /// 2. Clone or link into a temp directory in the cache.
    /// 3. Read the marketplace manifest to discover the canonical name.
    /// 4. Validate the name, rename to final location.
    /// 5. Register in `known_marketplaces.json`.
    pub fn add(
        &self,
        source: &str,
        protocol: GitProtocol,
    ) -> Result<MarketplaceAddResult, Error> {
        let ms = MarketplaceSource::detect(source);
        self.cache.ensure_dirs()?;

        let temp_name = format!("_pending_{}", std::process::id());
        let temp_dir = self.cache.marketplace_path(&temp_name);

        // Clean up any leftover temp directory from a prior interrupted run.
        if temp_dir.exists() {
            fs::remove_dir_all(&temp_dir).map_err(|e| {
                std::io::Error::new(e.kind(), format!("failed to clean up {}: {e}", temp_dir.display()))
            })?;
        }

        // Clone or link based on source type.
        let clone_result = self.clone_or_link(&ms, protocol, &temp_dir);
        if let Err(e) = clone_result {
            if let Err(e) = fs::remove_dir_all(&temp_dir) {
                warn!(path = %temp_dir.display(), error = %e, "failed to clean up temp directory");
            }
            return Err(e);
        }

        // Read marketplace manifest.
        let manifest_path = temp_dir.join(crate::MARKETPLACE_MANIFEST_PATH);
        let manifest_bytes = match fs::read(&manifest_path) {
            Ok(bytes) => bytes,
            Err(e) => {
                if let Err(e) = fs::remove_dir_all(&temp_dir) {
                warn!(path = %temp_dir.display(), error = %e, "failed to clean up temp directory");
            }
                return Err(crate::error::MarketplaceError::ManifestNotFound {
                    path: manifest_path,
                }.into());
            }
        };

        let manifest = match Marketplace::from_json(&manifest_bytes) {
            Ok(m) => m,
            Err(e) => {
                if let Err(e) = fs::remove_dir_all(&temp_dir) {
                warn!(path = %temp_dir.display(), error = %e, "failed to clean up temp directory");
            }
                return Err(crate::error::MarketplaceError::InvalidManifest {
                    reason: e.to_string(),
                }.into());
            }
        };

        let name = manifest.name.clone();

        if let Err(e) = validation::validate_name(&name) {
            if let Err(e) = fs::remove_dir_all(&temp_dir) {
                warn!(path = %temp_dir.display(), error = %e, "failed to clean up temp directory");
            }
            return Err(e.into());
        }

        // Rename temp dir to final location.
        let final_dir = self.cache.marketplace_path(&name);
        if final_dir.exists() {
            if let Err(e) = fs::remove_dir_all(&temp_dir) {
                warn!(path = %temp_dir.display(), error = %e, "failed to clean up temp directory");
            }
            return Err(crate::error::MarketplaceError::AlreadyRegistered {
                name: name.clone(),
            }.into());
        }

        fs::rename(&temp_dir, &final_dir).map_err(std::io::Error::from)?;

        // Register in known_marketplaces.json.
        let entry = KnownMarketplace {
            name: name.clone(),
            source: ms,
            protocol: Some(protocol),
            added_at: chrono::Utc::now(),
        };
        self.cache.add_known_marketplace(entry)?;

        let plugins = manifest
            .plugins
            .iter()
            .map(|p| PluginBasicInfo {
                name: p.name.clone(),
                description: p.description.clone(),
            })
            .collect();

        debug!(marketplace = %name, "marketplace added");

        Ok(MarketplaceAddResult { name, plugins })
    }

    fn clone_or_link(
        &self,
        ms: &MarketplaceSource,
        protocol: GitProtocol,
        dest: &std::path::Path,
    ) -> Result<(), Error> {
        match ms {
            MarketplaceSource::GitHub { repo } => {
                let url = crate::git::github_repo_to_url(repo, protocol);
                debug!(url = %url, dest = %dest.display(), "cloning GitHub marketplace");
                let opts = crate::git::CloneOptions::default();
                self.git.clone_repo(&url, dest, &opts)?;
            }
            MarketplaceSource::GitUrl { url } => {
                if protocol != GitProtocol::default() {
                    warn!("protocol parameter ignored for full git URL; the URL's own scheme is used");
                }
                debug!(url = %url, dest = %dest.display(), "cloning git marketplace");
                let opts = crate::git::CloneOptions::default();
                self.git.clone_repo(url, dest, &opts)?;
            }
            MarketplaceSource::LocalPath { path } => {
                let src = crate::cache::resolve_local_path(path)?;
                debug!(src = %src.display(), dest = %dest.display(), "linking local marketplace");
                platform::create_local_link(&src, dest)?;
            }
        }
        Ok(())
    }

    /// Remove a registered marketplace and its cached data.
    pub fn remove(&self, name: &str) -> Result<(), Error> {
        self.cache.remove_known_marketplace(name)?;

        let mp_path = self.cache.marketplace_path(name);
        if platform::is_local_link(&mp_path) {
            platform::remove_local_link(&mp_path)?;
        } else if mp_path.exists() {
            fs::remove_dir_all(&mp_path)?;
        }

        debug!(marketplace = %name, "marketplace removed");
        Ok(())
    }

    /// Update marketplace clone(s) from remote.
    ///
    /// If `name` is provided, only that marketplace is updated. Locally
    /// linked marketplaces are skipped since they always reflect disk state.
    pub fn update(&self, name: Option<&str>) -> Result<UpdateResult, Error> {
        let entries = self.cache.load_known_marketplaces()?;

        if entries.is_empty() {
            return Ok(UpdateResult::default());
        }

        let targets: Vec<_> = if let Some(filter_name) = name {
            let filtered: Vec<_> = entries.iter().filter(|e| e.name == *filter_name).collect();
            if filtered.is_empty() {
                return Err(crate::error::MarketplaceError::NotFound {
                    name: filter_name.to_owned(),
                }.into());
            }
            filtered
        } else {
            entries.iter().collect()
        };

        let mut result = UpdateResult::default();

        for entry in &targets {
            let mp_path = self.cache.marketplace_path(&entry.name);

            // Skip locally linked marketplaces — they always reflect disk state.
            // Also skip if the directory is not a git repo (e.g. copy-fallback
            // on Windows where junctions weren't available).
            if platform::is_local_link(&mp_path) {
                debug!(marketplace = %entry.name, "skipping local marketplace");
                result.skipped.push(entry.name.clone());
                continue;
            }

            if matches!(entry.source, MarketplaceSource::LocalPath { .. }) {
                debug!(marketplace = %entry.name, "skipping local marketplace (directory copy)");
                result.skipped.push(entry.name.clone());
                continue;
            }

            match self.git.pull_repo(&mp_path) {
                Ok(()) => {
                    debug!(marketplace = %entry.name, "marketplace updated");
                    result.updated.push(entry.name.clone());
                }
                Err(e) => {
                    warn!(marketplace = %entry.name, error = %e, "failed to update");
                    result.failed.push(FailedUpdate {
                        name: entry.name.clone(),
                        error: e.to_string(),
                    });
                }
            }
        }

        Ok(result)
    }

    /// List all registered marketplaces.
    pub fn list(&self) -> Result<Vec<KnownMarketplace>, Error> {
        Ok(self.cache.load_known_marketplaces()?)
    }
}
```

**Step 2: Register in `lib.rs`**

Add `pub mod service;` after `pub mod skill;`.

**Step 3: Run tests**

Run: `cargo test -p kiro-market-core`
Run: `cargo clippy --workspace -- -D warnings`
Expected: All pass.

**Step 4: Commit**

```
feat: add MarketplaceService with add/remove/update/list

Centralizes marketplace lifecycle logic that was previously duplicated
between CLI and Tauri frontends. Uses GitBackend trait for git ops and
platform module for cross-platform local links.
```

---

### Task 5: Add service unit tests with mock backend

**Files:**
- Modify: `crates/kiro-market-core/src/service.rs` (add test module)

**Step 1: Add a mock backend and service tests**

Add a `#[cfg(test)] mod tests` at the bottom of `service.rs`:

```rust
#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::cache::CacheDir;
    use crate::git::{CloneOptions, GitError};

    /// Records which git operations were called.
    #[derive(Debug, Default)]
    struct MockGitBackend {
        calls: Mutex<Vec<String>>,
    }

    impl GitBackend for MockGitBackend {
        fn clone_repo(&self, url: &str, dest: &Path, _opts: &CloneOptions) -> Result<(), GitError> {
            self.calls.lock().unwrap().push(format!("clone:{url}"));
            // Create the dest directory so the service can write into it.
            fs::create_dir_all(dest).unwrap();
            // Write a minimal marketplace manifest.
            let mp_dir = dest.join(".claude-plugin");
            fs::create_dir_all(&mp_dir).unwrap();
            fs::write(
                mp_dir.join("marketplace.json"),
                r#"{"name":"mock-market","owner":{"name":"Test"},"plugins":[{"name":"mock-plugin","source":"./plugins/mock"}]}"#,
            ).unwrap();
            Ok(())
        }

        fn pull_repo(&self, path: &Path) -> Result<(), GitError> {
            self.calls.lock().unwrap().push(format!("pull:{}", path.display()));
            Ok(())
        }

        fn verify_sha(&self, _path: &Path, _expected: &str) -> Result<(), GitError> {
            Ok(())
        }
    }

    fn temp_service() -> (tempfile::TempDir, MarketplaceService) {
        let dir = tempfile::tempdir().expect("tempdir");
        let cache = CacheDir::with_root(dir.path().to_path_buf());
        cache.ensure_dirs().expect("ensure_dirs");
        let svc = MarketplaceService::new(cache, MockGitBackend::default());
        (dir, svc)
    }

    #[test]
    fn add_marketplace_registers_and_returns_plugins() {
        let (_dir, svc) = temp_service();
        let result = svc.add("owner/repo", GitProtocol::Https).expect("add");

        assert_eq!(result.name, "mock-market");
        assert_eq!(result.plugins.len(), 1);
        assert_eq!(result.plugins[0].name, "mock-plugin");

        let known = svc.list().expect("list");
        assert_eq!(known.len(), 1);
        assert_eq!(known[0].name, "mock-market");
    }

    #[test]
    fn add_duplicate_marketplace_returns_error() {
        let (_dir, svc) = temp_service();
        svc.add("owner/repo", GitProtocol::Https).expect("first add");
        let err = svc.add("owner/repo", GitProtocol::Https).expect_err("duplicate");
        assert!(err.to_string().contains("already"), "got: {err}");
    }

    #[test]
    fn remove_marketplace_cleans_up() {
        let (_dir, svc) = temp_service();
        svc.add("owner/repo", GitProtocol::Https).expect("add");
        svc.remove("mock-market").expect("remove");

        let known = svc.list().expect("list");
        assert!(known.is_empty());
    }

    #[test]
    fn update_calls_pull_on_cloned_repos() {
        let (_dir, svc) = temp_service();
        svc.add("owner/repo", GitProtocol::Https).expect("add");

        let result = svc.update(None).expect("update");
        assert_eq!(result.updated.len(), 1);
        assert_eq!(result.updated[0], "mock-market");
    }

    #[test]
    fn update_nonexistent_returns_error() {
        let (_dir, svc) = temp_service();
        let err = svc.update(Some("nope")).expect_err("should fail");
        assert!(err.to_string().contains("not found"), "got: {err}");
    }

    #[test]
    fn list_empty_returns_empty_vec() {
        let (_dir, svc) = temp_service();
        let known = svc.list().expect("list");
        assert!(known.is_empty());
    }
}
```

**Step 2: Run tests**

Run: `cargo test -p kiro-market-core service`
Expected: All 6 new tests pass.

**Step 3: Commit**

```
test: add MarketplaceService unit tests with mock git backend

Tests add/remove/update/list lifecycle using MockGitBackend.
No filesystem git repos needed — validates orchestration logic only.
```

---

### Task 6: Migrate CLI marketplace handler to use service

**Files:**
- Modify: `crates/kiro-market/src/commands/marketplace.rs`

**Step 1: Rewrite the handler**

Replace the entire file with a thin wrapper around `MarketplaceService`:

```rust
//! `marketplace` subcommand: add, list, update, and remove marketplace sources.

use anyhow::{Context, Result};
use colored::Colorize;
use kiro_market_core::cache::CacheDir;
use kiro_market_core::git::{GitProtocol, GixCliBackend};
use kiro_market_core::service::MarketplaceService;

use crate::cli::MarketplaceAction;

/// Dispatch to the appropriate marketplace subcommand.
pub fn run(action: &MarketplaceAction) -> Result<()> {
    let cache = CacheDir::default_location()
        .context("could not determine data directory; is $HOME set?")?;
    let git = GixCliBackend::default();
    let svc = MarketplaceService::new(cache, git);

    match action {
        MarketplaceAction::Add { source, protocol } => add(&svc, source, *protocol),
        MarketplaceAction::List => list(&svc),
        MarketplaceAction::Update { name } => update(&svc, name.as_deref()),
        MarketplaceAction::Remove { name } => remove(&svc, name),
    }
}

fn add(svc: &MarketplaceService, source: &str, protocol: GitProtocol) -> Result<()> {
    print!("  Adding marketplace...");
    let result = svc.add(source, protocol)
        .context("failed to add marketplace")?;

    println!(
        " {} Added {} ({} plugin{})",
        "✓".green().bold(),
        result.name.bold(),
        result.plugins.len(),
        if result.plugins.len() == 1 { "" } else { "s" }
    );

    if !result.plugins.is_empty() {
        println!();
        println!("  {}", "Available plugins:".bold());
        for plugin in &result.plugins {
            let desc = plugin.description.as_deref().unwrap_or("(no description)");
            println!("    {} - {}", plugin.name.green(), desc);
        }
        println!();
        println!(
            "  Install with: {}",
            format!("kiro-market install <plugin>@{}", result.name).bold()
        );
    }

    Ok(())
}

fn list(svc: &MarketplaceService) -> Result<()> {
    let entries = svc.list().context("failed to load marketplaces")?;

    if entries.is_empty() {
        println!(
            "No marketplaces registered. Use {} to add one.",
            "kiro-market marketplace add".bold()
        );
        return Ok(());
    }

    println!("{}", "Registered marketplaces:".bold());
    for entry in &entries {
        println!("  {} ({})", entry.name.green().bold(), entry.source.label());
    }

    Ok(())
}

fn update(svc: &MarketplaceService, name: Option<&str>) -> Result<()> {
    let result = svc.update(name).context("failed to update marketplaces")?;

    for name in &result.skipped {
        println!("  {} {} (local, skipped)", "·".bold(), name.bold());
    }
    for name in &result.updated {
        println!("  {} {} {}", "✓".green().bold(), name.bold(), "done".green());
    }
    for fail in &result.failed {
        println!("  {} {} {}", "✗".red().bold(), fail.name.bold(), fail.error.red());
    }

    if !result.failed.is_empty() {
        anyhow::bail!(
            "{} marketplace{} failed to update",
            result.failed.len(),
            if result.failed.len() == 1 { "" } else { "s" }
        );
    }

    Ok(())
}

fn remove(svc: &MarketplaceService, name: &str) -> Result<()> {
    svc.remove(name)
        .with_context(|| format!("failed to remove marketplace '{name}'"))?;
    println!("{} Removed marketplace {}", "✓".green().bold(), name.bold());
    Ok(())
}
```

**Step 2: Run CLI tests**

Run: `cargo test -p kiro-market`
Expected: All tests pass (including the workflow tests).

**Step 3: Commit**

```
refactor: migrate CLI marketplace handler to MarketplaceService

CLI handler is now a thin wrapper: constructs the service, calls it,
formats output. All domain logic lives in kiro-market-core::service.
```

---

### Task 7: Migrate Tauri marketplace handler to use service

**Files:**
- Modify: `crates/kiro-control-center/src-tauri/src/commands/marketplaces.rs`

**Step 1: Rewrite the Tauri handler**

Replace the file with thin wrappers that call `MarketplaceService`. Remove the duplicated result types (now imported from `kiro_market_core::service`). The Tauri commands construct `GixCliBackend` and `MarketplaceService` inline, then map `Error` to `CommandError`.

The key change: remove `MarketplaceAddResult`, `PluginBasicInfo`, `UpdateResult`, `FailedUpdate` struct definitions from this file — they're now in `kiro_market_core::service`.

**Step 2: Run Tauri tests**

Run: `cargo test -p kiro-control-center`
Expected: All pass.

**Step 3: Run full suite**

Run: `cargo test --workspace`
Run: `cargo clippy --workspace -- -D warnings`
Expected: All pass.

**Step 4: Commit**

```
refactor: migrate Tauri marketplace handler to MarketplaceService

Tauri handlers are now thin wrappers like CLI. Result types imported
from kiro_market_core::service. Duplicate logic eliminated.
```

---

### Task 8: Remove deprecated free function wrappers

**Files:**
- Modify: `crates/kiro-market-core/src/git.rs`
- Modify: `crates/kiro-market/src/commands/install.rs` (update to use `GixCliBackend`)
- Modify: `crates/kiro-control-center/src-tauri/src/commands/browse.rs` (if it calls git functions)

**Step 1: Update remaining callers**

Search for any remaining calls to `git::clone_repo`, `git::pull_repo`, `git::verify_sha` free functions. Update them to use `GixCliBackend` directly.

The main caller is `install.rs` which calls `git::clone_repo` and `git::verify_sha` for plugin cloning. Update `install.rs` to construct a `GixCliBackend::default()` and call through it:

```rust
let git = GixCliBackend::default();
let opts = CloneOptions { git_ref: git_ref.map(ToOwned::to_owned) };
git.clone_repo(&url, &dest, &opts)?;
// ...
git.verify_sha(&dest, expected)?;
```

The Tauri `browse.rs` does NOT call git functions (it reads manifests from disk), so no changes needed there.

**Step 2: Remove the deprecated wrappers from `git.rs`**

Delete the `pub fn clone_repo(...)`, `pub fn pull_repo(...)`, `pub fn verify_sha(...)` wrapper functions.

**Step 3: Run full test suite**

Run: `cargo test --workspace`
Run: `cargo clippy --workspace -- -D warnings`
Expected: All pass.

**Step 4: Commit**

```
refactor: remove deprecated git free function wrappers

All callers now use GixCliBackend directly or through MarketplaceService.
The free functions served as a bridge during migration and are no longer
needed.
```

---

### Task 9: Update CLAUDE.md and final verification

**Files:**
- Modify: `CLAUDE.md`

**Step 1: Update architecture section**

Add a note about the service layer pattern and `GitBackend` trait.

**Step 2: Run full verification**

```bash
cargo fmt --all --check
cargo clippy --workspace -- -D warnings
cargo test --workspace
```

**Step 3: Commit**

```
docs: update CLAUDE.md with service layer architecture
```

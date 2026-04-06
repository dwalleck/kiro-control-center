# Tracking File Locking — Design

## Problem

`installed-skills.json` and `known_marketplaces.json` use unsynchronized
read-modify-write cycles. The CLI and Tauri Control Center can run concurrently
and clobber each other's writes — one process reads, modifies, and writes back,
overwriting changes the other process made in between.

## Solution

Wrap each read-modify-write cycle in an advisory file lock using a `.lock`
sibling file. Use the `fs4` crate (pure Rust, cross-platform fork of `fs2`)
for `lock_exclusive()`.

## Locking helper

Add a `file_lock` module to `kiro-market-core` with a function that acquires
an exclusive lock on a `.lock` sibling file, runs a closure, and releases
the lock on drop:

```rust
pub fn with_file_lock<T, E>(path: &Path, f: impl FnOnce() -> Result<T, E>) -> Result<T, E>
where
    E: From<io::Error>,
```

The lock file (e.g. `installed-skills.json.lock`) is created if it doesn't
exist. Parent directories are created if needed. The lock is blocking — if
another process holds it, the caller waits.

## Where locks are acquired

### `project.rs` — `installed-skills.json`

- `write_skill` (called by `install_skill` / `install_skill_force`):
  lock around `load_installed()` → insert → `write_tracking()`
- `remove_skill`: lock around `load_installed()` → remove → `write_tracking()`

### `cache.rs` — `known_marketplaces.json`

- `add_known_marketplace`: lock around `load_known_marketplaces()` → push → `write_registry()`
- `remove_known_marketplace`: lock around `load_known_marketplaces()` → retain → `write_registry()`

## What doesn't change

- Read-only calls (`load_installed`, `load_known_marketplaces`) — no lock needed
- Atomic write pattern (temp file + rename) — still used within the locked section
- JSON file format — no schema changes

## Lock file behavior

- Advisory: processes that don't use the lock can still read/write
- Auto-releases on process exit or crash (OS handles cleanup)
- `.lock` files persist on disk but are harmless (tiny, empty)
- Blocking: caller waits if another process holds the lock

## Dependencies

- `fs4` v0.13 — pure Rust, cross-platform file locking (successor to `fs2`)

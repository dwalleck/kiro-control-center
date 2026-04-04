> **Status: RESOLVED** — All critical, important, and suggested issues below were fixed in commits `6342c5b` through `e6f58b6` on the `feature/use-gix` branch. This document is retained for historical context only.

# PR Review: git2 to gix Migration

**Branch:** `feature/use-gix`
**Date:** 2026-04-03
**Reviewers:** code-reviewer, silent-failure-hunter, comment-analyzer, test-analyzer, type-design-analyzer, code-simplifier

---

## Critical Issues (1 found)

### 1. `pull_repo` is fundamentally broken — fetch never advances HEAD

**Files:** `crates/kiro-market-core/src/git.rs:138-199`
**Found by:** code-reviewer, silent-failure-hunter, comment-analyzer

The `gix` `receive()` call downloads objects and updates remote tracking refs (e.g., `refs/remotes/origin/main`), but does **not** advance the local branch ref or HEAD. The subsequent `git checkout --force HEAD` re-checks out the **current** HEAD — which was never moved — making the entire pull a no-op.

**Impact:** Every `kiro-market marketplace update` will report success while silently leaving the cache stale. Users will never receive updated plugins.

**Fix options:**
- Replace `git checkout --force HEAD` with `git merge --ff-only @{upstream}` (closest to old behavior)
- Use `git reset --hard FETCH_HEAD`
- Simplify the entire function to just `git pull --ff-only` since it already shells out to `git` anyway

---

## Important Issues (4 found)

### 2. SSH connect timeout protection removed with no replacement

**Files:** `crates/kiro-market/src/main.rs`, `crates/kiro-control-center/src-tauri/src/main.rs`, `crates/kiro-market-core/src/git.rs`
**Found by:** code-reviewer, silent-failure-hunter

The old `git2::opts::set_server_connect_timeout_in_milliseconds(30_000)` prevented infinite hangs when SSH port 22 is firewalled. This was deliberately added and is now deleted with no `gix` equivalent configured. Users behind corporate firewalls will experience indefinite hangs.

**Fix:** Configure `gix`'s transport connect timeout, or wrap `Command::new("git")` calls with a timeout mechanism, or use `GIT_SSH_COMMAND` with `ConnectTimeout`.

### 3. Missing `git` binary produces misleading error messages

**Files:** `crates/kiro-market-core/src/git.rs:80-90, 181-197`
**Found by:** silent-failure-hunter

If `git` is not in `$PATH`, `Command::new("git").output()` returns `io::Error(NotFound)` which maps to `"failed to clone https://...: No such file or directory"` — sounds like the URL is wrong, not that `git` is missing. The old `git2` implementation had zero dependency on a system `git` binary.

**Fix:** Detect `io::ErrorKind::NotFound` and produce a clear message like `"the 'git' command-line tool is required but was not found in PATH"`. Consider a shared `run_git` helper for both call sites.

### 4. Missing `--` separator before `refname` allows option injection

**Files:** `crates/kiro-market-core/src/git.rs:80-90`
**Found by:** silent-failure-hunter

The `refname` from marketplace manifests is passed directly to `git checkout refname`. A crafted manifest with `git_ref: "--orphan=malicious"` would be interpreted as flags.

**Fix:** Add `--` before the refname:
```rust
.args(["checkout", "--", refname])
```

### 5. CLAUDE.md still lists `git2` as a dependency

**Files:** `CLAUDE.md:40`
**Found by:** code-reviewer, comment-analyzer

The "Key Crate Dependencies" section says `git2 — git clone/pull operations`. This is now stale and will mislead contributors and AI tools.

**Fix:** Replace with `gix — git clone/pull operations`.

---

## Suggestions (10 found)

### Documentation / Comments

6. **Module doc says "all Git interactions" use gix** — inaccurate since code also shells out to `git` CLI. (`git.rs:1-4`)
7. **`pull_repo` doc says "fetch + reset"** — code does `checkout`, not `reset`. Also says "from `origin`" but code uses `find_default_remote`. (`git.rs:128`)
8. **`clone_repo` doc doesn't mention `git` CLI dependency** when `git_ref` is provided. (`git.rs:42-47`)
9. **`# Panics` section is misleading** — says "Cannot panic" but refers to only one call; consider removing it and using `NonZeroU32::MIN` instead. (`git.rs:53-55`)

### Code Quality

10. **Redundant length check in `verify_sha`** — `expected_sha.len() <= actual_sha.len()` is always true when `starts_with` returns true. (`git.rs:118`)
11. **Extract `pull_err` helper in `pull_repo`** to match `clone_repo`'s existing `map_err` pattern. Six near-identical closures could be one. (`git.rs:141-178`)
12. **Use `NonZeroU32::MIN`** instead of `NonZeroU32::new(1).expect("1 is non-zero")` — stable since Rust 1.79, project targets 1.85. (`git.rs:67`)

### Test Coverage Gaps

13. **No test for `clone_repo` with `git_ref`** — the `Command`-based checkout path is untested. (Criticality 9/10)
14. **No happy-path test for `pull_repo`** — only the negative case (non-repo) is tested. (Criticality 8/10)
15. **No test for `clone_repo` with invalid `git_ref`** — error capture from stderr is unverified. (Criticality 7/10)

---

## Strengths

- **Backend decoupling** — `git2` is completely removed from all downstream crates' dependency trees. The error type is now a proper abstraction boundary.
- **Unsafe code eliminated** — Two `unsafe` blocks removed from both binary crates.
- **API simplified** — `clone_repo` returns `()` instead of leaking a `Repository` handle; `verify_sha` takes `&Path` for self-contained verification.
- **Shallow clones** — New `depth=1` shallow clone when no `git_ref` is specified reduces transfer size.
- **Error types well-structured** — `Box<dyn Error + Send + Sync>` is the right trade-off for library-agnostic errors, consistent with `thiserror` patterns.

---

## Action Plan

1. **Fix the critical `pull_repo` bug** — marketplace updates silently do nothing
2. **Add `--` separator** before refname in `git checkout` calls
3. **Add connect timeout** configuration for `gix` or document the regression
4. **Improve `git` binary error messaging** — detect `ErrorKind::NotFound` specifically
5. **Update CLAUDE.md** — replace `git2` with `gix` in Key Crate Dependencies
6. **Add missing tests** — especially for `git_ref` checkout path and `pull_repo` happy path
7. **Apply code simplifications** — `NonZeroU32::MIN`, redundant length check, `pull_err` helper
8. **Fix documentation** — module doc, `pull_repo` doc, `clone_repo` doc accuracy
9. **Re-run review after fixes** to verify issues are resolved

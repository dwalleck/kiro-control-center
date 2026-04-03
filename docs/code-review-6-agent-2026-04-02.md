# 6-Agent Comprehensive PR Review

**Date:** 2026-04-02
**PR:** #1 — feat/initial-implementation
**Reviewers:** 6 specialized agents (code-reviewer, test-analyzer, silent-failure-hunter, type-design-analyzer, comment-analyzer, code-simplifier)

---

## Must-Fix Before Merge (Critical)

### 1. Marketplace name used in fs::rename BEFORE validation
**Source:** code-reviewer
**Location:** `crates/kiro-market/src/commands/marketplace.rs:133-146`
**Issue:** The marketplace name from untrusted `marketplace.json` is used in `fs::rename` before `validate_name` is called (which happens later in `add_known_marketplace`). A malicious manifest with `name: "../escape"` would move the cloned directory outside the cache.
**Fix:** Call `validate_name(&name)?` immediately after reading the manifest name, before any filesystem path construction.

### 2. String-matching error classification
**Source:** error-hunter, code-reviewer, code-simplifier
**Location:** `crates/kiro-market/src/commands/install.rs:261`
**Issue:** `msg.contains("already installed")` classifies errors by matching display strings. If the error message wording changes, this silently breaks. Any error containing "already installed" in its chain would be misclassified.
**Fix:** Use `e.downcast_ref::<kiro_market_core::error::Error>()` and match on `Error::Skill(SkillError::AlreadyInstalled { .. })`.

### 3. `discover_skill_dirs` doesn't validate skill paths from plugin.json
**Source:** type-design-analyzer
**Location:** `crates/kiro-market-core/src/plugin.rs:59`
**Issue:** Untrusted `skills` array entries from `plugin.json` are joined to filesystem paths without calling `validate_relative_path`. A malicious plugin.json with `"../../etc/passwd"` would escape the plugin root.
**Fix:** Call `validate_relative_path` on each skill path before `plugin_root.join()`.

### 4. `filter_map(Result::ok)` silently discards filesystem errors
**Source:** error-hunter
**Location:** `crates/kiro-market-core/src/plugin.rs:59`
**Issue:** During skill directory scanning, `filter_map(Result::ok)` silently drops I/O errors (permission denied, symlink cycles, etc.). Users get partial results with no indication that skills were skipped.
**Fix:** Replace with explicit match that logs each failed `DirEntry` at debug/warn level.

---

## Should-Fix (Important)

### 5. `load_plugin_manifest` treats corrupt JSON same as missing
**Source:** error-hunter
**Location:** `crates/kiro-market/src/commands/install.rs:405-431`
**Issue:** A malformed `plugin.json` is silently treated as absent, falling back to default skill paths. The debug log also mislabels all `fs::read` errors as "not found."
**Fix:** Distinguish `ErrorKind::NotFound` from other errors. Warn the user about malformed manifests.

### 6. Duplicate `load_skill_paths` with same silent swallowing
**Source:** error-hunter, code-simplifier
**Location:** `crates/kiro-market/src/commands/search.rs:136-151`, `info.rs:164-179`
**Issue:** Identical function duplicated in two files, both silently discarding parse errors with bare `_` wildcards.
**Fix:** Extract to shared helper in `commands/mod.rs` or a `commands/common.rs` module with proper error handling.

### 7. Duplicated `find_plugin_entry` function
**Source:** code-simplifier, code-reviewer
**Location:** `crates/kiro-market/src/commands/install.rs:84-106`, `info.rs:49-71`
**Issue:** Identical function in two files.
**Fix:** Extract to shared module.

### 8. Error details discarded in install failure messages
**Source:** error-hunter
**Location:** `crates/kiro-market/src/commands/install.rs:178-194`
**Issue:** `let...else` blocks print "Failed to read" but discard the actual error. Users have no way to know why (permission denied? encoding? not found?).
**Fix:** Use `match` to capture and include the error in the message.

### 9. Search/info silently skip unreadable SKILL.md files
**Source:** error-hunter
**Location:** `crates/kiro-market/src/commands/search.rs:88-94`, `info.rs:147-152`
**Issue:** Bare `continue` with no logging when SKILL.md can't be read or parsed. Users get incomplete results with no indication.
**Fix:** Add debug-level logging for each skipped entry.

### 10. `marketplace update` returns Ok(()) on failure
**Source:** error-hunter
**Location:** `crates/kiro-market/src/commands/marketplace.rs:266-274`
**Issue:** When pull fails, the error is printed but the command returns success (exit code 0). CI pipelines will believe the update succeeded.
**Fix:** Track failures and `bail!` if any update failed.

### 11. SKILL.md uses plain fs::write while tracking uses atomic_write
**Source:** code-reviewer
**Location:** `crates/kiro-market-core/src/project.rs:221`
**Issue:** Inconsistent crash safety. The arguably more important file (SKILL.md) is written non-atomically.
**Fix:** Use `atomic_write` for SKILL.md too.

### 12. `verify_sha` bidirectional prefix check is logically wrong
**Source:** error-hunter
**Location:** `crates/kiro-market-core/src/git.rs:101`
**Issue:** `expected_sha.starts_with(&actual_str)` would match if actual is a prefix of expected, which is backwards. Only `actual.starts_with(expected)` should be checked.
**Fix:** Remove the second condition.

### 13. Repeated clone-and-verify pattern in resolve_structured_source
**Source:** code-simplifier
**Location:** `crates/kiro-market/src/commands/install.rs:352-402`
**Issue:** Three match arms for GitHub/GitUrl/GitSubdir follow identical clone+verify logic (~30 lines duplicated).
**Fix:** Extract URL and optional subdir first, then run clone+verify once.

---

## Suggestions (Nice-to-Have)

### 14. Remove dead `Skill` struct
**Source:** comment-analyzer, type-design-analyzer
**Location:** `crates/kiro-market-core/src/skill.rs`
**Issue:** Defined but never constructed anywhere.

### 15. Remove `list_installed` trivial alias
**Source:** code-simplifier, type-design-analyzer
**Location:** `crates/kiro-market-core/src/project.rs:125-127`
**Issue:** One-line alias for `load_installed` with no semantic distinction.

### 16. Introduce `ValidatedName` newtype
**Source:** type-design-analyzer
**Issue:** Currently validation depends on callers remembering to call `validate_name`. A newtype would make the type system enforce it.

### 17. Introduce `PluginRef` newtype with clap `FromStr`
**Source:** type-design-analyzer
**Issue:** `plugin_ref: String` on 3 commands requires separate parsing. A newtype eliminates duplication.

### 18. Return `&str` body from `parse_frontmatter` instead of `usize` offset
**Source:** type-design-analyzer
**Issue:** Raw byte offset is easy to misuse. Returning the body slice directly prevents encoding bugs.

### 19. Use `tempfile::NamedTempFile` for atomic writes
**Source:** error-hunter
**Location:** `crates/kiro-market-core/src/cache.rs:216-221`
**Issue:** Deterministic `.tmp` filename means concurrent processes could collide. `NamedTempFile` generates unique names.

### 20. `Option<&String>` should be `Option<&str>` in install.rs
**Source:** code-simplifier
**Location:** `crates/kiro-market/src/commands/install.rs:131`

### 21. `extract_relative_md_links` doc comment missing `..` rejection condition
**Source:** comment-analyzer
**Location:** `crates/kiro-market-core/src/skill.rs:101-108`

### 22. `PluginSource` doc should warn about serde variant ordering sensitivity
**Source:** comment-analyzer
**Location:** `crates/kiro-market-core/src/marketplace.rs:36-48`

### 23. `remove_skill` uses `SkillMdNotFound` for "not installed" — semantically wrong
**Source:** comment-analyzer, type-design-analyzer
**Location:** `crates/kiro-market-core/src/project.rs:186-187`

### 24. Companion file read failures silently dropped
**Source:** error-hunter
**Location:** `crates/kiro-market/src/commands/install.rs:209-220`
**Issue:** SKILL.md explicitly references a companion but read failure is swallowed with debug log.

### 25. Stale plugin cache reused without integrity check
**Source:** error-hunter
**Location:** `crates/kiro-market/src/commands/install.rs:345-350`

---

## Test Gaps

| Priority | Gap | Location |
|----------|-----|----------|
| 1 | No unit tests for install.rs orchestration (431 lines) | `commands/install.rs` |
| 2 | No tests for search.rs or info.rs logic | `commands/search.rs`, `commands/info.rs` |
| 3 | No test for `pull_repo` fast-forward success path | `kiro-market-core/src/git.rs` |
| 4 | No end-to-end integration test for successful install | `tests/cli_tests.rs` |
| 5 | No CRLF frontmatter parsing test | `kiro-market-core/src/skill.rs` |
| 6 | No corrupted registry file test | `kiro-market-core/src/cache.rs` |
| 7 | No test for `clone_repo` with `git_ref` parameter | `kiro-market-core/src/git.rs` |

---

## Type Design Summary

| Type | Encapsulation | Expression | Usefulness | Enforcement |
|------|:---:|:---:|:---:|:---:|
| Error hierarchy | 8 | 8 | 9 | 8 |
| CacheDir/MarketplaceSource | 7 | 7 | 8 | 7 |
| KiroProject/InstalledSkills | 7 | 6 | 8 | 7 |
| PluginSource/StructuredSource | 6 | 7 | 7 | 5 |
| Cli/Command | 6 | 5 | 7 | 5 |
| SkillFrontmatter/ParseError | 3 | 6 | 7 | 6 |
| Marketplace | 2 | 3 | 5 | 3 |
| PluginManifest | 2 | 3 | 4 | 3 |

---

## What Previous Reviews Missed

The four existing code reviews in `docs/` (Claude/opencode, Copilot, Kiro CLI) collectively missed these findings that the 6-agent review caught:

- `discover_skill_dirs` skill path validation gap (security)
- `filter_map(Result::ok)` silent error suppression
- Marketplace name used before validation (ordering bug)
- `load_plugin_manifest` corrupt-vs-missing conflation
- `marketplace update` exit code lies
- Error details discarded in let...else blocks
- Dead `Skill` struct
- `verify_sha` bidirectional check logic error
- Companion file read failures silently dropped
- Stale plugin cache reused without integrity check

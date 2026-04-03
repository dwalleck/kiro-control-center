# Code Review: kiro-marketplace-cli

**Date:** 2026-04-02
**Reviewer:** Claude (opencode)

---

## Summary

This is a well-structured Rust CLI project for installing Claude Code marketplace skills into Kiro CLI projects. The codebase is clean, well-tested, and follows good Rust conventions. The project has a clear separation between `kiro-market-core` (library crate) and `kiro-market` (CLI binary), with comprehensive test coverage and proper error handling.

---

## Strengths

### Architecture & Organization

- Clean separation between library and binary crates
- Excellent modularity: each module has a focused responsibility
- Workspace configuration with shared dependencies and lints
- Good use of constants for repeated values (e.g., `SKILL_MD`)

### Error Handling (`crates/kiro-market-core/src/error.rs`)

- Well-designed error hierarchy using `thiserror`
- All domain errors are `#[non_exhaustive]` for future extensibility
- Good `#[source]` annotations for proper error chaining
- Comprehensive test coverage for error display formatting and conversions

### Testing

- Comprehensive unit tests in every module
- Good use of `rstest` for parameterized tests
- Proper test fixtures with `tempfile`
- Tests cover happy paths, edge cases, and error conditions

### Type Design

- Strong types for marketplace/plugin/skill concepts
- Good use of serde's untagged enums for flexible JSON parsing
- `PluginSource` enum correctly handles both relative paths and structured sources
- Deserialization validation ensures required fields are present

### Code Quality

- No `unsafe_code` (forbidden at workspace level)
- Clippy passes with no warnings
- Consistent code style and formatting

---

## Issues & Suggestions

### 1. Missing Documentation on `pub` APIs (Medium Priority)

Several public functions lack doc comments. Example in `crates/kiro-market-core/src/git.rs:27`:

```rust
/// Clone a remote Git repository into `dest`.
pub fn clone_repo(...) -> Result<Repository, GitError>
```

While the function has a doc comment, the public API in `lib.rs` could benefit from more comprehensive documentation with examples for crate users.

**Recommendation:** Add module-level documentation to `lib.rs` explaining the crate's purpose and key types.

---

### 2. `update` Command is a Stub (Medium Priority)

Location: `crates/kiro-market/src/commands/update.rs:10-30`

The command just prints a message telling users to use `remove` + `install --force`:

```rust
pub fn run(plugin_ref: Option<&str>) -> Result<()> {
    println!(
        "{} In-place update for {} is not yet supported.",
        "!".yellow().bold(),
        target
    );
    // ... suggests workaround
}
```

**Recommendation:** Either implement proper in-place skill updates or remove the stub. The workaround is non-obvious to users.

---

### 3. Potential Race Condition in Marketplace Add (Medium Priority)

Location: `crates/kiro-market/src/commands/marketplace.rs:134-150`

The rename operation after cloning could fail if multiple instances run simultaneously:

```rust
let final_dir = cache.marketplace_path(&name);
if final_dir.exists() {
    let _ = fs::remove_dir_all(&temp_dir);
    bail!("marketplace directory already exists...");
}
fs::rename(&temp_dir, &final_dir)
```

**Recommendation:** Consider using a file lock or atomic rename operation to prevent race conditions.

---

### 4. Duplicated Path Constants (Low Priority)

The path `.claude-plugin/marketplace.json` is repeated in multiple commands:

- `crates/kiro-market/src/commands/install.rs:20`
- `crates/kiro-market/src/commands/marketplace.rs:15`
- `crates/kiro-market/src/commands/search.rs:14`
- `crates/kiro-market/src/commands/info.rs:17`

Similarly, `DEFAULT_SKILL_PATHS` is duplicated in `install.rs`, `search.rs`, and `info.rs`.

**Recommendation:** Centralize these constants in a shared module:

```rust
// crates/kiro-market/src/constants.rs
pub const MARKETPLACE_MANIFEST: &str = ".claude-plugin/marketplace.json";
pub const DEFAULT_SKILL_PATHS: &[&str] = &["./"];
```

---

### 5. Unused `version` Field in `PluginEntry` (Low Priority)

Location: `crates/kiro-market-core/src/marketplace.rs:25-30`

```rust
pub struct PluginEntry {
    pub name: String,
    pub description: Option<String>,
    pub source: PluginSource,
    // version field exists in struct but is never read
}
```

The `version` field exists in the struct but is not serialized/deserialized. Looking at the design doc, the `PluginEntry` should have a `version` field but it's currently missing from the deserialization.

**Recommendation:** Add the `version` field to `PluginEntry` and deserialize it, or remove it if not needed.

---

### 6. Input Validation Could Be Tightened (Low Priority)

Location: `crates/kiro-market/src/cli.rs:86-88`

The `parse_plugin_ref` function accepts empty strings:

```rust
assert_eq!(parse_plugin_ref("@marketplace"), Some(("", "marketplace")));  // passes
assert_eq!(parse_plugin_ref("plugin@"), Some(("plugin", "")));  // passes
```

While the CLI does validate this in `install.rs:41-45`, the parser itself could be stricter.

**Recommendation:** Consider returning `None` for empty parts, or document that validation happens elsewhere.

---

### 7. Slightly Misleading Error Message (Low Priority)

Location: `crates/kiro-market-core/src/git.rs:108-111`

```rust
let refname = head_ref.name().ok_or_else(|| GitError::PullFailed {
    path: path.to_path_buf(),
    source: git2::Error::from_str("HEAD has non-UTF-8 refname"),
})?;
```

The error message says "non-UTF-8 refname" but the actual case is that `name()` returned `None` (detached HEAD or other issue).

**Recommendation:** Change the error message to match the actual condition, e.g., "HEAD ref name not found".

---

### 8. Missing `#[must_use]` on Some Functions (Low Priority)

Some functions that return `Result` or a non-trivial type lack `#[must_use]`. For example, `crates/kiro-market-core/src/git.rs:github_repo_to_url` already has `#[must_use]`, but several similar utility functions could benefit from it.

---

## Security Notes

- No security issues found
- Good use of `dirs` for XDG-compliant path resolution
- Proper error handling for file operations
- No exposure of secrets or keys

---

## Test Coverage Summary

| Module | Tests | Coverage |
|--------|-------|----------|
| `error.rs` | 15+ | Excellent - display formatting, conversions, source chains |
| `marketplace.rs` | 8 | Good - parsing variants, field validation |
| `plugin.rs` | 7 | Good - manifest parsing, skill discovery |
| `skill.rs` | 10 | Good - frontmatter parsing, link extraction, merging |
| `git.rs` | 5 | Good - clone, URL formatting, error handling |
| `cache.rs` | 9 | Good - registry operations, path handling |
| `project.rs` | 8 | Good - install, remove, list operations |
| `cli.rs` | 4 | Good - plugin reference parsing |
| `marketplace.rs` (cmd) | 9 | Good - source detection, registry operations |

---

## Recommendations

### High Priority

1. Complete the `update` command implementation or clearly document it as a stub

### Medium Priority

2. Centralize the marketplace manifest path constant across commands
3. Add the missing `version` field to `PluginEntry` or remove it
4. Address the potential race condition in marketplace add

### Low Priority

5. Add more doc examples to public APIs
6. Tighten input validation in `parse_plugin_ref`
7. Fix the misleading error message in `git.rs`
8. Add `#[must_use]` to appropriate functions

---

## Conclusion

This is a solid, production-ready codebase with good test coverage and clear architecture. The issues found are mostly minor improvements. The core functionality is well-implemented and the code follows good Rust practices. With the suggested improvements, the codebase would be excellent.

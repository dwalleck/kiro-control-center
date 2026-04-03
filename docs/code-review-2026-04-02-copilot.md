# Independent Code Review - 2026-04-02

**Scope:** Reviewed the Rust code under `crates/` and `crates/kiro-market/tests/`. Deliberately excluded `docs/` so this review stayed independent of prior writeups.

## Summary

The codebase is generally clean and covers happy paths well, but there are several important issues around path safety, persistence safety, and source pinning.

## Findings

### 1. High - Path boundary violations from untrusted manifest and skill data

**Affected areas**
- `crates/kiro-market/src/commands/install.rs`
- `crates/kiro-market-core/src/plugin.rs`
- `crates/kiro-market-core/src/skill.rs`
- `crates/kiro-market-core/src/project.rs`
- `crates/kiro-market/src/commands/search.rs`
- `crates/kiro-market/src/commands/info.rs`
- `crates/kiro-market/src/commands/marketplace.rs`

Untrusted values from marketplace manifests, `plugin.json`, `SKILL.md`, and skill frontmatter are joined into filesystem paths without canonicalization and root-boundary checks. That includes:

- relative plugin source paths
- `plugin.json.skills` entries
- `git-subdir.path`
- companion Markdown links extracted from `SKILL.md`
- skill names used as on-disk directory names
- marketplace names read from manifests

Inputs containing `../` or path separators can escape the intended cache or project directories and cause reads/writes outside the expected root.

**Recommendation:** Canonicalize paths at the system boundary, reject paths that escape the allowed root, and validate marketplace/skill names before using them as directory names.

### 2. High - Companion file merging can read files outside the skill directory

**Affected areas**
- `crates/kiro-market-core/src/skill.rs`
- `crates/kiro-market/src/commands/install.rs`

`extract_relative_md_links()` rejects URLs and absolute paths, but it still accepts links like `../shared/secret.md`. `process_skill()` then reads those paths with `skill_dir.join(link)` and merges the contents into the installed `SKILL.md`.

That lets a malicious skill pull arbitrary local Markdown files from outside its own directory into the installed output.

**Recommendation:** Canonicalize each companion path and require it to stay under the skill directory before reading it.

### 3. High - Persistence updates are not atomic

**Affected areas**
- `crates/kiro-market-core/src/cache.rs`
- `crates/kiro-market-core/src/project.rs`
- `crates/kiro-market/src/commands/marketplace.rs`

Important state files such as `known_marketplaces.json` and `installed-skills.json` are rewritten with plain `fs::write(...)`, and several operations update directories and tracking JSON in separate steps.

Examples:
- marketplace add renames the cached directory and only then updates the registry
- skill install writes `SKILL.md` and only then updates installed tracking
- skill remove deletes the directory and only then updates installed tracking

A crash or write failure can leave orphaned directories, truncated JSON, or metadata that no longer matches what is on disk.

**Recommendation:** Use temp-file-plus-rename atomic writes for JSON state and make install/remove flows transactional or recoverable.

### 4. Medium - Structured source `sha` is parsed but never enforced

**Affected areas**
- `crates/kiro-market-core/src/marketplace.rs`
- `crates/kiro-market/src/commands/install.rs`

Structured plugin sources accept both `ref` and `sha`, but install logic only uses `ref`. If a manifest specifies a commit SHA to pin an exact revision, that pin is silently ignored.

This breaks reproducibility and allows installs to drift when a branch or tag moves.

**Recommendation:** After cloning, resolve and verify the declared SHA, or check it out directly and fail if it does not match the fetched content.

### 5. Medium - Structured plugin cache can become stale

**Affected area**
- `crates/kiro-market/src/commands/install.rs`

`resolve_structured_source()` reuses an existing cached plugin directory purely based on `marketplace/plugin` path existence. If the marketplace manifest changes the source URL, `ref`, or `sha`, the cached clone is still reused without validation or refresh.

That can install the wrong plugin contents even after the marketplace definition changes.

**Recommendation:** Record source metadata for cached plugins and invalidate or refresh the cache when URL/ref/SHA changes.

### 6. Medium - `CacheDir::default_location()` can panic

**Affected area**
- `crates/kiro-market-core/src/cache.rs`

`CacheDir::default_location()` uses `dirs::data_dir().expect(...)`. In container, CI, or otherwise minimal environments, that can turn a recoverable runtime configuration problem into a hard process abort.

**Recommendation:** Return a typed error instead of panicking when no data directory can be determined.

### 7. Medium - `search` and `info` hide malformed `plugin.json`

**Affected areas**
- `crates/kiro-market/src/commands/search.rs`
- `crates/kiro-market/src/commands/info.rs`

Both commands treat an invalid `plugin.json` the same as a missing one and silently fall back to default skill paths. That can produce incomplete or misleading output while appearing successful.

**Recommendation:** Distinguish between "missing manifest" and "present but invalid", and surface malformed manifests as errors or at least explicit warnings.

## Meaningful Test Gaps

- No tests reject path escape attempts such as `../` in plugin sources, skill paths, companion links, or installed skill names.
- No tests cover interrupted or failed persistence writes for registry/tracking files.
- No tests prove SHA pinning is actually honored during structured-source installs.

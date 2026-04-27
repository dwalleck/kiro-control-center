---
name: rust-best-practices
description: |
  **ALWAYS USE** when working with Rust code. This skill contains 45 rules that prevent common bugs.

  Trigger on ANY of these:
  - Editing .rs files or Cargo.toml
  - Writing new Rust functions, structs, enums, or impl blocks
  - Reviewing Rust code before committing
  - Debugging Rust errors (borrow checker, Option/Result, lifetimes)
  - Working with: unwrap, expect, ?, Option, Result, Path, PathBuf
  - File I/O operations in Rust (read, write, atomic writes)
  - CLI development with clap
  - Error handling patterns
  - Functions returning Option or Result that might fail silently
  - Path operations (canonicalize, file_name, parent, extension)
  - Deserializing JSON/YAML/TOML from external sources

  Key rules to apply:
  - Rule 35: Log before returning None (prevent silent failures)
  - Rule 36: Return Err for invalid input, not empty collections
  - Rule 37: Canonicalize paths at system boundaries
  - Rule 39: Every public method needs tests
  - Rule 40: Validate untrusted data at point of use
  - Rule 41: Audit ergonomic error suppressors (.ok(), filter_map(Result::ok))
  - Rule 42: Match error types, not error strings

  **Before committing Rust code**, review the Pre-Commit Checklist in this skill.
---

# Rust Best Practices

Apply these patterns when writing or reviewing Rust code. For detailed examples and explanations, reference the deep-dive files in this skill directory.

## Quick Reference - 45 Rules

### Error Handling

**1. Handle Option types explicitly**
- Use `if let`, `match`, `unwrap_or`, or `ok_or()?` - never bare `.unwrap()` in production
- Common Option sources: `vec.get()`, `map.get()`, `path.file_name()`, `path.parent()`

**2. Path operations return Option**
- `file_name()`, `parent()`, `extension()` can all return `None`
- Root paths, empty paths, and `..` paths have edge cases

**3. expect() vs unwrap() vs ?**
- `?` for runtime errors (file I/O, user input, network)
- `.expect("why")` for invariants (hardcoded regex, compile-time values)
- `.unwrap_or()` for defaults
- Never bare `.unwrap()` except in tests

→ Deep dive: [error-handling.md](error-handling.md)

### File I/O Safety

**4. Use atomic writes for important files**
```rust
let mut temp = NamedTempFile::new_in(dir)?;
temp.write_all(data)?;
temp.sync_all()?;
temp.persist(path)?;
```

**5. Create parent directories before writing**
```rust
if let Some(parent) = path.parent() {
    fs::create_dir_all(parent)?;
}
```

**6. Test file I/O with tempfile crate**
- Roundtrip tests, parent directory creation, error cases

→ Deep dive: [file-io.md](file-io.md)

### Type Safety

**7. Use constants over magic strings**
```rust
pub const VALID_EVENTS: &[&str] = &["UserPromptSubmit", "PostToolUse"];
```

**8. Use enums for fixed value sets**
```rust
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum HookEvent { UserPromptSubmit, PostToolUse, Stop }
```

**9. Use newtypes to prevent ID confusion**
```rust
struct UserId(i32);
struct AssessmentId(i32);
// Compiler prevents mixing these up
```

**10. Validate immediately in setters**
- Return `Result<()>` and validate before adding to struct
- Don't defer to separate `validate()` method

**11. Add "did you mean" suggestions**
- Use `strsim::levenshtein` for typo suggestions in validation errors

→ Deep dive: [type-safety.md](type-safety.md)

### Performance

**12. Move loop-invariant computations outside loops**
```rust
// BAD: creates wrapper 100 times
for kw in keywords {
    let prompt_uc = UniCase::new(prompt); // Inside loop!
}

// GOOD: create once
let prompt_uc = UniCase::new(prompt);
for kw in keywords { ... }
```

**13. Know when NOT to use zero-copy abstractions**
- `UniCase` works for equality, NOT substring matching
- When in doubt, use `to_lowercase()` + standard methods

→ Deep dive: [performance.md](performance.md)

### Common Footguns

**14. Avoid TOCTOU races**
```rust
// BAD: race condition
if path.exists() { fs::read(path)? }

// GOOD: check via operation result
match fs::read(path) {
    Ok(data) => { /* existed */ },
    Err(e) if e.kind() == NotFound => { /* didn't exist */ },
    Err(e) => return Err(e.into()),
}
```

**15. Use `.cloned()` with HashSet when mutating source**
```rust
// BAD: borrow checker error
let existing: HashSet<&String> = self.vec.iter().collect();
self.vec.push(item); // ERROR!

// GOOD: clone to break borrow
let existing: HashSet<String> = self.vec.iter().cloned().collect();
self.vec.push(item); // OK
```

→ Deep dive: [common-footguns.md](common-footguns.md)

### Fundamentals

**16. Don't use redundant single-component imports**
```rust
// BAD
use serde_json;
serde_json::json!(...)

// GOOD - either fully qualified OR import specific items
serde_json::json!(...)  // No import needed
use serde_json::json;   // Then use json!(...)
```

**17. Initialize tracing subscribers in main()**
```rust
tracing_subscriber::fmt()
    .with_env_filter(EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info")))
    .init();
```

**18. Calculate conditions once, use everywhere**
```rust
let use_color = !args.no_color && env::var("NO_COLOR").is_err();
// Use use_color throughout, don't recalculate
```

**19. Check TTY for colored output**
```rust
use std::io::{self, IsTerminal};
let use_color = env::var("NO_COLOR").is_err() && io::stdout().is_terminal();
```

**20. Provide CLI feedback for file operations**
```rust
if file_existed {
    println!("Updated: {}", path.display());
} else {
    println!("Created: {}", path.display());
}
```

→ Deep dive: [fundamentals.md](fundamentals.md)

### CLI Development (clap)

**21. Use derive API over builder**
```rust
#[derive(Parser)]
#[command(version, about)]
struct Args {
    #[arg(short, long)]
    verbose: bool,
}
```

**22. Use subcommands for complex CLIs**
```rust
#[derive(Parser)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Add { name: String },
    Remove { id: u32 },
}
```

**23. Use ValueEnum for restricted choices**
```rust
#[derive(Copy, Clone, ValueEnum)]
enum Format { Json, Yaml, Toml }

#[arg(value_enum, default_value_t = Format::Json)]
format: Format,
```

**24. Handle -- for passthrough arguments**
```rust
#[arg(last = true)]
child_args: Vec<String>,  // Everything after --
```

**25. Support env var fallbacks**
```rust
#[arg(long, env = "API_TOKEN")]
token: String,
```

**26. Use semantic exit codes**
```rust
use exitcode;
std::process::exit(exitcode::CONFIG);  // Not just 1
```

**27. Handle Ctrl+C gracefully**
```rust
let running = Arc::new(AtomicBool::new(true));
ctrlc::set_handler(move || running.store(false, Ordering::SeqCst))?;
```

**28. Config priority: CLI > env > file > default**
```rust
// Load config, then override with CLI args
if let Some(val) = args.override_value {
    config.value = val;
}
```

→ Deep dive: [cli-development.md](cli-development.md)

### Concurrency & Parallel Code

**29. Handle mutex poisoning explicitly**
```rust
// BAD: silently loses errors
if let Ok(guard) = mutex.lock() { ... }

// GOOD: recover data from poisoned mutex
match mutex.lock() {
    Ok(guard) => { /* use guard */ },
    Err(poisoned) => {
        tracing::warn!("Mutex poisoned, recovering");
        let guard = poisoned.into_inner();
        // use recovered guard
    }
}
```

**30. Prefer try_borrow_mut() over borrow_mut()**
```rust
// RISKY: panics if already borrowed
let mut p = parser.borrow_mut();

// DEFENSIVE: returns Result
let mut p = parser.try_borrow_mut()
    .map_err(|_| Error::Parser("already borrowed"))?;
```

**31. Add equivalence tests for parallel code**
- When converting sequential → parallel, verify identical results
- Test determinism by running the same input multiple times

→ Deep dive: [common-footguns.md](common-footguns.md)

### Type Safety (continued)

**32. Use debug_assert! in constructors**
```rust
impl Data {
    pub fn new(path: PathBuf, line: u32) -> Self {
        debug_assert!(!path.is_absolute(), "path should be relative");
        debug_assert!(line > 0, "line numbers are 1-indexed");
        Self { path, line }
    }
}
```

→ Deep dive: [type-safety.md](type-safety.md)

### Clippy Style

**33. Use numeric separators for large literals**
```rust
// BAD: hard to read
let ts = 1234567890;

// GOOD: separators every 3 digits
let ts = 1_234_567_890;
```

**34. Use structured logging fields**
```rust
// BAD: no context
tracing::error!("Parse failed: {}", e);

// GOOD: structured fields
tracing::error!(file = %path.display(), error = %e, "Parse failed");
```

→ Deep dive: [fundamentals.md](fundamentals.md)

### Silent Failure Prevention

**35. Log before returning None from functions**
```rust
// BAD: Silent failure - caller has no idea why it returned None
fn parse_config(path: &Path) -> Option<Config> {
    let data = fs::read(path).ok()?;  // Silent!
    toml::from_slice(&data).ok()?;    // Silent!
}

// GOOD: Log context before early returns
fn parse_config(path: &Path) -> Option<Config> {
    let data = match fs::read(path) {
        Ok(d) => d,
        Err(e) => {
            debug!(path = %path.display(), error = %e, "Failed to read config");
            return None;
        }
    };
    // Or use let...else
    let Some(config) = toml::from_slice(&data).ok() else {
        debug!(path = %path.display(), "Failed to parse config as TOML");
        return None;
    };
    Some(config)
}
```

**36. Return Err instead of empty collections for invalid inputs**
```rust
// BAD: Looks like success but input was invalid
fn expand_glob(pattern: &str) -> io::Result<Vec<PathBuf>> {
    if !is_supported_pattern(pattern) {
        warn!("Unsupported pattern");
        return Ok(Vec::new());  // Misleading!
    }
    // ...
}

// GOOD: Invalid input is an error
fn expand_glob(pattern: &str) -> io::Result<Vec<PathBuf>> {
    if !is_supported_pattern(pattern) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("Unsupported pattern: {pattern}")
        ));
    }
    // ...
}
```

### Path Consistency

**37. Canonicalize paths at system boundaries**
```rust
// BAD: Paths compared without normalization
struct Cache {
    entries: HashMap<PathBuf, Data>,  // Some absolute, some relative?
}

impl Cache {
    fn get(&self, path: &Path) -> Option<&Data> {
        self.entries.get(path)  // May fail due to path form mismatch
    }
}

// GOOD: Canonicalize at entry/exit points
impl Cache {
    fn insert(&mut self, path: &Path, data: Data) -> io::Result<()> {
        let canonical = path.canonicalize()?;
        self.entries.insert(canonical, data);
        Ok(())
    }

    fn get(&self, path: &Path) -> Option<&Data> {
        let canonical = path.canonicalize().ok()?;
        self.entries.get(&canonical)
    }
}
```

**38. Document path contracts in types**
```rust
/// Information about a discovered crate.
pub struct CrateInfo {
    /// Crate name from Cargo.toml
    pub name: String,
    /// **Canonical absolute path** to crate directory
    pub path: PathBuf,
    /// Entry point path **relative to `path`**
    pub lib_path: Option<PathBuf>,
}
```

### Trust Boundaries & Error Precision

**40. Validate untrusted data at point of use, not just downstream**
```rust
// BAD: name from deserialized JSON used in fs::rename before validation
let name = manifest.name.clone();
let dest = cache_dir.join(&name);  // "../escape" escapes the cache!
fs::rename(&temp, &dest)?;
// ... validate_name(&name) called later in add_to_registry()

// GOOD: validate before every system boundary the data touches
let name = manifest.name.clone();
validate_name(&name)?;  // Before ANY path construction
let dest = cache_dir.join(&name);
fs::rename(&temp, &dest)?;
```
- Data from deserialized files (JSON manifests, YAML configs, TOML) is untrusted input
- Validate before *every* filesystem/network operation, not at one downstream checkpoint
- Even if `validate_name()` exists, call it *before* `fs::rename()`, not just before DB insert

**41. Audit Rust's ergonomic error suppressors**
```rust
// DANGEROUS: these one-liners silently discard errors
entries.filter_map(Result::ok)           // Drops I/O errors
let Ok(x) = expr else { continue; };    // Hides WHY it failed
let data = fs::read(path).ok()?;        // Caller sees None, not the error
let _ = fs::remove_dir_all(&temp);      // Cleanup failure invisible

// BETTER: ask "are there different failure modes here?"
for entry in entries {
    let entry = match entry {
        Ok(e) => e,
        Err(e) => {
            warn!(error = %e, "skipping unreadable entry");
            continue;
        }
    };
}

// BETTER: distinguish "missing" (expected) from "corrupt" (bug)
match fs::read(&manifest_path) {
    Ok(bytes) => match serde_json::from_slice(&bytes) {
        Ok(m) => Some(m),
        Err(e) => { warn!(error = %e, "manifest is malformed"); None }
    },
    Err(e) if e.kind() == NotFound => None,  // Expected, use defaults
    Err(e) => { warn!(error = %e, "failed to read manifest"); None }
}
```
- Before using `.ok()`, `filter_map(Result::ok)`, `let _ =`, or `let...else { continue }`, ask: *"Does anyone need to know which failure mode this was?"*
- `let _ =` is the most common silent discarder — if something can fail, at least log it: `if let Err(e) = operation { warn!(...) }`
- "Missing" and "corrupt" are different — collapsing them hides author mistakes

**42. Match error types, not error strings**
```rust
// BAD: fragile — breaks silently if error message wording changes
let msg = e.to_string();
if msg.contains("already installed") {
    // handle duplicate
}

// GOOD: compiler catches variant renames, exhaustive matching
match result {
    Err(Error::Skill(SkillError::AlreadyInstalled { .. })) => {
        // handle duplicate
    }
    Err(e) => bail!(e),
}
```
- Pattern match on error variants, not `.to_string().contains(...)`
- If the right variant doesn't exist, create it — 3 lines in `thiserror`
- String matching can't be caught by the compiler when error messages change

### Error Boundary Discipline

**43. Wrap internal errors to match the function's documented contract**
```rust
// BAD: run_git returns GitNotFound, but clone_repo documents CloneFailed
pub fn clone_repo(url: &str, dest: &Path) -> Result<(), GitError> {
    // ...
    let output = run_git(&["checkout", refname], dest)?;  // Leaks GitNotFound!
}

// GOOD: wrap so callers only see documented variants
pub fn clone_repo(url: &str, dest: &Path) -> Result<(), GitError> {
    // ...
    let output = run_git(&["checkout", refname], dest)
        .map_err(|e| GitError::CloneFailed {
            url: url.to_owned(),
            source: Box::new(e),
        })?;
}
```
- If a function's doc says "Returns `ErrorA`", every `?` inside must produce `ErrorA`
- Internal helpers may return different error types — `map_err` at the call site
- Leaking unexpected variants breaks callers who pattern-match on the documented contract
- This is especially important at trait boundaries where the impl may use different internal errors

**44. `Path::is_absolute()` is platform-dependent**
```rust
// BAD: assumes Unix behavior on all platforms
} else if Path::new(source).is_absolute() {
    // On Windows: "/home/user/foo" → false (no drive letter)
    // On Linux: "C:\Users\foo" → false (no leading /)
}

// GOOD: combine with explicit prefix checks for cross-platform code
} else if Path::new(source).is_absolute()
    || source.starts_with('/')  // Unix paths on Windows (WSL, configs)
{
```
- `Path::is_absolute()` uses platform-native rules at compile time
- On Windows, `/foo` is NOT absolute (needs `C:\` or `\\`)
- On Linux, `C:\foo` is NOT absolute (needs `/`)
- When parsing user input or config files that may originate from another platform, add explicit fallback checks
- Test cross-platform path logic with `#[cfg(windows)]` / `#[cfg(unix)]` gated tests

**45. Never discard Results with `let _ =` — log or propagate**
```rust
// BAD: cleanup failure invisible — temp dirs accumulate silently
let _ = fs::remove_dir_all(&temp_dir);

// GOOD: log the failure while still continuing
if let Err(e) = fs::remove_dir_all(&temp_dir) {
    warn!(path = %temp_dir.display(), error = %e, "failed to clean up temp directory");
}
```
- `let _ = expr` is a code smell when `expr` returns `Result`
- Even "best-effort" cleanup deserves a `warn!` so failures are discoverable
- Grep for `let _ =` in production code as a quality check — each hit should be justified or replaced
- If you truly don't care about the result AND it can't meaningfully fail, use a comment explaining why

→ Deep dive: [common-footguns.md](common-footguns.md)

### Test Coverage

**39. Every public method needs tests**
- Happy path test (normal usage)
- Edge case test (empty, None, boundary values)
- For lookup/matching: test specificity/priority logic

```rust
// If you add these public methods:
pub fn get_item_for_key(&self, key: &str) -> Option<&Item>
pub fn get_item_root(&self, key: &str) -> Option<&Path>

// You need at minimum:
#[test] fn get_item_for_key_finds_match() { ... }
#[test] fn get_item_for_key_returns_none_for_unknown() { ... }
#[test] fn get_item_for_key_prefers_specific_match() { ... }  // if applicable
```

→ Deep dive: [common-footguns.md](common-footguns.md)

## Pre-Commit Checklist

**Code Quality:**
- [ ] All Option/Result handled explicitly (no bare `.unwrap()`)
- [ ] Path operations handle None cases
- [ ] No redundant imports
- [ ] Loop-invariant code moved outside loops

**Silent Failure Prevention:**
- [ ] Functions returning Option log before returning None
- [ ] Invalid inputs return Err, not empty collections
- [ ] Early returns include context (debug! or warn!)
- [ ] No `filter_map(Result::ok)`, `.ok()?`, or `let _ =` hiding distinct failure modes
- [ ] "Missing" vs "corrupt" distinguished in error handling (not collapsed)
- [ ] No `let _ =` on Results in production code — log or propagate

**Error Precision:**
- [ ] Error matching uses type patterns, not `.to_string().contains(...)`
- [ ] Error variants are semantically correct (not reusing "close enough" variants)
- [ ] If the right error variant doesn't exist, create it
- [ ] Internal errors wrapped to match the function's documented error contract (no variant leaks)

**Trust Boundaries:**
- [ ] Deserialized data (JSON/YAML/TOML) validated before filesystem/network use
- [ ] Validation happens at point of use, not just downstream
- [ ] Each system boundary has its own validation (not relying on a single checkpoint)

**Path Handling:**
- [ ] Paths canonicalized at system boundaries
- [ ] Path contracts documented (absolute? relative to what?)
- [ ] Consistent path form when comparing/storing
- [ ] `Path::is_absolute()` not sole check when parsing cross-platform input

**File I/O:**
- [ ] Atomic writes for important files (NamedTempFile)
- [ ] Parent directories created before writes
- [ ] Integration tests with tempfile

**Type Safety:**
- [ ] Constants or enums for fixed value sets
- [ ] Newtypes for distinct ID types
- [ ] Immediate validation in setters
- [ ] `debug_assert!` in constructors for invariants

**Concurrency:**
- [ ] Mutex poisoning handled (not silently ignored)
- [ ] `try_borrow_mut()` for defensive RefCell access
- [ ] Equivalence tests for parallel code

**Test Coverage:**
- [ ] Every new public method has tests
- [ ] Happy path + edge cases covered
- [ ] Lookup/matching methods test specificity

**CLI/UX:**
- [ ] TTY detection for colored output
- [ ] User feedback for file operations
- [ ] Structured logging with context fields

**Tooling:**
- [ ] `cargo fmt` applied
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `cargo test` passes

## Deep-Dive Files

Load these for detailed examples when encountering specific issues:

| File | Topics | When to Load |
|------|--------|--------------|
| [planning-checklist.md](planning-checklist.md) | Platform assumptions, error path coverage, return types, behavioral equivalence, test design | **Before finalizing any implementation plan** — catches issues at design time |
| [error-handling.md](error-handling.md) | Option patterns, Path footguns, expect vs unwrap | Seeing `.unwrap()`, Option handling issues |
| [file-io.md](file-io.md) | Atomic writes, parent dirs, tempfile testing | File read/write code |
| [type-safety.md](type-safety.md) | Constants, enums, newtypes, validation, debug_assert | Magic strings, ID confusion, constructor validation |
| [performance.md](performance.md) | Loop optimization, zero-copy limits | Hot loops, performance issues |
| [common-footguns.md](common-footguns.md) | TOCTOU, borrow checker, Path edge cases, **concurrency**, **trust boundaries**, **error suppression**, **error boundary discipline**, **cross-platform paths** | Race conditions, borrow errors, **parallel code**, **deserialized data**, **filter_map(Result::ok)**, **`let _ =`**, **`Path::is_absolute()`** |
| [fundamentals.md](fundamentals.md) | Imports, tracing, CLI UX, color output, **clippy style** | General Rust setup, CLI apps, **structured logging** |
| [cli-development.md](cli-development.md) | clap derive, subcommands, custom parsing, cargo plugins | Building CLI tools with clap |

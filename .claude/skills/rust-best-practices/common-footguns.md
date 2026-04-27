# Common Footguns

This guide covers specific gotchas and common mistakes that trip up even experienced Rust developers.

## What This Guide Covers

1. **[Path Operations Return Options](#1-path-operations-return-options)** - Why Path methods are error-prone
2. **[TOCTOU Races](#2-toctou-races)** - Time-of-check-time-of-use vulnerabilities
3. **[Borrow Checker with HashSet](#3-borrow-checker-with-hashset)** - Collection borrowing pitfalls
4. **[Concurrency & Parallel Code](#4-concurrency--parallel-code)** - Mutex poisoning, RefCell panics
5. **[Trust Boundaries in Deserialized Data](#5-trust-boundaries-in-deserialized-data)** - Untrusted manifests, error suppression, string matching

**Quick Reference:** See [quick-reference.md](quick-reference.md) for scannable checklists

---

## 1. Path Operations Return Options

### Why This is a Common Footgun

Path operations like `file_name()`, `parent()`, and `extension()` return `Option<T>`, but developers often assume they'll always succeed and use `.unwrap()`. This is covered in detail in the [Error Handling Deep Dive](error-handling.md#2-common-footgun-path-operations).

### Quick Summary

```rust
// ❌ WRONG - Assumes file_name() always returns Some
let file_name = path.file_name().unwrap().to_string_lossy();

// ✅ CORRECT - Handle None case
let file_name = path.file_name()
    .map(|n| n.to_string_lossy())
    .unwrap_or_else(|| path.display().to_string().into());
```

**Why Path methods return Option:**

```rust
Path::new("/").file_name()          // None - root has no filename
Path::new("foo/..").file_name()     // None - parent reference
Path::new("/").parent()              // None - root has no parent
Path::new("Makefile").extension()    // None - no extension
```

**See full guide:** [Error Handling Deep Dive - Path Operations](error-handling.md#2-common-footgun-path-operations)

****

---

## 2. TOCTOU Races

### The Problem

**TOCTOU = Time-Of-Check-Time-Of-Use**

Checking if a file exists separately from using it creates a race condition where the file state can change between the check and use.

### Critical Example

**❌ BAD - Race condition:**

```rust
Commands::AddHook { path, ... } => {
    // Check if file exists
    let file_exists = std::path::Path::new(&path).exists();

    // ⚠️  Time passes... file could be created/deleted here!

    // Try to read based on old check
    let mut settings = ClaudeSettings::read(&path).unwrap_or_default();

    // Later, use outdated file_exists
    if file_exists {
        println!("Modified existing file");  // Might be wrong!
    } else {
        println!("Created new file");  // Might be wrong!
    }
}
```

**Race scenarios:**
1. **False negative:** File doesn't exist during check, gets created before read → wrong message
2. **False positive:** File exists during check, gets deleted before read → wrong message

### Solution: Check the Result, Not the Filesystem

**✅ GOOD - No race condition:**

```rust
Commands::AddHook { path, ... } => {
    // Try to read and let the Result tell us if it existed
    let (mut settings, file_existed) = match ClaudeSettings::read(&path) {
        Ok(s) => (s, true),   // File existed and was readable
        Err(_) => (ClaudeSettings::default(), false),  // File didn't exist
    };

    // ... add hook ...

    // Use the result from the ACTUAL operation
    if file_existed {
        println!("Modified existing file");  // We actually read it
    } else {
        println!("Created new file");  // We actually created it
    }
}
```

**Why this is better:**
1. **Atomic check-and-use:** Read attempt is a single atomic operation
2. **Truth from operation:** We know the file existed because we successfully read it
3. **No race window:** No time between check and use for state to change
4. **Handles all cases:** Covers not-exists, exists-but-unreadable, etc.

### Common TOCTOU Patterns

**File existence:**

```rust
// ❌ exists() then open()
if path.exists() { fs::File::open(path)? }

// ✅ Try open, handle NotFound
match fs::File::open(path) {
    Ok(f) => f,
    Err(e) if e.kind() == io::ErrorKind::NotFound => { /* handle */ },
    Err(e) => return Err(e.into()),
}
```

**Directory creation:**

```rust
// ❌ exists() then create
if !dir.exists() { fs::create_dir(dir)? }

// ✅ create_dir_all (idempotent)
fs::create_dir_all(dir)?;  // Succeeds if exists
```

**File metadata:**

```rust
// ❌ Check then use
if path.metadata()?.is_file() {
    fs::read(path)?
}

// ✅ Try operation, handle error
match fs::read(path) {
    Ok(data) => data,
    Err(e) if e.kind() == io::ErrorKind::InvalidInput => { /* not a file */ },
    Err(e) => return Err(e.into()),
}
```

### Security Implications

**Critical in security contexts:**

```rust
// 🔒 SECURITY ISSUE - TOCTOU vulnerability
fn check_and_open_secure_file(path: &Path) -> Result<File> {
    // Attacker could create symlink to /etc/passwd here!
    if path.exists() && is_safe_path(path) {
        // Between check and open, attacker swaps file
        fs::File::open(path)?  // Opens attacker's file!
    }
}

// ✅ SECURE - Open with specific flags
fn open_secure_file(path: &Path) -> Result<File> {
    fs::OpenOptions::new()
        .read(true)
        .create(false)    // Don't create
        .truncate(false)  // Don't modify
        .open(path)?      // Atomic open
    // Then verify it's what we expect
}
```

### The Golden Rule

**Never check filesystem state separately from using it. Let the operation itself tell you the state through its Result. Use idempotent operations like `create_dir_all()` instead of conditional operations.**

****

---

## 3. Borrow Checker with HashSet

### The Problem

Creating a HashSet from borrowed data while simultaneously trying to mutate the original collection causes borrow checker errors.

### Classic Example

**❌ WRONG - Borrow checker error:**

```rust
pub fn merge(&mut self, other: ClaudeSettings) {
    // Immutable borrow here
    let existing_servers: HashSet<_> = self.enabled_mcpjson_servers.iter().collect();

    for server in other.enabled_mcpjson_servers {
        if !existing_servers.contains(&server) {
            // ERROR: Mutable borrow while immutable borrow exists
            self.enabled_mcpjson_servers.push(server);
        }
    }
}
```

**Compiler error:**
```
error[E0502]: cannot borrow `self.enabled_mcpjson_servers` as mutable
because it is also borrowed as immutable
```

**Why it fails:**
- `.iter()` creates references to items in `self.enabled_mcpjson_servers`
- These references live in the `HashSet<&String>`
- We then try to push (mut borrow) while HashSet still holds references (immut borrow)

### Solution: Clone or Copy Elements

**✅ CORRECT - Clone elements to break the borrow:**

```rust
pub fn merge(&mut self, other: ClaudeSettings) {
    // Clone elements, no references to self
    let existing_servers: HashSet<_> =
        self.enabled_mcpjson_servers.iter().cloned().collect();

    for server in other.enabled_mcpjson_servers {
        if !existing_servers.contains(&server) {
            self.enabled_mcpjson_servers.push(server);  // Now OK!
        }
    }
}
```

### Why .cloned() Works

```rust
// Without .cloned() - HashSet<&String> (references to self)
let bad: HashSet<&String> = self.vec.iter().collect();

// With .cloned() - HashSet<String> (owned copies, no borrows)
let good: HashSet<String> = self.vec.iter().cloned().collect();
```

### Alternative Solutions

**Option 1: Drain and rebuild** (if you're replacing the whole vec)

```rust
let existing: HashSet<_> = self.vec.drain(..).collect();
// Now self.vec is empty, no borrow issues
for item in other.vec {
    if !existing.contains(&item) {
        self.vec.push(item);
    }
}
```

**Option 2: Build new vec then swap**

```rust
let existing: HashSet<_> = self.vec.iter().cloned().collect();
let mut new_vec = self.vec.clone();
for item in other.vec {
    if !existing.contains(&item) {
        new_vec.push(item);
    }
}
self.vec = new_vec;
```

**Option 3: Use Entry API** (for HashMap)

```rust
for (key, value) in other.map {
    self.map.entry(key).or_insert(value);  // No borrow issues
}
```

### Performance Considerations

**Cost of .cloned():**
- O(n) time to clone elements
- O(n) space for owned copies

**Still better than O(n²) contains():**

```rust
// ❌ O(n²) - contains() is O(n) in Vec
for item in other.vec {
    if !self.vec.contains(&item) {  // O(n) lookup
        self.vec.push(item);
    }
}

// ✅ O(n) - HashSet lookup is O(1)
let existing: HashSet<_> = self.vec.iter().cloned().collect();  // O(n)
for item in other.vec {  // O(n)
    if !existing.contains(&item) {  // O(1) lookup
        self.vec.push(item);
    }
}
```

### The Golden Rule

**Use `.cloned()` or `.copied()` when creating a HashSet/HashMap from borrowed data if you need to mutate the original collection. This breaks the borrow relationship and satisfies the borrow checker.**

****

---

## 4. Concurrency & Parallel Code

### Mutex Poisoning

When a thread panics while holding a `Mutex` lock, the mutex becomes "poisoned." Both `lock()` and `into_inner()` return `Result` types that must be handled.

**❌ WRONG - Silent error loss:**

```rust
// Silently drops errors if mutex is poisoned
if let Ok(mut guard) = errors.lock() {
    guard.push(error);
}

// Silently loses ALL collected errors if poisoned
if let Ok(collected) = errors.into_inner() {
    process(collected);
}
```

**Why this is dangerous:**
- Mutex poisoning indicates another thread panicked - something is already wrong
- Silently ignoring it loses error information needed for debugging
- The data inside the mutex is still valid and accessible

**✅ CORRECT - Recover data and log:**

```rust
// Handle poisoning - recover data, log the issue
match errors.lock() {
    Ok(mut guard) => {
        guard.push(error);
    }
    Err(poisoned) => {
        tracing::warn!("Mutex poisoned during error collection, recovering");
        poisoned.into_inner().push(error);
    }
}

// Same for into_inner()
match errors.into_inner() {
    Ok(collected) => process(collected),
    Err(poisoned) => {
        tracing::warn!("Mutex was poisoned, recovering collected data");
        process(poisoned.into_inner());
    }
}
```

**Key insight:** `PoisonError` contains the guard/data. Call `.into_inner()` on the error to access it.

### RefCell Borrow Panic Risk

`RefCell::borrow_mut()` panics if already borrowed. While often safe in practice, defensive coding uses `try_borrow_mut()`.

**❌ RISKY - Panics if already borrowed:**

```rust
thread_local! {
    static PARSER: RefCell<Parser> = RefCell::new(Parser::new());
}

fn parse(content: &str) -> Result<Tree> {
    PARSER.with(|parser| {
        let mut p = parser.borrow_mut();  // Panics if re-entered!
        p.parse(content)
    })
}
```

**✅ DEFENSIVE - Returns Result:**

```rust
fn parse(content: &str) -> Result<Tree> {
    PARSER.with(|parser| {
        let mut p = parser.try_borrow_mut()
            .map_err(|_| Error::Parser("parser already borrowed (re-entrant call?)"))?;
        p.parse(content)
    })
}
```

**When to use each:**
- `borrow_mut()`: When you're 100% certain no re-entrancy is possible
- `try_borrow_mut()`: When code structure might change, or for defensive coding

### Equivalence Testing for Parallel Code

When converting sequential code to parallel (e.g., with rayon), add tests that verify both produce identical results.

```rust
#[test]
fn parallel_produces_same_results_as_sequential() {
    let input = generate_test_data();

    // Run both implementations
    let sequential_result = process_sequential(&input);
    let parallel_result = process_parallel(&input);

    // Core equivalence assertions
    assert_eq!(sequential_result.count, parallel_result.count);
    assert_eq!(sequential_result.items, parallel_result.items);
    assert_eq!(sequential_result.errors.len(), parallel_result.errors.len());
}
```

**Why this matters:**
- Parallel code can introduce subtle non-determinism
- Race conditions may only appear under load
- Order-dependent bugs are easy to miss in unit tests

### The Golden Rule

**In concurrent code: Always handle `PoisonError` explicitly (data is recoverable), prefer `try_borrow_mut()` over `borrow_mut()` for defensive coding, and add equivalence tests when parallelizing sequential code.**

****

---

## 5. Trust Boundaries in Deserialized Data

### The Problem

Data from deserialized files (JSON manifests, YAML configs, TOML settings) is **untrusted input** — even when it comes from your own git repos. Developers often treat deserialized data like internal state because it "looks like our code," but anyone can author a manifest in a marketplace, plugin repository, or config package.

The danger isn't missing validation — it's **validating too late**. A `validate_name()` function may exist and work perfectly, but if it runs *after* the untrusted string has already been used in `fs::rename()` or `Path::join()`, the damage is done.

### Real-World Example: Path Traversal via Manifest Name

This bug was found in a code review of a CLI tool that installs plugins from marketplace repositories:

**❌ BAD — Name used in filesystem operation before validation:**

```rust
fn add_marketplace(source: &str) -> Result<()> {
    // 1. Clone the repo into a temp directory
    let temp_dir = cache.marketplace_path("_pending");
    git::clone_repo(url, &temp_dir, None)?;

    // 2. Read the manifest to get the marketplace name
    let manifest = Marketplace::from_json(&fs::read(temp_dir.join("manifest.json"))?)?;
    let name = manifest.name.clone();  // ⚠️ Untrusted!

    // 3. Use name in filesystem operation — BEFORE any validation
    let final_dir = cache.marketplace_path(&name);  // name = "../escape"
    fs::rename(&temp_dir, &final_dir)?;  // 💥 Escapes the cache directory!

    // 4. Validation happens here — TOO LATE
    cache.add_known_marketplace(KnownMarketplace { name, .. })?;
    //   └── validate_name(&entry.name)? called inside add_known_marketplace
}
```

**Attack:** A malicious manifest with `"name": "../escape"` causes `fs::rename` to move the cloned directory outside the intended cache, potentially overwriting system files.

**✅ GOOD — Validate immediately after deserialization, before any use:**

```rust
fn add_marketplace(source: &str) -> Result<()> {
    let temp_dir = cache.marketplace_path("_pending");
    git::clone_repo(url, &temp_dir, None)?;

    let manifest = Marketplace::from_json(&fs::read(temp_dir.join("manifest.json"))?)?;
    let name = manifest.name.clone();

    // Validate BEFORE any path construction or filesystem use
    validate_name(&name)
        .with_context(|| format!("manifest contains invalid name '{name}'"))?;

    let final_dir = cache.marketplace_path(&name);  // Now safe
    fs::rename(&temp_dir, &final_dir)?;
    // ...
}
```

### Real-World Example: Skill Path Escape via plugin.json

**❌ BAD — Untrusted skill paths joined directly to filesystem:**

```rust
pub fn discover_skill_dirs(plugin_root: &Path, skill_paths: &[&str]) -> Vec<PathBuf> {
    for &path_str in skill_paths {  // From plugin.json — untrusted!
        let candidate = plugin_root.join(path_str);  // "../../etc" escapes root
        // ... scan for SKILL.md ...
    }
}
```

**✅ GOOD — Validate each entry before joining:**

```rust
pub fn discover_skill_dirs(plugin_root: &Path, skill_paths: &[&str]) -> Vec<PathBuf> {
    for &path_str in skill_paths {
        if let Err(e) = validate_relative_path(path_str) {
            warn!(path = path_str, error = %e, "skipping invalid skill path");
            continue;
        }
        let candidate = plugin_root.join(path_str);  // Now safe
        // ...
    }
}
```

### Real-World Example: Ergonomic Error Suppression

Rust makes it syntactically easy to silence errors. These one-liners are fine for truly optional operations, but dangerous when applied to operations with *different failure modes*:

**❌ BAD — "Missing" and "corrupt" are different, but both become None:**

```rust
fn load_plugin_manifest(dir: &Path) -> Option<PluginManifest> {
    let bytes = fs::read(dir.join("plugin.json")).ok()?;       // Silent!
    serde_json::from_slice(&bytes).ok()?                        // Silent!
}
```

If `plugin.json` exists but has a syntax error, this silently falls back to default skill paths — installing the *wrong skills* with no warning to the user or plugin author.

**✅ GOOD — Distinguish expected absence from unexpected corruption:**

```rust
fn load_plugin_manifest(dir: &Path) -> Option<PluginManifest> {
    let manifest_path = dir.join("plugin.json");
    match fs::read(&manifest_path) {
        Ok(bytes) => match serde_json::from_slice(&bytes) {
            Ok(m) => Some(m),
            Err(e) => {
                warn!(path = %manifest_path.display(), error = %e,
                      "plugin.json is malformed, falling back to defaults");
                None
            }
        },
        Err(e) if e.kind() == io::ErrorKind::NotFound => None,  // Expected
        Err(e) => {
            warn!(path = %manifest_path.display(), error = %e,
                  "failed to read plugin.json");
            None
        }
    }
}
```

### Real-World Example: String Matching on Errors

**❌ BAD — Breaks silently if error message changes:**

```rust
match project.install_skill(name, content, meta) {
    Ok(()) => { /* success */ }
    Err(e) => {
        if e.to_string().contains("already installed") {
            println!("Skipped (already installed)");
        } else {
            eprintln!("Failed: {e}");
        }
    }
}
```

**✅ GOOD — Compiler catches variant renames:**

```rust
match project.install_skill(name, content, meta) {
    Ok(()) => { /* success */ }
    Err(Error::Skill(SkillError::AlreadyInstalled { .. })) => {
        println!("Skipped (already installed)");
    }
    Err(e) => eprintln!("Failed: {e}"),
}
```

If `AlreadyInstalled` is renamed to `Duplicate`, the string matcher silently breaks at runtime. The pattern matcher fails at compile time.

### The Checklist

When working with deserialized data, ask these questions:

1. **Who authors this file?** If it's not your code, it's untrusted input.
2. **What system boundaries does this data touch before validation?** Every `fs::rename`, `Path::join`, `fs::write`, or network call using the data needs validation *before* that call.
3. **Am I using `.ok()?` or `filter_map(Result::ok)` on data that could be corrupt vs missing?** If these are different failure modes, use explicit `match`.
4. **Am I matching on error strings?** If so, can I match on the error type instead?

### The Golden Rule

**Treat every deserialized string like user input from a web form. Validate it at the point of use, not just at some downstream checkpoint. The question is not "do we validate?" but "do we validate before every dangerous operation?"**

****

---

## Related Topics

### Error Handling
- **[Option handling](error-handling.md#1-understanding-option-types)** - Universal Option patterns
- **[Path operations](error-handling.md#2-common-footgun-path-operations)** - Full Path footgun guide

### File I/O
- **[Atomic writes](file-io.md#1-atomic-file-writes)** - Preventing data corruption
- **[Parent directory creation](file-io.md#2-parent-directory-creation)** - Avoiding TOCTOU with create_dir_all

### Performance
- **[Loop optimizations](performance.md#1-performance-critical-loop-optimizations)** - HashSet performance in loops

---

**[Quick Reference →](quick-reference.md)**

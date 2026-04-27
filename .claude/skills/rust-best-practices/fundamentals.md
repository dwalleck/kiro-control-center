# Fundamentals Deep Dive

This guide covers fundamental Rust patterns and CLI best practices that apply to all projects.

## What This Guide Covers

1. **[Redundant Single-Component Imports](#1-redundant-single-component-imports)** - Clean import patterns
2. **[Uninitialized Tracing Subscribers](#2-uninitialized-tracing-subscribers)** - Logging setup
3. **[Structured Logging with Context](#3-structured-logging-with-context)** - Error diagnostics in parallel code
4. **[Duplicated Logic](#4-duplicated-logic)** - DRY principle
5. **[TTY Detection for Colored Output](#5-tty-detection-for-colored-output)** - Terminal-aware output
6. **[CLI User Feedback for File Operations](#6-cli-user-feedback-for-file-operations)** - Informative UX
7. **[Clippy Style Rules](#7-clippy-style-rules)** - Common lint fixes

**Quick Reference:** See [quick-reference.md](quick-reference.md) for scannable checklists

---

## 1. Redundant Single-Component Imports

### The Problem

Clippy warns about redundant single-component path imports (`use serde_json;`) when you're using fully qualified paths. If you write `serde_json::json!`, you don't need `use serde_json;` - the crate is already available through `Cargo.toml`.

### Example

```rust
// ❌ WRONG - Redundant import with fully qualified paths
use serde_json;  // Clippy: this import is redundant

fn print_json_results(stats: &Stats, elapsed: Duration) {
    let json = serde_json::json!({     // Using fully qualified path
        "total_files": stats.total_files,
    });
    println!("{}", serde_json::to_string_pretty(&json).unwrap());  // Fully qualified
}
```

**Clippy error:** `clippy::single_component_path_imports` - "this import is redundant"

### Solution Options

**Option 1: Use fully qualified paths (no import needed)**

```rust
// ✅ CORRECT - No import, use fully qualified paths
fn print_json_results(stats: &Stats, elapsed: Duration) {
    let json = serde_json::json!({
        "total_files": stats.total_files,
    });
    println!("{}", serde_json::to_string_pretty(&json).unwrap());
}
```

**Option 2: Import specific items and use unqualified**

```rust
// ✅ ALSO CORRECT - Import specific items
use serde_json::json;

fn print_json_results(stats: &Stats, elapsed: Duration) {
    let json = json!({  // Now unqualified
        "total_files": stats.total_files,
    });
    println!("{}", serde_json::to_string_pretty(&json).unwrap());
}
```

### Common Cases

```rust
// ❌ WRONG - Redundant imports
use tracing_subscriber;
tracing_subscriber::fmt().init();

use serde_json;
serde_json::json!({"key": "value"})

// ✅ CORRECT - Fully qualified (no import)
tracing_subscriber::fmt().init();
serde_json::json!({"key": "value"})

// ✅ ALSO CORRECT - Import specific items
use tracing_subscriber::{fmt, EnvFilter};
fmt().with_env_filter(EnvFilter::new("info")).init();
```

### The Rule

**Use fully qualified paths (no import) OR import specific items (unqualified use). Never use single-component imports like `use serde_json;`**

****

---

## 2. Uninitialized Tracing Subscribers

### The Problem

Using `tracing::debug!`, `info!`, `warn!` etc. without initializing a subscriber means logs won't appear, even with `RUST_LOG=debug`.

### Example

```rust
// ❌ WRONG - No subscriber initialization
use tracing::debug;

fn main() -> Result<()> {
    debug!("This will never appear!");  // Silent failure
    // ... rest of code
}
```

**Issue:** Debug logs appear to work in development (other parts of codebase might initialize subscriber) but fail in production/standalone use.

### Solution

```rust
// ✅ CORRECT - Initialize subscriber in main()
use tracing::debug;
use tracing_subscriber;

fn main() -> Result<()> {
    // Initialize tracing subscriber (respects RUST_LOG env var)
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    debug!("This will appear with RUST_LOG=debug");
    // ... rest of code
}
```

### Dependencies Required

```toml
[dependencies]
tracing = { workspace = true }
tracing-subscriber = { workspace = true, features = ["env-filter"] }
```

### The Rule

**Every binary that uses tracing MUST initialize a subscriber in `main()`. Libraries should NOT initialize subscribers (let the binary decide).**

****

---

## 3. Structured Logging with Context

### The Problem

When errors occur in parallel or async code, logs without context make debugging nearly impossible. You need to know which file, which operation, and what state caused the error.

### Example

```rust
// ❌ WRONG - No context, useless for debugging
fn parse_file(path: &Path, language: Language) -> Result<Tree> {
    let parser = get_parser()?;
    parser.set_language(&language.to_tree_sitter())
        .map_err(|e| {
            tracing::error!("Failed to set language: {}", e);  // Which file? Which language?
            Error::Parser(e.to_string())
        })?;
    // ...
}
```

When this fails in parallel processing of 1000 files, you see:
```
ERROR Failed to set language: incompatible version
```

**Useless!** Which of 1000 files failed? What language?

### Solution: Structured Fields

```rust
// ✅ CORRECT - Structured fields with full context
fn parse_file(path: &Path, language: Language) -> Result<Tree> {
    let parser = get_parser()?;
    parser.set_language(&language.to_tree_sitter())
        .map_err(|e| {
            tracing::error!(
                language = ?language,
                file = %path.display(),
                error = %e,
                "Failed to set parser language"
            );
            Error::Parser(format!("failed to set language {language:?}: {e}"))
        })?;
    // ...
}
```

Now you see:
```
ERROR language=CSharp file="src/broken.cs" error="incompatible version" Failed to set parser language
```

### Field Formatting

- `%value` - Use `Display` trait (user-friendly): `file = %path.display()`
- `?value` - Use `Debug` trait (developer detail): `language = ?lang`
- `value` - Direct value if it implements `tracing::Value`

### Context for Different Scenarios

**Parallel file processing:**
```rust
tracing::error!(
    file = %path.display(),
    language = ?language,
    thread_id = ?std::thread::current().id(),
    error = %e,
    "Parse failed"
);
```

**Database operations:**
```rust
tracing::error!(
    operation = "insert_symbol",
    symbol_name = %name,
    file_id = file_id,
    error = %e,
    "Database write failed"
);
```

**Cycle detection / graph operations:**
```rust
tracing::debug!(
    node_count = nodes.len(),
    edge_count = edges.len(),
    "Starting DFS traversal"
);

tracing::info!(
    raw_cycles = raw_count,
    unique_cycles = deduped_count,
    "Cycle detection complete"
);
```

### When to Add Context

**Always include context for:**
- Error logs (`error!`, `warn!`)
- Operations that can fail silently
- Parallel/concurrent code paths
- Database operations
- File I/O operations

**Optional context for:**
- Debug logs (but helpful when debugging)
- Trace logs (can be verbose)

### The Rule

**Always use structured fields (`field = value`) instead of string interpolation in tracing macros. Include file paths, operation names, and relevant IDs so errors can be traced back to their source.**

****

---

## 4. Duplicated Logic

### The Problem

Checking the same condition in multiple places creates maintenance burden and potential bugs if conditions diverge.

### Example

```rust
// ❌ WRONG - Same logic in two places
fn main() -> Result<()> {
    let args = Args::parse();

    // First check (lines 196-198)
    if args.no_color || std::env::var("NO_COLOR").is_ok() {
        colored::control::set_override(false);
    }

    // ... 20 lines later ...

    // Second check (line 217) - DUPLICATE!
    let use_color = !args.no_color && std::env::var("NO_COLOR").is_err();

    if use_color {
        println!("{}", "text".bright_blue());
    }
}
```

**Issues:**
1. Same condition logic appears twice
2. If you update one, must remember to update the other
3. Logical inverse makes it harder to verify they're equivalent

### Solution

```rust
// ✅ CORRECT - Calculate once, use everywhere
fn main() -> Result<()> {
    let args = Args::parse();

    // Calculate color decision ONCE at the start
    let use_color = !args.no_color && std::env::var("NO_COLOR").is_err();

    // Set the global override based on our decision
    if !use_color {
        colored::control::set_override(false);
    }

    // ... rest of code uses `use_color` variable ...

    if use_color {
        println!("{}", "text".bright_blue());
    }
}
```

### Benefits

1. Single source of truth
2. Easier to modify behavior
3. More efficient (calculate once vs multiple times)
4. Clearer intent with descriptive variable name

### The Rule

**Calculate conditions once at the start of a function, store in a well-named variable, and reference that variable everywhere. Don't re-calculate the same condition.**

****

---

## 5. TTY Detection for Colored Output

### The Problem

Sending ANSI color codes to non-terminal outputs (pipes, files, CI logs) creates unreadable garbage characters and pollutes logs.

### Example

```rust
// ❌ WRONG - Always uses color codes based on NO_COLOR env var only
fn main() -> Result<()> {
    let use_color = env::var("NO_COLOR").is_err();

    if use_color {
        println!("{}", "✅ Success".green());  // Garbage in CI logs!
    }
}
```

**Problem scenarios:**

```bash
# Piped to file - color codes in file
settings-manager read settings.json > output.txt  # File contains \x1b[32m codes

# Piped to grep - can't match colored text
settings-manager validate settings.json | grep "Success"  # May not match

# CI logs - unreadable
# [32m✅ Success[0m  ← Garbage in GitHub Actions logs
```

### Solution: Check if stdout is a Terminal

```rust
use std::io::{self, IsTerminal};

fn main() -> Result<()> {
    // Check both NO_COLOR and whether stdout is a terminal
    let use_color = env::var("NO_COLOR").is_err() && io::stdout().is_terminal();

    if use_color {
        println!("{}", "✅ Success".green());
    } else {
        println!("✅ Success");
    }

    Ok(())
}
```

### TTY Detection Methods

**Stable Rust (1.70+):**

```rust
use std::io::{self, IsTerminal};

// Check stdout
let is_tty = io::stdout().is_terminal();

// Check stderr (for error messages)
let is_tty = io::stderr().is_terminal();
```

**With `atty` crate (older Rust):**

```rust
use atty::Stream;

let is_tty = atty::is(Stream::Stdout);
```

### Complete Color Detection Pattern

```rust
use std::env;
use std::io::{self, IsTerminal};

fn should_use_color() -> bool {
    // Respect NO_COLOR environment variable (standard)
    if env::var("NO_COLOR").is_ok() {
        return false;
    }

    // Respect FORCE_COLOR (for testing)
    if env::var("FORCE_COLOR").is_ok() {
        return true;
    }

    // Only use color if stdout is a terminal
    io::stdout().is_terminal()
}

fn main() -> Result<()> {
    let use_color = should_use_color();

    // Use color decision consistently
    if use_color {
        println!("{}", "Success".green());
    } else {
        println!("Success");
    }

    Ok(())
}
```

### Integration with `colored` Crate

```rust
use colored::*;

fn main() -> Result<()> {
    // Set global override at startup
    if !should_use_color() {
        colored::control::set_override(false);
    }

    // Now all colored output respects the setting
    println!("{}", "This respects TTY detection".green());

    Ok(())
}
```

### When to Check TTY

**Check stdout TTY for:**
- ✅ Regular output (results, status messages)
- ✅ JSON output (some tools colorize JSON)
- ✅ Table formatting

**Check stderr TTY for:**
- ✅ Error messages
- ✅ Warning messages
- ✅ Progress indicators

**Both might be different:**

```bash
# stdout piped, stderr to terminal
program 2> errors.log | less

# stdout to terminal, stderr piped
program > output.txt
```

### Testing

```bash
# Should NOT have color codes:
settings-manager read settings.json > output.txt
cat output.txt  # Should be plain text

# Should have color codes:
settings-manager read settings.json  # To terminal

# Should respect NO_COLOR:
NO_COLOR=1 settings-manager read settings.json  # No colors
```

### The Rule

**Always check if stdout is a terminal (`io::stdout().is_terminal()`) in addition to checking `NO_COLOR`. This prevents ANSI codes from polluting pipes, files, and CI logs.**

****

---

## 6. CLI User Feedback for File Operations

### The Problem

Silent file operations leave users confused about what actually happened. This is especially problematic for operations that create, modify, or delete files.

### Example

```rust
// ❌ WRONG - Silent file creation
Commands::AddHook { path, event, command, .. } => {
    // Load existing settings or create new
    let mut settings = ClaudeSettings::read(&path).unwrap_or_default();

    settings.add_hook(&event, hook_config);
    settings.write(&path)?;  // Did we create? Did we modify? User has no idea!

    println!("✅ Hook added");  // Incomplete feedback
}
```

**Problems:**
- User doesn't know if file was created or modified
- No confirmation of the file location
- Can't tell if operation was a no-op (hook already existed)
- Silent failures might go unnoticed

### Solution: Inform Users of Actions

```rust
// ✅ CORRECT - Clear feedback about what happened
Commands::AddHook { path, event, command, matcher, dry_run } => {
    let file_existed = path.exists();

    // Load existing settings or create new
    let mut settings = if file_existed {
        ClaudeSettings::read(&path)?
    } else {
        println!("📝 Creating new settings file: {}", path.display());
        ClaudeSettings::default()
    };

    let hook_config = HookConfig {
        matcher,
        hooks: vec![Hook {
            r#type: HOOK_TYPE_COMMAND.to_string(),
            command,
        }],
    };

    settings.add_hook(&event, hook_config);
    settings.validate()?;

    if dry_run {
        println!("🔍 Dry run - would write to: {}", path.display());
        println!("{}", serde_json::to_string_pretty(&settings)?);
    } else {
        settings.write(&path)?;

        if file_existed {
            println!("✅ Hook added to existing file: {}", path.display());
        } else {
            println!("✅ Created new settings file with hook: {}", path.display());
        }

        println!("   Event: {}", event);
        println!("   Command: {}", hook_config.hooks[0].command);
    }

    Ok(())
}
```

### Feedback Levels

**Minimal (quiet mode):**
```rust
// Just success/failure
println!("✅ Hook added");
```

**Standard (default):**
```rust
// What happened and where
println!("✅ Hook added to {}", path.display());
println!("   Event: {}", event);
```

**Verbose (--verbose flag):**
```rust
// Everything that happened
println!("📝 Loading settings from {}", path.display());
println!("✅ Hook added successfully");
println!("   Event: {}", event);
println!("   Command: {}", command);
println!("   File size: {} bytes", metadata.len());
```

### File Operation Feedback Patterns

**Creating files:**
```rust
if !path.exists() {
    println!("📝 Creating new file: {}", path.display());
}
fs::write(&path, content)?;
println!("✅ Created {}", path.display());
```

**Modifying files:**
```rust
if path.exists() {
    println!("📝 Updating existing file: {}", path.display());
} else {
    println!("📝 Creating new file: {}", path.display());
}
fs::write(&path, content)?;
println!("✅ Saved changes to {}", path.display());
```

**Deleting files:**
```rust
if path.exists() {
    println!("🗑️  Removing: {}", path.display());
    fs::remove_file(&path)?;
    println!("✅ Deleted");
} else {
    println!("ℹ️  File doesn't exist (nothing to delete): {}", path.display());
}
```

### Interactive Confirmations

For destructive operations, ask for confirmation:

```rust
use std::io::{self, Write};

fn confirm_overwrite(path: &Path) -> Result<bool> {
    print!("File {} already exists. Overwrite? [y/N] ", path.display());
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;

    Ok(input.trim().to_lowercase() == "y")
}

// Usage:
if path.exists() && !confirm_overwrite(&path)? {
    println!("❌ Operation cancelled");
    return Ok(());
}
```

### Summary Messages

For operations that affect multiple files:

```rust
println!("\n📊 Summary:");
println!("   Files created: {}", created_count);
println!("   Files modified: {}", modified_count);
println!("   Files skipped: {}", skipped_count);
if failed_count > 0 {
    println!("   ⚠️  Files failed: {}", failed_count);
}
```

### User Feedback Checklist

For CLI file operations:

- [ ] Inform when creating new files vs modifying existing
- [ ] Show file paths so users know where files went
- [ ] Provide summary of what changed
- [ ] Use visual indicators (✅ ❌ 📝 🗑️ ⚠️) for quick scanning
- [ ] Confirm destructive operations (delete, overwrite)
- [ ] Show dry-run results before actual changes
- [ ] Include relevant details (event, command, etc.) in output

### The Rule

**Always inform users about file operations. Tell them what happened (created/modified/deleted), where it happened (file path), and whether it succeeded. Use emojis and colors to make feedback scannable.**

****

---

## 7. Clippy Style Rules

These are common clippy lints that catch style issues. Running `cargo clippy -- -D warnings` will flag these.

### Numeric Separators (unreadable_literal)

Large numeric literals are hard to read without separators.

```rust
// ❌ WRONG - Hard to count zeros
let timestamp = 1234567890;
let big_number = 10000000;

// ✅ CORRECT - Use underscores as separators
let timestamp = 1_234_567_890;
let big_number = 10_000_000;
```

**Rule:** Use `_` separators for numbers with 5+ digits. Group by thousands for decimal, by 4 for hex.

### Raw String Hashes (needless_raw_string_hashes)

Don't use `r#"..."#` when the string contains no `#` characters.

```rust
// ❌ WRONG - Unnecessary hashes
let code = r#"
fn main() {
    println!("hello");
}
"#;

// ✅ CORRECT - No hashes needed
let code = r"
fn main() {
    println!("hello");
}
";

// ✅ CORRECT - Hashes needed (string contains quotes that would conflict)
let json = r#"{"key": "value"}"#;  // Contains " inside
```

**Rule:** Only use `r#"..."#` when the string contains `"` that would end the raw string.

### Doc Comment Formatting (doc_markdown)

Code-like text in doc comments needs backticks.

```rust
// ❌ WRONG - Code references without backticks
/// The module_path parameter should be like "crate::module"
/// Returns a PathBuf pointing to the file

// ✅ CORRECT - Backticks around code
/// The `module_path` parameter should be like `crate::module`
/// Returns a [`PathBuf`] pointing to the file
```

**What needs backticks:**
- Variable/parameter names: `` `path` ``, `` `config` ``
- Type names: `` `String` ``, `` `Option<T>` ``
- Module paths: `` `crate::module` ``
- Function names: `` `parse()` ``
- Code examples: `` `foo.bar()` ``

### Items After Statements (items_after_statements)

Constants and functions should be defined before `let` statements in a function.

```rust
// ❌ WRONG - const after let
fn process() {
    let data = fetch_data();
    const MAX_SIZE: usize = 100;  // Clippy warning!
    // ...
}

// ✅ CORRECT - const before let statements
fn process() {
    const MAX_SIZE: usize = 100;
    let data = fetch_data();
    // ...
}
```

### The Rule

**Run `cargo clippy -- -D warnings` before committing. Common fixes: add numeric separators, remove unnecessary raw string hashes, backtick code in docs, move constants before statements.**

****

---

## Related Topics

### Error Handling
- **[Option handling](error-handling.md#1-understanding-option-types)** - Type-safe null handling
- **[expect vs unwrap](error-handling.md#3-expect-vs-unwrap-vs--decision-guide)** - Error messaging

### File I/O
- **[Atomic writes](file-io.md#1-atomic-file-writes)** - Safe file operations
- **[TOCTOU races](common-footguns.md#2-toctou-races)** - Avoiding file existence checks

### Type Safety
- **[Validation patterns](type-safety.md)** - Input validation
- **[Did you mean suggestions](type-safety.md#5-did-you-mean-suggestions)** - User-friendly errors

---

**[Quick Reference →](quick-reference.md)**

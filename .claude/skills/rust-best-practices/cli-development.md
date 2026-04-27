# CLI Development with Clap

This guide covers building command-line interfaces in Rust, from argument parsing with clap to signal handling and configuration management.

## What This Guide Covers

1. **[Derive vs Builder API](#1-derive-vs-builder-api)** - Choosing the right approach
2. **[Basic Argument Patterns](#2-basic-argument-patterns)** - Flags, options, and positional args
3. **[Subcommand Patterns](#3-subcommand-patterns)** - Git-style hierarchical CLIs
4. **[Custom Type Parsing](#4-custom-type-parsing)** - Value validation and conversion
5. **[Escaped Positional Arguments](#5-escaped-positional-arguments)** - Handling `--` separators
6. **[Cargo Subcommand Pattern](#6-cargo-subcommand-pattern)** - Creating `cargo <your-tool>` commands
7. **[CLI UX Best Practices](#7-cli-ux-best-practices)** - User-friendly design
8. **[Exit Codes](#8-exit-codes)** - Standard conventions and the exitcode crate
9. **[Signal Handling](#9-signal-handling)** - Graceful Ctrl+C and shutdown
10. **[Configuration Files](#10-configuration-files)** - Loading and managing config

---

## 1. Derive vs Builder API

### Recommendation: Use Derive

The derive API is recommended for most CLIs. It's more concise, type-safe, and easier to maintain.

**✅ Derive API (Recommended):**
```rust
use clap::Parser;

#[derive(Parser, Debug)]
#[command(name = "myapp")]
#[command(version, about, long_about = None)]
struct Args {
    /// Name of the person to greet
    #[arg(short, long)]
    name: String,

    /// Number of times to greet
    #[arg(short, long, default_value_t = 1)]
    count: u8,
}

fn main() {
    let args = Args::parse();
    for _ in 0..args.count {
        println!("Hello {}!", args.name);
    }
}
```

**Builder API (for complex dynamic cases):**
```rust
use clap::{Command, Arg};

fn main() {
    let matches = Command::new("myapp")
        .version("1.0")
        .about("Does awesome things")
        .arg(Arg::new("name")
            .short('n')
            .long("name")
            .required(true))
        .get_matches();

    if let Some(name) = matches.get_one::<String>("name") {
        println!("Hello {}!", name);
    }
}
```

### When to Use Builder

- Dynamic argument generation at runtime
- Plugin systems where args aren't known at compile time
- Migrating legacy code incrementally

---

## 2. Basic Argument Patterns

### Flags (Boolean)

```rust
#[derive(Parser)]
struct Args {
    /// Enable verbose output
    #[arg(short, long)]
    verbose: bool,

    /// Enable debug mode (can be repeated: -ddd)
    #[arg(short, long, action = clap::ArgAction::Count)]
    debug: u8,
}
```

### Options with Values

```rust
#[derive(Parser)]
struct Args {
    /// Output file path
    #[arg(short, long)]
    output: Option<PathBuf>,

    /// Config file (required)
    #[arg(short, long)]
    config: PathBuf,

    /// Log level
    #[arg(short, long, default_value = "info")]
    level: String,
}
```

### Positional Arguments

```rust
#[derive(Parser)]
struct Args {
    /// Input file to process
    input: PathBuf,

    /// Additional files (optional, multiple)
    #[arg(trailing_var_arg = true)]
    files: Vec<PathBuf>,
}
```

### Value Enums

```rust
use clap::ValueEnum;

#[derive(Copy, Clone, PartialEq, Eq, ValueEnum, Debug)]
enum OutputFormat {
    Json,
    Yaml,
    Toml,
}

#[derive(Parser)]
struct Args {
    /// Output format
    #[arg(short, long, value_enum, default_value_t = OutputFormat::Json)]
    format: OutputFormat,
}
```

### Environment Variable Fallback

```rust
#[derive(Parser)]
struct Args {
    /// API token (or set API_TOKEN env var)
    #[arg(long, env = "API_TOKEN")]
    token: String,

    /// Database URL
    #[arg(long, env = "DATABASE_URL", default_value = "sqlite://local.db")]
    database: String,
}
```

---

## 3. Subcommand Patterns

### Git-Style Subcommands

```rust
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "git")]
#[command(about = "A fictional versioning CLI", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Clone a repository
    Clone {
        /// Repository URL
        remote: String,
    },

    /// Show changes
    Diff {
        #[arg(long)]
        staged: bool,

        /// Base revision
        #[arg(long)]
        base: Option<String>,

        /// Paths to diff
        #[arg(last = true)]
        paths: Vec<PathBuf>,
    },

    /// Manage stash
    #[command(subcommand)]
    Stash(StashCommands),
}

#[derive(Subcommand)]
enum StashCommands {
    /// Save changes to stash
    Push {
        #[arg(short, long)]
        message: Option<String>,
    },
    /// Restore stashed changes
    Pop {
        #[arg(long)]
        index: bool,
    },
    /// List stashes
    List,
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Commands::Clone { remote } => {
            println!("Cloning {}", remote);
        }
        Commands::Diff { staged, base, paths } => {
            println!("Diffing (staged: {}, base: {:?}, paths: {:?})", staged, base, paths);
        }
        Commands::Stash(stash) => match stash {
            StashCommands::Push { message } => println!("Stashing: {:?}", message),
            StashCommands::Pop { index } => println!("Popping (index: {})", index),
            StashCommands::List => println!("Listing stashes"),
        },
    }
}
```

### External Subcommands

Allow unknown subcommands to be passed to external tools:

```rust
#[derive(Subcommand)]
enum Commands {
    /// Built-in command
    Status,

    /// Pass to external tool
    #[command(external_subcommand)]
    External(Vec<String>),
}
```

### Default Subcommand

```rust
#[derive(Subcommand)]
enum Commands {
    /// Run the main action (default)
    #[command(name = "run")]
    Run {
        #[arg(long)]
        dry_run: bool,
    },

    /// Show status
    Status,
}

// Make "run" the default when no subcommand is specified
#[derive(Parser)]
#[command(args_conflicts_with_subcommands = true)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    // These args are used when no subcommand is given
    #[arg(long)]
    dry_run: bool,
}
```

---

## 4. Custom Type Parsing

### Implicit Parsing (FromStr)

Types implementing `FromStr` are automatically parsed:

```rust
use std::net::IpAddr;
use std::path::PathBuf;

#[derive(Parser)]
struct Args {
    /// Server address
    #[arg(long, default_value = "127.0.0.1")]
    host: IpAddr,

    /// Output path
    #[arg(long)]
    output: PathBuf,

    /// Timeout in seconds
    #[arg(long, default_value_t = 30)]
    timeout: u64,
}
```

### Restricted Values

```rust
use clap::builder::PossibleValuesParser;

#[derive(Parser)]
struct Args {
    /// Port number (only 22 or 80 allowed)
    #[arg(
        long,
        value_parser = PossibleValuesParser::new(["22", "80"])
            .map(|s| s.parse::<u16>().unwrap()),
    )]
    port: u16,
}
```

### Custom Parser Functions

```rust
use std::ops::RangeInclusive;

fn parse_port_in_range(s: &str) -> Result<u16, String> {
    const PORT_RANGE: RangeInclusive<usize> = 1..=65535;

    let port: usize = s
        .parse()
        .map_err(|_| format!("`{s}` is not a valid port number"))?;

    if PORT_RANGE.contains(&port) {
        Ok(port as u16)
    } else {
        Err(format!(
            "port must be in range {}-{}",
            PORT_RANGE.start(),
            PORT_RANGE.end()
        ))
    }
}

#[derive(Parser)]
struct Args {
    #[arg(long, value_parser = parse_port_in_range)]
    port: u16,
}
```

### Key-Value Pairs

```rust
fn parse_key_val(s: &str) -> Result<(String, String), String> {
    let pos = s
        .find('=')
        .ok_or_else(|| format!("invalid KEY=VALUE: no `=` found in `{s}`"))?;
    Ok((s[..pos].to_string(), s[pos + 1..].to_string()))
}

#[derive(Parser)]
struct Args {
    /// Set environment variables (KEY=VALUE)
    #[arg(short = 'e', long = "env", value_parser = parse_key_val)]
    env_vars: Vec<(String, String)>,
}
```

---

## 5. Escaped Positional Arguments

Handle arguments after `--` for passing to child processes:

```rust
#[derive(Parser)]
#[command(name = "runner")]
struct Args {
    /// Enable verbose output
    #[arg(short, long)]
    verbose: bool,

    /// Program to run
    program: String,

    /// Arguments to pass to the program (after --)
    #[arg(last = true)]
    program_args: Vec<String>,
}

fn main() {
    let args = Args::parse();

    // Usage: runner --verbose myprogram -- --flag value
    // args.program = "myprogram"
    // args.program_args = ["--flag", "value"]

    std::process::Command::new(&args.program)
        .args(&args.program_args)
        .spawn()
        .expect("Failed to run program");
}
```

**Usage examples:**
```bash
# Without --: arguments parsed by your CLI
runner --verbose myprogram

# With --: everything after goes to program_args
runner --verbose myprogram -- --child-flag value

# Even flags after -- are treated as positional
runner myprogram -- -v --help   # program_args = ["-v", "--help"]
```

---

## 6. Cargo Subcommand Pattern

Create a tool that works as both standalone and `cargo <tool>`:

```rust
use clap::{Parser, Subcommand};
use std::path::PathBuf;

// Wrapper for cargo invocation
#[derive(Parser)]
#[command(name = "cargo")]
#[command(bin_name = "cargo")]
enum CargoCli {
    /// Your tool description
    MyTool(MyToolArgs),
}

// Actual tool arguments
#[derive(clap::Args)]
#[command(version, about, long_about = None)]
struct MyToolArgs {
    /// Path to Cargo.toml
    #[arg(long)]
    manifest_path: Option<PathBuf>,

    /// Package to operate on
    #[arg(short, long)]
    package: Option<String>,

    #[command(subcommand)]
    command: Option<MyToolCommands>,
}

#[derive(Subcommand)]
enum MyToolCommands {
    /// Check the project
    Check,
    /// Fix issues
    Fix {
        #[arg(long)]
        dry_run: bool,
    },
}

fn main() {
    let CargoCli::MyTool(args) = CargoCli::parse();

    // Now handle args...
    if let Some(manifest) = &args.manifest_path {
        println!("Using manifest: {}", manifest.display());
    }
}
```

**Binary naming:** Name the binary `cargo-mytool` and it can be invoked as:
- `cargo-mytool check`
- `cargo mytool check`

---

## 7. CLI UX Best Practices

### Helpful Error Messages

```rust
use clap::Parser;

#[derive(Parser)]
#[command(name = "myapp")]
#[command(author, version, about)]
#[command(
    after_help = "Examples:\n  myapp --input file.txt\n  myapp -v --format json input.csv"
)]
struct Args {
    /// Input file to process
    #[arg(short, long, value_name = "FILE")]
    input: PathBuf,
}
```

### Argument Groups

```rust
use clap::{Parser, ArgGroup};

#[derive(Parser)]
#[command(group(
    ArgGroup::new("source")
        .required(true)
        .args(["file", "url"]),
))]
struct Args {
    /// Read from file
    #[arg(long)]
    file: Option<PathBuf>,

    /// Read from URL
    #[arg(long)]
    url: Option<String>,
}
```

### Conflicting Arguments

```rust
#[derive(Parser)]
struct Args {
    /// Enable quiet mode
    #[arg(short, long, conflicts_with = "verbose")]
    quiet: bool,

    /// Enable verbose mode
    #[arg(short, long)]
    verbose: bool,
}
```

### Required Unless

```rust
#[derive(Parser)]
struct Args {
    /// Config file (required unless --init is used)
    #[arg(long, required_unless_present = "init")]
    config: Option<PathBuf>,

    /// Initialize new config
    #[arg(long)]
    init: bool,
}
```

### Progress and User Feedback

Combine clap with good CLI UX patterns:

```rust
use std::io::{self, IsTerminal};

fn main() {
    let args = Args::parse();

    // Check if stdout is a terminal for progress indicators
    let use_progress = io::stdout().is_terminal();

    if use_progress {
        // Show spinner, progress bar, colors
        println!("Processing...");
    } else {
        // Machine-readable output for pipes
        eprintln!("Starting process");
    }
}
```

---

## 8. Exit Codes

### Standard Conventions

Programs should emit appropriate exit codes: **0 for success**, non-zero for errors. Rather than inventing custom codes, follow BSD conventions via the `exitcode` crate.

### Basic Pattern

```rust
use std::process::ExitCode;

fn main() -> ExitCode {
    let args = Args::parse();

    match run(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("Error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: Args) -> anyhow::Result<()> {
    // Application logic here
    Ok(())
}
```

### Semantic Exit Codes with exitcode Crate

```rust
use exitcode::{self, ExitCode};

fn main() {
    match run() {
        Ok(()) => std::process::exit(exitcode::OK),
        Err(e) => {
            eprintln!("Error: {e}");
            match e {
                AppError::ConfigNotFound(_) => std::process::exit(exitcode::CONFIG),
                AppError::InvalidInput(_) => std::process::exit(exitcode::DATAERR),
                AppError::IoError(_) => std::process::exit(exitcode::IOERR),
                AppError::PermissionDenied => std::process::exit(exitcode::NOPERM),
                _ => std::process::exit(exitcode::SOFTWARE),
            }
        }
    }
}
```

### Common Exit Codes (BSD Standard)

| Code | Constant | Meaning |
|------|----------|---------|
| 0 | `exitcode::OK` | Success |
| 64 | `exitcode::USAGE` | Command line usage error |
| 65 | `exitcode::DATAERR` | Data format error |
| 66 | `exitcode::NOINPUT` | Cannot open input |
| 73 | `exitcode::CANTCREAT` | Cannot create output file |
| 74 | `exitcode::IOERR` | I/O error |
| 77 | `exitcode::NOPERM` | Permission denied |
| 78 | `exitcode::CONFIG` | Configuration error |

### Best Practices

- **Always exit 0 on success** - scripts depend on this
- **Use stderr for errors** - `eprintln!()` not `println!()`
- **Be consistent** - same error type → same exit code
- **Document non-standard codes** - if you must use custom codes

---

## 9. Signal Handling

### When You Need Signal Handling

Signal handling is essential when your application needs graceful shutdown:
- Cleaning up temporary files
- Closing network connections
- Saving state before exit
- Restoring terminal settings

For simple CLIs, the default OS handling (immediate termination) is often fine.

### Basic Ctrl+C Handling

```rust
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

fn main() -> anyhow::Result<()> {
    // Shared flag for shutdown signaling
    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();

    ctrlc::set_handler(move || {
        r.store(false, Ordering::SeqCst);
        eprintln!("\nReceived Ctrl+C, shutting down...");
    })?;

    // Main loop checks the flag
    while running.load(Ordering::SeqCst) {
        // Do work...
        process_item()?;
    }

    // Cleanup
    cleanup()?;
    Ok(())
}
```

### Channel-Based Approach

For more complex scenarios, use channels to coordinate shutdown:

```rust
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

fn main() -> anyhow::Result<()> {
    let (tx, rx) = mpsc::channel();

    ctrlc::set_handler(move || {
        tx.send(()).expect("Could not send signal");
    })?;

    // Work loop with timeout checks
    loop {
        // Check for shutdown signal (non-blocking)
        match rx.try_recv() {
            Ok(_) => {
                eprintln!("Shutting down gracefully...");
                break;
            }
            Err(mpsc::TryRecvError::Empty) => {}
            Err(mpsc::TryRecvError::Disconnected) => break,
        }

        // Do work
        do_work()?;
    }

    cleanup()?;
    Ok(())
}
```

### Async Signal Handling (Tokio)

```rust
use tokio::signal;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let shutdown = async {
        signal::ctrl_c()
            .await
            .expect("Failed to install Ctrl+C handler");
        eprintln!("\nReceived Ctrl+C");
    };

    tokio::select! {
        _ = run_server() => {}
        _ = shutdown => {
            eprintln!("Shutting down...");
        }
    }

    cleanup().await?;
    Ok(())
}
```

### Handling Multiple Ctrl+C Presses

Users expect immediate termination on repeated Ctrl+C:

```rust
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

fn main() -> anyhow::Result<()> {
    let ctrl_c_count = Arc::new(AtomicUsize::new(0));
    let counter = ctrl_c_count.clone();

    ctrlc::set_handler(move || {
        let count = counter.fetch_add(1, Ordering::SeqCst);
        if count >= 1 {
            eprintln!("\nForce quit!");
            std::process::exit(130); // 128 + SIGINT(2)
        }
        eprintln!("\nShutting down... (press Ctrl+C again to force quit)");
    })?;

    // ... rest of application
    Ok(())
}
```

### Cross-Platform Considerations

- **Unix**: Traditional signal handling (SIGINT, SIGTERM, etc.)
- **Windows**: Console handlers, different mechanism

The `ctrlc` crate abstracts these differences for Ctrl+C handling. For broader signal support on Unix, use the `signal-hook` crate.

---

## 10. Configuration Files

### The confy Crate (Simple Approach)

For straightforward configuration needs, `confy` handles platform-specific paths automatically:

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Default)]
struct AppConfig {
    api_endpoint: String,
    timeout_seconds: u64,
    verbose: bool,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            api_endpoint: "https://api.example.com".to_string(),
            timeout_seconds: 30,
            verbose: false,
        }
    }
}

fn main() -> anyhow::Result<()> {
    // Loads from platform-specific location, creates default if missing
    let cfg: AppConfig = confy::load("myapp", None)?;

    println!("Using endpoint: {}", cfg.api_endpoint);
    Ok(())
}
```

**Platform locations:**
- Linux: `~/.config/myapp/default-config.toml`
- macOS: `~/Library/Application Support/myapp/default-config.toml`
- Windows: `C:\Users\<User>\AppData\Roaming\myapp\default-config.toml`

### Manual Configuration with directories Crate

For more control, use `directories` to find paths and handle loading yourself:

```rust
use directories::ProjectDirs;
use std::fs;
use std::path::PathBuf;

fn config_path() -> Option<PathBuf> {
    ProjectDirs::from("com", "mycompany", "myapp")
        .map(|dirs| dirs.config_dir().join("config.toml"))
}

fn load_config() -> anyhow::Result<AppConfig> {
    let path = config_path()
        .ok_or_else(|| anyhow::anyhow!("Could not determine config directory"))?;

    if path.exists() {
        let contents = fs::read_to_string(&path)?;
        let config: AppConfig = toml::from_str(&contents)?;
        Ok(config)
    } else {
        Ok(AppConfig::default())
    }
}

fn save_config(config: &AppConfig) -> anyhow::Result<()> {
    let path = config_path()
        .ok_or_else(|| anyhow::anyhow!("Could not determine config directory"))?;

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let contents = toml::to_string_pretty(config)?;
    fs::write(&path, contents)?;
    Ok(())
}
```

### Configuration Priority (CLI > Env > File > Default)

Combine clap with config files for flexible configuration:

```rust
use clap::Parser;

#[derive(Parser)]
struct Args {
    /// Config file path (overrides default location)
    #[arg(long, short)]
    config: Option<PathBuf>,

    /// API endpoint (overrides config file)
    #[arg(long, env = "MYAPP_ENDPOINT")]
    endpoint: Option<String>,

    /// Timeout in seconds (overrides config file)
    #[arg(long, env = "MYAPP_TIMEOUT")]
    timeout: Option<u64>,
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    // Load config file
    let mut config = if let Some(path) = &args.config {
        load_config_from(path)?
    } else {
        load_default_config()?
    };

    // CLI/env args override config file
    if let Some(endpoint) = args.endpoint {
        config.api_endpoint = endpoint;
    }
    if let Some(timeout) = args.timeout {
        config.timeout_seconds = timeout;
    }

    // Use final merged config
    run(config)
}
```

### Configuration File Formats

| Format | Crate | Pros | Cons |
|--------|-------|------|------|
| TOML | `toml` | Human-readable, Rust-native | Less common outside Rust |
| JSON | `serde_json` | Universal | No comments, verbose |
| YAML | `serde_yaml` | Human-readable, comments | Whitespace-sensitive |
| RON | `ron` | Rust-like syntax | Rust-specific |

**Recommendation:** TOML for user-facing configs (comments, readable), JSON for machine-generated.

---

## Common Dependencies

```toml
[dependencies]
clap = { version = "4", features = ["derive", "env"] }
anyhow = "1.0"              # Error handling

# Exit codes
exitcode = "1.1"            # BSD standard exit codes

# Signal handling
ctrlc = "3.4"               # Cross-platform Ctrl+C
# signal-hook = "0.3"       # Advanced Unix signals (if needed)

# Configuration
confy = "0.6"               # Simple config management
# directories = "5.0"       # Platform-specific paths (manual approach)
toml = "0.8"                # TOML parsing
serde = { version = "1.0", features = ["derive"] }
```

**clap feature flags:**
- `derive` - Enable derive macros (recommended)
- `env` - Enable environment variable support
- `cargo` - Enable cargo-specific features
- `wrap_help` - Wrap help text to terminal width
- `unicode` - Full Unicode support in help

---

## Quick Reference

### Clap Attributes

| Pattern | Attribute | Example |
|---------|-----------|---------|
| Short flag | `#[arg(short)]` | `-v` |
| Long flag | `#[arg(long)]` | `--verbose` |
| Both | `#[arg(short, long)]` | `-v`, `--verbose` |
| Required | Default for non-Option | Must provide |
| Optional | `Option<T>` | Can omit |
| Default | `#[arg(default_value = "x")]` | Falls back to "x" |
| Env fallback | `#[arg(env = "VAR")]` | Uses $VAR if not provided |
| Multiple | `Vec<T>` | Can repeat |
| Count | `#[arg(action = Count)]` | `-vvv` → 3 |
| Positional last | `#[arg(last = true)]` | After `--` |
| Value enum | `#[arg(value_enum)]` | Restricted choices |
| Subcommand | `#[command(subcommand)]` | Nested commands |

### Exit Codes (exitcode crate)

| Code | Constant | Use When |
|------|----------|----------|
| 0 | `OK` | Success |
| 64 | `USAGE` | Bad command line |
| 65 | `DATAERR` | Invalid input data |
| 74 | `IOERR` | I/O error |
| 78 | `CONFIG` | Config problem |

### Signal Handling Patterns

| Pattern | Crate | Use Case |
|---------|-------|----------|
| Ctrl+C flag | `ctrlc` | Simple graceful shutdown |
| Channel-based | `ctrlc` + `mpsc` | Multi-threaded shutdown |
| Async | `tokio::signal` | Async applications |
| Multiple signals | `signal-hook` | Unix-specific handling |

### Config Priority

```
CLI args > Environment vars > Config file > Defaults
```

---

## Related Topics

- **[CLI User Feedback](fundamentals.md#5-cli-user-feedback-for-file-operations)** - Informative output patterns
- **[TTY Detection](fundamentals.md#4-tty-detection-for-colored-output)** - Terminal-aware output
- **[Error Handling](error-handling.md)** - Graceful error messages

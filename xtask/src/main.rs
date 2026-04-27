//! Cross-platform automation entry point for kiro-market hooks.
//!
//! Invoked by Claude Code `PostToolUse` hooks via `cargo xtask <subcommand>`.
//! Reads the hook's stdin JSON payload, dispatches based on tool input, and
//! returns clippy findings (if any) on stdout for the model to consume.

use std::env;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

fn main() -> Result<()> {
    let mut args = env::args().skip(1);
    let cmd = args
        .next()
        .context("missing subcommand (expected: hook-post-edit | hook-block-cargo-lock)")?;
    match cmd.as_str() {
        "hook-post-edit" => hook_post_edit(),
        "hook-block-cargo-lock" => hook_block_cargo_lock(),
        other => bail!("unknown xtask subcommand: {other}"),
    }
}

fn hook_post_edit() -> Result<()> {
    let Some(file_path) = read_file_path_from_stdin()? else {
        return Ok(());
    };
    if file_path.extension().and_then(|e| e.to_str()) != Some("rs") {
        return Ok(());
    }
    if !file_path.is_file() {
        return Ok(());
    }
    run_rustfmt(&file_path);
    if let Some(pkg) = derive_package(&file_path) {
        run_clippy(pkg);
    }
    Ok(())
}

fn read_file_path_from_stdin() -> Result<Option<PathBuf>> {
    let mut input = String::new();
    std::io::stdin()
        .read_to_string(&mut input)
        .context("failed to read hook input from stdin")?;
    let json: serde_json::Value =
        serde_json::from_str(&input).context("hook input was not valid JSON")?;
    Ok(json
        .pointer("/tool_input/file_path")
        .and_then(|v| v.as_str())
        .map(PathBuf::from))
}

fn run_rustfmt(file: &Path) {
    match Command::new("rustfmt")
        .args(["--edition", "2024"])
        .arg(file)
        .status()
    {
        Ok(status) if !status.success() => {
            eprintln!("rustfmt exited {status} for {}", file.display());
        }
        Err(e) => {
            eprintln!("could not invoke rustfmt: {e}");
        }
        Ok(_) => {}
    }
}

fn run_clippy(pkg: &str) {
    let cwd = env::var_os("CLAUDE_PROJECT_DIR")
        .map(PathBuf::from)
        .or_else(|| env::current_dir().ok())
        .unwrap_or_else(|| Path::new(".").to_path_buf());

    let output = match Command::new("cargo")
        .current_dir(&cwd)
        .args([
            "clippy",
            "--package",
            pkg,
            "--no-deps",
            "--message-format=short",
            "--",
            "-D",
            "warnings",
        ])
        .output()
    {
        Ok(o) => o,
        Err(e) => {
            eprintln!("could not invoke cargo clippy: {e}");
            return;
        }
    };

    let mut combined = String::from_utf8_lossy(&output.stdout).into_owned();
    combined.push_str(&String::from_utf8_lossy(&output.stderr));

    let lines: Vec<&str> = combined.lines().collect();
    let tail_start = lines.len().saturating_sub(40);
    let tail = &lines[tail_start..];

    let has_issue = tail
        .iter()
        .any(|l| l.starts_with("error") || l.starts_with("warning"));
    if has_issue {
        println!("clippy ({pkg}) flagged issues:");
        for line in tail {
            println!("{line}");
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum LockfileDecision {
    Allow,
    Block,
}

fn evaluate_lockfile_edit(file: &Path, allow_override: bool) -> LockfileDecision {
    let is_lockfile = file.file_name().and_then(|n| n.to_str()) == Some("Cargo.lock");
    if !is_lockfile || allow_override {
        LockfileDecision::Allow
    } else {
        LockfileDecision::Block
    }
}

fn hook_block_cargo_lock() -> Result<()> {
    let Some(file_path) = read_file_path_from_stdin()? else {
        return Ok(());
    };
    let allow_override = env::var("KIRO_ALLOW_LOCKFILE_EDIT").as_deref() == Ok("1");
    if evaluate_lockfile_edit(&file_path, allow_override) == LockfileDecision::Allow {
        return Ok(());
    }
    eprintln!(
        "Blocked: direct edit to Cargo.lock.

The workspace Cargo.toml pins `curl = \"0.4\"` as a feature-unification shim for
gix-transport. Lockfile churn from unrelated edits can shift curl-sys's TLS
feature resolution and break Windows HTTPS clones.

To proceed:
  1. If this lockfile change is the result of a dep bump, regenerate it via
     `cargo update -p <crate>` instead of editing directly.
  2. To override this guard for one session, export KIRO_ALLOW_LOCKFILE_EDIT=1
     and retry."
    );
    std::process::exit(2);
}

fn derive_package(file: &Path) -> Option<&'static str> {
    let mut iter = file.components();
    while let Some(c) = iter.next() {
        if c.as_os_str() == "crates" {
            let name = iter.next()?.as_os_str().to_str()?;
            return match name {
                "kiro-market-core" => Some("kiro-market-core"),
                "kiro-market" => Some("kiro-market"),
                "kiro-control-center" => Some("kiro-control-center"),
                _ => None,
            };
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derive_package_recognizes_market_core() {
        let p = Path::new("/repo/crates/kiro-market-core/src/lib.rs");
        assert_eq!(derive_package(p), Some("kiro-market-core"));
    }

    #[test]
    fn derive_package_recognizes_control_center() {
        let p = Path::new("/repo/crates/kiro-control-center/src-tauri/src/lib.rs");
        assert_eq!(derive_package(p), Some("kiro-control-center"));
    }

    #[test]
    fn derive_package_recognizes_cli() {
        let p = Path::new("/repo/crates/kiro-market/src/main.rs");
        assert_eq!(derive_package(p), Some("kiro-market"));
    }

    #[test]
    #[cfg(windows)]
    fn derive_package_handles_windows_paths() {
        let p = Path::new(r"C:\repo\crates\kiro-market-core\src\lib.rs");
        assert_eq!(derive_package(p), Some("kiro-market-core"));
    }

    #[test]
    fn derive_package_returns_none_for_unrelated_paths() {
        assert_eq!(derive_package(Path::new("/tmp/random/file.rs")), None);
    }

    #[test]
    fn derive_package_returns_none_for_unknown_crate() {
        let p = Path::new("/repo/crates/some-other-crate/src/lib.rs");
        assert_eq!(derive_package(p), None);
    }

    #[test]
    fn lockfile_edit_blocked_by_default() {
        let p = Path::new("/repo/Cargo.lock");
        assert_eq!(evaluate_lockfile_edit(p, false), LockfileDecision::Block);
    }

    #[test]
    fn lockfile_edit_allowed_when_override_set() {
        let p = Path::new("/repo/Cargo.lock");
        assert_eq!(evaluate_lockfile_edit(p, true), LockfileDecision::Allow);
    }

    #[test]
    fn non_lockfile_edits_pass_through() {
        let p = Path::new("/repo/src/main.rs");
        assert_eq!(evaluate_lockfile_edit(p, false), LockfileDecision::Allow);
    }

    #[test]
    fn similarly_named_files_are_not_lockfiles() {
        // The original bash glob `*Cargo.lock` would have matched these by accident.
        let p = Path::new("/repo/Cargo.lockfile");
        assert_eq!(evaluate_lockfile_edit(p, false), LockfileDecision::Allow);
    }
}

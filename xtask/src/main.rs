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

mod plan_lint;

fn main() -> Result<()> {
    let mut args = env::args().skip(1);
    let cmd = args.next().context(
        "missing subcommand (expected: hook-post-edit | hook-block-cargo-lock | plan-lint)",
    )?;
    match cmd.as_str() {
        "hook-post-edit" => hook_post_edit(),
        "hook-block-cargo-lock" => hook_block_cargo_lock(),
        "plan-lint" => plan_lint::run(args),
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
        run_clippy(&pkg);
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

/// Walk up from `file`'s parent looking for the nearest `Cargo.toml` with
/// a `[package]` table and return its `name`. Virtual manifests (workspace
/// roots without `[package]`) are skipped — the walk continues past them.
/// Returns `None` when the parent chain is exhausted with no usable manifest.
///
/// Returning `String` (not `&'static str`) means new crates added to the
/// workspace are picked up automatically — no per-crate code change here
/// is needed.
///
/// Read and TOML errors are logged to stderr and the walk continues. A
/// final stderr diagnostic on exhaust distinguishes "outside any workspace"
/// from "every ancestor manifest was unreadable / malformed", so the hook
/// produces observable signal rather than silently doing nothing.
fn derive_package(file: &Path) -> Option<String> {
    let start = file.parent()?;
    for dir in start.ancestors() {
        let manifest = dir.join("Cargo.toml");
        if !manifest.is_file() {
            continue;
        }
        let text = match std::fs::read_to_string(&manifest) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("could not read {}: {e}", manifest.display());
                continue;
            }
        };
        match parse_package_name(&text) {
            Ok(Some(name)) => return Some(name),
            // Virtual manifest (workspace root): keep walking.
            Ok(None) => {}
            Err(e) => {
                eprintln!("could not parse {} as TOML: {e}", manifest.display());
            }
        }
    }
    eprintln!(
        "derive_package: no `[package]` Cargo.toml found in ancestors of {}; skipping clippy",
        file.display()
    );
    None
}

/// Parse `Cargo.toml` text and return the `[package].name`, or `None` if
/// the manifest is virtual (e.g. workspace root with no `[package]`).
/// Pure helper extracted from [`derive_package`] so the TOML logic can
/// be unit-tested without touching the filesystem.
fn parse_package_name(toml_text: &str) -> Result<Option<String>, toml::de::Error> {
    let parsed: CargoManifest = toml::from_str(toml_text)?;
    Ok(parsed.package.map(|p| p.name))
}

#[derive(serde::Deserialize)]
struct CargoManifest {
    package: Option<CargoPackage>,
}

#[derive(serde::Deserialize)]
struct CargoPackage {
    name: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_package_name_extracts_simple_name() {
        let toml = r#"
[package]
name = "my-crate"
version = "0.1.0"
"#;
        assert_eq!(
            parse_package_name(toml).unwrap().as_deref(),
            Some("my-crate")
        );
    }

    #[test]
    fn parse_package_name_returns_none_for_virtual_manifest() {
        let toml = r#"
[workspace]
members = ["a", "b"]
"#;
        assert_eq!(parse_package_name(toml).unwrap(), None);
    }

    #[test]
    fn parse_package_name_surfaces_toml_errors() {
        // Truncated table header — invalid TOML.
        assert!(parse_package_name("[package\nname =").is_err());
    }

    #[test]
    fn derive_package_walks_up_to_xtask_manifest() {
        // The xtask crate's own source files must resolve to `xtask`.
        // Anchored on CARGO_MANIFEST_DIR so the test works regardless of
        // where `cargo test` was invoked from.
        let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        let this_file = manifest_dir.join("src").join("main.rs");
        assert_eq!(derive_package(&this_file).as_deref(), Some("xtask"));
    }

    #[test]
    fn derive_package_walks_up_through_workspace_to_member_crate() {
        // A file inside `crates/kiro-market-core/src/` must resolve to
        // `kiro-market-core` (the member's `[package].name`), skipping
        // the workspace's virtual root manifest.
        let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask is a workspace member, parent is workspace root");
        let core_lib = workspace_root
            .join("crates")
            .join("kiro-market-core")
            .join("src")
            .join("lib.rs");
        assert_eq!(
            derive_package(&core_lib).as_deref(),
            Some("kiro-market-core")
        );
    }

    #[test]
    fn derive_package_returns_none_outside_any_workspace() {
        // A path with no `Cargo.toml` anywhere up its parent chain.
        let nowhere = Path::new("/tmp/definitely-not-a-cargo-project/file.rs");
        assert_eq!(derive_package(nowhere), None);
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

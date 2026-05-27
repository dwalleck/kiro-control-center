//! Cross-platform automation entry point for kiro-market hooks.
//!
//! Invoked by Claude Code `PostToolUse` and `Stop` hooks via
//! `cargo xtask <subcommand>`. Reads the hook's stdin JSON payload, dispatches
//! per subcommand, and surfaces findings on stdout — as plain text for
//! `PostToolUse` (where the transcript shows hook stdout on exit 0) and as a
//! `{"systemMessage": ...}` JSON envelope for `Stop` (where plain stdout goes
//! to the debug log only).

use std::env;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use anyhow::{Context, Result, bail};

mod comment_lint;
mod plan_lint;

/// Exit codes documented by `cargo xtask plan-lint --help`:
/// `0` = clean, `1` = lint findings, `2` = internal error.
/// The hook subcommands only ever return 0 or 2 (any error is internal).
fn main() -> ExitCode {
    match dispatch() {
        Ok(code) => ExitCode::from(code),
        Err(e) => {
            eprintln!("Error: {e:?}");
            ExitCode::from(2)
        }
    }
}

fn dispatch() -> Result<u8> {
    let mut args = env::args().skip(1);
    let cmd = args.next().context(
        "missing subcommand (expected: hook-post-edit | hook-stop-frontend-check | hook-block-cargo-lock | plan-lint | comment-lint)",
    )?;
    match cmd.as_str() {
        "hook-post-edit" => {
            hook_post_edit()?;
            Ok(0)
        }
        "hook-stop-frontend-check" => {
            hook_stop_frontend_check();
            Ok(0)
        }
        "hook-block-cargo-lock" => {
            hook_block_cargo_lock()?;
            Ok(0)
        }
        "plan-lint" => {
            let findings = plan_lint::run(args)?;
            Ok(u8::from(findings > 0))
        }
        "comment-lint" => {
            let findings = comment_lint::run(args)?;
            Ok(u8::from(findings > 0))
        }
        other => bail!("unknown xtask subcommand: {other}"),
    }
}

fn hook_post_edit() -> Result<()> {
    let input = read_hook_input()?;
    let Some(file_path) = input.file_path else {
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
        let workspace_dir = resolve_workspace_dir(input.cwd.as_deref())?;
        run_clippy(&pkg, &workspace_dir);
    }
    Ok(())
}

/// Stop-hook entry point: runs `npm run check` (svelte-check + tsc) at end-of-turn
/// when any TypeScript or Svelte file under `crates/kiro-control-center/` is dirty
/// in git. A pure-Rust turn pays zero cost; a frontend turn pays one ~5–15s check.
///
/// Findings (when present) are emitted as a Claude Code `systemMessage` JSON
/// envelope on stdout so the transcript surfaces them. Returns `()` rather
/// than `Result` because the Stop hook MUST NOT abort a turn over an
/// infrastructure hiccup — every error path here is swallowed at the source
/// (logged via `eprintln!` or surfaced via `emit_system_message`).
fn hook_stop_frontend_check() {
    let input = match read_hook_input() {
        Ok(i) => i,
        Err(e) => {
            eprintln!("hook-stop-frontend-check: skipping ({e})");
            return;
        }
    };
    let workspace_dir = match resolve_workspace_dir(input.cwd.as_deref()) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("hook-stop-frontend-check: skipping ({e})");
            return;
        }
    };
    if !frontend_files_dirty(&workspace_dir) {
        return;
    }
    run_svelte_check(&workspace_dir);
}

/// Classification of a non-zero `git status` exit's stderr message.
#[derive(Debug, PartialEq, Eq)]
enum GitFailureKind {
    /// Expected, user-can't-do-anything-about-it cases: workspace isn't a git
    /// repo, or the directory disappeared (subagent worktree cleaned up).
    Benign,
    /// Real symptom the user needs to know about: index corruption, permission
    /// denied, lockfile contention, etc.
    Surfaceable,
}

fn classify_git_status_failure(stderr: &str) -> GitFailureKind {
    let s = stderr.to_lowercase();
    if s.contains("not a git repository") || s.contains("no such file or directory") {
        GitFailureKind::Benign
    } else {
        GitFailureKind::Surfaceable
    }
}

/// Returns true if `git status --porcelain` reports any `.ts` / `.svelte` file
/// under `crates/kiro-control-center/` as modified, added, renamed, or untracked.
///
/// Failure-mode policy:
/// - Missing workspace dir → silent skip (the most common cause is a stale
///   `cwd` from a subagent worktree that was cleaned up between edit and stop).
/// - `git` spawn error → surface via `systemMessage` (user-actionable: install
///   git / put it on PATH).
/// - Non-zero `git status` exit → classified: benign cases (not a git repo)
///   skip silently; everything else (index corrupt, permissions, lockfile
///   contention) surfaces via `systemMessage`.
///
/// Always returns `false` on any failure path so the Stop hook never aborts.
fn frontend_files_dirty(project_dir: &Path) -> bool {
    if !project_dir.is_dir() {
        eprintln!(
            "frontend_files_dirty: workspace dir {} not found; skipping",
            project_dir.display()
        );
        return false;
    }
    // `-c core.quotePath=false` so non-ASCII filenames (e.g. translation files
    // named `Header_日本語.svelte`) come through as literal UTF-8 instead of as
    // quoted, octal-escaped `"crates/.../Header_\346\227\245..."`. The quoted
    // form would fail the prefix check in `is_frontend_path` and silently
    // skip the very files this hook exists to catch.
    let output = match Command::new("git")
        .current_dir(project_dir)
        .args(["-c", "core.quotePath=false", "status", "--porcelain"])
        .output()
    {
        Ok(o) => o,
        Err(e) => {
            emit_system_message(&format!(
                "hook-stop-frontend-check could not invoke `git status`: {e}\n\
                 Ensure `git` is on PATH."
            ));
            return false;
        }
    };
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        match classify_git_status_failure(&stderr) {
            GitFailureKind::Benign => {
                eprintln!(
                    "frontend_files_dirty: workspace not in a git repo; skipping ({})",
                    stderr.trim()
                );
            }
            GitFailureKind::Surfaceable => {
                emit_system_message(&format!(
                    "hook-stop-frontend-check: `git status` failed unexpectedly\n\
                     exit: {}\n\
                     stderr: {}",
                    output.status,
                    stderr.trim()
                ));
            }
        }
        return false;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_dirty_paths_from_git_status(&stdout)
        .iter()
        .any(|p| is_frontend_path(p))
}

/// Parse the porcelain v1 format. Each entry is `XY <path>` or, for renames,
/// `R  <old> -> <new>` (we take the destination). Drops the first 3 columns,
/// then for renames keeps the post-`-> ` portion.
fn parse_dirty_paths_from_git_status(porcelain: &str) -> Vec<PathBuf> {
    porcelain
        .lines()
        .filter_map(|line| {
            if line.len() < 4 {
                return None;
            }
            let rest = &line[3..];
            // Only `R`-prefixed (rename) and `C`-prefixed (copy) lines carry the
            // `old -> new` form; for every other status the path is the whole
            // tail and may legitimately contain ` -> ` as part of the filename.
            // Splitting unconditionally would silently truncate such names.
            let path = if line.starts_with('R') || line.starts_with('C') {
                rest.rsplit_once(" -> ").map_or(rest, |(_, dst)| dst)
            } else {
                rest
            };
            Some(PathBuf::from(path))
        })
        .collect()
}

/// True for `.ts` / `.svelte` files anywhere under `crates/kiro-control-center/`.
/// Paths are repo-relative (as `git status` reports them). `git` emits `/`
/// separators on every platform we run on, but we normalize backslashes
/// defensively (see `is_frontend_path_normalizes_backslashes`) so callers
/// that synthesize their own paths during testing — or future code paths
/// that hand us OS-native paths — still match.
fn is_frontend_path(path: &Path) -> bool {
    let Some(s) = path.to_str() else { return false };
    let normalized = s.replace('\\', "/");
    if !normalized.starts_with("crates/kiro-control-center/") {
        return false;
    }
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("ts" | "svelte")
    )
}

/// Emit a Claude Code `systemMessage` JSON envelope on stdout. For Stop hooks
/// exiting 0, plain stdout goes to the debug log only — the envelope is the
/// documented mechanism for routing content to the transcript so the user and
/// Claude actually see it. Body is JSON-escaped automatically by `serde_json`.
fn emit_system_message(body: &str) {
    let payload = serde_json::json!({ "systemMessage": body });
    println!("{payload}");
}

/// Shell out to `npm run check` from the frontend crate. Findings (when
/// present) are emitted as a `systemMessage` JSON envelope so they surface
/// to the transcript. Cross-platform npm invocation: Windows resolves the
/// `.cmd` shim explicitly because `std::process::Command` does not auto-
/// append `.cmd` extensions.
fn run_svelte_check(project_dir: &Path) {
    let frontend_dir = project_dir.join("crates").join("kiro-control-center");
    if !frontend_dir.is_dir() {
        // Internal-only — this means the resolved workspace dir is something
        // other than the kiro repo root, which is a config issue the user
        // can't act on from a transcript message.
        eprintln!(
            "run_svelte_check: {} not found; skipping",
            frontend_dir.display()
        );
        return;
    }

    #[cfg(windows)]
    let npm = "npm.cmd";
    #[cfg(not(windows))]
    let npm = "npm";

    let output = match Command::new(npm)
        .current_dir(&frontend_dir)
        .args(["run", "check"])
        .output()
    {
        Ok(o) => o,
        Err(e) => {
            // User-actionable: npm missing from PATH, Node not installed.
            emit_system_message(&format!(
                "hook-stop-frontend-check could not invoke `{npm} run check`: {e}\n\
                 Install Node.js or ensure `npm` is on PATH to enable frontend type checking."
            ));
            return;
        }
    };

    let mut combined = String::from_utf8_lossy(&output.stdout).into_owned();
    combined.push_str(&String::from_utf8_lossy(&output.stderr));

    let lines: Vec<&str> = combined.lines().collect();
    let tail_start = lines.len().saturating_sub(40);
    let tail = &lines[tail_start..];

    // Status code is the authoritative signal — `svelte-check` exits non-zero
    // when (and only when) it finds errors. No belt-and-suspenders substring
    // scan: the previous `"Error" / "Warn"` patterns false-positive on file
    // names like `ErrorBoundary.svelte` and aren't needed once the status
    // signal is trusted.
    if !output.status.success() {
        let mut body = String::from("svelte-check flagged issues:\n");
        for line in tail {
            body.push_str(line);
            body.push('\n');
        }
        emit_system_message(&body);
    }
}

/// Fields the xtask hooks care about, projected from the JSON stdin payload
/// Claude Code sends. Both are `Option` because Stop hooks omit `tool_input`
/// entirely, and the rest of the wire format is intentionally not modeled
/// (the xtask only acts on these two fields).
#[derive(Debug, Default, PartialEq, Eq)]
struct HookInput {
    file_path: Option<PathBuf>,
    cwd: Option<PathBuf>,
}

/// Private wire-format struct the JSON deserializes into. The flat `HookInput`
/// above is the post-projection shape. Routing through serde here means a
/// wrong-typed field (e.g. `cwd: 42`) surfaces as a parse error rather than
/// silently becoming `None` — that was the previous manual-pointer behavior.
#[derive(Debug, Default, serde::Deserialize)]
#[serde(default)]
struct HookInputWire {
    tool_input: Option<ToolInputWire>,
    cwd: Option<PathBuf>,
}

#[derive(Debug, Default, serde::Deserialize)]
#[serde(default)]
struct ToolInputWire {
    file_path: Option<PathBuf>,
}

fn read_hook_input() -> Result<HookInput> {
    let mut buf = String::new();
    std::io::stdin()
        .read_to_string(&mut buf)
        .context("failed to read hook input from stdin")?;
    parse_hook_input(&buf)
}

/// Parses the hook stdin payload via serde. Empty-or-whitespace-only input
/// returns `HookInput::default()` so manual smoke tests (`echo '' | cargo xtask
/// ...`) don't error out before the soft-fail paths can run. Real syntactically-
/// invalid JSON surfaces as an `Err` — see `hook_stop_frontend_check` for the
/// Stop-hook policy of swallowing such errors at the entry point.
fn parse_hook_input(text: &str) -> Result<HookInput> {
    if text.trim().is_empty() {
        return Ok(HookInput::default());
    }
    let wire: HookInputWire =
        serde_json::from_str(text).context("hook input was not valid JSON")?;
    Ok(HookInput {
        file_path: wire.tool_input.and_then(|t| t.file_path),
        cwd: wire.cwd,
    })
}

/// Resolves the workspace dir from the live environment, deferring to the
/// pure `_inner` helper so tests can exercise the priority chain without
/// touching real env vars (which is unsound under cargo test's parallelism).
fn resolve_workspace_dir(stdin_cwd: Option<&Path>) -> Result<PathBuf> {
    let env_var = env::var_os("CLAUDE_PROJECT_DIR");
    let cwd = env::current_dir().ok();
    resolve_workspace_dir_inner(stdin_cwd, env_var.as_deref(), cwd.as_deref())
}

/// Pure priority chain:
/// 1. `cwd` from hook stdin — Claude Code populates this per tool call, so a
///    subagent operating in an isolated worktree gets its workspace dir from
///    the worktree path (not the parent session's project root).
/// 2. `CLAUDE_PROJECT_DIR` env var — set at session launch; matches the dir
///    `claude` was started in.
/// 3. The process's current directory — last fallback for manual `cargo xtask`
///    invocations outside Claude Code.
///
/// Returns `Err` when all three are unresolvable. The previous `Path::new(".")`
/// fallback "succeeded" in a state where every downstream call would misbehave
/// — bailing makes the failure mode honest.
fn resolve_workspace_dir_inner(
    stdin_cwd: Option<&Path>,
    env_var: Option<&std::ffi::OsStr>,
    cwd: Option<&Path>,
) -> Result<PathBuf> {
    if let Some(p) = stdin_cwd {
        return Ok(p.to_path_buf());
    }
    if let Some(v) = env_var {
        return Ok(PathBuf::from(v));
    }
    if let Some(p) = cwd {
        return Ok(p.to_path_buf());
    }
    bail!(
        "could not determine workspace dir: stdin cwd absent, CLAUDE_PROJECT_DIR unset, and current_dir() failed"
    )
}

fn run_rustfmt(file: &Path) {
    // PostToolUse hook stdout is shown in the transcript on exit 0; stderr is
    // hidden by default. Both failure modes here are user-actionable (file
    // has a syntax error rustfmt can't parse; rustfmt isn't on PATH), so
    // route them through stdout via `println!`.
    match Command::new("rustfmt")
        .args(["--edition", "2024"])
        .arg(file)
        .status()
    {
        Ok(status) if !status.success() => {
            println!(
                "hook-post-edit: rustfmt exited {status} for {} (file may have a syntax error)",
                file.display()
            );
        }
        Err(e) => {
            println!(
                "hook-post-edit: could not invoke rustfmt: {e}\n\
                 Install a Rust toolchain or ensure `rustfmt` is on PATH."
            );
        }
        Ok(_) => {}
    }
}

fn run_clippy(pkg: &str, workspace_dir: &Path) {
    let output = match Command::new("cargo")
        .current_dir(workspace_dir)
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
            // User-actionable (cargo missing / not on PATH) — surface via stdout
            // so the PostToolUse transcript shows it. stderr from a hook isn't
            // displayed by default.
            println!("hook-post-edit: could not invoke cargo clippy: {e}");
            return;
        }
    };

    let mut combined = String::from_utf8_lossy(&output.stdout).into_owned();
    combined.push_str(&String::from_utf8_lossy(&output.stderr));

    let lines: Vec<&str> = combined.lines().collect();
    let tail_start = lines.len().saturating_sub(40);
    let tail = &lines[tail_start..];

    // Primary signal: with `-D warnings`, any warning fails the build, so a
    // non-zero exit is the reliable "issues found" indicator. The substring
    // fallback catches the rare case where clippy emits diagnostics but
    // exits 0 (e.g. forced-allow on a sub-target). Note: clippy's
    // `--message-format=short` lines look like `path.rs:N:M: error: ...` —
    // the `.starts_with("error")` filter the original 1aada20 used never
    // matched these, so every diagnostic was being silently dropped.
    let has_issue = !output.status.success()
        || tail
            .iter()
            .any(|l| l.contains(": error:") || l.contains(": warning:"));
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
    let Some(file_path) = read_hook_input()?.file_path else {
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
    use std::ffi::OsStr;

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

    #[test]
    fn git_status_parser_handles_modified_and_untracked() {
        let porcelain = " M crates/kiro-control-center/src/App.svelte\n\
                         ?? crates/kiro-control-center/src/new.ts\n\
                         A  crates/kiro-control-center/src/added.ts\n";
        let paths = parse_dirty_paths_from_git_status(porcelain);
        assert_eq!(
            paths,
            vec![
                PathBuf::from("crates/kiro-control-center/src/App.svelte"),
                PathBuf::from("crates/kiro-control-center/src/new.ts"),
                PathBuf::from("crates/kiro-control-center/src/added.ts"),
            ]
        );
    }

    #[test]
    fn git_status_parser_takes_rename_destination() {
        // Rename entries surface as `R  old/path -> new/path`; we want the destination.
        let porcelain =
            "R  crates/kiro-control-center/src/old.ts -> crates/kiro-control-center/src/new.ts\n";
        let paths = parse_dirty_paths_from_git_status(porcelain);
        assert_eq!(
            paths,
            vec![PathBuf::from("crates/kiro-control-center/src/new.ts")]
        );
    }

    #[test]
    fn git_status_parser_ignores_short_lines() {
        // Empty trailing line from `git status` shouldn't produce a phantom PathBuf("").
        let porcelain = " M foo.rs\n\n";
        let paths = parse_dirty_paths_from_git_status(porcelain);
        assert_eq!(paths, vec![PathBuf::from("foo.rs")]);
    }

    #[test]
    fn git_status_parser_preserves_arrow_substring_in_non_rename_paths() {
        // Defensive: a non-rename status (e.g. `M `, `??`) with a filename that
        // happens to contain ` -> ` must NOT be split. Only `R`- and `C`-prefixed
        // lines carry the `old -> new` rename/copy form. Without the status-gate
        // the path would be silently truncated to whatever follows ` -> `.
        let porcelain = " M crates/kiro-control-center/src/weird -> name.ts\n";
        let paths = parse_dirty_paths_from_git_status(porcelain);
        assert_eq!(
            paths,
            vec![PathBuf::from(
                "crates/kiro-control-center/src/weird -> name.ts"
            )]
        );
    }

    #[test]
    fn git_status_parser_handles_non_ascii_paths_unquoted() {
        // With `-c core.quotePath=false` (set in frontend_files_dirty), non-ASCII
        // paths flow through as literal UTF-8 — no surrounding quotes to strip.
        let porcelain = " M crates/kiro-control-center/src/Header_日本語.svelte\n";
        let paths = parse_dirty_paths_from_git_status(porcelain);
        assert_eq!(
            paths,
            vec![PathBuf::from(
                "crates/kiro-control-center/src/Header_日本語.svelte"
            )]
        );
    }

    #[test]
    fn classify_git_status_failure_treats_not_a_repo_as_benign() {
        let stderr = "fatal: not a git repository (or any of the parent directories): .git";
        assert_eq!(classify_git_status_failure(stderr), GitFailureKind::Benign);
    }

    #[test]
    fn classify_git_status_failure_treats_missing_dir_as_benign() {
        // Subagent worktree was cleaned up between the edit and the Stop hook.
        let stderr = "fatal: No such file or directory";
        assert_eq!(classify_git_status_failure(stderr), GitFailureKind::Benign);
    }

    #[test]
    fn classify_git_status_failure_surfaces_index_corruption() {
        let stderr = "fatal: index file corrupt";
        assert_eq!(
            classify_git_status_failure(stderr),
            GitFailureKind::Surfaceable
        );
    }

    #[test]
    fn classify_git_status_failure_surfaces_permission_denied() {
        let stderr = "fatal: unable to access '.git/HEAD': Permission denied";
        assert_eq!(
            classify_git_status_failure(stderr),
            GitFailureKind::Surfaceable
        );
    }

    #[test]
    fn classify_git_status_failure_surfaces_lockfile_contention() {
        let stderr = "fatal: Unable to create '/repo/.git/index.lock': File exists";
        assert_eq!(
            classify_git_status_failure(stderr),
            GitFailureKind::Surfaceable
        );
    }

    #[test]
    fn git_status_parser_takes_copy_destination() {
        // Copies use the same `<old> -> <new>` shape as renames; the comment on
        // parse_dirty_paths_from_git_status only calls out renames, but the
        // logic handles copies identically. Lock that in.
        let porcelain =
            "C  crates/kiro-control-center/src/a.ts -> crates/kiro-control-center/src/b.ts\n";
        let paths = parse_dirty_paths_from_git_status(porcelain);
        assert_eq!(
            paths,
            vec![PathBuf::from("crates/kiro-control-center/src/b.ts")]
        );
    }

    #[test]
    fn is_frontend_path_accepts_ts_and_svelte_under_frontend_crate() {
        assert!(is_frontend_path(Path::new(
            "crates/kiro-control-center/src/App.svelte"
        )));
        assert!(is_frontend_path(Path::new(
            "crates/kiro-control-center/src/lib/stores/foo.ts"
        )));
    }

    #[test]
    fn is_frontend_path_rejects_other_extensions_and_crates() {
        // Rust files in the frontend crate's src-tauri/ don't count.
        assert!(!is_frontend_path(Path::new(
            "crates/kiro-control-center/src-tauri/src/main.rs"
        )));
        // .ts files outside the frontend crate don't count either.
        assert!(!is_frontend_path(Path::new("scripts/build.ts")));
        // .json under the frontend crate is not handled by svelte-check.
        assert!(!is_frontend_path(Path::new(
            "crates/kiro-control-center/package.json"
        )));
    }

    #[test]
    fn is_frontend_path_accepts_svelte_dot_ts_rune_modules() {
        // Svelte 5 rune modules use the `.svelte.ts` double-extension.
        // `Path::extension()` returns `Some("ts")` for these, so they match —
        // lock that in so a refactor to file-stem-based matching doesn't
        // silently break the convention.
        assert!(is_frontend_path(Path::new(
            "crates/kiro-control-center/src/lib/stores/installPlugin.svelte.ts"
        )));
    }

    #[test]
    fn is_frontend_path_accepts_d_ts_ambient_declarations() {
        // `bindings.ts` is regenerated from Rust types; `.d.ts` ambient files
        // also live under the frontend crate. Both should trigger svelte-check.
        assert!(is_frontend_path(Path::new(
            "crates/kiro-control-center/src/app.d.ts"
        )));
    }

    #[test]
    fn is_frontend_path_normalizes_backslashes() {
        // Windows-style paths must still match the frontend prefix check.
        assert!(is_frontend_path(Path::new(
            r"crates\kiro-control-center\src\App.svelte"
        )));
    }

    #[test]
    fn parse_hook_input_extracts_both_file_path_and_cwd() {
        let json = r#"{
            "session_id": "abc",
            "cwd": "/home/user/repo-worktree-foo",
            "tool_input": { "file_path": "/home/user/repo-worktree-foo/src/main.rs" }
        }"#;
        let parsed = parse_hook_input(json).expect("valid JSON");
        assert_eq!(
            parsed.file_path,
            Some(PathBuf::from("/home/user/repo-worktree-foo/src/main.rs"))
        );
        assert_eq!(
            parsed.cwd,
            Some(PathBuf::from("/home/user/repo-worktree-foo"))
        );
    }

    #[test]
    fn parse_hook_input_tolerates_missing_fields() {
        // Stop hooks have no tool_input; older Claude Code may omit cwd.
        let json = r#"{"session_id": "abc", "hook_event_name": "Stop"}"#;
        let parsed = parse_hook_input(json).expect("valid JSON");
        assert_eq!(parsed, HookInput::default());
    }

    #[test]
    fn parse_hook_input_treats_empty_input_as_default() {
        // Smoke-test invocation (`echo '' | cargo xtask ...`) shouldn't error.
        assert_eq!(parse_hook_input("").unwrap(), HookInput::default());
        assert_eq!(parse_hook_input("   \n").unwrap(), HookInput::default());
    }

    #[test]
    fn parse_hook_input_rejects_malformed_json() {
        // Real corrupt input should surface, not be swallowed.
        assert!(parse_hook_input("{not json").is_err());
    }

    // The priority chain is tested via `_inner` (pure, deterministic) rather
    // than the env-reading `resolve_workspace_dir` wrapper — `std::env::set_var`
    // is unsound under cargo test's thread parallelism.
    #[test]
    fn resolve_workspace_dir_inner_prefers_stdin_cwd() {
        let stdin_cwd = PathBuf::from("/worktree/path");
        let resolved = resolve_workspace_dir_inner(
            Some(&stdin_cwd),
            Some(OsStr::new("/parent/project")),
            Some(Path::new("/cwd")),
        )
        .unwrap();
        assert_eq!(resolved, stdin_cwd);
    }

    #[test]
    fn resolve_workspace_dir_inner_falls_back_to_env_var() {
        let resolved = resolve_workspace_dir_inner(
            None,
            Some(OsStr::new("/parent/project")),
            Some(Path::new("/cwd")),
        )
        .unwrap();
        assert_eq!(resolved, PathBuf::from("/parent/project"));
    }

    #[test]
    fn resolve_workspace_dir_inner_falls_back_to_cwd_when_env_unset() {
        let resolved = resolve_workspace_dir_inner(None, None, Some(Path::new("/cwd"))).unwrap();
        assert_eq!(resolved, PathBuf::from("/cwd"));
    }

    #[test]
    fn resolve_workspace_dir_inner_errors_when_all_sources_absent() {
        // The previous design returned `Path::new(".")` here — a pretend-success
        // that left every downstream call in a broken state. Errors are honest.
        let err = resolve_workspace_dir_inner(None, None, None).unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("could not determine workspace dir"),
            "got: {msg}"
        );
    }
}

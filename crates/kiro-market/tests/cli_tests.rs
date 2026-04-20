mod common;

use common::{run_in_dir, stderr, stdout};
use kiro_market_core::test_utils::path_to_file_url;
use tempfile::TempDir;

#[test]
fn help_shows_usage() {
    let dir = TempDir::new().expect("temp dir");
    let output = run_in_dir(dir.path(), &["--help"]);

    assert!(
        output.status.success(),
        "expected success, got: {}",
        stderr(&output)
    );

    let out = stdout(&output);
    assert!(
        out.contains("kiro-market"),
        "expected 'kiro-market' in help output:\n{out}"
    );
    assert!(
        out.contains("marketplace"),
        "expected 'marketplace' in help output:\n{out}"
    );
}

#[test]
fn version_flag() {
    let dir = TempDir::new().expect("temp dir");
    let output = run_in_dir(dir.path(), &["--version"]);

    assert!(
        output.status.success(),
        "expected success, got: {}",
        stderr(&output)
    );

    let out = stdout(&output);
    assert!(
        out.contains("kiro-market"),
        "expected 'kiro-market' in version output:\n{out}"
    );
    // Version string should contain a semver-like pattern.
    assert!(
        out.contains("0.1.0"),
        "expected version number in output:\n{out}"
    );
}

#[test]
fn marketplace_list_empty() {
    let dir = TempDir::new().expect("temp dir");
    let output = run_in_dir(dir.path(), &["marketplace", "list"]);

    assert!(
        output.status.success(),
        "expected success, got: {}",
        stderr(&output)
    );

    let out = stdout(&output);
    assert!(
        out.contains("No marketplaces registered"),
        "expected 'No marketplaces registered' in output:\n{out}"
    );
}

#[test]
fn stdout_has_no_ansi_escapes_when_piped() {
    // IsTerminal gate must disable colored output when stdout is not a tty.
    // `marketplace list` on an empty registry normally includes a `.bold()`
    // hint, which would emit ANSI escapes if colour were on.
    let dir = TempDir::new().expect("temp dir");
    let output = run_in_dir(dir.path(), &["marketplace", "list"]);
    assert!(output.status.success(), "stderr: {}", stderr(&output));

    let out = stdout(&output);
    assert!(
        !out.contains('\x1b'),
        "stdout must not contain ANSI escapes when piped:\n{out:?}"
    );
}

#[test]
fn no_color_env_suppresses_ansi_escapes() {
    // Even if the user has a TTY, NO_COLOR=1 must disable colour per
    // https://no-color.org. Integration tests always pipe output, so this
    // primarily regresses the env-var branch of `force_no_color`.
    use std::process::Command;
    let dir = TempDir::new().expect("temp dir");
    let output = Command::new(common::get_binary())
        .args(["marketplace", "list"])
        .current_dir(dir.path())
        .env("KIRO_MARKET_DATA_DIR", dir.path().join(".data"))
        .env("NO_COLOR", "1")
        .output()
        .expect("run kiro-market");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let out = String::from_utf8_lossy(&output.stdout);
    assert!(
        !out.contains('\x1b'),
        "NO_COLOR=1 must suppress ANSI escapes:\n{out:?}"
    );
}

#[test]
fn list_no_installed_skills() {
    let dir = TempDir::new().expect("temp dir");
    let output = run_in_dir(dir.path(), &["list"]);

    assert!(
        output.status.success(),
        "expected success, got: {}",
        stderr(&output)
    );

    let out = stdout(&output);
    assert!(
        out.contains("No skills installed"),
        "expected 'No skills installed' in output:\n{out}"
    );
}

#[test]
fn install_missing_marketplace_fails() {
    let dir = TempDir::new().expect("temp dir");
    let output = run_in_dir(dir.path(), &["install", "dotnet@nonexistent"]);

    assert!(!output.status.success(), "expected failure");

    let err = stderr(&output);
    assert!(
        err.contains("not found"),
        "expected 'not found' in stderr:\n{err}"
    );
}

#[test]
fn marketplace_add_rejects_http_url_by_default() {
    // End-to-end coverage of the http:// gate: the CLI must surface the
    // InsecureSource error without needing the --allow-insecure-http
    // flag. Without this test, only the service-layer gate is verified
    // — a regression that drops the CLI plumbing wouldn't be caught.
    let dir = TempDir::new().expect("temp dir");
    let output = run_in_dir(
        dir.path(),
        &["marketplace", "add", "http://example.com/repo.git"],
    );

    assert!(
        !output.status.success(),
        "http:// add must fail without opt-in"
    );
    let err = stderr(&output);
    assert!(
        err.contains("http://") && err.contains("allow-insecure-http"),
        "error must mention http:// and the remediation flag, got:\n{err}"
    );
}

#[test]
fn marketplace_add_help_documents_allow_insecure_http_flag() {
    // Documentation regression: if the --allow-insecure-http flag is
    // ever renamed or removed, this test catches it via clap's auto-
    // generated help. The error message in the gate above references
    // this flag by name; the two MUST stay in sync.
    let dir = TempDir::new().expect("temp dir");
    let output = run_in_dir(dir.path(), &["marketplace", "add", "--help"]);
    assert!(output.status.success(), "stderr: {}", stderr(&output));

    let out = stdout(&output);
    assert!(
        out.contains("--allow-insecure-http"),
        "marketplace add --help must document --allow-insecure-http:\n{out}"
    );
}

#[test]
fn install_help_documents_accept_mcp_flag() {
    // Same regression guard for the MCP opt-in: the install help must
    // document --accept-mcp because the InstallWarning::McpServersRequireOptIn
    // message references it by name.
    let dir = TempDir::new().expect("temp dir");
    let output = run_in_dir(dir.path(), &["install", "--help"]);
    assert!(output.status.success(), "stderr: {}", stderr(&output));

    let out = stdout(&output);
    assert!(
        out.contains("--accept-mcp"),
        "install --help must document --accept-mcp:\n{out}"
    );
}

#[test]
fn cache_prune_dry_run_on_clean_cache_succeeds() {
    // Smoke test for the new `cache prune` command: even on an empty
    // cache (no marketplaces ever added) it must exit zero and report
    // "no orphans". A regression that mishandles missing dirs would
    // surface here.
    let dir = TempDir::new().expect("temp dir");
    let output = run_in_dir(dir.path(), &["cache", "prune", "--dry-run"]);

    assert!(
        output.status.success(),
        "cache prune on a fresh dir must succeed: {}",
        stderr(&output)
    );
    let out = stdout(&output);
    assert!(
        out.contains("clean"),
        "expected 'cache is clean' wording, got:\n{out}"
    );
}

#[test]
fn update_command_exits_nonzero_while_unimplemented() {
    // Regression: the top-level `update` command is a documented stub
    // that previously returned Ok(()), so CI couldn't distinguish
    // "update succeeded" from "update is not implemented." It must now
    // exit non-zero so automation notices.
    let dir = TempDir::new().expect("temp dir");
    let output = run_in_dir(dir.path(), &["update"]);
    assert!(
        !output.status.success(),
        "unimplemented update must exit non-zero: stdout={} stderr={}",
        stdout(&output),
        stderr(&output)
    );
    let err = stderr(&output);
    assert!(
        err.contains("not yet") || err.contains("not implemented"),
        "stderr should explain the command is unimplemented: {err}"
    );
}

#[test]
fn remove_nonexistent_skill_fails() {
    let dir = TempDir::new().expect("temp dir");
    let output = run_in_dir(dir.path(), &["remove", "nonexistent-skill"]);

    // remove_skill returns an error when the skill directory does not exist.
    assert!(
        !output.status.success(),
        "expected failure for nonexistent skill"
    );

    let err = stderr(&output);
    assert!(
        err.contains("nonexistent-skill"),
        "expected skill name in stderr:\n{err}"
    );
}

#[test]
fn workflow_add_marketplace_and_list_plugins() {
    let dir = TempDir::new().expect("temp dir");

    // Create a local git repo that looks like a marketplace.
    let marketplace_dir = dir.path().join("origin-marketplace");
    std::fs::create_dir_all(&marketplace_dir).expect("create marketplace dir");
    common::fixtures::create_marketplace_repo(&marketplace_dir);

    // Add via file:// URL so it gets cloned (not symlinked).
    // Local path symlinks are Unix-only; file:// works on all platforms.
    let url = path_to_file_url(&marketplace_dir);
    let output = run_in_dir(dir.path(), &["marketplace", "add", &url]);
    assert!(
        output.status.success(),
        "marketplace add failed: {}",
        stderr(&output)
    );
    let out = stdout(&output);
    assert!(
        out.contains("test-marketplace"),
        "expected marketplace name in output:\n{out}"
    );

    // List marketplaces — should show the newly added one.
    let output = run_in_dir(dir.path(), &["marketplace", "list"]);
    assert!(
        output.status.success(),
        "marketplace list failed: {}",
        stderr(&output)
    );
    let out = stdout(&output);
    assert!(
        out.contains("test-marketplace"),
        "expected 'test-marketplace' in list output:\n{out}"
    );

    // Search — should find the test plugin.
    let output = run_in_dir(dir.path(), &["search", "test"]);
    assert!(
        output.status.success(),
        "search failed: {}",
        stderr(&output)
    );
    let out = stdout(&output);
    assert!(
        out.contains("test-plugin"),
        "expected 'test-plugin' in search output:\n{out}"
    );
}

#[test]
fn workflow_install_skill_and_verify_on_disk() {
    let dir = TempDir::new().expect("temp dir");

    // Create and add a local marketplace via file:// URL (cross-platform).
    let marketplace_dir = dir.path().join("origin-marketplace");
    std::fs::create_dir_all(&marketplace_dir).expect("create marketplace dir");
    common::fixtures::create_marketplace_repo(&marketplace_dir);

    let url = path_to_file_url(&marketplace_dir);
    let output = run_in_dir(dir.path(), &["marketplace", "add", &url]);
    assert!(
        output.status.success(),
        "marketplace add failed: {}",
        stderr(&output)
    );

    // Install a skill from the marketplace.
    let output = run_in_dir(dir.path(), &["install", "test-plugin@test-marketplace"]);
    assert!(
        output.status.success(),
        "install failed: {}",
        stderr(&output)
    );

    // Verify the skill file was written to disk.
    let skill_path = dir.path().join(".kiro/skills/test-skill/SKILL.md");
    assert!(
        skill_path.exists(),
        "SKILL.md should exist at {}",
        skill_path.display()
    );

    let content = std::fs::read_to_string(&skill_path).expect("read SKILL.md");
    assert!(
        content.contains("Test Skill"),
        "SKILL.md should contain skill content:\n{content}"
    );

    // List installed skills — should show the installed skill.
    let output = run_in_dir(dir.path(), &["list"]);
    assert!(output.status.success(), "list failed: {}", stderr(&output));
    let out = stdout(&output);
    assert!(
        out.contains("test-skill"),
        "expected 'test-skill' in list output:\n{out}"
    );
}

#[test]
fn workflow_marketplace_update_fetches_new_content() {
    let dir = TempDir::new().expect("temp dir");

    // Create a local git repo marketplace — NOT a local path symlink,
    // because symlinked marketplaces are skipped during update.
    // We add it via its file:// URL so it gets cloned (not symlinked).
    let marketplace_dir = dir.path().join("origin-marketplace");
    std::fs::create_dir_all(&marketplace_dir).expect("create marketplace dir");
    common::fixtures::create_marketplace_repo(&marketplace_dir);

    let url = path_to_file_url(&marketplace_dir);
    let output = run_in_dir(dir.path(), &["marketplace", "add", &url]);
    assert!(
        output.status.success(),
        "marketplace add failed: {}",
        stderr(&output)
    );

    // Install a skill before the update.
    let output = run_in_dir(dir.path(), &["install", "test-plugin@test-marketplace"]);
    assert!(
        output.status.success(),
        "install failed: {}",
        stderr(&output)
    );

    // Read the installed skill content before update.
    let skill_path = dir.path().join(".kiro/skills/test-skill/SKILL.md");
    let before = std::fs::read_to_string(&skill_path).expect("read SKILL.md before");
    assert!(
        before.contains("This is a test skill."),
        "expected original content:\n{before}"
    );

    // Add a second commit to the origin (simulating upstream update).
    common::fixtures::add_marketplace_update(&marketplace_dir);

    // Run marketplace update.
    let output = run_in_dir(dir.path(), &["marketplace", "update"]);
    assert!(
        output.status.success(),
        "marketplace update failed: {}",
        stderr(&output)
    );
    let out = stdout(&output);
    assert!(
        out.contains("done") || out.contains("test-marketplace"),
        "expected update confirmation in output:\n{out}"
    );

    // Re-install with --force to pick up the updated content.
    let output = run_in_dir(
        dir.path(),
        &["install", "test-plugin@test-marketplace", "--force"],
    );
    assert!(
        output.status.success(),
        "re-install failed: {}",
        stderr(&output)
    );

    // Verify the skill content was updated.
    let after = std::fs::read_to_string(&skill_path).expect("read SKILL.md after");
    assert!(
        after.contains("updated"),
        "expected updated content after marketplace update + reinstall:\n{after}"
    );
}

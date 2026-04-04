mod common;

use common::{run_in_dir, stderr, stdout};
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

    // Add it as a local path marketplace.
    let source = marketplace_dir.to_str().expect("valid utf-8");
    let output = run_in_dir(dir.path(), &["marketplace", "add", source]);
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

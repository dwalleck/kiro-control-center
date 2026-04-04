mod common;

use std::path::Path;

use common::{run_in_dir, stderr, stdout};
use tempfile::TempDir;

/// Convert a local path into a valid `file://` URL on all platforms.
fn path_to_file_url(path: &Path) -> String {
    let s = path.display().to_string().replace('\\', "/");
    if s.starts_with('/') {
        format!("file://{s}")
    } else {
        format!("file:///{s}")
    }
}

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

#[test]
fn workflow_install_skill_and_verify_on_disk() {
    let dir = TempDir::new().expect("temp dir");

    // Create and add a local marketplace.
    let marketplace_dir = dir.path().join("origin-marketplace");
    std::fs::create_dir_all(&marketplace_dir).expect("create marketplace dir");
    common::fixtures::create_marketplace_repo(&marketplace_dir);

    let source = marketplace_dir.to_str().expect("valid utf-8");
    let output = run_in_dir(dir.path(), &["marketplace", "add", source]);
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

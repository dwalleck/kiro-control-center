use std::path::Path;
use std::process::Command;

/// Run `git <args>` in `dir` with deterministic author identity, asserting
/// success. Shared between fixture setup and update helpers so we only have
/// one place that understands the test-git environment.
fn git_run(dir: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "Test")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "Test")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .output()
        .expect("git command should run");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Create a local git repository containing a valid marketplace manifest
/// with one plugin that has one skill.
pub fn create_marketplace_repo(dir: &Path) {
    let plugin_dir = dir.join("plugins/test-plugin");
    let skill_dir = plugin_dir.join("skills/test-skill");
    std::fs::create_dir_all(dir.join(".claude-plugin")).expect("create .claude-plugin");
    std::fs::create_dir_all(&skill_dir).expect("create skill dir");

    std::fs::write(
        dir.join(".claude-plugin/marketplace.json"),
        r#"{
  "name": "test-marketplace",
  "owner": { "name": "Test" },
  "plugins": [
    {
      "name": "test-plugin",
      "description": "A test plugin",
      "source": "./plugins/test-plugin"
    }
  ]
}"#,
    )
    .expect("write marketplace.json");

    std::fs::write(
        plugin_dir.join("plugin.json"),
        r#"{
  "name": "test-plugin",
  "version": "1.0.0",
  "description": "A test plugin for workflow tests",
  "skills": ["skills/test-skill"]
}"#,
    )
    .expect("write plugin.json");

    std::fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: test-skill\ndescription: A test skill\n---\n# Test Skill\n\nThis is a test skill.\n",
    )
    .expect("write SKILL.md");

    git_run(dir, &["init"]);
    git_run(dir, &["add", "."]);
    git_run(
        dir,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-m",
            "initial marketplace",
        ],
    );
}

/// Add a second commit to an existing marketplace repo to simulate an update.
pub fn add_marketplace_update(dir: &Path) {
    let skill_md = dir.join("plugins/test-plugin/skills/test-skill/SKILL.md");
    std::fs::write(
        &skill_md,
        "---\nname: test-skill\ndescription: An updated test skill\n---\n# Test Skill v1.1\n\nThis skill has been updated.\n",
    )
    .expect("write updated SKILL.md");

    git_run(dir, &["add", "."]);
    git_run(
        dir,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-m",
            "update marketplace",
        ],
    );
}

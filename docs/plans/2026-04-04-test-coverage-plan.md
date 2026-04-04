# Test Coverage Improvement — Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Close the test coverage gaps identified in `docs/plans/2026-04-03-test-coverage-design.md` — add Tauri command tests, CLI workflow tests, and Playwright E2E smoke tests.

**Architecture:** Three independent test layers. Layer 1 adds Rust unit tests for untested Tauri commands (`installed.rs`). Layer 2 adds CLI integration tests that exercise full marketplace → install → update workflows using local `file://` git repos. Layer 3 scaffolds Playwright for the Tauri desktop app with happy-path smoke tests. The `marketplaces.rs` commands are covered by CLI workflow tests (Layer 2) rather than direct unit tests because they call `CacheDir::default_location()` which requires process-level env var isolation — the CLI test harness already provides this via `XDG_DATA_HOME`.

**Tech Stack:** Rust (rstest, tempfile), cargo integration tests, Playwright + @playwright/test, Tauri 2.x dev server

---

### Task 1: Add unit tests for `installed.rs` Tauri commands

**Files:**
- Modify: `crates/kiro-control-center/src-tauri/src/commands/installed.rs`

These commands accept `project_path: String` so they're directly testable with a temp directory — no env var hacking needed.

**Step 1: Write tests**

Add a test module at the bottom of `installed.rs`:

```rust
#[cfg(test)]
mod tests {
    use chrono::Utc;
    use kiro_market_core::project::{InstalledSkillMeta, KiroProject};

    use super::*;

    fn temp_project_with_skill(name: &str) -> (tempfile::TempDir, String) {
        let dir = tempfile::tempdir().expect("tempdir");
        let project = KiroProject::new(dir.path().to_path_buf());
        let meta = InstalledSkillMeta {
            marketplace: "test-market".into(),
            plugin: "test-plugin".into(),
            version: Some("1.0.0".into()),
            installed_at: Utc::now(),
        };
        project
            .install_skill(name, "# Test Skill\nBody content", meta)
            .expect("install_skill");
        let path = dir.path().to_str().expect("valid utf-8").to_owned();
        (dir, path)
    }

    #[tokio::test]
    async fn list_installed_skills_returns_sorted_list() {
        let dir = tempfile::tempdir().expect("tempdir");
        let project = KiroProject::new(dir.path().to_path_buf());
        let path = dir.path().to_str().expect("valid utf-8").to_owned();

        // Install skills in non-alphabetical order.
        for name in &["zulu-skill", "alpha-skill", "mike-skill"] {
            let meta = InstalledSkillMeta {
                marketplace: "test-market".into(),
                plugin: "test-plugin".into(),
                version: Some("1.0.0".into()),
                installed_at: Utc::now(),
            };
            project
                .install_skill(name, "# Skill\nBody", meta)
                .expect("install_skill");
        }

        let result = list_installed_skills(path).await.expect("should succeed");

        assert_eq!(result.len(), 3);
        assert_eq!(result[0].name, "alpha-skill");
        assert_eq!(result[1].name, "mike-skill");
        assert_eq!(result[2].name, "zulu-skill");
        assert_eq!(result[0].marketplace, "test-market");
        assert_eq!(result[0].plugin, "test-plugin");
        assert!(result[0].version.as_deref() == Some("1.0.0"));
    }

    #[tokio::test]
    async fn list_installed_skills_empty_project_returns_empty_vec() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().to_str().expect("valid utf-8").to_owned();

        let result = list_installed_skills(path).await.expect("should succeed");

        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn remove_skill_removes_from_project() {
        let (_dir, path) = temp_project_with_skill("removable-skill");

        remove_skill("removable-skill".into(), path.clone())
            .await
            .expect("should succeed");

        let result = list_installed_skills(path).await.expect("should succeed");
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn remove_skill_nonexistent_returns_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().to_str().expect("valid utf-8").to_owned();

        let err = remove_skill("nonexistent".into(), path)
            .await
            .expect_err("should fail");

        assert!(
            err.message.contains("nonexistent"),
            "expected skill name in error: {}",
            err.message
        );
    }
}
```

**Step 2: Add tokio dev-dependency**

In `crates/kiro-control-center/src-tauri/Cargo.toml`, add to `[dev-dependencies]`:

```toml
tokio = { version = "1", features = ["macros", "rt"] }
```

The `#[tauri::command]` functions are `async fn`, so tests need a tokio runtime.

**Step 3: Run tests**

Run: `cargo test -p kiro-control-center`
Expected: All existing tests pass plus 4 new tests.

Run: `cargo clippy --workspace -- -D warnings`
Expected: No warnings.

**Step 4: Commit**

```bash
git add crates/kiro-control-center/src-tauri/src/commands/installed.rs crates/kiro-control-center/src-tauri/Cargo.toml
git commit -m "test: add unit tests for installed.rs Tauri commands

Tests list_installed_skills (sorted, empty) and remove_skill (success,
nonexistent). Covers the previously untested installed.rs module.

Part of test coverage improvement plan."
```

---

### Task 2: Create marketplace fixture helper for CLI workflow tests

**Files:**
- Create: `crates/kiro-market/tests/common/fixtures.rs`
- Modify: `crates/kiro-market/tests/common/mod.rs`

The CLI workflow tests need a local git repo that looks like a real marketplace. This helper creates one.

**Step 1: Create the fixture helper**

Create `crates/kiro-market/tests/common/fixtures.rs`:

```rust
use std::path::Path;
use std::process::Command;

/// Create a local git repository containing a valid marketplace manifest
/// with one plugin that has one skill.
///
/// Layout:
/// ```text
/// <dir>/
///   .claude-plugin/
///     marketplace.json
///   plugins/
///     test-plugin/
///       plugin.json
///       skills/
///         test-skill/
///           SKILL.md
/// ```
pub fn create_marketplace_repo(dir: &Path) {
    // Create directory structure.
    let plugin_dir = dir.join("plugins/test-plugin");
    let skill_dir = plugin_dir.join("skills/test-skill");
    std::fs::create_dir_all(dir.join(".claude-plugin")).expect("create .claude-plugin");
    std::fs::create_dir_all(&skill_dir).expect("create skill dir");

    // Write marketplace manifest.
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

    // Write plugin manifest.
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

    // Write skill file.
    std::fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: test-skill\ndescription: A test skill\n---\n# Test Skill\n\nThis is a test skill.\n",
    )
    .expect("write SKILL.md");

    // Initialize git repo and commit.
    let run = |args: &[&str]| {
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
    };
    run(&["init"]);
    run(&["add", "."]);
    run(&["-c", "commit.gpgsign=false", "commit", "-m", "initial marketplace"]);
}

/// Add a second commit to an existing marketplace repo to simulate an update.
/// Creates a new file `plugins/test-plugin/skills/test-skill/CHANGELOG.md`.
pub fn add_marketplace_update(dir: &Path) {
    let changelog = dir.join("plugins/test-plugin/skills/test-skill/CHANGELOG.md");
    std::fs::write(&changelog, "# Changelog\n\n## 1.1.0\n- Updated skill\n")
        .expect("write CHANGELOG.md");

    // Update the skill content too.
    let skill_md = dir.join("plugins/test-plugin/skills/test-skill/SKILL.md");
    std::fs::write(
        &skill_md,
        "---\nname: test-skill\ndescription: An updated test skill\n---\n# Test Skill v1.1\n\nThis skill has been updated.\n",
    )
    .expect("write updated SKILL.md");

    let run = |args: &[&str]| {
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
    };
    run(&["add", "."]);
    run(&["-c", "commit.gpgsign=false", "commit", "-m", "update marketplace"]);
}
```

**Step 2: Register the module**

In `crates/kiro-market/tests/common/mod.rs`, add at the bottom:

```rust
pub mod fixtures;
```

**Step 3: Verify it compiles**

Run: `cargo test -p kiro-market --no-run`
Expected: Compiles without errors.

**Step 4: Commit**

```bash
git add crates/kiro-market/tests/common/fixtures.rs crates/kiro-market/tests/common/mod.rs
git commit -m "test: add marketplace fixture helpers for CLI workflow tests

create_marketplace_repo() builds a local git repo with a valid
marketplace manifest, plugin, and skill. add_marketplace_update()
adds a second commit to simulate remote updates."
```

---

### Task 3: CLI workflow test — marketplace add → list → search

**Files:**
- Modify: `crates/kiro-market/tests/cli_tests.rs`

**Step 1: Write the workflow test**

Add to `cli_tests.rs`:

```rust
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
```

**Step 2: Run tests**

Run: `cargo test -p kiro-market workflow_add_marketplace`
Expected: PASS

**Step 3: Commit**

```bash
git add crates/kiro-market/tests/cli_tests.rs
git commit -m "test: add CLI workflow test for marketplace add → list → search

Exercises the full flow: create local marketplace repo, add via CLI,
verify it appears in list and search results."
```

---

### Task 4: CLI workflow test — install skill → verify on disk

**Files:**
- Modify: `crates/kiro-market/tests/cli_tests.rs`

**Step 1: Write the test**

Add to `cli_tests.rs`:

```rust
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
    let output = run_in_dir(
        dir.path(),
        &["install", "test-plugin@test-marketplace"],
    );
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
    assert!(
        output.status.success(),
        "list failed: {}",
        stderr(&output)
    );
    let out = stdout(&output);
    assert!(
        out.contains("test-skill"),
        "expected 'test-skill' in list output:\n{out}"
    );
}
```

**Step 2: Run tests**

Run: `cargo test -p kiro-market workflow_install_skill`
Expected: PASS

**Step 3: Commit**

```bash
git add crates/kiro-market/tests/cli_tests.rs
git commit -m "test: add CLI workflow test for install skill → verify on disk

Exercises: add marketplace → install plugin skill → verify SKILL.md
written to .kiro/skills/ → verify skill appears in list output."
```

---

### Task 5: CLI workflow test — marketplace update → verify new content

**Files:**
- Modify: `crates/kiro-market/tests/cli_tests.rs`

This is the test that would have caught the `pull_repo` no-op bug.

**Step 1: Write the test**

Add to `cli_tests.rs`:

```rust
#[test]
fn workflow_marketplace_update_fetches_new_content() {
    let dir = TempDir::new().expect("temp dir");

    // Create a local git repo marketplace — NOT a local path symlink,
    // because symlinked marketplaces are skipped during update.
    // We add it via its file:// URL so it gets cloned (not symlinked).
    let marketplace_dir = dir.path().join("origin-marketplace");
    std::fs::create_dir_all(&marketplace_dir).expect("create marketplace dir");
    common::fixtures::create_marketplace_repo(&marketplace_dir);

    let url = format!("file://{}", marketplace_dir.display());
    let output = run_in_dir(dir.path(), &["marketplace", "add", &url]);
    assert!(
        output.status.success(),
        "marketplace add failed: {}",
        stderr(&output)
    );

    // Install a skill before the update.
    let output = run_in_dir(
        dir.path(),
        &["install", "test-plugin@test-marketplace"],
    );
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
```

**Step 2: Run tests**

Run: `cargo test -p kiro-market workflow_marketplace_update`
Expected: PASS

**Step 3: Commit**

```bash
git add crates/kiro-market/tests/cli_tests.rs
git commit -m "test: add CLI workflow test for marketplace update → verify new content

This is the test that would have caught the pull_repo no-op bug.
Exercises: add marketplace via git URL → install skill → add upstream
commit → marketplace update → reinstall → verify updated content."
```

---

### Task 6: Scaffold Playwright for Tauri app

**Files:**
- Create: `crates/kiro-control-center/playwright.config.ts`
- Create: `crates/kiro-control-center/tests/e2e/app.spec.ts`
- Modify: `crates/kiro-control-center/package.json` (add dependencies and scripts)

**Step 1: Install Playwright**

```bash
cd crates/kiro-control-center
npm install -D @playwright/test
npx playwright install chromium
```

**Step 2: Create Playwright config**

Create `crates/kiro-control-center/playwright.config.ts`:

```typescript
import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: "./tests/e2e",
  timeout: 60_000,
  retries: 0,
  use: {
    baseURL: "http://localhost:1420",
    trace: "on-first-retry",
  },
  webServer: {
    command: "cargo tauri dev",
    url: "http://localhost:1420",
    timeout: 120_000,
    reuseExistingServer: !process.env.CI,
  },
});
```

**Step 3: Create the smoke test**

Create `crates/kiro-control-center/tests/e2e/app.spec.ts`:

```typescript
import { test, expect } from "@playwright/test";

test.describe("Kiro Control Center", () => {
  test("app loads and shows all tabs", async ({ page }) => {
    await page.goto("/");

    // Wait for the app to mount.
    await expect(page.locator("body")).toBeVisible();

    // Verify all three tabs are present.
    await expect(page.getByRole("tab", { name: /browse/i })).toBeVisible();
    await expect(page.getByRole("tab", { name: /installed/i })).toBeVisible();
    await expect(
      page.getByRole("tab", { name: /marketplace/i })
    ).toBeVisible();
  });
});
```

**Step 4: Add test script to package.json**

Add to the `"scripts"` section of `package.json`:

```json
"test:e2e": "playwright test"
```

**Step 5: Verify the config is valid**

Run: `cd crates/kiro-control-center && npx playwright test --list`
Expected: Lists the test without running it (the Tauri dev server may not be running).

**Step 6: Commit**

```bash
git add crates/kiro-control-center/playwright.config.ts crates/kiro-control-center/tests/e2e/app.spec.ts crates/kiro-control-center/package.json
git commit -m "test: scaffold Playwright E2E for Tauri control center

Adds playwright config with cargo tauri dev as webServer, and a
smoke test that verifies the app loads with all three tabs visible."
```

---

### Task 7: Playwright E2E — add marketplace and install skill flow

**Files:**
- Modify: `crates/kiro-control-center/tests/e2e/app.spec.ts`

**Step 1: Add workflow tests**

Append to `app.spec.ts`:

```typescript
test.describe("Marketplace workflow", () => {
  test("add local marketplace and browse plugins", async ({ page }) => {
    await page.goto("/");

    // Navigate to Marketplaces tab.
    await page.getByRole("tab", { name: /marketplace/i }).click();

    // Add a marketplace using a local path.
    // NOTE: This test requires a pre-built marketplace fixture.
    // For CI, set FIXTURE_MARKETPLACE_PATH env var.
    const fixturePath = process.env.FIXTURE_MARKETPLACE_PATH;
    if (!fixturePath) {
      test.skip(true, "FIXTURE_MARKETPLACE_PATH not set");
      return;
    }

    const input = page.getByPlaceholder(/source|url|path/i);
    await input.fill(fixturePath);

    const addButton = page.getByRole("button", { name: /add/i });
    await addButton.click();

    // Wait for success feedback.
    await expect(page.getByText(/added|success/i)).toBeVisible({
      timeout: 30_000,
    });

    // Switch to Browse tab and verify plugin appears.
    await page.getByRole("tab", { name: /browse/i }).click();
    await expect(page.getByText(/test-plugin/i)).toBeVisible();
  });

  test("install skill from browse tab", async ({ page }) => {
    await page.goto("/");

    // This test assumes a marketplace is already added (from previous test
    // or fixture setup). If no marketplace exists, skip.
    await page.getByRole("tab", { name: /browse/i }).click();

    const pluginLink = page.getByText(/test-plugin/i);
    if (!(await pluginLink.isVisible({ timeout: 5_000 }).catch(() => false))) {
      test.skip(true, "No marketplace with test-plugin available");
      return;
    }

    await pluginLink.click();

    // Find and click install on a skill.
    const installButton = page.getByRole("button", { name: /install/i });
    await installButton.first().click();

    // Verify success.
    await expect(page.getByText(/installed|success/i)).toBeVisible({
      timeout: 30_000,
    });

    // Switch to Installed tab and verify skill appears.
    await page.getByRole("tab", { name: /installed/i }).click();
    await expect(page.getByText(/test-skill/i)).toBeVisible();
  });
});
```

**Step 2: Commit**

```bash
git add crates/kiro-control-center/tests/e2e/app.spec.ts
git commit -m "test: add Playwright E2E tests for marketplace add and skill install

Happy-path smoke tests for:
- Add local marketplace → verify in browse tab
- Install skill from browse → verify in installed tab

Tests skip gracefully when FIXTURE_MARKETPLACE_PATH is not set."
```

---

### Task 8: Final verification and cleanup

**Step 1: Run the full Rust test suite**

```bash
cargo test --workspace
```

Expected: All tests pass.

**Step 2: Run clippy**

```bash
cargo clippy --workspace -- -D warnings
```

Expected: No warnings.

**Step 3: Verify Playwright config**

```bash
cd crates/kiro-control-center && npx playwright test --list
```

Expected: Lists 3 tests (smoke + 2 workflow).

**Step 4: Commit any remaining fixes**

If any tests needed adjustment, commit the fixes.

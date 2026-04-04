# Test Coverage Improvement — Design

**Date:** 2026-04-03
**Goal:** Close the test coverage gaps identified during the gix migration review, establishing a testing foundation that grows with the project.

**Strategy:** Three parallel, orthogonal test layers — each catches a different class of bug.

---

## Layer 1: Rust Command Tests (Tauri Backend)

**Target:** `crates/kiro-control-center/src-tauri/src/commands/installed.rs` (0% coverage) and `marketplaces.rs` (0% coverage).

**Approach:** Call the Tauri command functions directly (they're regular `async fn` taking primitives) with `tempfile` directories standing in for project/cache paths. No running Tauri app needed.

**Fixtures:** Temp directories with realistic structure:
- Fake project dir: `.kiro/installed.json` + `.kiro/skills/`
- Fake cache dir: cloned marketplace repos with `marketplace.json`

**Tests:**

| Command | Tests |
|---------|-------|
| `list_installed_skills` | Returns sorted list; empty project → empty vec; missing project dir → error |
| `remove_skill` | Removes skill from project; non-existent skill → error |
| `add_marketplace` | Local path source creates symlink; already-registered → error; missing manifest → error with cleanup |
| `remove_marketplace` | Removes from registry + filesystem; non-existent → error |
| `update_marketplace` | Skips symlinked (local) marketplaces; single + all modes |

**Scoping note:** Skip testing `add_marketplace` with real git clones — that's covered by `kiro-market-core` tests. Command tests verify orchestration logic only.

---

## Layer 2: Rust CLI Workflow Tests

**Target:** `crates/kiro-market/` — no existing test exercises a real end-to-end workflow.

**Approach:** Integration test file (`tests/workflow_test.rs`) that exercises the full lifecycle using local `file://` git repos. No network calls, no mocking.

**Fixture:** A helper that creates a local git repo with realistic marketplace structure:
```
marketplace.json
plugins/
  example-plugin/
    plugin.json
    skills/
      example-skill/
        SKILL.md
```

**Workflows to test:**

| Workflow | What it catches |
|----------|----------------|
| `marketplace add` → list → verify plugins visible | Manifest parsing + registration |
| `install` a skill → verify SKILL.md written to `.kiro/skills/` | Clone + skill merge + project state |
| `marketplace update` → verify new content arrives | The `pull_repo` no-op class of bugs |
| `install` with `git_ref` → verify correct branch checked out | The ref checkout path |

**This is the highest-leverage layer.** One workflow test would have caught the `pull_repo` no-op, the `--` separator bug, and the missing timeout regression.

---

## Layer 3: Playwright E2E Smoke Tests

**Target:** The Tauri desktop app (`crates/kiro-control-center/`). Happy-path only.

**Approach:** Playwright drives the WebView content against the Tauri dev server (`localhost:1420`).

**Setup:**
- `playwright.config.ts` in `crates/kiro-control-center/`
- `@playwright/test` as a dev dependency
- `webServer` config starts `cargo tauri dev`, waits for port
- `globalSetup` starts the server once for the entire suite (cold start is slow)
- Runs in a separate CI job, not on every `cargo test`

**Tests:**

| Test | What it verifies |
|------|-----------------|
| App loads, all 3 tabs render | Tauri bridge works, Svelte mounts |
| Add marketplace → plugins appear in browse tab | Full add → browse flow through the bridge |
| Install skill → appears in installed tab | Install flow + tab cross-references |
| Remove installed skill → disappears from list | Remove flow + UI reactivity |

**Fixture:** Pre-built local marketplace fixture committed to the repo (simplest).

**Scoping:** Tests assert "data appeared in the UI," not specific CSS or layout. Stable across UI redesigns.

---

## Priority Order

1. **Layer 2 (CLI workflows)** — highest leverage, catches the broadest class of bugs
2. **Layer 1 (Tauri command tests)** — fills the zero-coverage gap in the desktop app backend
3. **Layer 3 (Playwright E2E)** — establishes the UI testing foundation

## Expected Outcome

| Metric | Before | After |
|--------|--------|-------|
| `kiro-market-core` tests | 118 | 118 (unchanged) |
| `kiro-market` CLI tests | 12 | ~20 (+workflow tests) |
| `kiro-control-center` tests | 15 | ~30 (+command tests) |
| Playwright E2E tests | 0 | ~4 (smoke tests) |
| Untested Tauri command modules | 2 | 0 |
| End-to-end workflow coverage | none | full lifecycle |

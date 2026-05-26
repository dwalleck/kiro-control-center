# Codebase Info

<!-- tags: overview, metadata, structure -->

## Project

**Name:** Kiro Control Center  
**Purpose:** Desktop app (`kcc`) and CLI (`kiro-market`) for browsing, installing, and managing Claude Code marketplace skills and agents in Kiro projects.  
**Languages:** Rust (backend/core/CLI), TypeScript + Svelte 5 (frontend)  
**Min Rust:** 1.85.0 (edition 2024)  
**License:** MIT

## Workspace Layout

```
crates/
  kiro-market-core/     # Shared library — all business logic
  kiro-market/          # CLI binary (kiro-market)
  kiro-control-center/  # Tauri 2 desktop app (kcc)
    src-tauri/          #   Rust backend
    src/                #   Svelte 5 frontend
xtask/                  # Build/lint/hook automation
.github/
  workflows/            # CI (ci.yml, release.yml, kiro-review.yml)
  scripts/              # post-review-comments.py (PR review automation)
```

## Crate Summary

| Crate | Type | Key Features |
|---|---|---|
| `kiro-market-core` | lib | `cli`, `specta`, `test-support` |
| `kiro-market` | bin | activates `cli` feature on core |
| `kiro-control-center` (src-tauri) | lib+bin | activates `specta` feature on core |
| `xtask` | bin | hooks, plan-lint, workspace automation |

## Technology Stack

- **Rust:** serde/serde_json, thiserror, gix (git), blake3, fs4, chrono, clap, tauri 2, tauri-specta
- **Frontend:** Svelte 5 (runes), SvelteKit, TypeScript, Tailwind CSS 4, Vite 6, Vitest, Playwright
- **CI:** GitHub Actions — 10 jobs (commitlint, format, lint, test×3OS, frontend, build-cli×3OS, build-tauri×3OS, cargo-deny, assert-curl-tls, coverage)

## Key Constants (kiro-market-core)

| Constant | Value |
|---|---|
| `MARKETPLACE_MANIFEST_PATH` | `.claude-plugin/marketplace.json` |
| `DEFAULT_SKILL_PATHS` | `["./skills/"]` |
| `DEFAULT_AGENT_PATHS` | `["./agents/"]` |
| `DEFAULT_STEERING_PATHS` | `["./steering/"]` |

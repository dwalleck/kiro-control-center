# Codebase Information

## Project Identity

- **Name**: kiro-marketplace-cli (Kiro Control Center)
- **Repository**: https://github.com/dwalleck/kiro-marketplace-cli
- **License**: MIT
- **Version**: 0.1.0

## Purpose

A desktop app and CLI for browsing, installing, and managing Claude Code marketplace skills in Kiro projects. Bridges the gap between Claude Code's marketplace skill format and Kiro's `.kiro/skills/<name>/SKILL.md` layout.

## Technology Stack

| Layer | Technology | Version |
|-------|-----------|---------|
| Core library | Rust | Edition 2024, MSRV 1.85.0 |
| CLI | Rust + clap 4 (derive) | — |
| Desktop backend | Tauri 2 | — |
| Desktop frontend | Svelte 5 + SvelteKit | — |
| Styling | TailwindCSS 4 | — |
| Type bindings | tauri-specta 2 | — |
| Git operations | gix 0.81 + CLI fallback | — |
| Build system | Cargo (workspace) + npm | — |
| CI/CD | GitHub Actions | — |

## Workspace Structure

```
kiro-marketplace-cli/
├── Cargo.toml                    # Workspace root
├── crates/
│   ├── kiro-market-core/         # Shared library (types, git, cache, project state)
│   ├── kiro-market/              # CLI binary (kiro-market)
│   └── kiro-control-center/      # Tauri desktop app (kcc)
│       ├── src-tauri/            #   Rust backend
│       └── src/                  #   Svelte 5 frontend
├── .claude/                      # Claude Code hooks and skills
└── .github/workflows/            # CI pipelines
```

## Key Metrics

- **Workspace members**: 3 Rust crates + 1 frontend package
- **Primary languages**: Rust, TypeScript, Svelte
- **Test count**: 147+ across workspace

## Build & Run

| Task | Command |
|------|---------|
| Build all | `cargo build` |
| Test all | `cargo test` |
| Lint | `cargo clippy --workspace -- -D warnings` |
| Format | `cargo fmt --all` |
| Frontend dev | `cd crates/kiro-control-center && npm run dev` |
| Tauri dev | `cd crates/kiro-control-center && npx tauri dev` |
| Regenerate TS bindings | `cargo test -p kiro-control-center --lib -- --ignored generate_types` |

## Configuration Files

| File | Purpose |
|------|---------|
| `Cargo.toml` (root) | Workspace definition, shared deps, lints |
| `deny.toml` | cargo-deny license/advisory audit config |
| `.claude/settings.json` | Claude Code hooks (rustfmt, clippy, block Cargo.lock) |
| `.github/workflows/ci.yml` | Full CI pipeline |
| `crates/kiro-control-center/package.json` | Frontend deps and scripts |
| `crates/kiro-control-center/vite.config.js` | Vite bundler config |
| `crates/kiro-control-center/svelte.config.js` | SvelteKit config (static adapter) |

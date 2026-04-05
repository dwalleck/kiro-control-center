# kiro-market — Developer Guide

## Build
```bash
cargo build
```

## Test
```bash
cargo test                          # all tests
cargo test -p kiro-market-core      # core library tests
cargo test -p kiro-market           # CLI + integration tests
```

## Lint
```bash
cargo clippy --workspace -- -D warnings
```

## Project Structure
- `crates/kiro-market-core/` — library crate (types, parsing, git, cache, project state)
- `crates/kiro-market/` — binary crate (CLI commands)

## Code Style
- Edition 2024, rust-version 1.85.0
- `thiserror` for typed errors in kiro-market-core
- `anyhow` for error propagation in kiro-market binary
- `rstest` for parameterized tests, `tempfile` for test fixtures
- `clippy::all` and `clippy::pedantic` enabled as warnings
- `unsafe_code` is forbidden

## Architecture
The tool reads Claude Code `marketplace.json` catalogs, discovers plugins and skills,
and installs SKILL.md files into Kiro CLI projects at `.kiro/skills/`.

Multi-file Claude Code skills (SKILL.md + companion .md files) are merged into a
single SKILL.md since Kiro doesn't support deferred loading of companion files.

### Service Layer
Marketplace operations (add/remove/update/list) live in `kiro-market-core::service::MarketplaceService`.
CLI and Tauri handlers are thin wrappers that construct the service, call it, and format output.
Domain logic is never duplicated between frontends.

### Git Abstraction
Git operations are abstracted behind the `GitBackend` trait (`kiro-market-core::git`).
`GixCliBackend` implements the trait using `gix` for clone/open and the system `git` CLI
for pull/checkout. The trait enables mock-based testing without filesystem git repos.

### Platform Abstraction
Local marketplace linking uses `kiro-market-core::platform` which provides
`create_local_link`/`is_local_link`/`remove_local_link`. On Unix this uses symlinks,
on Windows it uses directory junctions with copy fallback.

## Key Crate Dependencies
- `gix` + system `git` CLI — git operations (gix for clone/open, system git for pull/checkout)
- `clap` (derive) — CLI framework
- `pulldown-cmark` — markdown parsing for skill merging
- `serde` / `serde_json` / `serde_yaml` — JSON and YAML parsing
- `colored` — terminal output
- `dirs` — XDG path resolution

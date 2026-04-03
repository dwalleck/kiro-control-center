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

## Key Crate Dependencies
- `gix` — git clone/pull operations
- `clap` (derive) — CLI framework
- `pulldown-cmark` — markdown parsing for skill merging
- `serde` / `serde_json` / `serde_yaml` — JSON and YAML parsing
- `colored` — terminal output
- `dirs` — XDG path resolution

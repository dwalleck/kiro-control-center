---
name: cargo-check
description: Run cargo check after Rust file modifications to catch compile errors early
user-invocable: false
---

After modifying .rs or Cargo.toml files, run `cargo check --workspace` to verify compilation.
If there are errors, fix them before proceeding.

Before committing, run `cargo clippy --workspace -- -D warnings` to catch lint issues.
Fix any clippy warnings before creating the commit.

---
name: cargo-audit
description: Run cargo audit and triage advisories by crate scope. Use whenever bumping dependencies, editing Cargo.toml or Cargo.lock, or investigating a RUSTSEC advisory. Also triggers on user phrases like "audit deps", "check CVEs", "are our dependencies safe".
user-invocable: false
---

# Cargo Audit Runner

This workspace has a split advisory profile: `kiro-market-core` / `kiro-market` ship as a broadly-distributed CLI, while `kiro-control-center` is a Tauri desktop app with a much larger transitive graph (gtk-rs, wry, webkit bindings) that carries many unmaintained advisories not reachable from the CLI. Always classify advisories by scope before reporting.

## When to run

- Before any `cargo update` or dep bump
- After merging a PR that touches `Cargo.toml` at workspace or crate level
- When a user asks about dependency safety
- When editing crates that parse untrusted input (`agent/`, `skill.rs`, `plugin.rs`, `marketplace.rs`)

## How to run

```bash
cargo audit --json 2>/dev/null || { echo "Install: cargo install cargo-audit"; exit 1; }
```

For each advisory in the output:

1. Identify the affected crate and version.
2. Run `cargo tree -e normal -i <affected>` to find the workspace member that pulls it in.
3. Classify:
   - **Core path** — in `kiro-market-core` or `kiro-market` dep graph
   - **Tauri path** — only in `kiro-control-center` dep graph
   - **Dev-only** — only through `[dev-dependencies]` or build scripts

4. Note if a fix version exists (check `versions.patched` in the advisory) and the upgrade path.

## Known standing advisories

These are tracked and not-yet-fixed; don't re-report them as new:

- **RUSTSEC-2024-0320** (`serde_yaml` 0.9 unmaintained) — Core path. Migration to `serde_yml` or `marked-yaml` is on the roadmap. Blast radius: agent/skill frontmatter (4-field YAML), not arbitrary user YAML. Still must-fix before 1.0.
- **Tauri gtk-rs / wry / webkit advisories** (RUSTSEC-2024-04xx / 2025-0xxx / 2026-0097 cluster) — Tauri path only. Tracked upstream by Tauri releases; we follow their version bumps.

Anything NOT on this list is new and must be triaged.

## Output format

Mirror the `/audit-deps` slash command format. Advise the user on which bucket blocks a release vs which can wait for upstream.

If the scan is clean of new advisories, say so and list the known standing ones for context.

## Exit criteria

You've done your job when the user has:
- A bucketed list of advisories
- A clear statement of which ones are "new and must be triaged" vs "known and tracked"
- Concrete upgrade paths for fixable advisories

---
description: Run cargo audit and triage advisories by crate scope (core CLI vs Tauri desktop)
---

Run `cargo audit --json` on the workspace and produce a triage list that separates advisories reaching `kiro-market-core` / `kiro-market` (the user-facing CLI, shipped broadly) from those only reaching `kiro-control-center` (the Tauri desktop app).

## Steps

1. Run `cargo audit --json` at the workspace root. If the command fails because `cargo-audit` isn't installed, tell the user to `cargo install cargo-audit` and stop.

2. Parse the JSON output. For each advisory, determine which workspace crate depends on the affected package. Use `cargo tree -p <affected>` or `cargo tree -e normal -i <affected>` to walk backward from the vulnerable crate to the workspace member that pulls it in.

3. Classify each advisory into one of three buckets:
   - **Core path** — reachable from `kiro-market-core` or `kiro-market`. These are highest priority because the CLI is the broadly-distributed artifact.
   - **Tauri path** — reachable only from `kiro-control-center`. Still important but scoped to desktop-app users.
   - **Dev-only** — reachable only through `[dev-dependencies]` or build scripts. Lowest priority for users but still flag.

4. For each advisory note: RUSTSEC ID, crate + version, severity (if CVSS available), whether a fixed version exists, and a one-line upgrade path.

5. Surface `RUSTSEC-2024-0320` (`serde_yaml` unmaintained) specifically if present — it's a known critical in the core path that blocks YAML frontmatter migration.

## Output format

```markdown
## Dependency Audit — <date>

### Core path (blocks CLI release) — <count>
- `RUSTSEC-XXXX-XXXX` — `<crate>@<version>` — <summary>. Fix: upgrade to `<version>` / migrate to `<alternative>`.

### Tauri path — <count>
- ...

### Dev-only — <count>
- ...

### Clean
(or: "Core and Tauri paths both clean — only dev-only advisories remain.")
```

If `cargo-audit` is not installed, fall back to OSV.dev: `curl -s -X POST https://api.osv.dev/v1/query -d '{"package":{"name":"<name>","ecosystem":"crates.io"},"version":"<ver>"}'` for a spot-check on `serde_yaml`, `gix`, `curl`, `chrono`, `fs4`.

No prose beyond the structured sections.

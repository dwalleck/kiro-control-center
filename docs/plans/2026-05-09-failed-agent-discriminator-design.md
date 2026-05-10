# `FailedAgent` → tagged-enum wire format — design 2026-05-09

## Problem

The companion-bundle install path in `crates/kiro-market-core/src/service/mod.rs:1962-1966` reports failures using `FailedAgent { name: None, source_path: scan_root, ... }`, where `scan_root` is the `agents/` directory in the marketplace cache. The wire shape is structurally indistinguishable from a per-agent failure; the user sees `name: null` plus a path that ends in a slash. In the field this misled a debug session into diagnosing a discovery bug ("a directory is being detected as a file") when the actual cause was orphan-file conflict on a project (`kiro-control-center` itself) where `.kiro/agents/*.json` are committed to git for CI use.

The shape is structurally ambiguous:

- `name: Option<String>` overloads "agent name" and "no name available."
- `source_path: PathBuf` overloads "the agent file we tried to install" and "the bundle scan-root we couldn't install."
- The error variant inside `AgentError` carries the *real* path of the conflicting destination, but the FE has no type-level signal to look there.

## Design

Convert `FailedAgent` from a struct to a `#[serde(tag = "kind")]` enum with three variants. Precedent: `UpdateChangeSignal` (`crates/kiro-market-core/src/service/mod.rs:632`) already uses this pattern at the FFI boundary, so no new toolchain risk.

```rust
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FailedAgent {
    /// A native or translated agent failed during install.
    /// Name is known because parsing succeeded.
    Agent {
        name: String,
        source_path: PathBuf,
        #[serde(serialize_with = "serialize_agent_error")]
        #[cfg_attr(feature = "specta", specta(type = String))]
        error: AgentError,
    },
    /// Failed before parse, so no name is available.
    /// `source_path` is the only identifier.
    UnparseableAgent {
        source_path: PathBuf,
        #[serde(serialize_with = "serialize_agent_error")]
        #[cfg_attr(feature = "specta", specta(type = String))]
        error: AgentError,
    },
    /// A plugin's companion-file bundle (e.g. `agents/prompts/*.md`)
    /// failed atomically. Plugin-scoped, not agent-scoped.
    CompanionBundle {
        plugin: PluginName,
        /// Destination paths under `.kiro/agents/` that conflicted.
        /// Today: length-1 (engine bails on first conflict). Empty
        /// when the rejection fires before per-file enumeration
        /// (e.g. `MultipleScanRootsNotSupported`). Shape-compatible
        /// with future "collect all conflicts" engine work without
        /// another wire migration.
        conflicts: Vec<PathBuf>,
        #[serde(serialize_with = "serialize_agent_error")]
        #[cfg_attr(feature = "specta", specta(type = String))]
        error: AgentError,
    },
}
```

Key decisions:

- **Keep the type name `FailedAgent`.** `Vec<FailedAgent>` field stays; only per-element shape changes. Renaming would cosmetic-churn `result.failed` accesses with no semantic gain.
- **Three variants, not two.** Folding `UnparseableAgent` into `Agent` (with `name: Option<String>`) reintroduces the nullable-name problem this PR is trying to remove.
- **`conflicts: Vec<PathBuf>`** even though the engine produces length-1 today. Forward-compatible; locks the wire shape now. Empty for `MultipleScanRootsNotSupported` (rejection fires before any file enumeration).
- **`plugin: PluginName`** (validated newtype). Specta-friendly per `crates/kiro-market-core/src/validation.rs:428-431` (`PluginName` already has `#[cfg_attr(feature = "specta", derive(specta::Type))]`).

## Wire format (TS)

Before:

```ts
type FailedAgent = {
  name: string | null;
  source_path: string;
  error: string;
};
```

After:

```ts
type FailedAgent =
  | { kind: "agent";
      name: string;
      source_path: string;
      error: string }
  | { kind: "unparseable_agent";
      source_path: string;
      error: string }
  | { kind: "companion_bundle";
      plugin: string;
      conflicts: string[];
      error: string };
```

## Consumer impact

LSP `findReferences` was unavailable during scoping (rust-analyzer was still building its cross-crate index on first start). Sites below were enumerated by grep; the post-change compiler errors will be the authoritative list.

### Rust construction sites — 11 total

All in `crates/kiro-market-core/src/service/mod.rs` except the helper at line 2778, which is in the same file.

| Line | Today | New variant | Notes |
|------|-------|-------------|-------|
| 1648 | parse-failure on translated agent | `UnparseableAgent` | name unknown at this point |
| 1727 | per-translated-agent install failure | `Agent` | name known via `def.name` |
| 1772 | multi-scan-root rejection | `CompanionBundle` | `conflicts: vec![]` (rejection pre-enumeration) |
| 1812 | parse-failure on native agent | `UnparseableAgent` | name unknown |
| 1841 | native-manifest-invalid-name | `Agent` | name from `bundle.name` |
| 1854 | hash failure on native agent | `Agent` | name from `bundle.name` |
| 1895 | per-native-agent install failure | `Agent` | name from `bundle.name` |
| 1921 | strip-prefix failure during companion | `CompanionBundle` | discovery-contract violation |
| 1939 | hash failure during companion bundle | `CompanionBundle` | bundle-level |
| 1962 | companion bundle install failure | `CompanionBundle` | the original misdiagnosed site |
| 2778 | `required_source_path` helper for translated agents | `Agent` | name passed in by caller |

### Rust test surface

Grep found 1 explicit pattern-match (`crates/kiro-control-center/src-tauri/src/commands/agents.rs:421-426`). Other `result.failed[N].name`-style accesses across the test module will surface as compile errors after the type change — that's the authoritative count, not grep's. Each becomes a variant-aware match:

```rust
match &result.failed[0] {
    FailedAgent::Agent { name, .. } => assert_eq!(name, "..."),
    other => panic!("expected Agent variant, got {other:?}"),
}
```

### TypeScript consumers

| File | Line | Today | After |
|------|------|-------|-------|
| `crates/kiro-control-center/src/lib/format.ts` | 232-234, 246 | `agents.failed.length` | unchanged — variant-independent |
| `crates/kiro-control-center/src/lib/plugin-actions.ts` | 146 | `result.data.agents.failed` (passed as `console.error` payload) | unchanged — diagnostic logger dumps the array, doesn't read fields |

**Net FE consumer impact today: zero changes required.** The existing FE only reads `.length`. The wire-format upgrade pre-positions for the future inline-failure UI envisaged in the `plugin-actions.ts:130-138` comment without coupling that work to this PR.

### bindings.ts

Three current type definitions at lines 290-330 (`FailedAgent`, `FailedAgent_Serialize`, `FailedAgent_Deserialize`) are auto-generated by specta. Regenerate via `cargo test -p kiro-control-center --lib -- --ignored`. The `_Serialize` / `_Deserialize` split is a specta round-trip artifact; both collapse to the same discriminated union after regen.

## Migration plan

1. **Rust enum rewrite** — `crates/kiro-market-core/src/service/mod.rs:644-662` (struct → enum). `serialize_agent_error` field-level attribute migrates with each variant; no separate update needed.
2. **Update 11 construction sites** per the table above. Pattern-replace pass with explicit re-read per site (no skimming).
3. **`required_source_path` return type** stays `Result<RelativePath, FailedAgent>`; just constructs `FailedAgent::Agent { ... }` internally.
4. **Tests** — `cargo test --workspace`, fix every compile error with the variant-aware match pattern shown above.
5. **Bindings regen** — `cargo test -p kiro-control-center --lib -- --ignored`.
6. **TS** — `npm run check`. Existing FE compiles unchanged. Add a comment near `plugin-actions.ts:130` documenting the discriminator-pushdown pattern future inline-failure UI should use (`switch (entry.kind) { ... default: const _: never = entry; ... }` plus `_ASSERT_EXHAUSTIVE` per CLAUDE.md's "discriminator-pushdown discipline" rule).
7. **Verify clean** — `cargo fmt --all --check`, `cargo clippy --workspace --tests -- -D warnings`, `npm run check`, `npm run test:unit`, `cargo test --workspace`.

## Follow-on work (intentionally not in this PR)

Each item below is shaped so a future PR can pick it up independently. Numbered for cross-reference.

### F1. Engine-level "collect all conflicts in a bundle"

`classify_companion_collision` (`crates/kiro-market-core/src/project.rs:3104`) returns the first orphan-or-cross-plugin conflict and bails. The new `conflicts: Vec<PathBuf>` field is shaped to hold many; ships length-1 here. Extension would walk every `rel` in `input.rel_paths` collecting all conflicts, then return them as a single `Err(... { conflicts: Vec<...> })`. The wire format does not need to change for this work.

**Why deferred:** larger test surface (matrix of orphan / cross-plugin / multi-conflict combinations) and an engine-side error-shape change (`AgentError::OrphanFileAtDestination { path: PathBuf }` → `AgentError::OrphanFilesAtDestination { paths: Vec<PathBuf> }` or similar). Out of scope for a wire-format-only PR.

### F2. `FailedSteeringFile` parallel restructure

Steering has its own `FailedSteeringFile` wire shape with parallel ambiguity potential (a future steering-bundle concept would face the same nullable-name problem). When steering grows a bundle-level concept it should adopt the same enum pattern as this PR.

**Why deferred:** steering doesn't have a bundle-level construct today. Restructuring before there's a use case is YAGNI.

### F3. Inline per-failure UI

`crates/kiro-control-center/src/lib/plugin-actions.ts:130-138` documents this explicitly:

> Temporary diagnostic: the current banner only carries the count-level summary ("1 steering failed · 8 agents failed"), so the per-item reasons that the Rust backend already sends (FailedSkill.error, FailedSteeringFile.error, FailedAgent.error — each carrying the full error chain) are otherwise invisible to the user. Log them to the DevTools console so they can be inspected without a backend-side console (release builds run under the `windows` subsystem, which detaches from the launching terminal). Follow-up work will surface these failures inline in the UI; see runPluginRemove's per-failure `<details>` panel in InstalledTab.svelte for the target shape.

The new discriminated `FailedAgent` is what that UI should consume. Pair the `switch (entry.kind)` rendering with the `_ASSERT_EXHAUSTIVE` value-position guard (CLAUDE.md "discriminator-pushdown discipline").

**Why deferred:** UI work is a separate concern from the wire-format change. Bundling them would force this PR to also touch InstalledTab.svelte and component testing.

### F4. "Remove it manually" remediation message UX

The orphan-file error message (`crates/kiro-market-core/src/error.rs:363-364`):

```
file exists at `{path}` but has no tracking entry; remove it manually or pass --force
```

is poor advice when the orphan is git-tracked (the kiro-control-center case that triggered this PR). A nicer message would acknowledge that orphans may be intentional repo content and suggest force-install or a different target.

**Why deferred:** UX-text change in `AgentError` Display impl. Doesn't depend on or affect the wire-format change.

### F5. Detect "destination is a git-tracked file" at install time

A higher-leverage fix than F4: at install time, when an orphan is detected, check `git ls-files --error-unmatch <path>`. If git claims the file, the remediation message can be specific ("this file is git-tracked in your repo; force-install will produce a diff in your working tree").

**Why deferred:** introduces a `git` subprocess dependency to `kiro-market-core`, which violates the "domain core stays free of external runtimes" rule from CLAUDE.md. The check belongs in the Tauri command layer or CLI, not core. Designable; not designed.

## Risks

- **specta + tagged enums**: validated by `UpdateChangeSignal` precedent. Low risk; verify regenerated `bindings.ts` shape matches design before merge.
- **`PluginName` in wire format**: confirmed `#[cfg_attr(feature = "specta", derive(specta::Type))]` at `crates/kiro-market-core/src/validation.rs:429`. Low risk.
- **Pattern-replace fatigue across 11 construction sites**: mitigation = re-read each site, don't skim. The compiler will catch logic errors but won't catch wrong-variant choice (e.g. accidentally using `UnparseableAgent` where `Agent` is correct because the name is in scope).

## Verification gates (CLAUDE.md plan-review checklist)

- **Gate 1 — Grounding**: each migration step cites a specific file:line. ✓
- **Gate 2 — Threat Model**: no new attack surface (wire shape only).  ✓
- **Gate 3 — Wire Format**: before/after TS shapes specified above.  ✓
- **Gate 4 — External Type Boundary**: `AgentError` already projected via `serialize_agent_error`; no external error types newly exposed. ✓
- **Gate 5 — Type Design**: three-variant decomposition rationale in "Key decisions" above.  ✓
- **Gate 6 — Reference vs Transcription**: design cites the `UpdateChangeSignal` mechanism (the codebase precedent), not just the output shape it produces. ✓

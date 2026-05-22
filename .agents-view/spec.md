# Feature: Create / manage user-authored agents in Control Center

A new top-level surface in `kiro-control-center` that lets developers using
kiro-cli author and manage agent definitions (`.kiro/agents/<name>.json`) of
the project they are currently working in. Adds list + 7-section editor UI,
plus the new-to-this-feature Tauri command surface that performs the JSON
file CRUD that the editor depends on.

## What this is

Today, `.kiro/agents/<name>.json` files in a kiro project can only be created
two ways: (1) installed from a marketplace via the existing
`install_plugin_agents` / `install_agents` Tauri commands, or (2) hand-written
by editing JSON files outside the Control Center. There is no first-party UI
for authoring an agent from scratch.

This feature adds that. A new "Workflows" sidebar group with an "Agents" entry
opens a list of every agent in `.kiro/agents/`, regardless of whether it was
marketplace-installed or user-authored. From there the user can create new
agents, edit existing ones (including marketplace-installed ones), duplicate
them, and delete them. The seven-section editor is a direct port of the
high-fidelity design at
`Kiro Control Center Design System/design_handoff_agents/` (referenced
throughout as "the design bundle").

## Users

- **Kiro CLI developer / agent author**: developers using kiro-cli inside a
  project they control. They open kiro-control-center, point it at their
  project, want to either tweak a marketplace-supplied agent to fit their        njnj
  workflow or compose a fresh agent declaratively. They expect the editor to
  reflect their `.kiro/agents/` directory and to write files that kiro-cli's
  existing agent parser (`crates/kiro-market-core/src/agent/parse_native.rs`)
  loads without complaint.

The word "user" without a qualifier is not used elsewhere in this artifact.

## Behavior

### B1. Open the Agents view

- **Given**: a kiro project is selected in Control Center (existing
  ProjectDropdown / ProjectPicker state non-null).
- **When**: the user clicks the new "Agents" item under the new "Workflows"
  group in `NavRail.svelte`.
- **Then**: the list page renders. If `.kiro/agents/` does not exist, it is
  created (eager mkdir on view-open). The page lists every `*.json` file in
  `.kiro/agents/` (one row per file, parsed via the existing native-agent
  parser). Rows for agents present in `InstalledAgents` show a
  marketplace-lineage badge (`marketplace · plugin · version`).

### B2. No project selected

- **Given**: no project is currently selected.
- **When**: the user navigates to Agents.
- **Then**: same empty-state-style screen used by `BrowseTab` / `InstalledTab`
  today (mirror their no-project rendering — no new pattern).

### B3. Empty `.kiro/agents/`

- **Given**: project selected, `.kiro/agents/` exists, contains zero files.
- **When**: the user is on the Agents list page.
- **Then**: render the design's empty-state (centered robot icon + helper
  text, see `screenshots/01-list.png` empty variant).

### B4. Filter list

- **Given**: list page rendered with ≥1 agent.
- **When**: the user types in the filter input.
- **Then**: rows are filtered case-insensitive against `name`, `description`,
  and `model`. No debounce. Filter state is component-local, not URL-routed.

### B5. Create new agent

- **Given**: list page rendered.
- **When**: the user clicks "+ Create Agent".
- **Then**: the list page is replaced (same content area, no route change)
  with the editor shell in "new" mode. Topbar shows "New agent". Identity
  section is focused, Name input empty. Save button labeled "Create Agent",
  disabled until Name is non-empty and matches `^[a-z0-9][a-z0-9-]*$`.

### B6. Save new agent

- **Given**: editor in "new" mode, Name validates.
- **When**: the user clicks "Create Agent".
- **Then**: a new Tauri command `create_user_agent(name, draft, project_path)`
  writes `<project>/.kiro/agents/<name>.json` atomically. The JSON includes
  `"$schema": "../agent-schema.json"` (matching existing-file convention).
  Empty optional string fields collapse to `null` in the serialized output.
  On success: navigate back to list, toast banner success variant.
  On `<name>.json` already exists: error banner above the editor body,
  editor stays open, no file written.

### B7. Edit existing agent (user-authored, no marketplace lineage)

- **Given**: list page; the row is NOT present in `InstalledAgents`.
- **When**: the user clicks "Edit".
- **Then**: editor opens pre-filled. Save writes via
  `save_user_agent(name, draft, project_path)` — atomic write to
  `<project>/.kiro/agents/<name>.json`.

### B8. Edit existing agent (marketplace-installed)

- **Given**: list page; the row IS present in `InstalledAgents`.
- **When**: the user clicks "Edit", makes changes, clicks "Save Changes".
- **Then**: a save-time dialog appears: "Keep linked to marketplace `<m>` /
  plugin `<p>` (future updates will need `--force`)" or "Detach from
  marketplace and treat as a user-authored agent." Save commits the choice:
  - **Keep linked**: write the JSON; leave `InstalledAgents` entry intact.
    The next marketplace install of this agent will hit
    `ContentChangedRequiresForce` (existing path, no new code).
  - **Detach**: write the JSON; remove the `InstalledAgents` entry.

### B9. Rename agent on save

- **Given**: editor in edit mode for agent named `foo`, user changes Name to
  `bar`, clicks Save.
- **When**: save executes.
- **Then**: write `bar.json` atomically. On success, delete `foo.json`. If
  `bar.json` already exists at the time of the write, save fails with a
  typed error (new variant; spec'd in the data model below) — neither file
  is modified. If the post-write delete of `foo.json` fails after `bar.json`
  has been written, surface a warning toast ("Renamed to `bar`; old file
  `foo.json` could not be removed: <error>") and proceed.

### B10. Duplicate agent

- **Given**: list page, a row's "duplicate" icon button.
- **When**: clicked.
- **Then**: a new Tauri command `duplicate_user_agent(source_name,
  project_path)` deep-clones the JSON (preserving every field), names it
  `<source>-copy` (or `-copy-2`, `-copy-3`, …, finding the first unused
  suffix), writes it, refreshes the list, toast banner. The duplicate is
  always user-authored — never carries `InstalledAgents` lineage even when
  the source is marketplace-installed.

### B11. Delete agent (user-authored)

- **Given**: list page; the row is NOT present in `InstalledAgents`.
- **When**: the user clicks the trash icon, confirms.
- **Then**: a new Tauri command `delete_user_agent(name, project_path)`
  removes `.kiro/agents/<name>.json`. List refreshes. Toast.

### B12. Delete agent (marketplace-installed)

- **Given**: list page; the row IS present in `InstalledAgents`.
- **When**: the user clicks the trash icon, confirms.
- **Then**: the same delete flow ALSO removes the `InstalledAgents` entry.
  Implementation mirrors the existing `remove_agent` semantic — i.e., the
  new `delete_user_agent` command delegates to the same core method used by
  the existing `remove_agent` Tauri command when tracking is present.

### B13. Cancel discards

- **Given**: editor open with unsaved changes.
- **When**: the user clicks "Cancel" or the back link.
- **Then**: no file write, navigate back to list, draft state discarded.
  No "unsaved changes" prompt in slice 1 (can be added later if it surfaces
  as a real complaint).

### B14. Schema gap fix (ships in this feature)

- The `description` field is added to `ComplexResource` (knowledge base
  entry) in both the Rust JSON-Schema generator and any tests that pin the
  schema shape. The UI emits the field; round-tripping the editor's output
  through `parse_native.rs` does not reject it.
- The authoritative agent schema (currently only in
  `Kiro Control Center Design System/design_handoff_agents/schemas/agent-spec.json`)
  is vendored into `crates/kiro-market-core/schemas/agent-spec.json` in
  slice 1, so the round-trip test from S5 can validate against a file the
  workspace owns rather than a design bundle that may be deleted post-port.
  Mirroring the same fix back upstream into kiro-cli's own schema is a
  follow-up out-of-scope for this feature; track via rivets issue.

## Slice scope

Decided in interrogation: multi-slice. Spec covers the full feature; slice
boundaries are an implementation choice, recorded here so each slice has a
defensible "done."

- **Slice 1 — Skeleton**: Behaviors B1–B14 above, but the editor body
  renders only **Identity** and **System Prompt** sections. The section rail
  shows the other five section names as disabled placeholders.
  Slice-1 success means a user can author a *minimum-viable* agent
  (name + description + model + prompt) end-to-end.
- **Slice 2 — Tools section**: design § 5 (auto-allowed list, available
  tools grid, external MCP tools group). Adds `lib/agents/tools-catalog.ts`
  as a static port of `agents-data.js` AGENT_TOOLS. No new Tauri commands.
- **Slice 3 — MCP Servers section**: design § 6. Stdio + http transports;
  registry transport included; OAuth fields skipped (out-of-scope below).
- **Slice 4 — Resources section + Knowledge Base modal**: design § 7. Ships
  the B14 schema gap fix.
- **Slice 5 — Hooks section**: design § 8.
- **Slice 6 — Advanced section**: design § 9 (legacy MCP toggle + JSON
  textarea for `toolsSettings`).

The section-rail count badges and live-counts behavior land slice-by-slice
as each section is wired.

## Success criteria

Each measurable. Each scoped to slice 1 unless noted.

| # | Criterion | Number / unit | Method |
|---|---|---|---|
| S1 | CRUD round-trip works end-to-end against a real `.kiro/agents/` directory | 1 Playwright e2e test passes | New `tests/e2e/agents.spec.ts` creates → edits (incl. rename) → duplicates → deletes an agent; asserts file presence/absence on disk after each step. Gated on `FIXTURE_MARKETPLACE_PATH` env, same pattern as `tests/e2e/app.spec.ts` |
| S2 | Marketplace-coexistence behavior works | 4 assertions in 1 integration test | New `crates/kiro-control-center/src-tauri/tests/agents_authoring.rs`: install an agent via marketplace, list shows lineage badge, edit-save-keep-linked preserves `InstalledAgents` entry, delete removes both file and entry |
| S3 | Type-check, lint, format pass | 0 errors / 0 warnings | `npm run check`; `cargo clippy --workspace --tests -- -D warnings`; `cargo fmt --all --check` |
| S4 | New Tauri commands follow the `_impl(svc, …)` pattern from CLAUDE.md | 100% of slice-1 service-consuming commands | Code review checklist; `_impl` tests in `commands/agents_authoring.rs` against `kiro_market_core::service::test_support` fixtures |
| S5 | Editor's emitted JSON parses cleanly through the existing native-agent parser | 1 round-trip test | New unit test in `crates/kiro-market-core/`: serialize a fully-populated `AgentDraft`, parse via `parse_native::parse_kiro_cli_agent_json`, assert structural equality |
| S6 | Save latency on local SSD | p95 ≤ 50ms for an agent JSON of ≤4KB | Manual timing during e2e test; logged via existing tracing instrumentation |
| S7 | Visual fidelity to the design bundle | Reviewer sign-off against screenshots 01, 02, 03-inline, 03-file | Subjective code-review pass; no per-pixel metric |
| S8 (full feature) | All 7 sections plus KB modal implemented, every field of `agent-spec.json` round-trips | 1 e2e + 1 round-trip test per slice | Each follow-up slice ships its own e2e step and round-trip extension |

## Edge cases and decisions

| Edge | Decision | Rationale |
|---|---|---|
| `.kiro/agents/` does not exist on view open | Auto-create on view open (eager mkdir) | User chose option B in the interrogation; see B1 |
| No project selected | Reuse existing no-project state from BrowseTab/InstalledTab | No new pattern; B2 |
| Empty `.kiro/agents/` directory | Render the design's empty-state | B3 |
| Filename collision on create | Save fails with typed error, editor stays open with banner | B6 |
| Filename collision on rename | Same; neither file modified | B9 |
| Rename succeeds but old-file delete fails | Toast warning, new file kept, old file orphaned | B9; best-effort cleanup |
| Marketplace agent edit, "keep linked" | `InstalledAgents` entry untouched; future install hits `ContentChangedRequiresForce` | B8; reuses existing detection mechanism |
| Marketplace agent edit, "detach" | Remove tracking entry, no future linkage | B8 |
| Delete marketplace agent | Remove both `.kiro/agents/<name>.json` and `InstalledAgents` entry | B12 (mirrors existing `remove_agent`) |
| Duplicate marketplace agent | Always produces user-authored (no lineage) copy | B10 |
| Agent name regex violation | Disable Save button; inline validation message under Name field | Per design § 2 save validation |
| Switching System Prompt mode (inline ↔ file) | Clears the value to prevent serializing wrong shape | Per design § 4 |
| `$schema` field on save | Write `"$schema": "../agent-schema.json"` matching existing-file convention | Decided in interrogation; schema target file may not exist but the convention persists |
| Concurrent edits (two Control Center windows open) | Last writer wins (no file-lock); future concern | Out of slice 1; matches existing `installed-agents.json.lock` use only for tracking writes |
| Orphan tracking entry: agent name in `installed-agents.json` but no file at `.kiro/agents/<name>.json` | Silently exclude from the list view in slice 1; file a rivets follow-up if real users hit the case | Surfaced by prove-it-prototype (`.agents-view/probe/README.md`, finding 2); both natural source-of-truth strategies arrive at silent-drop as the convergent default. Auto-removing the orphan tracking entry would risk losing recoverable state on a transient filesystem hiccup; surfacing a "missing file" UX row is feasible but out of slice 1 scope |
| Disk-write failure mid-save | Atomic write means partial-file state cannot leak; user sees error banner; no list refresh | Reuses existing `kiro-market-core` atomic-write helpers |
| Schema-gap KB `description` field | Add the field to `ComplexResource` Rust + JSON Schema in slice 4 | B14 |

## Out of scope

This change does NOT include:

- **MCP OAuth and `oauthScopes` fields** — the design bundle explicitly
  excludes them; `mcp__*__authenticate` flows are kiro-cli concerns, not
  Control Center authoring concerns.
- **`toolsSettings` per-tool typed forms** — kept as a JSON textarea
  (design § 9), no per-tool form generation.
- **Routing** — editor replaces list in-place; no URL change. URL-level
  routing is layered on top by the rest of the app if needed.
- **Autosave or unsaved-changes confirm prompt** — Cancel discards silently
  in slice 1.
- **Per-OS file-lock arbitration for concurrent Control Center windows** —
  last writer wins; not a slice-1 problem.
- **User-global `~/.kiro/agents/`** — only project-scoped. The existing
  Control Center model is project-scoped throughout.
- **Schema-validating saved files at write time** — `$schema` is written as
  metadata, but no runtime JSON Schema validation is performed; the
  existing `parse_native.rs` is the validation gate.
- **Dynamic native-tool catalog queried from kiro-cli at runtime** — static
  TS module port, per interrogation answer.
- **MCP opt-in dialog at save time** — user authored the config themselves;
  no opt-in.
- **Multi-pane editing (two agents side-by-side)** — out of scope.
- **Import / export of agent JSON files via OS file picker** — out of scope
  for slice 1; can be added later if requested.

## Constraints

| Dimension | Limit | How measured |
|---|---|---|
| `kiro-market-core` dependency cleanliness | Zero new `tauri` / UI / async-runtime / frontend deps in `kiro-market-core/Cargo.toml` | `cargo tree -p kiro-market-core` diff in PR review |
| Tauri command shape | All new service-consuming commands follow `_impl(svc, …)` pattern; project-only ones inline | Code review against CLAUDE.md "Tauri command handlers" section |
| FFI-crossing strings | All new wire-format types use validation newtypes (`AgentName` exists; introduce `KnowledgeBaseName` if needed) | bindings.ts inspection; `#[cfg_attr(feature = "specta", derive(specta::Type))]` on newtypes |
| Error variants on new core types | `#[non_exhaustive]` on every new pub enum touching the FFI; external errors mapped at adapter boundary (no `serde_json::Error` in public API) | `cargo xtask plan-lint --gate gate-4-external-error-boundary` passes |
| Save latency (slice 1) | p95 ≤ 50ms for agent JSON ≤4KB on local SSD | Manual / tracing during e2e |
| Existing tests stay green | All workspace tests pass | `cargo test --workspace` |
| No `.unwrap()` / `.expect()` in new production code | Zero, per CLAUDE.md zero-tolerance rule | `cargo xtask plan-lint --gate no-unwrap-in-production` passes |
| Bindings.ts regen | `bindings.ts` is up-to-date with new commands | `cargo test -p kiro-control-center --lib -- --ignored` passes |

## Decisions log

| # | Question | Decision | Why |
|---|---|---|---|
| 1 | How does this view interact with marketplace-installed agents tracked in `InstalledAgents`? | Mixed list, marketplace agents are fully editable | Requester chose maximum-freedom option in 2026-05-18 interrogation; downstream UX records lineage explicitly |
| 2 | On edit-save of a marketplace agent, what happens to `InstalledAgents` tracking? | Prompt at save time: keep-linked OR detach | Requester wants explicit per-save choice rather than implicit policy |
| 3 | On delete of a marketplace agent, what happens to tracking? | Remove both file and tracking entry (mirror existing `remove_agent`) | Symmetry with how skill removal works; avoids orphan tracking |
| 4 | Slice scope: single PR or many? | Multi-slice, skeleton first, then sections incrementally | Reviewer-friendly; lets early feedback reshape later sections |
| 5 | Native tool catalog source | Static TS module ported from design's `agents-data.js` | Kiro-cli tool surface is fixed per release; no runtime query exists |
| 6 | MCP opt-in on save? | No opt-in — the user typed the config themselves | Opt-in exists for marketplace install because user is consuming others' config; symmetry breaks here |
| 7 | KB `description` schema gap | Fix upstream — add field to schema + Rust type in slice 4 | Avoids future inconsistency; one-line schema bump |
| 8 | Rename semantics | Write new, delete old; error on target collision | Simplest; no in-place state to track |
| 9 | Empty / missing `.kiro/agents/` | Auto-create on view open | Requester chose eager creation over lazy-on-first-save |
| 10 | `$schema` field on save | Write `"../agent-schema.json"` on every save | Matches existing-file convention in `.kiro/agents/*.json`; kiro-cli's own schema does not publish a canonical public URL |
| 11 | Are the schemas in `design_handoff_agents/schemas/` authoritative? | Yes — confirmed by requester 2026-05-18 as the definitive kiro-cli schemas | Strengthens decisions #7 and #10; pin a vendored copy in `crates/kiro-market-core/schemas/` so the workspace owns its validation target |
| 12 | What does the list view show when `installed-agents.json` references an agent whose file is missing on disk? | Silently exclude in slice 1; rivets follow-up if it becomes a real UX problem | Surfaced by prove-it-prototype 2026-05-21 — both natural source-of-truth strategies converge on silent-drop; deciding to *surface* missing entries would require a new row state and a "remove tracking?" affordance, scope-creeping slice 1 for a hypothetical user need |
| 13 | What does the list view do with a `*.json` file in `.kiro/agents/` that fails JSON parsing? | Silently exclude and `tracing::warn!`-log in slice 1; rivets follow-up `kiro-fd40` (broken-row UX) deferred unless users hit it | Surfaced during falsifiable-design 2026-05-21; parallels D12's silent-skip discipline. A broken-row UX is feasible but slice 1 scope-creep; users can edit `.kiro/agents/` outside the Control Center and fix it externally |
| 14 | What `name` does the list-row carry when the JSON file's `name` field is absent or disagrees with the filename stem? | Use the JSON's `name` field if present, else fall back to the filename stem | Surfaced during falsifiable-design 2026-05-21 — save path enforces filename=name, but the list path is tolerant of pre-existing drift so broken state stays visible rather than hidden. Empirically validated: probe + oracle both produce a row for a no-name fixture file with name=filename-stem |

## Sign-off

The requester typed, verbatim:

> This feature enables developers to manage kiro agents through kiro control center. You can manage existing agents you created or that came through plugins, or create your own. Slice 1 is a thin narrow feature to be able to create a minimum viable agent. The bar for done is long, but it comes down to CRUD for an agent

Date: 2026-05-21

Vocabulary note: the requester's "plugins" maps to the spec's "marketplace"
(in kiro-cli, plugins are the unit shipped via marketplaces; the existing
`InstalledAgents` tracking and Tauri-command surface use the "marketplace"
nomenclature, which the spec preserves for code-search consistency).
No semantic divergence between the restatement and the artifact.

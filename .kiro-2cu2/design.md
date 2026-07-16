# Falsifiable design — BrowseTab MCP consent

**Issue:** kiro-2cu2  
**Status:** cheapest falsifier passed; awaiting explicit design approval  
**Probe:** `.kiro-2cu2/probe.py`  
**Independent oracle:** `.kiro-2cu2/oracle.mjs`  

## Purpose

Give a project maintainer an explicit, fail-closed way to consent to MCP-bearing marketplace agents from both BrowseTab install surfaces: whole-plugin Install/Update on `PluginCard`, and per-item Apply in `CustomizeDrawer`.

The existing backend gate remains authoritative. The UI only supplies an explicit boolean; `false` continues to skip MCP-bearing agents and return `mcp_servers_require_opt_in`, while `true` permits the existing install path.

## Probe constraint

Probe and AST oracle agree on three facts:

1. `AgentItemInfo` has six fields and no pre-install MCP signal.
2. BrowseTab's drawer and whole-plugin paths both submit `false`.
3. `mcp_servers_require_opt_in` exists only in post-install results.

A conditional pre-install control therefore requires catalog metadata. Replacing the two literals without that metadata would produce an unexplained control on every plugin or require a two-attempt retry flow.

## Input shapes

### Catalog agents

| Dimension | Production-reachable shapes |
|---|---|
| Plugin agents | empty; one; multiple |
| Dialect | Claude markdown (no MCP syntax); Copilot markdown; native Kiro JSON |
| MCP map | absent/empty; one server; multiple servers with repeated transport; multiple mixed transports (`stdio`, `http`, `sse`); unfamiliar future label in the generated client |
| Parse result | valid; malformed MCP config routed to the existing per-item parse skip |
| Install state | installed; not installed (orthogonal to MCP presence) |

### Consent surfaces

| Surface | Shapes |
|---|---|
| PluginCard | plugin has no MCP servers; has one; has several across agents; Install; Update; action pending |
| CustomizeDrawer | no agent install diff; selected non-MCP agents only; one MCP agent; mixed MCP/non-MCP agents; multiple MCP agents |
| Consent | unchecked; checked; checked then MCP selection grows/shrinks; consumed by an action; component destroyed and recreated |
| Plugin identity | one plugin; switch A → B; filter removes/recreates a keyed card |
| Install mode | normal; force; update (force implied by existing `runPluginInstall`) |
| Result | full success; partial success with MCP warning; wrapper error; refresh error |

Malformed catalog agents cannot present a consent control because they do not produce `AgentItemInfo`; their existing parse-skip row remains the observable outcome.

## Removed-invariant sweep

This feature is subtractive underneath: explicit consent removes BrowseTab's current invariant that `acceptMcp` is always false.

The old constant guaranteed, for free:

1. BrowseTab could never install an agent that spawns a process or connects to an MCP endpoint.
2. Consent could not leak between plugins, surfaces, retries, or updates because consent did not exist.
3. Force mode could not accidentally imply MCP consent.
4. A false-gated MCP agent always produced a warning rather than an installed file.

The design keeps these properties except for the exact, visible, one-shot action where the maintainer checks consent. Claims C5–C10 fence the relaxed invariant.

## Architecture

### Core catalog projection

Add one required field to the existing wire type:

```rust
pub struct AgentItemInfo {
    // existing fields...
    /// One normalized transport label per declared MCP server.
    /// Empty means this agent needs no MCP consent.
    pub mcp_server_transports: Vec<String>,
}
```

`list_agents_with_manifest` already holds the parsed `AgentDefinition` or `NativeAgentBundle`, and both expose typed `BTreeMap<String, McpServerConfig>` values. A private browse helper maps each value through `transport_label()`.

Contract:

- one vector element per server; duplicates retained so `len()` is the server count;
- labels normalized to `stdio`, `http`, or `sse` today;
- deterministic BTreeMap key order inside each agent;
- empty vector for absent/empty MCP maps;
- every non-empty vector triggers consent, including an unfamiliar future label;
- no command, URL, header, argument, or environment value crosses the catalog wire.

`Vec<String>` intentionally mirrors the existing `InstallWarning::McpServersRequireOptIn.transports` contract. Core parsing is the producer, so current values remain constrained by `McpServerConfig`; the frontend does not exhaustively switch on strings and therefore remains fail-closed for a future transport. Regenerate `bindings.ts`; do not edit it manually.

### Pure drawer/catalog derivations

Deepen the existing `drawer-diff.ts` module rather than add a consent store or state wrapper:

```ts
export type CustomizeDrawerApply = {
  skills: { install: string[]; remove: string[] };
  steering: { install: string[]; remove: string[] };
  agents: { install: string[]; remove: string[] };
  acceptMcp: boolean;
};

export type McpConsentSummary = {
  agentNames: readonly string[];
  serverCount: number;
  transports: readonly { label: string; count: number }[];
};

export function summarizePluginMcp(
  agents: readonly AgentItemInfo[],
): McpConsentSummary | null;

export function summarizeSelectedMcpInstalls(
  agents: readonly AgentItemInfo[],
  selected: ReadonlySet<string>,
): McpConsentSummary | null;
```

The first helper includes every MCP-bearing agent in a whole-plugin action. The second includes only selected, not-installed agents. Explicit functions avoid a `null`/optional scope sentinel whose meaning callers would have to remember. Both summaries preserve catalog order in `agentNames`, sort transport buckets lexicographically, and report the raw server count. `null` means the action needs no MCP consent. Duplicate transports remain in the server count and are grouped only for display.

This is the test seam for scope/count math. Reactive consent remains boring leaf-owned `$state(false)` in each Svelte component.

### PluginCard

Each keyed card owns `let acceptMcp = $state(false)`. MCP summary is `$derived.by` from the reactive `entry` prop; a plain initialization would become stale after catalog refresh.

For a card whose whole-plugin summary is non-null and whose action state is Install, Update, or a pending Install/Update, add a full-width warning divider below the description and before the actions in DOM/tab order:

```text
MCP access · 2 agents · 1 stdio, 1 http
[ ] Allow MCP servers declared by terraform-tools
    MCP servers can run local commands or connect to external services.
    If unchecked, those agents are skipped; other plugin items still install.
    This choice is not saved.
                                      [Customize] [Install]
```

Do not crowd the existing compact action cluster with a third inline control. Use a native checkbox, `kiro-warning` divider/background tokens, visible text (not `title`), a focus ring, and `aria-describedby`. The accessible label includes the plugin name. The control is absent for Manage-only states and disabled while pending.

`onInstall` and `onUpdate` widen from `() => void` to `(acceptMcp: boolean) => void`. Each click handler snapshots `hasMcp && acceptMcp`, immediately resets local state to false, and passes the primitive to the parent. BrowseTab sends it to `runPluginInstall`; force/update mode remains independent.

### CustomizeDrawer

The drawer derives an MCP summary only from newly selected agents in `diff.agents.install`. It marks each MCP-bearing agent row with a textual `MCP · <transports>` badge and renders the warning divider at the top of the fixed footer only while the summary is non-null:

```text
MCP access
[ ] Allow MCP servers declared by terraform-tools
    1 selected agent uses MCP (stdio). MCP servers can run local commands
    or connect to external services. If unchecked, that agent is skipped;
    other selected changes still apply. This choice is not saved.
Apply will install 2 skills. 1 selected MCP agent will be skipped.
[ Apply ]
```

The dynamic footer status uses `aria-live="polite"` without stealing focus. The native checkbox uses visible label/detail text linked with `aria-describedby`.

Any agent selection mutation resets `acceptMcp` to false; skill and steering mutations do not. The emitted payload uses the final guard `acceptMcp: summary !== null && acceptMcp`, then immediately resets local state before awaiting the parent. Apply remains usable while unchecked: safe items/removals proceed, MCP agents are skipped, and the structured warning remains visible after the drawer closes.

Move the apply payload type out of the component into `drawer-diff.ts` so BrowseTab and the drawer share one interface. `applyDrawerDiff` consumes `diff.acceptMcp` only in `commands.installAgents`.

### Component identity fence

`CustomizeDrawer` currently seeds selection sets with `untrack`, while its comment assumes a truthy `{#if drawerEntry}` recreates the component. Svelte updates props when a truthy conditional stays truthy; it does not remount automatically. Nest the drawer under:

```svelte
{#key pluginKey(drawerMarketplace, drawerEntry.plugin)}
  <CustomizeDrawer ... />
{/key}
```

This guarantees both selection and consent reinitialize when plugin identity changes. Plugin cards are already keyed by `(marketplace, plugin)`.

### Warning fallback and TOCTOU posture

Existing whole-plugin and drawer warning formatters remain. Metadata controls pre-install visibility; the post-install warning remains the authoritative fallback if catalog and source change between refresh and install.

Consent authorizes one install attempt from the named plugin, not a specific content hash. This matches the CLI's existing `--accept-mcp` contract. The UI never claims the maintainer reviewed command lines or URLs; it discloses transport class and keeps the core parser/gate authoritative.

## Decisions requiring approval

1. **Signal source — recommend catalog metadata.** Rejected alternatives: show the control on every plugin (unnecessary security noise), or require a failed first attempt before revealing consent (double-run with partial side effects).
2. **Scope — recommend per plugin, per surface, per action.** No marketplace/workspace preference. Consent starts unchecked and is consumed once.
3. **Unchecked action — recommend partial install.** Do not disable Apply/Install; safe items proceed and MCP-bearing agents remain skipped with the existing warning. Summary copy explicitly names the skipped subset.
4. **Disclosure — recommend affected agents + count + normalized transports.** Do not expose raw command lines, URLs, headers, environment values, or credentials in the browse catalog.
5. **Authorization granularity — recommend one plugin action, not a content fingerprint.** This matches `--accept-mcp`; catalog drift is disclosed through the backend warning only when consent is false, not by binding consent to a source hash.
6. **Manual fences — approve C6 and C7 browser scenarios.** This repository has no CI Svelte component-interaction harness; pure derivations and backend behavior remain deterministic CI fences.

## Claims

1. **C1:** Every valid catalog agent carries exactly one normalized transport label per declared MCP server for Copilot and native dialects, and an empty vector otherwise.
2. **C2:** Malformed MCP declarations remain parse skips and never become catalog agents with fabricated metadata.
3. **C3:** Generated TypeScript bindings expose required `mcp_server_transports: string[]` on `AgentItemInfo` without handwritten binding edits.
4. **C4:** MCP summaries count only included, not-installed agents, retain duplicate transports in the server total, and group transport copy deterministically.
5. **C5:** `runPluginInstall` forwards `acceptMcp` unchanged to the Tauri command independently of force mode.
6. **C6:** PluginCard shows a local unchecked control only for MCP-bearing Install/Update actions, emits the current relevant boolean once, resets immediately, and never shares state with another keyed card.
7. **C7:** CustomizeDrawer shows consent only when the agent install diff contains MCP-bearing agents, resets on every agent-selection mutation/plugin remount, emits a fail-closed payload, and accurately announces skipped-vs-included outcomes.
8. **C8:** With consent false, mixed safe/MCP batches install safe agents, skip MCP agents, write no MCP agent file, and retain the structured warning; with consent true, the same MCP agent installs.
9. **C9:** Force mode and MCP consent remain independent booleans; checking Force does not check or imply MCP consent.
10. **C10:** Unknown future non-empty transport labels still trigger the control and render as escaped text rather than being treated as safe or causing an exhaustive-switch failure.

## Falsification

| # | Claim | Falsifier | Independent oracle | Cost | Status | Regression fence |
|---|---|---|---|---:|---|---|
| C1 | Exact transport projection | Catalog fixture with Copilot `{a: stdio, b: stdio, c: http}`, native `{x: sse}`, and no-MCP Claude agent. Any vector other than two `stdio` + one `http`, one `sse`, and empty respectively falsifies. A buggy boolean/dedupe projection fails. | Hand-authored manifest/JSON maps and their counted entries. | 10m | pending | Core tests `list_agents_for_plugin_surfaces_translated_mcp_transports` and `...native_mcp_transports` |
| C2 | Parse failures stay skips | Malformed `mcp-servers` entry missing required transport fields. A catalog agent or consent metadata instead of `AgentParseSkip` falsifies. A permissive fallback fails. | Fixture is invalid against `McpServerConfig`'s required fields. | 10m | pending | Core test `list_agents_for_plugin_malformed_mcp_remains_skipped` |
| C3 | Binding carries field | Regenerate twice and parse `AgentItemInfo` with TypeScript AST. Missing/non-array field or second-run diff falsifies. Forgetting specta propagation fails. | Rust struct field plus TypeScript compiler AST. | 5m | pending | Existing ignored binding generator + widened `bindings_export_plugin_catalog_view` fence |
| C4 | Summary is subset-accurate | A selected uninstalled agent has two stdio, an unselected agent one http, a selected already-installed agent one sse, and another selected agent unknown `quic`. Expected 2 agents/3 servers/`stdio:2,quic:1`; inclusion of http/sse or dedupe to 2 servers falsifies. | Hand-counted fixture. | 5m | pending | `drawer-diff.test.ts` MCP subset/duplicate/unknown cases |
| C5 | Whole-install helper forwards boolean | Call `runPluginInstall` with `acceptMcp: true`; observe fifth Tauri argument. `false` falsifies and is exactly the current hardcoded bug shape. | Injected Vitest spy arguments. | 1m | **passed** | Existing test `acceptMcp: true propagates to Tauri command`; add update-mode + force × consent matrix |
| C6 | Card control is scoped and consumed | Browser fixture with plugin A (MCP) and B (none): B has no control; check A, filter A out/back, then Install and Update. Wrong visibility, retained check, shared state, or false invocation falsifies. A plain global boolean fails. | Browser accessibility tree plus captured Tauri invocation. | 15m | pending | **Manual browser fence requiring approval**; C4/C5 provide deterministic math/relay fences |
| C7 | Drawer control follows install diff | Open mixed plugin: installed/unselected MCP agents yield no control; select an uninstalled MCP agent → unchecked control; check it, grow selection → unchecked; switch plugin key → fresh selection/consent; Apply copy and invocation reflect false/true. Always-visible, stale-checked, stale selection, misleading copy, or wrong invocation falsifies. | Catalog fixture flags, accessibility tree, and captured Tauri invocation. | 15m | pending | **Manual browser fence requiring approval**; C4 provides deterministic subset fence |
| C8 | Backend remains fail-closed | Mixed safe/MCP fixture under false and true. MCP file appearing under false, warning missing, safe agent skipped, or MCP file absent under true falsifies. A UI-side bypass of the backend gate fails. | Temp filesystem and hand-authored source set. | 2m | passed substrate | Existing core tests `install_plugin_agents_skips_mcp_agents_without_opt_in` and `...installs_mcp_agents_when_opted_in`; add mixed-batch arm |
| C9 | Force is independent | Four-cell table `(force, consent)`: only consent controls fifth Tauri arg; force controls fourth. Any equality/coupling between them falsifies. Reusing `forceInstall` as consent fails. | Injected spy argument matrix. | 5m | pending | `plugin-actions.test.ts` parameterized force × consent test |
| C10 | Unknown transport fails closed | Pure summary fixture with `mcp_server_transports: ["quic"]`. Null summary, hidden relevance, dropped label, or thrown switch falsifies. Whitelisting only stdio/http/sse fails. | Hand-authored non-empty future label. | 3m | pending | `drawer-diff.test.ts` unknown-label case |

### Cheapest falsifier execution

Command:

```text
npm run test:unit -- src/lib/plugin-actions.test.ts \
  -t "acceptMcp: true propagates to Tauri command"
```

Result: **1 passed, 30 skipped**. The existing deep `runPluginInstall` seam already forwards `true`; the missing behavior is catalog metadata, leaf state, presentation, and the two BrowseTab call sites, not the helper or Tauri contract.

## Negative space

1. **InstalledTab Update is excluded under verified issue kiro-yr2f.** This branch changes BrowseTab only.
2. **No persistent consent preference.** Per-plugin, per-marketplace, and workspace settings would outlive the action whose risk text the maintainer saw.
3. **No raw MCP configuration in browse results.** Count and transport are enough for this consent decision; omitting commands, URLs, headers, environment values, and credentials minimizes wire exposure.
4. **No per-server or per-agent consent matrix.** Existing backend contract is one boolean for a plugin/agent batch; the UI mirrors that interface rather than inventing finer semantics it cannot enforce.
5. **No automatic retry.** A false-gated install remains observable through the existing warning; the UI never turns that warning into implicit consent.
6. **No change to backend gate semantics or CLI `--accept-mcp`.** The established core tests remain the authority.

## Verification posture

- Deterministic Rust and Vitest fences cover projection, parse failure, binding generation, subset/count math, unknown labels, argument propagation, force independence, and backend safety.
- `svelte-check` and the official Svelte autofixer cover component prop/reactivity correctness.
- C6 and C7 use real Tauri/browser smoke evidence because this repository has no CI component-interaction harness. Approving this design explicitly approves those two manual regression fences for this PR.

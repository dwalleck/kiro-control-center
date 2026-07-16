# Budgeted plan — BrowseTab MCP consent

**Issue:** kiro-2cu2  
**Approved design:** `.kiro-2cu2/design.md` at commit `b7fc2bd`  
**Probe/oracle:** `.kiro-2cu2/probe.py` + `.kiro-2cu2/oracle.mjs`; exact agreement re-confirmed at commit `57016c4`  
**Cheapest falsifier:** `acceptMcp: true propagates to Tauri command` — 1 passed, 30 skipped  

Seven slices. Every slice changes at most two source/test files. Pre-typed code is advisory; the claim, fixture, oracle, and budgets are the contract.

## Gates common to every slice

After the slice-specific checks:

1. `cargo nextest run`
2. `cargo clippy --workspace --all-targets -- -D warnings`
3. `cargo fmt --all -- --check`
4. `cargo test --workspace --doc`
5. Rebuild the affected Rust/frontend target.
6. Run probe and independent AST oracle and require exact parsed-JSON equality.
7. Run the slice's named regression fence.
8. Check the slice's loop budget against the stated production scale.

Frontend slices also run `npm run test:unit`, `npm run check`, and `npm run build` from `crates/kiro-control-center`.

---

## Slice 1: Project MCP transport metadata into catalog agents

**Claim:** C1 + C2 — every valid Copilot/native catalog agent carries one normalized label per declared MCP server; no-MCP agents carry `[]`; malformed MCP remains an `AgentParseSkip` rather than fabricated metadata.

**Oracle:** The Python regex probe and Svelte/TypeScript AST oracle agree that `AgentItemInfo` now has an MCP field while both BrowseTab consent arguments remain literal `false`. Hand-authored MCP maps independently establish expected labels/counts.

**Stress fixture:** In one catalog fixture, include: a Copilot agent with BTreeMap entries `a=stdio`, `b=stdio`, `c=http`; a native agent with `x=sse`; a Claude agent with no MCP; and a malformed Copilot MCP entry missing required transport data. Expected vectors are `['stdio','stdio','http']` in key order, `['sse']`, and `[]`; the malformed source appears only in parse skips. This targets one-dialect-only projection, accidental deduplication, and permissive parse fallback.

**Smallest code change:** Add required `mcp_server_transports: Vec<String>` to `AgentItemInfo`, one private iterator/collector over `McpServerConfig::transport_label`, populate both translated and native projections, and add focused inline Rust tests.

**Loop budget:**
- New inner projection is `O(S)` per valid agent, where `S` is declared MCP servers.
- Existing catalog traversal makes incremental total `O(sum(S))` across agents.
- Production bound: ≤100 marketplaces/plugins in one selected catalog × ≤20 agents/plugin × ≤10 servers/agent = ≤20,000 label visits and allocations per forced catalog refresh, far below $10^6$ operations and with zero new syscalls.

**Wall budget:** N/A — catalog refresh is user/load triggered, not an always-on phase; no new I/O is introduced.

**Files:**
- `crates/kiro-market-core/src/service/browse.rs`

**Verification:**
- [ ] Focused Rust projection/parse tests pass
- [ ] Stress fixture yields exact vectors and parse skip
- [ ] Probe and oracle JSON agree
- [ ] Incremental label visits ≤20,000 at stated scale
- [ ] C1/C2 regression fences pass
- [ ] Common full gates pass

---

## Slice 2: Regenerate and fence the TypeScript wire contract

**Claim:** C3 — generated TypeScript requires `AgentItemInfo.mcp_server_transports: string[]`, without handwritten binding drift or chrono/PathBuf leakage.

**Oracle:** Rust's specta-derived `AgentItemInfo` field and a TypeScript compiler-AST query independently report the same required array field. The ignored binding generator is run twice; byte-identical output proves determinism.

**Stress fixture:** Run the binding generator, checksum `bindings.ts`, run it again, and compare checksum plus AST. Missing field, optional field, non-array type, or second-run byte drift falsifies. This targets forgetting one FFI derive/path and manual generated-file edits.

**Smallest code change:** Regenerate `bindings.ts`; strengthen the existing `bindings_export_plugin_catalog_view` Rust test to assert the required field/type and retained no-chrono constraint.

**Loop budget:** No runtime loops. Test/compiler AST traversal is build-time `O(N)` over one generated file (~1,300 lines).

**Wall budget:** N/A — build/test only.

**Files:**
- `crates/kiro-control-center/src/lib/bindings.ts` (generated only)
- `crates/kiro-control-center/src-tauri/src/lib.rs`

**Verification:**
- [ ] Ignored binding generator passes twice
- [ ] Two generated-file checksums match
- [ ] Binding AST reports required `string[]`
- [ ] `bindings_export_plugin_catalog_view` and no-chrono fences pass
- [ ] Probe and oracle JSON agree
- [ ] Common full gates pass

---

## Slice 3: Add pure MCP scope and display summaries

**Claim:** C4 + C10 — pure helpers distinguish whole-plugin scope from selected/not-installed drawer scope, preserve raw server count, group labels deterministically, and treat every unknown non-empty label as consent-requiring.

**Oracle:** Hand-counted input: selected uninstalled agent A has `['stdio','stdio']`; unselected B has `['http']`; selected installed C has `['sse']`; selected uninstalled D has `['quic']`. Drawer expectation is agent names `[A,D]`, server count `3`, buckets `quic:1, stdio:2`. Whole-plugin expectation includes all four MCP agents and all five servers. Empty/no-MCP input yields `null`.

**Stress fixture:** The exact four-agent fixture above plus empty input and unknown `quic`. Inclusion of B/C in drawer scope, deduping to two servers, dropping `quic`, unstable bucket order, or a non-null empty summary falsifies.

**Smallest code change:** In `drawer-diff.ts`, add shared `CustomizeDrawerApply`, `McpConsentSummary`, `summarizePluginMcp`, and `summarizeSelectedMcpInstalls`; add focused Vitest cases. Use built-in `Map`/sort rather than a new dependency.

**Loop budget:**
- Filtering/aggregation: `O(A + S)`, with `A` agents and `S` total transport entries.
- Bucket ordering: `O(U log U)`, with `U` unique labels.
- Production bound: `A ≤ 100`, ≤10 servers/agent gives `S ≤ 1,000`; conservatively `U ≤ 1,000`, so <11,000 comparisons/visits per reactive recomputation. User-driven selection/catalog changes only; below $10^6$.

**Wall budget:** N/A — no polling/always-on phase.

**Files:**
- `crates/kiro-control-center/src/lib/drawer-diff.ts`
- `crates/kiro-control-center/src/lib/drawer-diff.test.ts`

**Verification:**
- [ ] New tests are observed failing before helper implementation, then pass
- [ ] Adversarial subset/duplicate/unknown fixture matches exact expected summary
- [ ] Empty/no-MCP inputs return `null`
- [ ] Probe and oracle JSON agree
- [ ] Cost remains <11,000 visits/comparisons at stated scale
- [ ] C4/C10 regression fences pass
- [ ] Common Rust and frontend gates pass

---

## Slice 4: Fence backend partial-install safety

**Claim:** C8 — false consent installs safe agents, skips MCP agents, writes no MCP destination, and returns structured warning; true consent installs the same MCP agent.

**Oracle:** A temp project filesystem is independent of the returned vectors: destination existence/hash establishes what was written; hand-authored source agents establish which files are safe/MCP.

**Stress fixture:** One batch contains `safe-agent`, `mcp-stdio`, and `mcp-http`. Under false, only `safe-agent` exists and both MCP names are skipped with transport-preserving warnings; under true all three exist. This targets the plausible overcorrection “one MCP item aborts the entire batch” and accidental UI-side bypass assumptions.

**Smallest code change:** Add/extend one core service regression test around the existing gate; production core semantics remain unchanged unless the fixture exposes real drift.

**Loop budget:** No production loops added. Test setup is `O(3)` files.

**Wall budget:** N/A — test only.

**Files:**
- `crates/kiro-market-core/src/service/mod.rs`

**Verification:**
- [ ] Mixed-batch core test passes with exact files/results/warnings
- [ ] Temporary mutation that aborts the mixed batch makes the fence fail, then is removed
- [ ] Probe and oracle JSON agree
- [ ] No production complexity change
- [ ] C8 regression fence passes
- [ ] Common full gates pass

---

## Slice 5: Fence force and consent as independent action inputs

**Claim:** C5 + C9 substrate — `runPluginInstall` forwards force and consent unchanged in all four boolean combinations; update mode forces only the force dimension.

**Oracle:** Injected Tauri spy arguments are independent of helper internals. For `(force, consent)` in `FF, FT, TF, TT`, fourth/fifth command arguments must exactly match the matrix. Update-mode expectation is `(true, suppliedConsent)`.

**Stress fixture:** Four-cell matrix plus update-mode `consent=false` and `consent=true`. Swapping arguments, using force as consent, forcing consent during update, or retaining a literal false fails. This targets the exact coupling bug security review is concerned about.

**Smallest code change:** Expand `plugin-actions.test.ts` around the already-passing helper; no production change expected. Characterization tests are validated by a temporary coupled-argument mutation that must fail before restoration.

**Loop budget:** No production loops. Parameter table is six test cases, `O(6)`.

**Wall budget:** N/A — test only.

**Files:**
- `crates/kiro-control-center/src/lib/plugin-actions.test.ts`

**Verification:**
- [ ] Four-cell + update matrix passes
- [ ] Temporary force/consent coupling makes the matrix fail, then is removed
- [ ] Existing cheapest falsifier still passes
- [ ] Probe and oracle JSON agree
- [ ] No production complexity change
- [ ] C5/C9 substrate fences pass
- [ ] Common Rust and frontend gates pass

---

## Slice 6: Wire one-shot consent into PluginCard Install and Update

**Claim:** C5 + C6 + C9 UI — only MCP-bearing Install/Update cards show a local unchecked disclosure; click snapshots a relevant boolean once, resets it, and BrowseTab relays it independently from force/update mode.

**Oracle:** Real Tauri calls write or skip MCP agent files in a temp project, while the browser accessibility tree independently establishes checkbox visibility, label, description, unchecked default, reset, and per-card isolation.

**Stress fixture:** Real local marketplace with plugin A (safe agent + MCP agent using duplicate stdio/http servers) and plugin B (safe only). Verify: B has no control; A does; A starts unchecked; hide/re-show A via search resets; unchecked Install writes safe only and warns; checked Install writes MCP; mutate A's local source to create Update, verify Update control starts unchecked, false update skips new MCP content, checked update installs it; Force/update never checks the box. Observe disabled/busy posture during action. This targets global state, stale `$derived`, Manage-only leakage, pending double-submit, and update implicitly accepting MCP.

**Smallest code change:** Restructure `PluginCard` root to keep content/risk/action DOM order with current visual hierarchy; add local state/derived summary/accessible disclosure and boolean callbacks; widen BrowseTab's callbacks and `runPluginInstall` call-site input.

**Loop budget:** No new loop beyond `summarizePluginMcp`, budgeted in Slice 3 at <11,000 visits/comparisons. Markup itself adds no traversal/polling.

**Wall budget:** N/A — reactive work runs only when props/actions change; backend operation latency is unchanged.

**Files:**
- `crates/kiro-control-center/src/lib/components/PluginCard.svelte`
- `crates/kiro-control-center/src/lib/components/BrowseTab.svelte`

**Verification:**
- [ ] Svelte compile/check/autofixer are clean
- [ ] Browser/Tauri stress fixture produces exact visibility/reset/filesystem/warning outcomes
- [ ] Probe and oracle agree with both reporting whole-plugin consent as `dynamic`
- [ ] Slice 5 force/consent matrix passes
- [ ] Summary helper stays within Slice 3 budget
- [ ] C5/C6/C9 regression fences pass; C6 manual evidence recorded in commit
- [ ] Common full gates pass

---

## Slice 7: Wire scoped consent into CustomizeDrawer Apply

**Claim:** C7 + C8 relay — the drawer shows consent only for selected, not-installed MCP agents; accurately describes skipped/included subsets; resets on any agent-selection change and plugin remount; and sends a fail-closed boolean only to `installAgents`.

**Oracle:** The pure Slice 3 summary determines expected scope independently; real Tauri filesystem/result banners establish what Apply did; browser accessibility tree establishes dynamic disclosure/badges/reset/copy.

**Stress fixture:** In plugin A, include one installed MCP agent, one uninstalled safe agent, and two uninstalled MCP agents with duplicate/mixed transports. Initial and installed-only/removal states have no control. Select one MCP agent: unchecked control appears with badge/detail; check it, then change any agent selection: it resets. With unchecked mixed changes, safe install/removal succeeds, MCP files stay absent, and summary/banner says they are skipped. Reopen, check, Apply: MCP files appear. While A drawer remains truthy, programmatically click plugin B's behind-overlay Customize action to force A→B prop identity; keyed remount must seed B selections and clear consent. This targets selection leakage from `untrack`, stale consent when scope grows, misleading summary counts, and passing true when relevance disappears.

**Smallest code change:** Move drawer payload to shared type; derive MCP scope/copy; add local state, agent badges, native accessible footer disclosure, selection/apply resets; pass `diff.acceptMcp` to `installAgents`; key the drawer by marketplace/plugin in BrowseTab.

**Loop budget:**
- Existing diff loops remain `O(A)`.
- New MCP summary is Slice 3's `O(A + S + U log U)` (<11,000 at scale).
- Per-agent badge label dedupe is `O(S)` each, total `O(A × S_per_agent)` ≤1,000 visits/render.
- Summary copy walks three fixed categories plus `U` transport buckets; ≤1,003 visits.
- No new syscalls or polling.

**Wall budget:** N/A — all recomputation is user-event/prop driven.

**Files:**
- `crates/kiro-control-center/src/lib/components/CustomizeDrawer.svelte`
- `crates/kiro-control-center/src/lib/components/BrowseTab.svelte`

**Verification:**
- [ ] Svelte compile/check/autofixer are clean
- [ ] Browser/Tauri stress fixture matches scope/reset/copy/filesystem outcomes
- [ ] Direct A→B truthy identity switch remounts and clears selection/consent
- [ ] Probe and oracle agree with drawer consent reported as `dynamic`
- [ ] Slice 3 summary and Slice 4 backend fences pass
- [ ] Costs remain <13,003 visits/comparisons at stated scale
- [ ] C7/C8 regression fences pass; C7 manual evidence recorded in commit
- [ ] Common full gates pass

---

## Final integration gate

After Slice 7:

- run all common Rust/frontend gates once more;
- run every focused regression fence from C1–C10;
- rerun real Tauri browser scenarios for PluginCard and CustomizeDrawer;
- rerun probe/oracle exact JSON comparison;
- confirm generated bindings are deterministic;
- confirm no raw MCP commands/URLs/headers/environment values entered the catalog wire;
- confirm no consent state exists in a global store or persistence layer.

## Plan self-review

### List 1 — loops and budgets

| Slice | Loop | Cost | Production scale | Within ceiling |
|---|---|---|---|---|
| S1 | MCP server projection | `O(sum S)` | ≤20,000 labels/catalog refresh | yes |
| S2 | build-time TS AST | `O(N)` | ~1,300 generated lines | yes |
| S3 | agent/server aggregation | `O(A + S)` | ≤1,100 visits/recompute | yes |
| S3 | transport bucket sort | `O(U log U)` | ≤1,000 labels; <10,000 comparisons | yes |
| S4 | test setup only | `O(3)` | 3 fixture agents | yes |
| S5 | test matrix only | `O(6)` | 6 cases | yes |
| S6 | reuses S3 | no additional loop | <11,000 | yes |
| S7 | badge labels | `O(A × S_per_agent)` | ≤1,000 visits/render | yes |
| S7 | summary copy | `O(U)` | ≤1,003 visits | yes |

No always-on phase and no new syscall loop; no wall budget required.

### List 2 — adversarial fixtures

| Slice | Bug class targeted |
|---|---|
| S1 | one dialect omitted; server labels deduplicated; malformed config accepted |
| S2 | FFI derive forgotten; generated file hand-edited/non-deterministic |
| S3 | installed/unselected items included; duplicates collapsed; future label whitelisted away |
| S4 | one MCP item aborts safe batch; false consent still writes MCP |
| S5 | force and consent swapped/coupled; update implies consent |
| S6 | global/stale card state; Manage/pending leakage; update bypass |
| S7 | `untrack` selection leak; scope growth keeps consent; summary promises skipped installs |

Every fixture names a plausible bug and includes a negative/adversarial arm.

### List 3 — doc-comment preconditions

No new public function relies on a caller-only precondition:

- `summarizePluginMcp` accepts empty/no-MCP arrays and returns `null`.
- `summarizeSelectedMcpInstalls` accepts empty/unknown selected names and ignores them consistently with existing `deriveDiff`; no wrong output requiring a runtime refusal is possible.
- `mcp_server_transports` documents producer semantics, not a caller precondition; both production constructors are fenced.
- `CustomizeDrawerApply.acceptMcp` is re-gated against current non-null scope at emission, so correctness does not depend on a caller remembering a precondition.

### List 4 — write targets and output classification

| Slice | Output | Class |
|---|---|---|
| S1/S2 | catalog/Tauri binding payload | data; returned through IPC, not stdout |
| S2 | generated `bindings.ts` | build artifact/data |
| S4–S7 | project agent files | user-requested data; existing atomic/platform write path |
| S6/S7 | visible disclosure, status, result banners | user-facing data |
| all | test assertion failures | diagnostics via test harness stderr |

No new `println!`, `console.*`, or process-stream write is introduced.

### List 5 — tracker references

| Reference | Scope | Verified |
|---|---|---|
| `kiro-yr2f` | InstalledTab update MCP consent remains outside BrowseTab issue | yes; created during prototype and depends on kiro-2cu2 |

No other work is deferred. Every design claim is covered:

| Design claim | Slice |
|---|---|
| C1 | S1 |
| C2 | S1 |
| C3 | S2 |
| C4 | S3 |
| C5 | S5 + S6 |
| C6 | S6 |
| C7 | S7 |
| C8 | S4 + S7 |
| C9 | S5 + S6 |
| C10 | S3 |

### Hard-gate checklist

- [x] Every slice has claim, oracle, stress fixture, smallest change, loop budget, wall budget, files, and verification.
- [x] Every new loop has asymptotic cost, production scale, and bound.
- [x] Every slice has an adversarial fixture.
- [x] Claim coverage matches C1–C10.
- [x] Every tracker reference resolves.
- [x] No slice touches more than two files.

Ready for checkpointed build.

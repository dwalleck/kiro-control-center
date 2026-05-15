# Review-feedback decisions — second pass (35 commits + working tree vs origin/main)

**Inputs:** Five-agent PR review run on 2026-05-15 covering the cumulative branch state (35 commits + ~700 uncommitted lines, ~6,400 LOC delta). Five specialist reviewers ran in parallel: code-reviewer, pr-test-analyzer, silent-failure-hunter, type-design-analyzer, comment-analyzer.

**Scope:** Second pass. The first 24 commits of the branch were reviewed and decided on 2026-05-14 (see `2026-05-14-list-plugin-catalog-review.md`); the accept/modify items from that pass shipped between commits `5fa3f28..79c779b`. This pass covers the cumulative state (the original 24 + 11 follow-on commits + uncommitted work) since the second feature stream (kiro-zx73 per-item steering/agents) and the working-tree refactor merit re-review.

**Method:** `gilfoyle:assessing-review-feedback`. Per-finding: (1) categorize, (2) verify the bug claim by inspection, (3) cross-check against `2026-05-14` log and rivets backlog before treating as new, (4) evaluate the proposed fix, (5) decide. Defer decisions name a rivets ID — no silent drops.

**Outcome summary:** 8 accept · 0 modify · 18 defer (6 new rivets filed, 7 reference prior decisions, 5 fold into existing rivets) · 4 reject (3 over-flagged severity downgrades + 1 wrong-claim). Distribution skews toward defer because the cumulative branch already shipped one round of fixes; remaining items are net-new deferrable work or duplicates of already-tracked tickets.

**Reviewer-noise calibration:** The skill warns that "two reviewers agreed, so it must be right" is a red flag. This pass produced one *three-reviewer convergence* (string-match remediation, `--force to transfer`) — verified, and it turned out to be a duplicate of prior I4, with structural fix already tracked to kiro-xzrk. Convergence ≠ correctness, but it did flag a known-real concern with an already-named follow-up.

---

## Decision table

| # | Finding (one line) | Reviewer | Category | Verified? | Decision | Note |
|---|---|---|---|---|---|---|
| N1 | `applyDrawerDiff` parallel install batches overwrite `installError` non-deterministically | silent-failure | Bug | Yes — `BrowseTab.svelte:752-770` writes `installError = ...` inside three concurrent `Promise.all` legs; if two fail, second wins. | **Reject (defer)** — kiro-ti4x | Reviewer rated Critical; downgraded to Important (user sees *some* error, not silent failure). Defer because the fix wants to coordinate with the kiro-k5hj batch-helper extraction. |
| N2 | `surface_unmatched_agent_names` silently drops `Names` entries that fail `AgentName::new` | silent-failure | Bug | Yes — `service/mod.rs:3169-3177` warns + early-returns from `push_miss` without recording. | **Reject (defer)** — duplicate of prior I6 → kiro-bury | Same finding as the 2026-05-14 pass; already tracked. No action needed beyond the existing kiro-bury scope. |
| N3 | Install paths skip per-name path-traversal validation that remove paths enforce | silent-failure | Bug | Yes — `commands/steering.rs:111-143`, `commands/agents.rs:119-149` lack per-name validation; their remove counterparts at `:160-175` and `:166-182` validate. But: names go to `InstallFilter::Names` as string-equality match keys, never as path joins; today safe. | **Reject (defer)** — kiro-kvuh | Reviewer rated Critical; downgraded to low-priority defense-in-depth (P4). Install never touches the names as paths today — a `../etc/passwd` echoes back as `RequestedButNotFound`. The asymmetry is real but the security implication is zero until a future refactor adds path-join. |
| N4 | `InstallFilter` doc-comment claims skills-only (`service/mod.rs:103-108`) | comment-analyzer | Bug (rotten doc on public API) | Yes — comment names skills exclusively; the enum drives `install_skills`, `install_plugin_steering`, `install_plugin_agents` (lines 1377, 1692, 1815) each with a different join key. | **Accept** | Rewrite doc to enumerate the join key per category. Same "drift comment lands in same PR" rationale as prior I10-partial. |
| N5 | Comment claims drawer onApply receives skill-only diff (`BrowseTab.svelte:1607-1612`) | comment-analyzer | Bug (rotten doc) | Yes — kiro-zx73 has shipped; `CustomizeDrawerDiff` and `applyDrawerDiff` both handle `{skills, steering, agents}` end-to-end. The "Option A — kiro-zx73 widens to per-item steering/agents" framing reads as forward-work that's already done. | **Accept** | Replace with current-behavior text: "The drawer's onApply receives a per-category diff (skills + steering + agents). applyDrawerDiff fans out to one batch install + one remove loop per category." |
| N6 | Two adjacent comment blocks attach to wrong functions (`BrowseTab.svelte:955-994`) | comment-analyzer | Bug (readability) | Yes — lines 955-958 describe `formatFailedAgentForBanner` (defined line 973); 959-966 describe `basenameOf` (defined line 968). Order is preamble-A, preamble-B, function-B, function-A, with no blank line separating preambles. | **Accept** | Reorder so each comment is immediately above its function. Smallest possible fix. |
| N7 | Drawer post-apply refresh failure overwrites the success/failure summary | silent-failure | Bug | Yes — `BrowseTab.svelte:945-951` clobbers `installError` instead of appending. | **Reject (defer)** — kiro-ts3u | New ticket. Fix is small (use a separate `installWarning` slot or append), but the drawer summary composition is also pending extraction for kiro-k5hj; defer to coordinate. |
| N8 | Failure-summary lacks category labels — same-named items across skill/steering/agent render identically | silent-failure | Bug (UX) | Yes — `BrowseTab.svelte:893-901` concatenates `skillsFailed/steeringFailed/agentsFailed` without category prefix. A plugin shipping skill `rules` + steering `rules` + agent `rules` that all fail reads as `Failed: rules, rules, rules`. | **Accept** | Tag each entry with a `category` field at accumulation time and render `"&lt;category&gt;:&lt;name&gt;"` in the summary. Small fix; goes with N4-N6 in the same drift-in-same-PR commit. |
| N9 | Non-UTF-8 steering filename silent skip; log severity asymmetry catalog vs install | silent-failure | Bug | Yes — `mod.rs:1730-1733` continues without log; `browse.rs` catalog path emits `debug!`. | **Reject** — matches prior I7 disposition | The 2026-05-14 pass already decided to keep the defensive `debug!` log because the upstream discovery filter makes the branch unreachable in practice. No change in that calculus. |
| N10 | `failureMentionsOwnership` substring-matches `"--force to transfer"` (3-reviewer convergence) | type-design + silent-failure + pr-test-analyzer | Bug (fragility) | Yes — `BrowseTab.svelte:179-216` substring-matches Display output of `SteeringError::PathOwnedByOtherPlugin`. | **Reject (defer)** — duplicate of prior I4 → kiro-xzrk | Prior pass fixed the misleading comment; structural fix (typed `kind: "ownership_conflict"` discriminator) tracked to kiro-xzrk. Triple-reviewer convergence confirmed observable, did not change the disposition. |
| N11 | `InstallFilter::Names(&[T])` and `InstallFilter::SingleName(T)` are gratuitously duplicated | type-design | Design | Yes — `mod.rs:109-113`; rstest at `mod.rs:3299-3317` *proves* equivalence; every classifier has 3 arms when 2 would do. | **Reject (defer)** — kiro-xbet | Public API change requires coordinated bindings regen + classifier sweep. New ticket P3. |
| N12 | `print_agent_failure` has no CLI-side test (5-way match over `#[non_exhaustive]`) | pr-test-analyzer | Coverage | Yes — `install.rs:390-449` extracted in commit `119eba9` with no `#[cfg(test)]`. Vitest covers the analog (`format.test.ts:377`); a CLI regression to `{:?}` rendering stays green. | **Reject (defer)** — kiro-bkqu | **Reassessed during application.** Decision log originally said "Accept. Add print_install_agents_* test cases" but implementation reveals install.rs has no test module at all and `print_agent_failure` writes directly to `eprintln!`. Adding tests requires a structural refactor (take `&mut impl Write`), which exceeds the drift-cleanup commit's scope. Per the skill's "don't ship a fix you no longer believe in" rule, deferring to its own ticket. |
| N13 | `install_agents_impl` per-name filter has no native-format (`kiro-cli` dialect) coverage | pr-test-analyzer | Coverage | Yes — `commands/agents.rs:609-740` tests use translated markdown agents; the native path is only exercised via `install_plugin_agents_impl` (whole-plugin, `InstallFilter::All`). | **Accept** | One test `install_agents_impl_native_path_filters_by_parsed_json_name` mirroring `write_native_plugin` at line 227. |
| N14 | `applyDrawerDiff` warning-banner composition trapped in `.svelte` (per CLAUDE.md no-component-test rule) | pr-test-analyzer | Coverage | Yes — composition lives entirely in `BrowseTab.svelte:826-918`. The bug class C2 fixed (warnings dropping) is exactly the surface this would prevent regression on. | **Reject (defer)** — folded into kiro-k5hj | The extraction (`composeApplyBanner` → `.ts`) is structurally identical to the kiro-k5hj batch-helper work. Updating kiro-k5hj's scope rather than filing separately. |
| N15 | `PluginCard.svelte` three-state visual aggregation untested | pr-test-analyzer | Coverage | Yes — `counts` + `effectiveInstalled` + `stripeClass` derivation at `PluginCard.svelte:64-91`; non-obvious reconciliation rule (`installed && counts.state === "installed"` vs `||`). | **Reject (defer)** — folded into kiro-k5hj | Same fix pattern (extract pure derivation to `.ts`, vitest the truth table). Same rationale as N14. |
| N16 | `AgentParseSkip` and `SkippedItem::AgentParse` are the same 3 fields, hand-projected | type-design | Design | Yes — `browse.rs:113-118` vs `:273-279`; `assemble_catalog_entry` at `:484-493` destructures one into the other. Maintenance drift hazard. | **Reject (defer)** — kiro-twxs | Public API change; not blocking. New ticket P3. |
| N17 | `SkippedItem::AgentParse.reason: String` collapses typed failure at core boundary (skills got structured `SkippedSkillReason`) | type-design | Design | Yes — skills path uses `SkippedSkillReason::{ReadFailed, FrontmatterInvalid, DuplicateName}`; agents path emits `error_full_chain(&e)` to a `String`. | **Reject (defer)** — kiro-jmgb | New ticket; coupled to kiro-deph (RequestedButNotFound name-type discipline). |
| N18 | `PluginCatalogEntry` / `SteeringItemInfo` / `AgentItemInfo` are anemic public structs (ownership invariant lives as a duplicated `is_some_and` block at 4 sites) | type-design | Design | Yes — `browse.rs:241-251, 44-55, 65-73`. The `installed: true` IFF `(plugin, marketplace)` ownership match invariant lives in `list_*_with_manifest` helper bodies. | **Reject (defer)** — folded into kiro-i3ll | kiro-i3ll already tracks `#[non_exhaustive]` on these three types; broadening to "harden via constructors + helper" fits naturally. Updating kiro-i3ll's scope. |
| N19 | `InstallFilter::Names(&[])` is a representable silent-no-op state | type-design | Design | Yes — same surface as prior C4. | **Reject** — duplicate of prior C4 (already shipped) | The `reject_empty_names` IPC-boundary check at `commands/mod.rs:35-66` already addresses this. Reviewer rediscovered a solved problem. |
| N20 | Dead `.toString()` on string-typed wire fields (`BrowseTab.svelte:822, 983`) | code-reviewer | Polish | Yes — `FailedSteeringFile_Serialize.source` and `FailedAgent.source_path` are typed `string` per `bindings.ts:658-661, 487-490`. The `.toString()` is leftover from the pre-FFI-string PathBuf era. | **Accept** | Drop both `.toString()` calls. Goes with N4-N6, N8 in the drift-cleanup commit. |
| N21 | `test_clippy_format.exe` (112KB) + `test_clippy_format.pdb` (1.3MB) at repo root, untracked + not gitignored | code-reviewer + pr-test-analyzer | Polish (hygiene) | Yes — visible in `git status` at the start of this session. May 11 timestamps suggest one-off local artifacts. | **Accept** | Delete both files. Optionally add `/test_*.exe` and `/test_*.pdb` to `.gitignore` to prevent recurrence. |
| N22 | `Kiro Control Center Design System/` folder at repo root, untracked, duplicates real component names | code-reviewer | Polish (hygiene) | Yes — folder contains `BrowseTab.svelte`, `CustomizeDrawer.svelte`, `PluginCard.svelte` etc. that shadow `src/lib/components/` names. `git add -A` would risk clobbering production components on careless rebase. | **Accept** | Move folder out of repo (to a sibling directory) OR explicitly gitignore it. User picks based on workflow. |
| N23 | Comment rot: stale `Slice N` / `Sn` / `kiro-zx73 slice An` phase markers throughout | comment-analyzer | Polish | Yes — 24 individual sub-findings; spot-checked `service/browse.rs:968, 862, 911, 143`, etc. All slices have shipped; the durable references (test names, function names) are already present. | **Reject (defer)** — duplicate of prior I10 → kiro-pgel | kiro-pgel already covers this sweep; the second-pass review rediscovered the same backlog item. The prior pass also committed to fixing two outright-drift cases in this same PR; that's what N4 / N5 / N6 are. |
| N24 | Stale `used to vanish into warn!` / `restored in slice 2 follow-up` archaeology comments | comment-analyzer | Polish | Yes — `install.rs:526-529`, `BrowseTab.svelte:365, 392`, `CustomizeDrawer.svelte:18-22, 229-235`. Describe a state that no longer exists. | **Reject (defer)** — folded into kiro-pgel | Same sweep as N23. |
| N25 | `format.test.ts:25-32` references `bindings.ts` line numbers that are auto-regenerated (already stale) | comment-analyzer | Polish | Yes — every cited line number is already wrong at HEAD. The auto-regen workflow guarantees this rots. | **Accept (small)** | Drop line-number suffixes; reference type names only. Smaller than the kiro-pgel sweep; lands in the drift-cleanup commit alongside N4-N6, N8, N20. |
| N26 | `format.ts:17-21` "currently-unreachable variants" framing for `SkippedReason` | comment-analyzer | Polish | Yes — but the framing is technically still accurate: `SkippedPlugin` banners consume the pre-rendered `sp.reason` string, not the structured `SkippedReason`, so `formatSkippedReason`'s only consumer remains `skillCountTitle`. | **Reject** | Comment is correct today; "currently-unreachable" qualifier is appropriate. No action. |
| N27 | `if (steeringData.warnings)` / `if (agentsData.warnings)` always truthy (T[] is never undefined per bindings) | code-reviewer | Polish | Yes — `bindings.ts:715, 763, 925` type these as `SteeringWarning[]` / `InstallWarning[]`. | **Accept (small)** | Drop the guards or change to `.length > 0` for short-circuit semantics. Lands in drift-cleanup commit. |
| N28 | `error_full_chain(&e)` not used in `validate_relative_path` mapper (`commands/steering.rs:165, agents.rs:117`) | silent-failure | Polish | Yes — both use `{e}` (Display only). Functionally safe today because `ValidationError` is internal-typed and has no `#[source]` chain. Prophylactic. | **Reject** | The convention is for *external* errors and FFI string fields; `ValidationError`'s Display is the FFI contract and has no chain to lose. Add later if `ValidationError` ever gains a `#[source]`. |
| N29 | `forceInstall` toggle is page-global but used per-plugin in drawer Apply (`BrowseTab.svelte:116, 779-799`) | code-reviewer | Design | Yes — same `$state` used by whole-plugin install button. Stale-toggle hazard between flows. | **Reject (defer)** — kiro-2cu2-adjacent | Similar UX-scoping question to MCP opt-in (kiro-2cu2). File separately if user feedback surfaces; per prior S7 disposition, defer until a real user-confusion report. |
| N30 | `availablePlugins` discards `view.skipped` so broken-manifest plugins have no card placeholder | code-reviewer | Design | Yes — `BrowseTab.svelte:242-252` filters skipped plugins out of the grid. Skipped banners exist but no per-plugin card. | **Reject** | Out of scope for this PR — banner stack already surfaces the information; a "broken plugin" card is a UX improvement, not a correctness fix. If user feedback surfaces, file as a UX issue. |

---

## Decisions cross-referenced to apply order

Order intent: smallest correctness-or-readability fixes first, hygiene next. Each accept is its own commit unless explicitly grouped.

1. **N4 + N5 + N6 + N8 + N20 + N25 + N27** — Drift-in-same-PR cleanup. One commit:
   - `service/mod.rs:103-108` — rewrite `InstallFilter` doc
   - `BrowseTab.svelte:1607-1612` — replace the "skill-only diff" comment
   - `BrowseTab.svelte:955-994` — reorder comment blocks so each preamble adjoins its function
   - `BrowseTab.svelte:893-901` — tag failure entries with category in the summary
   - `BrowseTab.svelte:822, 983` — drop dead `.toString()`
   - `format.test.ts:25-32` — drop bindings.ts line-number suffixes
   - `BrowseTab.svelte:826, 836` — drop always-truthy `if (x.warnings)` guards
2. **N13** — Test addition. One commit:
   - `crates/kiro-control-center/src-tauri/src/commands/agents.rs` — `install_agents_impl_native_path_filters_by_parsed_json_name`
   - N12 was reassessed to defer during application (see N12's row); the file `install.rs` requires a `print_agent_failure` testability refactor first, tracked in kiro-bkqu.
3. **N21 + N22** — Repo hygiene. One commit:
   - Delete `test_clippy_format.exe`, `test_clippy_format.pdb`
   - Move or gitignore `Kiro Control Center Design System/`
   - Add `/test_*.exe`, `/test_*.pdb` to `.gitignore` (preventive)

After 3 commits the accept slate is complete. The remaining 22 decisions are deferrals (18) or rejects (4) — no further code action required in this PR. Updating kiro-pgel / kiro-k5hj / kiro-i3ll to absorb folded findings is a tracker-edit step, not a code commit.

---

## Tracker updates (separate from code commits)

- **kiro-pgel** — absorbs N23, N24 (already in scope, no description edit needed; the 24 sub-findings from comment-analyzer are the same backlog the rivets description anticipates).
- **kiro-k5hj** — extend description to include N14 (composeApplyBanner extraction) and N15 (derivePluginCardState extraction). Both share the "extract pure helper from `.svelte` to enable vitest" pattern with the existing S4 batch-helper item.
- **kiro-i3ll** — extend description to include N18 (anemic-struct hardening via constructors + ownership-installed helper). Same three types (`PluginCatalogEntry`, `SteeringItemInfo`, `AgentItemInfo`) already in scope.

New rivets filed in this pass:
- **kiro-ti4x** — applyDrawerDiff parallel installError overwrite (N1)
- **kiro-ts3u** — Drawer post-apply refresh failure overwrites summary (N7)
- **kiro-kvuh** — Install commands lack per-name path-traversal validation (N3, P4 defense-in-depth)
- **kiro-xbet** — InstallFilter::Names vs SingleName gratuitous duplication (N11)
- **kiro-twxs** — Dedup AgentParseSkip / SkippedItem::AgentParse projection (N16)
- **kiro-jmgb** — SkippedAgentReason structured variant (N17, coupled to kiro-deph)
- **kiro-bkqu** — Make `print_agent_failure` testable via writer-injection (N12, reassessed mid-application)

---

## Verification appendix — what was checked

Each "Verified? Yes" claim above is anchored to one of:

- A `Read` of the cited file/line range from the reviewer's report (most findings — N1, N3, N4, N5, N6, N7, N9, N16, N18, N20, N21, N22, N25, N26, N27).
- A `Grep` for the specific symbol across the codebase (used to confirm `surface_unmatched_*_names` is agent-specific for N2; to confirm `InstallFilter` consumers for N4; to spot-check phase markers for N23-24).
- A cross-reference against the prior decision log `2026-05-14-list-plugin-catalog-review.md` (used for N2, N9, N10, N19, N23, N24).
- A cross-reference against the rivets open backlog (used to detect kiro-bury, kiro-pgel, kiro-pr98, kiro-2cu2, kiro-xzrk, kiro-deph, kiro-k5hj, kiro-i3ll as existing tickets).

No finding was applied — accepted, modified, or otherwise — without the verification step. Three reviewer-rated Criticals (N1, N2, N3) were downgraded after verification revealed the bug was either (a) lossy not silent, (b) already tracked, or (c) defense-in-depth not exploitable. One Critical was correctly rated and accepted (the N4-N6 doc-rot trio, treated as a single drift-cleanup unit). The skill's "two reviewers agreed, so it must be right" red flag fired once (N10 triple-convergence) and verification confirmed the finding was real but already-tracked, not a new problem.

Reject (defer) decisions where the bug exists but the fix is scope-creep are NOT treated as "the finding was wrong" — they are tracked as "the finding is right but lands elsewhere," with the rivets entries above as the durable record.

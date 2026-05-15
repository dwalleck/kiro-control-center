# Review-feedback decisions — list_plugin_catalog branch (origin/main..HEAD, 24 commits)

**Inputs:** Multi-agent PR review run on 2026-05-14 covering the catalog-redesign branch (24 commits, ~5,700 LOC). Six specialist reviewers ran in parallel: code-reviewer, pr-test-analyzer, silent-failure-hunter, type-design-analyzer, comment-analyzer, code-simplifier.

**Scope:** All 24 findings. C1 (commit-scope) was applied immediately via `git filter-branch` and is omitted from this log. The remaining 23 findings were each verified by reading the cited code before deciding accept / modify / reject.

**Method:** `gilfoyle:assessing-review-feedback`. Per-finding: (1) categorize, (2) verify the bug claim by inspection, (3) evaluate the proposed fix on its own merits, (4) decide. Defer decisions name a rivets ID — no silent drops.

**Outcome summary:** 9 accept · 4 modify · 10 defer (filed as 8 rivets issues, two paired) · 1 reject (rationale documented). A healthy review distribution per the skill's expectation that 2–3 of 6 findings come back as reject/modify; this one trends toward defer because the 24-commit batch already shipped substantial public-API surface and many follow-ups are best landed in their own scope.

---

## Decision table

| # | Finding (one line) | Reviewer | Category | Verified? | Decision | Note |
|---|---|---|---|---|---|---|
| C2 | Drawer install batches discard `r.data.warnings` (Steering+Agent paths) | silent-failure + code-reviewer + pr-test-analyzer | Bug | Yes — `BrowseTab.svelte:771-836` reads only `installed`/`failed`/`skipped`. `InstallSteeringResult.warnings` / `InstallAgentsResult.warnings` exist on the wire (`bindings.ts:715, :763, :918, :925`). | **Accept** | Surface warnings in the drawer summary banner mirroring whole-plugin path's `formatSteeringWarning` usage. |
| C3 | Drawer hardcodes `acceptMcp: false` | silent-failure | Bug (partially wrong) | Yes — but BOTH paths hardcode false (`BrowseTab.svelte:818, :1125`); reviewer's premise that whole-plugin threads from state was incorrect. | **Reject (defer)** — kiro-2cu2 | The drawer's `false` is *consistent* with the whole-plugin path. Real issue is the UI surface gap (no MCP opt-in toggle anywhere). Tracked separately so a UX decision drives the design. |
| C4 | `InstallFilter::Names(&[])` silent no-op | type-design | Bug | Yes — `filter_matches` (`service/mod.rs:3142`) returns false for every name in an empty slice; `surface_unmatched_agent_names` iterates the (empty) requested set, surfaces nothing. | **Modify** | Reject empty `Names` at the IPC adapter boundary with `CommandError::new("empty names list", ErrorType::Validation)`. Cheaper + better-localized than introducing a `NonEmpty` newtype across the public Rust API. |
| C5 | `formatSkippedItemsForPlugin` has zero vitest coverage; missing value-position `_SKIPPED_ITEM_KINDS` const-assert | type-design + pr-test-analyzer | Bug + Style | Yes — `format.test.ts:226-228` only protects `_FAILED_AGENT_KINDS`; no describe block for `formatSkippedItemsForPlugin` exists. | **Accept** | Add vitest cases (per-arm + overflow + mixed-buckets + assertNever). Add `_SKIPPED_ITEM_KINDS as const satisfies` + `_AssertSkippedItemKindExhaustive` + value-position `const _assert: T = true` mirroring the FailedAgent precedent. |
| I1 | CLI `print_agent_outcome` falls back to `Debug` rendering for `RequestedButNotFound` | code-reviewer | Bug | Yes — `install.rs:433` `{other:?}` (Debug, not Display). The inline comment claiming "thiserror Display impls" is wrong: `{:?}` does not invoke thiserror's Display. | **Accept** | Add an explicit match arm for `FailedAgent::RequestedButNotFound { name, plugin }` rendering `"agent '{name}' not found in plugin '{plugin}'"` to match `format.ts:215`. |
| I2 | Steering/agent catalogs lack per-name dedup across scan paths (skill catalog dedupes) | code-reviewer | Bug | Yes — `service/browse.rs:1432-1456` shows skill `seen_names` dedup with `SkippedSkillReason::DuplicateName`; `:894-927` (steering) and `:983-1041` (agents) have no equivalent. | **Modify (deferred)** — kiro-0pbb | Adding `SteeringDuplicateName` + `AgentDuplicateName` `SkippedItem` variants is a public-API change requiring bindings regen, format.ts renderer, and C12 fence updates. Scoped to its own PR. |
| I3 | `list_plugin_catalog_for_marketplace_impl` calls `list_plugin_entries` twice per request | code-reviewer | Bug | Yes — `commands/browse.rs:401-423` calls `svc.list_plugin_catalog()` then `svc.list_plugin_entries()` again just to build `source_types: HashMap`. The internal `list_plugin_catalog` already iterates the same registry. | **Accept** | Add `source: PluginSource` to `PluginCatalogEntry` (core type). Wrapper maps `source` → `SourceType` in a single pass over `view.plugins`. Eliminates the second scan AND S9's defensive `unwrap_or(SourceType::Relative)` fallback. |
| I4 | `failureMentionsOwnership` uses substring match with comment defending via misapplied CLAUDE.md rule | silent-failure | Bug | Yes — `BrowseTab.svelte:163-166` cites "CLAUDE.md's structural-error rule" but CLAUDE.md actually says the opposite: prefer typed variants over `reason: String` sentinels when branching on the semantic. | **Modify** | Fix the misleading comment now. Structural fix (adding `kind: "ownership_conflict"` discriminator on `FailedSteeringFile` + `FailedSkill`) is deferred to kiro-xzrk (steering side already in flight). |
| I5 | RequestedButNotFound name-type drift: `String` (skill), `PathBuf+String` (steering), `AgentName+PluginName` (agent) | type-design | Bug (drift) | Yes — `mod.rs:301` (skill raw), `steering/types.rs:103` (PathBuf+String), `mod.rs:717` (newtyped). | **Reject (defer)** — kiro-deph | Three-module public API change. Right answer is the harmonization, but scope is design-doc-sized and should follow kiro-xzrk's restructure. |
| I6 | `surface_unmatched_agent_names` silently drops names that fail `AgentName::new` | silent-failure | Bug | Yes — `service/mod.rs:3169-3177` warns and returns without recording. The defending comment claims "the install would have failed at the catalog read anyway" but a CLI caller bypasses the catalog entirely; the silent drop IS reachable. | **Reject (defer)** — kiro-bury | Requires new `FailedAgent::InvalidName { raw, reason }` variant — public API change. Comment fix lands as part of the kiro-bury issue's scope (don't fix only the comment, since that creates a misleading-defense-removed-but-bug-still-there state). |
| I7 | Non-UTF-8 steering filename silent skip; no per-item idempotent reinstall test | silent-failure + pr-test-analyzer | Bug + Coverage | Yes (test gap) — `commands/steering.rs` tests don't cover the `install_steering_files_impl` second-call idempotent path. Yes (silent skip) but the path is documented unreachable (discovery already filters). | **Modify** | Add per-item idempotent reinstall test mirroring `install_skills_impl_force_mode_overwrites_existing_install`. Leave the non-UTF-8 defensive skip with its current `debug!` log — the upstream discovery filter makes the branch unreachable in practice, and pushing a synthetic failure for a path that never fires is busywork. |
| I8 | Drawer diff math (`deriveDiff`, `deriveSectionState`, `pluralize`) trapped inside `.svelte` | pr-test-analyzer | Bug (testability) | Yes — `CustomizeDrawer.svelte` contains these helpers inline; no vitest coverage possible per CLAUDE.md's no-svelte-testing rule. | **Accept** | Extract to `crates/kiro-control-center/src/lib/drawer-diff.ts` + vitest. Pure-function set-difference math is exactly the case CLAUDE.md prescribes the helper-extraction pattern for. |
| I9 | `InstallFilter` accepts raw `&str` / `&[String]` instead of validation-newtype inputs | type-design | Style | Yes — same enum surface as C4. | **Accept (merged into C4)** | C4's boundary-rejection fix handles this by routing through `AgentName::new` / `RelativePath::new` at the IPC adapter, which is where untrusted input enters. Wrapping the enum itself in newtypes would change the public Rust API without practical security benefit. |
| I10 | Comment rot: pervasive slice/rivets/PR/user references across 10 files | comment-analyzer | Style | Yes — spot-checked `browse.rs:443-445`, `BrowseTab.svelte:204-207`; both narrate scaffolding already removed in this same PR. | **Modify (deferred)** — kiro-pgel | Fix the two outright-drift cases now (within scope: same PR removed the scaffolding the comments narrate). Defer the full 10-file sweep to its own commit so the diff is reviewable. |
| S1 | `InstallFilter::SingleName` arm untested at service layer | pr-test-analyzer | Coverage | Yes — `service/mod.rs:3145` arm exists but tests use only `Names(&[...])`. | **Accept** | Add one `rstest` parameterized over `Names(&[one])` vs `SingleName(one)` to lock the two-arm equivalence. |
| S2 | `formatFailedAgent` missing test for `requested_but_not_found` arm | pr-test-analyzer | Coverage | Yes — `format.test.ts:311-383` covers `agent`/`unparseable_agent`/`companion_bundle`/`assertNever`, not the new variant. | **Accept** | One vitest case mirroring `formatFailedSkill`'s `requested_but_not_found` test (`format.test.ts:288-297`). |
| S3 | Extract `installed_by_this_plugin(meta, plugin, marketplace)` helper (4 call sites) | code-simplifier | Polish | Yes — 4 occurrences of the same closure shape (`browse.rs:921, 993, 1027, 1467`). | **Reject (defer)** — kiro-k5hj | Behavior-preserving refactor; PR is already large. |
| S4 | Collapse 6 install/remove batch helpers in `applyDrawerDiff` | code-simplifier | Polish | Yes — `BrowseTab.svelte:746-894` has parallel install/remove shapes. | **Reject (defer)** — kiro-k5hj | Same rationale as S3. |
| S5 | Replace 3 identical tracking-file load blocks in catalog wrapper | code-simplifier | Polish | Yes — `commands/browse.rs:376-399`. | **Accept (merged into I3)** | The I3 fix eliminates the second `list_plugin_entries` scan AND localizes the three tracking-loads into a single pass, obviating the standalone helper extraction. |
| S6 | `#[non_exhaustive]` missing on `PluginCatalogEntry` / `SteeringItemInfo` / `AgentItemInfo` | type-design | Polish | Yes — `service/browse.rs:155, 178, 128`. `SkippedItem` next door has it; inconsistency is the smell. | **Reject (defer)** — kiro-i3ll | These are output (read) types; the practical risk of a struct-literal-construction caller in another crate is low. Filed for consistency, not urgency. |
| S7 | Companion bundle silently skipped on `Names(_)` filter | silent-failure | Design | Yes — `service/mod.rs:2026` requires `InstallFilter::All` for companion install. | **Reject** | Intentional behavior per the inline comment ("the user is curating individual agents, not asking for the bundle"). The drawer-vs-whole-plugin behavioral difference is the correct product semantic — bundles are plugin-scoped, items are not. If user feedback proves this is confusing, file a UX issue then. |
| S8 | Native-agent `description: None` hardcoded; UI permanently description-less | code-reviewer | Bug (UX) | Yes — `service/browse.rs:1018` with comment "until the parser is widened." | **Reject (defer)** — kiro-wks5 | Parser-widening work; orthogonal to catalog redesign. |
| S9 | Source-type fallback `unwrap_or(SourceType::Relative)` lies to UI on registry desync | silent-failure | Design | Yes — `commands/browse.rs:429-432`. | **Accept (merged into I3)** | The I3 fix moves `source` into `PluginCatalogEntry` itself, eliminating the lookup and the fallback. No standalone change needed. |

---

## Decisions cross-referenced to apply order

Order intent: foundational typed changes first, consumers next, tests + polish last. Each accept/modify is its own commit.

1. **C5 + S2** — `formatSkippedItemsForPlugin` vitest + value-position `_SKIPPED_ITEM_KINDS` guard + `formatFailedAgent` `requested_but_not_found` test. (Tests-first; foundational for the wire-format renderer.)
2. **I1** — CLI `print_agent_outcome` arm for `RequestedButNotFound`. (Small, defensive.)
3. **S1** — `rstest` parameterized over `InstallFilter::Names(&[one])` vs `SingleName(one)` in service layer.
4. **C4 + I9** — IPC-boundary validation rejecting empty `Names` slice (`CommandError::Validation`) in the three new commands (`install_steering_files`, `install_agents`, `list_plugin_catalog_for_marketplace` where applicable).
5. **I3 + S5 + S9** — Add `source: PluginSource` to `PluginCatalogEntry`; rewrite `list_plugin_catalog_for_marketplace_impl` to single-pass. Eliminates second `list_plugin_entries` call, three tracking-load blocks become one, lying-fallback `unwrap_or(SourceType::Relative)` removed.
6. **C2** — Drawer `installSteeringBatch` + `installAgentsBatch` surface `r.data.warnings` in the apply summary alongside failures.
7. **I7** — Per-item idempotent reinstall `rstest` for `install_steering_files_impl` + `install_agents_impl`.
8. **I8** — Extract `deriveDiff` / `deriveSectionState` / `pluralize` to `drawer-diff.ts` + vitest.
9. **I4** — Fix the misleading `failureMentionsOwnership` comment (point at kiro-xzrk; remove the misattributed CLAUDE.md citation).
10. **I10 partial** — Fix the two outright-drift comments (`browse.rs:443-445` + `BrowseTab.svelte:204-207`). Full sweep deferred to kiro-pgel.

After 10 commits the PR's accept/modify slate is complete. The remaining 10 deferrals are tracked in 8 rivets issues (some bundle pairs):
- kiro-2cu2 (C3)
- kiro-0pbb (I2)
- kiro-deph (I5)
- kiro-bury (I6)
- kiro-pgel (I10 sweep)
- kiro-k5hj (S3 + S4)
- kiro-i3ll (S6)
- kiro-wks5 (S8)
- (S7 outright rejected, no rivets)

---

## Verification appendix — what was checked

Each "Verified? Yes" claim above is anchored to one of:

- A `Read` of the cited file/line range from the reviewer's report (most findings).
- A `Grep` for the specific symbol across the codebase (used for the `acceptMcp` audit on C3, the `warnings` field audit on C2, and the bindings export check on C5).
- A cross-reference against CLAUDE.md anchors (I4's "misapplied CLAUDE.md rule" claim required reading the rule itself, not just trusting the reviewer's framing).

No finding was applied — accepted, modified, or otherwise — without the verification step. Reject (defer) decisions where the bug exists but the fix is scope-creep are NOT treated as "the finding was wrong" — they are treated as "the finding is right but lands elsewhere," and the rivets entries above are the durable record.

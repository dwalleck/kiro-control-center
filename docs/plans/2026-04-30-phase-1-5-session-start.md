# Phase 1.5 — Session-Start Prompt

> Paste this verbatim into a fresh Claude Code session to start Phase 1.5 implementation. Self-contained briefing.

---

Context: I'm starting Phase 1.5 implementation of the plugin-first install architecture for the kiro-control-center desktop app. Phase 1 (PR #94) shipped 23 commits with an 8-reviewer aggregated review whose Critical convergent finding was the swap-arg footgun on `marketplace`/`plugin` strings across 7+ public APIs. Phase 1.5 closes that footgun with validated `MarketplaceName` / `PluginName` newtypes (A1) plus the missing `marketplace` field on `InstallPluginResult` (A4).

Worktree: /home/dwalleck/repos/kiro-marketplace-cli-phase-1-5
Branch: feat/phase-1-5-types (tracks origin/main, already at the post-#94 state, plus 4 design+plan+amendments commits)

Required reading, in order, before touching any code:

1. docs/plans/2026-04-30-phase-1-5-type-safety-design.md — architecture and user-locked decisions (newtypes are *Name not *Id, no Default derive, A2/A3 deferred, scope is strictly the install/remove cascade family).

2. docs/plans/2026-04-30-phase-1-5-type-safety-plan.md — 8 tasks. Tasks 3 and 5 are heavy (project.rs and service/mod.rs); Tasks 1, 2, 4, 6 are focused; Tasks 7-8 trail the migration through Tauri + CLI + bindings.ts + final sweep + open PR.

3. docs/plans/2026-04-30-phase-1-5-type-safety-plan-amendments.md — 5 amendments (P1.5-1 through P1.5-5) from the 5-gates plan-review pass. Two are real compile-error fixes (P1.5-1, P1.5-2 — both Task 6 step 1, comparison shape and PluginError variant fields), the rest are clarity. Fold them at the relevant task; don't skip.

4. docs/plans/2026-04-29-plugin-first-install-plan-amendments.md — Phase 1's 25 amendments (A-1 through A-25). NOT all relevant, but A-1/A-14/A-21 (associated-fn drift), A-12/A-24 (orphan recovery — A-24 was closed in PR #94), A-25 (tauri-specta skip_serializing_if rule) are useful precedents you'll cross-reference.

Conventions you must follow (from CLAUDE.md and Phase 1 precedent):

- Edition 2024, rust-version 1.85.0
- thiserror in libs, anyhow in bins
- Zero-tolerance in production: no .unwrap() / .expect() / let _ = ... discarding Result / #[allow(...)] (tests exempt)
- Validation newtypes flowing through Tauri bindings need #[cfg_attr(feature = "specta", derive(specta::Type))]
- Map external errors at the adapter boundary (no #[source] serde_json::Error etc. on pub types)
- Tauri commands split into thin wrapper + private fn <name>_impl that takes &MarketplaceService. Project-only reads (list_installed_plugins, remove_plugin, list_installed_skills, remove_skill) follow the no-_impl precedent — body inline.
- Pre-commit: cargo fmt --all --check, cargo test --workspace, cargo clippy --workspace --tests -- -D warnings, npm run check (in crates/kiro-control-center/), and TETHYS_BIN=/home/dwalleck/repos/rivets/target/release/tethys cargo xtask plan-lint

Use the LSP tool first when researching code structure (per memory feedback_lsp_first.md and A-8 in Phase 1's amendments). Grep is fallback. The LSP-first discipline caught P1.5-1 and P1.5-2 in the plan-review pass; it'll catch similar drift during execution.

Execution model — subagent-driven-development:

Use the superpowers:subagent-driven-development skill. Dispatch a fresh subagent per task, with the design + plan + amendments docs as required context. Read the diff yourself between tasks; optionally dispatch pr-review-toolkit:code-reviewer for a deeper pass after every 2-3 tasks (Phase 1 used this rhythm).

Don't redo plan-time review. The plan has 5 amendments; trust them and the compiler. If a finding does come up during implementation, capture it as P1.5-6+ in the amendments doc (audit trail) — but the goal is forward motion, not more review. Phase 1 captured A-24 and A-25 mid-execution this way.

Don't try to compile or run from /home/dwalleck/repos/kiro-marketplace-cli (main repo). Always work in /home/dwalleck/repos/kiro-marketplace-cli-phase-1-5. The worktree has its own target/ and node_modules/. Memory file feedback_review_worktree.md explains why.

When you dispatch review-agent subagents (later, when checking convergence with deeper review), pass the worktree absolute path explicitly and warn against reading the main repo — Phase 1 had a wrong-directory review that produced phantom dead-code findings.

Heavy lifts to anticipate:
- Task 3 (project.rs): 4 meta types + InstalledPluginInfo + KiroProject removal/install API + free helpers + ~30 test fixtures. Single coherent commit (or with as_str() shims for sub-commits). After Task 3 the workspace doesn't compile — Tasks 5-7 fix the ripple.
- Task 5 (service/mod.rs): AgentInstallContext + InstallPluginResult + A4 marketplace field + drop Default + MarketplaceService install API. ~12 test fixtures.

After Phase 1.5 is done (8 tasks), open a PR titled "feat: type-safety hardening — MarketplaceName/PluginName newtypes (Phase 1.5)" referencing PR #94 in the body. The plan's Task 8 step 7 has the full PR body template.

First action: cd to the worktree, read the three plan docs (design + plan + amendments) plus the relevant parts of Phase 1's amendments (A-8 LSP discipline, A-25 skip_serializing_if rule), then start Task 1 (define MarketplaceName + PluginName) via a fresh subagent.

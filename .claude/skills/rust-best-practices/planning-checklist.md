# Implementation Plan Quality Checklist

Reference this checklist before finalizing any implementation plan. Each item addresses a failure mode observed in real refactoring work.

## Platform Assumptions

- [ ] **Every platform behavior claim has a verification test.** If the plan says "`Path::is_symlink()` covers junctions on Windows" or "`XDG_DATA_HOME` works on macOS," there must be a `#[cfg(target)]` test that proves it. If you can't write the test (no CI runner for that platform), mark it as **UNVERIFIED ASSUMPTION** in the plan.
- [ ] **Cross-platform functions are tested on each platform they claim to support.** Not "it compiles on both" — a behavioral test that creates the thing and verifies detection/cleanup.
- [ ] **`std::path::Path` methods are checked for platform-specific behavior.** `is_absolute()`, `is_symlink()`, `canonicalize()` all behave differently across Unix/Windows. Don't assume Unix behavior is universal.

## Error Path Coverage

- [ ] **Every `match` arm in the plan's code has a corresponding test.** Enumerate them explicitly in the test task. If `clone_or_link` has GitHub/GitUrl/LocalPath branches, there are 3 tests minimum.
- [ ] **Every error return path has a test.** If `add()` can fail at clone, manifest read, validation, rename, or registration, each failure mode needs a test with a mock/stub that triggers it.
- [ ] **Error types preserve the original cause.** When mapping errors across boundaries, check: does the mapped error still tell the user *why* it failed? `Err(_e)` with an underscore prefix is a red flag — it means the original error is being discarded. Distinguish "not found" from "permission denied" from "I/O error."
- [ ] **Cleanup in error paths is structural, not manual.** If the plan's code has the same cleanup block copied 3+ times, it needs a `Drop` guard or scope helper. Manual cleanup is one forgotten error path away from a resource leak.

## Return Type Completeness

- [ ] **Return types express all meaningful outcome states.** If an operation can succeed in qualitatively different ways (linked vs. copied, partial success vs. full success), the return type should distinguish them. Don't collapse distinct outcomes into `Ok(())` and rely on logging to communicate the difference.
- [ ] **`io::Result<()>` is too coarse when there are 3+ outcomes.** Consider an enum: `Linked`/`Copied`/`Failed`, `FullSuccess`/`PartialSuccess`/`TotalFailure`.

## Behavioral Equivalence

- [ ] **Before rewriting a handler, document its observable behaviors.** What does it print for each case? What exit codes does it return? What side effects does it have? The new handler must match every one, or the plan must explicitly note the intentional change.
- [ ] **Empty/zero/none cases are explicitly tested.** "What happens when there are no marketplaces?" "What happens when the list is empty?" These are the cases that silently regress because the loop body works fine — it just never executes.
- [ ] **User-visible output is part of the spec.** If the old code printed "No marketplaces registered" and the new code prints nothing, that's a regression even if the return value is correct.

## Code Migration vs. Redesign

- [ ] **When moving code to a new abstraction, audit it for the new context.** A service method has different callers than a CLI handler. Error handling that was appropriate for one context may be wrong for the other. Don't blindly port — re-evaluate.
- [ ] **Check for patterns the old code got wrong.** If the code being moved had `let _ =` patterns, discarded errors, or duplicated logic, the plan should fix them during the move, not preserve them. The whole point of a refactor is improvement.
- [ ] **Document what the old code did wrong.** Explicitly list the defects being fixed so the implementer knows what to avoid reproducing.

## Frontend Code (Svelte / TypeScript)

- [ ] **Every async backend call has an error branch.** If the code has `if (result.status === "ok") { ... }`, there MUST be an `else` that surfaces the error to the user. Silent swallowing of backend errors is the frontend equivalent of `let _ =` in Rust.
- [ ] **`$effect` vs `onMount` is a deliberate choice.** Use `onMount` for one-shot initialization. Use `$effect` only when you need reactive re-execution. Document why in a comment. `$effect` with an async function that mutates store state is fragile — reads after `await` aren't tracked, but reads before it are.
- [ ] **Fields that are always the same value should not exist.** If `kiro_initialized` is always `true` and `skill_count` is always `0`, remove them. A perpetually-constant field misleads consumers into thinking it carries information.
- [ ] **Store state has no redundant fields.** If `projectPath` always equals `projectInfo.path`, keep one. Two sources of truth for the same value will diverge.
- [ ] **Duplicate input is guarded.** If the user can add the same scan root twice, check before adding. This applies to any user-facing "add to list" operation.

## Concurrency & State

- [ ] **Read-modify-write patterns have a concurrency note.** If code loads state from disk, mutates one field, and saves — what happens if two calls overlap? In Tauri, commands dispatch concurrently. Either use a Mutex, use file locking, or document the race as accepted.
- [ ] **Collection operations match their requirements.** `dedup_by` only removes *consecutive* duplicates — the list must be sorted by the dedup key first. `sort_by(name)` then `dedup_by(path)` is a bug.

## Test Design

- [ ] **Tests prove behavior, not that the test framework works.** "MockGitBackend records calls and the service constructs it" proves the mock works. "When clone fails, the service cleans up temp dirs and returns an error" proves the service is correct. Write the second kind.
- [ ] **For mock-based tests, the mock should be able to fail.** If the mock always succeeds, you've only tested the happy path. Create a `FailingMock` variant or add a failure-injection mechanism.
- [ ] **Enumerate branches before writing tests.** For each function in the plan, list every `if/else`, `match` arm, and early return. Each one is a test case. Put this enumeration in the plan explicitly.
- [ ] **Persistence roundtrip tests verify field preservation.** If `save_scan_roots` modifies one field, test that the OTHER fields survive the roundtrip. Read-modify-write bugs silently erase unrelated state.

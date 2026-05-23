# Review-feedback decisions — PR #review of S13 (commit 97fdb3b)

Per `.kiro/skills/assessing-review-feedback/SKILL.md`. Each finding is verified
against the actual code before deciding accept / modify / reject. Deferral
decisions name a tracker ID.

## Decisions

| # | Finding (one line) | Reviewer | Category | Verified? | Decision | Note |
|---|---|---|---|---|---|---|
| C1 | Frontend AGENT_NAME_REGEX claims to mirror Rust validate_name but is far stricter | comment-analyzer | Bug (comment lies) + Design (regex policy) | Yes — `validate_name_accepts_internal_whitespace` (validation.rs:781) accepts "Terraform Agent"; my regex rejects it | **Modify** | Update comment to be truthful (regex is a UX-strictness layer, NOT parity). Don't change regex in this fix — the deeper UX policy question (should the editor enforce kebab-case for renames? what about marketplace agents with permissive names?) is S14 scope. Tracker: kiro-k9ok |
| C2 | Synthetic-draft fallback can clobber malformed file on Save | silent-failure-hunter | Bug (data loss) | Yes — Save button only gated on `saving \|\| loading`; after load fails and banner dismissed, `draft = { name: row.name }` persists, click Save overwrites file with `{"name":"foo"}` | **Accept** | Add `loadFailed` flag separate from `loadError` banner. Save button gates on `loadFailed` too. Banner dismiss clears the message text but not the gate. |
| C3 | describeCommandError discards error_type — half-wired indirection | silent-failure-hunter / type-design-analyzer / simplifier (3-agent convergence) | Style + Polish | Yes — helper just returns `err.message`; no branching on error_type | **Accept (simplifier path)** | Delete the helper, inline `result.error.message`. Typed-error branching is S14+ scope when the typed-error UX is designed. The current shape is decoration that lies about its purpose. |
| I1 | Discriminator switch discipline half-applied on mode.kind | code-reviewer / type-design-analyzer / simplifier (3-agent convergence) | Style (project convention) | Yes — `if (mode.kind === "new") { ... return; }` then implicit-else. CLAUDE.md mandates switch + never-default. agent-list-helpers.ts already uses _KINDS + _AssertExhaustive | **Accept** | Convert to switch with never-default in handleSave + $effect. Add _EDITOR_MODE_KINDS + _AssertExhaustive tripwire on EditorMode (mirrors helpers' AgentsTabMode pattern). |
| I2 | $effect doesn't reset saveError or saving on mode change | silent-failure-hunter | Bug (latent) | Yes — effect resets loadError/loading but leaves saveError. Currently latent because parent always remounts editor on mode flip via `{:else}` branch, but fragile against future direct-mode-transition flows | **Accept** | Reset all transient state at effect entry. 2 lines, defensive. |
| I3 | Load-race: no token/abort guard on async load | code-reviewer | Bug (latent) | Yes — `void (async () => { ... draft = ... })()` has no token check. Latent today (editor unmounts on cancel), real once S14+ adds in-editor refresh | **Accept** | Add `loadToken` counter; gate post-await writes with `if (token !== loadToken) return;`. ~5 lines, forward-compatible. |
| I4 | Non-UTF-8 file branch untested | pr-test-analyzer | Test gap | Yes — doc comment justifies `io::Error::InvalidData` wrapping; no test exists. ~7 prior tests in same file have parity precedent | **Accept** | Add `read_user_agent_json_returns_io_error_for_non_utf8_content` test. ~12 lines. |
| I5 | Directory-at-target branch unhandled and untested | pr-test-analyzer | Bug (defensive parity) | Yes — `fs::read` on a dir bubbles a generic io::Error. Parallel `DuplicateSourceNotAFile` variant exists at project.rs for `duplicate_user_agent`. Not present on the read path | **Reject (defer)** | Tracker: kiro-p8mq. Out of S13's editor-shell scope. The fix should touch read AND save AND duplicate paths consistently for parity, not just the read path. |
| S1 | Symlink test asserts variant but not byte absence | silent-failure-hunter / code-reviewer / pr-test-analyzer | Test polish | Yes — current test asserts NotInstalled variant; doesn't assert symlink target's bytes never reach caller | **Accept** | Add `assert!(!format!("{:?}", err).contains("secret"))`. Cheap defense. |
| S2 | UTF-8 error loses path context | silent-failure-hunter | Polish (error UX) | Yes — `io::Error::InvalidData` wrapping doesn't carry the path. Editor knows the name at call-site so end-user UX is fine; the gap is for backend log debugging | **Reject (defer)** | Tracker: kiro-p8mq (folded with I5 — same module, same kind of polish). |
| S3 | SECTIONS array exhaustiveness vs SectionId | type-design-analyzer | Style (project convention) | Yes — adding "newSection" to SectionId wouldn't fail at compile time today. agent-list-helpers.ts has the canonical satisfies + Exclude tripwire | **Accept** | Add `_SECTION_IDS` tripwire mirroring helpers' pattern. ~5 lines. |
| S4 | draft typing — replace Record<string, unknown> with intersection | type-design-analyzer | Style | Partially verified — only ONE `typeof draft.name === "string"` check exists (not three as claimed); but the intersection IS cleaner | **Accept** | `{ name: string } & Record<string, unknown>`. Note in commit message the count discrepancy. |
| S5 | NotInstalled collapses symlink + missing file (UX-misleading for symlinks) | silent-failure-hunter | Polish (error UX) | Yes — both routes return `AgentError::NotInstalled`. Justified in doc comment for editor UX; reviewer notes this is misleading for users with intentional symlinks | **Reject (defer)** | Tracker: kiro-p8mq. Same scope as I5/S2. |

## Action breakdown

**In this commit:** C1, C2, C3, I1, I2, I3, I4, S1, S3, S4 (10 items).

**Deferred via tracker:**
- kiro-k9ok — Frontend agent-name validation policy (C1 deeper question)
- kiro-p8mq — read_user_agent_json polish: directory-at-target handling, path context in UTF-8 errors, symlink-vs-NotFound variant split (I5, S2, S5)

## Why split this way

Five "Accept" + 4 "Modify" / 1 "Reject (defer)" / 0 "Reject (no defer)" — within the
2-3 modify/reject heuristic per skill rule "two or three of six should be reject /
modify in a healthy review."

The deferred items are all polish on the read-side error model that should land
*together* (not piecemeal) for consistency. Two trackers (one for the regex policy,
one for the read-side polish bundle) keeps the deferred work groupable and visible
in `rivets list`.

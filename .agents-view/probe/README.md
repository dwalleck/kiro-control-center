# Prove-it-prototype: agents-view slice 1

Discharges the `gilfoyle:prove-it-prototype` gate before any design / plan /
code / test for the agents-view feature. Spec: `../spec.md`.

## Question

> Given a project's `.kiro/agents/` directory and its `.kiro/installed-agents.json`
> tracking file, can the list-page payload (one row per agent file, with
> marketplace lineage badge when present) be computed reliably?

This is the data dependency for spec behaviors B1, B3, B7, B8, B10, B11, B12.
If this join doesn't work cleanly, every downstream UI behavior is in trouble.

## Probe

`probe.py` — Python 3.13, filesystem-first.

1. Walk `.kiro/agents/*.json` on disk.
2. Parse each via `json.loads`.
3. Project to row shape (name, description, model, tools_count, mcp_count,
   resources_count, hooks_count).
4. Look up the agent's `name` in tracking; attach lineage if found, else null.
5. Sort by name, emit JSON.

```
python probe.py <project_path>
```

## Oracle

`oracle.ps1` — PowerShell 7, tracking-first.

Different runtime (CLR vs CPython), different JSON parser (`System.Text.Json`
via `ConvertFrom-Json` vs CPython `json` module), different file-iteration
model (`Get-ChildItem` pipeline vs `Path.glob`). Different starting source of
truth: walks **tracking** first, only emits a row if the file exists; then
filesystem-walks for files NOT in tracking.

If both arrive at the same row set, the data join is sound regardless of which
direction the implementation chooses.

```
pwsh -File oracle.ps1 <project_path>
```

(First oracle attempt was `oracle.sh` (bash + jq). It produced a false
disagreement on Windows because Windows-native `jq.exe` emits CRLF and bash's
`read -r` left trailing `\r` on each name, breaking the `[ -f $jf ]` check. Bug
cause #4 from the skill — the oracle was wrong, not the system. Rewritten in
PowerShell, which is native to the actual runtime. Disagreement disappeared.)

## Comparison

`compare.py` — semantic diff (both outputs normalized by sorting rows by
`name`, then deep-equal). Necessary because Python and PowerShell serialize
JSON keys in different orders (alphabetical vs insertion); a text diff would
report false disagreement on every row.

## Results

### Slice A — synthetic mixed-lineage fixture (`fixture/`)

Two agent files (`user-authored.json` with no tracking, `marketplace-tracked.json`
with tracking) plus an orphan tracking entry (`orphan-tracking` is in
`installed-agents.json` but has no file on disk).

```
> python probe.py fixture > probe_fixture.json
> pwsh -File oracle.ps1 fixture > oracle_fixture.json
> python compare.py probe_fixture.json oracle_fixture.json
AGREE on 2 rows: ['marketplace-tracked', 'user-authored']
```

Both implementations produce identical payloads for both rows, all 8 fields.
Both silently exclude the orphan tracking entry — by different mechanisms but
to the same end. See **What I learned (2)** below.

### Slice B — real `.kiro/` in this repo

Production-shape data: 7 agents, all marketplace-installed via the
`kiro-starter-kit` / `kiro-code-reviewer-v2` plugin.

```
> python probe.py C:\Users\dwall\repos\kiro-control-center > probe_real.json
> pwsh -File oracle.ps1 C:\Users\dwall\repos\kiro-control-center > oracle_real.json
> python compare.py probe_real.json oracle_real.json
AGREE on 7 rows: ['code-reviewer', 'code-simplifier', 'comment-analyzer',
                  'pr-test-analyzer', 'review-orchestrator',
                  'silent-failure-hunter', 'type-design-analyzer']
```

Identical payloads, all 7 rows. The list-page payload is computable from this
repo's actual data by two independent paths that agree.

## What I learned (each, one sentence)

1. **The repo's existing `.kiro/agents/` has zero user-authored agents** —
   every file is marketplace-tracked through `kiro-starter-kit`. I told the
   skill the opposite. Spec success-criterion **S2** (marketplace coexistence)
   has live data to test against, but slice 1's e2e for the user-authored path
   needs a synthesized fixture; no production user-authored data exists in
   this repo to exercise against.

2. **The natural list-payload join silently drops orphan tracking entries**
   (agent named in `installed-agents.json` but no file at
   `.kiro/agents/<name>.json`). Both source-of-truth strategies arrive at this
   same default. The current spec doesn't decide whether the UI should surface
   such entries as a "missing file" warning row or stay silent — this is an
   edge case the spec should explicitly nail down before slice 1 builds.

3. **No `kiro-market-core` Rust code is needed to compute the list-page
   payload.** The whole join is `serde_json::from_slice` over two files. The
   existing `parse_native::parse_kiro_cli_agent_json` exists for *install-time
   validation*, not for *display-time projection*. The new Tauri command can
   read files directly, saving an indirection layer and a non-trivial parser
   dependency for the listing path.

4. **Every existing agent in this repo has `model: null`.** None set an
   explicit model override. The list-page model chip must handle null
   gracefully (design says "Use default" placeholder, confirmed required, not
   just defensive).

## Hard gate (skill requires all four)

- [x] Probe written and runs against the real codebase
- [x] Oracle defined (different runtime, different parser, different starting
      source of truth) and produces output
- [x] Probe and oracle agree on at least one non-trivial slice (two slices:
      synthetic mixed-lineage, then real production data)
- [x] Four "what I learned" notes (above), all non-obvious before running

## Spec impact (must be addressed before `falsifiable-design`)

- **Finding 2** is a new edge case the spec must explicitly decide. Three
  candidate behaviors:
  - (a) Silently drop orphan entries (current natural behavior; matches both
    probe and oracle).
  - (b) Show a "missing file — restore or remove tracking?" UX row with a
    one-click fix.
  - (c) Auto-remove orphan tracking entries when the list view opens (data
    hygiene; matches "single source of truth = filesystem").

Recommendation: (a) for slice 1, file a rivets follow-up for (b) if real
users hit the case. (c) risks losing recoverable state.

## Hand-off

This artifact discharges the gate. The next gilfoyle skill in the chain is
[[falsifiable-design]], which can now design the implementation with the
ground truth established here.

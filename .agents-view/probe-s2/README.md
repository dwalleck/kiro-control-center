# Prove-it-prototype: agents-view slice 2 (kiro-vgnw)

Discharges the `gilfoyle:prove-it-prototype` gate before any design / plan /
code for the **Tools section** of the agents-view editor. Spec: `../spec.md`
decision #5 (static catalog from the design bundle, not a runtime query
into kiro-cli).

## Question

> What does the design bundle's `AGENT_TOOLS` array actually contain, and
> can it be ported into `crates/kiro-control-center/src/lib/agents/tools-catalog.ts`
> without silently dropping a tool or category?

This is the data dependency for spec § 5 of the design (Tools section UI).
A silent drop during port has no test surface today — the future TS module
would simply render fewer tools than the screenshots show, and the
regression would only surface during manual QA against the screenshots.

## Probe

`probe.py` — Python 3.13, regex-extract from the JS text.

1. Read `Kiro Control Center Design System/design_handoff_agents/source/agents-data.js`.
2. Locate `window.AGENT_TOOLS = [ ... ];` with a non-greedy multiline regex.
3. Quote the unquoted JS keys, drop a trailing comma, hand to `json.loads`.
4. Project to `{name, category, summary}`, sort by name, emit normalized JSON.

```
python probe.py "../../Kiro Control Center Design System/design_handoff_agents/source/agents-data.js" > probe.out
```

## Oracle

`oracle.mjs` — Node.js, V8-evaluated.

Different on three independent axes:
- **Runtime**: Node.js / V8 vs CPython.
- **Parser**: V8 reads the JS as actual JavaScript (the source-of-truth
  language) vs Python's regex-over-text approach.
- **Source-of-truth direction**: probe parses text and reconstructs the
  array; oracle executes the file in a `vm.createContext` sandbox with
  `window = {}` shim and reads `window.AGENT_TOOLS` directly.

```
node oracle.mjs "../../Kiro Control Center Design System/design_handoff_agents/source/agents-data.js" > oracle.out
```

If both arrive at the same row set, the data port has a faithful target
regardless of which extraction strategy the future `tools-catalog.ts` uses.

## Result — AGREE

`diff probe.out oracle.out` is empty after both write LF line endings.
Confirmed on 2026-05-26 against the design bundle as of commit `b491c3e`
on `feat/typed-strip-yaml-warnings`.

**Locked answer:**

```
tool_count: 15
category_count: 9
categories: Cloud, Code, Filesystem, Meta, Orchestration, Planning, Reasoning, Shell, Web
```

Tools by category:

| Category | Tools |
|---|---|
| Cloud | `use_aws` |
| Code | `code`, `glob`, `grep` |
| Filesystem | `fs_read`, `fs_write` |
| Meta | `introspect`, `report_issue`, `session` |
| Orchestration | `use_subagent` |
| Planning | `todo_list` |
| Reasoning | `thinking` |
| Shell | `execute_bash` |
| Web | `web_fetch`, `web_search` |

The `tools-catalog.ts` port MUST emit these 15 tools across these 9 categories.
A vitest assertion `expect(TOOLS_CATALOG.length).toBe(15)` plus
`expect([...new Set(TOOLS_CATALOG.map(t => t.category))].sort()).toEqual(<above>)`
is the slice-2 falsifier — any future regression that drops a category
during port fails immediately.

## CRLF gotcha (recorded for posterity)

First diff run reported "every line different" — false disagreement caused
by Python's text-mode `print()` emitting CRLF on Windows while Node's
`stdout.write` emitted LF. Fixed by routing probe output through
`sys.stdout.buffer.write(...)` with an explicit `\n` terminator. Mirrors
the same Windows-CRLF gotcha hit in `probe/` slice-1 (originally
`oracle.sh` + jq.exe).

## Out of scope for this probe

- **Cross-section state cleanup behavior** (toggle off → scrub `tools[]`,
  `allowedTools[]`, `toolAliases{}`). This is a *code claim*, not a data
  observation — falsifier lives inside the slice as a vitest unit test, not
  here.
- **AGENT_MODELS / HOOK_EVENTS / SAMPLE_AGENTS**. The probe is scoped to
  the Tools section only; future slices (Models, Hooks, etc.) get their
  own probes if a similar port-fidelity question lands.
- **Visual fidelity to the screenshots**. Subjective, reviewer-eyeball.
  Out of scope per spec criterion S7.

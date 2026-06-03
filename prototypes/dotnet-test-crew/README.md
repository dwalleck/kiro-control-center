# dotnet-test → Kiro crew conversion (prove-it prototype)

A hand conversion of the **generator + researcher + planner + implementer** slice of the
`dotnet-test` marketplace plugin into Kiro's parent-only crew (DAG) model. The goal is to
falsify the conversion approach against ONE real .NET repo before generalizing to a converter.

## What this proves (and what it can't)

The source pipeline is 5 dispatch levels deep and Kiro is parent-only (1 level). This slice
flattens it to a 3-stage linear DAG:

```
research ──▶ plan ──▶ implement
(.testagent/research.md) (.testagent/plan.md) (writes tests, build/test/fix inline)
```

Flattening decisions baked in:
- **builder/tester/fixer/linter agents are deleted** — the implementer runs `dotnet build` /
  `dotnet test` / `dotnet format` itself (collapses one nesting level).
- **per-phase fan-out is collapsed** into a single `implement` stage that loops phases, because
  the planner decides the phase count at runtime (a static DAG can't enumerate them).
- **the researcher's "spawn parallel sub-agents" step is removed** — it greps/globs itself.
- **the iterative "loop until coverage" strategy lives in the orchestrator's turns**, not the
  DAG: the generator reads `.testagent/status.md` + coverage, then issues another narrowed crew
  pass. A DAG is acyclic; the loop cannot live inside it.
- **stage handoff is via `.testagent/*.md` on a shared cwd**, NOT via prompt templating — the
  crew tool only substitutes `{task}`. This is why the data-flow survives the `{task}`-only limit.

## Crew tool wiring (RESOLVED — from `kiro-cli-chat` 2.5.0 binary extraction)

Source of truth: `~/repos/cyril/docs/kiro-subagent-tool-schemas.md` (schemas pulled from the
backend binary, not inferred). Key facts:

- **There are two engines.** v1 (`--agent-engine v1`) exposes `use_subagent` (parallel-only,
  no `depends_on`) and legacy `delegate`. **v2 is the default** and exposes **`agent_crew`** —
  the DAG-with-loops tool whose schema (`task` + `mode:["blocking"]` + `stages[]` with
  `name`/`role`/`prompt_template`/`depends_on`/`model`/`loop_to`) is exactly what `crew-dag.json`
  uses. `use_subagent` CANNOT express ordering and is NOT a fallback.
- **Agent-config grant:** the orchestrator must list **`subagent`** in its `tools[]`
  (or use the `@builtin` sigil), and configure **`toolsSettings.subagent`** with
  `availableAgents` / `trustedAgents` (glob patterns supported). NB the settings key is
  `subagent`, **not** `crew` — matching this repo's existing `.kiro/agents/review-orchestrator.json`.
  The wire/tool name the model emits on v2 is `agent_crew`; the config-side grant name is `subagent`.
- **Leaves inherit their own config.** A spawned stage uses its own agent's `tools` /
  `toolsSettings` / `allowedTools` — trust does not inherit. Our leaf configs grant no
  `subagent`, so they structurally cannot spawn. Flatten enforced at the capability layer.

These corrections are already applied to all four `*.json` (`subagent` in `tools[]`,
`toolsSettings.subagent`). The `stages[]` payload was already schema-valid.

## Two variants in this folder

| File | Stages | Iteration | Final validation | Use to test |
|---|---|---|---|---|
| `crew-dag.json` | 3 (research→plan→implement) | manual — orchestrator re-issues a narrowed crew pass across its own turns | orchestrator runs it (Step 5) | the minimal flatten + disk handoff (F1) |
| `crew-dag-loop.json` | 5 (+`validate`) | **native** — `validate` fires `loop_to → implement` | the `validate` stage runs it in-crew | the loop primitive + summary-driven feedback (F5–F7) |

Both share the same `research`/`plan`/`implement` agents; the loop variant adds
`code-testing-validator`. The orchestrator's `availableAgents`/`trustedAgents` include the
validator, so it can drive either variant.

## Native `loop_to` variant — how it works

`agent_crew` stages support a `loop_to` block — `{ target, trigger, max_iterations }` — that
re-runs `target` (with the triggering stage's feedback as context) when this stage's output
contains `trigger`. A stage fires it by calling the `summary` tool with
`resultType: "changes_needed"`. Bounds: `trigger` ≥ 4 chars, `max_iterations` ∈ 1..=10, no
self-loops, no mutual loops, planned upfront.

`crew-dag-loop.json` adds a `validate` stage (role `code-testing-validator`) after `implement`
with `loop_to: { target: "implement", trigger: "COVERAGE_GAP", max_iterations: 3 }`. The
validator runs the full-solution build/test, audits for weak tests, and does the coverage-gap
review; when gaps remain it calls `summary(resultType="changes_needed")` with `COVERAGE_GAP` +
a concrete gap list, and the engine re-runs `implement` **with that feedback as context**. When
clean it calls `summary(resultType="terminal")` and the crew ends.

Legality (all enforced by Kiro): `validate → implement` is not a self-loop; `implement` reaches
`validate` only via a plain `depends_on` (not a `loop_to`), so it is not a mutual loop;
`max_iterations: 3 ∈ 1..=10`; `trigger` "COVERAGE_GAP" is 12 chars ≥ 4. The forward
`depends_on` chain stays acyclic; the cycle exists only as the `loop_to` overlay.

### Extra falsifiers the loop variant introduces

- **F5 — loop fires:** on a repo with a deliberately under-covered class, confirm `implement`
  runs a 2nd time after `validate` emits `COVERAGE_GAP`. If it doesn't, the
  `summary(resultType="changes_needed")` → `loop_to.trigger` match isn't wiring up.
- **F6 — feedback injection:** confirm the 2nd `implement` run actually received the validator's
  gap list as context (it targets the named files, not a blind re-run of all phases). This is the
  one edge the docs say injects predecessor output — verify it really does.
- **F7 — `summary` is reachable from a leaf:** the validator config does **not** list a `summary`
  tool (docs call it "built-in" for subagent sessions). Verify a stage agent can actually call
  `summary` without an explicit grant. If not, add it to the validator's `tools[]` and re-test.
  This is the loop variant's analogue of F1 — the one undocumented assumption.

## How to run the prove-it test

The deterministic target lives in [`fixture/`](./fixture/) — a green `net8.0` solution
(`DotnetTestFixture.sln`) with **two source modules across two projects** (`Fixture.Numbers`,
`Fixture.Orders`; four branchy classes, partial existing tests). Multi-module is deliberate: a
single-file request shortcuts to the Direct strategy and never spawns a crew, so the fixture is
sized to make the crew the honest choice. See `fixture/README.md` for the gap table and
per-falsifier expectations.

1. Run `setup-test-project.sh [TARGET]` — it assembles the fixture + `.kiro/agents/` +
   `.kiro/skills/` into a standalone Kiro project and confirms it builds green.
2. In Kiro, invoke the generator with **module/solution scope** so it routes to the crew:
   *"generate comprehensive tests for the Fixture.Numbers and Fixture.Orders modules"*.
   (A single-file ask → Direct → no crew. If it still shortcuts, add: *"use the
   research→plan→implement crew pipeline; do not use the Direct strategy."*)
3. Confirm the crew ran: `.testagent/research.md` + `plan.md` exist.
4. Check `crew-dag.json` behavior first (F1/F2), then drive `crew-dag-loop.json` (F5–F7).

## Doc ⇄ binary reconciliation (public docs vs `kiro-cli-chat` 2.5.0)

`https://kiro.dev/docs/cli/chat/subagents/` **agrees with the binary extraction** on every
wiring fact: tool granted as **`subagent`** in `tools[]` (or `@builtin`); config under
**`toolsSettings.subagent`** with `availableAgents` / `trustedAgents`; DAG planned upfront;
≤4 parallel; review loops with trigger ≥4 chars and `max_iterations` ≤10, no self-loops;
`is_interactive` and `dangerously_trust_all_tools` present. The corrections in this prototype
are now confirmed by two independent sources.

Two gaps the docs leave open, both already handled here:
- **Nesting depth is unspecified in the public docs** (the binary doc and Kiro's own
  orchestrator gate say parent-only). Moot for us: the leaf configs grant no `subagent`, so they
  **cannot** spawn regardless of the engine limit — the flatten is enforced at the capability
  layer, not by trusting the docs.
- **No shared-state mechanism is documented.** Docs say results return via the **`summary`** tool
  to the *main agent*, and forward `prompt_template` only substitutes `{task}` — so a successor
  stage is NOT handed its predecessor's output by the engine. Disk (`.testagent/*.md`) is the only
  available forward channel, and the docs neither guarantee nor forbid that stages share a
  writable cwd. **That is exactly what F1 tests.**

## What to verify (the falsifiers)

- **F1 — disk handoff works (make-or-break):** after the run, `.testagent/research.md` and
  `.testagent/plan.md` exist in the project root and `plan.md` clearly consumed `research.md`.
  If the planner stage ran against a missing/empty research file, **stages do not share a
  writable cwd** and the single-DAG design is invalid. **Fallback:** drop the one-shot DAG and
  have the orchestrator sequence stages across its own turns — run the `research` stage, read its
  `summary`, then emit the `plan` stage with the research text inlined into its `prompt_template`
  (the orchestrator composes that string, so it can embed predecessor output that the engine
  won't auto-pass). Slower, more orchestrator tokens, but immune to the cwd question.
- **F2 — flatten holds:** the implementer actually built and ran tests via its own `dotnet`
  calls (no attempt to spawn builder/tester). If it tried to delegate and failed, the inline
  flatten is incomplete.
- **F3 — fan-in (deferred):** this slice is linear, so the dangerous "can a stage read TWO
  predecessors' disk artifacts" question isn't exercised yet. Add a fan-in stage later to test it.
- **F4 — final validation:** the orchestrator ran a full-solution `dotnet build`/`dotnet test`
  after the crew returned, not just the scoped build.

If F1 and F2 hold on a real repo, the conversion approach is sound and worth generalizing.
```

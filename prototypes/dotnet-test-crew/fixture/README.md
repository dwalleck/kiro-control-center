# Fixture: deterministic crew-pipeline target (multi-module)

A throwaway .NET solution sized to make the **crew** the honest strategy choice — two source
modules across two projects, four branchy classes, partial existing tests. A single-file request
gets shortcut to the Direct strategy; this multi-module shape routes the generator to a
Research→Plan→Implement crew instead. Pinned to SDK 8.0.x via `global.json` for reproducibility
(classic `.sln`, `net8.0`).

```
DotnetTestFixture.sln
src/Fixture.Numbers/   NumberClassifier.cs   Statistics.cs
src/Fixture.Orders/    Money.cs              DiscountPolicy.cs
tests/Fixture.Numbers.Tests/  NumberClassifierTests.cs   (covers 1 branch)
tests/Fixture.Orders.Tests/   MoneyTests.cs              (covers 1 happy path)
```

## Starting state (verified green)

```
dotnet build DotnetTestFixture.sln --no-incremental   # 0 errors, 4 projects
dotnet test  DotnetTestFixture.sln                    # Passed: 1 + 1
```

## The intentional gaps (what coverage analysis must find)

| Class | Covered | Untested (the gap) |
|---|---|---|
| `Numbers.NumberClassifier` | `Classify(0)`→"zero" | Classify negative/small/large; all of IsPrime; all of Factorial incl. throw |
| `Numbers.Statistics` | nothing | Mean (+empty throw); Max (+empty throw); Trend up/down/flat |
| `Orders.Money` | happy ctor + ToString + normalization | negative-amount throw; blank-currency throw; Add happy; Add currency-mismatch throw |
| `Orders.DiscountPolicy` | nothing | all four rate tiers; negative-quantity throw; `Apply` (crosses into Money) |

A correct run grows the two existing test files (append-only) and/or adds new files (e.g.
`StatisticsTests.cs`, `DiscountPolicyTests.cs`) in the matching test project — never edits `src/`.

## Invocation that forces the crew (not Direct)

The Direct strategy fires only for a single small file. To route to the crew, scope the request
to the modules / solution:

> use the code-testing-generator agent to generate comprehensive tests for the
> **Fixture.Numbers and Fixture.Orders modules** (the whole solution)

If `.testagent/research.md` + `plan.md` appear, the crew path was taken. (If the generator still
shortcuts, add: *"Use the research→plan→implement crew pipeline; do not use the Direct strategy."*)

## What each falsifier should observe

- **F1 (disk handoff):** `.testagent/research.md` enumerates all four classes across both modules
  with per-file coverage estimates, and `.testagent/plan.md` references those exact classes —
  proving `plan` read `research`'s file across stages.
- **F2 (inline flatten):** `.testagent/status.md` shows the implementer built/tested via its own
  `dotnet` calls per phase; no attempt to spawn builder/tester agents.
- **F3 (fan-in, still deferred):** linear DAG, not exercised here.
- **F4 (final validation):** a full-solution `dotnet build`/`dotnet test` ran after the crew —
  catching any cross-project break between Fixture.Numbers and Fixture.Orders.
- **F5 (loop fires, loop variant):** with two modules and ~16 branches, the first implement pass
  is more likely to leave a real gap; `validate` then emits `COVERAGE_GAP` and `implement` reruns.
  To guarantee a loop, narrow the first pass to `Fixture.Numbers` only so all of `Fixture.Orders`
  stays an unmistakable gap.
- **F6 (feedback injection):** the 2nd `implement` run targets exactly the classes `validate`
  named in `COVERAGE_GAP` — not a blind re-run.
- **F7 (`summary` reachable from a leaf):** `validate` calls `summary` despite not listing it in
  `tools[]`. If the loop never fires, add `summary` to `code-testing-validator.json`'s `tools[]`.

## Reset between runs

The throwaway project (e.g. `~/repos/dotnet-test-crew`) is not a git repo, so re-run the setup
script to restore the partial-test starting state:

```
prototypes/dotnet-test-crew/setup-test-project.sh ~/repos/dotnet-test-crew
```

# Demo Runbook — dotnet-test crew on a real .NET project

A one-page script for demoing the converted crew to your team. Pitch: *"proof that a
Copilot/Claude agent pipeline runs natively in Kiro as a multi-agent crew"* — **not** "the
dotnet-test plugin is ported." Scope the whole demo to **test generation only**.

---

## 0. Pick the project (do this first — it decides whether the demo works)

- ✅ **Multi-project solution** (2+ `.csproj`) — single-project/single-file → the generator shortcuts to Direct and no crew runs.
- ✅ **Green test baseline** — `dotnet test` passes before you start, so any failure is clearly the tool's.
- ✅ **An obvious untested class or two** — visible payoff.
- ❌ Avoid first-demo repos with red tests, central package management quirks, multi-targeting, or heavy integration tests.

If unsure the structure will trigger the crew, send me the repo layout and I'll sanity-check it.

## 1. Install into the real repo (NOT the fixture script — that copies the fixture)

```bash
PROTO=/home/dwalleck/repos/kiro-control-center/prototypes/dotnet-test-crew
REPO=/path/to/your/real/dotnet/repo                      # <-- set this
SKILLS=/home/dwalleck/repos/skills/plugins/dotnet-test/skills

mkdir -p "$REPO/.kiro/agents" "$REPO/.kiro/skills"
cp -r "$PROTO/agents/." "$REPO/.kiro/agents/"            # 5 configs + prompts/
cp -r "$SKILLS/code-testing-extensions" "$REPO/.kiro/skills/"
cp -r "$SKILLS/code-testing-agent"      "$REPO/.kiro/skills/"
# the generator config references these at the repo root — create if absent:
for f in CLAUDE.md AGENTS.md .editorconfig; do [ -f "$REPO/$f" ] || : > "$REPO/$f"; done

cd "$REPO" && dotnet test *.sln                          # confirm GREEN baseline
echo "$REPO/.testagent/" >> "$REPO/.gitignore"           # crew writes its audit trail here
```

## 2. Dress-rehearse (the highest-value step — do it ≥1×, ideally 2× to see the variance)

Run the full §3 flow privately on this exact repo before the team sees it. Two runs tells you
whether the strategy gate is stable for *your* repo. If it shortcuts to Direct both rehearsals,
use the explicit override (§3) in the live run.

---

## 3. Live demo — the invocation

Start Kiro in the repo, confirm `code-testing-generator` is listed, then paste **verbatim**:

> Use the **code-testing-generator** agent. Generate comprehensive unit tests for the
> **whole solution (all modules/projects)**. **Use the research → plan → implement crew
> pipeline — do not use the Direct strategy.**

Narrate while it runs (watch stages with **Ctrl+G**):
- *"It's spawning a crew — research, then plan, then implement — as separate Kiro agents."*
- *"Each stage hands off through `.testagent/` files; the plan stage reads what research wrote."*

## 4. Reveal the evidence (this is the money shot)

```bash
cat .testagent/research.md      # research enumerated the classes + a dependency graph
cat .testagent/plan.md          # plan ordered phases leaf-first off that graph
cat .testagent/status.md        # per-phase implement results
dotnet test *.sln               # NEW tests, all green
git diff --stat                 # only test files changed — src/ untouched (append-only)
```

The headline: *"It analyzed, planned, implemented, and validated — and it never touched
production code."*

## 5. Recovery lines (if something goes sideways)

- **It chose Direct (no `.testagent/`):** *"It judged the scope small enough to do in one pass —
  that's the built-in fast path. Let me scope it to force the full crew,"* then re-invoke adding
  *"treat this as a large multi-module task; use the crew pipeline."*
- **A generated test fails:** *"The implementer fixes assertions against real behavior within a
  retry budget — on a real codebase that occasionally needs a human nudge,"* then move on.
- **Full-solution build is slow:** *"It runs a non-incremental full build to catch cross-project
  breaks — thorough over fast; scoped builds run during implementation."*

## 6. Guardrails for Q&A

- **"Can it migrate my xUnit v2→v3 / audit test quality?"** → *"Not in this converted slice — only
  the generation pipeline is ported. Those agents are next."* (Don't let it get steered there;
  test-migration / testability-migration / test-quality-auditor aren't converted.)
- **"Is this the full plugin?"** → *"It's 5 of the 11 agents — the test-generation crew — proven
  end-to-end. It's a conversion proof-of-concept, not a finished port."*
- **"How reliable is the strategy choice?"** → Be honest: *"The Direct-vs-crew decision is
  model-driven and not perfectly deterministic yet — that's why a real converter would add a
  review checkpoint."*

## Reset between runs

`.testagent/` is preserved by design (it's the audit trail). To re-run clean:
```bash
rm -rf "$REPO/.testagent"; git -C "$REPO" checkout -- tests/   # discard generated tests
```

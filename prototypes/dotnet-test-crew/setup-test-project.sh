#!/usr/bin/env bash
# Assemble a standalone, ready-to-run Kiro project from the crew prototype + fixture.
# After running, launch kiro-cli in $TARGET and invoke the code-testing-generator agent.
#
# Usage:  ./setup-test-project.sh [TARGET_DIR]
#   TARGET_DIR defaults to ~/repos/dotnet-test-crew-run (wiped and recreated each run).
set -euo pipefail

PROTO="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SKILLS_SRC="/home/dwalleck/repos/skills/plugins/dotnet-test/skills"
TARGET="${1:-$HOME/repos/dotnet-test-crew-run}"

echo "==> Target project: $TARGET"
rm -rf "$TARGET"
mkdir -p "$TARGET/.kiro/agents" "$TARGET/.kiro/skills"

echo "==> 1/4 Copy the fixture (the .NET solution under test)"
cp -r "$PROTO/fixture/." "$TARGET/"
rm -f "$TARGET/README.md"            # drop the fixture's own README from the project root

echo "==> 2/4 Install the crew agents (configs + prompts) into .kiro/agents/"
cp -r "$PROTO/agents/." "$TARGET/.kiro/agents/"

echo "==> 3/4 Install the two skills the agents reference into .kiro/skills/"
for s in code-testing-extensions code-testing-agent; do
  if [[ -d "$SKILLS_SRC/$s" ]]; then
    cp -r "$SKILLS_SRC/$s" "$TARGET/.kiro/skills/$s"
    echo "      + $s"
  else
    echo "      ! MISSING skill source: $SKILLS_SRC/$s" >&2
  fi
done

echo "==> 4/4 Create the context files the agent resources reference (empty is fine)"
: > "$TARGET/CLAUDE.md"
: > "$TARGET/AGENTS.md"
: > "$TARGET/.editorconfig"

# Drop the prototype crew-dag examples in for reference (not required by Kiro)
cp "$PROTO/crew-dag.json" "$PROTO/crew-dag-loop.json" "$TARGET/.kiro/"

echo
echo "==> Sanity: project builds green before any test generation"
( cd "$TARGET" && DOTNET_NOLOGO=1 DOTNET_CLI_TELEMETRY_OPTOUT=1 \
    dotnet test DotnetTestFixture.sln 2>&1 | tail -2 )

cat <<EOF

==> READY. Next steps (these run interactively in Kiro — not scriptable here):

  1. cd "$TARGET"
  2. Confirm the default engine is v2 (so agent_crew is live):
       kiro-cli acp --help | grep -i agent-engine     # expect [default: v2]
  3. Start a chat and confirm the agents were discovered:
       kiro-cli chat
       > /agents          (or however your build lists agents — look for code-testing-generator)
  4. Invoke the orchestrator with MODULE/SOLUTION scope so it routes to the crew
     (a single-file request shortcuts to the Direct strategy and no crew runs):
       use the code-testing-generator agent to generate comprehensive tests for the
       Fixture.Numbers and Fixture.Orders modules (the whole solution)
     If it still shortcuts, add: "Use the research->plan->implement crew pipeline;
     do not use the Direct strategy."
  5. Watch stages live with Ctrl+G. After it finishes, check the falsifiers:
       cat .testagent/research.md     # F1: did 'plan' get real research?
       cat .testagent/status.md       # F2: inline dotnet build/test?
       dotnet test DotnetTestFixture.sln   # did coverage actually grow?

  To exercise the native loop (F5-F7), tell the generator to use the loop variant /
  the validate stage, or temporarily narrow the first implement pass to Classify only
  so IsPrime/Factorial remain a guaranteed COVERAGE_GAP. See fixture/README.md.

  Reset between runs: just re-run this script (it wipes and rebuilds $TARGET).
EOF

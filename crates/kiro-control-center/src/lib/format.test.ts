import { describe, expect, it } from "vitest";
import type {
  InstallPluginResult_Serialize,
  MarketplaceName,
  PluginName,
  RemovePluginResult,
} from "$lib/bindings";
import { formatInstallPluginResult, formatRemovePluginResult } from "./format";

// Field names + structure tracked from bindings.ts (see plan
// "Source-of-truth references"):
//  - InstallSkillsResult:             bindings.ts:667-686
//  - InstallSteeringResult_Serialize: bindings.ts:699-703
//  - InstallAgentsResult_Serialize:   bindings.ts:522-560
//  - FailedSkill:                     bindings.ts:352-356
//  - InstalledSteeringOutcome:        bindings.ts:853-859
//  - InstallOutcomeKind:              bindings.ts:568-581
function emptyInstallResult(): InstallPluginResult_Serialize {
  return {
    marketplace: "acme" as MarketplaceName,
    plugin: "p" as PluginName,
    version: null,
    skills: { installed: [], skipped: [], failed: [], skipped_skills: [] },
    steering: { installed: [], failed: [], warnings: [] },
    // InstallAgentsResult_Serialize requires installed_native +
    // installed_companions (bindings.ts:553, :559). Both default to
    // empty/null in this fixture.
    agents: {
      installed: [],
      skipped: [],
      failed: [],
      warnings: [],
      installed_native: [],
      installed_companions: null,
    },
  };
}

describe("formatInstallPluginResult", () => {
  it("happy path: counts all 3 sub-results", () => {
    const r = emptyInstallResult();
    r.skills.installed = ["a", "b"];
    r.steering.installed = [
      { source: "s.md", destination: "s.md", kind: "installed", source_hash: "h", installed_hash: "h" },
    ];
    // installed: string[] (bindings.ts:527), not an object array.
    r.agents.installed = ["g"];
    const out = formatInstallPluginResult(r, "p");
    expect(out.summary).toContain("2 skill");
    expect(out.summary).toContain("1 steering");
    expect(out.summary).toContain("1 agent");
    expect(out.anyInstalled).toBe(true);
    expect(out.anyFailed).toBe(false);
  });

  it("failures-only: anyInstalled=false, anyFailed=true", () => {
    const r = emptyInstallResult();
    // FailedSkill requires `kind: FailedSkillReason` (bindings.ts:352-356).
    r.skills.failed = [
      { name: "broken", error: "oops", kind: { kind: "install_failed" } },
    ];
    const out = formatInstallPluginResult(r, "p");
    expect(out.anyInstalled).toBe(false);
    expect(out.anyFailed).toBe(true);
    expect(out.summary).toContain("1 skill failed");
  });

  it("warnings-only (e.g. MCP-gated agent): warnings string present, no failure flag", () => {
    const r = emptyInstallResult();
    r.agents.warnings = [
      { kind: "mcp_servers_require_opt_in", agent: "scary", transports: ["stdio"] },
    ];
    const out = formatInstallPluginResult(r, "p");
    expect(out.warnings).not.toBeNull();
    expect(out.warnings).toContain("scary");
    expect(out.anyFailed).toBe(false);
  });

  it("empty: summary reads 'nothing to install'", () => {
    const r = emptyInstallResult();
    const out = formatInstallPluginResult(r, "p");
    expect(out.summary).toBe("nothing to install");
    expect(out.anyInstalled).toBe(false);
    expect(out.anyFailed).toBe(false);
  });

  it("skipped (idempotent skill): counted as 'already installed'", () => {
    const r = emptyInstallResult();
    r.skills.skipped = ["a", "b"];
    const out = formatInstallPluginResult(r, "p");
    expect(out.summary).toContain("2 skills already installed");
  });
});

// Field names + structure tracked from bindings.ts:
//  - RemovePluginResult:  bindings.ts:1171-1175
//  - RemoveSkillsResult:  bindings.ts:1181-1184
//  - RemoveSteeringResult: bindings.ts:1190-1193
//  - RemoveAgentsResult:  bindings.ts:1134-1137
//  - RemoveItemFailure:   bindings.ts:1145-1156
function emptyRemoveResult(): RemovePluginResult {
  return {
    skills: { removed: [], failures: [] },
    steering: { removed: [], failures: [] },
    agents: { removed: [], failures: [] },
  };
}

describe("formatRemovePluginResult", () => {
  it("happy path: counts all 3 sub-results", () => {
    const r = emptyRemoveResult();
    r.skills.removed = ["a", "b", "c"];
    r.steering.removed = ["s.md"];
    r.agents.removed = ["g1", "g2"];
    const out = formatRemovePluginResult(r, "p");
    expect(out.summary).toContain("3 skill");
    expect(out.summary).toContain("1 steering");
    expect(out.summary).toContain("2 agent");
    expect(out.hasItems).toBe(true);
    expect(out.hasFailures).toBe(false);
  });

  it("steering failure lands in summary (failed count) and hasFailures=true", () => {
    const r = emptyRemoveResult();
    r.steering.failures = [{ item: "broken.md", error: "permission denied" }];
    const out = formatRemovePluginResult(r, "p");
    expect(out.hasFailures).toBe(true);
    expect(out.summary).toContain("1 steering failed");
  });

  it("empty (zero items, zero failures): hasItems=false, hasFailures=false", () => {
    const r = emptyRemoveResult();
    const out = formatRemovePluginResult(r, "p");
    expect(out.hasItems).toBe(false);
    expect(out.hasFailures).toBe(false);
    expect(out.summary).toBe("nothing to remove");
  });

  it("treats undefined removed/failures as empty arrays", () => {
    // The wire format makes both fields optional (#[serde(default)]).
    const r: RemovePluginResult = {
      skills: {},
      steering: {},
      agents: {},
    };
    const out = formatRemovePluginResult(r, "p");
    expect(out.hasItems).toBe(false);
    expect(out.hasFailures).toBe(false);
  });
});

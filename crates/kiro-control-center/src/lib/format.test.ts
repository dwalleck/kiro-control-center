import { describe, expect, it } from "vitest";
import type {
  AgentName,
  FailedAgent,
  FailedSkill,
  FailedSkillReason,
  FailedSteeringFile_Serialize,
  InstallPluginResult_Serialize,
  MarketplaceName,
  PluginName,
  RemovePluginResult,
  SkippedSkill,
} from "$lib/bindings";
import {
  formatFailedAgent,
  formatFailedSkill,
  formatFailedSteeringFile,
  formatInstallPluginResult,
  formatRemovePluginResult,
  formatSkippedSkillsForPlugin,
} from "./format";

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
  it("happy path: counts all 3 sub-results joined by mid-dot", () => {
    const r = emptyInstallResult();
    r.skills.installed = ["a", "b"];
    r.steering.installed = [
      { source: "s.md", destination: "s.md", kind: "installed", source_hash: "h", installed_hash: "h" },
    ];
    r.agents.installed = ["g"];
    const out = formatInstallPluginResult(r);
    expect(out.summary).toBe("2 skills · 1 steering file · 1 agent");
    expect(out.anyInstalled).toBe(true);
    expect(out.anyFailed).toBe(false);
  });

  it("singular nouns: 1 skill / 1 steering file / 1 agent", () => {
    const r = emptyInstallResult();
    r.skills.installed = ["a"];
    r.steering.installed = [
      { source: "s.md", destination: "s.md", kind: "installed", source_hash: "h", installed_hash: "h" },
    ];
    r.agents.installed = ["g"];
    const out = formatInstallPluginResult(r);
    expect(out.summary).toBe("1 skill · 1 steering file · 1 agent");
  });

  it("failures-only: anyInstalled=false, anyFailed=true, exact summary", () => {
    const r = emptyInstallResult();
    r.skills.failed = [
      { name: "broken", error: "oops", kind: { kind: "install_failed" } },
    ];
    const out = formatInstallPluginResult(r);
    expect(out.anyInstalled).toBe(false);
    expect(out.anyFailed).toBe(true);
    expect(out.summary).toBe("1 skill failed");
  });

  it("warnings-only (e.g. MCP-gated agent): warnings string present, no failure flag", () => {
    const r = emptyInstallResult();
    r.agents.warnings = [
      { kind: "mcp_servers_require_opt_in", agent: "scary", transports: ["stdio"] },
    ];
    const out = formatInstallPluginResult(r);
    expect(out.warnings).toBe(
      "agent 'scary' declares MCP servers [stdio] — re-run with --accept-mcp to install",
    );
    expect(out.anyFailed).toBe(false);
  });

  it("multiple warnings from different sub-results join with ' | '", () => {
    const r = emptyInstallResult();
    r.steering.warnings = [
      { kind: "scan_path_invalid", path: "/bad", reason: "not absolute" },
    ];
    r.agents.warnings = [
      { kind: "mcp_servers_require_opt_in", agent: "scary", transports: ["stdio"] },
    ];
    const out = formatInstallPluginResult(r);
    expect(out.warnings).toBe(
      "invalid scan path '/bad': not absolute" +
        " | " +
        "agent 'scary' declares MCP servers [stdio] — re-run with --accept-mcp to install",
    );
  });

  it("empty: summary reads 'nothing to install'", () => {
    const r = emptyInstallResult();
    const out = formatInstallPluginResult(r);
    expect(out.summary).toBe("nothing to install");
    expect(out.anyInstalled).toBe(false);
    expect(out.anyFailed).toBe(false);
  });

  it("skipped (idempotent skills): counted as 'already installed'", () => {
    const r = emptyInstallResult();
    r.skills.skipped = ["a", "b"];
    const out = formatInstallPluginResult(r);
    expect(out.summary).toBe("2 skills already installed");
  });

  it("does not interpolate plugin metadata (marketplace, version, plugin name) into summary", () => {
    const r = emptyInstallResult();
    r.marketplace = "verysecret-marketplace" as MarketplaceName;
    r.plugin = "verysecret-plugin" as PluginName;
    r.version = "9.9.9";
    r.skills.installed = ["a"];
    const out = formatInstallPluginResult(r);
    expect(out.summary).not.toContain("verysecret");
    expect(out.summary).not.toContain("9.9.9");
    expect(out.warnings).toBeNull();
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
  it("happy path: counts all 3 sub-results joined by mid-dot", () => {
    const r = emptyRemoveResult();
    r.skills.removed = ["a", "b", "c"];
    r.steering.removed = ["s.md"];
    r.agents.removed = ["g1", "g2"];
    const out = formatRemovePluginResult(r);
    expect(out.summary).toBe("3 skills · 1 steering file · 2 agents");
    expect(out.hasItems).toBe(true);
    expect(out.hasFailures).toBe(false);
  });

  it("singular nouns: 1 skill / 1 steering file / 1 agent removed", () => {
    const r = emptyRemoveResult();
    r.skills.removed = ["a"];
    r.steering.removed = ["s.md"];
    r.agents.removed = ["g"];
    const out = formatRemovePluginResult(r);
    expect(out.summary).toBe("1 skill · 1 steering file · 1 agent");
  });

  it("singular failure nouns: 1 skill / steering / 1 agent failed", () => {
    const r = emptyRemoveResult();
    r.skills.failures = [{ item: "x", error: "oops" }];
    r.steering.failures = [{ item: "s.md", error: "denied" }];
    r.agents.failures = [{ item: "g", error: "boom" }];
    const out = formatRemovePluginResult(r);
    expect(out.summary).toBe("1 skill failed · 1 steering failed · 1 agent failed");
  });

  it("steering failure lands in summary (failed count) and hasFailures=true", () => {
    const r = emptyRemoveResult();
    r.steering.failures = [{ item: "broken.md", error: "permission denied" }];
    const out = formatRemovePluginResult(r);
    expect(out.hasFailures).toBe(true);
    expect(out.summary).toBe("1 steering failed");
  });

  it("mixed removed + failures within one sub-result: both flags true", () => {
    const r = emptyRemoveResult();
    r.skills.removed = ["a"];
    r.skills.failures = [{ item: "broken", error: "oops" }];
    const out = formatRemovePluginResult(r);
    expect(out.hasItems).toBe(true);
    expect(out.hasFailures).toBe(true);
    expect(out.summary).toBe("1 skill · 1 skill failed");
  });

  it("empty (zero items, zero failures): hasItems=false, hasFailures=false", () => {
    const r = emptyRemoveResult();
    const out = formatRemovePluginResult(r);
    expect(out.hasItems).toBe(false);
    expect(out.hasFailures).toBe(false);
    expect(out.summary).toBe("nothing to remove");
  });

  it("treats undefined removed/failures as empty arrays", () => {
    const r: RemovePluginResult = {
      skills: {},
      steering: {},
      agents: {},
    };
    const out = formatRemovePluginResult(r);
    expect(out.hasItems).toBe(false);
    expect(out.hasFailures).toBe(false);
  });
});

function frontmatterFailure(name: string): SkippedSkill {
  return {
    plugin: "p",
    name_hint: name,
    path: `/p/${name}/SKILL.md`,
    reason: { kind: "frontmatter_invalid", reason: "missing name field" },
  };
}

describe("formatSkippedSkillsForPlugin", () => {
  it("empty list reads as 0 skill(s) failed with no body", () => {
    expect(formatSkippedSkillsForPlugin([])).toBe("0 skill(s) failed to load — ");
  });

  it("under-cap (3 entries): all listed inline, no overflow suffix", () => {
    const list = [frontmatterFailure("a"), frontmatterFailure("b"), frontmatterFailure("c")];
    const out = formatSkippedSkillsForPlugin(list);
    expect(out).toBe(
      "3 skill(s) failed to load — " +
        "a: malformed frontmatter: missing name field; " +
        "b: malformed frontmatter: missing name field; " +
        "c: malformed frontmatter: missing name field",
    );
  });

  it("at-cap (5 entries): all listed, no overflow suffix", () => {
    const list = ["a", "b", "c", "d", "e"].map(frontmatterFailure);
    const out = formatSkippedSkillsForPlugin(list);
    expect(out.startsWith("5 skill(s) failed to load — ")).toBe(true);
    expect(out).not.toContain("more");
  });

  it("over-cap (6 entries): first 5 listed, '+1 more' suffix", () => {
    const list = ["a", "b", "c", "d", "e", "f"].map(frontmatterFailure);
    const out = formatSkippedSkillsForPlugin(list);
    expect(out.endsWith("; +1 more")).toBe(true);
    expect(out.startsWith("6 skill(s) failed to load — ")).toBe(true);
    expect(out).toContain("e: malformed frontmatter");
    expect(out).not.toContain("f: malformed frontmatter");
  });
});

describe("formatFailedSteeringFile", () => {
  it("renders source and error joined by em-dash", () => {
    const f: FailedSteeringFile_Serialize = {
      source: "some/file.md",
      error: "permission denied: foo",
    };
    expect(formatFailedSteeringFile(f)).toBe("some/file.md — permission denied: foo");
  });
});

describe("formatFailedSkill", () => {
  it("install_failed variant: renders name — error", () => {
    const f: FailedSkill = {
      name: "my-skill",
      error: "io error: permission denied",
      kind: { kind: "install_failed" },
    };
    expect(formatFailedSkill(f)).toBe("my-skill — io error: permission denied");
  });

  it("requested_but_not_found variant: renders name — error", () => {
    const f: FailedSkill = {
      name: "my-skill",
      error: "skill 'my-skill' not found in plugin 'acme'",
      kind: { kind: "requested_but_not_found", plugin: "acme" },
    };
    expect(formatFailedSkill(f)).toBe(
      "my-skill — skill 'my-skill' not found in plugin 'acme'",
    );
  });

  it("assertNever path: throws for unknown kind", () => {
    const f: FailedSkill = {
      name: "my-skill",
      error: "some error",
      // Cast through `unknown` to inject an invalid runtime variant that
      // TypeScript cannot catch statically — exercises the default assertNever arm.
      kind: { kind: "totally_new_variant" } as unknown as FailedSkillReason,
    };
    expect(() => formatFailedSkill(f)).toThrow();
  });
});

describe("formatFailedAgent", () => {
  it("agent variant: renders name (source_path) — error", () => {
    const f: FailedAgent = {
      kind: "agent",
      name: "code-reviewer" as AgentName,
      source_path: "agents/code-reviewer.md",
      error: "io error: permission denied",
    };
    expect(formatFailedAgent(f)).toBe(
      "code-reviewer (agents/code-reviewer.md) — io error: permission denied",
    );
  });

  it("unparseable_agent variant: renders source_path (unparseable) — error", () => {
    const f: FailedAgent = {
      kind: "unparseable_agent",
      source_path: "agents/broken.md",
      error: "missing frontmatter fence",
    };
    expect(formatFailedAgent(f)).toBe(
      "agents/broken.md (unparseable) — missing frontmatter fence",
    );
  });

  it("companion_bundle with empty conflicts: renders [no enumeration] placeholder", () => {
    // covers MultipleScanRootsNotSupported — rejection before per-file enumeration
    const f: FailedAgent = {
      kind: "companion_bundle",
      plugin: "demo-plugin" as PluginName,
      conflicts: [],
      error: "multiple scan roots not supported",
    };
    expect(formatFailedAgent(f)).toBe(
      "demo-plugin bundle [no enumeration] — multiple scan roots not supported",
    );
  });

  it("companion_bundle with one conflict: renders the conflict path", () => {
    const f: FailedAgent = {
      kind: "companion_bundle",
      plugin: "demo-plugin" as PluginName,
      conflicts: ["agents/prompts/x.md"],
      error: "orphan conflict at agents/prompts/x.md",
    };
    expect(formatFailedAgent(f)).toBe(
      "demo-plugin bundle [agents/prompts/x.md] — orphan conflict at agents/prompts/x.md",
    );
  });

  it("companion_bundle with multiple conflicts: comma-joins all paths", () => {
    // Forward-compat coverage: the wire shape supports N conflicts even though
    // the engine emits length-0 or length-1 today (per F1 / kiro-1ah3, deferred).
    // Pins the .join(", ") behavior against a future refactor to indexed access.
    const f: FailedAgent = {
      kind: "companion_bundle",
      plugin: "demo-plugin" as PluginName,
      conflicts: ["agents/prompts/x.md", "agents/prompts/y.md"],
      error: "multiple orphan conflicts",
    };
    expect(formatFailedAgent(f)).toBe(
      "demo-plugin bundle [agents/prompts/x.md, agents/prompts/y.md] — multiple orphan conflicts",
    );
  });

  it("assertNever path: throws for unknown kind", () => {
    // Double-cast through `unknown` to inject an invalid runtime variant —
    // the ts-expect-error directive does not fire on double-casts (valid TS
    // expression), so `as unknown as FailedAgent` is the correct pattern.
    // Same approach as formatFailedSkill's assertNever test above.
    const f = { kind: "future_variant", error: "x" } as unknown as FailedAgent;
    expect(() => formatFailedAgent(f)).toThrow();
  });
});

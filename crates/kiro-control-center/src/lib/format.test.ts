import { describe, expect, it } from "vitest";
import type {
  AgentName,
  CommandError,
  FailedAgent,
  FailedSkill,
  FailedSkillReason,
  FailedSteeringFile_Serialize,
  InstallPluginResult_Serialize,
  MarketplaceName,
  PluginName,
  RemovePluginResult,
  SkippedItem,
  SkippedSkill,
} from "$lib/bindings";
import type { SteeringWarning } from "$lib/bindings";
import {
  formatCommandError,
  formatFailedAgent,
  formatFailedSkill,
  formatFailedSteeringFile,
  formatInstallPluginResult,
  formatRemovePluginResult,
  formatSkippedItemsForPlugin,
  formatSkippedSkillsForPlugin,
  formatSteeringWarning,
} from "./format";

// Field names + structure tracked from bindings.ts. Line numbers
// omitted because bindings.ts is auto-regenerated and they rot;
// search by type name:
//  - InstallSkillsResult
//  - InstallSteeringResult_Serialize
//  - InstallAgentsResult_Serialize
//  - FailedSkill
//  - InstalledSteeringOutcome
//  - InstallOutcomeKind
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

describe("formatCommandError", () => {
  it.each([
    {
      name: "keeps an error without remediation unchanged",
      error: {
        message: "disk full",
        error_type: "io_error",
        remediation: null,
      } satisfies CommandError,
      expected: "disk full",
    },
    {
      name: "appends remediation after the stable message",
      error: {
        message: "plugin is not available locally",
        error_type: "validation",
        remediation:
          "use the CLI: run `kiro-market install p@<marketplace>` to clone it locally",
      } satisfies CommandError,
      expected:
        "plugin is not available locally — use the CLI: run `kiro-market install p@<marketplace>` to clone it locally",
    },
    {
      name: "treats whitespace-only remediation as absent",
      error: {
        message: "disk full",
        error_type: "io_error",
        remediation: "   ",
      } satisfies CommandError,
      expected: "disk full",
    },
    {
      name: "uses a fallback for an empty message",
      error: {
        message: "   ",
        error_type: "unknown",
        remediation: null,
      } satisfies CommandError,
      expected: "Unknown error",
    },
    {
      name: "keeps a raw string error verbatim",
      error: "raw error message",
      expected: "raw error message",
    },
    {
      name: "uses a fallback for a null error",
      error: null,
      expected: "Unknown error",
    },
  ])("$name", ({ error, expected }) => {
    expect(formatCommandError(error)).toBe(expected);
  });
});

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

  it("requested_but_not_found variant: composes 'agent X not found in plugin Y'", () => {
    const f: FailedAgent = {
      kind: "requested_but_not_found",
      name: "ghost-agent" as AgentName,
      plugin: "demo-plugin" as PluginName,
    };
    expect(formatFailedAgent(f)).toBe(
      "agent 'ghost-agent' not found in plugin 'demo-plugin'",
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

describe("formatSkippedItemsForPlugin", () => {
  function skillItem(name: string): SkippedItem {
    return {
      kind: "skill",
      skill: {
        plugin: "p",
        name_hint: name,
        path: `/p/${name}/SKILL.md`,
        reason: { kind: "frontmatter_invalid", reason: "missing name field" },
      },
    };
  }
  function steeringItem(path: string): SkippedItem {
    return {
      kind: "steering_discovery",
      warning: { kind: "scan_path_invalid", path, reason: "not absolute" },
    };
  }
  function agentItem(source: string, reason: string): SkippedItem {
    return {
      kind: "agent_parse",
      plugin: "p",
      source_path: source,
      reason,
    };
  }

  it("empty list: returns empty string (no parts joined)", () => {
    expect(formatSkippedItemsForPlugin([])).toBe("");
  });

  it("skill-only bucket: routes through formatSkippedSkillsForPlugin", () => {
    const out = formatSkippedItemsForPlugin([skillItem("a"), skillItem("b")]);
    expect(out).toBe(
      "2 skill(s) failed to load — " +
        "a: malformed frontmatter: missing name field; " +
        "b: malformed frontmatter: missing name field",
    );
  });

  it("steering-only bucket under cap: lists detail, no overflow", () => {
    const out = formatSkippedItemsForPlugin([
      steeringItem("/a"),
      steeringItem("/b"),
    ]);
    expect(out).toBe(
      "2 steering warning(s) — " +
        "invalid scan path '/a': not absolute; " +
        "invalid scan path '/b': not absolute",
    );
  });

  it("steering bucket over cap (4 entries, MAX=3): '+1 more' suffix", () => {
    const out = formatSkippedItemsForPlugin([
      steeringItem("/a"),
      steeringItem("/b"),
      steeringItem("/c"),
      steeringItem("/d"),
    ]);
    expect(out.startsWith("4 steering warning(s) — ")).toBe(true);
    expect(out.endsWith("; +1 more")).toBe(true);
    expect(out).toContain("'/c'");
    expect(out).not.toContain("'/d'");
  });

  it("agent-only bucket: source_path: reason joined by '; '", () => {
    const out = formatSkippedItemsForPlugin([
      agentItem("/agents/a.md", "missing frontmatter"),
      agentItem("/agents/b.json", "invalid JSON"),
    ]);
    expect(out).toBe(
      "2 agent(s) failed to parse — " +
        "/agents/a.md: missing frontmatter; " +
        "/agents/b.json: invalid JSON",
    );
  });

  it("agent bucket over cap (4 entries, MAX=3): '+1 more' suffix", () => {
    const out = formatSkippedItemsForPlugin([
      agentItem("/a.md", "r1"),
      agentItem("/b.md", "r2"),
      agentItem("/c.md", "r3"),
      agentItem("/d.md", "r4"),
    ]);
    expect(out.startsWith("4 agent(s) failed to parse — ")).toBe(true);
    expect(out.endsWith("; +1 more")).toBe(true);
    expect(out).toContain("/c.md: r3");
    expect(out).not.toContain("/d.md: r4");
  });

  it("mixed buckets: skills | steering | agents joined by ' | ' in that order", () => {
    const out = formatSkippedItemsForPlugin([
      skillItem("a"),
      steeringItem("/bad"),
      agentItem("/agents/x.md", "boom"),
    ]);
    expect(out).toBe(
      "1 skill(s) failed to load — a: malformed frontmatter: missing name field" +
        " | " +
        "1 steering warning(s) — invalid scan path '/bad': not absolute" +
        " | " +
        "1 agent(s) failed to parse — /agents/x.md: boom",
    );
  });

  it("assertNever path: throws for unknown kind", () => {
    const bad = { kind: "future_variant" } as unknown as SkippedItem;
    expect(() => formatSkippedItemsForPlugin([bad])).toThrow();
  });
});

describe("formatSteeringWarning", () => {
  it("scan_path_invalid: renders path and reason", () => {
    const w: SteeringWarning = {
      kind: "scan_path_invalid",
      path: "/bad",
      reason: "not absolute",
    };
    expect(formatSteeringWarning(w)).toBe("invalid scan path '/bad': not absolute");
  });

  it("scan_dir_unreadable: renders path and reason", () => {
    const w: SteeringWarning = {
      kind: "scan_dir_unreadable",
      path: "/tmp/plugins/x/steering",
      reason: "permission denied",
    };
    expect(formatSteeringWarning(w)).toBe(
      "could not read steering dir '/tmp/plugins/x/steering': permission denied",
    );
  });

  it("source_not_utf8: names the path AND states bytes installed verbatim", () => {
    // The "installed bytes verbatim" tail is the user-actionable signal —
    // tells them the file is on disk and recoverable. Drift here drops a
    // key piece of the lenient-install contract.
    const w: SteeringWarning = {
      kind: "source_not_utf8",
      path: "/tmp/plugins/x/steering/binary.md",
    };
    expect(formatSteeringWarning(w)).toBe(
      "steering source '/tmp/plugins/x/steering/binary.md' is not valid UTF-8; installed bytes verbatim",
    );
  });

  it("unclosed_frontmatter: names the path AND states bytes installed verbatim", () => {
    const w: SteeringWarning = {
      kind: "unclosed_frontmatter",
      path: "/tmp/plugins/x/steering/unfinished.md",
    };
    expect(formatSteeringWarning(w)).toBe(
      "steering source '/tmp/plugins/x/steering/unfinished.md' has an unclosed YAML frontmatter fence; installed bytes verbatim",
    );
  });

  it("assertNever path: throws for unknown kind", () => {
    const bad = { kind: "future_variant" } as unknown as SteeringWarning;
    expect(() => formatSteeringWarning(bad)).toThrow();
  });
});

import { describe, expect, test } from "vitest";

import {
  type AddCustomResult,
  addAllowed,
  addCustomTool,
  partitionTools,
  removeAllowed,
  toggleTool,
  type ToolsDraft,
  toolsRailBadge,
} from "./tool-state";

const emptyDraft: ToolsDraft = Object.freeze({
  tools: [],
  allowedTools: [],
  toolAliases: {},
});

describe("toggleTool — cascade cleanup (C2)", () => {
  // Plan case 1: full cascade on disable.
  test("disable scrubs name from tools, allowedTools, and toolAliases", () => {
    const before: ToolsDraft = {
      tools: ["fs_read"],
      allowedTools: ["fs_read"],
      toolAliases: { fs_read: "read" },
    };
    const after = toggleTool(before, "fs_read");
    expect(after.tools).toEqual([]);
    expect(after.allowedTools).toEqual([]);
    expect(after.toolAliases).toEqual({});
  });

  // Plan case 2: disable preserves other tools' state.
  test("disable preserves other tools' allowedTools and aliases", () => {
    const before: ToolsDraft = {
      tools: ["fs_read", "grep"],
      allowedTools: ["other"],
      toolAliases: { grep: "g" },
    };
    const after = toggleTool(before, "fs_read");
    expect(after.tools).toEqual(["grep"]);
    expect(after.allowedTools).toEqual(["other"]);
    expect(after.toolAliases).toEqual({ grep: "g" });
  });

  // Plan case 3: enable does not touch allowedTools or aliases.
  test("enable appends to tools, leaves allowedTools and aliases unchanged", () => {
    const before: ToolsDraft = {
      tools: [],
      allowedTools: ["pre-existing"],
      toolAliases: { other: "o" },
    };
    const after = toggleTool(before, "fs_read");
    expect(after.tools).toEqual(["fs_read"]);
    expect(after.allowedTools).toEqual(["pre-existing"]);
    expect(after.toolAliases).toEqual({ other: "o" });
  });

  // Plan case 4: idempotent enable — double-enable doesn't duplicate.
  test("enable is idempotent — calling twice does not duplicate", () => {
    let draft: ToolsDraft = emptyDraft;
    draft = toggleTool(draft, "fs_read");
    draft = toggleTool(draft, "fs_read"); // second call is now a disable
    expect(draft.tools).toEqual([]);
  });

  test("re-enable after disable produces clean state (cascade verification)", () => {
    let draft: ToolsDraft = {
      tools: ["fs_read"],
      allowedTools: ["fs_read"],
      toolAliases: { fs_read: "read" },
    };
    draft = toggleTool(draft, "fs_read"); // disable: cascade
    draft = toggleTool(draft, "fs_read"); // re-enable
    // Without the cascade, the re-enable would leave the alias and
    // allowed-list entries that the disable should have scrubbed.
    expect(draft.tools).toEqual(["fs_read"]);
    expect(draft.allowedTools).toEqual([]);
    expect(draft.toolAliases).toEqual({});
  });
});

describe("addAllowed (C3)", () => {
  // Plan case 5: add to empty allowedTools, name not in tools[].
  // The "yellow chip / NOT VISIBLE" design state.
  test("appends to allowedTools even when name not in tools[]", () => {
    const before: ToolsDraft = {
      tools: ["a"],
      allowedTools: [],
      toolAliases: { a: "x" },
    };
    const after = addAllowed(before, "yellow_chip");
    expect(after.tools).toEqual(["a"]);
    expect(after.allowedTools).toEqual(["yellow_chip"]);
    expect(after.toolAliases).toEqual({ a: "x" });
  });

  // Plan case 6: dedupe.
  test("duplicate add is a no-op", () => {
    const before: ToolsDraft = {
      tools: [],
      allowedTools: ["a"],
      toolAliases: {},
    };
    const after = addAllowed(before, "a");
    expect(after).toEqual(before);
  });

  // Plan case 7: empty name rejected.
  test("empty / whitespace-only name returns unchanged", () => {
    expect(addAllowed(emptyDraft, "")).toEqual(emptyDraft);
    expect(addAllowed(emptyDraft, "   ")).toEqual(emptyDraft);
  });
});

describe("removeAllowed (C3)", () => {
  // Plan case 8.
  test("removes only from allowedTools — tools and aliases unchanged", () => {
    const before: ToolsDraft = {
      tools: ["a"],
      allowedTools: ["a"],
      toolAliases: { a: "x" },
    };
    const after = removeAllowed(before, "a");
    expect(after.tools).toEqual(["a"]);
    expect(after.allowedTools).toEqual([]);
    expect(after.toolAliases).toEqual({ a: "x" });
  });

  // Plan case 9.
  test("remove of non-existent name is idempotent (returns same draft)", () => {
    const before: ToolsDraft = {
      tools: [],
      allowedTools: ["a"],
      toolAliases: {},
    };
    const after = removeAllowed(before, "not-there");
    expect(after).toBe(before); // referential equality — no-op fast path
  });
});

describe("partitionTools (C4)", () => {
  // Plan case 10.
  test("pure native: empty external group", () => {
    expect(partitionTools(["fs_read", "grep"])).toEqual({
      native: ["fs_read", "grep"],
      external: [],
    });
  });

  // Plan case 11.
  test("pure MCP: empty native group", () => {
    expect(partitionTools(["@svc/foo", "@bar"])).toEqual({
      native: [],
      external: ["@svc/foo", "@bar"],
    });
  });

  // Plan case 12: source order preserved within each group.
  test("mixed: preserves source order within each group", () => {
    expect(
      partitionTools(["fs_read", "@svc/foo", "grep", "@bar"]),
    ).toEqual({
      native: ["fs_read", "grep"],
      external: ["@svc/foo", "@bar"],
    });
  });

  // Plan case 13 — adversarial. A name with `@` mid-string must
  // route to `native`. A `.includes("@")` substring-match would
  // misroute it; `.startsWith("@")` is the right anchor.
  test("adversarial: `@` mid-name routes to native (startsWith anchor)", () => {
    expect(partitionTools(["weird@embedded"])).toEqual({
      native: ["weird@embedded"],
      external: [],
    });
  });
});

describe("addCustomTool (C6)", () => {
  // Plan case 14.
  test("whitespace-only input returns ok:false, reason:empty", () => {
    const r = addCustomTool(emptyDraft, "   ");
    expect(r).toEqual<AddCustomResult>({ ok: false, reason: "empty" });
  });

  // Plan case 15.
  test("non-`@`-prefixed input returns ok:false, reason:not-mcp", () => {
    // The +Add custom flow is for MCP-style entries only. A native
    // name like "fs_read" must route through the by-category grid's
    // checkbox, not this affordance.
    expect(addCustomTool(emptyDraft, "fs_read")).toEqual({
      ok: false,
      reason: "not-mcp",
    });
  });

  // Plan case 16.
  test("well-formed new entry appends to BOTH tools and allowedTools", () => {
    const r = addCustomTool(emptyDraft, "@svc/foo");
    expect(r.ok).toBe(true);
    if (r.ok) {
      expect(r.draft.tools).toEqual(["@svc/foo"]);
      expect(r.draft.allowedTools).toEqual(["@svc/foo"]);
      expect(r.draft.toolAliases).toEqual({});
    }
  });

  // Plan case 17.
  test("duplicate returns ok:false, reason:duplicate", () => {
    const before: ToolsDraft = {
      tools: ["@svc/foo"],
      allowedTools: [],
      toolAliases: {},
    };
    expect(addCustomTool(before, "@svc/foo")).toEqual({
      ok: false,
      reason: "duplicate",
    });
  });

  // Plan case 18.
  test("trimmed input — surrounding whitespace stripped before validation", () => {
    const r = addCustomTool(emptyDraft, "  @svc/foo  ");
    expect(r.ok).toBe(true);
    if (r.ok) expect(r.draft.tools).toEqual(["@svc/foo"]);
  });

  // Allowed-dedupe edge: tool absent but already-allowed (e.g. user
  // yellow-chipped the name into allowedTools earlier). The customAdd
  // appends to tools but must NOT duplicate the allowedTools entry.
  test("allowedTools dedupe: pre-existing yellow-chip name not duplicated", () => {
    const before: ToolsDraft = {
      tools: [],
      allowedTools: ["@svc/foo"], // yellow-chip state
      toolAliases: {},
    };
    const r = addCustomTool(before, "@svc/foo");
    expect(r.ok).toBe(true);
    if (r.ok) {
      expect(r.draft.tools).toEqual(["@svc/foo"]);
      expect(r.draft.allowedTools).toEqual(["@svc/foo"]); // not duplicated
    }
  });
});

describe("toolsRailBadge (C5)", () => {
  test("empty tools returns null (badge hidden)", () => {
    expect(toolsRailBadge({ tools: [] })).toBeNull();
  });

  test("single tool returns 1", () => {
    expect(toolsRailBadge({ tools: ["fs_read"] })).toBe(1);
  });

  // Mix native + MCP — both count toward the badge.
  test("mixed native + MCP entries both count", () => {
    expect(toolsRailBadge({ tools: ["fs_read", "@svc/foo"] })).toBe(2);
  });

  // Adversarial: frozen input array must not be mutated by the read.
  test("does not mutate the input array (frozen-safe)", () => {
    const frozen = Object.freeze(["fs_read", "grep"]);
    expect(() => toolsRailBadge({ tools: frozen })).not.toThrow();
    expect(frozen).toEqual(["fs_read", "grep"]);
  });
});

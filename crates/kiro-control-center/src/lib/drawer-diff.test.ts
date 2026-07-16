import { describe, expect, it } from "vitest";
import type { AgentItemInfo } from "./bindings";
import {
  deriveDiff,
  deriveSectionState,
  pluralize,
  summarizePluginMcp,
  summarizeSelectedMcpInstalls,
} from "./drawer-diff";

// SvelteSet ≡ Set for the read-only contract (`.has`, `.size`) — vitest
// doesn't need the reactive shim, so use plain Set for fixtures.

describe("deriveSectionState", () => {
  it("empty: zero items returns 'empty' regardless of selected size", () => {
    expect(deriveSectionState([], new Set())).toBe("empty");
    expect(deriveSectionState([], new Set(["x"]))).toBe("empty");
  });

  it("none: zero selected returns 'none'", () => {
    expect(deriveSectionState([{ name: "a" }, { name: "b" }], new Set())).toBe("none");
  });

  it("all: every item selected returns 'all'", () => {
    expect(
      deriveSectionState([{ name: "a" }, { name: "b" }], new Set(["a", "b"])),
    ).toBe("all");
  });

  it("partial: some selected returns 'partial'", () => {
    expect(deriveSectionState([{ name: "a" }, { name: "b" }], new Set(["a"]))).toBe(
      "partial",
    );
  });

  it("partial when selected size matches via different names (size-only signal)", () => {
    // deriveSectionState only inspects `.size`, NOT membership; a
    // selected set whose `.size` equals items.length classifies as
    // "all" even if the names diverge. Documenting this edge so a
    // future change that adds membership-based check has a test to
    // update.
    expect(deriveSectionState([{ name: "a" }], new Set(["unrelated"]))).toBe("all");
  });
});

describe("deriveDiff", () => {
  it("empty inputs: empty diff", () => {
    expect(deriveDiff([], new Set())).toEqual({ install: [], remove: [] });
  });

  it("install: selected but not installed", () => {
    const items = [
      { name: "a", installed: false },
      { name: "b", installed: false },
    ];
    expect(deriveDiff(items, new Set(["a"]))).toEqual({
      install: ["a"],
      remove: [],
    });
  });

  it("remove: installed but not selected", () => {
    const items = [
      { name: "a", installed: true },
      { name: "b", installed: true },
    ];
    expect(deriveDiff(items, new Set(["a"]))).toEqual({
      install: [],
      remove: ["b"],
    });
  });

  it("no-op cases: selected+installed → noop; unselected+!installed → noop", () => {
    const items = [
      { name: "kept", installed: true },
      { name: "absent", installed: false },
    ];
    expect(deriveDiff(items, new Set(["kept"]))).toEqual({
      install: [],
      remove: [],
    });
  });

  it("mixed install + remove + noop", () => {
    const items = [
      { name: "add", installed: false },
      { name: "drop", installed: true },
      { name: "kept", installed: true },
      { name: "skipped", installed: false },
    ];
    expect(deriveDiff(items, new Set(["add", "kept"]))).toEqual({
      install: ["add"],
      remove: ["drop"],
    });
  });

  it("preserves item order in install/remove arrays", () => {
    const items = [
      { name: "z", installed: false },
      { name: "a", installed: false },
      { name: "m", installed: false },
    ];
    expect(deriveDiff(items, new Set(["z", "a", "m"]))).toEqual({
      install: ["z", "a", "m"],
      remove: [],
    });
  });

  it("selected name not in items list is silently ignored", () => {
    // A drawer state that holds a name no longer in entry.items
    // (catalog refresh shrunk the list while the drawer was open)
    // shouldn't crash or produce a phantom install entry — there's
    // no item to push.
    const items = [{ name: "a", installed: false }];
    expect(deriveDiff(items, new Set(["a", "phantom"]))).toEqual({
      install: ["a"],
      remove: [],
    });
  });
});

describe("pluralize", () => {
  it("returns singular for n=1", () => {
    expect(pluralize(1, "skill", "skills")).toBe("skill");
  });

  it("returns plural for n=0", () => {
    expect(pluralize(0, "skill", "skills")).toBe("skills");
  });

  it("returns plural for n>1", () => {
    expect(pluralize(2, "skill", "skills")).toBe("skills");
    expect(pluralize(99, "skill", "skills")).toBe("skills");
  });

  it("works with multi-word nouns", () => {
    expect(pluralize(1, "steering file", "steering files")).toBe("steering file");
    expect(pluralize(3, "steering file", "steering files")).toBe("steering files");
  });
});

function agent(
  name: string,
  installed: boolean,
  mcpServerTransports: string[],
): AgentItemInfo {
  return {
    name,
    description: null,
    plugin: "demo",
    marketplace: "test",
    installed,
    dialect: "copilot",
    mcp_server_transports: mcpServerTransports,
  };
}

describe("MCP consent summaries", () => {
  const agents = [
    agent("selected-new", false, ["stdio", "stdio"]),
    agent("unselected-new", false, ["http"]),
    agent("selected-installed", true, ["sse"]),
    agent("selected-future", false, ["quic"]),
    agent("plain", false, []),
  ];

  it("whole-plugin scope preserves agent order and counts every server", () => {
    expect(summarizePluginMcp(agents)).toEqual({
      agentNames: [
        "selected-new",
        "unselected-new",
        "selected-installed",
        "selected-future",
      ],
      serverCount: 5,
      transports: [
        { label: "http", count: 1 },
        { label: "quic", count: 1 },
        { label: "sse", count: 1 },
        { label: "stdio", count: 2 },
      ],
    });
  });

  it("drawer scope includes only selected agents that are not installed", () => {
    expect(
      summarizeSelectedMcpInstalls(
        agents,
        new Set(["selected-new", "selected-installed", "selected-future", "phantom"]),
      ),
    ).toEqual({
      agentNames: ["selected-new", "selected-future"],
      serverCount: 3,
      transports: [
        { label: "quic", count: 1 },
        { label: "stdio", count: 2 },
      ],
    });
  });

  it("empty and MCP-free scopes need no consent", () => {
    expect(summarizePluginMcp([])).toBeNull();
    expect(summarizePluginMcp([agent("plain", false, [])])).toBeNull();
    expect(
      summarizeSelectedMcpInstalls(agents, new Set(["plain", "unselected-new"])),
    ).toEqual({
      agentNames: ["unselected-new"],
      serverCount: 1,
      transports: [{ label: "http", count: 1 }],
    });
    expect(summarizeSelectedMcpInstalls(agents, new Set(["plain"]))).toBeNull();
  });

  it("unknown transport labels remain consent-requiring and renderable", () => {
    expect(
      summarizeSelectedMcpInstalls(agents, new Set(["selected-future"])),
    ).toEqual({
      agentNames: ["selected-future"],
      serverCount: 1,
      transports: [{ label: "quic", count: 1 }],
    });
  });
});

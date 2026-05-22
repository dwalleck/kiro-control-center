import { describe, expect, it } from "vitest";
import type { UserAgentLineage, UserAgentRow } from "$lib/bindings";
import {
  filterAgentRows,
  formatLineageBadge,
  formatModelChip,
  headerLabel,
  type AgentsTabMode,
} from "$lib/agent-list-helpers";

// Test fixture: covers the four field-presence shapes the filter
// helper must handle without crashing — description null, model null,
// lineage null, plus a fully-populated row.
function row(overrides: Partial<UserAgentRow> = {}): UserAgentRow {
  return {
    name: "code-reviewer",
    description: "General-purpose code review agent.",
    model: "claude-opus-4-7",
    tools_count: 4,
    mcp_count: 0,
    resources_count: 2,
    hooks_count: 0,
    lineage: null,
    ...overrides,
  };
}

describe("filterAgentRows", () => {
  it("empty query returns every row (including null-description rows)", () => {
    // Bug class: a naive `description.toLowerCase()` chain crashes on
    // null. The empty-query path must NOT touch description/model,
    // so this also catches an over-eager filter that always inspects them.
    const rows = [
      row({ name: "a", description: null }),
      row({ name: "b", model: null }),
      row({ name: "c" }),
    ];
    expect(filterAgentRows(rows, "")).toEqual(rows);
  });

  it("case-insensitive name match", () => {
    const rows = [row({ name: "code-reviewer" })];
    expect(filterAgentRows(rows, "REVIEWER")).toHaveLength(1);
  });

  it("matches against description even when name does not", () => {
    const rows = [
      row({ name: "code-reviewer", description: "Orchestrates the pass" }),
    ];
    expect(filterAgentRows(rows, "orchestrat")).toHaveLength(1);
  });

  it("matches against model even when name and description do not", () => {
    // Bug class: a naive row.name.includes(q) implementation would miss
    // this and silently shrink the result.
    const rows = [
      row({
        name: "alpha",
        description: "beta",
        model: "claude-opus-4-7",
      }),
    ];
    expect(filterAgentRows(rows, "opus")).toHaveLength(1);
  });

  it("tolerates null description / model without crashing", () => {
    const rows = [row({ description: null, model: null })];
    // Query won't match (no fields contain "x") but must not throw.
    expect(filterAgentRows(rows, "x")).toHaveLength(0);
  });

  it("matches Unicode names", () => {
    const rows = [row({ name: "agent-üñîçødé" })];
    expect(filterAgentRows(rows, "üñîçødé")).toHaveLength(1);
  });

  it("returns a new array (defensive copy on empty query)", () => {
    // The empty-query branch should not return the input ref — the
    // caller may mutate the filtered list (re-sort, append) without
    // affecting the upstream source.
    const rows = [row()];
    const filtered = filterAgentRows(rows, "");
    expect(filtered).not.toBe(rows);
    expect(filtered).toEqual(rows);
  });
});

describe("formatLineageBadge", () => {
  it("returns null for user-authored rows", () => {
    expect(formatLineageBadge(null)).toBeNull();
  });

  it("emits marketplace · plugin · version for full lineage", () => {
    const lineage: UserAgentLineage = {
      marketplace: "kiro-starter-kit",
      plugin: "kiro-code-reviewer-v2",
      version: "0.1.0",
    };
    expect(formatLineageBadge(lineage)).toBe(
      "kiro-starter-kit · kiro-code-reviewer-v2 · 0.1.0",
    );
  });

  it("omits version segment when absent", () => {
    const lineage: UserAgentLineage = {
      marketplace: "mp",
      plugin: "p",
      version: null,
    };
    expect(formatLineageBadge(lineage)).toBe("mp · p");
  });
});

describe("formatModelChip", () => {
  it("returns 'Use default' for null model", () => {
    expect(formatModelChip(null)).toBe("Use default");
  });

  it("returns the model string when present", () => {
    expect(formatModelChip("claude-sonnet-4-6")).toBe("claude-sonnet-4-6");
  });
});

describe("headerLabel", () => {
  it("renders the list mode label", () => {
    const mode: AgentsTabMode = { kind: "list" };
    expect(headerLabel(mode)).toBe("Agents");
  });

  it("renders the new-agent mode label", () => {
    const mode: AgentsTabMode = { kind: "new" };
    expect(headerLabel(mode)).toBe("New agent");
  });

  // Pins the previously-ternaried "Editing ${row.name}" arm. A naive
  // chained ternary would produce this string for ANY non-"new" mode
  // (including "list" if the surrounding `if` regressed); the switch
  // routes through `mode.kind === "edit"` only.
  it("renders the edit-agent mode label with the row name", () => {
    const mode: AgentsTabMode = {
      kind: "edit",
      row: {
        name: "code-reviewer",
        description: null,
        model: null,
        tools_count: 0,
        mcp_count: 0,
        resources_count: 0,
        hooks_count: 0,
        lineage: null,
      },
    };
    expect(headerLabel(mode)).toBe("Editing code-reviewer");
  });
});

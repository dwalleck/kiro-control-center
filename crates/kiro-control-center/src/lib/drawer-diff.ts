import type { AgentItemInfo } from "$lib/bindings";

/**
 * Pure-function helpers for the CustomizeDrawer's per-item diff math.
 *
 * Extracted from CustomizeDrawer.svelte so vitest can cover the
 * set-difference logic without depending on Svelte runes or jsdom.
 * The .svelte component owns the SvelteSet state and renders;
 * everything that reads-only on that state lives here.
 *
 * The `SelectedSet` interface accepts both plain `Set<string>` and
 * `SvelteSet<string>` — both provide `.has(name)` and `.size`.
 */

export type SectionToggleState = "empty" | "none" | "partial" | "all";

export interface SelectedSet {
  has(name: string): boolean;
  readonly size: number;
}

export type CustomizeDrawerApply = {
  skills: { install: string[]; remove: string[] };
  steering: { install: string[]; remove: string[] };
  agents: { install: string[]; remove: string[] };
  acceptMcp: boolean;
};

export type McpConsentSummary = {
  readonly agentNames: readonly string[];
  readonly serverCount: number;
  readonly transports: readonly { label: string; count: number }[];
};

function summarizeMcp(
  agents: readonly AgentItemInfo[],
  selected: SelectedSet | null,
): McpConsentSummary | null {
  const agentNames: string[] = [];
  const transportCounts = new Map<string, number>();
  let serverCount = 0;

  for (const agent of agents) {
    if (selected !== null && (agent.installed || !selected.has(agent.name))) continue;
    if (agent.mcp_server_transports.length === 0) continue;

    agentNames.push(agent.name);
    for (const label of agent.mcp_server_transports) {
      serverCount++;
      transportCounts.set(label, (transportCounts.get(label) ?? 0) + 1);
    }
  }

  if (serverCount === 0) return null;
  const transports = Array.from(transportCounts, ([label, count]) => ({ label, count }));
  transports.sort((left, right) =>
    left.label < right.label ? -1 : left.label > right.label ? 1 : 0,
  );
  return { agentNames, serverCount, transports };
}

/** Summarize every MCP-bearing agent in a whole-plugin action. */
export function summarizePluginMcp(
  agents: readonly AgentItemInfo[],
): McpConsentSummary | null {
  return summarizeMcp(agents, null);
}

/** Summarize selected, not-installed MCP agents in a drawer action. */
export function summarizeSelectedMcpInstalls(
  agents: readonly AgentItemInfo[],
  selected: SelectedSet,
): McpConsentSummary | null {
  return summarizeMcp(agents, selected);
}

/**
 * Classify a category's current selection state for the section header
 * toggle. "empty" disables the toggle, "none"/"all" check or uncheck on
 * click, "partial" displays an indeterminate state.
 */
export function deriveSectionState(
  items: readonly { name: string }[],
  selected: SelectedSet,
): SectionToggleState {
  if (items.length === 0) return "empty";
  if (selected.size === 0) return "none";
  if (selected.size === items.length) return "all";
  return "partial";
}

/**
 * Compute the install/remove lists for one category. Compares the
 * current selection set against each item's `installed` flag:
 *   - selected AND !installed → install
 *   - !selected AND installed → remove
 *   - otherwise → no-op
 */
export function deriveDiff(
  items: readonly { name: string; installed: boolean }[],
  selected: SelectedSet,
): { install: string[]; remove: string[] } {
  const install: string[] = [];
  const remove: string[] = [];
  for (const i of items) {
    const willBeChecked = selected.has(i.name);
    if (willBeChecked && !i.installed) install.push(i.name);
    if (!willBeChecked && i.installed) remove.push(i.name);
  }
  return { install, remove };
}

/** Pick singular or plural form based on count. */
export function pluralize(n: number, singular: string, plural: string): string {
  return n === 1 ? singular : plural;
}

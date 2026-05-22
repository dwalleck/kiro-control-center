// Pure-logic helpers for the Agents list page (Workflows > Agents).
// Slice S10 of agents-view slice 1. Per CLAUDE.md vitest discipline,
// testable logic lives in non-`.svelte.ts` modules; the AgentsTab
// component (slice S12) is the dumb consumer of these helpers.

import type { UserAgentLineage, UserAgentRow } from "$lib/bindings";

/**
 * Filter agent rows by a free-text query.
 *
 * Empty query returns a fresh copy of every row. Non-empty matches
 * case-insensitively against `name`, `description`, and `model`. Per
 * spec B4: no debounce, filter state is component-local. Per design
 * input shapes: tolerant of `null` description and model.
 */
export function filterAgentRows(
  rows: readonly UserAgentRow[],
  query: string,
): UserAgentRow[] {
  if (!query) return [...rows];
  const q = query.toLowerCase();
  return rows.filter((r) => {
    if (r.name.toLowerCase().includes(q)) return true;
    if ((r.description ?? "").toLowerCase().includes(q)) return true;
    if ((r.model ?? "").toLowerCase().includes(q)) return true;
    return false;
  });
}

/**
 * Build the lineage badge text for a row. Returns `null` for
 * user-authored rows (no badge); otherwise the
 * `<marketplace> · <plugin> · <version>` triple. Version is omitted
 * if absent in tracking.
 */
export function formatLineageBadge(
  lineage: UserAgentLineage | null,
): string | null {
  if (!lineage) return null;
  if (lineage.version) {
    return `${lineage.marketplace} · ${lineage.plugin} · ${lineage.version}`;
  }
  return `${lineage.marketplace} · ${lineage.plugin}`;
}

/**
 * Display string for the model chip. `"Use default"` when null —
 * matches the design's placeholder for agents with no explicit
 * model override (every existing agent in `.kiro/agents/` has
 * `model: null`, surfaced as probe finding 4).
 */
export function formatModelChip(model: string | null): string {
  return model ?? "Use default";
}

// Pure-logic helpers for the Agents list page. Extracted from
// `AgentsTab.svelte` per CLAUDE.md's vitest discipline: testable
// logic lives in non-`.svelte.ts` modules so the component stays a
// dumb consumer.

import type { UserAgentLineage, UserAgentRow } from "$lib/bindings";

/**
 * Filter agent rows by a free-text query.
 *
 * Empty query returns a fresh copy of every row. Non-empty matches
 * case-insensitively against `name`, `description`, and `model`. No
 * debounce; filter state is component-local. Tolerant of `null`
 * description and model.
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
 * empirically, every agent in `.kiro/agents/` ships with
 * `model: null` so a sensible placeholder is the common case.
 */
export function formatModelChip(model: string | null): string {
  return model ?? "Use default";
}

/**
 * Discriminated union for the Agents tab's view mode. Pulled into a
 * shared module so the exhaustiveness guard in [`headerLabel`] has
 * a single canonical type to enumerate.
 */
export type AgentsTabMode =
  | { kind: "list" }
  | { kind: "new" }
  | { kind: "edit"; row: UserAgentRow };

/**
 * Per the CLAUDE.md TS-discipline rule, a `switch (mode.kind)` with a
 * `never`-typed default arm. A chained ternary would silently fall
 * through if a third arm landed without explicit handling.
 *
 * Tripwire: the `_AssertExhaustive` type alias resolves to `never`
 * iff every `AgentsTabMode` arm is enumerated in `_KINDS`; the
 * value-position `_assert: _AssertExhaustive = true` makes that
 * fire at compile time when a future contributor adds a fourth
 * `mode.kind` without updating both arrays.
 */
const _KINDS = ["list", "new", "edit"] as const satisfies ReadonlyArray<
  AgentsTabMode["kind"]
>;
type _AssertExhaustive =
  Exclude<AgentsTabMode["kind"], (typeof _KINDS)[number]> extends never
    ? true
    : never;
const _assert: _AssertExhaustive = true;
void _assert;

export function headerLabel(mode: AgentsTabMode): string {
  switch (mode.kind) {
    case "list":
      return "Agents";
    case "new":
      return "New agent";
    case "edit":
      return `Editing ${mode.row.name}`;
    default: {
      const _exhaustive: never = mode;
      throw new Error(
        `headerLabel: unhandled AgentsTabMode arm ${JSON.stringify(
          _exhaustive,
        )}`,
      );
    }
  }
}

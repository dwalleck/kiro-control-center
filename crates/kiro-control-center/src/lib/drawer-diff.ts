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

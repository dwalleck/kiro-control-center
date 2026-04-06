# Skill Card Description Expansion

## Problem

`SkillCard.svelte` uses CSS `truncate` (single-line ellipsis) on skill descriptions.
Long descriptions are cut off with no way to read the full text in the Control Center GUI.

## Solution

Replace single-line truncation with a 3-line clamp and an expand-in-place toggle.

## Design

### Behavior

- **Default:** description shows up to 3 lines via `line-clamp-3`. If the text fits,
  no toggle appears.
- **Overflow detected:** a "Show more" text button appears below the description.
- **Expanded:** clamp is removed, full description visible, button reads "Show less".
- **Collapsed:** returns to 3-line clamp.

### Overflow detection

Use a Svelte `$effect` that compares `scrollHeight > clientHeight` on the description
`<p>` element after mount and on content changes. This conditionally shows the toggle
button only when the text actually overflows the clamp.

### Scope

- **Modified:** `crates/kiro-control-center/src/lib/components/SkillCard.svelte`
- **No backend changes** — `SkillInfo.description` already carries the full text.
- **No new components** — the toggle lives inside the existing card.

### Alternatives considered

- **Detail panel/sidebar:** More room for metadata but heavier lift; can be added later.
- **Tooltip on hover:** Doesn't work on touch, hard to read long text in a popover.

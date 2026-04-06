# Skill Card Description Expansion — Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Allow users to see full skill descriptions in the Control Center's browse view instead of truncating to one line.

**Architecture:** Replace single-line CSS `truncate` with `line-clamp-3` and add a reactive expand/collapse toggle. Overflow detection uses `scrollHeight > clientHeight` comparison in a Svelte `$effect`.

**Tech Stack:** Svelte 5 (runes), Tailwind CSS v4

---

### Task 1: Update SkillCard with line-clamp and expand toggle

**Files:**
- Modify: `crates/kiro-control-center/src/lib/components/SkillCard.svelte`

**Step 1: Add reactive state and element binding**

Add to the `<script>` block:

```svelte
let expanded = $state(false);
let overflows = $state(false);
let descEl: HTMLParagraphElement | undefined = $state();

$effect(() => {
  if (descEl) {
    overflows = descEl.scrollHeight > descEl.clientHeight;
  }
});
```

**Step 2: Replace the description paragraph**

Change line 38 from:

```svelte
<p class="mt-1 text-sm text-kiro-text-secondary truncate">{skill.description}</p>
```

To:

```svelte
<p
  bind:this={descEl}
  class="mt-1 text-sm text-kiro-text-secondary"
  class:line-clamp-3={!expanded}
>
  {skill.description}
</p>
{#if overflows || expanded}
  <button
    type="button"
    class="mt-1 text-xs text-kiro-accent-400 hover:text-kiro-accent-300 focus:outline-none"
    onclick={(e) => { e.stopPropagation(); expanded = !expanded; }}
  >
    {expanded ? "Show less" : "Show more"}
  </button>
{/if}
```

**Step 3: Build and verify**

Run: `cd crates/kiro-control-center && npm run build`
Expected: clean build, no errors.

**Step 4: Manual test**

Run: `cd crates/kiro-control-center/src-tauri && cargo tauri dev`
- Browse to a plugin with long skill descriptions
- Verify: short descriptions show fully with no "Show more" button
- Verify: long descriptions clamp at 3 lines with "Show more"
- Click "Show more" — full text visible, button says "Show less"
- Click "Show less" — collapses back
- Verify: clicking "Show more" does NOT toggle the checkbox

**Step 5: Commit**

```bash
git add crates/kiro-control-center/src/lib/components/SkillCard.svelte
git commit -m "feat(control-center): expand/collapse long skill descriptions in SkillCard"
```

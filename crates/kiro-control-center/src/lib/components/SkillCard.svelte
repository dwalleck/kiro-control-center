<script lang="ts">
  import type { SkillInfo } from "$lib/bindings";

  let { skill, selected, onToggle }: {
    skill: SkillInfo;
    selected: boolean;
    onToggle: () => void;
  } = $props();

  let expanded = $state(false);
  let overflows = $state(false);
  let descEl: HTMLParagraphElement | undefined = $state();

  $effect(() => {
    if (descEl) {
      overflows = descEl.scrollHeight > descEl.clientHeight;
    }
  });
</script>

<button
  type="button"
  class="flex items-start gap-3 w-full text-left p-4 rounded-lg border transition-colors duration-150
    {selected
      ? 'border-kiro-accent-500 border-l-kiro-accent-400 bg-kiro-accent-900/20'
      : 'border-kiro-muted border-l-2 border-l-kiro-accent-800 bg-kiro-overlay hover:border-l-kiro-accent-500 hover:bg-kiro-accent-900/5'}
    {skill.installed ? 'opacity-60' : ''}"
  onclick={onToggle}
  disabled={skill.installed}
>
  <input
    type="checkbox"
    checked={selected}
    disabled={skill.installed}
    class="mt-1 h-4 w-4 rounded border-kiro-muted text-kiro-accent-500 focus:ring-kiro-accent-500"
    onclick={(e: MouseEvent) => e.stopPropagation()}
    onchange={onToggle}
  />
  <div class="flex-1 min-w-0">
    <div class="flex items-center gap-2">
      <span class="font-semibold text-kiro-text">{skill.name}</span>
      {#if skill.installed}
        <span class="inline-flex items-center px-2 py-0.5 text-xs font-medium rounded-full bg-kiro-success/15 text-kiro-success">
          Installed
        </span>
      {/if}
      <span class="inline-flex items-center px-2 py-0.5 text-xs font-medium rounded-full bg-kiro-info/15 text-kiro-info">
        {skill.plugin}
      </span>
    </div>
    <p
      bind:this={descEl}
      class="mt-1 text-sm text-kiro-text-secondary"
      class:line-clamp-3={!expanded}
    >
      {skill.description}
    </p>
    {#if overflows || expanded}
      <span
        role="button"
        tabindex="0"
        class="mt-1 inline-block text-xs text-kiro-accent-400 hover:text-kiro-accent-300 cursor-pointer"
        onclick={(e) => { e.stopPropagation(); expanded = !expanded; }}
        onkeydown={(e) => { if (e.key === 'Enter' || e.key === ' ') { e.stopPropagation(); e.preventDefault(); expanded = !expanded; } }}
      >
        {expanded ? "Show less" : "Show more"}
      </span>
    {/if}
  </div>
</button>

<script lang="ts">
  import type { SkillInfo } from "$lib/bindings";

  let { skill, selected, onToggle }: {
    skill: SkillInfo;
    selected: boolean;
    onToggle: () => void;
  } = $props();
</script>

<button
  type="button"
  class="flex items-start gap-3 w-full text-left p-4 rounded-lg border transition-colors duration-150
    {selected
      ? 'border-kiro-accent-500 bg-kiro-accent-900/20'
      : 'border-kiro-muted bg-kiro-overlay hover:border-kiro-subtle'}
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
    </div>
    <p class="mt-1 text-sm text-kiro-text-secondary truncate">{skill.description}</p>
  </div>
</button>

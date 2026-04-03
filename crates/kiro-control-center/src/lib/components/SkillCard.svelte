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
      ? 'border-blue-500 bg-blue-50 dark:bg-blue-900/20 dark:border-blue-400'
      : 'border-gray-200 dark:border-gray-700 bg-white dark:bg-gray-800 hover:border-gray-300 dark:hover:border-gray-600'}
    {skill.installed ? 'opacity-60' : ''}"
  onclick={onToggle}
  disabled={skill.installed}
>
  <input
    type="checkbox"
    checked={selected}
    disabled={skill.installed}
    class="mt-1 h-4 w-4 rounded border-gray-300 text-blue-600 focus:ring-blue-500"
    onclick={(e: MouseEvent) => e.stopPropagation()}
    onchange={onToggle}
  />
  <div class="flex-1 min-w-0">
    <div class="flex items-center gap-2">
      <span class="font-semibold text-gray-900 dark:text-gray-100">{skill.name}</span>
      {#if skill.installed}
        <span class="inline-flex items-center px-2 py-0.5 text-xs font-medium rounded-full bg-green-100 text-green-800 dark:bg-green-900/30 dark:text-green-400">
          Installed
        </span>
      {/if}
    </div>
    <p class="mt-1 text-sm text-gray-600 dark:text-gray-400 truncate">{skill.description}</p>
  </div>
</button>

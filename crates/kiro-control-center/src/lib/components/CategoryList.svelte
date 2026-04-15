<script lang="ts">
  let {
    categories,
    activeCategory,
    onSelect,
    matchCounts,
  }: {
    categories: { key: string; label: string; count: number }[];
    activeCategory: string;
    onSelect: (key: string) => void;
    matchCounts: Record<string, number> | null;
  } = $props();
</script>

<nav class="flex flex-col gap-0.5 p-2">
  {#each categories as cat (cat.key)}
    {@const count = matchCounts !== null ? (matchCounts[cat.key] ?? 0) : cat.count}
    {@const dimmed = matchCounts !== null && count === 0}
    <button
      type="button"
      class="w-full text-left px-3 py-2 text-sm rounded-md transition-colors duration-100 flex items-center justify-between
        {activeCategory === cat.key
          ? 'bg-kiro-muted text-kiro-text font-medium'
          : dimmed
            ? 'text-kiro-subtle/40 cursor-not-allowed'
            : 'text-kiro-text-secondary hover:bg-kiro-overlay'}"
      disabled={dimmed}
      onclick={() => !dimmed && onSelect(cat.key)}
    >
      <span class="truncate">{cat.label}</span>
      <span class="ml-2 text-xs {activeCategory === cat.key ? 'text-kiro-text-secondary' : 'text-kiro-subtle'} flex-shrink-0">
        {count}
      </span>
    </button>
  {/each}
</nav>

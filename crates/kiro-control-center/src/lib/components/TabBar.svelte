<script lang="ts">
  let { tabs, activeTab, onTabChange }: {
    tabs: string[];
    activeTab: string;
    onTabChange: (tab: string) => void;
  } = $props();
</script>

<nav class="flex border-b border-kiro-muted bg-kiro-surface">
  {#each tabs as tab (tab)}
    <button
      class="px-6 py-3 text-sm font-medium transition-colors duration-150 focus:outline-none
        {activeTab === tab
          ? 'text-kiro-accent-300 border-b-2 border-kiro-accent-400 bg-kiro-accent-900/10'
          : 'text-kiro-subtle hover:text-kiro-text-secondary hover:border-b-2 hover:border-kiro-subtle'}"
      onclick={() => onTabChange(tab)}
    >
      {tab}
    </button>
  {/each}
</nav>

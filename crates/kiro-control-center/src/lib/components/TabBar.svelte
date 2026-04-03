<script lang="ts">
  let { tabs, activeTab, onTabChange }: {
    tabs: string[];
    activeTab: string;
    onTabChange: (tab: string) => void;
  } = $props();
</script>

<nav class="flex border-b border-gray-200 dark:border-gray-700 bg-white dark:bg-gray-900">
  {#each tabs as tab (tab)}
    <button
      class="px-6 py-3 text-sm font-medium transition-colors duration-150 focus:outline-none
        {activeTab === tab
          ? 'text-blue-600 dark:text-blue-400 border-b-2 border-blue-600 dark:border-blue-400'
          : 'text-gray-500 dark:text-gray-400 hover:text-gray-700 dark:hover:text-gray-300 hover:border-b-2 hover:border-gray-300 dark:hover:border-gray-600'}"
      onclick={() => onTabChange(tab)}
    >
      {tab}
    </button>
  {/each}
</nav>

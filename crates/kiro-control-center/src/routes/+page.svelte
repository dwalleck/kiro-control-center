<script lang="ts">
  import { store, initialize } from "$lib/stores/project.svelte";
  import TabBar from "$lib/components/TabBar.svelte";
  import BrowseTab from "$lib/components/BrowseTab.svelte";
  import InstalledTab from "$lib/components/InstalledTab.svelte";
  import MarketplacesTab from "$lib/components/MarketplacesTab.svelte";
  import ProjectPicker from "$lib/components/ProjectPicker.svelte";
  import ProjectDropdown from "$lib/components/ProjectDropdown.svelte";
  import ScanRootsPanel from "$lib/components/ScanRootsPanel.svelte";

  const tabs = ["Browse", "Installed", "Marketplaces"];
  let activeTab: string = $state("Browse");
  let showManageRoots = $state(false);

  // Initialize on mount — loads settings, discovers projects, restores last project.
  $effect(() => {
    initialize();
  });
</script>

{#if store.loading}
  <div class="flex items-center justify-center h-screen bg-gray-100 dark:bg-gray-950">
    <p class="text-gray-500 dark:text-gray-400">Loading...</p>
  </div>
{:else if store.projectPath}
  <div class="flex flex-col h-screen bg-gray-100 dark:bg-gray-950 text-gray-900 dark:text-gray-100">
    <header class="flex items-center justify-between px-6 py-3 bg-white dark:bg-gray-900 border-b border-gray-200 dark:border-gray-700 shadow-sm">
      <h1 class="text-lg font-semibold">Kiro Control Center</h1>
      <ProjectDropdown onManageRoots={() => (showManageRoots = true)} />
    </header>

    <TabBar {tabs} {activeTab} onTabChange={(tab) => (activeTab = tab)} />

    <main class="flex-1 overflow-hidden">
      {#if activeTab === "Browse"}
        <BrowseTab projectPath={store.projectPath} />
      {:else if activeTab === "Installed"}
        <InstalledTab projectPath={store.projectPath} />
      {:else if activeTab === "Marketplaces"}
        <MarketplacesTab />
      {/if}
    </main>
  </div>
{:else}
  <ProjectPicker />
{/if}

{#if showManageRoots}
  <ScanRootsPanel onClose={() => (showManageRoots = false)} />
{/if}

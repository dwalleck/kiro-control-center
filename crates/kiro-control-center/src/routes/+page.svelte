<script lang="ts">
  import { commands } from "$lib/bindings";
  import type { ProjectInfo } from "$lib/bindings";
  import TabBar from "$lib/components/TabBar.svelte";
  import BrowseTab from "$lib/components/BrowseTab.svelte";
  import InstalledTab from "$lib/components/InstalledTab.svelte";
  import MarketplacesTab from "$lib/components/MarketplacesTab.svelte";

  const tabs = ["Browse", "Installed", "Marketplaces"];
  let activeTab: string = $state("Browse");

  let projectPath: string = $state(".");
  let projectInfo: ProjectInfo | null = $state(null);
  let projectError: string | null = $state(null);

  async function loadProjectInfo() {
    const result = await commands.getProjectInfo(projectPath);
    if (result.status === "ok") {
      projectInfo = result.data;
      projectPath = result.data.path;
    } else {
      projectError = result.error.message;
    }
  }

  $effect(() => {
    loadProjectInfo();
  });
</script>

<div class="flex flex-col h-screen bg-gray-100 dark:bg-gray-950 text-gray-900 dark:text-gray-100">
  <!-- Header -->
  <header class="flex items-center justify-between px-6 py-3 bg-white dark:bg-gray-900 border-b border-gray-200 dark:border-gray-700 shadow-sm">
    <h1 class="text-lg font-semibold">Kiro Control Center</h1>
    <div class="flex items-center gap-3 text-sm text-gray-500 dark:text-gray-400">
      {#if projectInfo}
        <span class="truncate max-w-md" title={projectInfo.path}>{projectInfo.path}</span>
        {#if !projectInfo.kiro_initialized}
          <span class="inline-flex items-center px-2 py-0.5 text-xs font-medium rounded-full bg-yellow-100 text-yellow-800 dark:bg-yellow-900/30 dark:text-yellow-400">
            Not initialized
          </span>
        {/if}
      {:else if projectError}
        <span class="text-red-500 dark:text-red-400 text-xs">{projectError}</span>
      {:else}
        <span>Loading...</span>
      {/if}
    </div>
  </header>

  <!-- Tabs -->
  <TabBar {tabs} {activeTab} onTabChange={(tab) => (activeTab = tab)} />

  <!-- Content -->
  <main class="flex-1 overflow-hidden">
    {#if activeTab === "Browse"}
      <BrowseTab {projectPath} />
    {:else if activeTab === "Installed"}
      <InstalledTab {projectPath} />
    {:else if activeTab === "Marketplaces"}
      <MarketplacesTab />
    {/if}
  </main>
</div>

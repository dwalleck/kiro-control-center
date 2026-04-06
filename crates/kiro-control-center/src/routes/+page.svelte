<script lang="ts">
  import { onMount } from "svelte";
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

  // Initialize once on mount. Uses onMount (not $effect) because initialize()
  // is a one-shot async function that should not re-trigger on state changes.
  onMount(() => {
    initialize().catch((e) => {
      console.error("Initialization failed:", e);
      store.projectError = "Application failed to initialize. Please restart.";
      store.loading = false;
    });
  });
</script>

{#if store.loading}
  <div class="flex items-center justify-center h-screen bg-kiro-base">
    <p class="text-kiro-subtle">Loading...</p>
  </div>
{:else if store.projectPath}
  <div class="flex flex-col h-screen bg-kiro-base text-kiro-text">
    <header class="flex items-center justify-between px-6 py-3 bg-kiro-surface border-b border-kiro-muted shadow-sm">
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

{#if store.projectError}
  <div class="fixed bottom-4 right-4 z-50 max-w-md px-4 py-3 rounded-lg bg-kiro-error/10 border border-kiro-error/30 shadow-lg">
    <p class="text-sm text-kiro-error">{store.projectError}</p>
    <button
      class="mt-1 text-xs text-kiro-error/70 hover:text-kiro-error"
      onclick={() => (store.projectError = null)}
    >
      Dismiss
    </button>
  </div>
{/if}

{#if showManageRoots}
  <ScanRootsPanel onClose={() => (showManageRoots = false)} />
{/if}

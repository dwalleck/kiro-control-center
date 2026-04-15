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
  import SettingsView from "$lib/components/SettingsView.svelte";

  const tabs = ["Browse", "Installed", "Marketplaces"];
  let activeTab: string = $state("Browse");
  let showManageRoots = $state(false);
  let showSettings = $state(false);

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
    <header class="flex items-center justify-between px-6 py-3 bg-kiro-surface border-b-2 border-kiro-accent-700 shadow-sm">
      <div class="flex items-center gap-3">
        <h1 class="text-lg font-semibold">Kiro Control Center</h1>
        <button
          type="button"
          aria-label="Open settings"
          onclick={() => (showSettings = true)}
          class="p-1.5 rounded-md text-kiro-subtle hover:text-kiro-text-secondary hover:bg-kiro-overlay transition-colors"
        >
          <svg class="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M10.325 4.317c.426-1.756 2.924-1.756 3.35 0a1.724 1.724 0 002.573 1.066c1.543-.94 3.31.826 2.37 2.37a1.724 1.724 0 001.066 2.573c1.756.426 1.756 2.924 0 3.35a1.724 1.724 0 00-1.066 2.573c.94 1.543-.826 3.31-2.37 2.37a1.724 1.724 0 00-2.573 1.066c-.426 1.756-2.924 1.756-3.35 0a1.724 1.724 0 00-2.573-1.066c-1.543.94-3.31-.826-2.37-2.37a1.724 1.724 0 00-1.066-2.573c-1.756-.426-1.756-2.924 0-3.35a1.724 1.724 0 001.066-2.573c-.94-1.543.826-3.31 2.37-2.37.996.608 2.296.07 2.572-1.065z" />
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M15 12a3 3 0 11-6 0 3 3 0 016 0z" />
          </svg>
        </button>
      </div>
      {#if !showSettings}
        <ProjectDropdown onManageRoots={() => (showManageRoots = true)} />
      {/if}
    </header>

    {#if showSettings}
      <SettingsView onClose={() => (showSettings = false)} />
    {:else}
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
    {/if}
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

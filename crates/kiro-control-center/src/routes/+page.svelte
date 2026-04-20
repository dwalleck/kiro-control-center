<script lang="ts">
  import { onMount } from "svelte";
  import { SvelteMap } from "svelte/reactivity";
  import { store, initialize } from "$lib/stores/project.svelte";
  import { commands } from "$lib/bindings";
  import type { SettingEntry } from "$lib/bindings";
  import type { Tab, SettingCategory } from "$lib/types";
  import NavRail from "$lib/components/NavRail.svelte";
  import BrowseTab from "$lib/components/BrowseTab.svelte";
  import InstalledTab from "$lib/components/InstalledTab.svelte";
  import MarketplacesTab from "$lib/components/MarketplacesTab.svelte";
  import ProjectPicker from "$lib/components/ProjectPicker.svelte";
  import ProjectDropdown from "$lib/components/ProjectDropdown.svelte";
  import ScanRootsPanel from "$lib/components/ScanRootsPanel.svelte";
  import SettingsView from "$lib/components/SettingsView.svelte";

  let activeTab: Tab = $state("Browse");
  let settingsCategory: string | null = $state(null);
  let showManageRoots = $state(false);

  let allEntries: SettingEntry[] = $state([]);
  let settingsLoading: boolean = $state(true);
  let settingsLoadError: string | null = $state(null);

  let categories: SettingCategory[] = $derived.by(() => {
    const seen = new SvelteMap<string, SettingCategory>();
    for (const entry of allEntries) {
      const existing = seen.get(entry.category);
      if (existing) {
        existing.count += 1;
      } else {
        seen.set(entry.category, { key: entry.category, label: entry.category_label, count: 1 });
      }
    }
    return Array.from(seen.values());
  });

  // Seed or repair the active category: pick the first when unset, or when the
  // prior selection is no longer in the loaded set (backend category removed).
  $effect(() => {
    if (categories.length === 0) return;
    const keep = settingsCategory !== null && categories.some((c) => c.key === settingsCategory);
    if (!keep) settingsCategory = categories[0].key;
  });

  function handleSettingsUpdate(updated: SettingEntry) {
    allEntries = allEntries.map((e) => (e.key === updated.key ? updated : e));
  }

  async function loadKiroSettings() {
    try {
      const result = await commands.getKiroSettings();
      if (result.status === "ok") {
        allEntries = result.data;
      } else {
        settingsLoadError = result.error.message;
      }
    } catch (e) {
      settingsLoadError = e instanceof Error
        ? `Failed to load settings: ${e.message}`
        : "Failed to load settings due to an unexpected error.";
    } finally {
      settingsLoading = false;
    }
  }

  // Initialize once on mount. Uses onMount (not $effect) because initialize()
  // is a one-shot async function that should not re-trigger on state changes.
  onMount(() => {
    initialize().catch((e) => {
      console.error("Initialization failed:", e);
      const reason = e instanceof Error ? e.message : String(e);
      store.projectError = `Application failed to initialize: ${reason}`;
      store.loading = false;
    });
    loadKiroSettings();
  });
</script>

{#if store.loading}
  <div class="flex items-center justify-center h-screen bg-kiro-base">
    <p class="text-kiro-subtle">Loading...</p>
  </div>
{:else if store.projectPath}
  <div class="flex flex-col h-screen bg-kiro-base text-kiro-text">
    <header class="flex items-center justify-between px-6 py-3 bg-kiro-surface border-b-2 border-kiro-accent-700 shadow-sm">
      <h1 class="text-lg font-semibold">Kiro Control Center</h1>
      <ProjectDropdown onManageRoots={() => (showManageRoots = true)} />
    </header>

    <div class="flex flex-1 min-h-0 overflow-hidden">
      <NavRail
        {activeTab}
        onTabChange={(t) => (activeTab = t)}
        {settingsCategory}
        onSettingsCategoryChange={(k) => (settingsCategory = k)}
        {categories}
      />

      <main class="flex-1 overflow-hidden">
        {#if activeTab === "Browse"}
          <BrowseTab projectPath={store.projectPath} />
        {:else if activeTab === "Installed"}
          <InstalledTab projectPath={store.projectPath} />
        {:else if activeTab === "Marketplaces"}
          <MarketplacesTab />
        {:else if activeTab === "Kiro Settings"}
          <SettingsView
            {allEntries}
            loading={settingsLoading}
            loadError={settingsLoadError}
            activeCategory={settingsCategory}
            onUpdate={handleSettingsUpdate}
          />
        {/if}
      </main>
    </div>
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

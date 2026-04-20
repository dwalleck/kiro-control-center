<script lang="ts">
  import type { SettingEntry } from "$lib/bindings";
  import SettingsPanel from "./SettingsPanel.svelte";

  let {
    allEntries,
    loading,
    loadError,
    activeCategory,
    onUpdate,
  }: {
    allEntries: SettingEntry[];
    loading: boolean;
    loadError: string | null;
    activeCategory: string | null;
    onUpdate: (entry: SettingEntry) => void;
  } = $props();

  let searchQuery: string = $state("");

  let displayEntries = $derived.by(() => {
    const q = searchQuery.trim().toLowerCase();
    if (q) {
      return allEntries.filter(
        (e) =>
          e.label.toLowerCase().includes(q) ||
          e.description.toLowerCase().includes(q) ||
          e.key.toLowerCase().includes(q)
      );
    }
    return allEntries.filter((e) => e.category === activeCategory);
  });

  let showCategoryHeaders = $derived(searchQuery.trim().length > 0);
</script>

<div class="flex flex-col h-full bg-kiro-base text-kiro-text">
  <div class="flex items-center gap-4 px-6 py-3 bg-kiro-surface border-b border-kiro-muted shadow-sm">
    <h1 class="text-sm font-semibold text-kiro-text">Kiro Settings</h1>
    <div class="flex-1 max-w-sm ml-auto">
      <input
        type="search"
        placeholder="Search settings..."
        bind:value={searchQuery}
        class="w-full px-3 py-1.5 text-sm rounded-md border border-kiro-muted bg-kiro-overlay text-kiro-text placeholder-kiro-subtle focus:outline-none focus:ring-2 focus:ring-kiro-accent-500 focus:border-transparent"
      />
    </div>
  </div>

  {#if loading}
    <div class="flex items-center justify-center flex-1 text-kiro-subtle">
      <p class="text-sm">Loading settings...</p>
    </div>
  {:else if loadError}
    <div class="flex items-center justify-center flex-1">
      <div class="px-4 py-3 rounded-md bg-kiro-error/10 border border-kiro-error/30">
        <p class="text-sm text-kiro-error">{loadError}</p>
      </div>
    </div>
  {:else if allEntries.length === 0}
    <div class="flex items-center justify-center flex-1 text-kiro-subtle">
      <p class="text-sm">No settings available.</p>
    </div>
  {:else}
    <div class="flex flex-1 overflow-hidden">
      <SettingsPanel
        entries={displayEntries}
        {showCategoryHeaders}
        {onUpdate}
      />
    </div>
  {/if}
</div>

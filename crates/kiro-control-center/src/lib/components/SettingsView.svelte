<script lang="ts">
  import { onMount } from "svelte";
  import { SvelteMap } from "svelte/reactivity";
  import { commands } from "$lib/bindings";
  import type { SettingEntry } from "$lib/bindings";
  import CategoryList from "./CategoryList.svelte";
  import SettingsPanel from "./SettingsPanel.svelte";

  let { onClose }: { onClose: () => void } = $props();

  let allEntries: SettingEntry[] = $state([]);
  let loading: boolean = $state(true);
  let loadError: string | null = $state(null);
  let searchQuery: string = $state("");
  let _activeCategoryOverride: string | null = $state(null);

  let categories = $derived.by(() => {
    const seen = new SvelteMap<string, { key: string; label: string; count: number }>();
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

  // Derive the active category: use the user's choice if set, otherwise default to first category
  let activeCategory = $derived(
    _activeCategoryOverride !== null ? _activeCategoryOverride : (categories[0]?.key ?? "")
  );

  function setActiveCategory(key: string) {
    _activeCategoryOverride = key;
  }

  let filteredEntries = $derived.by(() => {
    if (!searchQuery.trim()) return [];
    const q = searchQuery.toLowerCase();
    return allEntries.filter(
      (e) =>
        e.label.toLowerCase().includes(q) ||
        e.description.toLowerCase().includes(q) ||
        e.key.toLowerCase().includes(q)
    );
  });

  let matchCounts = $derived.by((): Record<string, number> | null => {
    if (!searchQuery.trim()) return null;
    const counts: Record<string, number> = {};
    for (const entry of filteredEntries) {
      counts[entry.category] = (counts[entry.category] ?? 0) + 1;
    }
    return counts;
  });

  let displayEntries = $derived.by(() => {
    if (searchQuery.trim()) return filteredEntries;
    return allEntries.filter((e) => e.category === activeCategory);
  });

  let showCategoryHeaders = $derived(searchQuery.trim().length > 0);

  function handleUpdate(updated: SettingEntry) {
    allEntries = allEntries.map((e) => (e.key === updated.key ? updated : e));
  }

  onMount(async () => {
    const result = await commands.getKiroSettings();
    if (result.status === "ok") {
      allEntries = result.data;
    } else {
      loadError = result.error.message;
    }
    loading = false;
  });
</script>

<div class="flex flex-col h-full bg-kiro-base text-kiro-text">
  <!-- Top bar -->
  <div class="flex items-center gap-4 px-6 py-3 bg-kiro-surface border-b border-kiro-muted shadow-sm">
    <button
      type="button"
      onclick={onClose}
      class="flex items-center gap-1.5 text-sm text-kiro-text-secondary hover:text-kiro-text transition-colors"
    >
      <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M15 19l-7-7 7-7" />
      </svg>
      Back
    </button>

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

  <!-- Body -->
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
  {:else}
    <div class="flex flex-1 overflow-hidden">
      <!-- Sidebar -->
      <div class="w-56 flex-shrink-0 border-r border-kiro-muted bg-kiro-surface overflow-y-auto">
        <CategoryList
          {categories}
          {activeCategory}
          onSelect={setActiveCategory}
          {matchCounts}
        />
      </div>

      <!-- Main panel -->
      <SettingsPanel
        entries={displayEntries}
        {showCategoryHeaders}
        onUpdate={handleUpdate}
      />
    </div>
  {/if}
</div>

<script lang="ts">
  import { commands } from "$lib/bindings";
  import type { MarketplaceInfo, PluginInfo, SkillInfo } from "$lib/bindings";
  import { SvelteSet } from "svelte/reactivity";
  import SkillCard from "./SkillCard.svelte";

  let { projectPath }: { projectPath: string } = $props();

  let marketplaces: MarketplaceInfo[] = $state([]);
  let pluginsByMarketplace: Record<string, PluginInfo[]> = $state({});
  let expandedMarketplace: string | null = $state(null);
  let selectedMarketplace: string | null = $state(null);
  let selectedPlugin: string | null = $state(null);

  let skills: SkillInfo[] = $state([]);
  let selectedSkills = new SvelteSet<string>();
  let filterText: string = $state("");
  let forceInstall: boolean = $state(false);

  let loadingMarketplaces: boolean = $state(false);
  let loadingPlugins: string | null = $state(null);
  let loadingSkills: boolean = $state(false);
  let installing: boolean = $state(false);

  let error: string | null = $state(null);
  let installMessage: string | null = $state(null);

  let filteredSkills = $derived(
    skills.filter(
      (s) =>
        s.name.toLowerCase().includes(filterText.toLowerCase()) ||
        s.description.toLowerCase().includes(filterText.toLowerCase())
    )
  );

  let selectedCount = $derived(selectedSkills.size);

  async function loadMarketplaces() {
    loadingMarketplaces = true;
    error = null;
    const result = await commands.listMarketplaces();
    if (result.status === "ok") {
      marketplaces = result.data;
    } else {
      error = result.error.message;
    }
    loadingMarketplaces = false;
  }

  async function toggleMarketplace(name: string) {
    if (expandedMarketplace === name) {
      expandedMarketplace = null;
      return;
    }
    expandedMarketplace = name;

    if (pluginsByMarketplace[name]) return;

    loadingPlugins = name;
    const result = await commands.listPlugins(name);
    if (result.status === "ok") {
      pluginsByMarketplace[name] = result.data;
    } else {
      error = result.error.message;
    }
    loadingPlugins = null;
  }

  async function selectPlugin(marketplace: string, plugin: string) {
    selectedMarketplace = marketplace;
    selectedPlugin = plugin;
    selectedSkills.clear();
    loadingSkills = true;
    error = null;
    installMessage = null;

    const result = await commands.listAvailableSkills(marketplace, plugin, projectPath);
    if (result.status === "ok") {
      skills = result.data;
    } else {
      error = result.error.message;
      skills = [];
    }
    loadingSkills = false;
  }

  function toggleSkill(name: string) {
    if (selectedSkills.has(name)) {
      selectedSkills.delete(name);
    } else {
      selectedSkills.add(name);
    }
  }

  async function installSelected() {
    if (!selectedMarketplace || !selectedPlugin || selectedSkills.size === 0) return;
    // Capture non-null values before await to satisfy TypeScript control flow
    const mp = selectedMarketplace;
    const pl = selectedPlugin;
    installing = true;
    error = null;
    installMessage = null;

    const result = await commands.installSkills(
      mp,
      pl,
      Array.from(selectedSkills),
      forceInstall,
      projectPath
    );

    if (result.status === "ok") {
      const { installed, skipped, failed } = result.data;
      const parts: string[] = [];
      if (installed.length > 0) parts.push(`Installed: ${installed.join(", ")}`);
      if (skipped.length > 0) parts.push(`Skipped: ${skipped.join(", ")}`);
      if (failed.length > 0) parts.push(`Failed: ${failed.map((f) => `${f.name} (${f.error})`).join(", ")}`);
      installMessage = parts.join(" | ");
      selectedSkills.clear();

      await selectPlugin(mp, pl);
    } else {
      error = result.error.message;
    }
    installing = false;
  }

  $effect(() => {
    loadMarketplaces();
  });
</script>

<div class="flex h-full">
  <!-- Sidebar -->
  <div class="w-64 flex-shrink-0 border-r border-kiro-muted bg-kiro-surface overflow-y-auto">
    <div class="p-3">
      <h3 class="text-xs font-semibold text-kiro-subtle uppercase tracking-wider mb-2">
        Marketplaces
      </h3>
      {#if loadingMarketplaces}
        <p class="text-sm text-kiro-subtle px-2 animate-pulse">Loading...</p>
      {:else if marketplaces.length === 0}
        <p class="text-sm text-kiro-subtle px-2">No marketplaces found</p>
      {:else}
        {#each marketplaces as mp (mp.name)}
          <div class="mb-1">
            <button
              class="w-full text-left px-3 py-2 text-sm rounded-md transition-colors duration-100
                {expandedMarketplace === mp.name
                  ? 'bg-kiro-muted text-kiro-text'
                  : 'text-kiro-text-secondary hover:bg-kiro-overlay'}"
              onclick={() => toggleMarketplace(mp.name)}
            >
              <span class="flex items-center justify-between">
                <span class="flex items-center gap-2 truncate">
                  <span class="w-2 h-2 rounded-full flex-shrink-0 {
                    mp.source_type === 'github' ? 'bg-kiro-info' :
                    mp.source_type === 'local' ? 'bg-kiro-warning' :
                    'bg-kiro-accent-400'
                  }"></span>
                  <span class="truncate font-medium">{mp.name}</span>
                </span>
                <span class="text-xs text-kiro-subtle">{mp.plugin_count}</span>
              </span>
            </button>

            {#if expandedMarketplace === mp.name}
              <div class="ml-3 mt-1 space-y-0.5">
                {#if loadingPlugins === mp.name}
                  <p class="text-xs text-kiro-subtle px-3 py-1">Loading plugins...</p>
                {:else if pluginsByMarketplace[mp.name]}
                  {#each pluginsByMarketplace[mp.name] as plugin (plugin.name)}
                    <button
                      class="w-full text-left px-3 py-1.5 text-sm rounded-md transition-colors duration-100
                        {selectedMarketplace === mp.name && selectedPlugin === plugin.name
                          ? 'bg-kiro-accent-900/30 text-kiro-accent-300'
                          : 'text-kiro-text-secondary hover:bg-kiro-overlay'}"
                      onclick={() => selectPlugin(mp.name, plugin.name)}
                    >
                      <span class="flex items-center justify-between">
                        <span class="truncate">{plugin.name}</span>
                        <span class="text-xs text-kiro-subtle">{plugin.skill_count}</span>
                      </span>
                    </button>
                  {/each}
                {/if}
              </div>
            {/if}
          </div>
        {/each}
      {/if}
    </div>
  </div>

  <!-- Main area -->
  <div class="flex-1 flex flex-col min-w-0">
    <!-- Search bar -->
    <div class="p-4 border-b border-kiro-muted">
      <input
        type="text"
        placeholder="Filter skills by name or description..."
        bind:value={filterText}
        class="w-full px-3 py-2 text-sm rounded-md border border-kiro-muted bg-kiro-overlay text-kiro-text placeholder-kiro-subtle focus:outline-none focus:ring-2 focus:ring-kiro-accent-500 focus:border-transparent"
      />
    </div>

    <!-- Error display -->
    {#if error}
      <div class="mx-4 mt-3 px-4 py-3 rounded-md bg-kiro-error/10 border border-kiro-error/30">
        <p class="text-sm text-kiro-error">{error}</p>
      </div>
    {/if}

    <!-- Install success message -->
    {#if installMessage}
      <div class="mx-4 mt-3 px-4 py-3 rounded-md bg-kiro-success/10 border border-kiro-success/30">
        <p class="text-sm text-kiro-success">{installMessage}</p>
      </div>
    {/if}

    <!-- Skills content -->
    <div class="flex-1 overflow-y-auto p-4">
      {#if !selectedPlugin}
        <div class="flex flex-col items-center justify-center h-full text-kiro-subtle gap-3">
          <svg class="w-10 h-10 text-kiro-accent-800" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="1.5"
              d="M20 7l-8-4-8 4m16 0l-8 4m8-4v10l-8 4m0-10L4 7m8 4v10M4 7v10l8 4" />
          </svg>
          <p class="text-sm">Select a plugin from the sidebar to browse skills</p>
        </div>
      {:else if loadingSkills}
        <div class="flex flex-col items-center justify-center h-full text-kiro-subtle gap-3">
          <svg class="w-8 h-8 text-kiro-accent-800 animate-pulse" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="1.5"
              d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15" />
          </svg>
          <p class="text-sm">Loading skills...</p>
        </div>
      {:else if filteredSkills.length === 0}
        <div class="flex flex-col items-center justify-center h-full text-kiro-subtle gap-3">
          <svg class="w-10 h-10 text-kiro-accent-800" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="1.5"
              d="M21 21l-6-6m2-5a7 7 0 11-14 0 7 7 0 0114 0z" />
          </svg>
          <p class="text-sm">
            {filterText ? "No skills match the filter" : "No skills available"}
          </p>
        </div>
      {:else}
        <div class="grid gap-3 grid-cols-1 lg:grid-cols-2">
          {#each filteredSkills as skill (skill.name)}
            <SkillCard
              {skill}
              selected={selectedSkills.has(skill.name)}
              onToggle={() => toggleSkill(skill.name)}
            />
          {/each}
        </div>
      {/if}
    </div>

    <!-- Bottom bar -->
    {#if selectedPlugin}
      <div class="p-4 border-t border-kiro-muted bg-kiro-surface flex items-center justify-between">
        <label class="flex items-center gap-2 text-sm text-kiro-text-secondary">
          <input
            type="checkbox"
            bind:checked={forceInstall}
            class="h-4 w-4 rounded border-kiro-muted text-kiro-accent-500 focus:ring-kiro-accent-500"
          />
          Force reinstall
        </label>
        <button
          class="px-4 py-2 text-sm font-medium rounded-md text-white transition-colors duration-150
            {selectedCount > 0 && !installing
              ? 'bg-kiro-accent-600 hover:bg-kiro-accent-700'
              : 'bg-kiro-muted text-kiro-subtle cursor-not-allowed'}"
          disabled={selectedCount === 0 || installing}
          onclick={installSelected}
        >
          {installing ? "Installing..." : `Install ${selectedCount} selected`}
        </button>
      </div>
    {/if}
  </div>
</div>

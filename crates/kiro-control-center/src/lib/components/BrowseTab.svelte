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
  <div class="w-64 flex-shrink-0 border-r border-gray-200 dark:border-gray-700 bg-gray-50 dark:bg-gray-900 overflow-y-auto">
    <div class="p-3">
      <h3 class="text-xs font-semibold text-gray-500 dark:text-gray-400 uppercase tracking-wider mb-2">
        Marketplaces
      </h3>
      {#if loadingMarketplaces}
        <p class="text-sm text-gray-400 dark:text-gray-500 px-2">Loading...</p>
      {:else if marketplaces.length === 0}
        <p class="text-sm text-gray-400 dark:text-gray-500 px-2">No marketplaces found</p>
      {:else}
        {#each marketplaces as mp (mp.name)}
          <div class="mb-1">
            <button
              class="w-full text-left px-3 py-2 text-sm rounded-md transition-colors duration-100
                {expandedMarketplace === mp.name
                  ? 'bg-gray-200 dark:bg-gray-700 text-gray-900 dark:text-gray-100'
                  : 'text-gray-700 dark:text-gray-300 hover:bg-gray-100 dark:hover:bg-gray-800'}"
              onclick={() => toggleMarketplace(mp.name)}
            >
              <span class="flex items-center justify-between">
                <span class="truncate font-medium">{mp.name}</span>
                <span class="text-xs text-gray-400">{mp.plugin_count}</span>
              </span>
            </button>

            {#if expandedMarketplace === mp.name}
              <div class="ml-3 mt-1 space-y-0.5">
                {#if loadingPlugins === mp.name}
                  <p class="text-xs text-gray-400 px-3 py-1">Loading plugins...</p>
                {:else if pluginsByMarketplace[mp.name]}
                  {#each pluginsByMarketplace[mp.name] as plugin (plugin.name)}
                    <button
                      class="w-full text-left px-3 py-1.5 text-sm rounded-md transition-colors duration-100
                        {selectedMarketplace === mp.name && selectedPlugin === plugin.name
                          ? 'bg-blue-100 dark:bg-blue-900/30 text-blue-700 dark:text-blue-300'
                          : 'text-gray-600 dark:text-gray-400 hover:bg-gray-100 dark:hover:bg-gray-800'}"
                      onclick={() => selectPlugin(mp.name, plugin.name)}
                    >
                      <span class="flex items-center justify-between">
                        <span class="truncate">{plugin.name}</span>
                        <span class="text-xs text-gray-400">{plugin.skill_count}</span>
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
    <div class="p-4 border-b border-gray-200 dark:border-gray-700">
      <input
        type="text"
        placeholder="Filter skills by name or description..."
        bind:value={filterText}
        class="w-full px-3 py-2 text-sm rounded-md border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-800 text-gray-900 dark:text-gray-100 placeholder-gray-400 dark:placeholder-gray-500 focus:outline-none focus:ring-2 focus:ring-blue-500 focus:border-transparent"
      />
    </div>

    <!-- Error display -->
    {#if error}
      <div class="mx-4 mt-3 px-4 py-3 rounded-md bg-red-50 dark:bg-red-900/20 border border-red-200 dark:border-red-800">
        <p class="text-sm text-red-700 dark:text-red-400">{error}</p>
      </div>
    {/if}

    <!-- Install success message -->
    {#if installMessage}
      <div class="mx-4 mt-3 px-4 py-3 rounded-md bg-green-50 dark:bg-green-900/20 border border-green-200 dark:border-green-800">
        <p class="text-sm text-green-700 dark:text-green-400">{installMessage}</p>
      </div>
    {/if}

    <!-- Skills content -->
    <div class="flex-1 overflow-y-auto p-4">
      {#if !selectedPlugin}
        <div class="flex items-center justify-center h-full text-gray-400 dark:text-gray-500">
          <p class="text-sm">Select a plugin from the sidebar to browse skills</p>
        </div>
      {:else if loadingSkills}
        <div class="flex items-center justify-center h-full text-gray-400 dark:text-gray-500">
          <p class="text-sm">Loading skills...</p>
        </div>
      {:else if filteredSkills.length === 0}
        <div class="flex items-center justify-center h-full text-gray-400 dark:text-gray-500">
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
      <div class="p-4 border-t border-gray-200 dark:border-gray-700 bg-gray-50 dark:bg-gray-900 flex items-center justify-between">
        <label class="flex items-center gap-2 text-sm text-gray-600 dark:text-gray-400">
          <input
            type="checkbox"
            bind:checked={forceInstall}
            class="h-4 w-4 rounded border-gray-300 text-blue-600 focus:ring-blue-500"
          />
          Force reinstall
        </label>
        <button
          class="px-4 py-2 text-sm font-medium rounded-md text-white transition-colors duration-150
            {selectedCount > 0 && !installing
              ? 'bg-blue-600 hover:bg-blue-700 dark:bg-blue-500 dark:hover:bg-blue-600'
              : 'bg-gray-300 dark:bg-gray-700 cursor-not-allowed'}"
          disabled={selectedCount === 0 || installing}
          onclick={installSelected}
        >
          {installing ? "Installing..." : `Install ${selectedCount} selected`}
        </button>
      </div>
    {/if}
  </div>
</div>

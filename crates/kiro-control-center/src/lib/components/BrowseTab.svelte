<script lang="ts">
  import { onMount } from "svelte";
  import { SvelteSet } from "svelte/reactivity";
  import { commands } from "$lib/bindings";
  import type { MarketplaceInfo, PluginInfo, SkillInfo } from "$lib/bindings";
  import SkillCard from "./SkillCard.svelte";

  let { projectPath }: { projectPath: string } = $props();

  // Composite-key helpers. The ASCII Unit Separator (\u001f) is reserved for
  // exactly this purpose and cannot occur in marketplace/plugin/skill names,
  // so it never collides the way "/" or ":" would.
  const DELIM = "\u001f";
  const pluginKey = (mp: string, plugin: string) => `${mp}${DELIM}${plugin}`;
  const skillKey = (mp: string, plugin: string, name: string) =>
    `${mp}${DELIM}${plugin}${DELIM}${name}`;
  const parsePluginKey = (key: string) => {
    const [marketplace, plugin] = key.split(DELIM);
    return { marketplace, plugin };
  };
  const parseSkillKey = (key: string) => {
    const [marketplace, plugin, name] = key.split(DELIM);
    return { marketplace, plugin, name };
  };

  let marketplaces: MarketplaceInfo[] = $state([]);
  let pluginsByMarketplace: Record<string, PluginInfo[]> = $state({});
  let skillsByPluginPair: Record<string, SkillInfo[]> = $state({});

  let selectedMarketplaces = new SvelteSet<string>();
  let selectedPlugins = new SvelteSet<string>();
  let selectedSkills = new SvelteSet<string>();
  let installedOnly: boolean = $state(false);
  let filterText: string = $state("");
  let forceInstall: boolean = $state(false);
  let popoverOpen: boolean = $state(false);
  let popRef: HTMLDivElement | undefined = $state();

  let loadingMarketplaces: boolean = $state(false);
  let pendingPluginFetches = new SvelteSet<string>();
  let pendingSkillFetches = new SvelteSet<string>();
  let installing: boolean = $state(false);

  // Per-source fetch errors, keyed by an origin-tagged string. Successful
  // fetches only clear their own entry, so concurrent failures from
  // independent sources don't overwrite each other in a race. Install
  // failures use their own `installError` because they're user-initiated
  // and deserve prominent, independent clearing semantics.
  let fetchErrors: Record<string, string> = $state({});
  let installError: string | null = $state(null);
  let installMessage: string | null = $state(null);

  const ERR_MARKETPLACES = "marketplaces";
  const pluginsErrKey = (mp: string) => `plugins:${mp}`;
  const skillsErrKey = (mp: string, plugin: string) => `skills:${pluginKey(mp, plugin)}`;

  let availablePlugins = $derived.by(() => {
    const out: { marketplace: string; plugin: PluginInfo }[] = [];
    for (const mp of selectedMarketplaces) {
      const list = pluginsByMarketplace[mp] ?? [];
      for (const plugin of list) out.push({ marketplace: mp, plugin });
    }
    return out;
  });

  let skills = $derived.by(() => {
    const rows: SkillInfo[] = [];
    for (const mp of selectedMarketplaces) {
      const list = pluginsByMarketplace[mp] ?? [];
      for (const pl of list) {
        if (selectedPlugins.size > 0 && !selectedPlugins.has(pluginKey(mp, pl.name))) continue;
        const pairSkills = skillsByPluginPair[pluginKey(mp, pl.name)] ?? [];
        rows.push(...pairSkills);
      }
    }
    return rows;
  });

  let filteredSkills = $derived.by(() => {
    let rows: SkillInfo[] = skills;
    if (installedOnly) rows = rows.filter((s) => s.installed);
    const q = filterText.trim().toLowerCase();
    if (q) {
      rows = rows.filter(
        (s) =>
          s.name.toLowerCase().includes(q) ||
          s.description.toLowerCase().includes(q)
      );
    }
    return rows;
  });

  let activeFilterCount = $derived(
    selectedMarketplaces.size + selectedPlugins.size + (installedOnly ? 1 : 0)
  );

  // Spinner only when no skills yet — avoids flicker when toggling filters
  // while previously-fetched skills are still on screen.
  let showLoadingSpinner = $derived(
    skills.length === 0 &&
      (loadingMarketplaces || pendingPluginFetches.size > 0 || pendingSkillFetches.size > 0)
  );

  let initialLoadFailed = $derived(
    fetchErrors[ERR_MARKETPLACES] !== undefined &&
      marketplaces.length === 0 &&
      !loadingMarketplaces
  );

  let selectedCount = $derived(selectedSkills.size);

  async function loadMarketplaces() {
    loadingMarketplaces = true;
    try {
      const result = await commands.listMarketplaces();
      if (result.status === "ok") {
        marketplaces = result.data;
        if (marketplaces.length > 0 && selectedMarketplaces.size === 0) {
          selectedMarketplaces.add(marketplaces[0].name);
        }
        delete fetchErrors[ERR_MARKETPLACES];
      } else {
        fetchErrors[ERR_MARKETPLACES] = result.error.message;
      }
    } catch (e) {
      fetchErrors[ERR_MARKETPLACES] = e instanceof Error ? e.message : String(e);
    } finally {
      loadingMarketplaces = false;
    }
  }

  async function fetchPluginsFor(mp: string) {
    if (pendingPluginFetches.has(mp) || pluginsByMarketplace[mp]) return;
    pendingPluginFetches.add(mp);
    const errKey = pluginsErrKey(mp);
    try {
      const result = await commands.listPlugins(mp);
      if (result.status === "ok") {
        pluginsByMarketplace[mp] = result.data;
        delete fetchErrors[errKey];
      } else {
        fetchErrors[errKey] = `${mp}: ${result.error.message}`;
      }
    } catch (e) {
      const reason = e instanceof Error ? e.message : String(e);
      fetchErrors[errKey] = `${mp}: ${reason}`;
    } finally {
      pendingPluginFetches.delete(mp);
    }
  }

  async function fetchSkillsFor(mp: string, plugin: string, force = false) {
    const key = pluginKey(mp, plugin);
    if (!force && (pendingSkillFetches.has(key) || skillsByPluginPair[key])) return;
    pendingSkillFetches.add(key);
    const errKey = skillsErrKey(mp, plugin);
    try {
      const result = await commands.listAvailableSkills(mp, plugin, projectPath);
      if (result.status === "ok") {
        skillsByPluginPair[key] = result.data;
        delete fetchErrors[errKey];
      } else {
        fetchErrors[errKey] = `${mp}/${plugin}: ${result.error.message}`;
      }
    } catch (e) {
      const reason = e instanceof Error ? e.message : String(e);
      fetchErrors[errKey] = `${mp}/${plugin}: ${reason}`;
    } finally {
      pendingSkillFetches.delete(key);
    }
  }

  $effect(() => {
    for (const mp of selectedMarketplaces) fetchPluginsFor(mp);
  });

  $effect(() => {
    for (const mp of selectedMarketplaces) {
      const list = pluginsByMarketplace[mp] ?? [];
      for (const pl of list) {
        if (selectedPlugins.size > 0 && !selectedPlugins.has(pluginKey(mp, pl.name))) continue;
        fetchSkillsFor(mp, pl.name);
      }
    }
  });

  // Skill caches are project-scoped — `installed` flags flip meaning when
  // projectPath changes, so invalidate everything and drop pending selections.
  let priorProjectPath: string | null = null;
  $effect(() => {
    if (priorProjectPath !== null && priorProjectPath !== projectPath) {
      skillsByPluginPair = {};
      selectedSkills.clear();
    }
    priorProjectPath = projectPath;
  });

  // Drop selected-skill keys that no longer refer to a visible skill — happens
  // when the user deselects a marketplace that contained the selected skills.
  $effect(() => {
    const valid = new Set(skills.map((s) => skillKey(s.marketplace, s.plugin, s.name)));
    for (const key of selectedSkills) {
      if (!valid.has(key)) selectedSkills.delete(key);
    }
  });

  function toggleMarketplace(name: string) {
    if (selectedMarketplaces.has(name)) selectedMarketplaces.delete(name);
    else selectedMarketplaces.add(name);
    // Plugin keys embed their marketplace; clearing keeps the set meaningful.
    selectedPlugins.clear();
  }

  function togglePlugin(key: string) {
    if (selectedPlugins.has(key)) selectedPlugins.delete(key);
    else selectedPlugins.add(key);
  }

  function toggleSkill(key: string) {
    if (selectedSkills.has(key)) selectedSkills.delete(key);
    else selectedSkills.add(key);
  }

  function resetFilters() {
    selectedMarketplaces.clear();
    selectedPlugins.clear();
    installedOnly = false;
    if (marketplaces.length > 0) selectedMarketplaces.add(marketplaces[0].name);
  }

  async function installSelected() {
    if (selectedSkills.size === 0) return;
    installing = true;
    installError = null;
    installMessage = null;

    type Group = { marketplace: string; plugin: string; names: string[] };
    const groups = new Map<string, Group>();
    for (const key of selectedSkills) {
      const { marketplace, plugin, name } = parseSkillKey(key);
      const groupId = pluginKey(marketplace, plugin);
      let group = groups.get(groupId);
      if (!group) {
        group = { marketplace, plugin, names: [] };
        groups.set(groupId, group);
      }
      group.names.push(name);
    }

    const installedAll: string[] = [];
    const skippedAll: string[] = [];
    const failedAll: { name: string; error: string }[] = [];
    const notAttempted: string[] = [];

    try {
      for (const group of groups.values()) {
        try {
          const result = await commands.installSkills(
            group.marketplace,
            group.plugin,
            group.names,
            forceInstall,
            projectPath
          );
          if (result.status === "ok") {
            installedAll.push(...result.data.installed);
            skippedAll.push(...result.data.skipped);
            failedAll.push(...result.data.failed);
          } else {
            notAttempted.push(`${group.marketplace}/${group.plugin} (${result.error.message})`);
          }
        } catch (e) {
          const reason = e instanceof Error ? e.message : String(e);
          notAttempted.push(`${group.marketplace}/${group.plugin} (${reason})`);
        }
      }

      const hadSuccess = installedAll.length > 0 || skippedAll.length > 0;
      const parts: string[] = [];
      if (installedAll.length > 0) parts.push(`Installed: ${installedAll.join(", ")}`);
      if (skippedAll.length > 0) parts.push(`Skipped: ${skippedAll.join(", ")}`);
      if (failedAll.length > 0) {
        parts.push(`Failed: ${failedAll.map((f) => `${f.name} (${f.error})`).join(", ")}`);
      }
      if (notAttempted.length > 0) {
        parts.push(`Not attempted: ${notAttempted.join("; ")}`);
      }

      if (!hadSuccess && notAttempted.length > 0 && failedAll.length === 0) {
        installError = `Install failed: ${notAttempted.join("; ")}`;
      } else if (parts.length > 0) {
        installMessage = parts.join(" | ");
      }

      selectedSkills.clear();

      // Force-refresh so `installed` flags reflect new state. Fan out in
      // parallel — these reads are independent and serializing them delays
      // the grid refresh in proportion to the number of affected plugins.
      await Promise.all(
        Array.from(groups.values(), (group) =>
          fetchSkillsFor(group.marketplace, group.plugin, true)
        )
      );
    } finally {
      installing = false;
    }
  }

  $effect(() => {
    if (!popoverOpen) return;
    const onMouseDown = (e: MouseEvent) => {
      if (popRef && !popRef.contains(e.target as Node)) popoverOpen = false;
    };
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") popoverOpen = false;
    };
    document.addEventListener("mousedown", onMouseDown);
    document.addEventListener("keydown", onKeyDown);
    return () => {
      document.removeEventListener("mousedown", onMouseDown);
      document.removeEventListener("keydown", onKeyDown);
    };
  });

  onMount(() => {
    loadMarketplaces();
  });
</script>

{#snippet chipX(label: string, onclick: () => void, extraClass = "")}
  <button
    type="button"
    aria-label={label}
    {onclick}
    class="inline-flex items-center justify-center w-4 h-4 rounded-full opacity-70 hover:opacity-100 {extraClass}"
  >
    <svg class="w-2.5 h-2.5" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5">
      <path stroke-linecap="round" d="M6 6l12 12M6 18L18 6" />
    </svg>
  </button>
{/snippet}

<div class="flex flex-col h-full min-w-0">
  <div class="flex items-center gap-2 p-4 border-b border-kiro-muted">
    <input
      type="text"
      placeholder="Filter skills by name or description..."
      bind:value={filterText}
      class="flex-1 px-3 py-2 text-sm rounded-md border border-kiro-muted bg-kiro-overlay text-kiro-text placeholder-kiro-subtle focus:outline-none focus:ring-2 focus:ring-kiro-accent-500 focus:border-transparent"
    />

    <div class="relative" bind:this={popRef}>
      <button
        type="button"
        onclick={() => (popoverOpen = !popoverOpen)}
        aria-expanded={popoverOpen}
        aria-haspopup="true"
        class="inline-flex items-center gap-2 px-3.5 py-2 text-sm font-medium rounded-md border transition-colors focus:outline-none focus:ring-2 focus:ring-kiro-accent-500
          {popoverOpen || activeFilterCount > 1
            ? 'bg-kiro-accent-900/30 text-kiro-accent-300 border-transparent'
            : 'bg-kiro-overlay text-kiro-text-secondary border-kiro-muted hover:bg-kiro-muted hover:text-kiro-text'}"
      >
        <svg class="w-3.5 h-3.5" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
          <path stroke-linecap="round" stroke-linejoin="round" d="M3 4h18M6 12h12M10 20h4" />
        </svg>
        <span>Filters</span>
        {#if activeFilterCount > 0}
          <span class="inline-flex items-center justify-center min-w-[18px] h-[18px] px-1.5 text-[11px] font-semibold rounded-full bg-kiro-accent-500 text-white">
            {activeFilterCount}
          </span>
        {/if}
      </button>

      {#if popoverOpen}
        <div class="absolute top-[calc(100%+10px)] right-0 w-[280px] z-50 p-3.5 rounded-lg border border-kiro-muted bg-kiro-overlay shadow-lg">
          <div class="mb-3.5">
            <div class="mb-1.5 text-[10px] font-semibold uppercase tracking-wider text-kiro-subtle">Marketplace</div>
            {#each marketplaces as mp (mp.name)}
              <label class="flex items-center gap-2 px-1.5 py-1 text-[13px] text-kiro-text-secondary rounded hover:bg-kiro-accent-900/15 hover:text-kiro-text cursor-pointer">
                <input
                  type="checkbox"
                  checked={selectedMarketplaces.has(mp.name)}
                  onchange={() => toggleMarketplace(mp.name)}
                  class="h-3.5 w-3.5 rounded border-kiro-muted text-kiro-accent-500"
                />
                <span class="w-2 h-2 rounded-full flex-shrink-0 {
                  mp.source_type === 'github' ? 'bg-kiro-info' :
                  mp.source_type === 'local' ? 'bg-kiro-warning' :
                  'bg-kiro-accent-400'
                }"></span>
                <span class="flex-1 truncate">{mp.name}</span>
                <span class="text-[11px] text-kiro-subtle">{mp.plugin_count}</span>
              </label>
            {/each}
          </div>

          {#if availablePlugins.length > 0}
            <div class="mb-3.5">
              <div class="mb-1.5 text-[10px] font-semibold uppercase tracking-wider text-kiro-subtle">Plugin</div>
              {#each availablePlugins as ap (pluginKey(ap.marketplace, ap.plugin.name))}
                {@const key = pluginKey(ap.marketplace, ap.plugin.name)}
                <label class="flex items-center gap-2 px-1.5 py-1 text-[13px] text-kiro-text-secondary rounded hover:bg-kiro-accent-900/15 hover:text-kiro-text cursor-pointer">
                  <input
                    type="checkbox"
                    checked={selectedPlugins.has(key)}
                    onchange={() => togglePlugin(key)}
                    class="h-3.5 w-3.5 rounded border-kiro-muted text-kiro-accent-500"
                  />
                  <span class="flex-1 truncate">{ap.plugin.name}</span>
                  <span class="text-[11px] text-kiro-subtle">{ap.plugin.skill_count}</span>
                </label>
              {/each}
            </div>
          {/if}

          <div class="mb-3.5">
            <div class="mb-1.5 text-[10px] font-semibold uppercase tracking-wider text-kiro-subtle">Status</div>
            <label class="flex items-center gap-2 px-1.5 py-1 text-[13px] text-kiro-text-secondary rounded hover:bg-kiro-accent-900/15 hover:text-kiro-text cursor-pointer">
              <input
                type="checkbox"
                checked={installedOnly}
                onchange={() => (installedOnly = !installedOnly)}
                class="h-3.5 w-3.5 rounded border-kiro-muted text-kiro-accent-500"
              />
              <span class="flex-1">Installed only</span>
            </label>
          </div>

          <div class="flex items-center justify-between mt-3 pt-2.5 border-t border-kiro-muted text-xs">
            <span class="text-kiro-subtle">
              {filteredSkills.length} {filteredSkills.length === 1 ? "skill" : "skills"}
            </span>
            <button
              type="button"
              onclick={resetFilters}
              disabled={activeFilterCount <= 1}
              class="text-kiro-accent-300 hover:text-kiro-accent-400 disabled:text-kiro-subtle disabled:cursor-default"
            >
              Reset
            </button>
          </div>
        </div>
      {/if}
    </div>
  </div>

  {#if activeFilterCount > 0}
    <div class="flex items-center flex-wrap gap-1.5 px-4 py-2.5 border-b border-kiro-muted bg-kiro-surface/50">
      <span class="text-[11px] text-kiro-subtle mr-0.5">Showing:</span>

      {#each [...selectedMarketplaces] as name (name)}
        {@const mp = marketplaces.find((m) => m.name === name)}
        <span class="inline-flex items-center gap-1.5 pl-2.5 pr-1 py-[3px] text-xs font-medium rounded-full bg-kiro-accent-900/30 text-kiro-accent-300">
          <span class="w-1.5 h-1.5 rounded-full {
            mp?.source_type === 'github' ? 'bg-kiro-info' :
            mp?.source_type === 'local' ? 'bg-kiro-warning' :
            'bg-kiro-accent-400'
          }"></span>
          {name}
          {@render chipX(`Remove ${name}`, () => toggleMarketplace(name), "hover:bg-kiro-accent-500/30")}
        </span>
      {/each}

      {#each [...selectedPlugins] as key (key)}
        {@const ref = parsePluginKey(key)}
        <span class="inline-flex items-center gap-1.5 pl-2.5 pr-1 py-[3px] text-xs font-medium rounded-full bg-kiro-info/[0.18] text-kiro-info">
          {ref.plugin}
          {@render chipX(`Remove ${ref.plugin}`, () => togglePlugin(key))}
        </span>
      {/each}

      {#if installedOnly}
        <span class="inline-flex items-center gap-1.5 pl-2.5 pr-1 py-[3px] text-xs font-medium rounded-full bg-kiro-success/[0.18] text-kiro-success">
          Installed only
          {@render chipX("Remove installed-only filter", () => (installedOnly = false))}
        </span>
      {/if}

      {#if activeFilterCount > 1}
        <button
          type="button"
          onclick={resetFilters}
          class="ml-auto px-1.5 py-0.5 text-[11px] text-kiro-subtle hover:text-kiro-text"
        >
          Clear all
        </button>
      {/if}
    </div>
  {/if}

  {#each Object.entries(fetchErrors) as [key, message] (key)}
    <div
      data-testid="fetch-error"
      class="mx-4 mt-3 px-4 py-3 rounded-md bg-kiro-error/10 border border-kiro-error/30 flex items-start gap-3"
    >
      <p class="text-sm text-kiro-error flex-1">{message}</p>
      <button
        type="button"
        onclick={() => {
          delete fetchErrors[key];
        }}
        aria-label="Dismiss error"
        class="text-kiro-error/70 hover:text-kiro-error text-lg leading-none flex-shrink-0 focus:outline-none focus:ring-2 focus:ring-kiro-accent-500 rounded"
      >
        ×
      </button>
    </div>
  {/each}

  {#if installError}
    <div
      data-testid="install-error"
      class="mx-4 mt-3 px-4 py-3 rounded-md bg-kiro-error/10 border border-kiro-error/30 flex items-start gap-3"
    >
      <p class="text-sm text-kiro-error flex-1">{installError}</p>
      <button
        type="button"
        onclick={() => (installError = null)}
        aria-label="Dismiss error"
        class="text-kiro-error/70 hover:text-kiro-error text-lg leading-none flex-shrink-0 focus:outline-none focus:ring-2 focus:ring-kiro-accent-500 rounded"
      >
        ×
      </button>
    </div>
  {/if}

  {#if installMessage}
    <div class="mx-4 mt-3 px-4 py-3 rounded-md bg-kiro-success/10 border border-kiro-success/30">
      <p class="text-sm text-kiro-success">{installMessage}</p>
    </div>
  {/if}

  <div class="flex-1 overflow-y-auto p-4">
    {#if showLoadingSpinner}
      <div class="flex flex-col items-center justify-center h-full text-kiro-subtle gap-3">
        <svg class="w-8 h-8 text-kiro-accent-800 animate-pulse" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="1.5"
            d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15" />
        </svg>
        <p class="text-sm">Loading skills...</p>
      </div>
    {:else if initialLoadFailed}
      <div class="flex flex-col items-center justify-center h-full text-kiro-subtle gap-3">
        <svg class="w-10 h-10 text-kiro-error" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="1.5"
            d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-3L13.732 4c-.77-1.333-2.694-1.333-3.464 0L3.34 16c-.77 1.333.192 3 1.732 3z" />
        </svg>
        <p class="text-sm">Failed to load marketplaces. See error above.</p>
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
        {#each filteredSkills as skill (skillKey(skill.marketplace, skill.plugin, skill.name))}
          {@const key = skillKey(skill.marketplace, skill.plugin, skill.name)}
          <SkillCard
            {skill}
            selected={selectedSkills.has(key)}
            onToggle={() => toggleSkill(key)}
          />
        {/each}
      </div>
    {/if}
  </div>

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
      type="button"
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
</div>

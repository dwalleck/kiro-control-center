<script lang="ts">
  import { onMount } from "svelte";
  import { SvelteMap, SvelteSet } from "svelte/reactivity";
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

  // Error-source key family. The `plugins\u001f` / `skills\u001f` /
  // `bulk-skills\u001f` prefixes embed DELIM so a marketplace literally
  // named `plugins` or `skills` still produces a distinct key from the
  // namespace tag.
  const PLUGINS_ERR_PREFIX = `plugins${DELIM}` as const;
  const SKILLS_ERR_PREFIX = `skills${DELIM}` as const;
  const BULK_SKILLS_ERR_PREFIX = `bulk-skills${DELIM}` as const;
  const ERR_MARKETPLACES = "marketplaces" as const;
  type ErrorSource =
    | typeof ERR_MARKETPLACES
    | `${typeof PLUGINS_ERR_PREFIX}${string}`
    | `${typeof SKILLS_ERR_PREFIX}${string}${typeof DELIM}${string}`
    | `${typeof BULK_SKILLS_ERR_PREFIX}${string}`;
  // Compile-time guard: fails if any `as const` above is removed and the
  // union silently widens back to `string` (which would defeat typo
  // protection on `fetchErrors.get/set/delete` with zero compile errors).
  type _AssertNarrow = string extends ErrorSource ? never : ErrorSource;
  const pluginsErrKey = (mp: string): ErrorSource => `${PLUGINS_ERR_PREFIX}${mp}`;
  const skillsErrKey = (mp: string, plugin: string): ErrorSource =>
    `${SKILLS_ERR_PREFIX}${mp}${DELIM}${plugin}`;
  const bulkSkillsErrKey = (mp: string): ErrorSource => `${BULK_SKILLS_ERR_PREFIX}${mp}`;

  // Short source-label for screen-reader aria-label on dismiss buttons. The
  // banner body already holds the full message; the button label just needs
  // enough context to disambiguate N stacked identical-looking controls.
  function errLabel(key: ErrorSource): string {
    if (key === ERR_MARKETPLACES) return "Dismiss marketplaces error";
    if (key.startsWith(PLUGINS_ERR_PREFIX)) {
      return `Dismiss error for ${key.slice(PLUGINS_ERR_PREFIX.length)}`;
    }
    if (key.startsWith(BULK_SKILLS_ERR_PREFIX)) {
      return `Dismiss error for ${key.slice(BULK_SKILLS_ERR_PREFIX.length)}`;
    }
    const { marketplace, plugin } = parsePluginKey(key.slice(SKILLS_ERR_PREFIX.length));
    return `Dismiss error for ${marketplace}/${plugin}`;
  }

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
  let pendingBulkSkillFetches = new SvelteSet<string>();
  let installing: boolean = $state(false);

  // Keyed per-source so a concurrent success for one fetch can't clear
  // another source's failure mid-race.
  let fetchErrors = new SvelteMap<ErrorSource, string>();
  let installError: string | null = $state(null);
  let installMessage: string | null = $state(null);

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
      (loadingMarketplaces ||
        pendingPluginFetches.size > 0 ||
        pendingSkillFetches.size > 0 ||
        pendingBulkSkillFetches.size > 0)
  );

  // Only the initial-marketplaces fetch gates the grid's empty-state UI.
  // Plugin and skill fetch errors surface as their own banners but don't
  // imply an empty grid — a selected marketplace can have a mix of working
  // and broken plugins and still render successful pairs' skills.
  let initialLoadFailed = $derived(
    fetchErrors.has(ERR_MARKETPLACES) &&
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
        fetchErrors.delete(ERR_MARKETPLACES);
      } else {
        console.error("[BrowseTab] listMarketplaces returned error", result.error);
        fetchErrors.set(ERR_MARKETPLACES, result.error.message);
      }
    } catch (e) {
      console.error("[BrowseTab] listMarketplaces threw", e);
      fetchErrors.set(ERR_MARKETPLACES, e instanceof Error ? e.message : String(e));
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
        fetchErrors.delete(errKey);
      } else {
        console.error(`[BrowseTab] listPlugins(${mp}) returned error`, result.error);
        fetchErrors.set(errKey, `${mp}: ${result.error.message}`);
      }
    } catch (e) {
      console.error(`[BrowseTab] listPlugins(${mp}) threw`, e);
      const reason = e instanceof Error ? e.message : String(e);
      fetchErrors.set(errKey, `${mp}: ${reason}`);
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
        fetchErrors.delete(errKey);
      } else {
        console.error(`[BrowseTab] listAvailableSkills(${mp}, ${plugin}) returned error`, result.error);
        fetchErrors.set(errKey, `${mp}/${plugin}: ${result.error.message}`);
      }
    } catch (e) {
      console.error(`[BrowseTab] listAvailableSkills(${mp}, ${plugin}) threw`, e);
      const reason = e instanceof Error ? e.message : String(e);
      fetchErrors.set(errKey, `${mp}/${plugin}: ${reason}`);
    } finally {
      pendingSkillFetches.delete(key);
    }
  }

  // Bulk path: one backend call populates per-plugin cache entries for an
  // entire marketplace. Used when no plugin filter is active; a marketplace
  // with 50 plugins would otherwise fire 50 concurrent `listAvailableSkills`
  // calls on first paint. Plugins with zero skills get empty-array cache
  // entries so the per-pair guard in `fetchSkillsFor` doesn't re-fetch them
  // later if the user applies a plugin filter.
  async function fetchAllSkillsForMarketplace(mp: string, plugins: PluginInfo[]) {
    if (pendingBulkSkillFetches.has(mp)) return;
    const allCached = plugins.every(
      (p) => skillsByPluginPair[pluginKey(mp, p.name)] !== undefined
    );
    if (allCached) return;

    pendingBulkSkillFetches.add(mp);
    const errKey = bulkSkillsErrKey(mp);
    try {
      const result = await commands.listAllSkillsForMarketplace(mp, projectPath);
      if (result.status === "ok") {
        const byPlugin = new Map<string, SkillInfo[]>();
        for (const s of result.data) {
          const arr = byPlugin.get(s.plugin);
          if (arr) arr.push(s);
          else byPlugin.set(s.plugin, [s]);
        }
        for (const p of plugins) {
          skillsByPluginPair[pluginKey(mp, p.name)] = byPlugin.get(p.name) ?? [];
        }
        fetchErrors.delete(errKey);
        // The bulk response is authoritative for this marketplace — clear any
        // stale per-plugin skill errors lingering from a prior filtered path.
        const stale: ErrorSource[] = [];
        for (const key of fetchErrors.keys()) {
          if (key.startsWith(SKILLS_ERR_PREFIX)) {
            const { marketplace } = parsePluginKey(key.slice(SKILLS_ERR_PREFIX.length));
            if (marketplace === mp) stale.push(key);
          }
        }
        for (const key of stale) fetchErrors.delete(key);
      } else {
        console.error(`[BrowseTab] listAllSkillsForMarketplace(${mp}) returned error`, result.error);
        fetchErrors.set(errKey, `${mp}: ${result.error.message}`);
      }
    } catch (e) {
      console.error(`[BrowseTab] listAllSkillsForMarketplace(${mp}) threw`, e);
      const reason = e instanceof Error ? e.message : String(e);
      fetchErrors.set(errKey, `${mp}: ${reason}`);
    } finally {
      pendingBulkSkillFetches.delete(mp);
    }
  }

  $effect(() => {
    for (const mp of selectedMarketplaces) fetchPluginsFor(mp);
  });

  // When no plugin filter is active, prefer the bulk path — one call per
  // marketplace instead of one per (mp, plugin). Once a filter narrows the
  // set, fall back to per-plugin calls which avoid over-fetching skills
  // the user explicitly hid.
  $effect(() => {
    for (const mp of selectedMarketplaces) {
      const plugins = pluginsByMarketplace[mp];
      if (plugins === undefined) continue;

      if (selectedPlugins.size === 0) {
        fetchAllSkillsForMarketplace(mp, plugins);
      } else {
        for (const pl of plugins) {
          if (!selectedPlugins.has(pluginKey(mp, pl.name))) continue;
          fetchSkillsFor(mp, pl.name);
        }
      }
    }
  });

  // Skill caches and skill-fetch errors are both project-scoped — `installed`
  // flags flip and error messages cite paths under the previous project — so
  // invalidate the lot when projectPath changes. Plugin-fetch and marketplace
  // errors are project-agnostic and survive.
  let priorProjectPath: string | null = null;
  $effect(() => {
    if (priorProjectPath !== null && priorProjectPath !== projectPath) {
      skillsByPluginPair = {};
      selectedSkills.clear();
      // Snapshot first — deleting during `for (const key of fetchErrors.keys())`
      // would re-trigger the effect on each delete.
      const stale: ErrorSource[] = [];
      for (const key of fetchErrors.keys()) {
        if (
          key.startsWith(SKILLS_ERR_PREFIX) ||
          key.startsWith(BULK_SKILLS_ERR_PREFIX)
        ) {
          stale.push(key);
        }
      }
      for (const key of stale) fetchErrors.delete(key);
    }
    priorProjectPath = projectPath;
  });

  // Drop stale selections and banners when the filter set narrows — leaving a
  // banner for a deselected source misattributes responsibility to a filter
  // the user set intentionally. Marketplace-listing errors always survive.
  $effect(() => {
    const valid = new Set(skills.map((s) => skillKey(s.marketplace, s.plugin, s.name)));
    for (const key of selectedSkills) {
      if (!valid.has(key)) selectedSkills.delete(key);
    }

    const stale: ErrorSource[] = [];
    for (const key of fetchErrors.keys()) {
      if (key === ERR_MARKETPLACES) continue;
      if (key.startsWith(PLUGINS_ERR_PREFIX)) {
        const mp = key.slice(PLUGINS_ERR_PREFIX.length);
        if (!selectedMarketplaces.has(mp)) stale.push(key);
      } else if (key.startsWith(BULK_SKILLS_ERR_PREFIX)) {
        const mp = key.slice(BULK_SKILLS_ERR_PREFIX.length);
        if (!selectedMarketplaces.has(mp)) stale.push(key);
      } else if (key.startsWith(SKILLS_ERR_PREFIX)) {
        const { marketplace, plugin } = parsePluginKey(key.slice(SKILLS_ERR_PREFIX.length));
        const stillSelected =
          selectedMarketplaces.has(marketplace) &&
          (selectedPlugins.size === 0 || selectedPlugins.has(pluginKey(marketplace, plugin)));
        if (!stillSelected) stale.push(key);
      }
    }
    for (const key of stale) fetchErrors.delete(key);
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
      // fetchSkillsFor never rejects externally (its own try/catch surfaces
      // failures via fetchErrors), so this Promise.all should resolve. The
      // outer try/catch is defense-in-depth against a future regression in
      // that invariant that would otherwise strand `installing = true`.
      try {
        await Promise.all(
          Array.from(groups.values(), (group) =>
            fetchSkillsFor(group.marketplace, group.plugin, true)
          )
        );
      } catch (e) {
        console.error("[BrowseTab] post-install refresh rejected unexpectedly", e);
        const reason = e instanceof Error ? e.message : String(e);
        installError = `Post-install refresh failed: ${reason}`;
      }
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

  <!-- Banners render newest-first (reverse insertion order) and cap at 3 so
       a storm of broken plugins doesn't push the grid off-screen. Dismissing
       a banner or resolving its source surfaces the next-newest below. -->
  {#each [...fetchErrors].reverse().slice(0, 3) as [key, message] (key)}
    <div
      data-testid="fetch-error"
      class="mx-4 mt-3 px-4 py-3 rounded-md bg-kiro-error/10 border border-kiro-error/30 flex items-start gap-3"
    >
      <p class="text-sm text-kiro-error flex-1">{message}</p>
      <button
        type="button"
        onclick={() => fetchErrors.delete(key)}
        aria-label={errLabel(key)}
        class="text-kiro-error/70 hover:text-kiro-error text-lg leading-none flex-shrink-0 focus:outline-none focus:ring-2 focus:ring-kiro-accent-500 rounded"
      >
        ×
      </button>
    </div>
  {/each}
  {#if fetchErrors.size > 3}
    <div
      data-testid="fetch-error-overflow"
      class="mx-4 mt-3 px-4 py-2 text-xs text-kiro-subtle text-center border border-kiro-muted/50 rounded-md bg-kiro-surface/30"
    >
      +{fetchErrors.size - 3} more {fetchErrors.size - 3 === 1 ? "error" : "errors"} — dismiss or resolve above to see the rest
    </div>
  {/if}

  {#if installError}
    <div
      data-testid="install-error"
      class="mx-4 mt-3 px-4 py-3 rounded-md bg-kiro-error/10 border border-kiro-error/30 flex items-start gap-3"
    >
      <p class="text-sm text-kiro-error flex-1">{installError}</p>
      <button
        type="button"
        onclick={() => (installError = null)}
        aria-label="Dismiss install error"
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
          {#if filterText}
            No skills match the filter
          {:else if fetchErrors.size > 0}
            Skills unavailable due to errors above
          {:else}
            No skills available
          {/if}
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

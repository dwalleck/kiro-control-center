<script lang="ts">
  import { onMount } from "svelte";
  import { SvelteMap, SvelteSet } from "svelte/reactivity";
  import { commands } from "$lib/bindings";
  import {
    formatSkippedSkill,
    formatSkippedSkillsForPlugin,
    formatSteeringWarning,
    formatFailedSkill,
    formatFailedSteeringFile,
    formatFailedAgent,
    skillCountLabel,
    skillCountTitle,
  } from "$lib/format";
  import {
    DELIM,
    pluginKey,
    skillKey,
    parsePluginKey,
    parseSkillKey,
  } from "$lib/keys";
  import { pluginUpdates } from "$lib/stores/plugin-updates.svelte";
  import type { BrowseAction } from "$lib/stores/plugin-updates";
  import {
    ERR_INSTALLED_PLUGINS,
    ERR_UPDATE_FETCH,
    UPDATE_CHECK_PREFIX,
    parseUpdateCheckKey,
  } from "$lib/error-source";
  import type { UpdateCheckKey } from "$lib/error-source";
  import { runPluginInstall as doPluginInstall } from "$lib/plugin-actions";
  import type { PluginActionMode } from "$lib/plugin-actions";
  import { usePluginUpdateBanners } from "$lib/stores/plugin-update-banners.svelte";
  import type {
    InstalledPluginInfo,
    InstallPluginResult_Serialize,
    MarketplaceInfo,
    PluginCatalogEntryView,
    PluginCatalogResponseView,
    PluginInfo,
    SkillInfo,
    SkippedSkill,
  } from "$lib/bindings";
  import BannerStack from "./BannerStack.svelte";
  import SkillCard from "./SkillCard.svelte";
  import PluginCard from "./PluginCard.svelte";

  type BrowseView = "plugins" | "skills";

  let { projectPath }: { projectPath: string } = $props();

  // Error-source key family. The `catalog\u001f` and
  // `catalog-skipped\u001f` prefixes embed DELIM so a marketplace
  // literally named `catalog` or `catalog-skipped` still produces a
  // distinct key from the namespace tag. CATALOG keys cover whole-
  // marketplace fetch failures (one banner per failed catalog call);
  // CATALOG_SKIPPED keys cover per-plugin skips returned in
  // PluginCatalogResponseView.skipped (one banner per skipped plugin).
  // Slice 2 of the BrowseTab redesign collapsed the prior
  // `plugins` / `skills` / `bulk-skills` prefix trio into these two,
  // mirroring the bulk catalog command's response surface.
  const CATALOG_ERR_PREFIX = `catalog${DELIM}` as const;
  const CATALOG_SKIPPED_PREFIX = `catalog-skipped${DELIM}` as const;
  const ERR_MARKETPLACES = "marketplaces" as const;
  type ErrorSource =
    | typeof ERR_MARKETPLACES
    | typeof ERR_INSTALLED_PLUGINS
    | typeof ERR_UPDATE_FETCH
    | UpdateCheckKey
    | `${typeof CATALOG_ERR_PREFIX}${string}`
    | `${typeof CATALOG_SKIPPED_PREFIX}${string}${typeof DELIM}${string}`;
  // Compile-time guard: fails if any `as const` above is removed and the
  // union silently widens back to `string` (which would defeat typo
  // protection on `fetchErrors.get/set/delete` with zero compile errors).
  const _ = null as unknown as (string extends ErrorSource ? never : 0);
  void _;
  const catalogErrKey = (mp: string): ErrorSource => `${CATALOG_ERR_PREFIX}${mp}`;
  const catalogSkippedKey = (mp: string, plugin: string): ErrorSource =>
    `${CATALOG_SKIPPED_PREFIX}${mp}${DELIM}${plugin}`;

  // Short source-label for screen-reader aria-label on dismiss buttons. The
  // banner body already holds the full message; the button label just needs
  // enough context to disambiguate N stacked identical-looking controls.
  function errLabel(key: ErrorSource): string {
    if (key === ERR_MARKETPLACES) return "Dismiss marketplaces error";
    if (key === ERR_INSTALLED_PLUGINS) return "Dismiss installed-plugins error";
    if (key === ERR_UPDATE_FETCH) return "Dismiss update-check error";
    if (key.startsWith(UPDATE_CHECK_PREFIX + DELIM)) {
      const { marketplace } = parseUpdateCheckKey(key);
      return `Dismiss update-check banner for ${marketplace}`;
    }
    if (key.startsWith(CATALOG_ERR_PREFIX)) {
      return `Dismiss error for ${key.slice(CATALOG_ERR_PREFIX.length)}`;
    }
    const { marketplace, plugin } = parsePluginKey(key.slice(CATALOG_SKIPPED_PREFIX.length));
    return `Dismiss error for ${marketplace}/${plugin}`;
  }

  let marketplaces: MarketplaceInfo[] = $state([]);
  // Bulk plugin catalog, keyed by marketplace name. Populated by
  // `fetchCatalogFor(mp)` which calls
  // `commands.listPluginCatalogForMarketplace`. Each entry carries the
  // plugin's full per-category item tree (skills + steering + agents)
  // with per-item `installed: bool` flags computed against the project's
  // tracking files at fetch time. Slice 2 of the BrowseTab redesign
  // collapses the prior `pluginsByMarketplace + skillsByPluginPair`
  // pair (powered by separate listPlugins / listAvailableSkills /
  // listAllSkillsForMarketplace fetches) into this single source.
  let catalogByMarketplace: Record<string, PluginCatalogResponseView> = $state({});

  let selectedMarketplaces = new SvelteSet<string>();
  let selectedPlugins = new SvelteSet<string>();
  let selectedSkills = new SvelteSet<string>();
  let installedOnly: boolean = $state(false);
  let filterText: string = $state("");
  let forceInstall: boolean = $state(false);
  let popoverOpen: boolean = $state(false);
  let popRef: HTMLDivElement | undefined = $state();
  let browseView: BrowseView = $state("plugins");

  let pendingPluginActions = new SvelteMap<string, BrowseAction>();

  let installedPlugins: InstalledPluginInfo[] = $state([]);
  let installedPluginKeys = $derived(
    new Set(installedPlugins.map((p) => pluginKey(p.marketplace, p.plugin))),
  );

  let loadingMarketplaces: boolean = $state(false);
  // Single pending-fetch tracker keyed by ErrorSource — each in-flight fetch
  // "owns" its error-source key until it completes. Unifying the three
  // previous per-fetcher sets puts pending-tracking on the same typed
  // taxonomy as the errors they mirror (every pending key is also a
  // potential fetchErrors key).
  let pendingFetches = new SvelteSet<ErrorSource>();
  let installing: boolean = $state(false);

  // Keyed per-source so a concurrent success for one fetch can't clear
  // another source's failure mid-race.
  let fetchErrors = new SvelteMap<ErrorSource, string>();
  let installError: string | null = $state(null);
  let installMessage: string | null = $state(null);
  let installWarning: string | null = $state(null);
  let installStaleRefresh: string | null = $state(null);
  let installResult: InstallPluginResult_Serialize | null = $state(null);
  let installResultPlugin: string | null = $state(null);
  let installResultHasFailures = $derived.by(() => {
    if (installResult === null) return false;
    return (
      installResult.skills.failed.length +
        installResult.steering.failed.length +
        installResult.agents.failed.length >
      0
    );
  });

  // Project a PluginCatalogEntryView into the today-shaped PluginInfo
  // so PluginCard, skillCountLabel, and skillCountTitle stay unchanged.
  // The catalog only includes plugins whose manifest loaded; broken-
  // manifest plugins live in `view.skipped` and surface as banners
  // (see fetchCatalogFor). This is a UX shift from the prior path,
  // which kept manifest_failed plugins in the grid with a colored
  // count — banners are more discoverable, and the previous on-card
  // signal was a 1px color change easy to miss.
  function pluginInfoFromEntry(entry: PluginCatalogEntryView): PluginInfo {
    return {
      name: entry.plugin,
      description: entry.description,
      skill_count: { state: "known", count: entry.skills.length },
      source_type: entry.source_type,
    };
  }

  let availablePlugins = $derived.by(() => {
    const out: { marketplace: string; plugin: PluginInfo }[] = [];
    for (const mp of selectedMarketplaces) {
      const view = catalogByMarketplace[mp];
      if (view === undefined) continue;
      for (const entry of view.plugins) {
        out.push({ marketplace: mp, plugin: pluginInfoFromEntry(entry) });
      }
    }
    return out;
  });

  let skills = $derived.by(() => {
    const rows: SkillInfo[] = [];
    for (const mp of selectedMarketplaces) {
      const view = catalogByMarketplace[mp];
      if (view === undefined) continue;
      for (const entry of view.plugins) {
        if (selectedPlugins.size > 0 && !selectedPlugins.has(pluginKey(mp, entry.plugin))) continue;
        rows.push(...entry.skills);
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
    skills.length === 0 && (loadingMarketplaces || pendingFetches.size > 0)
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

  // Shared scaffold for fetch helpers: manages `pendingFetches` membership
  // and structured console logging. The guard deliberately does NOT mutate
  // `fetchErrors` — callers own their banner lifecycle end-to-end via the
  // `onSuccess` / `onError` callbacks. A prior design had the guard clear
  // the banner at `pendingKey` after `onSuccess` returned, which silently
  // wiped any banner the callback had set under the same key (see the
  // `skipped_skills` regression on the single-plugin path). Keeping error
  // ownership at the call site prevents that class of collision — the
  // guard never touches keys it doesn't own.
  async function withFetchGuard<T>(
    pendingKey: ErrorSource,
    label: string,
    op: () => Promise<{ status: "ok"; data: T } | { status: "error"; error: { message: string } }>,
    callbacks: {
      onSuccess: (data: T) => void;
      onError: (message: string) => void;
    }
  ): Promise<void> {
    pendingFetches.add(pendingKey);
    try {
      const result = await op();
      if (result.status === "ok") {
        callbacks.onSuccess(result.data);
      } else {
        console.error(`[BrowseTab] ${label} returned error`, result.error);
        callbacks.onError(result.error.message);
      }
    } catch (e) {
      console.error(`[BrowseTab] ${label} threw`, e);
      callbacks.onError(e instanceof Error ? e.message : String(e));
    } finally {
      pendingFetches.delete(pendingKey);
    }
  }

  // Single bulk fetch per marketplace, replacing the prior
  // listPlugins / listAvailableSkills / listAllSkillsForMarketplace
  // cascade. Returns the full per-plugin item tree (skills + steering +
  // agents) plus structurally-surfaced skipped plugins. Per-item parse
  // failures live on each entry's `skipped_items` and aren't surfaced as
  // banners here — slice 3 (visual redesign) decides where to render
  // them; today's banner-stack carries only plugin-level skips, matching
  // the prior `bulk-skills` and `skills` prefixes' semantics.
  //
  // Loop budget: O(K × items) inside the backend per call, K ≤ 50
  // plugins per marketplace at production scale. This frontend function
  // is one Tauri call per invocation.
  async function fetchCatalogFor(mp: string, force = false) {
    const key = catalogErrKey(mp);
    if (pendingFetches.has(key)) return;
    if (!force && catalogByMarketplace[mp] !== undefined) return;
    await withFetchGuard(
      key,
      mp,
      () => commands.listPluginCatalogForMarketplace(mp, projectPath),
      {
        onSuccess: (view) => {
          catalogByMarketplace[mp] = view;
          // The bulk response is authoritative for working plugins;
          // every per-plugin skipped-banner from a prior fetch for THIS
          // marketplace is replaced wholesale. Build the new banner set
          // first, then sweep stale entries — same ownership boundary as
          // the prior fetchAllSkillsForMarketplace path.
          for (const sp of view.skipped) {
            fetchErrors.set(catalogSkippedKey(mp, sp.name), `${mp}/${sp.name}: ${sp.reason}`);
          }
          const fresh = new Set(view.skipped.map((sp) => catalogSkippedKey(mp, sp.name)));
          const stale: ErrorSource[] = [];
          for (const k of fetchErrors.keys()) {
            if (!k.startsWith(CATALOG_SKIPPED_PREFIX)) continue;
            const { marketplace } = parsePluginKey(k.slice(CATALOG_SKIPPED_PREFIX.length));
            if (marketplace !== mp) continue;
            if (fresh.has(k)) continue;
            stale.push(k);
          }
          for (const k of stale) fetchErrors.delete(k);
          fetchErrors.delete(key);
        },
        onError: (message) => {
          fetchErrors.set(key, `${mp}: ${message}`);
        },
      }
    );
  }

  async function fetchInstalledPlugins() {
    if (!projectPath) {
      installedPlugins = [];
      fetchErrors.delete(ERR_INSTALLED_PLUGINS);
      return;
    }
    try {
      const result = await commands.listInstalledPlugins(projectPath);
      if (result.status === "ok") {
        // Wire format is `InstalledPluginsView` (I13) — read `.plugins`,
        // not `.data` directly. Surfacing partial-load warnings via
        // `fetchErrors` keeps them in the same banner stack as every
        // other failure on this tab; a console-only log would let the
        // user wonder why a plugin they just installed shows "Install"
        // again instead of "Installed".
        installedPlugins = result.data.plugins;
        const warnings = result.data.partial_load_warnings ?? [];
        if (warnings.length > 0) {
          const summary = warnings
            .map((w) => `${w.tracking_file}: ${w.error}`)
            .join("; ");
          fetchErrors.set(
            ERR_INSTALLED_PLUGINS,
            `Installed plugins partially loaded — ${summary}`,
          );
        } else {
          fetchErrors.delete(ERR_INSTALLED_PLUGINS);
        }
      } else {
        // Real Tauri-layer error (e.g. validate_kiro_project_path failed).
        // Surface — don't silently fall back to an empty array, which
        // would re-show "Install" buttons for plugins that may already
        // be installed and re-enable double-installs.
        console.error("[BrowseTab] listInstalledPlugins error", result.error);
        fetchErrors.set(
          ERR_INSTALLED_PLUGINS,
          `Could not load installed plugins: ${result.error.message}`,
        );
      }
    } catch (e) {
      console.error("[BrowseTab] listInstalledPlugins rejected", e);
      fetchErrors.set(
        ERR_INSTALLED_PLUGINS,
        `Could not load installed plugins: ${e instanceof Error ? e.message : String(e)}`,
      );
    }
  }

  $effect(() => {
    // Read projectPath to register the dependency.
    void projectPath;
    fetchInstalledPlugins();
  });

  usePluginUpdateBanners({
    projectPath: () => projectPath,
    fetchErrors,
    logPrefix: "BrowseTab",
  });

  // Single effect: one bulk catalog fetch per selected marketplace.
  // No second effect for plugin-narrowing — the catalog returns ALL
  // plugins per marketplace in one call, and `availablePlugins` /
  // `skills` derives narrow visually. Over-fetch on plugin filter (the
  // optimization the prior dual-effect setup chased) is replaced by
  // never re-fetching once the catalog is cached for the marketplace.
  $effect(() => {
    for (const mp of selectedMarketplaces) fetchCatalogFor(mp);
  });

  // The catalog is project-scoped: every entry's `installed: bool`
  // flags are computed against the previous project's tracking files,
  // and any `catalog-skipped` banner cites paths under the previous
  // project root. Drop the catalog cache and project-scoped banners
  // when projectPath changes so the next $effect run re-fetches under
  // the new project. Marketplace-listing errors are project-agnostic
  // and survive.
  let priorProjectPath: string | null = null;
  $effect(() => {
    if (priorProjectPath !== null && priorProjectPath !== projectPath) {
      catalogByMarketplace = {};
      selectedSkills.clear();
      installError = null;
      installMessage = null;
      installWarning = null;
      installStaleRefresh = null;
      installResult = null;
      installResultPlugin = null;
      pendingPluginActions.clear();
      // Snapshot first — deleting during keys() iteration re-triggers the effect.
      const stale: ErrorSource[] = [];
      for (const key of fetchErrors.keys()) {
        if (
          key.startsWith(CATALOG_ERR_PREFIX) ||
          key.startsWith(CATALOG_SKIPPED_PREFIX) ||
          key.startsWith(UPDATE_CHECK_PREFIX + DELIM) ||
          key === ERR_UPDATE_FETCH ||
          key === ERR_INSTALLED_PLUGINS
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
      if (key.startsWith(CATALOG_ERR_PREFIX)) {
        const mp = key.slice(CATALOG_ERR_PREFIX.length);
        if (!selectedMarketplaces.has(mp)) stale.push(key);
      } else if (key.startsWith(CATALOG_SKIPPED_PREFIX)) {
        const { marketplace, plugin } = parsePluginKey(key.slice(CATALOG_SKIPPED_PREFIX.length));
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
    installWarning = null;
    installStaleRefresh = null;

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
    const unreadableAll: SkippedSkill[] = [];
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
            // Per-skill read/parse failures surface separately from
            // `failed` (which is install-time errors only). Previously
            // these vanished into `warn!` logs and the user saw a
            // shorter "installed N" count with no explanation.
            unreadableAll.push(...result.data.skipped_skills);
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
      if (unreadableAll.length > 0) {
        parts.push(
          `Unreadable: ${unreadableAll.map(formatSkippedSkill).join(", ")}`
        );
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

      // Force-refresh so `installed` flags reflect new state. Fan out
      // by marketplace (not by plugin) — the catalog returns the entire
      // marketplace's tree per call, so re-fetching once per affected
      // marketplace covers every plugin that just had skills installed.
      // fetchCatalogFor never rejects externally (its own try/catch
      // surfaces failures via fetchErrors), so this Promise.all should
      // resolve. The outer try/catch is defense-in-depth against a
      // future regression in that invariant that would otherwise
      // strand `installing = true`.
      try {
        const affected = new Set(
          Array.from(groups.values(), (group) => group.marketplace),
        );
        await Promise.all(
          [...affected].map((mp) => fetchCatalogFor(mp, true)),
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

  // Per-plugin in-flight tracker so two plugins can install in parallel
  // without colliding, and a stuck install on plugin A doesn't disable
  // plugin B's button.
  let pendingSteeringInstalls = new SvelteSet<string>();

  // Whole-plugin steering install — the install model is coarser than
  // skills (no per-file picker; backend installs every steering file
  // declared by plugin.json or under the default `./steering/` path).
  // Surfaces results via the existing installMessage / installError
  // banners — same banners the skill flow uses so a single dismiss UX
  // covers both. forceInstall is the same checkbox driving installSelected
  // (deliberately shared — one global force toggle covers any install
  // action the user takes).
  async function installSteering(marketplace: string, plugin: string) {
    const key = pluginKey(marketplace, plugin);
    if (pendingSteeringInstalls.has(key)) return;
    pendingSteeringInstalls.add(key);
    installError = null;
    installMessage = null;
    installWarning = null;
    installStaleRefresh = null;

    try {
      const result = await commands.installPluginSteering(
        marketplace,
        plugin,
        forceInstall,
        projectPath
      );
      if (result.status === "ok") {
        const { installed, failed, warnings } = result.data;
        const parts: string[] = [];
        if (installed.length > 0) {
          const noun = installed.length === 1 ? "file" : "files";
          parts.push(`Installed ${installed.length} steering ${noun}`);
        }
        if (failed.length > 0) {
          parts.push(
            `Failed: ${failed.map((f) => `${f.source} (${f.error})`).join(", ")}`
          );
        }
        if (warnings.length > 0) {
          parts.push(`Warnings: ${warnings.map(formatSteeringWarning).join(", ")}`);
        }

        // Mirror installSelected's hadSuccess ladder: any install OR
        // any skip-equivalent (here, "had warnings but nothing failed")
        // is a success-ish outcome → installMessage. Pure-failure with
        // zero installs lands in installError so the red banner matches
        // the actual outcome.
        const hadSuccess = installed.length > 0;
        if (parts.length === 0) {
          // No installed, no failed, no warnings — plugin declares no
          // steering or every file was already idempotent. Tell the
          // user something rather than letting the click feel inert.
          installMessage = `Steering for ${plugin}: nothing to install`;
        } else if (!hadSuccess && failed.length > 0) {
          installError = `Steering for ${plugin}: ${parts.join(" | ")}`;
        } else if (!hadSuccess && warnings.length > 0) {
          // Warnings-only with zero installs reads as misleading-success
          // in a green banner. Prefix to disambiguate.
          installMessage = `Steering for ${plugin}: No files installed — ${parts.join(" | ")}`;
        } else {
          installMessage = `Steering for ${plugin}: ${parts.join(" | ")}`;
        }
      } else {
        installError = `Steering install failed for ${plugin}: ${result.error.message}`;
      }
    } catch (e) {
      const reason = e instanceof Error ? e.message : String(e);
      installError = `Steering install failed for ${plugin}: ${reason}`;
    } finally {
      pendingSteeringInstalls.delete(key);
    }
  }

  // Plugin install/update delegated to lib/plugin-actions.ts — see PluginActionContext
  // doc-comment for the banner contract and MCP-gating rationale.
  async function runPluginInstall(
    marketplace: string,
    plugin: string,
    mode: BrowseAction,
  ) {
    const key = pluginKey(marketplace, plugin);
    if (pendingPluginActions.has(key)) return;
    pendingPluginActions.set(key, mode);
    installError = null;
    installMessage = null;
    installWarning = null;
    installStaleRefresh = null;
    installResult = null;
    installResultPlugin = null;

    try {
      // Switch over BrowseAction so a future arm becomes a compile error
      // here rather than silently constructing `{kind: "update"}`. Matches
      // the exhaustiveness convention used in plugin-actions.ts and format.ts.
      let modeArg: PluginActionMode;
      switch (mode) {
        case "install":
          modeArg = { kind: "install", force: forceInstall };
          break;
        case "update":
          modeArg = { kind: "update" };
          break;
        default: {
          const _exhaustive: never = mode;
          throw new Error(
            `unhandled BrowseAction in runPluginInstall: ${JSON.stringify(_exhaustive)}`,
          );
        }
      }

      const outcome = await doPluginInstall(
        {
          marketplace,
          plugin,
          projectPath,
          acceptMcp: false,
          refresh: () => fetchInstalledPlugins(),
          installPlugin: commands.installPlugin,
          storeRefresh: (p) => pluginUpdates.refresh(p),
        },
        modeArg,
      );

      if (outcome.kind === "ok") {
        const p = outcome.banner.primary;
        installError = p?.kind === "error" ? p.text : null;
        installMessage = p?.kind === "message" ? p.text : null;
        installWarning = outcome.banner.warning;
        installStaleRefresh = outcome.banner.staleRefresh;
        const failureSum =
          outcome.installResult.skills.failed.length +
          outcome.installResult.steering.failed.length +
          outcome.installResult.agents.failed.length;
        if (failureSum > 0) {
          installResult = outcome.installResult;
          installResultPlugin = plugin;
        }
      } else {
        installError = outcome.error;
      }
    } finally {
      pendingPluginActions.delete(key);
    }
  }

  // Steering install operates on whole plugins, so the natural scope is
  // "exactly one plugin in focus." Sources of focus, in priority order:
  //   1. A plugin filter is active (visible chip at the top of the page).
  //   2. Selected skills all belong to the same plugin (shared selection).
  // Anything else (no scope, multiple plugins) disables the action — no
  // batch UI for now, no first-plugin-wins ambiguity.
  let singlePluginInScope = $derived.by(() => {
    const keys = new Set<string>();
    for (const k of selectedPlugins) keys.add(k);
    for (const k of selectedSkills) {
      const { marketplace, plugin } = parseSkillKey(k);
      keys.add(pluginKey(marketplace, plugin));
    }
    if (keys.size !== 1) return null;
    const [only] = keys;
    return parsePluginKey(only);
  });

  let steeringInstallPending = $derived(
    singlePluginInScope !== null &&
      pendingSteeringInstalls.has(
        pluginKey(singlePluginInScope.marketplace, singlePluginInScope.plugin)
      )
  );

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
    fetchInstalledPlugins();
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
                  <span
                    class="text-[11px] {ap.plugin.skill_count.state === 'manifest_failed' ? 'text-kiro-warning' : 'text-kiro-subtle'}"
                    title={skillCountTitle(ap.plugin.skill_count)}
                    aria-label={skillCountTitle(ap.plugin.skill_count)}
                  >{skillCountLabel(ap.plugin.skill_count)}</span>
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

  <BannerStack
    errors={fetchErrors}
    message={installMessage}
    warning={installWarning}
    staleRefresh={installStaleRefresh}
    fatalError={installError}
    errLabel={errLabel}
    ondismiss={(key) => fetchErrors.delete(key)}
    onmessageDismiss={() => (installMessage = null)}
    onwarningDismiss={() => (installWarning = null)}
    onstaleRefreshDismiss={() => (installStaleRefresh = null)}
    onfatalErrorDismiss={() => (installError = null)}
  />

  {#if installResult && installResultPlugin && installResultHasFailures}
    {@const anyInstalled =
      installResult.skills.installed.length +
        installResult.steering.installed.length +
        installResult.agents.installed.length >
      0}
    <div
      class="mx-4 mt-3 px-4 py-3 rounded-md text-sm flex items-start gap-3
        {anyInstalled
          ? 'bg-kiro-warning/10 border border-kiro-warning/30 text-kiro-warning'
          : 'bg-kiro-error/10 border border-kiro-error/30 text-kiro-error'}"
    >
      <details
        class="flex-1"
        open
      >
        <summary class="cursor-pointer text-xs opacity-85">
          Show failures
        </summary>
        <div class="mt-2 pl-3 border-l-2 border-current/40 text-xs space-y-1">
          <!-- Index keys throughout: FailedSkill.name and FailedSteeringFile.source
               are not structurally unique on the Rust side (service/mod.rs can push
               duplicate FailedSkill::RequestedButNotFound when a Names(_) filter
               contains the same name twice), and FailedAgent variants don't share a
               single identity field. Svelte's {#each} silently dedupes on key
               collision — index keys eliminate that drop risk. -->
          {#each installResult.skills.failed as f, i (i)}
            <div><b>Skill failed:</b> {formatFailedSkill(f)}</div>
          {/each}
          {#each installResult.steering.failed as f, i (i)}
            <div><b>Steering failed:</b> {formatFailedSteeringFile(f)}</div>
          {/each}
          {#each installResult.agents.failed as f, i (i)}
            <div><b>Agent failed:</b> {formatFailedAgent(f)}</div>
          {/each}
        </div>
      </details>
      <button
        type="button"
        onclick={() => { installResult = null; installResultPlugin = null; }}
        aria-label="Dismiss install failures"
        class="opacity-70 hover:opacity-100 text-lg leading-none flex-shrink-0 focus:outline-none focus:ring-2 focus:ring-kiro-accent-500 rounded"
      >
        ×
      </button>
    </div>
  {/if}

  <div class="flex items-center gap-1 px-4 py-2 border-b border-kiro-muted bg-kiro-surface/30">
    <div class="inline-flex rounded-md border border-kiro-muted bg-kiro-overlay overflow-hidden">
      <button
        type="button"
        aria-pressed={browseView === "plugins"}
        onclick={() => (browseView = "plugins")}
        class="px-3 py-1.5 text-xs font-medium transition-colors
          {browseView === 'plugins'
            ? 'bg-kiro-accent-900/30 text-kiro-accent-300'
            : 'text-kiro-text-secondary hover:bg-kiro-muted hover:text-kiro-text'}"
      >
        Plugins
      </button>
      <button
        type="button"
        aria-pressed={browseView === "skills"}
        onclick={() => (browseView = "skills")}
        class="px-3 py-1.5 text-xs font-medium transition-colors border-l border-kiro-muted
          {browseView === 'skills'
            ? 'bg-kiro-accent-900/30 text-kiro-accent-300'
            : 'text-kiro-text-secondary hover:bg-kiro-muted hover:text-kiro-text'}"
      >
        Skills
      </button>
    </div>
  </div>

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
    {:else if browseView === "skills"}
      {#if filteredSkills.length === 0}
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
              No skills available — try the Plugins view to install plugins that ship steering.
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
    {:else}
      <!-- browseView === "plugins" -->
      {#if availablePlugins.length === 0}
        <div class="flex flex-col items-center justify-center h-full text-kiro-subtle gap-3">
          <p class="text-sm">No plugins available — pick a marketplace from Filters.</p>
        </div>
      {:else}
        <div class="grid gap-3 grid-cols-1 lg:grid-cols-2">
          {#each availablePlugins as ap (pluginKey(ap.marketplace, ap.plugin.name))}
            {@const key = pluginKey(ap.marketplace, ap.plugin.name)}
            <PluginCard
              plugin={ap.plugin}
              marketplace={ap.marketplace}
              installed={installedPluginKeys.has(key)}
              pending={pendingPluginActions.get(key)}
              update={pluginUpdates.updateFor(ap.marketplace, ap.plugin.name)}
              failure={pluginUpdates.failureFor(ap.marketplace, ap.plugin.name)}
              projectPicked={!!projectPath}
              onInstall={() => runPluginInstall(ap.marketplace, ap.plugin.name, "install")}
              onUpdate={() => runPluginInstall(ap.marketplace, ap.plugin.name, "update")}
            />
          {/each}
        </div>
      {/if}
    {/if}
  </div>

  <div class="p-4 border-t border-kiro-muted bg-kiro-surface flex items-center justify-between gap-3">
    <label class="flex items-center gap-2 text-sm text-kiro-text-secondary">
      <input
        type="checkbox"
        bind:checked={forceInstall}
        class="h-4 w-4 rounded border-kiro-muted text-kiro-accent-500 focus:ring-kiro-accent-500"
      />
      Force reinstall
    </label>
    <div class="flex items-center gap-2">
      <button
        type="button"
        class="px-4 py-2 text-sm font-medium rounded-md transition-colors duration-150
          {singlePluginInScope && projectPath && !installing && !steeringInstallPending
            ? 'bg-kiro-overlay text-kiro-accent-300 border border-kiro-muted hover:bg-kiro-muted hover:text-kiro-accent-200'
            : 'bg-kiro-muted text-kiro-subtle border border-transparent cursor-not-allowed'}"
        disabled={!singlePluginInScope || !projectPath || installing || steeringInstallPending}
        title={!projectPath
          ? "Pick a project first"
          : !singlePluginInScope
          ? "Filter to one plugin (or select skills from one plugin) to install its steering"
          : `Install steering files for ${singlePluginInScope.plugin}`}
        aria-busy={steeringInstallPending}
        onclick={() => {
          if (singlePluginInScope) {
            installSteering(singlePluginInScope.marketplace, singlePluginInScope.plugin);
          }
        }}
      >
        {steeringInstallPending
          ? "Installing steering…"
          : singlePluginInScope
          ? `Install steering for ${singlePluginInScope.plugin}`
          : "Install steering"}
      </button>
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
</div>

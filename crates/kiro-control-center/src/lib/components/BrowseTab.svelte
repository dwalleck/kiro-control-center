<script lang="ts">
  import { onMount } from "svelte";
  import { SvelteMap, SvelteSet } from "svelte/reactivity";
  import { commands } from "$lib/bindings";
  import {
    formatSkippedSkill,
    formatSkippedItemsForPlugin,
    formatSteeringWarning,
    formatInstallWarning,
    formatFailedSkill,
    formatFailedSteeringFile,
    formatFailedAgent,
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
    SkillInfo,
    SkippedSkill,
  } from "$lib/bindings";
  import BannerStack from "./BannerStack.svelte";
  import SkillCard from "./SkillCard.svelte";
  import PluginCard from "./PluginCard.svelte";
  import CustomizeDrawer from "./CustomizeDrawer.svelte";

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

  // Detects the install-time cross-plugin ownership-conflict failure
  // class so the UI can surface the forceInstall remediation hint.
  // The Rust side surfaces these as SteeringError::PathOwnedByOtherPlugin
  // and the analogous agent / companion-bundle variants, all of which
  // carry the suffix "pass --force to transfer ownership" in their
  // Display impls.
  //
  // Substring-matching the rendered error is a temporary heuristic
  // until the underlying error types gain structured `kind` fields.
  // CLAUDE.md explicitly prefers typed enum variants over reason-String
  // sentinels when callers branch on the semantic — which is what this
  // function does. The structural fix is tracked: for steering, by the
  // FailedSteeringFile tagged-enum restructure in rivets-xzrk; for the
  // skill side, by the harmonization task in rivets-deph. Once both
  // land, this function disappears in favor of `f.kind === "ownership_conflict"`
  // checks against the typed variants.
  //
  // The detection covers BOTH banner surfaces:
  //   - installResult — set by the whole-plugin Install button via
  //     runPluginInstall → commands.installPlugin → InstallPluginResult.
  //   - installError / installMessage — set by the drawer's Apply path
  //     (applyDrawerDiff composes per-category failure strings into
  //     these).
  function failureMentionsOwnership(s: string): boolean {
    return s.includes("--force to transfer");
  }
  let hasOwnershipConflict = $derived.by(() => {
    if (installResult !== null) {
      for (const f of installResult.skills.failed) {
        if (failureMentionsOwnership(f.error)) return true;
      }
      for (const f of installResult.steering.failed) {
        if (failureMentionsOwnership(f.error)) return true;
      }
      for (const f of installResult.agents.failed) {
        // FailedAgent variants either carry an `error: string` field
        // (agent / unparseable_agent / companion_bundle) or no error
        // (requested_but_not_found). The ownership-conflict variants
        // are all in the error-bearing set, so missing-error variants
        // can't trigger this signal.
        switch (f.kind) {
          case "agent":
          case "unparseable_agent":
          case "companion_bundle":
            if (failureMentionsOwnership(f.error)) return true;
            break;
          case "requested_but_not_found":
            break;
          default: {
            const _exhaustive: never = f;
            throw new Error(
              `unhandled FailedAgent variant: ${JSON.stringify(_exhaustive)}`,
            );
          }
        }
      }
    }
    if (installError !== null && failureMentionsOwnership(installError)) return true;
    if (installMessage !== null && failureMentionsOwnership(installMessage)) return true;
    return false;
  });

  // Slice 4: customize-drawer host. Holds the entry whose drawer is
  // open, or null when closed. The drawer derives selectedSkills from
  // the entry's per-item flags at mount, so a fresh-from-cache entry
  // is the source of truth — `applyDrawerDiff` re-fetches the catalog
  // before re-opening to avoid showing stale flags after an Apply.
  let drawerEntry: PluginCatalogEntryView | null = $state(null);
  let drawerMarketplace: string | null = $state(null);

  function openDrawer(marketplace: string, entry: PluginCatalogEntryView) {
    drawerMarketplace = marketplace;
    drawerEntry = entry;
  }

  function closeDrawer() {
    drawerEntry = null;
    drawerMarketplace = null;
  }

  // The Plugins-view grid pairs each catalog entry with its
  // marketplace name. PluginCard consumes the full catalog entry
  // directly so it can derive its three-state stripe and "X of Y
  // installed" badge from per-item flags. Broken-manifest plugins
  // live in view.skipped and surface as banners (see fetchCatalogFor);
  // they don't reach this derived list.
  let availablePlugins = $derived.by(() => {
    const out: { marketplace: string; entry: PluginCatalogEntryView }[] = [];
    for (const mp of selectedMarketplaces) {
      const view = catalogByMarketplace[mp];
      if (view === undefined) continue;
      for (const entry of view.plugins) {
        out.push({ marketplace: mp, entry });
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
          // marketplace is replaced wholesale. Two banner sources share
          // the catalog-skipped<mp><plugin> key:
          //
          //   1. Plugin-level skips (view.skipped) — the plugin couldn't
          //      be enumerated at all (missing dir, malformed manifest,
          //      remote source).
          //   2. Per-item skips (entry.skipped_items inside a working
          //      plugin) — restored here in slice 2 follow-up after the
          //      cache swap silently dropped them.
          //
          // The two sources are disjoint: a plugin is either in
          // view.skipped OR in view.plugins, never both, so they can
          // share the banner key without collision. Build the union of
          // fresh keys, then sweep stale entries — mirrors the prior
          // fetchAllSkillsForMarketplace ownership boundary.
          const fresh = new Set<ErrorSource>();
          for (const sp of view.skipped) {
            const k = catalogSkippedKey(mp, sp.name);
            fetchErrors.set(k, `${mp}/${sp.name}: ${sp.reason}`);
            fresh.add(k);
          }
          for (const entry of view.plugins) {
            if (entry.skipped_items.length === 0) continue;
            const k = catalogSkippedKey(mp, entry.plugin);
            fetchErrors.set(
              k,
              `${mp}/${entry.plugin}: ${formatSkippedItemsForPlugin(entry.skipped_items)}`,
            );
            fresh.add(k);
          }
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

  // Apply the customize drawer's diff across all three categories
  // (skills, steering, agents). kiro-zx73 added the four Tauri
  // commands the steering and agent paths use:
  //   - commands.installSteeringFiles + commands.removeSteeringFile
  //   - commands.installAgents + commands.removeAgent
  // Each category's install is a single batch call; removes loop one
  // call per item. Per-category outcomes flow into a single combined
  // summary banner — no separate banner family per category, matching
  // installSelected's "one banner per user action" UX.
  //
  // Refreshes the catalog before closing so the next drawer open
  // sees fresh per-item flags.
  async function applyDrawerDiff(
    marketplace: string,
    plugin: string,
    diff: {
      skills: { install: string[]; remove: string[] };
      steering: { install: string[]; remove: string[] };
      agents: { install: string[]; remove: string[] };
    },
  ) {
    installError = null;
    installMessage = null;
    installWarning = null;
    installStaleRefresh = null;

    // Per-category outcome accumulators. Each install batch returns
    // installed + failed lists; each remove loop tracks success count
    // and per-name failures. Skills additionally surface "skipped"
    // (already-installed) and "unreadable" (per-skill parse failures
    // that landed before install attempt).
    const skillsInstalled: string[] = [];
    const skillsSkipped: string[] = [];
    const skillsFailed: { name: string; error: string }[] = [];
    const skillsUnreadable: SkippedSkill[] = [];
    const steeringInstalled: string[] = [];
    const steeringFailed: { name: string; error: string }[] = [];
    // Steering/agent install paths surface non-fatal warnings
    // (scan_path_invalid, unmapped_tool, mcp_servers_require_opt_in,
    // agent_parse_failed) on r.data.warnings. The drawer used to drop
    // these — a user who hit an MCP gate saw "0 installed, 0 failed"
    // with no signal an opt-in was required. Accumulate per-category
    // so the summary banner surfaces them alongside failures.
    const steeringWarnings: import("$lib/bindings").SteeringWarning[] = [];
    const agentsWarnings: import("$lib/bindings").InstallWarning[] = [];
    const agentsInstalled: string[] = [];
    const agentsSkipped: string[] = [];
    const agentsFailed: { name: string; error: string }[] = [];
    let skillsRemoved = 0;
    const skillsRemoveFailed: { name: string; error: string }[] = [];
    let steeringRemoved = 0;
    const steeringRemoveFailed: { name: string; error: string }[] = [];
    let agentsRemoved = 0;
    const agentsRemoveFailed: { name: string; error: string }[] = [];

    // Per-category install — fire each as an independent Promise so a
    // failure on one category doesn't abort the others. Each helper
    // returns true on completion (success or partial failure surfaced
    // in the accumulators) or sets installError + returns false on a
    // hard wrapper-level error (the entire batch never ran).
    async function installSkillsBatch(): Promise<boolean> {
      if (diff.skills.install.length === 0) return true;
      try {
        const r = await commands.installSkills(
          marketplace,
          plugin,
          diff.skills.install,
          forceInstall,
          projectPath,
        );
        if (r.status === "ok") {
          skillsInstalled.push(...r.data.installed);
          skillsSkipped.push(...r.data.skipped);
          skillsFailed.push(...r.data.failed);
          skillsUnreadable.push(...r.data.skipped_skills);
          return true;
        }
        installError = `Customize apply: skill install failed for ${marketplace}/${plugin}: ${r.error.message}`;
        return false;
      } catch (e) {
        const reason = e instanceof Error ? e.message : String(e);
        installError = `Customize apply: skill install threw for ${marketplace}/${plugin}: ${reason}`;
        return false;
      }
    }
    async function installSteeringBatch(): Promise<boolean> {
      if (diff.steering.install.length === 0) return true;
      try {
        const r = await commands.installSteeringFiles(
          marketplace,
          plugin,
          diff.steering.install,
          forceInstall,
          projectPath,
        );
        if (r.status === "ok") {
          // Use the destination basename — that's the user-facing
          // identifier (matches the catalog's SteeringItemInfo.name
          // and lives under .kiro/steering/ which has the canonical
          // separator). `out.source` is the on-disk source path with
          // mixed `\` / `/` separators because joining a Windows
          // marketplace base with a Unix-style relative path doesn't
          // normalize separators — surfacing it would render an
          // awkward "C:\...kiro-starter-kit\./plugins/..." string in
          // the success banner. Same fix on the failed side.
          for (const out of r.data.installed) {
            steeringInstalled.push(basenameOf(out.destination));
          }
          for (const f of r.data.failed) {
            steeringFailed.push({
              name: basenameOf(f.source.toString()),
              error: f.error,
            });
          }
          if (r.data.warnings) {
            steeringWarnings.push(...r.data.warnings);
          }
          return true;
        }
        installError = `Customize apply: steering install failed for ${marketplace}/${plugin}: ${r.error.message}`;
        return false;
      } catch (e) {
        const reason = e instanceof Error ? e.message : String(e);
        installError = `Customize apply: steering install threw for ${marketplace}/${plugin}: ${reason}`;
        return false;
      }
    }
    async function installAgentsBatch(): Promise<boolean> {
      if (diff.agents.install.length === 0) return true;
      try {
        const r = await commands.installAgents(
          marketplace,
          plugin,
          diff.agents.install,
          forceInstall,
          /* acceptMcp */ false,
          projectPath,
        );
        if (r.status === "ok") {
          agentsInstalled.push(...r.data.installed);
          agentsSkipped.push(...r.data.skipped);
          for (const f of r.data.failed) {
            agentsFailed.push(formatFailedAgentForBanner(f));
          }
          if (r.data.warnings) {
            agentsWarnings.push(...r.data.warnings);
          }
          return true;
        }
        installError = `Customize apply: agent install failed for ${marketplace}/${plugin}: ${r.error.message}`;
        return false;
      } catch (e) {
        const reason = e instanceof Error ? e.message : String(e);
        installError = `Customize apply: agent install threw for ${marketplace}/${plugin}: ${reason}`;
        return false;
      }
    }

    // Run all three install batches in parallel. Promise.all here
    // because each commands.* call is independent (different tracking
    // files, no shared lock). If any returns false, installError is
    // already set; we still continue to the remove loops since those
    // are user-requested removals that shouldn't block on an install
    // failure of a different category.
    await Promise.all([
      installSkillsBatch(),
      installSteeringBatch(),
      installAgentsBatch(),
    ]);

    // Per-category remove loops in parallel. Each item is its own
    // Tauri call (no batch remove API); the inner loop is sequential
    // because the tracking-file lock per category is exclusive.
    async function removeSkillsLoop() {
      for (const name of diff.skills.remove) {
        try {
          const r = await commands.removeSkill(name, projectPath);
          if (r.status === "ok") skillsRemoved++;
          else skillsRemoveFailed.push({ name, error: r.error.message });
        } catch (e) {
          skillsRemoveFailed.push({
            name,
            error: e instanceof Error ? e.message : String(e),
          });
        }
      }
    }
    async function removeSteeringLoop() {
      for (const name of diff.steering.remove) {
        try {
          const r = await commands.removeSteeringFile(name, projectPath);
          if (r.status === "ok") steeringRemoved++;
          else steeringRemoveFailed.push({ name, error: r.error.message });
        } catch (e) {
          steeringRemoveFailed.push({
            name,
            error: e instanceof Error ? e.message : String(e),
          });
        }
      }
    }
    async function removeAgentsLoop() {
      for (const name of diff.agents.remove) {
        try {
          const r = await commands.removeAgent(name, projectPath);
          if (r.status === "ok") agentsRemoved++;
          else agentsRemoveFailed.push({ name, error: r.error.message });
        } catch (e) {
          agentsRemoveFailed.push({
            name,
            error: e instanceof Error ? e.message : String(e),
          });
        }
      }
    }
    await Promise.all([removeSkillsLoop(), removeSteeringLoop(), removeAgentsLoop()]);

    // Compose summary banner. Per-category totals so a mixed apply
    // reads like "Installed: 2 skills, 1 agent | Removed: 1 steering".
    const installedParts: string[] = [];
    if (skillsInstalled.length > 0) {
      installedParts.push(
        `${skillsInstalled.length} skill${skillsInstalled.length === 1 ? "" : "s"} (${skillsInstalled.join(", ")})`,
      );
    }
    if (steeringInstalled.length > 0) {
      installedParts.push(
        `${steeringInstalled.length} steering (${steeringInstalled.join(", ")})`,
      );
    }
    if (agentsInstalled.length > 0) {
      installedParts.push(
        `${agentsInstalled.length} agent${agentsInstalled.length === 1 ? "" : "s"} (${agentsInstalled.join(", ")})`,
      );
    }
    const skippedTotal = skillsSkipped.length + agentsSkipped.length;
    const removedTotal = skillsRemoved + steeringRemoved + agentsRemoved;
    const failedAll = [
      ...skillsFailed,
      ...steeringFailed,
      ...agentsFailed,
      ...skillsRemoveFailed,
      ...steeringRemoveFailed,
      ...agentsRemoveFailed,
    ];

    const parts: string[] = [];
    if (installedParts.length > 0) parts.push(`Installed: ${installedParts.join(" | ")}`);
    if (skippedTotal > 0) parts.push(`Already installed: ${skippedTotal}`);
    if (removedTotal > 0) parts.push(`Removed: ${removedTotal}`);
    if (failedAll.length > 0) {
      parts.push(`Failed: ${failedAll.map((f) => `${f.name} (${f.error})`).join(", ")}`);
    }
    if (skillsUnreadable.length > 0) {
      parts.push(`Unreadable: ${skillsUnreadable.map(formatSkippedSkill).join(", ")}`);
    }
    // Non-fatal warnings flow into a dedicated banner section so an
    // MCP-opt-in gate (agentsWarnings) or scan_path_invalid
    // (steeringWarnings) isn't silently dropped. Mirrors how the
    // whole-plugin path's runPluginInstall flow renders warnings
    // (see runPluginInstall → outcome.banner.warning around line
    // 1137).
    if (steeringWarnings.length > 0 || agentsWarnings.length > 0) {
      const warningStrs = [
        ...steeringWarnings.map(formatSteeringWarning),
        ...agentsWarnings.map(formatInstallWarning),
      ];
      parts.push(`Warnings: ${warningStrs.join(" | ")}`);
    }

    const hadSuccess =
      skillsInstalled.length > 0
      || steeringInstalled.length > 0
      || agentsInstalled.length > 0
      || removedTotal > 0;
    if (parts.length === 0) {
      installMessage = `${marketplace}/${plugin}: no changes applied`;
    } else if (!hadSuccess && failedAll.length > 0) {
      installError = `${marketplace}/${plugin}: ${parts.join(" | ")}`;
    } else if (installError === null) {
      // Don't overwrite a hard wrapper error from one of the install
      // batches — those already set installError for the user.
      installMessage = `${marketplace}/${plugin}: ${parts.join(" | ")}`;
    }

    // Refresh catalog so the drawer's next open sees fresh per-item
    // installed flags. Closing happens AFTER refresh so a re-open
    // from BrowseTab (rare, but possible if the user clicks Manage
    // again immediately) doesn't read stale state.
    try {
      await fetchCatalogFor(marketplace, true);
    } catch (e) {
      console.error("[BrowseTab] post-drawer-apply refresh rejected", e);
      const reason = e instanceof Error ? e.message : String(e);
      installError = `Post-apply refresh failed: ${reason}`;
    }
    closeDrawer();
  }

  // Project a FailedAgent into a flat (name, error) shape for the
  // banner accumulator. Mirrors formatFailedAgent's switch but pulls
  // out the name field directly so the banner reads consistently
  // alongside skills/steering failure lists.
  // Cross-platform path basename. The wire shape from the Rust side
  // is a String, but `out.source` and `out.destination` for steering
  // can carry mixed `\` (Windows base) and `/` (Unix-style relative)
  // separators because the marketplace cache root is a Windows
  // PathBuf and the manifest's scan paths are Unix-style strings —
  // PathBuf::join doesn't normalize. Splitting on either separator
  // and taking the last non-empty segment gives the canonical
  // basename regardless of how the source path was assembled.
  function basenameOf(path: string): string {
    const segments = path.split(/[\\/]/).filter((seg) => seg.length > 0);
    return segments.length > 0 ? segments[segments.length - 1] : path;
  }

  function formatFailedAgentForBanner(
    f: import("$lib/bindings").FailedAgent_Serialize,
  ): { name: string; error: string } {
    switch (f.kind) {
      case "agent":
        return { name: f.name, error: f.error };
      case "unparseable_agent":
        // source_path is the file we couldn't parse — show the
        // basename for banner brevity (same separator-normalization
        // story as basenameOf's doc).
        return { name: basenameOf(f.source_path.toString()), error: f.error };
      case "companion_bundle":
        return { name: `${f.plugin} (companion bundle)`, error: f.error };
      case "requested_but_not_found":
        return { name: f.name, error: `not found in plugin ${f.plugin}` };
      default: {
        const _exhaustive: never = f;
        throw new Error(
          `unhandled FailedAgent variant in formatFailedAgentForBanner: ${JSON.stringify(_exhaustive)}`,
        );
      }
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
              {#each availablePlugins as ap (pluginKey(ap.marketplace, ap.entry.plugin))}
                {@const key = pluginKey(ap.marketplace, ap.entry.plugin)}
                {@const skillLabel = `${ap.entry.skills.length} ${ap.entry.skills.length === 1 ? "skill" : "skills"}`}
                <label class="flex items-center gap-2 px-1.5 py-1 text-[13px] text-kiro-text-secondary rounded hover:bg-kiro-accent-900/15 hover:text-kiro-text cursor-pointer">
                  <input
                    type="checkbox"
                    checked={selectedPlugins.has(key)}
                    onchange={() => togglePlugin(key)}
                    class="h-3.5 w-3.5 rounded border-kiro-muted text-kiro-accent-500"
                  />
                  <span class="flex-1 truncate">{ap.entry.plugin}</span>
                  <span
                    class="text-[11px] text-kiro-subtle"
                    title={skillLabel}
                    aria-label={skillLabel}
                  >{skillLabel}</span>
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

  {#if hasOwnershipConflict}
    <!--
      Standalone ownership-transfer hint. Appears whenever an install
      surfaces "owned by another plugin" failures, regardless of which
      banner surface produced them (installResult inline panel from
      runPluginInstall OR installError/installMessage from the drawer's
      applyDrawerDiff path). Without the standalone render, the
      drawer-driven failures didn't get the hint because the inline
      installResult panel above never rendered for that path.
      The button just flips the existing forceInstall toggle — the
      user still consciously re-clicks Install / Apply (no auto-retry)
      so the ownership-transfer decision stays explicit.
    -->
    <div
      class="mx-4 mt-3 px-4 py-2.5 rounded-md text-sm bg-kiro-info/[0.10] text-kiro-info border border-kiro-info/30 leading-relaxed"
    >
      <strong>Some items belong to another plugin</strong> — likely a
      previous version installed under a different plugin name. To
      transfer ownership to this plugin, enable
      <strong>Force reinstall</strong> and try the install again.
      {#if !forceInstall}
        <button
          type="button"
          onclick={() => (forceInstall = true)}
          class="ml-1 underline cursor-pointer hover:opacity-100 opacity-90 bg-transparent border-none p-0 text-current text-sm"
        >
          Enable Force Reinstall now
        </button>
      {:else}
        <span class="ml-1 italic opacity-90">
          (Force Reinstall is on — re-run Install / Apply.)
        </span>
      {/if}
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
          {#each availablePlugins as ap (pluginKey(ap.marketplace, ap.entry.plugin))}
            {@const key = pluginKey(ap.marketplace, ap.entry.plugin)}
            <PluginCard
              entry={ap.entry}
              marketplace={ap.marketplace}
              installed={installedPluginKeys.has(key)}
              pending={pendingPluginActions.get(key)}
              update={pluginUpdates.updateFor(ap.marketplace, ap.entry.plugin)}
              failure={pluginUpdates.failureFor(ap.marketplace, ap.entry.plugin)}
              projectPicked={!!projectPath}
              onInstall={() => runPluginInstall(ap.marketplace, ap.entry.plugin, "install")}
              onUpdate={() => runPluginInstall(ap.marketplace, ap.entry.plugin, "update")}
              onCustomize={() => openDrawer(ap.marketplace, ap.entry)}
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

<!--
  Customize drawer host. Rendered outside the main BrowseTab flex
  column so its position:fixed overlay isn't constrained by the
  flex parent. The drawer's onApply receives a skill-only diff
  (Option A — kiro-zx73 widens to per-item steering/agents).
-->
{#if drawerEntry && drawerMarketplace}
  <CustomizeDrawer
    entry={drawerEntry}
    marketplace={drawerMarketplace}
    onClose={closeDrawer}
    onApply={(diff) => applyDrawerDiff(drawerMarketplace!, drawerEntry!.plugin, diff)}
  />
{/if}

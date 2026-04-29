<script lang="ts">
  import { onMount } from "svelte";
  import { SvelteMap, SvelteSet } from "svelte/reactivity";
  import { commands } from "$lib/bindings";
  import {
    formatSkippedSkill,
    formatSkippedSkillsForPlugin,
    skillCountLabel,
    skillCountTitle,
  } from "$lib/format";
  import type {
    MarketplaceInfo,
    PluginInfo,
    SkillInfo,
    SkillCount,
    SkippedReason,
    SkippedSkill,
    SteeringWarning,
  } from "$lib/bindings";
  import SkillCard from "./SkillCard.svelte";

  // Render a SteeringWarning. The explicit `default` arm with a `never`
  // binding is the load-bearing exhaustiveness guard — if a new
  // SteeringWarning variant lands in core (regenerating bindings.ts),
  // the assignment fails to compile because the new variant isn't `never`.
  // A return-type-narrowing approach (no default arm, function returns
  // `string`) would also catch it via TS2366, but only as long as the
  // return type stays non-`undefined` — refactoring to `void`-returning
  // logging would silently disable the check. The assertNever pattern
  // survives both.
  function formatSteeringWarning(w: SteeringWarning): string {
    switch (w.kind) {
      case "scan_path_invalid":
        return `invalid scan path '${w.path}': ${w.reason}`;
      case "scan_dir_unreadable":
        return `could not read steering dir '${w.path}': ${w.reason}`;
      default: {
        const _exhaustive: never = w;
        throw new Error(`unhandled SteeringWarning variant: ${JSON.stringify(_exhaustive)}`);
      }
    }
  }

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
  // named `plugins`, `skills`, or `bulk-skills` still produces a distinct
  // key from the namespace tag.
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

  async function fetchPluginsFor(mp: string) {
    const key = pluginsErrKey(mp);
    if (pendingFetches.has(key) || pluginsByMarketplace[mp]) return;
    await withFetchGuard(key, mp, () => commands.listPlugins(mp), {
      onSuccess: (data) => {
        pluginsByMarketplace[mp] = data;
        fetchErrors.delete(key);
      },
      onError: (message) => {
        fetchErrors.set(key, `${mp}: ${message}`);
      },
    });
  }

  async function fetchSkillsFor(mp: string, plugin: string, force = false) {
    const cacheKey = pluginKey(mp, plugin);
    const key = skillsErrKey(mp, plugin);
    if (pendingFetches.has(key)) return;
    if (!force && skillsByPluginPair[cacheKey]) return;
    await withFetchGuard(
      key,
      `${mp}/${plugin}`,
      () => commands.listAvailableSkills(mp, plugin, projectPath),
      {
        onSuccess: (data) => {
          // data is PluginSkillsResult: { skills, skipped_skills }.
          // Happy-path skills populate the card grid; per-skill read
          // failures surface via the same per-plugin fetchErrors banner
          // stream used by plugin-level skips (from the bulk path), so a
          // user who sees "plugin X" in the warning list can also see
          // "plugin Y has 2 skills that failed to load" on the same
          // panel. The set/delete branches are mutually exclusive: a
          // retry that fixes the malformed SKILL.md must clear the stale
          // banner, a retry that still has failures must replace it.
          skillsByPluginPair[cacheKey] = data.skills;
          if (data.skipped_skills.length > 0) {
            fetchErrors.set(
              key,
              `${mp}/${plugin}: ${formatSkippedSkillsForPlugin(data.skipped_skills)}`
            );
          } else {
            fetchErrors.delete(key);
          }
        },
        onError: (message) => {
          fetchErrors.set(key, `${mp}/${plugin}: ${message}`);
        },
      }
    );
  }

  // Bulk path: one backend call populates per-plugin cache entries for an
  // entire marketplace. Used when no plugin filter is active; a marketplace
  // with 50 plugins would otherwise fire 50 concurrent `listAvailableSkills`
  // calls on first paint. Plugins with zero skills get empty-array cache
  // entries so the per-pair guard in `fetchSkillsFor` doesn't re-fetch them
  // later if the user applies a plugin filter. Plugins the backend couldn't
  // load (missing dir, malformed manifest) come back in `skipped` and get
  // per-plugin error banners — avoids the silent-partial-listing footgun.
  async function fetchAllSkillsForMarketplace(
    mp: string,
    plugins: readonly { name: string }[]
  ) {
    const key = bulkSkillsErrKey(mp);
    if (plugins.length === 0 || pendingFetches.has(key)) return;
    const allCached = plugins.every(
      (p) => skillsByPluginPair[pluginKey(mp, p.name)] !== undefined
    );
    if (allCached) return;

    await withFetchGuard(
      key,
      mp,
      () => commands.listAllSkillsForMarketplace(mp, projectPath),
      {
        onSuccess: ({ skills: skillList, skipped, skipped_skills }) => {
          const byPlugin = new Map<string, SkillInfo[]>();
          for (const s of skillList) {
            const arr = byPlugin.get(s.plugin);
            if (arr) arr.push(s);
            else byPlugin.set(s.plugin, [s]);
          }
          const skippedNames = new Set(skipped.map((s) => s.name));

          // Group per-skill skips by plugin so each plugin gets one
          // warning banner regardless of how many of its skills failed.
          // Per-skill and plugin-level skips are disjoint (a plugin whose
          // directory fails to resolve never reaches the per-skill loop),
          // so the two banner streams don't collide on the same key.
          const skippedSkillsByPlugin = new Map<string, SkippedSkill[]>();
          for (const s of skipped_skills) {
            const arr = skippedSkillsByPlugin.get(s.plugin);
            if (arr) arr.push(s);
            else skippedSkillsByPlugin.set(s.plugin, [s]);
          }

          for (const p of plugins) {
            // Skipped plugins get NO cache entry so the per-plugin path can
            // still be tried if the user narrows to them — a retry might
            // succeed (e.g. if the manifest was hand-edited).
            if (skippedNames.has(p.name)) continue;
            skillsByPluginPair[pluginKey(mp, p.name)] = byPlugin.get(p.name) ?? [];
          }

          // Record per-plugin error banners for every skipped plugin.
          for (const s of skipped) {
            fetchErrors.set(skillsErrKey(mp, s.name), `${mp}/${s.name}: ${s.reason}`);
          }
          // Record per-plugin error banners for plugins with per-skill
          // read/parse failures — previously dropped silently (the exact
          // regression the code-review called out for the bulk path).
          for (const [plugin, list] of skippedSkillsByPlugin) {
            fetchErrors.set(
              skillsErrKey(mp, plugin),
              `${mp}/${plugin}: ${formatSkippedSkillsForPlugin(list)}`
            );
          }

          // The bulk response is authoritative for working plugins — clear
          // stale per-plugin skill errors for THIS mp EXCEPT for pairs whose
          // fetch is still in flight (racing write could clobber us) and
          // EXCEPT entries we just set above for skipped plugins OR plugins
          // with surfaced per-skill failures (both populate the same
          // banner key and must survive the post-bulk cleanup).
          const stale: ErrorSource[] = [];
          for (const k of fetchErrors.keys()) {
            if (!k.startsWith(SKILLS_ERR_PREFIX)) continue;
            const { marketplace, plugin } = parsePluginKey(k.slice(SKILLS_ERR_PREFIX.length));
            if (marketplace !== mp) continue;
            if (skippedNames.has(plugin)) continue;
            if (skippedSkillsByPlugin.has(plugin)) continue;
            if (pendingFetches.has(skillsErrKey(marketplace, plugin))) continue;
            stale.push(k);
          }
          for (const k of stale) fetchErrors.delete(k);

          // Clear the bulk-level error banner for this marketplace. The
          // per-plugin writes above target a different prefix, so this
          // does not collide with anything we just set.
          fetchErrors.delete(key);
        },
        onError: (message) => {
          fetchErrors.set(key, `${mp}: ${message}`);
        },
      }
    );
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

  // Skill caches and skill-fetch errors (both per-plugin `skills\u001f` and
  // marketplace-level `bulk-skills\u001f`) are project-scoped — `installed`
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
      {#if !filterText && availablePlugins.length > 0}
        <!-- Skills are zero but plugins exist (e.g., a marketplace whose
             plugins ship steering files but no skills). Surface the
             plugins inline so the user has a direct install path —
             without this they'd have to discover the filter popover to
             scope the bottom-bar steering button. -->
        <div class="flex flex-col gap-4">
          <p class="text-sm text-kiro-subtle">
            No skills in {selectedMarketplaces.size === 1 ? "this marketplace" : "these marketplaces"} —
            install steering for a plugin below.
          </p>
          <div class="flex flex-col gap-2">
            {#each availablePlugins as ap (pluginKey(ap.marketplace, ap.plugin.name))}
              {@const key = pluginKey(ap.marketplace, ap.plugin.name)}
              <div class="flex items-center gap-3 px-3 py-2.5 rounded-md border border-kiro-muted bg-kiro-overlay">
                <div class="flex-1 min-w-0">
                  <div class="text-sm font-medium text-kiro-text truncate">{ap.plugin.name}</div>
                  {#if ap.plugin.description}
                    <div class="text-xs text-kiro-subtle truncate">{ap.plugin.description}</div>
                  {/if}
                </div>
                <span
                  class="text-[11px] {ap.plugin.skill_count.state === 'manifest_failed' ? 'text-kiro-warning' : 'text-kiro-subtle'} flex-shrink-0"
                  title={skillCountTitle(ap.plugin.skill_count)}
                  aria-label={skillCountTitle(ap.plugin.skill_count)}
                >{skillCountLabel(ap.plugin.skill_count)} skill{ap.plugin.skill_count.state === "known" && ap.plugin.skill_count.count === 1 ? "" : "s"}</span>
                <button
                  type="button"
                  onclick={() => installSteering(ap.marketplace, ap.plugin.name)}
                  disabled={!projectPath || pendingSteeringInstalls.has(key)}
                  title={projectPath
                    ? `Install steering files for ${ap.plugin.name}`
                    : "Pick a project first"}
                  aria-label="Install steering files for {ap.plugin.name}"
                  aria-busy={pendingSteeringInstalls.has(key)}
                  class="px-3 py-1.5 text-xs font-medium rounded-md transition-colors flex-shrink-0
                    {projectPath && !pendingSteeringInstalls.has(key)
                      ? 'bg-kiro-overlay border border-kiro-muted text-kiro-accent-300 hover:bg-kiro-muted hover:text-kiro-accent-200'
                      : 'bg-kiro-muted text-kiro-subtle border border-transparent cursor-not-allowed'}"
                >
                  {pendingSteeringInstalls.has(key) ? "Installing…" : "Install steering"}
                </button>
              </div>
            {/each}
          </div>
        </div>
      {:else}
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
      {/if}
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

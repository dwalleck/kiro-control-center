<script lang="ts">
  import { onMount } from "svelte";
  import { SvelteMap } from "svelte/reactivity";
  import { commands } from "$lib/bindings";
  import type {
    InstalledSkillInfo,
    InstalledPluginInfo,
    PluginUpdateInfo,
    PluginUpdateFailure,
    RemovePluginResult,
  } from "$lib/bindings";
  import { DELIM, pluginKey } from "$lib/keys";
  import { pluginUpdates } from "$lib/stores/plugin-updates.svelte";
  import { kindLabel } from "$lib/stores/plugin-updates";
  import type { PluginAction } from "$lib/stores/plugin-updates";
  import { formatInstallPluginResult, formatRemovePluginResult } from "$lib/format";
  import BannerStack from "./BannerStack.svelte";

  let { projectPath }: { projectPath: string } = $props();

  let plugins: InstalledPluginInfo[] = $state([]);
  let skills: InstalledSkillInfo[] = $state([]);
  let loading: boolean = $state(true);
  // `loadError` narrows in 2b: only fetch/refresh failures land here.
  // Remove-action failures route to `installError` (matching BrowseTab).
  let loadError: string | null = $state(null);

  // 3-banner pattern mirrored from BrowseTab. installError = red fatal,
  // installMessage = green success, installWarning = amber non-fatal.
  let installError: string | null = $state(null);
  let installMessage: string | null = $state(null);
  let installWarning: string | null = $state(null);

  // Per-plugin in-flight tracker, keyed by pluginKey(marketplace, plugin).
  // Narrows the shared PluginAction union to the actions InstalledTab
  // performs (Remove + Update — never Install, that's BrowseTab's surface).
  let pendingPluginActions = new SvelteMap<string, Extract<PluginAction, "remove" | "update">>();

  // The most recent RemovePluginResult — drives the inline <details>
  // block below the BannerStack. Stays set until the next Remove or
  // a project change clears it.
  let removeResult: RemovePluginResult | null = $state(null);
  let removeResultPlugin: string | null = $state(null);
  let removeResultHasFailures: boolean = $state(false);

  // Banner-stack typing — InstalledTab's ErrorSource union is small for
  // 2b: only the new update-check + update-fetch keys plus a single
  // installed-plugins key for partial-load warnings (existing).
  const ERR_INSTALLED_PLUGINS = "installed-plugins" as const;
  const ERR_UPDATE_FETCH = "update-fetch" as const;
  const UPDATE_CHECK_PREFIX = "update-check" as const;
  type ErrorSource =
    | typeof ERR_INSTALLED_PLUGINS
    | typeof ERR_UPDATE_FETCH
    | `${typeof UPDATE_CHECK_PREFIX}${typeof DELIM}${string}${typeof DELIM}${string}`;
  let fetchErrors = new SvelteMap<ErrorSource, string>();

  function errLabel(key: ErrorSource): string {
    if (key === ERR_INSTALLED_PLUGINS) return "Dismiss installed-plugins warning";
    if (key === ERR_UPDATE_FETCH) return "Dismiss update-check error";
    if (key.startsWith(UPDATE_CHECK_PREFIX + DELIM)) {
      const parts = key.split(DELIM);
      if (parts.length === 3) {
        return `Dismiss update-check banner for ${parts[2]}`;
      }
      return "Dismiss update-check banner";
    }
    return "Dismiss banner";
  }

  async function refresh() {
    loading = true;
    loadError = null;
    try {
      const [pluginsResult, skillsResult] = await Promise.all([
        commands.listInstalledPlugins(projectPath),
        commands.listInstalledSkills(projectPath),
      ]);
      if (pluginsResult.status === "ok") {
        plugins = pluginsResult.data.plugins;
        const warnings = pluginsResult.data.partial_load_warnings ?? [];
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
        loadError = pluginsResult.error.message;
      }
      if (skillsResult.status === "ok") {
        skills = skillsResult.data;
      } else if (loadError === null) {
        loadError = skillsResult.error.message;
      }
    } catch (e) {
      const reason = e instanceof Error ? e.message : String(e);
      loadError = `Failed to load installed state: ${reason}`;
    } finally {
      loading = false;
    }
  }

  async function removePlugin(marketplace: string, plugin: string) {
    const key = pluginKey(marketplace, plugin);
    if (pendingPluginActions.has(key)) return;
    pendingPluginActions.set(key, "remove");
    installError = null;
    installMessage = null;
    installWarning = null;
    removeResult = null;
    removeResultPlugin = null;
    removeResultHasFailures = false;
    try {
      const result = await commands.removePlugin(marketplace, plugin, projectPath);
      if (result.status === "ok") {
        const { summary, hasItems, hasFailures } =
          formatRemovePluginResult(result.data, plugin);
        if (hasItems || hasFailures) {
          removeResult = result.data;
          removeResultPlugin = plugin;
          removeResultHasFailures = hasFailures;
        }
        if (hasFailures) {
          installWarning = `Removed plugin ${plugin}: ${summary}`;
        } else {
          installMessage = `Removed plugin ${plugin}: ${summary}`;
        }
        // Order: pluginUpdates.refresh first, local refresh second.
        await pluginUpdates.refresh(projectPath);
        await refresh();
      } else {
        installError = `Remove failed for ${plugin}: ${result.error.message}`;
      }
    } catch (e) {
      const reason = e instanceof Error ? e.message : String(e);
      installError = `Remove failed for ${plugin}: ${reason}`;
    } finally {
      pendingPluginActions.delete(key);
    }
  }

  async function updatePlugin(marketplace: string, plugin: string) {
    const key = pluginKey(marketplace, plugin);
    if (pendingPluginActions.has(key)) return;
    pendingPluginActions.set(key, "update");
    installError = null;
    installMessage = null;
    installWarning = null;
    removeResult = null;
    removeResultPlugin = null;
    try {
      const result = await commands.installPlugin(
        marketplace,
        plugin,
        /*force=*/ true,
        /*acceptMcp=*/ false,
        projectPath,
      );
      if (result.status === "ok") {
        const { summary, warnings, anyInstalled, anyFailed } =
          formatInstallPluginResult(result.data, plugin);
        if (anyFailed && !anyInstalled) {
          installError = `Update failed for ${plugin}: ${summary}`;
        } else {
          installMessage = `Updated ${plugin}: ${summary}`;
        }
        if (warnings) {
          installWarning = `Updated ${plugin}: ${warnings}`;
        }
        await pluginUpdates.refresh(projectPath);
        await refresh();
      } else {
        installError = `Update failed for ${plugin}: ${result.error.message}`;
      }
    } catch (e) {
      const reason = e instanceof Error ? e.message : String(e);
      installError = `Update failed for ${plugin}: ${reason}`;
    } finally {
      pendingPluginActions.delete(key);
    }
  }

  function formatDate(iso: string): string {
    const d = new Date(iso);
    return Number.isNaN(d.getTime()) ? iso : d.toLocaleString();
  }

  function contentSummary(p: InstalledPluginInfo): string {
    const parts: string[] = [];
    if (p.skill_count > 0) parts.push(`${p.skill_count} skill${p.skill_count === 1 ? "" : "s"}`);
    if (p.steering_count > 0) parts.push(`${p.steering_count} steering`);
    if (p.agent_count > 0) parts.push(`${p.agent_count} agent${p.agent_count === 1 ? "" : "s"}`);
    return parts.length > 0 ? parts.join(" · ") : "(empty)";
  }

  onMount(refresh);

  // Re-fetch when the project changes. Reading projectPath registers the
  // dependency; the void cast keeps lint happy about an unused expression.
  $effect(() => {
    void projectPath;
    refresh();
  });

  // Eager scan on project mount + on every projectPath change.
  $effect(() => {
    void projectPath;
    pluginUpdates.refresh(projectPath);
  });

  // Project per-marketplace failure groups into the fetchErrors map.
  $effect(() => {
    const seen = new Set<ErrorSource>();
    for (const group of pluginUpdates.failureGroups) {
      const key: ErrorSource =
        `${UPDATE_CHECK_PREFIX}${DELIM}${group.remediation}${DELIM}${group.marketplace}` as ErrorSource;
      seen.add(key);
      const noun = group.plugins.length === 1 ? "plugin" : "plugins";
      const list = group.plugins.join(", ");
      fetchErrors.set(
        key,
        `${group.plugins.length} ${noun} from ${group.marketplace}: ${group.remediationHint} (${list})`,
      );
    }
    for (const k of fetchErrors.keys()) {
      if (k.startsWith(UPDATE_CHECK_PREFIX + DELIM) && !seen.has(k)) {
        fetchErrors.delete(k);
      }
    }
  });

  // Surface the toplevel pluginUpdates.fetchError as its own banner.
  $effect(() => {
    if (pluginUpdates.fetchError) {
      fetchErrors.set(
        ERR_UPDATE_FETCH,
        `Couldn't check for updates: ${pluginUpdates.fetchError}`,
      );
    } else {
      fetchErrors.delete(ERR_UPDATE_FETCH);
    }
  });

  // Phase 2b I-1: clear stale action-result state on project change so the
  // <details> block, banners, and pending-action map don't leak across
  // project switches. The store's race-condition guard already handles
  // pluginUpdates.result; this effect handles InstalledTab-local state.
  let priorProjectPath: string | null = null;
  $effect(() => {
    if (priorProjectPath !== null && priorProjectPath !== projectPath) {
      removeResult = null;
      removeResultPlugin = null;
      removeResultHasFailures = false;
      installError = null;
      installMessage = null;
      installWarning = null;
      // Clear in-flight tracker — actions clear themselves via finally{}
      // under normal operation, but a project change while a Remove or
      // Update is mid-flight would leave a stale entry that disables the
      // row's buttons indefinitely.
      pendingPluginActions.clear();
      // Clear per-project banner keys (update-check<DELIM> family +
      // installed-plugins partial-load). The pluginUpdates.refresh and
      // local refresh that follow on the same projectPath change will
      // repopulate from fresh state.
      fetchErrors.clear();
    }
    priorProjectPath = projectPath;
  });

  // Helpers used by the table render block.
  function statusUpdateLabel(u: PluginUpdateInfo): string {
    // ContentChanged: phrased as a status (full sentence) rather than the
    // PluginCard button's action label ("Update (content changed)").
    // The two surfaces intentionally differ — column = state; button = action.
    if (u.change_signal.kind === "content_changed") return "Content changed since install";
    // VersionBumped: prefer the explicit "v_old → v_new" form when both
    // versions are known; fall back to "vN available" for legacy installs
    // (installed_version: None) and to a bare "Update available" when the
    // marketplace manifest declares no version.
    if (u.installed_version && u.available_version) {
      return `v${u.installed_version} → v${u.available_version}`;
    }
    if (u.available_version) return `v${u.available_version} available`;
    return "Update available";
  }

  function updateInfoFor(p: InstalledPluginInfo): PluginUpdateInfo | undefined {
    return pluginUpdates.updateFor(p.marketplace, p.plugin);
  }

  function failureFor(p: InstalledPluginInfo): PluginUpdateFailure | undefined {
    return pluginUpdates.failureFor(p.marketplace, p.plugin);
  }
</script>

<div class="flex flex-col h-full">
  <BannerStack
    errors={fetchErrors}
    message={installMessage}
    warning={installWarning}
    fatalError={installError}
    errLabel={errLabel}
    ondismiss={(key) => fetchErrors.delete(key)}
    onmessageDismiss={() => (installMessage = null)}
    onwarningDismiss={() => (installWarning = null)}
    onfatalErrorDismiss={() => (installError = null)}
  />

  {#if removeResult && removeResultPlugin}
    <div
      class="mx-4 mt-3 px-4 py-3 rounded-md text-sm flex items-start gap-3
        {removeResultHasFailures
          ? 'bg-kiro-warning/10 border border-kiro-warning/30 text-kiro-warning'
          : 'bg-kiro-success/10 border border-kiro-success/30 text-kiro-success'}"
    >
      <details
        class="flex-1"
        open={removeResultHasFailures}
      >
        <summary class="cursor-pointer text-xs opacity-85">
          {removeResultHasFailures ? "Show items + failures" : "Show items"}
        </summary>
        <div class="mt-2 pl-3 border-l-2 border-current/40 text-xs space-y-1">
          {#if (removeResult.skills.removed ?? []).length > 0}
            <div><b>Skills removed:</b> {(removeResult.skills.removed ?? []).join(", ")}</div>
          {/if}
          {#if (removeResult.steering.removed ?? []).length > 0}
            <div><b>Steering removed:</b> {(removeResult.steering.removed ?? []).join(", ")}</div>
          {/if}
          {#if (removeResult.agents.removed ?? []).length > 0}
            <div><b>Agents removed:</b> {(removeResult.agents.removed ?? []).join(", ")}</div>
          {/if}
          {#each removeResult.skills.failures ?? [] as f (f.item)}
            <div><b>Skill failed:</b> {f.item} — {f.error}</div>
          {/each}
          {#each removeResult.steering.failures ?? [] as f (f.item)}
            <div><b>Steering failed:</b> {f.item} — {f.error}</div>
          {/each}
          {#each removeResult.agents.failures ?? [] as f (f.item)}
            <div><b>Agent failed:</b> {f.item} — {f.error}</div>
          {/each}
        </div>
      </details>
      <button
        type="button"
        onclick={() => { removeResult = null; removeResultPlugin = null; }}
        aria-label="Dismiss remove summary"
        class="opacity-70 hover:opacity-100 text-lg leading-none flex-shrink-0 focus:outline-none focus:ring-2 focus:ring-kiro-accent-500 rounded"
      >
        ×
      </button>
    </div>
  {/if}

  <div class="flex-1 overflow-y-auto p-4">
    {#if loading}
      <div class="flex flex-col items-center justify-center h-full text-kiro-subtle gap-3">
        <svg class="w-8 h-8 text-kiro-accent-800 animate-pulse" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="1.5"
            d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15" />
        </svg>
        <p class="text-sm">Loading installed state...</p>
      </div>
    {:else if loadError}
      <div class="px-4 py-3 rounded-md bg-kiro-error/10 border border-kiro-error/30">
        <p class="text-sm text-kiro-error">{loadError}</p>
      </div>
    {:else}
      <section class="mb-6">
        <h2 class="text-sm font-semibold text-kiro-text mb-3">Installed plugins</h2>
        {#if plugins.length === 0}
          <p class="text-sm text-kiro-subtle">No plugins installed in this project.</p>
        {:else}
          <table class="w-full text-sm">
            <thead>
              <tr class="text-left text-[11px] uppercase tracking-wider text-kiro-subtle border-b border-kiro-muted">
                <th class="px-4 py-2">Plugin</th>
                <th class="px-4 py-2">Marketplace</th>
                <th class="px-4 py-2">Version</th>
                <th class="px-4 py-2">Status</th>
                <th class="px-4 py-2">Contents</th>
                <th class="px-4 py-2">Installed at</th>
                <th class="px-4 py-2"></th>
              </tr>
            </thead>
            <tbody>
              {#each plugins as p (pluginKey(p.marketplace, p.plugin))}
                {@const key = pluginKey(p.marketplace, p.plugin)}
                {@const updateInfo = updateInfoFor(p)}
                {@const failure = failureFor(p)}
                {@const action = pendingPluginActions.get(key)}
                <tr class="border-b border-kiro-muted/50">
                  <td class="px-4 py-3 font-medium text-kiro-text">{p.plugin}</td>
                  <td class="px-4 py-3 text-kiro-text-secondary">{p.marketplace}</td>
                  <td class="px-4 py-3 text-kiro-text-secondary">{p.installed_version ?? "—"}</td>
                  <td class="px-4 py-3">
                    {#if updateInfo}
                      <span
                        class="px-2 py-0.5 text-[11px] font-medium text-kiro-warning border border-kiro-warning/40 rounded"
                      >
                        {statusUpdateLabel(updateInfo)}
                      </span>
                    {:else if failure}
                      <span
                        class="px-2 py-0.5 text-[11px] font-medium text-kiro-error border border-kiro-error/40 rounded"
                        title={kindLabel(failure.kind)}
                      >
                        Update check failed
                      </span>
                    {:else}
                      <span class="text-kiro-success text-[11px]">Up to date</span>
                    {/if}
                  </td>
                  <td class="px-4 py-3 text-kiro-text-secondary">{contentSummary(p)}</td>
                  <td class="px-4 py-3 text-kiro-text-secondary">{formatDate(p.latest_install)}</td>
                  <td class="px-4 py-3 text-right">
                    <div class="inline-flex gap-2">
                      {#if updateInfo}
                        <button
                          type="button"
                          onclick={() => updatePlugin(p.marketplace, p.plugin)}
                          disabled={action !== undefined}
                          aria-busy={action === "update"}
                          title="Update will replace local edits to plugin files"
                          class="px-2 py-0.5 text-[11px] text-kiro-warning hover:text-kiro-warning/80 disabled:cursor-not-allowed disabled:opacity-50"
                        >
                          {action === "update" ? "Updating…" : "Update"}
                        </button>
                      {/if}
                      <button
                        type="button"
                        onclick={() => removePlugin(p.marketplace, p.plugin)}
                        disabled={action !== undefined}
                        aria-busy={action === "remove"}
                        class="px-2 py-0.5 text-[11px] text-kiro-subtle hover:text-kiro-error disabled:cursor-not-allowed disabled:opacity-50"
                      >
                        {action === "remove" ? "Removing…" : "Remove"}
                      </button>
                    </div>
                  </td>
                </tr>
              {/each}
            </tbody>
          </table>
        {/if}
      </section>

      <details class="mb-6">
        <summary class="cursor-pointer text-sm font-medium text-kiro-text-secondary hover:text-kiro-text">
          All installed skills (flat view)
        </summary>
        <div class="mt-3">
          {#if skills.length === 0}
            <p class="text-sm text-kiro-subtle">No skills installed.</p>
          {:else}
            <table class="w-full text-sm">
              <thead>
                <tr class="text-left text-[11px] uppercase tracking-wider text-kiro-subtle border-b border-kiro-muted">
                  <th class="px-4 py-2">Name</th>
                  <th class="px-4 py-2">Marketplace</th>
                  <th class="px-4 py-2">Plugin</th>
                  <th class="px-4 py-2">Version</th>
                  <th class="px-4 py-2">Installed</th>
                </tr>
              </thead>
              <tbody>
                {#each skills as skill (skill.name)}
                  <tr class="border-b border-kiro-muted/50">
                    <td class="px-4 py-3 text-kiro-text">{skill.name}</td>
                    <td class="px-4 py-3 text-kiro-text-secondary">{skill.marketplace}</td>
                    <td class="px-4 py-3 text-kiro-text-secondary">{skill.plugin}</td>
                    <td class="px-4 py-3 text-kiro-text-secondary">{skill.version ?? "—"}</td>
                    <td class="px-4 py-3 text-kiro-text-secondary">{formatDate(skill.installed_at)}</td>
                  </tr>
                {/each}
              </tbody>
            </table>
          {/if}
        </div>
      </details>
    {/if}
  </div>
</div>

<script lang="ts">
  import { onMount } from "svelte";
  import { commands } from "$lib/bindings";
  import type {
    InstalledSkillInfo,
    InstalledPluginInfo,
  } from "$lib/bindings";
  import { pluginKey } from "$lib/keys";

  let { projectPath }: { projectPath: string } = $props();

  let plugins: InstalledPluginInfo[] = $state([]);
  let skills: InstalledSkillInfo[] = $state([]);
  let loading: boolean = $state(true);
  let loadError: string | null = $state(null);
  // Non-fatal partial-load detail (one or more `installed-*.json` tracking
  // files failed to parse; the others loaded). Distinct from `loadError`
  // (red, fatal) — the table still has rows from the files that DID load.
  let loadWarning: string | null = $state(null);
  // Single removal in flight at a time — `remove_plugin` reads/writes the
  // installed-skills/steering/agents tracking files, so racing two removes
  // could clobber each other. Disabling all Remove buttons while one is
  // pending is the simplest correctness-preserving UI.
  let removingKey: string | null = $state(null);

  async function refresh() {
    loading = true;
    loadError = null;
    loadWarning = null;
    try {
      const [pluginsResult, skillsResult] = await Promise.all([
        commands.listInstalledPlugins(projectPath),
        commands.listInstalledSkills(projectPath),
      ]);
      if (pluginsResult.status === "ok") {
        // Wire format is `InstalledPluginsView` (I13): `.plugins` carries
        // the rows, `.partial_load_warnings` carries per-tracking-file
        // load failures (corrupt installed-*.json) so the table renders
        // the partial state instead of a misleading empty list.
        plugins = pluginsResult.data.plugins;
        const warnings = pluginsResult.data.partial_load_warnings ?? [];
        if (warnings.length > 0) {
          const summary = warnings
            .map((w) => `${w.tracking_file}: ${w.error}`)
            .join("; ");
          loadWarning = `Installed plugins partially loaded — ${summary}`;
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
    if (removingKey !== null) return;
    removingKey = key;
    try {
      const result = await commands.removePlugin(marketplace, plugin, projectPath);
      if (result.status === "ok") {
        await refresh();
      } else {
        loadError = `Remove failed for ${plugin}: ${result.error.message}`;
      }
    } catch (e) {
      const reason = e instanceof Error ? e.message : String(e);
      loadError = `Remove failed for ${plugin}: ${reason}`;
    } finally {
      removingKey = null;
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
</script>

<div class="flex flex-col h-full">
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
      {#if loadWarning}
        <div
          data-testid="installed-load-warning"
          class="mb-4 px-4 py-3 rounded-md bg-kiro-warning/10 border border-kiro-warning/30 flex items-start gap-3"
        >
          <p class="text-sm text-kiro-warning flex-1">{loadWarning}</p>
          <button
            type="button"
            onclick={() => (loadWarning = null)}
            aria-label="Dismiss installed-plugins warning"
            class="text-kiro-warning/70 hover:text-kiro-warning text-lg leading-none flex-shrink-0 focus:outline-none focus:ring-2 focus:ring-kiro-accent-500 rounded"
          >
            ×
          </button>
        </div>
      {/if}
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
                <th class="px-4 py-2">Contents</th>
                <th class="px-4 py-2">Installed</th>
                <th class="px-4 py-2"></th>
              </tr>
            </thead>
            <tbody>
              {#each plugins as p (pluginKey(p.marketplace, p.plugin))}
                {@const key = pluginKey(p.marketplace, p.plugin)}
                <tr class="border-b border-kiro-muted/50">
                  <td class="px-4 py-3 font-medium text-kiro-text">{p.plugin}</td>
                  <td class="px-4 py-3 text-kiro-text-secondary">{p.marketplace}</td>
                  <td class="px-4 py-3 text-kiro-text-secondary">{p.installed_version ?? "—"}</td>
                  <td class="px-4 py-3 text-kiro-text-secondary">{contentSummary(p)}</td>
                  <td class="px-4 py-3 text-kiro-text-secondary">{formatDate(p.latest_install)}</td>
                  <td class="px-4 py-3 text-right">
                    <button
                      type="button"
                      onclick={() => removePlugin(p.marketplace, p.plugin)}
                      disabled={removingKey !== null}
                      aria-busy={removingKey === key}
                      class="px-2 py-0.5 text-[11px] text-kiro-subtle hover:text-kiro-error disabled:cursor-not-allowed"
                    >
                      {removingKey === key ? "Removing…" : "Remove"}
                    </button>
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

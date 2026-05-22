<script lang="ts">
  // Workflows > Agents — list page (slice S12) + mode swap to editor
  // (slice S13 fills in the editor branch). Spec behaviors B1, B3-B6,
  // B10-B12 (B7-B9, B13 require the editor in S13).

  import { onMount } from "svelte";
  import { commands } from "$lib/bindings";
  import type { UserAgentRow } from "$lib/bindings";
  import {
    filterAgentRows,
    formatLineageBadge,
    formatModelChip,
  } from "$lib/agent-list-helpers";

  let { projectPath }: { projectPath: string } = $props();

  type Mode =
    | { kind: "list" }
    | { kind: "new" }
    | { kind: "edit"; row: UserAgentRow };

  let mode: Mode = $state({ kind: "list" });
  let rows: UserAgentRow[] = $state([]);
  let loading: boolean = $state(true);
  let loadError: string | null = $state(null);
  let actionError: string | null = $state(null);
  let toast: string | null = $state(null);
  let filter: string = $state("");

  let filtered: UserAgentRow[] = $derived(filterAgentRows(rows, filter));

  async function refresh() {
    loading = true;
    loadError = null;
    try {
      const result = await commands.listUserAgents(projectPath);
      if (result.status === "ok") {
        rows = result.data;
      } else {
        loadError = result.error.message;
      }
    } catch (e) {
      loadError = e instanceof Error ? e.message : String(e);
    } finally {
      loading = false;
    }
  }

  function showToast(text: string) {
    toast = text;
    setTimeout(() => {
      if (toast === text) toast = null;
    }, 3000);
  }

  async function handleDuplicate(row: UserAgentRow) {
    actionError = null;
    try {
      const result = await commands.duplicateUserAgent(row.name, projectPath);
      if (result.status === "ok") {
        await refresh();
        showToast(`Duplicated as “${result.data}”`);
      } else {
        actionError = result.error.message;
      }
    } catch (e) {
      actionError = e instanceof Error ? e.message : String(e);
    }
  }

  async function handleDelete(row: UserAgentRow) {
    const label = row.lineage
      ? `Delete the marketplace-installed agent “${row.name}”? The tracking entry will also be removed.`
      : `Delete the agent “${row.name}”?`;
    // window.confirm is the Control Center's current confirm
    // affordance (per design "Things to watch out for" item 6 — the
    // design references a future replacement). Acceptable for slice
    // S12; visual upgrade is reviewer-deferred.
    if (!confirm(label)) return;
    actionError = null;
    try {
      const result = await commands.deleteUserAgent(row.name, projectPath);
      if (result.status === "ok") {
        await refresh();
        showToast(`Deleted “${row.name}”`);
      } else {
        actionError = result.error.message;
      }
    } catch (e) {
      actionError = e instanceof Error ? e.message : String(e);
    }
  }

  onMount(refresh);

  $effect(() => {
    void projectPath;
    refresh();
  });
</script>

{#if mode.kind === "list"}
  <div class="flex flex-col h-full">
    <!-- Toolbar -->
    <div class="flex items-center gap-3 px-4 py-3 border-b border-kiro-muted">
      <input
        type="text"
        placeholder="Filter agents by name, description, or model"
        bind:value={filter}
        class="flex-1 px-3 py-1.5 text-sm bg-kiro-overlay border border-kiro-muted rounded text-kiro-text placeholder:text-kiro-subtle focus:outline-none focus:ring-2 focus:ring-kiro-accent-500"
      />
      <button
        type="button"
        onclick={() => (mode = { kind: "new" })}
        class="px-3 py-1.5 text-sm font-medium bg-kiro-accent-700 hover:bg-kiro-accent-600 text-white rounded focus:outline-none focus:ring-2 focus:ring-kiro-accent-500"
      >
        + Create Agent
      </button>
    </div>

    <!-- Banners -->
    {#if actionError}
      <div class="mx-4 mt-3 px-4 py-2 rounded-md text-sm bg-kiro-error/10 border border-kiro-error/30 text-kiro-error flex items-start gap-3">
        <p class="flex-1">{actionError}</p>
        <button
          type="button"
          aria-label="Dismiss error"
          onclick={() => (actionError = null)}
          class="opacity-70 hover:opacity-100 text-lg leading-none"
        >×</button>
      </div>
    {/if}
    {#if toast}
      <div
        class="mx-4 mt-3 px-4 py-2 rounded-md text-sm bg-kiro-success/10 border border-kiro-success/30 text-kiro-success"
        role="status"
      >
        {toast}
      </div>
    {/if}

    <!-- Body -->
    <div class="flex-1 overflow-y-auto p-4">
      {#if loading}
        <p class="text-sm text-kiro-subtle">Loading agents…</p>
      {:else if loadError}
        <div class="px-4 py-3 rounded-md bg-kiro-error/10 border border-kiro-error/30">
          <p class="text-sm text-kiro-error">{loadError}</p>
        </div>
      {:else if rows.length === 0}
        <!-- Spec B3: design's empty state -->
        <div class="flex flex-col items-center justify-center h-full text-kiro-subtle gap-3">
          <svg class="w-10 h-10 text-kiro-accent-800" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="1.5"
              d="M9.75 17L9 20l-1 1h8l-1-1-.75-3M3 13h18M5 17h14a2 2 0 002-2V5a2 2 0 00-2-2H5a2 2 0 00-2 2v10a2 2 0 002 2z" />
          </svg>
          <p class="text-sm">No agents yet.</p>
          <p class="text-xs">Click “+ Create Agent” to author your first agent.</p>
        </div>
      {:else if filtered.length === 0}
        <p class="text-sm text-kiro-subtle">No agents match your filter.</p>
      {:else}
        <ul class="flex flex-col gap-2.5">
          {#each filtered as row (row.name)}
            {@const lineageBadge = formatLineageBadge(row.lineage)}
            <li
              class="rounded-lg bg-kiro-overlay border border-kiro-muted border-l-2 border-l-kiro-accent-800 hover:border-l-kiro-accent-500 hover:bg-kiro-accent-900/10 transition-colors px-4 py-3 flex items-start gap-4"
            >
              <div class="flex-1 min-w-0">
                <div class="flex items-center gap-2 flex-wrap">
                  <svg class="w-3 h-3 text-kiro-accent-400 flex-shrink-0" fill="currentColor" viewBox="0 0 24 24">
                    <path d="M12 2a3 3 0 013 3v1h1a4 4 0 014 4v8a4 4 0 01-4 4H8a4 4 0 01-4-4v-8a4 4 0 014-4h1V5a3 3 0 013-3z"/>
                  </svg>
                  <span class="font-mono text-[13px] font-semibold text-kiro-text">{row.name}</span>
                  {#if lineageBadge}
                    <span
                      class="px-1.5 py-px text-[10px] font-semibold uppercase tracking-wider bg-kiro-accent-900/35 text-kiro-accent-300 rounded"
                      title={lineageBadge}
                    >
                      from {row.lineage?.marketplace}
                    </span>
                  {/if}
                </div>
                {#if row.description}
                  <p class="mt-1 text-[13px] text-kiro-text-secondary line-clamp-2">{row.description}</p>
                {/if}
                <div class="mt-1.5 flex items-center gap-2 text-[12px] text-kiro-text-secondary flex-wrap">
                  <span class="px-1.5 py-px font-mono text-[11px] bg-kiro-accent-900/20 text-kiro-accent-300 rounded">
                    {formatModelChip(row.model)}
                  </span>
                  <span class="text-kiro-subtle">·</span>
                  <span>{row.tools_count} tool{row.tools_count === 1 ? "" : "s"}</span>
                  {#if row.mcp_count > 0}
                    <span class="text-kiro-subtle">·</span>
                    <span>{row.mcp_count} MCP</span>
                  {/if}
                  {#if row.resources_count > 0}
                    <span class="text-kiro-subtle">·</span>
                    <span>{row.resources_count} resource{row.resources_count === 1 ? "" : "s"}</span>
                  {/if}
                  {#if row.hooks_count > 0}
                    <span class="text-kiro-subtle">·</span>
                    <span>{row.hooks_count} hook{row.hooks_count === 1 ? "" : "s"}</span>
                  {/if}
                </div>
              </div>
              <div class="flex items-center gap-1 flex-shrink-0">
                <button
                  type="button"
                  onclick={() => (mode = { kind: "edit", row })}
                  class="px-2 py-1 text-[12px] text-kiro-text-secondary hover:text-kiro-text hover:bg-kiro-accent-900/20 rounded focus:outline-none focus:ring-2 focus:ring-kiro-accent-500"
                >Edit</button>
                <button
                  type="button"
                  onclick={() => handleDuplicate(row)}
                  aria-label={`Duplicate ${row.name}`}
                  title="Duplicate"
                  class="p-1 text-kiro-text-secondary hover:text-kiro-text hover:bg-kiro-accent-900/20 rounded focus:outline-none focus:ring-2 focus:ring-kiro-accent-500"
                >
                  <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path stroke-linecap="round" stroke-linejoin="round" stroke-width="1.5"
                      d="M8 5H6a2 2 0 00-2 2v12a2 2 0 002 2h10a2 2 0 002-2v-2M8 5a2 2 0 002 2h6a2 2 0 002-2M8 5a2 2 0 012-2h6a2 2 0 012 2"/>
                  </svg>
                </button>
                <button
                  type="button"
                  onclick={() => handleDelete(row)}
                  aria-label={`Delete ${row.name}`}
                  title="Delete"
                  class="p-1 text-kiro-text-secondary hover:text-kiro-error hover:bg-kiro-error/10 rounded focus:outline-none focus:ring-2 focus:ring-kiro-error"
                >
                  <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path stroke-linecap="round" stroke-linejoin="round" stroke-width="1.5"
                      d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6M1 7h22M9 7V3a2 2 0 012-2h2a2 2 0 012 2v4"/>
                  </svg>
                </button>
              </div>
            </li>
          {/each}
        </ul>
      {/if}
    </div>

    <!-- Footer -->
    {#if !loading && rows.length > 0}
      <div class="px-4 py-2 border-t border-kiro-muted text-[11px] text-kiro-subtle flex items-center justify-between">
        <span>
          {filtered.length === rows.length
            ? `${rows.length} agent${rows.length === 1 ? "" : "s"}`
            : `${filtered.length} of ${rows.length} agents`}
        </span>
        <span class="font-mono">Stored at .kiro/agents/</span>
      </div>
    {/if}
  </div>
{:else}
  <!-- Editor branch placeholder. Slice S13 replaces this with
       AgentEditor.svelte. Until then, render a back-link + a
       "Coming in S13" notice so the navigation flow is functional. -->
  <div class="flex flex-col h-full">
    <div class="flex items-center gap-3 px-4 py-3 border-b border-kiro-muted">
      <button
        type="button"
        onclick={() => (mode = { kind: "list" })}
        class="text-sm text-kiro-text-secondary hover:text-kiro-text"
      >
        ‹ Agents
      </button>
      <span class="text-sm text-kiro-text font-medium">
        {mode.kind === "new" ? "New agent" : `Editing ${mode.row.name}`}
      </span>
    </div>
    <div class="flex-1 flex items-center justify-center text-kiro-subtle">
      <p class="text-sm">Editor lands in slice S13 (kiro-vgnw and onward fill out the section panels).</p>
    </div>
  </div>
{/if}

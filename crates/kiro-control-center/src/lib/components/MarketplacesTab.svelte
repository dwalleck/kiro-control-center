<script lang="ts">
  import { commands } from "$lib/bindings";
  import type { MarketplaceInfo, GitProtocol } from "$lib/bindings";

  let marketplaces: MarketplaceInfo[] = $state([]);
  let newSource: string = $state("");
  let protocol: GitProtocol = $state("https");

  let loading: boolean = $state(false);
  let adding: boolean = $state(false);
  let updatingName: string | null = $state(null);
  let removingName: string | null = $state(null);

  let error: string | null = $state(null);
  let successMessage: string | null = $state(null);

  async function loadMarketplaces() {
    loading = true;
    error = null;
    const result = await commands.listMarketplaces();
    if (result.status === "ok") {
      marketplaces = result.data;
    } else {
      error = result.error.message;
    }
    loading = false;
  }

  async function addMarketplace() {
    const source = newSource.trim();
    if (!source) return;
    adding = true;
    error = null;
    successMessage = null;

    const result = await commands.addMarketplace(source, protocol);
    if (result.status === "ok") {
      const { name, plugins } = result.data;
      successMessage = `Added "${name}" with ${plugins.length} plugin${plugins.length === 1 ? "" : "s"}`;
      newSource = "";
      await loadMarketplaces();
    } else {
      error = result.error.message;
    }
    adding = false;
  }

  async function updateMarketplace(name: string) {
    updatingName = name;
    error = null;
    successMessage = null;

    const result = await commands.updateMarketplace(name);
    if (result.status === "ok") {
      const { updated, failed, skipped } = result.data;
      const parts: string[] = [];
      if (updated.length > 0) parts.push(`Updated: ${updated.join(", ")}`);
      if (skipped.length > 0) parts.push(`Skipped: ${skipped.join(", ")}`);
      if (failed.length > 0) parts.push(`Failed: ${failed.map((f) => `${f.name} (${f.error})`).join(", ")}`);
      successMessage = parts.join(" | ");
      await loadMarketplaces();
    } else {
      error = result.error.message;
    }
    updatingName = null;
  }

  async function removeMarketplace(name: string) {
    if (!confirm(`Remove marketplace "${name}"? This will delete the cached data.`)) {
      return;
    }

    removingName = name;
    error = null;
    successMessage = null;

    const result = await commands.removeMarketplace(name);
    if (result.status === "ok") {
      successMessage = `Removed "${name}"`;
      await loadMarketplaces();
    } else {
      error = result.error.message;
    }
    removingName = null;
  }

  function sourceTypeBadgeClass(sourceType: string): string {
    switch (sourceType) {
      case "github":
        return "bg-kiro-info/15 text-kiro-info";
      case "git":
        return "bg-kiro-accent-900/30 text-kiro-accent-300";
      case "local":
        return "bg-kiro-warning/15 text-kiro-warning";
      default:
        return "bg-kiro-muted text-kiro-text-secondary";
    }
  }

  $effect(() => {
    loadMarketplaces();
  });
</script>

<div class="flex flex-col h-full">
  <!-- Add marketplace section -->
  <div class="p-4 border-b border-kiro-muted">
    <h3 class="text-sm font-medium text-kiro-text-secondary mb-2">Add marketplace</h3>
    <form
      class="flex gap-2"
      onsubmit={(e: Event) => { e.preventDefault(); addMarketplace(); }}
    >
      <input
        type="text"
        placeholder="owner/repo, git URL, or local path"
        bind:value={newSource}
        disabled={adding}
        class="flex-1 px-3 py-2 text-sm rounded-md border border-kiro-muted bg-kiro-overlay text-kiro-text placeholder-kiro-subtle focus:outline-none focus:ring-2 focus:ring-kiro-accent-500 focus:border-transparent disabled:opacity-50"
      />
      <select
        bind:value={protocol}
        disabled={adding}
        class="px-3 py-2 text-sm rounded-md border border-kiro-muted bg-kiro-overlay text-kiro-text focus:outline-none focus:ring-2 focus:ring-kiro-accent-500 focus:border-transparent disabled:opacity-50"
      >
        <option value="https">HTTPS</option>
        <option value="ssh">SSH</option>
      </select>
      <button
        type="submit"
        disabled={adding || !newSource.trim()}
        class="px-4 py-2 text-sm font-medium rounded-md text-white transition-colors duration-150
          {!adding && newSource.trim()
            ? 'bg-kiro-accent-600 hover:bg-kiro-accent-700'
            : 'bg-kiro-muted text-kiro-subtle cursor-not-allowed'}"
      >
        {adding ? "Adding..." : "Add"}
      </button>
    </form>
  </div>

  <!-- Error display -->
  {#if error}
    <div class="mx-4 mt-3 px-4 py-3 rounded-md bg-kiro-error/10 border border-kiro-error/30">
      <p class="text-sm text-kiro-error">{error}</p>
    </div>
  {/if}

  <!-- Success message -->
  {#if successMessage}
    <div class="mx-4 mt-3 px-4 py-3 rounded-md bg-kiro-success/10 border border-kiro-success/30">
      <p class="text-sm text-kiro-success">{successMessage}</p>
    </div>
  {/if}

  <!-- Marketplace list -->
  <div class="flex-1 overflow-y-auto p-4">
    {#if loading}
      <div class="flex items-center justify-center h-full text-kiro-subtle">
        <p class="text-sm">Loading marketplaces...</p>
      </div>
    {:else if marketplaces.length === 0}
      <div class="flex items-center justify-center h-full text-kiro-subtle">
        <p class="text-sm">No marketplaces registered. Add one above to get started.</p>
      </div>
    {:else}
      <div class="space-y-3">
        {#each marketplaces as mp (mp.name)}
          <div class="flex items-center justify-between p-4 rounded-lg border border-kiro-muted bg-kiro-overlay">
            <div class="flex items-center gap-3 min-w-0">
              <div class="min-w-0">
                <div class="flex items-center gap-2">
                  <span class="font-semibold text-kiro-text truncate">{mp.name}</span>
                  <span class="inline-flex items-center px-2 py-0.5 text-xs font-medium rounded-full {sourceTypeBadgeClass(mp.source_type)}">
                    {mp.source_type}
                  </span>
                </div>
                <p class="mt-0.5 text-sm text-kiro-subtle">
                  {mp.plugin_count} plugin{mp.plugin_count === 1 ? "" : "s"}
                </p>
              </div>
            </div>
            <div class="flex items-center gap-2 flex-shrink-0">
              <button
                class="px-3 py-1.5 text-xs font-medium rounded-md transition-colors duration-150
                  {updatingName === mp.name
                    ? 'bg-kiro-muted text-kiro-subtle cursor-not-allowed'
                    : 'bg-kiro-muted text-kiro-text-secondary hover:bg-kiro-muted'}"
                disabled={updatingName === mp.name}
                onclick={() => updateMarketplace(mp.name)}
              >
                {updatingName === mp.name ? "Updating..." : "Update"}
              </button>
              <button
                class="px-3 py-1.5 text-xs font-medium rounded-md transition-colors duration-150
                  {removingName === mp.name
                    ? 'bg-kiro-muted text-kiro-subtle cursor-not-allowed'
                    : 'bg-kiro-error/10 text-kiro-error hover:bg-kiro-error/20'}"
                disabled={removingName === mp.name}
                onclick={() => removeMarketplace(mp.name)}
              >
                {removingName === mp.name ? "Removing..." : "Remove"}
              </button>
            </div>
          </div>
        {/each}
      </div>
    {/if}
  </div>
</div>

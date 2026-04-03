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
        return "bg-blue-100 text-blue-800 dark:bg-blue-900/30 dark:text-blue-400";
      case "git":
        return "bg-purple-100 text-purple-800 dark:bg-purple-900/30 dark:text-purple-400";
      case "local":
        return "bg-yellow-100 text-yellow-800 dark:bg-yellow-900/30 dark:text-yellow-400";
      default:
        return "bg-gray-100 text-gray-800 dark:bg-gray-700 dark:text-gray-300";
    }
  }

  $effect(() => {
    loadMarketplaces();
  });
</script>

<div class="flex flex-col h-full">
  <!-- Add marketplace section -->
  <div class="p-4 border-b border-gray-200 dark:border-gray-700">
    <h3 class="text-sm font-medium text-gray-700 dark:text-gray-300 mb-2">Add marketplace</h3>
    <form
      class="flex gap-2"
      onsubmit={(e: Event) => { e.preventDefault(); addMarketplace(); }}
    >
      <input
        type="text"
        placeholder="owner/repo, git URL, or local path"
        bind:value={newSource}
        disabled={adding}
        class="flex-1 px-3 py-2 text-sm rounded-md border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-800 text-gray-900 dark:text-gray-100 placeholder-gray-400 dark:placeholder-gray-500 focus:outline-none focus:ring-2 focus:ring-blue-500 focus:border-transparent disabled:opacity-50"
      />
      <select
        bind:value={protocol}
        disabled={adding}
        class="px-3 py-2 text-sm rounded-md border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-800 text-gray-900 dark:text-gray-100 focus:outline-none focus:ring-2 focus:ring-blue-500 focus:border-transparent disabled:opacity-50"
      >
        <option value="https">HTTPS</option>
        <option value="ssh">SSH</option>
      </select>
      <button
        type="submit"
        disabled={adding || !newSource.trim()}
        class="px-4 py-2 text-sm font-medium rounded-md text-white transition-colors duration-150
          {!adding && newSource.trim()
            ? 'bg-blue-600 hover:bg-blue-700 dark:bg-blue-500 dark:hover:bg-blue-600'
            : 'bg-gray-300 dark:bg-gray-700 cursor-not-allowed'}"
      >
        {adding ? "Adding..." : "Add"}
      </button>
    </form>
  </div>

  <!-- Error display -->
  {#if error}
    <div class="mx-4 mt-3 px-4 py-3 rounded-md bg-red-50 dark:bg-red-900/20 border border-red-200 dark:border-red-800">
      <p class="text-sm text-red-700 dark:text-red-400">{error}</p>
    </div>
  {/if}

  <!-- Success message -->
  {#if successMessage}
    <div class="mx-4 mt-3 px-4 py-3 rounded-md bg-green-50 dark:bg-green-900/20 border border-green-200 dark:border-green-800">
      <p class="text-sm text-green-700 dark:text-green-400">{successMessage}</p>
    </div>
  {/if}

  <!-- Marketplace list -->
  <div class="flex-1 overflow-y-auto p-4">
    {#if loading}
      <div class="flex items-center justify-center h-full text-gray-400 dark:text-gray-500">
        <p class="text-sm">Loading marketplaces...</p>
      </div>
    {:else if marketplaces.length === 0}
      <div class="flex items-center justify-center h-full text-gray-400 dark:text-gray-500">
        <p class="text-sm">No marketplaces registered. Add one above to get started.</p>
      </div>
    {:else}
      <div class="space-y-3">
        {#each marketplaces as mp (mp.name)}
          <div class="flex items-center justify-between p-4 rounded-lg border border-gray-200 dark:border-gray-700 bg-white dark:bg-gray-800">
            <div class="flex items-center gap-3 min-w-0">
              <div class="min-w-0">
                <div class="flex items-center gap-2">
                  <span class="font-semibold text-gray-900 dark:text-gray-100 truncate">{mp.name}</span>
                  <span class="inline-flex items-center px-2 py-0.5 text-xs font-medium rounded-full {sourceTypeBadgeClass(mp.source_type)}">
                    {mp.source_type}
                  </span>
                </div>
                <p class="mt-0.5 text-sm text-gray-500 dark:text-gray-400">
                  {mp.plugin_count} plugin{mp.plugin_count === 1 ? "" : "s"}
                </p>
              </div>
            </div>
            <div class="flex items-center gap-2 flex-shrink-0">
              <button
                class="px-3 py-1.5 text-xs font-medium rounded-md transition-colors duration-150
                  {updatingName === mp.name
                    ? 'bg-gray-200 dark:bg-gray-700 text-gray-400 cursor-not-allowed'
                    : 'bg-gray-100 dark:bg-gray-700 text-gray-700 dark:text-gray-300 hover:bg-gray-200 dark:hover:bg-gray-600'}"
                disabled={updatingName === mp.name}
                onclick={() => updateMarketplace(mp.name)}
              >
                {updatingName === mp.name ? "Updating..." : "Update"}
              </button>
              <button
                class="px-3 py-1.5 text-xs font-medium rounded-md transition-colors duration-150
                  {removingName === mp.name
                    ? 'bg-gray-200 dark:bg-gray-700 text-gray-400 cursor-not-allowed'
                    : 'bg-red-50 dark:bg-red-900/20 text-red-600 dark:text-red-400 hover:bg-red-100 dark:hover:bg-red-900/40'}"
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

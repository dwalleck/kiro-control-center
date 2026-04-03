<script lang="ts">
  import { commands } from "$lib/bindings";
  import type { InstalledSkillInfo } from "$lib/bindings";
  import { SvelteSet } from "svelte/reactivity";

  let { projectPath }: { projectPath: string } = $props();

  let skills: InstalledSkillInfo[] = $state([]);
  let selectedSkills = new SvelteSet<string>();
  let filterText: string = $state("");

  let loading: boolean = $state(false);
  let removing: boolean = $state(false);
  let error: string | null = $state(null);
  let successMessage: string | null = $state(null);

  let filteredSkills = $derived(
    skills.filter(
      (s) =>
        s.name.toLowerCase().includes(filterText.toLowerCase()) ||
        s.plugin.toLowerCase().includes(filterText.toLowerCase()) ||
        s.marketplace.toLowerCase().includes(filterText.toLowerCase())
    )
  );

  let selectedCount = $derived(selectedSkills.size);

  async function loadSkills() {
    loading = true;
    error = null;
    const result = await commands.listInstalledSkills(projectPath);
    if (result.status === "ok") {
      skills = result.data;
    } else {
      error = result.error.message;
    }
    loading = false;
  }

  function toggleSkill(name: string) {
    if (selectedSkills.has(name)) {
      selectedSkills.delete(name);
    } else {
      selectedSkills.add(name);
    }
  }

  function toggleAll() {
    if (selectedSkills.size === filteredSkills.length) {
      selectedSkills.clear();
    } else {
      for (const s of filteredSkills) {
        selectedSkills.add(s.name);
      }
    }
  }

  async function removeSelected() {
    if (selectedSkills.size === 0) return;
    removing = true;
    error = null;
    successMessage = null;

    const names = Array.from(selectedSkills);
    const removed: string[] = [];
    const failed: string[] = [];

    for (const name of names) {
      const result = await commands.removeSkill(name, projectPath);
      if (result.status === "ok") {
        removed.push(name);
      } else {
        failed.push(`${name}: ${result.error.message}`);
      }
    }

    if (removed.length > 0) {
      successMessage = `Removed: ${removed.join(", ")}`;
    }
    if (failed.length > 0) {
      error = `Failed to remove: ${failed.join("; ")}`;
    }

    selectedSkills.clear();
    await loadSkills();
    removing = false;
  }

  function formatDate(iso: string): string {
    const date = new Date(iso);
    if (isNaN(date.getTime())) {
      console.warn(`Invalid date string from backend: "${iso}"`);
      return iso;
    }
    return date.toLocaleDateString(undefined, {
      year: "numeric",
      month: "short",
      day: "numeric",
      hour: "2-digit",
      minute: "2-digit",
    });
  }

  $effect(() => {
    loadSkills();
  });
</script>

<div class="flex flex-col h-full">
  <!-- Search bar -->
  <div class="p-4 border-b border-gray-200 dark:border-gray-700">
    <input
      type="text"
      placeholder="Filter by name, plugin, or marketplace..."
      bind:value={filterText}
      class="w-full px-3 py-2 text-sm rounded-md border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-800 text-gray-900 dark:text-gray-100 placeholder-gray-400 dark:placeholder-gray-500 focus:outline-none focus:ring-2 focus:ring-blue-500 focus:border-transparent"
    />
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

  <!-- Table -->
  <div class="flex-1 overflow-y-auto">
    {#if loading}
      <div class="flex items-center justify-center h-full text-gray-400 dark:text-gray-500">
        <p class="text-sm">Loading installed skills...</p>
      </div>
    {:else if filteredSkills.length === 0}
      <div class="flex items-center justify-center h-full text-gray-400 dark:text-gray-500">
        <p class="text-sm">
          {filterText ? "No installed skills match the filter" : "No skills installed"}
        </p>
      </div>
    {:else}
      <table class="w-full text-sm text-left">
        <thead class="text-xs text-gray-500 dark:text-gray-400 uppercase bg-gray-50 dark:bg-gray-900 sticky top-0">
          <tr>
            <th class="px-4 py-3 w-10">
              <input
                type="checkbox"
                checked={selectedSkills.size === filteredSkills.length && filteredSkills.length > 0}
                onchange={toggleAll}
                class="h-4 w-4 rounded border-gray-300 text-blue-600 focus:ring-blue-500"
              />
            </th>
            <th class="px-4 py-3">Name</th>
            <th class="px-4 py-3">Plugin</th>
            <th class="px-4 py-3">Marketplace</th>
            <th class="px-4 py-3">Version</th>
            <th class="px-4 py-3">Installed</th>
          </tr>
        </thead>
        <tbody>
          {#each filteredSkills as skill (skill.name)}
            <tr
              class="border-b border-gray-100 dark:border-gray-800 hover:bg-gray-50 dark:hover:bg-gray-800/50 transition-colors duration-100
                {selectedSkills.has(skill.name) ? 'bg-blue-50 dark:bg-blue-900/10' : ''}"
            >
              <td class="px-4 py-3">
                <input
                  type="checkbox"
                  checked={selectedSkills.has(skill.name)}
                  onchange={() => toggleSkill(skill.name)}
                  class="h-4 w-4 rounded border-gray-300 text-blue-600 focus:ring-blue-500"
                />
              </td>
              <td class="px-4 py-3 font-medium text-gray-900 dark:text-gray-100">{skill.name}</td>
              <td class="px-4 py-3 text-gray-600 dark:text-gray-400">{skill.plugin}</td>
              <td class="px-4 py-3 text-gray-600 dark:text-gray-400">{skill.marketplace}</td>
              <td class="px-4 py-3 text-gray-600 dark:text-gray-400">{skill.version ?? "---"}</td>
              <td class="px-4 py-3 text-gray-600 dark:text-gray-400">{formatDate(skill.installed_at)}</td>
            </tr>
          {/each}
        </tbody>
      </table>
    {/if}
  </div>

  <!-- Bottom bar -->
  <div class="p-4 border-t border-gray-200 dark:border-gray-700 bg-gray-50 dark:bg-gray-900 flex items-center justify-between">
    <span class="text-sm text-gray-500 dark:text-gray-400">
      {skills.length} skill{skills.length === 1 ? "" : "s"} installed
    </span>
    <button
      class="px-4 py-2 text-sm font-medium rounded-md text-white transition-colors duration-150
        {selectedCount > 0 && !removing
          ? 'bg-red-600 hover:bg-red-700 dark:bg-red-500 dark:hover:bg-red-600'
          : 'bg-gray-300 dark:bg-gray-700 cursor-not-allowed'}"
      disabled={selectedCount === 0 || removing}
      onclick={removeSelected}
    >
      {removing ? "Removing..." : `Remove ${selectedCount} selected`}
    </button>
  </div>
</div>

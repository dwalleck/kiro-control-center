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
  <div class="p-4 border-b border-kiro-muted">
    <input
      type="text"
      placeholder="Filter by name, plugin, or marketplace..."
      bind:value={filterText}
      class="w-full px-3 py-2 text-sm rounded-md border border-kiro-muted bg-kiro-overlay text-kiro-text placeholder-kiro-subtle focus:outline-none focus:ring-2 focus:ring-kiro-accent-500 focus:border-transparent"
    />
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

  <!-- Table -->
  <div class="flex-1 overflow-y-auto">
    {#if loading}
      <div class="flex flex-col items-center justify-center h-full text-kiro-subtle gap-3">
        <svg class="w-8 h-8 text-kiro-accent-800 animate-pulse" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="1.5"
            d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15" />
        </svg>
        <p class="text-sm">Loading installed skills...</p>
      </div>
    {:else if filteredSkills.length === 0}
      <div class="flex flex-col items-center justify-center h-full text-kiro-subtle gap-3">
        <svg class="w-10 h-10 text-kiro-accent-800" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="1.5"
            d="M20 13V6a2 2 0 00-2-2H6a2 2 0 00-2 2v7m16 0v5a2 2 0 01-2 2H6a2 2 0 01-2-2v-5m16 0h-2.586a1 1 0 00-.707.293l-2.414 2.414a1 1 0 01-.707.293h-3.172a1 1 0 01-.707-.293l-2.414-2.414A1 1 0 006.586 13H4" />
        </svg>
        <p class="text-sm">
          {filterText ? "No installed skills match the filter" : "No skills installed"}
        </p>
      </div>
    {:else}
      <table class="w-full text-sm text-left">
        <thead class="text-xs text-kiro-subtle uppercase bg-kiro-surface sticky top-0">
          <tr>
            <th class="px-4 py-3 w-10">
              <input
                type="checkbox"
                checked={selectedSkills.size === filteredSkills.length && filteredSkills.length > 0}
                onchange={toggleAll}
                class="h-4 w-4 rounded border-kiro-muted text-kiro-accent-500 focus:ring-kiro-accent-500"
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
              class="border-b border-kiro-muted/50 hover:bg-kiro-overlay transition-colors duration-100
                {selectedSkills.has(skill.name) ? 'bg-kiro-accent-900/10' : ''}"
            >
              <td class="px-4 py-3">
                <input
                  type="checkbox"
                  checked={selectedSkills.has(skill.name)}
                  onchange={() => toggleSkill(skill.name)}
                  class="h-4 w-4 rounded border-kiro-muted text-kiro-accent-500 focus:ring-kiro-accent-500"
                />
              </td>
              <td class="px-4 py-3 font-medium text-kiro-text">{skill.name}</td>
              <td class="px-4 py-3 text-kiro-text-secondary">{skill.plugin}</td>
              <td class="px-4 py-3 text-kiro-text-secondary">{skill.marketplace}</td>
              <td class="px-4 py-3 text-kiro-text-secondary">{skill.version ?? "---"}</td>
              <td class="px-4 py-3 text-kiro-text-secondary">{formatDate(skill.installed_at)}</td>
            </tr>
          {/each}
        </tbody>
      </table>
    {/if}
  </div>

  <!-- Bottom bar -->
  <div class="p-4 border-t border-kiro-muted bg-kiro-surface flex items-center justify-between">
    <span class="text-sm text-kiro-subtle">
      {skills.length} skill{skills.length === 1 ? "" : "s"} installed
    </span>
    <button
      class="px-4 py-2 text-sm font-medium rounded-md text-white transition-colors duration-150
        {selectedCount > 0 && !removing
          ? 'bg-kiro-error hover:bg-kiro-error-hover'
          : 'bg-kiro-muted text-kiro-subtle cursor-not-allowed'}"
      disabled={selectedCount === 0 || removing}
      onclick={removeSelected}
    >
      {removing ? "Removing..." : `Remove ${selectedCount} selected`}
    </button>
  </div>
</div>

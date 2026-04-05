<script lang="ts">
  import { open } from "@tauri-apps/plugin-dialog";
  import { store, selectProject, addScanRoot } from "$lib/stores/project.svelte";

  async function handleAddDirectory() {
    const selected = await open({ directory: true, title: "Select a directory to scan for projects" });
    if (selected === null) return;
    await addScanRoot(selected);
  }

  async function handleOpenOther() {
    const selected = await open({ directory: true, title: "Select a Kiro project" });
    if (selected === null) return;
    await selectProject(selected);
  }
</script>

<div class="flex items-center justify-center h-full bg-gray-100 dark:bg-gray-950">
  <div class="max-w-2xl w-full mx-auto p-8">
    <h1 class="text-2xl font-bold text-gray-900 dark:text-gray-100 mb-2">Kiro Control Center</h1>
    <p class="text-gray-500 dark:text-gray-400 mb-8">Select a project to manage its skills.</p>

    {#if store.discoveredProjects.length > 0}
      <div class="space-y-2 mb-6">
        {#each store.discoveredProjects as project (project.path)}
          <button
            class="w-full text-left px-4 py-3 rounded-lg border border-gray-200 dark:border-gray-700 bg-white dark:bg-gray-900 hover:border-blue-400 dark:hover:border-blue-500 transition-colors"
            onclick={() => selectProject(project.path)}
          >
            <div class="font-medium text-gray-900 dark:text-gray-100">{project.name}</div>
            <div class="text-sm text-gray-500 dark:text-gray-400 truncate">{project.path}</div>
          </button>
        {/each}
      </div>
    {:else if (store.settings.scan_roots ?? []).length > 0}
      <p class="text-gray-500 dark:text-gray-400 mb-6">No projects found in your configured directories.</p>
    {/if}

    <div class="flex gap-3">
      <button
        class="px-4 py-2 rounded-lg bg-blue-600 text-white hover:bg-blue-700 transition-colors text-sm font-medium"
        onclick={handleAddDirectory}
      >
        Add Directory to Scan
      </button>
      <button
        class="px-4 py-2 rounded-lg border border-gray-300 dark:border-gray-600 text-gray-700 dark:text-gray-300 hover:bg-gray-50 dark:hover:bg-gray-800 transition-colors text-sm font-medium"
        onclick={handleOpenOther}
      >
        Open Other...
      </button>
    </div>

    {#if (store.settings.scan_roots ?? []).length > 0}
      <div class="mt-8 text-xs text-gray-400 dark:text-gray-500">
        Scanning: {(store.settings.scan_roots ?? []).join(", ")}
      </div>
    {/if}
  </div>
</div>

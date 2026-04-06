<script lang="ts">
  import { open } from "@tauri-apps/plugin-dialog";
  import { store, selectProject } from "$lib/stores/project.svelte";

  let { onManageRoots }: { onManageRoots: () => void } = $props();

  let isOpen = $state(false);

  function toggle() {
    isOpen = !isOpen;
  }

  function close() {
    isOpen = false;
  }

  async function handleSelectProject(path: string) {
    close();
    await selectProject(path);
  }

  async function handleOpenOther() {
    close();
    const selected = await open({ directory: true, title: "Select a Kiro project" });
    if (selected === null) return;
    await selectProject(selected);
  }

  function handleManageRoots() {
    close();
    onManageRoots();
  }
</script>

{#if isOpen}
  <div class="fixed inset-0 z-40" onclick={close} role="none"></div>
{/if}

<div class="relative">
  <button
    class="flex items-center gap-2 text-sm text-gray-700 dark:text-gray-300 hover:text-gray-900 dark:hover:text-gray-100 transition-colors"
    onclick={toggle}
  >
    <span class="truncate max-w-xs font-medium">
      {store.projectInfo?.path ?? "No project"}
    </span>
    <svg class="w-4 h-4 opacity-50" fill="none" stroke="currentColor" viewBox="0 0 24 24">
      <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M19 9l-7 7-7-7" />
    </svg>
  </button>

  {#if isOpen}
    <div class="absolute right-0 top-full mt-1 w-80 bg-white dark:bg-gray-800 rounded-lg shadow-lg border border-gray-200 dark:border-gray-700 z-50 overflow-hidden">
      {#if store.discoveredProjects.length > 0}
        <div class="max-h-64 overflow-y-auto py-1">
          {#each store.discoveredProjects as project (project.path)}
            <button
              class="w-full text-left px-4 py-2 hover:bg-gray-100 dark:hover:bg-gray-700 transition-colors
                {project.path === store.projectPath ? 'bg-blue-50 dark:bg-blue-900/20' : ''}"
              onclick={() => handleSelectProject(project.path)}
            >
              <div class="text-sm font-medium text-gray-900 dark:text-gray-100">{project.name}</div>
              <div class="text-xs text-gray-500 dark:text-gray-400 truncate">{project.path}</div>
            </button>
          {/each}
        </div>
      {/if}

      <div class="border-t border-gray-200 dark:border-gray-700 py-1">
        <button
          class="w-full text-left px-4 py-2 text-sm text-gray-600 dark:text-gray-400 hover:bg-gray-100 dark:hover:bg-gray-700"
          onclick={handleOpenOther}
        >
          Open Other...
        </button>
        <button
          class="w-full text-left px-4 py-2 text-sm text-gray-600 dark:text-gray-400 hover:bg-gray-100 dark:hover:bg-gray-700"
          onclick={handleManageRoots}
        >
          Manage Directories...
        </button>
      </div>
    </div>
  {/if}
</div>

<script lang="ts">
  import { open } from "@tauri-apps/plugin-dialog";
  import { store, addScanRoot, removeScanRoot } from "$lib/stores/project.svelte";

  let { onClose }: { onClose: () => void } = $props();

  async function handleAddRoot() {
    const selected = await open({ directory: true, title: "Select a directory to scan" });
    if (selected === null) return;
    await addScanRoot(selected);
  }
</script>

<div class="fixed inset-0 z-50 flex items-center justify-center bg-black/50" onclick={onClose} role="none">
  <div
    class="bg-white dark:bg-gray-800 rounded-lg shadow-xl w-full max-w-md mx-4 overflow-hidden"
    onclick={(e) => e.stopPropagation()}
    onkeydown={(e) => { if (e.key === 'Escape') onClose(); }}
    role="dialog"
    aria-label="Manage scan directories"
    tabindex="0"
  >
    <div class="flex items-center justify-between px-4 py-3 border-b border-gray-200 dark:border-gray-700">
      <h2 class="text-sm font-semibold text-gray-900 dark:text-gray-100">Scan Directories</h2>
      <button
        class="text-gray-400 hover:text-gray-600 dark:hover:text-gray-300"
        onclick={onClose}
        aria-label="Close dialog"
      >
        <svg class="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12" />
        </svg>
      </button>
    </div>

    <div class="p-4">
      {#if (store.settings.scan_roots ?? []).length > 0}
        <ul class="space-y-2 mb-4">
          {#each store.settings.scan_roots ?? [] as root (root)}
            <li class="flex items-center justify-between px-3 py-2 bg-gray-50 dark:bg-gray-700/50 rounded text-sm">
              <span class="truncate text-gray-700 dark:text-gray-300">{root}</span>
              <button
                class="ml-2 text-gray-400 hover:text-red-500 flex-shrink-0"
                onclick={() => removeScanRoot(root)}
                aria-label="Remove {root}"
              >
                <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12" />
                </svg>
              </button>
            </li>
          {/each}
        </ul>
      {:else}
        <p class="text-sm text-gray-500 dark:text-gray-400 mb-4">
          No directories configured. Add a directory to discover Kiro projects.
        </p>
      {/if}

      <button
        class="w-full px-4 py-2 rounded-lg border border-dashed border-gray-300 dark:border-gray-600 text-sm text-gray-600 dark:text-gray-400 hover:border-blue-400 hover:text-blue-500 transition-colors"
        onclick={handleAddRoot}
      >
        + Add Directory
      </button>
    </div>
  </div>
</div>

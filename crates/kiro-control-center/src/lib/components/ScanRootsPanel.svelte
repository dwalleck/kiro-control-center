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
    class="bg-kiro-overlay rounded-lg shadow-xl w-full max-w-md mx-4 overflow-hidden"
    onclick={(e) => e.stopPropagation()}
    onkeydown={(e) => { if (e.key === 'Escape') onClose(); }}
    role="dialog"
    aria-label="Manage scan directories"
    tabindex="0"
  >
    <div class="flex items-center justify-between px-4 py-3 border-b border-kiro-muted">
      <h2 class="text-sm font-semibold text-kiro-text">Scan Directories</h2>
      <button
        class="text-kiro-subtle hover:text-kiro-text-secondary"
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
            <li class="flex items-center justify-between px-3 py-2 bg-kiro-overlay rounded text-sm">
              <span class="truncate text-kiro-text-secondary">{root}</span>
              <button
                class="ml-2 text-kiro-subtle hover:text-kiro-error flex-shrink-0"
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
        <p class="text-sm text-kiro-subtle mb-4">
          No directories configured. Add a directory to discover Kiro projects.
        </p>
      {/if}

      <button
        class="w-full px-4 py-2 rounded-lg border border-dashed border-kiro-muted text-sm text-kiro-text-secondary hover:border-kiro-accent-400 hover:text-kiro-accent-400 transition-colors"
        onclick={handleAddRoot}
      >
        + Add Directory
      </button>
    </div>
  </div>
</div>

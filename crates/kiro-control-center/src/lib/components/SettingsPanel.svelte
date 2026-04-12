<script lang="ts">
  import { SvelteMap } from "svelte/reactivity";
  import type { SettingEntry } from "$lib/bindings";
  import SettingControl from "./SettingControl.svelte";

  let {
    entries,
    showCategoryHeaders,
    onUpdate,
  }: {
    entries: SettingEntry[];
    showCategoryHeaders: boolean;
    onUpdate: (updated: SettingEntry) => void;
  } = $props();

  let grouped = $derived.by(() => {
    if (!showCategoryHeaders) return null;
    const map = new SvelteMap<string, SettingEntry[]>();
    for (const entry of entries) {
      const existing = map.get(entry.category_label);
      if (existing) {
        existing.push(entry);
      } else {
        map.set(entry.category_label, [entry]);
      }
    }
    return map;
  });
</script>

<div class="flex-1 overflow-y-auto p-6">
  {#if entries.length === 0}
    <div class="flex items-center justify-center h-32 text-kiro-subtle">
      <p class="text-sm">No settings match the search</p>
    </div>
  {:else if showCategoryHeaders && grouped !== null}
    {#each grouped as [label, group] (label)}
      <section class="mb-8">
        <h2 class="text-xs font-semibold text-kiro-subtle uppercase tracking-wider mb-3">{label}</h2>
        <div class="rounded-lg border border-kiro-muted bg-kiro-surface px-4">
          {#each group as entry (entry.key)}
            <SettingControl {entry} {onUpdate} />
          {/each}
        </div>
      </section>
    {/each}
  {:else}
    <div class="rounded-lg border border-kiro-muted bg-kiro-surface px-4">
      {#each entries as entry (entry.key)}
        <SettingControl {entry} {onUpdate} />
      {/each}
    </div>
  {/if}
</div>

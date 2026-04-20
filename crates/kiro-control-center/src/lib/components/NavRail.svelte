<script lang="ts">
  import type { Tab, SettingCategory } from "$lib/types";

  let {
    activeTab,
    onTabChange,
    settingsCategory,
    onSettingsCategoryChange,
    categories,
  }: {
    activeTab: Tab;
    onTabChange: (tab: Tab) => void;
    settingsCategory: string | null;
    onSettingsCategoryChange: (key: string) => void;
    categories: SettingCategory[];
  } = $props();

  const navGroups: { label: string; items: { id: Tab; hasSubItems?: boolean }[] }[] = [
    { label: "Skills", items: [{ id: "Browse" }, { id: "Installed" }] },
    { label: "Sources", items: [{ id: "Marketplaces" }] },
    { label: "Configuration", items: [{ id: "Kiro Settings", hasSubItems: true }] },
  ];
</script>

<nav class="w-[200px] flex-shrink-0 px-2 py-4 bg-kiro-surface border-r border-kiro-muted flex flex-col gap-1 overflow-y-auto" aria-label="Primary">
  {#each navGroups as group (group.label)}
    <div class="flex flex-col gap-0.5 mb-3">
      <div class="px-2.5 pt-1.5 pb-1 text-[11px] font-semibold uppercase tracking-wider text-kiro-subtle">
        {group.label}
      </div>
      {#each group.items as item (item.id)}
        <button
          type="button"
          aria-current={activeTab === item.id ? "page" : undefined}
          onclick={() => onTabChange(item.id)}
          class="px-2.5 py-1.5 text-sm font-medium text-left rounded-md transition-colors duration-100 font-inherit focus:outline-none focus:ring-2 focus:ring-kiro-accent-500
            {activeTab === item.id
              ? 'bg-kiro-accent-900/30 text-kiro-accent-300'
              : 'text-kiro-text-secondary hover:bg-kiro-overlay hover:text-kiro-text'}"
        >
          {item.id}
        </button>

        {#if item.hasSubItems && activeTab === item.id}
          <div class="flex flex-col gap-px ml-2.5 my-0.5 pl-2.5 border-l border-kiro-muted">
            {#each categories as cat (cat.key)}
              <button
                type="button"
                aria-current={settingsCategory === cat.key ? "true" : undefined}
                onclick={() => onSettingsCategoryChange(cat.key)}
                class="px-2.5 py-1 text-xs text-left rounded transition-colors duration-100 font-inherit focus:outline-none focus:ring-2 focus:ring-kiro-accent-500
                  {settingsCategory === cat.key
                    ? 'bg-kiro-accent-900/25 text-kiro-accent-300 font-medium'
                    : 'text-kiro-text-secondary hover:bg-kiro-overlay hover:text-kiro-text'}"
              >
                {cat.label}
              </button>
            {/each}
          </div>
        {/if}
      {/each}
    </div>
  {/each}
</nav>

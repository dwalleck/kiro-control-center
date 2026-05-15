<script lang="ts">
  import { untrack } from "svelte";
  import { SvelteSet } from "svelte/reactivity";
  import type { PluginCatalogEntryView } from "$lib/bindings";

  /// Diff payload emitted to the BrowseTab apply handler. Per-category
  /// install/remove name lists. Skills join on frontmatter name (catalog
  /// `SkillInfo.name`); steering joins on the file's relative path under
  /// `.kiro/steering/` (catalog `SteeringItemInfo.name`); agents join on
  /// the parsed agent name (catalog `AgentItemInfo.name`).
  ///
  /// All three categories are now per-item granular following kiro-zx73,
  /// which added the four Tauri commands the BrowseTab apply path uses
  /// (installSteeringFiles, removeSteeringFile, installAgents, removeAgent).
  /// The slice-4 noInteractiveItems banner and "read-only" section
  /// labels are gone because every category now toggles.
  export type CustomizeDrawerDiff = {
    skills: { install: string[]; remove: string[] };
    steering: { install: string[]; remove: string[] };
    agents: { install: string[]; remove: string[] };
  };

  type Props = {
    entry: PluginCatalogEntryView;
    marketplace: string;
    onClose: () => void;
    /// Async so the drawer can await the install/remove round-trip and
    /// keep the Apply button in its busy state until the catalog
    /// refresh completes — otherwise the user can re-click Apply
    /// against stale state.
    onApply: (diff: CustomizeDrawerDiff) => Promise<void>;
  };

  let { entry, marketplace, onClose, onApply }: Props = $props();

  // Per-category selection sets — the user-edited "what should be
  // installed when Apply lands." Seeded from current installed flags
  // so the initial diff is empty; every checkbox toggle is a change
  // relative to that baseline.
  //
  // `untrack` makes the "initial-value-only" read explicit — the
  // drawer is destroyed/recreated by the {#if drawerEntry} guard in
  // BrowseTab whenever the user opens a different plugin's drawer,
  // so we deliberately want to capture the entry's flags ONCE at
  // mount and not re-seed on prop changes.
  let selectedSkills = new SvelteSet<string>(
    untrack(() => entry.skills.filter((s) => s.installed).map((s) => s.name)),
  );
  let selectedSteering = new SvelteSet<string>(
    untrack(() => entry.steering.filter((s) => s.installed).map((s) => s.name)),
  );
  let selectedAgents = new SvelteSet<string>(
    untrack(() => entry.agents.filter((a) => a.installed).map((a) => a.name)),
  );

  type SectionToggleState = "empty" | "none" | "partial" | "all";

  function deriveSectionState(
    items: readonly { name: string }[],
    selected: SvelteSet<string>,
  ): SectionToggleState {
    if (items.length === 0) return "empty";
    if (selected.size === 0) return "none";
    if (selected.size === items.length) return "all";
    return "partial";
  }

  function toggleItem(set: SvelteSet<string>, name: string) {
    if (set.has(name)) set.delete(name);
    else set.add(name);
  }

  function toggleAll(
    items: readonly { name: string }[],
    set: SvelteSet<string>,
    state: SectionToggleState,
  ) {
    if (state === "all") {
      set.clear();
    } else {
      set.clear();
      for (const i of items) set.add(i.name);
    }
  }

  const skillsSectionState = $derived.by(() =>
    deriveSectionState(entry.skills, selectedSkills),
  );
  const steeringSectionState = $derived.by(() =>
    deriveSectionState(entry.steering, selectedSteering),
  );
  const agentsSectionState = $derived.by(() =>
    deriveSectionState(entry.agents, selectedAgents),
  );

  // Diff vs the original (initial) installed state. Computed per-
  // category from each entry's `installed` flags, NOT from a snapshot
  // of selectedX at mount — those agree at mount and would diverge
  // pointlessly if the catalog updated underneath the drawer.
  function deriveDiff(
    items: readonly { name: string; installed: boolean }[],
    selected: SvelteSet<string>,
  ): { install: string[]; remove: string[] } {
    const install: string[] = [];
    const remove: string[] = [];
    for (const i of items) {
      const willBeChecked = selected.has(i.name);
      if (willBeChecked && !i.installed) install.push(i.name);
      if (!willBeChecked && i.installed) remove.push(i.name);
    }
    return { install, remove };
  }

  const diff = $derived.by(() => ({
    skills: deriveDiff(entry.skills, selectedSkills),
    steering: deriveDiff(entry.steering, selectedSteering),
    agents: deriveDiff(entry.agents, selectedAgents),
  }));

  const noChanges = $derived(
    diff.skills.install.length === 0
      && diff.skills.remove.length === 0
      && diff.steering.install.length === 0
      && diff.steering.remove.length === 0
      && diff.agents.install.length === 0
      && diff.agents.remove.length === 0,
  );

  // Compose the human summary. Pluralizes per-category nouns and
  // omits zero-count categories so a steering-only diff doesn't
  // read "install 0 skills, install 1 steering, install 0 agents."
  function pluralize(n: number, singular: string, plural: string): string {
    return n === 1 ? singular : plural;
  }

  const summary = $derived.by(() => {
    if (noChanges) return "No changes to apply.";
    const installParts: string[] = [];
    if (diff.skills.install.length > 0) {
      installParts.push(
        `${diff.skills.install.length} ${pluralize(diff.skills.install.length, "skill", "skills")}`,
      );
    }
    if (diff.steering.install.length > 0) {
      installParts.push(
        `${diff.steering.install.length} ${pluralize(diff.steering.install.length, "steering file", "steering files")}`,
      );
    }
    if (diff.agents.install.length > 0) {
      installParts.push(
        `${diff.agents.install.length} ${pluralize(diff.agents.install.length, "agent", "agents")}`,
      );
    }
    const removeParts: string[] = [];
    if (diff.skills.remove.length > 0) {
      removeParts.push(
        `${diff.skills.remove.length} ${pluralize(diff.skills.remove.length, "skill", "skills")}`,
      );
    }
    if (diff.steering.remove.length > 0) {
      removeParts.push(
        `${diff.steering.remove.length} ${pluralize(diff.steering.remove.length, "steering file", "steering files")}`,
      );
    }
    if (diff.agents.remove.length > 0) {
      removeParts.push(
        `${diff.agents.remove.length} ${pluralize(diff.agents.remove.length, "agent", "agents")}`,
      );
    }
    const phrases: string[] = [];
    if (installParts.length > 0) phrases.push(`install ${installParts.join(", ")}`);
    if (removeParts.length > 0) phrases.push(`remove ${removeParts.join(", ")}`);
    return `Apply will ${phrases.join(" and ")}.`;
  });

  let applying = $state(false);

  async function apply() {
    if (noChanges || applying) return;
    applying = true;
    try {
      await onApply({
        skills: {
          install: [...diff.skills.install],
          remove: [...diff.skills.remove],
        },
        steering: {
          install: [...diff.steering.install],
          remove: [...diff.steering.remove],
        },
        agents: {
          install: [...diff.agents.install],
          remove: [...diff.agents.remove],
        },
      });
    } finally {
      applying = false;
    }
  }

  // Esc closes the drawer. Using $effect with the cleanup return so
  // the listener tears down if the drawer is destroyed by something
  // other than the close handler (e.g., parent unmount).
  $effect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !applying) onClose();
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  });
</script>

<!--
  Modal pattern: backdrop + sliding panel anchored to the right edge.
  z-[100] sits above BannerStack (z-50 in app.css) so banners
  triggered by the apply round-trip don't visually compete with the
  drawer footer.
-->
<div
  class="fixed inset-0 z-[100] flex justify-end"
  role="dialog"
  aria-modal="true"
  aria-label="Customize {entry.plugin}"
>
  <button
    type="button"
    aria-label="Close drawer"
    onclick={onClose}
    class="absolute inset-0 bg-kiro-base/70 backdrop-blur-sm border-none cursor-pointer"
  ></button>

  <aside
    class="relative w-[400px] max-w-full bg-kiro-base border-l border-kiro-muted shadow-2xl flex flex-col"
  >
    <header class="flex items-start gap-3 px-4 pt-4 pb-3 border-b border-kiro-muted">
      <div class="flex-1 min-w-0">
        <div class="text-base font-semibold text-kiro-text truncate">{entry.plugin}</div>
        <div class="mt-0.5 text-[11px] text-kiro-subtle">{marketplace}</div>
      </div>
      <button
        type="button"
        onclick={onClose}
        aria-label="Close"
        class="p-1 rounded text-kiro-subtle hover:text-kiro-text hover:bg-kiro-overlay transition-colors bg-transparent border-none cursor-pointer"
      >
        <svg class="w-4 h-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
          <path stroke-linecap="round" d="M6 6l12 12M6 18L18 6" />
        </svg>
      </button>
    </header>

    {#if entry.description}
      <p
        class="px-4 py-3 text-xs text-kiro-text-secondary leading-relaxed border-b border-kiro-muted m-0"
      >
        {entry.description}
      </p>
    {/if}

    <!--
      kiro-zx73 ships per-item granularity for all three categories.
      The slice-4 noInteractiveItems banner is gone — there's no longer
      a "this section is read-only" UX dead-end to explain. Each
      section's toggle-all header + per-item checkbox is interactive
      and contributes to the apply-diff.
    -->

    <div class="flex-1 overflow-y-auto py-1">
      {#snippet sectionHeader(
        label: string,
        installedCount: number,
        total: number,
        state: SectionToggleState,
        onToggleAll: () => void,
      )}
        <button
          type="button"
          onclick={onToggleAll}
          aria-label={`Toggle all ${label}`}
          class="flex w-full items-center gap-2 px-4 pt-3 pb-1.5 bg-kiro-base border-b border-kiro-muted text-[10px] font-semibold uppercase tracking-wider text-kiro-subtle hover:text-kiro-text-secondary transition-colors cursor-pointer"
        >
          <span
            class="inline-block w-3.5 h-3.5 rounded-sm flex-shrink-0 relative
              {state === 'none'
                ? 'bg-transparent border border-kiro-muted'
                : 'bg-kiro-accent-500 border border-kiro-accent-500'}"
          >
            {#if state === "all"}
              <span
                class="absolute left-[3px] top-[1px] w-1 h-2 border-r-2 border-b-2 border-white rotate-45"
              ></span>
            {:else if state === "partial"}
              <span class="absolute left-[3px] right-[3px] top-[5px] h-[2px] bg-white"></span>
            {/if}
          </span>
          <span class="flex-1 text-left">{label}</span>
          <span class="text-[10px] font-medium tracking-normal normal-case text-kiro-text-secondary">
            {installedCount} / {total}
          </span>
        </button>
      {/snippet}

      {#snippet itemRow(
        name: string,
        description: string | null,
        installed: boolean,
        checked: boolean,
        onchange: () => void,
      )}
        <label
          class="flex items-center gap-2 px-4 py-1.5 text-[13px] cursor-pointer transition-colors hover:bg-kiro-accent-900/[0.12] hover:text-kiro-text
            {checked ? 'text-kiro-text' : 'text-kiro-text-secondary'}"
        >
          <input
            type="checkbox"
            checked={checked}
            onchange={onchange}
            disabled={applying}
            class="h-3.5 w-3.5 rounded border-kiro-muted text-kiro-accent-500 focus:ring-kiro-accent-500"
          />
          <span class="flex-1 min-w-0 truncate" title={description ?? ""}>{name}</span>
          {#if installed}
            <span
              class="inline-flex items-center px-1.5 py-0.5 text-[10px] font-medium rounded-full bg-kiro-success/[0.18] text-kiro-success"
            >
              installed
            </span>
          {/if}
        </label>
      {/snippet}

      {#if entry.skills.length > 0}
        {@const installedCount = entry.skills.filter((s) => selectedSkills.has(s.name)).length}
        <section class="mb-2">
          {@render sectionHeader(
            "Skills",
            installedCount,
            entry.skills.length,
            skillsSectionState,
            () => toggleAll(entry.skills, selectedSkills, skillsSectionState),
          )}
          <div class="flex flex-col py-1">
            {#each entry.skills as skill (skill.name)}
              {@render itemRow(
                skill.name,
                skill.description,
                skill.installed,
                selectedSkills.has(skill.name),
                () => toggleItem(selectedSkills, skill.name),
              )}
            {/each}
          </div>
        </section>
      {/if}

      {#if entry.steering.length > 0}
        {@const installedCount = entry.steering.filter((s) => selectedSteering.has(s.name)).length}
        <section class="mb-2">
          {@render sectionHeader(
            "Steering files",
            installedCount,
            entry.steering.length,
            steeringSectionState,
            () => toggleAll(entry.steering, selectedSteering, steeringSectionState),
          )}
          <div class="flex flex-col py-1">
            {#each entry.steering as item (item.name)}
              {@render itemRow(
                item.name,
                null,
                item.installed,
                selectedSteering.has(item.name),
                () => toggleItem(selectedSteering, item.name),
              )}
            {/each}
          </div>
        </section>
      {/if}

      {#if entry.agents.length > 0}
        {@const installedCount = entry.agents.filter((a) => selectedAgents.has(a.name)).length}
        <section class="mb-2">
          {@render sectionHeader(
            "Agents",
            installedCount,
            entry.agents.length,
            agentsSectionState,
            () => toggleAll(entry.agents, selectedAgents, agentsSectionState),
          )}
          <div class="flex flex-col py-1">
            {#each entry.agents as agent (agent.name)}
              {@render itemRow(
                agent.name,
                agent.description,
                agent.installed,
                selectedAgents.has(agent.name),
                () => toggleItem(selectedAgents, agent.name),
              )}
            {/each}
          </div>
        </section>
      {/if}

      {#if entry.skills.length === 0 && entry.steering.length === 0 && entry.agents.length === 0}
        <div class="flex items-center justify-center h-32 text-xs text-kiro-subtle">
          This plugin has no items.
        </div>
      {/if}
    </div>

    <footer
      class="flex flex-col gap-2 px-4 py-3 border-t border-kiro-muted bg-kiro-surface"
    >
      <span class="text-xs text-kiro-text-secondary">{summary}</span>
      <button
        type="button"
        onclick={apply}
        disabled={noChanges || applying}
        aria-busy={applying}
        class="px-4 py-2 text-sm font-medium rounded-md transition-colors
          {noChanges || applying
            ? 'bg-kiro-muted text-kiro-subtle cursor-not-allowed'
            : 'bg-kiro-accent-600 hover:bg-kiro-accent-700 text-white'}"
      >
        {applying ? "Applying…" : noChanges ? "No changes" : "Apply"}
      </button>
    </footer>
  </aside>
</div>

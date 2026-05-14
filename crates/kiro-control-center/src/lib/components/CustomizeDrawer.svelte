<script lang="ts">
  import { untrack } from "svelte";
  import { SvelteSet } from "svelte/reactivity";
  import type { PluginCatalogEntryView } from "$lib/bindings";

  /// Diff payload emitted to the BrowseTab apply handler. Skills carry
  /// install/remove name lists since the catalog and `commands.installSkills`
  /// / `commands.removeSkill` agree on per-skill identity. Steering and
  /// agents are absent from this shape because Option A — chosen during
  /// the BrowseTab redesign falsifiable-design pass — keeps them at
  /// whole-plugin granularity until kiro-zx73 lands per-item Tauri
  /// commands. Adding fields here pre-emptively would invite a future
  /// drawer arm that silently no-ops on apply.
  export type CustomizeDrawerDiff = {
    skills: { install: string[]; remove: string[] };
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

  // Local skill-selection set — the user-edited "what should be
  // installed when Apply lands." Seeded from current installed flags
  // so the initial diff is empty; every checkbox toggle is a change
  // relative to that baseline. Steering/agents are deliberately NOT
  // tracked here (Option A — see CustomizeDrawerDiff doc).
  //
  // `untrack` makes the "initial-value-only" read explicit — the
  // drawer is destroyed/recreated by the {#if drawerEntry} guard in
  // BrowseTab whenever the user opens a different plugin's drawer,
  // so we deliberately want to capture the entry's flags ONCE at
  // mount and not re-seed on prop changes.
  let selectedSkills = new SvelteSet<string>(
    untrack(() => entry.skills.filter((s) => s.installed).map((s) => s.name)),
  );

  function toggleSkill(name: string) {
    if (selectedSkills.has(name)) selectedSkills.delete(name);
    else selectedSkills.add(name);
  }

  type SectionToggleState = "empty" | "none" | "partial" | "all";
  const skillsSectionState = $derived.by<SectionToggleState>(() => {
    if (entry.skills.length === 0) return "empty";
    if (selectedSkills.size === 0) return "none";
    if (selectedSkills.size === entry.skills.length) return "all";
    return "partial";
  });

  function toggleAllSkills() {
    if (skillsSectionState === "all") {
      selectedSkills.clear();
    } else {
      selectedSkills.clear();
      for (const s of entry.skills) selectedSkills.add(s.name);
    }
  }

  // Diff vs the original (initial) installed state. Computed from
  // entry.skills' `installed` flags, NOT from a snapshot of
  // selectedSkills at mount — those agree at mount and would diverge
  // pointlessly if the catalog updated underneath the drawer.
  const diff = $derived.by(() => {
    const install: string[] = [];
    const remove: string[] = [];
    for (const s of entry.skills) {
      const willBeChecked = selectedSkills.has(s.name);
      if (willBeChecked && !s.installed) install.push(s.name);
      if (!willBeChecked && s.installed) remove.push(s.name);
    }
    return { install, remove };
  });

  const noChanges = $derived(
    diff.install.length === 0 && diff.remove.length === 0,
  );

  const summary = $derived.by(() => {
    if (noChanges) return "No changes to apply.";
    const parts: string[] = [];
    if (diff.install.length > 0) {
      const noun = diff.install.length === 1 ? "skill" : "skills";
      parts.push(`install ${diff.install.length} ${noun}`);
    }
    if (diff.remove.length > 0) {
      const noun = diff.remove.length === 1 ? "skill" : "skills";
      parts.push(`remove ${diff.remove.length} ${noun}`);
    }
    return `Apply will ${parts.join(" and ")}.`;
  });

  // True when the drawer has no interactive items — the plugin ships
  // only steering and/or agents (no skills), so every visible
  // checkbox is the disabled Option-A surface. Without an explicit
  // banner, users open Customize and see a wall of grayed-out
  // checkboxes that reads as "broken" rather than "not yet
  // supported." Triggers the explanatory banner below.
  const noInteractiveItems = $derived(
    entry.skills.length === 0
      && (entry.steering.length > 0 || entry.agents.length > 0),
  );

  let applying = $state(false);

  async function apply() {
    if (noChanges || applying) return;
    applying = true;
    try {
      await onApply({
        skills: {
          install: [...diff.install],
          remove: [...diff.remove],
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

    {#if noInteractiveItems}
      <!--
        Skills-less plugin (only steering and/or agents). Without this
        banner, users open Customize and see only disabled checkboxes,
        which reads as "drawer broken." The banner names the limitation
        and points at the remediation.
      -->
      <div
        class="px-4 py-3 text-xs leading-relaxed border-b border-kiro-muted bg-kiro-info/[0.10] text-kiro-info"
      >
        This plugin ships only steering and/or agents. Per-item toggling
        for those categories isn't supported yet — install or update
        them as a whole using the plugin's <strong>Install</strong> /
        <strong>Update</strong> button on the card.
      </div>
    {/if}

    <div class="flex-1 overflow-y-auto py-1">
      <!-- Skills: per-item interactive (Option A's only granular category). -->
      {#if entry.skills.length > 0}
        {@const installedCount = entry.skills.filter((s) => selectedSkills.has(s.name)).length}
        <section class="mb-2">
          <button
            type="button"
            onclick={toggleAllSkills}
            aria-label="Toggle all Skills"
            class="flex w-full items-center gap-2 px-4 pt-3 pb-1.5 bg-kiro-base border-b border-kiro-muted text-[10px] font-semibold uppercase tracking-wider text-kiro-subtle hover:text-kiro-text-secondary transition-colors cursor-pointer"
          >
            <span
              class="inline-block w-3.5 h-3.5 rounded-sm flex-shrink-0 relative
                {skillsSectionState === 'none'
                  ? 'bg-transparent border border-kiro-muted'
                  : 'bg-kiro-accent-500 border border-kiro-accent-500'}"
            >
              {#if skillsSectionState === "all"}
                <span
                  class="absolute left-[3px] top-[1px] w-1 h-2 border-r-2 border-b-2 border-white rotate-45"
                ></span>
              {:else if skillsSectionState === "partial"}
                <span class="absolute left-[3px] right-[3px] top-[5px] h-[2px] bg-white"></span>
              {/if}
            </span>
            <span class="flex-1 text-left">Skills</span>
            <span class="text-[10px] font-medium tracking-normal normal-case text-kiro-text-secondary">
              {installedCount} / {entry.skills.length}
            </span>
          </button>
          <div class="flex flex-col py-1">
            {#each entry.skills as skill (skill.name)}
              {@const checked = selectedSkills.has(skill.name)}
              <label
                class="flex items-center gap-2 px-4 py-1.5 text-[13px] cursor-pointer transition-colors hover:bg-kiro-accent-900/[0.12] hover:text-kiro-text
                  {checked ? 'text-kiro-text' : 'text-kiro-text-secondary'}"
              >
                <input
                  type="checkbox"
                  checked={checked}
                  onchange={() => toggleSkill(skill.name)}
                  disabled={applying}
                  class="h-3.5 w-3.5 rounded border-kiro-muted text-kiro-accent-500 focus:ring-kiro-accent-500"
                />
                <span class="flex-1 min-w-0 truncate" title={skill.description}>
                  {skill.name}
                </span>
                {#if skill.installed}
                  <span
                    class="inline-flex items-center px-1.5 py-0.5 text-[10px] font-medium rounded-full bg-kiro-success/[0.18] text-kiro-success"
                  >
                    installed
                  </span>
                {/if}
              </label>
            {/each}
          </div>
        </section>
      {/if}

      <!--
        Steering and Agents: read-only per Option A. Items render with
        disabled checkboxes + a tooltip pointing users at the whole-
        plugin Install action. The visual parity with the Skills
        section helps users see what an "Install all" would bring in,
        even when they can't toggle individual items today.
        kiro-zx73 widens this to per-item commands (matching the
        Skills surface above).
      -->
      {#if entry.steering.length > 0}
        {@const installedCount = entry.steering.filter((s) => s.installed).length}
        <section class="mb-2">
          <div
            class="flex w-full items-center gap-2 px-4 pt-3 pb-1.5 bg-kiro-base border-b border-kiro-muted text-[10px] font-semibold uppercase tracking-wider text-kiro-subtle"
          >
            <span class="flex-1 text-left">Steering files</span>
            <span class="text-[10px] font-medium normal-case tracking-normal text-kiro-subtle italic">
              read-only
            </span>
            <span class="text-[10px] font-medium tracking-normal normal-case text-kiro-text-secondary">
              {installedCount} / {entry.steering.length}
            </span>
          </div>
          <div class="flex flex-col py-1">
            {#each entry.steering as item (item.name)}
              <div
                class="flex items-center gap-2 px-4 py-1.5 text-[13px] text-kiro-text-secondary"
                title="Per-file steering toggling lands with kiro-zx73; install all steering via the plugin's Install button."
              >
                <input
                  type="checkbox"
                  checked={item.installed}
                  disabled
                  class="h-3.5 w-3.5 rounded border-kiro-muted text-kiro-accent-500 cursor-not-allowed opacity-60"
                />
                <span class="flex-1 min-w-0 truncate">{item.name}</span>
                {#if item.installed}
                  <span
                    class="inline-flex items-center px-1.5 py-0.5 text-[10px] font-medium rounded-full bg-kiro-success/[0.18] text-kiro-success"
                  >
                    installed
                  </span>
                {/if}
              </div>
            {/each}
          </div>
        </section>
      {/if}

      {#if entry.agents.length > 0}
        {@const installedCount = entry.agents.filter((a) => a.installed).length}
        <section class="mb-2">
          <div
            class="flex w-full items-center gap-2 px-4 pt-3 pb-1.5 bg-kiro-base border-b border-kiro-muted text-[10px] font-semibold uppercase tracking-wider text-kiro-subtle"
          >
            <span class="flex-1 text-left">Agents</span>
            <span class="text-[10px] font-medium normal-case tracking-normal text-kiro-subtle italic">
              read-only
            </span>
            <span class="text-[10px] font-medium tracking-normal normal-case text-kiro-text-secondary">
              {installedCount} / {entry.agents.length}
            </span>
          </div>
          <div class="flex flex-col py-1">
            {#each entry.agents as agent (agent.name)}
              <div
                class="flex items-center gap-2 px-4 py-1.5 text-[13px] text-kiro-text-secondary"
                title="Per-agent toggling lands with kiro-zx73; install all agents via the plugin's Install button."
              >
                <input
                  type="checkbox"
                  checked={agent.installed}
                  disabled
                  class="h-3.5 w-3.5 rounded border-kiro-muted text-kiro-accent-500 cursor-not-allowed opacity-60"
                />
                <span class="flex-1 min-w-0 truncate" title={agent.description}>
                  {agent.name}
                </span>
                {#if agent.installed}
                  <span
                    class="inline-flex items-center px-1.5 py-0.5 text-[10px] font-medium rounded-full bg-kiro-success/[0.18] text-kiro-success"
                  >
                    installed
                  </span>
                {/if}
              </div>
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

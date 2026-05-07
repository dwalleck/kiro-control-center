<script lang="ts">
  import type {
    PluginInfo,
    PluginUpdateFailure,
    PluginUpdateInfo,
  } from "$lib/bindings";
  import { actionUpdateLabel, kindLabel } from "$lib/stores/plugin-updates";
  import type { BrowseAction } from "$lib/stores/plugin-updates";
  import { skillCountLabel, skillCountTitle } from "$lib/format";

  type Props = {
    plugin: PluginInfo;
    marketplace: string;
    installed: boolean;
    // Single discriminator carried through from the producer's
    // `pendingPluginActions.get(key)` (a `BrowseAction | undefined`).
    // Importing `BrowseAction` couples this card to the per-tab vocabulary,
    // but the alternative — a duplicated `"install" | "update"` literal
    // — drifts silently if `BrowseAction` ever widens. The single producer
    // today is BrowseTab; if a second consumer with a different action
    // vocabulary materializes, widen this prop to a shared union.
    pending: BrowseAction | undefined;
    update: PluginUpdateInfo | undefined;
    failure: PluginUpdateFailure | undefined;
    projectPicked: boolean;
    onInstall: () => void;
    onUpdate: () => void;
  };

  let {
    plugin,
    marketplace,
    installed,
    pending,
    update,
    failure,
    projectPicked,
    onInstall,
    onUpdate,
  }: Props = $props();

  const installTitle = $derived(
    !projectPicked
      ? "Pick a project first"
      : installed
        ? `${plugin.name} is already installed in this project`
        : `Install ${plugin.name} (skills + steering + agents) into the active project`,
  );

  const updateLabel = $derived(update ? actionUpdateLabel(update) : "Update");

  // Exhaustive label helper for the `pending` discriminator. A future
  // BrowseAction arm becomes a compile error in the default branch
  // rather than silently rendering "Updating".
  function pendingLabel(p: BrowseAction): string {
    switch (p) {
      case "install":
        return "Installing";
      case "update":
        return "Updating";
      default: {
        const _exhaustive: never = p;
        throw new Error(`unhandled BrowseAction: ${JSON.stringify(_exhaustive)}`);
      }
    }
  }
</script>

<div class="flex items-start gap-3 px-3 py-3 rounded-md border border-kiro-muted bg-kiro-overlay">
  <div class="flex-1 min-w-0">
    <div class="flex items-center gap-2 flex-wrap">
      <span class="text-sm font-medium text-kiro-text truncate">{plugin.name}</span>
      <span
        class="text-[11px] {plugin.skill_count.state === 'manifest_failed'
          ? 'text-kiro-warning'
          : 'text-kiro-subtle'} flex-shrink-0"
        title={skillCountTitle(plugin.skill_count)}
        aria-label={skillCountTitle(plugin.skill_count)}
      >
        {skillCountLabel(plugin.skill_count)} skill{plugin.skill_count.state === "known" &&
        plugin.skill_count.count === 1
          ? ""
          : "s"}
      </span>
    </div>
    {#if plugin.description}
      <div class="mt-1 text-xs text-kiro-subtle">{plugin.description}</div>
    {/if}
    <div class="mt-1.5 text-[10px] uppercase tracking-wider text-kiro-subtle">
      {marketplace}
    </div>
  </div>

  <div class="flex flex-col items-end gap-1.5 flex-shrink-0">
    {#if pending}
      {@const label = pendingLabel(pending)}
      <button
        type="button"
        disabled
        aria-busy="true"
        aria-label="{label} {plugin.name}"
        class="px-3 py-1.5 text-xs font-medium rounded-md bg-kiro-muted text-kiro-subtle border border-transparent cursor-not-allowed"
      >
        {label}…
      </button>
    {:else if failure && installed}
      <span
        class="px-2 py-0.5 text-[11px] font-medium text-kiro-error border border-kiro-error/40 rounded"
        title={kindLabel(failure.kind)}
      >
        Update check failed
      </span>
    {:else if update}
      <button
        type="button"
        onclick={onUpdate}
        disabled={!projectPicked}
        title="Update will replace local edits to plugin files"
        aria-label="Update {plugin.name}"
        class="px-3 py-1.5 text-xs font-medium rounded-md transition-colors
          {projectPicked
            ? 'bg-kiro-warning/10 border border-kiro-warning/40 text-kiro-warning hover:bg-kiro-warning/15'
            : 'bg-kiro-muted text-kiro-subtle border border-transparent cursor-not-allowed'}"
      >
        {updateLabel}
      </button>
    {:else if installed}
      <span
        class="px-2 py-0.5 text-[11px] font-medium text-kiro-success border border-kiro-success/40 rounded"
      >
        Installed
      </span>
    {:else}
      <button
        type="button"
        onclick={onInstall}
        disabled={!projectPicked}
        title={installTitle}
        aria-label="Install {plugin.name}"
        class="px-3 py-1.5 text-xs font-medium rounded-md transition-colors
          {projectPicked
            ? 'bg-kiro-overlay border border-kiro-muted text-kiro-accent-300 hover:bg-kiro-muted hover:text-kiro-accent-200'
            : 'bg-kiro-muted text-kiro-subtle border border-transparent cursor-not-allowed'}"
      >
        Install
      </button>
    {/if}
  </div>
</div>

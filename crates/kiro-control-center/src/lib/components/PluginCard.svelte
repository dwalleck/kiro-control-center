<script lang="ts">
  import type {
    PluginCatalogEntryView,
    PluginUpdateFailure,
    PluginUpdateInfo,
  } from "$lib/bindings";
  import { actionUpdateLabel, kindLabel } from "$lib/stores/plugin-updates";
  import type { BrowseAction } from "$lib/stores/plugin-updates";

  type Props = {
    /// Catalog entry from `commands.listPluginCatalogForMarketplace`.
    /// Drives the new three-state visual (stripe + badge + per-category
    /// counts) by aggregating per-item `installed` flags across the
    /// skills, steering, and agents arrays.
    entry: PluginCatalogEntryView;
    marketplace: string;
    /// Whole-plugin tracking state from `installedPluginKeys` (which is
    /// derived from `listInstalledPlugins`). Distinct from the per-item
    /// state derived from `entry`: a plugin can be tracked-as-installed
    /// (this prop true) while having only a subset of its items present
    /// (e.g., user removed one skill via removeSkill after a whole-
    /// plugin install). The button matrix below uses this prop; the
    /// stripe/badge use the per-item state. The two signals together
    /// describe both "is this plugin formally installed" and "how much
    /// of it is on disk."
    installed: boolean;
    // Single discriminator carried through from the producer's
    // `pendingPluginActions.get(key)` (a `BrowseAction | undefined`).
    pending: BrowseAction | undefined;
    update: PluginUpdateInfo | undefined;
    failure: PluginUpdateFailure | undefined;
    projectPicked: boolean;
    onInstall: () => void;
    onUpdate: () => void;
    /// Slice 4: opens the customize drawer for per-skill granular
    /// install/remove. No-op-tolerant — the parent decides whether
    /// the click does anything (e.g., disabled when no project
    /// picked). The drawer caller is BrowseTab; the card just emits.
    onCustomize: () => void;
  };

  let {
    entry,
    marketplace,
    installed,
    pending,
    update,
    failure,
    projectPicked,
    onInstall,
    onUpdate,
    onCustomize,
  }: Props = $props();

  // Per-item state, derived purely from the catalog entry's per-item
  // `installed` flags. Independent of the `installed` prop (which
  // tracks whole-plugin install state via installed-plugins.json).
  // The stripe color and badge consume itemState; the button matrix
  // consumes BOTH itemState and the `installed` prop via
  // `effectiveInstalled` below — see its doc-comment for the
  // two-signal reconciliation rationale.
  type ItemState = "not_installed" | "partial" | "installed";
  const counts = $derived.by(() => {
    const all = [...entry.skills, ...entry.steering, ...entry.agents];
    const total = all.length;
    const installedCount = all.filter((i) => i.installed).length;
    let state: ItemState;
    if (total === 0 || installedCount === 0) state = "not_installed";
    else if (installedCount === total) state = "installed";
    else state = "partial";
    return { installed: installedCount, total, state };
  });

  // Reconcile the two install signals for the button matrix. The
  // `installed` prop tracks `.kiro/installed-plugins.json` (whole-
  // plugin install via commands.installPlugin); per-item flags track
  // the per-category tracking files (commands.installSkills,
  // installPluginSteering, etc.). A plugin installed entirely via
  // the Skills view's multi-select would have `installed=false` BUT
  // every per-item flag true — without this reconciliation the card
  // would show "[Customize] [Install]" while the header pill said
  // "Installed", which users correctly read as broken.
  //
  // Treats per-item-fully-installed as "installed enough" for button
  // purposes, so the user sees [Manage] and can curate via the
  // drawer. The `installed` prop still gates `failure` and `update`
  // branches because those signals are only emitted for plugins
  // tracked in installed-plugins.json (the update-check store keys
  // on whole-plugin tracking).
  const effectiveInstalled = $derived(installed || counts.state === "installed");

  // Stripe color uses a switch with `const _exhaustive: never` per
  // CLAUDE.md's discriminator-pushdown discipline so a future
  // ItemState arm becomes a compile error rather than a silent
  // fallback to one of the existing colors.
  const stripeClass = $derived.by(() => {
    switch (counts.state) {
      case "installed":
        return "border-l-kiro-success";
      case "partial":
        return "border-l-kiro-warning";
      case "not_installed":
        return "border-l-kiro-accent-800";
      default: {
        const _exhaustive: never = counts.state;
        throw new Error(`unhandled ItemState: ${JSON.stringify(_exhaustive)}`);
      }
    }
  });

  // Per-category subline: only show categories that have items, so an
  // agent-less plugin doesn't read "5 skills · 0 steering · 0 agents."
  const categorySummary = $derived.by(() => {
    const parts: string[] = [];
    if (entry.skills.length > 0) {
      parts.push(`${entry.skills.length} skill${entry.skills.length === 1 ? "" : "s"}`);
    }
    if (entry.steering.length > 0) {
      parts.push(
        `${entry.steering.length} steering`,
      );
    }
    if (entry.agents.length > 0) {
      parts.push(`${entry.agents.length} agent${entry.agents.length === 1 ? "" : "s"}`);
    }
    return parts.length === 0 ? "no items" : parts.join(" · ");
  });

  const installTitle = $derived(
    !projectPicked
      ? "Pick a project first"
      : installed
        ? `${entry.plugin} is already installed in this project`
        : `Install ${entry.plugin} (skills + steering + agents) into the active project`,
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

<div
  class="flex items-start gap-3 px-3 py-3 rounded-md border border-kiro-muted bg-kiro-overlay border-l-2 {stripeClass}"
>
  <div class="flex-1 min-w-0">
    <div class="flex items-center gap-2 flex-wrap">
      <span class="text-sm font-medium text-kiro-text truncate">{entry.plugin}</span>
      {#if counts.state === "installed"}
        <span
          class="inline-flex items-center px-2 py-0.5 text-[11px] font-medium rounded-full bg-kiro-success/15 text-kiro-success"
        >
          Installed
        </span>
      {:else if counts.state === "partial"}
        <span
          class="inline-flex items-center px-2 py-0.5 text-[11px] font-medium rounded-full bg-kiro-warning/15 text-kiro-warning"
        >
          {counts.installed} of {counts.total} installed
        </span>
      {/if}
    </div>
    <div class="mt-1 text-[10px] uppercase tracking-wider text-kiro-subtle">
      {marketplace} <span class="normal-case tracking-normal text-kiro-subtle">· {categorySummary}</span>
    </div>
    {#if entry.description}
      <div class="mt-1 text-xs text-kiro-subtle">{entry.description}</div>
    {/if}
  </div>

  <div class="flex items-center gap-1.5 flex-shrink-0">
    {#if pending}
      {@const label = pendingLabel(pending)}
      <button
        type="button"
        disabled
        aria-busy="true"
        aria-label="{label} {entry.plugin}"
        class="px-3 py-1.5 text-xs font-medium rounded-md bg-kiro-muted text-kiro-subtle border border-transparent cursor-not-allowed"
      >
        {label}…
      </button>
    {:else if failure && installed}
      <!--
        Update check failed: show the error chip AND a Manage button so
        the user can still open the drawer to inspect / curate items.
        Without the Manage button this branch was actionless, leaving
        users stranded after a transient update-check error.
      -->
      <span
        class="px-2 py-0.5 text-[11px] font-medium text-kiro-error border border-kiro-error/40 rounded"
        title={kindLabel(failure.kind)}
      >
        Update check failed
      </span>
      <button
        type="button"
        onclick={onCustomize}
        disabled={!projectPicked}
        aria-label="Manage {entry.plugin}"
        title={projectPicked ? `Manage installed items for ${entry.plugin}` : "Pick a project first"}
        class="px-3 py-1.5 text-xs font-medium rounded-md transition-colors
          {projectPicked
            ? 'bg-transparent border border-kiro-muted text-kiro-text-secondary hover:bg-kiro-muted hover:text-kiro-text'
            : 'bg-kiro-muted text-kiro-subtle border border-transparent cursor-not-allowed'}"
      >
        Manage
      </button>
    {:else if update}
      <!-- Update available: pair with Customize so the user can preview/edit before updating. -->
      <button
        type="button"
        onclick={onCustomize}
        disabled={!projectPicked}
        aria-label="Customize {entry.plugin}"
        title={projectPicked ? `Customize installed items for ${entry.plugin}` : "Pick a project first"}
        class="px-3 py-1.5 text-xs font-medium rounded-md transition-colors
          {projectPicked
            ? 'bg-transparent border border-kiro-muted text-kiro-text-secondary hover:bg-kiro-muted hover:text-kiro-text'
            : 'bg-kiro-muted text-kiro-subtle border border-transparent cursor-not-allowed'}"
      >
        Customize
      </button>
      <button
        type="button"
        onclick={onUpdate}
        disabled={!projectPicked}
        title="Update will replace local edits to plugin files"
        aria-label="Update {entry.plugin}"
        class="px-3 py-1.5 text-xs font-medium rounded-md transition-colors
          {projectPicked
            ? 'bg-kiro-warning/10 border border-kiro-warning/40 text-kiro-warning hover:bg-kiro-warning/15'
            : 'bg-kiro-muted text-kiro-subtle border border-transparent cursor-not-allowed'}"
      >
        {updateLabel}
      </button>
    {:else if effectiveInstalled}
      <!--
        Installed (either via whole-plugin tracking OR fully-installed
        per-item — see effectiveInstalled doc). Manage opens the
        drawer for add/remove.
      -->
      <button
        type="button"
        onclick={onCustomize}
        disabled={!projectPicked}
        aria-label="Manage {entry.plugin}"
        title={projectPicked ? `Manage installed items for ${entry.plugin}` : "Pick a project first"}
        class="px-3 py-1.5 text-xs font-medium rounded-md transition-colors
          {projectPicked
            ? 'bg-transparent border border-kiro-muted text-kiro-text-secondary hover:bg-kiro-muted hover:text-kiro-text'
            : 'bg-kiro-muted text-kiro-subtle border border-transparent cursor-not-allowed'}"
      >
        Manage
      </button>
    {:else}
      <!--
        Not installed (any per-item state, including partial via skill-
        view installs). Customize opens the drawer for granular pick;
        Install does the whole-plugin runPluginInstall path. Both go
        through the same pendingPluginActions guard so a click on
        either correctly disables BOTH while the action runs.
      -->
      <button
        type="button"
        onclick={onCustomize}
        disabled={!projectPicked}
        aria-label="Customize {entry.plugin}"
        title={projectPicked ? `Customize which items to install for ${entry.plugin}` : "Pick a project first"}
        class="px-3 py-1.5 text-xs font-medium rounded-md transition-colors
          {projectPicked
            ? 'bg-transparent border border-kiro-muted text-kiro-text-secondary hover:bg-kiro-muted hover:text-kiro-text'
            : 'bg-kiro-muted text-kiro-subtle border border-transparent cursor-not-allowed'}"
      >
        Customize
      </button>
      <button
        type="button"
        onclick={onInstall}
        disabled={!projectPicked}
        title={installTitle}
        aria-label="Install {entry.plugin}"
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

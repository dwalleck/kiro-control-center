<script lang="ts">
  import type { PluginInfo } from "$lib/bindings";
  import { skillCountLabel, skillCountTitle } from "$lib/format";

  type Props = {
    plugin: PluginInfo;
    marketplace: string;
    installed: boolean;
    installing: boolean;
    projectPicked: boolean;
    onInstall: () => void;
  };

  let {
    plugin,
    marketplace,
    installed,
    installing,
    projectPicked,
    onInstall,
  }: Props = $props();

  const title = $derived(
    !projectPicked
      ? "Pick a project first"
      : installed
        ? `${plugin.name} is already installed in this project`
        : `Install ${plugin.name} (skills + steering + agents) into the active project`,
  );
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
    {#if installed}
      <span
        class="px-2 py-0.5 text-[11px] font-medium text-kiro-success border border-kiro-success/40 rounded"
      >
        Installed
      </span>
    {:else}
      <button
        type="button"
        onclick={onInstall}
        disabled={!projectPicked || installing}
        aria-busy={installing}
        {title}
        aria-label="Install {plugin.name}"
        class="px-3 py-1.5 text-xs font-medium rounded-md transition-colors
          {projectPicked && !installing
            ? 'bg-kiro-overlay border border-kiro-muted text-kiro-accent-300 hover:bg-kiro-muted hover:text-kiro-accent-200'
            : 'bg-kiro-muted text-kiro-subtle border border-transparent cursor-not-allowed'}"
      >
        {installing ? "Installing…" : "Install"}
      </button>
    {/if}
  </div>
</div>

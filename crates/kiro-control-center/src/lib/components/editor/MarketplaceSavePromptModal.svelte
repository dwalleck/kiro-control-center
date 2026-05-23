<script lang="ts">
  // Save-time prompt for marketplace-tracked agents (slice S16).
  //
  // Shown by AgentEditor when the user clicks Save on an agent
  // whose row carries marketplace lineage. Two clearly-labeled
  // actions:
  //   - **Keep linked**: save with detach=false. The lineage entry
  //     in installed-agents.json is preserved; future plugin
  //     updates can compare against installed_hash.
  //   - **Detach**: save with detach=true. The lineage entry is
  //     removed; the agent becomes user-authored.
  //
  // Cancel (Escape, backdrop click, or the explicit Cancel button)
  // returns the editor to its pre-Save state without writing.
  //
  // The modal does **not** call IPC itself — it returns a
  // SaveChoice (or cancel signal) up to AgentEditor, which then
  // calls saveUserAgent with the appropriate detach flag. Per
  // amendment A1: the post-save SaveOutcome handling lives in
  // AgentEditor, not here.

  import type { SaveChoice } from "$lib/save-params";

  type Props = {
    open: boolean;
    agentName: string;
    marketplace: string;
    plugin: string;
    version: string | null;
    onChoice: (choice: SaveChoice) => void;
    onCancel: () => void;
  };

  let { open, agentName, marketplace, plugin, version, onChoice, onCancel }:
    Props = $props();

  // Focus the "Keep linked" button when the modal opens. Default
  // action: the safer choice (preserves lineage). The user opts
  // into Detach explicitly.
  let keepLinkedRef: HTMLButtonElement | undefined = $state();

  $effect(() => {
    if (open && keepLinkedRef) {
      // Defer to next microtask so the button is mounted before we
      // grab focus. Without this, focus() can be a no-op when the
      // {#if open} block is rendering on the same tick.
      queueMicrotask(() => keepLinkedRef?.focus());
    }
  });

  function handleKey(e: KeyboardEvent) {
    if (!open) return;
    if (e.key === "Escape") {
      e.preventDefault();
      onCancel();
    }
  }

  function handleBackdropClick(e: MouseEvent) {
    // Only the backdrop itself triggers cancel — clicks inside the
    // dialog card propagate up but the target check filters them
    // out. Without this, every button click would also trigger
    // cancel via event bubbling.
    if (e.target === e.currentTarget) {
      onCancel();
    }
  }

  let badgeLabel = $derived(
    version
      ? `${marketplace} · ${plugin} · ${version}`
      : `${marketplace} · ${plugin}`,
  );
</script>

<svelte:window onkeydown={handleKey} />

{#if open}
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    role="presentation"
    onclick={handleBackdropClick}
    class="fixed inset-0 z-50 flex items-center justify-center bg-black/50 backdrop-blur-sm p-4"
  >
    <div
      role="dialog"
      aria-modal="true"
      aria-labelledby="save-prompt-title"
      aria-describedby="save-prompt-body"
      class="w-full max-w-md rounded-lg bg-kiro-surface border border-kiro-muted shadow-xl flex flex-col gap-4 p-5"
    >
      <header class="flex flex-col gap-2">
        <h2
          id="save-prompt-title"
          class="text-base font-semibold text-kiro-text"
        >
          Keep marketplace link?
        </h2>
        <span
          class="self-start px-1.5 py-px text-[10px] font-semibold uppercase tracking-wider bg-kiro-accent-900/35 text-kiro-accent-300 rounded"
          title={badgeLabel}
        >
          from {marketplace}
        </span>
      </header>

      <div id="save-prompt-body" class="flex flex-col gap-3 text-sm text-kiro-text-secondary">
        <p>
          The agent <code class="font-mono text-kiro-text">{agentName}</code>
          was installed from
          <strong class="text-kiro-text">{plugin}</strong>
          in the <strong class="text-kiro-text">{marketplace}</strong>
          marketplace{#if version}{" "}({version}){/if}.
        </p>
        <p>
          You can keep the marketplace link so future plugin updates can
          detect content changes, or detach this agent so it becomes
          user-authored and stops tracking the marketplace version.
        </p>
      </div>

      <footer class="flex items-center justify-end gap-2 pt-2">
        <button
          type="button"
          onclick={onCancel}
          class="px-3 py-1.5 text-sm text-kiro-text-secondary hover:text-kiro-text focus:outline-none focus:ring-2 focus:ring-kiro-accent-500 rounded"
        >
          Cancel
        </button>
        <button
          type="button"
          onclick={() => onChoice("detach")}
          class="px-3 py-1.5 text-sm font-medium text-kiro-text-secondary border border-kiro-muted hover:bg-kiro-overlay focus:outline-none focus:ring-2 focus:ring-kiro-accent-500 rounded"
        >
          Detach
        </button>
        <button
          type="button"
          bind:this={keepLinkedRef}
          onclick={() => onChoice("keep-linked")}
          class="px-3 py-1.5 text-sm font-medium bg-kiro-accent-700 hover:bg-kiro-accent-600 text-white focus:outline-none focus:ring-2 focus:ring-kiro-accent-500 rounded"
        >
          Keep linked
        </button>
      </footer>
    </div>
  </div>
{/if}

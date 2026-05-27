<script lang="ts">
  // Auto-allowed tools sub-region of the Tools section (top of
  // ToolsPanel). Ports the React `AllowedToolsList` at
  // `Kiro Control Center Design System/design_handoff_agents/source/
  // AgentEditor.jsx:336`.
  //
  // Two interactions:
  //   1. Remove an existing allowed entry → `onRemove(name)`.
  //   2. Add a new entry, either by picking from the candidate list
  //      (enabled-not-allowed, then catalog-not-enabled-not-allowed)
  //      or by typing free text. Both paths route through `onAdd(name)`
  //      — the parent maps that to `addAllowed(draft, name)` per the
  //      A1 plan amendment (slice-2 plan-slice-2.md): a single
  //      reducer per the React reference's `addAllowed`, no
  //      side-effects on `tools[]`.
  //
  // The "NOT VISIBLE" yellow chip on a picker row surfaces when a
  // candidate is in `allowed[]` already but NOT in `enabled[]` — the
  // independence between visibility and auto-allow that the design
  // intentionally surfaces (claim C3).

  import type { Tool } from "$lib/tools-catalog";

  type Props = {
    allowed: readonly string[];
    enabled: readonly string[];
    catalog: readonly Tool[];
    onAdd: (name: string) => void;
    onRemove: (name: string) => void;
  };

  let { allowed, enabled, catalog, onAdd, onRemove }: Props = $props();

  let adding = $state(false);
  let pickerQuery = $state("");

  // Candidates: enabled-not-allowed first (most likely intent), then
  // catalog-not-enabled-not-allowed (so the user can pre-allow a
  // not-yet-visible native tool — the yellow-chip workflow).
  let candidates = $derived.by(() => {
    const allowedSet = new Set(allowed);
    const enabledNotAllowed = enabled.filter((n) => !allowedSet.has(n));
    const catalogRest = catalog
      .map((t) => t.name)
      .filter((n) => !enabled.includes(n) && !allowedSet.has(n));
    return [...enabledNotAllowed, ...catalogRest];
  });

  let filtered = $derived.by(() => {
    if (!pickerQuery) return candidates;
    const q = pickerQuery.toLowerCase();
    return candidates.filter((n) => n.toLowerCase().includes(q));
  });

  // Free-add is offered when the query doesn't exactly match a
  // candidate AND isn't already allowed. Mirrors the React reference's
  // `isFreeAddable` check (AgentEditor.jsx:356).
  let isFreeAddable = $derived.by(() => {
    const q = pickerQuery.trim();
    if (!q) return false;
    if (allowed.includes(q)) return false;
    if (candidates.includes(q)) return false;
    return true;
  });

  function describe(name: string): string {
    if (name.startsWith("@")) return "External (MCP) tool";
    const t = catalog.find((x) => x.name === name);
    return t ? t.summary : "Custom tool";
  }

  function submit(name: string): void {
    onAdd(name);
    pickerQuery = "";
    adding = false;
  }

  function closePicker(): void {
    pickerQuery = "";
    adding = false;
  }

  function handlePickerKeydown(e: KeyboardEvent): void {
    if (e.key === "Escape") {
      closePicker();
      return;
    }
    if (e.key === "Enter") {
      // Free-add takes precedence over auto-select-first-match when
      // the query is a deliberate novel string. Matches the React
      // reference's two-arm check at AgentEditor.jsx:425-426.
      if (isFreeAddable) {
        submit(pickerQuery.trim());
        return;
      }
      if (filtered[0]) submit(filtered[0]);
    }
  }
</script>

<div class="flex flex-col gap-3 rounded-md border border-kiro-muted bg-kiro-surface p-4">
  <header class="flex items-baseline justify-between gap-2">
    <div class="flex flex-col gap-1">
      <h4 class="flex items-center gap-2 text-sm font-semibold text-kiro-text">
        <span>Auto-allowed tools</span>
        <span
          class="rounded-sm bg-kiro-overlay px-1.5 py-0.5 font-mono text-xs text-kiro-subtle"
        >
          {allowed.length}
        </span>
      </h4>
      <p class="text-xs text-kiro-subtle">
        Tools that run without per-call confirmation. Anything not listed
        prompts the user before execution.
      </p>
    </div>
  </header>

  {#if allowed.length === 0}
    <div
      class="flex items-start gap-3 rounded-md border border-dashed border-kiro-muted bg-kiro-base px-3 py-3"
    >
      <div class="flex flex-col gap-0.5">
        <div class="text-xs font-medium text-kiro-text">No auto-allowed tools</div>
        <div class="text-xs text-kiro-subtle">
          Every tool call will require confirmation.
        </div>
      </div>
    </div>
  {:else}
    <ul class="flex flex-col gap-1">
      {#each allowed as name (name)}
        <li
          class="flex items-center gap-2 rounded-sm border border-kiro-muted bg-kiro-base px-2 py-1.5"
        >
          <code class="font-mono text-xs text-kiro-text">{name}</code>
          <span class="flex-1 truncate text-xs text-kiro-subtle">{describe(name)}</span>
          <button
            type="button"
            class="rounded-sm px-1.5 py-0.5 text-xs text-kiro-subtle hover:bg-kiro-overlay hover:text-kiro-error"
            title={`Remove "${name}" from auto-allow`}
            onclick={() => onRemove(name)}
          >
            ✕
          </button>
        </li>
      {/each}
    </ul>
  {/if}

  {#if !adding}
    <button
      type="button"
      class="self-start rounded-sm border border-kiro-muted bg-kiro-base px-2 py-1 text-xs font-medium text-kiro-text hover:bg-kiro-overlay"
      onclick={() => (adding = true)}
    >
      + Add tool
    </button>
  {:else}
    <div class="flex flex-col gap-2 rounded-md border border-kiro-muted bg-kiro-base p-2">
      <div class="flex items-center gap-2">
        <input
          type="text"
          class="flex-1 rounded-sm border border-kiro-muted bg-kiro-surface px-2 py-1 font-mono text-xs text-kiro-text focus:border-kiro-accent-500 focus:outline-none"
          placeholder="Search tools or type a custom name..."
          bind:value={pickerQuery}
          onkeydown={handlePickerKeydown}
        />
        <button
          type="button"
          class="rounded-sm px-1.5 py-0.5 text-xs text-kiro-subtle hover:bg-kiro-overlay"
          title="Cancel"
          onclick={closePicker}
        >
          ✕
        </button>
      </div>
      <div class="flex flex-col gap-1">
        {#each filtered.slice(0, 8) as name (name)}
          <button
            type="button"
            class="flex items-center gap-2 rounded-sm px-2 py-1 text-left hover:bg-kiro-overlay"
            onclick={() => submit(name)}
          >
            <code class="font-mono text-xs text-kiro-text">{name}</code>
            <span class="flex-1 truncate text-xs text-kiro-subtle">
              {describe(name)}
            </span>
            {#if !enabled.includes(name)}
              <span
                class="rounded-sm bg-kiro-warning/20 px-1.5 py-0.5 text-[10px] font-semibold uppercase tracking-wide text-kiro-warning"
                title="Not currently visible to this agent"
              >
                not visible
              </span>
            {/if}
          </button>
        {/each}
        {#if filtered.length === 0 && !isFreeAddable}
          <div class="px-2 py-1 text-xs text-kiro-subtle">No matches.</div>
        {/if}
        {#if isFreeAddable}
          <button
            type="button"
            class="flex items-center gap-2 rounded-sm border border-dashed border-kiro-accent-500/40 px-2 py-1 text-left hover:bg-kiro-overlay"
            onclick={() => submit(pickerQuery.trim())}
          >
            <span class="text-xs font-medium text-kiro-accent-500">+ Add custom:</span>
            <code class="font-mono text-xs text-kiro-text">{pickerQuery.trim()}</code>
          </button>
        {/if}
      </div>
    </div>
  {/if}
</div>

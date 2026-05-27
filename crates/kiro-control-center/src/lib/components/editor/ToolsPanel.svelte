<script lang="ts">
  // Tools section of the agent editor (slice S5). Composes three
  // sub-regions per design § 5 of design_handoff_agents/README.md
  // (screenshot 04-tools.png):
  //
  //   1. AllowedToolsList    — auto-allowed picker (top)
  //   2. By-category grid    — native tool checkboxes + alias input
  //   3. External (MCP) list — @-prefixed entries with +Add form
  //
  // The panel is the integration point between the pure-logic
  // reducers in `$lib/tool-state` and the React-ported visual layout.
  // It does not own the draft state — the AgentEditor parent owns
  // it. The panel reads `draft` via props and emits the full updated
  // draft via `onChange(next)`.
  //
  // Cross-section cleanup invariant (claim C2): toggling a native
  // tool off via toggleTool scrubs the name from tools[],
  // allowedTools[], AND toolAliases{} together. The External (MCP)
  // remove path uses the same `toggleTool` reducer — the aliases
  // scrub is a no-op for external tools (no alias UI for them) but
  // the allowedTools scrub matches the React reference's inline
  // cascade at AgentEditor.jsx:326-329.

  import AllowedToolsList from "./AllowedToolsList.svelte";
  import {
    type AddExternalReason,
    type AddExternalResult,
    addAllowed,
    addExternalTool,
    partitionTools,
    removeAllowed,
    setAlias,
    toggleTool,
    type ToolsDraft,
  } from "$lib/tool-state";
  import { CATEGORY_ORDER, TOOLS_CATALOG, type ToolCategory } from "$lib/tools-catalog";

  type Props = {
    draft: ToolsDraft;
    onChange: (next: ToolsDraft) => void;
  };

  let { draft, onChange }: Props = $props();

  // External-MCP form local state. Lives here (not in tool-state.ts)
  // because it's transient UI state, not part of the draft.
  let externalDraft = $state("");
  let externalError = $state<string | null>(null);

  let externalTools = $derived(partitionTools(draft.tools).external);

  // The by-category grid renders TOOLS_CATALOG, not draft.tools — so
  // a user can see and check on a tool they haven't enabled yet. The
  // enabled status comes from draft.tools membership.
  function isEnabled(name: string): boolean {
    return draft.tools.includes(name);
  }

  function isAllowed(name: string): boolean {
    return draft.allowedTools.includes(name);
  }

  function toolsInCategory(cat: ToolCategory) {
    return TOOLS_CATALOG.filter((t) => t.category === cat);
  }

  function handleToggleTool(name: string): void {
    onChange(toggleTool(draft, name));
  }

  function handleAddAllowed(name: string): void {
    onChange(addAllowed(draft, name));
  }

  function handleRemoveAllowed(name: string): void {
    onChange(removeAllowed(draft, name));
  }

  // The External (MCP) remove behavior matches the React reference's
  // inline cascade (AgentEditor.jsx:326-329): scrub from tools[] AND
  // allowedTools[]. toggleTool already does this (plus the
  // toolAliases scrub, which is a harmless no-op for external tools).
  function handleRemoveExternal(name: string): void {
    onChange(toggleTool(draft, name));
  }

  function handleAddExternal(): void {
    const result: AddExternalResult = addExternalTool(draft, externalDraft);
    if (!result.ok) {
      externalError = formatExternalError(result.reason);
      return;
    }
    onChange(result.draft);
    externalDraft = "";
    externalError = null;
  }

  // `AddExternalReason` is imported from `$lib/tool-state` (its single
  // source of truth) rather than re-declared here. Two compile-time
  // safety nets fire when a fourth reason is added in tool-state.ts:
  //   1. The call `formatExternalError(result.reason)` would pass a
  //      widened union into the narrow parameter — fails at the call
  //      site (line above).
  //   2. The `default: _exhaustive: never` arm below is the local
  //      backup: if the parameter type were ever loosened, the switch
  //      itself would fail to compile.
  function formatExternalError(reason: AddExternalReason): string {
    switch (reason) {
      case "empty":
        return "Enter an MCP tool name.";
      case "not-mcp":
        return 'External tool names must start with "@" (e.g. "@server/tool" or "@server").';
      case "duplicate":
        return "That tool is already in the list.";
      default: {
        const _exhaustive: never = reason;
        throw new Error(`unhandled add-external reason: ${JSON.stringify(_exhaustive)}`);
      }
    }
  }

  function handleSetAlias(name: string, value: string): void {
    onChange(setAlias(draft, name, value));
  }
</script>

<div class="flex flex-col gap-5">
  <header class="flex flex-col gap-1">
    <h2 class="text-base font-semibold text-kiro-text">Tools</h2>
    <p class="text-xs text-kiro-subtle">
      Tools this agent can see. Use "@server/tool" or "@server" syntax to
      include MCP tools.
    </p>
  </header>

  <!-- Sub-region 1: AllowedToolsList -->
  <AllowedToolsList
    allowed={draft.allowedTools}
    enabled={draft.tools}
    catalog={TOOLS_CATALOG}
    onAdd={handleAddAllowed}
    onRemove={handleRemoveAllowed}
  />

  <!-- Sub-region 2: by-category grid -->
  <section class="flex flex-col gap-3">
    <div class="flex flex-col gap-1">
      <h4 class="text-sm font-semibold text-kiro-text">Available tools</h4>
      <p class="text-xs text-kiro-subtle">
        Toggle to expose the tool to this agent. Tools that aren't visible
        can never be called.
      </p>
    </div>

    {#each CATEGORY_ORDER as cat (cat)}
      {@const inCat = toolsInCategory(cat)}
      {#if inCat.length > 0}
        <div class="flex flex-col gap-2">
          <div
            class="text-[10px] font-semibold uppercase tracking-wide text-kiro-subtle"
          >
            {cat}
          </div>
          <div class="grid grid-cols-1 gap-2 sm:grid-cols-2">
            {#each inCat as t (t.name)}
              {@const on = isEnabled(t.name)}
              {@const allowed = isAllowed(t.name)}
              {@const aliasVal = draft.toolAliases[t.name] ?? ""}
              <div
                class="flex flex-col gap-2 rounded-md border border-kiro-muted bg-kiro-base p-3"
                class:bg-kiro-surface={on}
              >
                <button
                  type="button"
                  class="flex items-start gap-2 text-left"
                  onclick={() => handleToggleTool(t.name)}
                >
                  <span
                    class="mt-0.5 flex h-4 w-4 flex-shrink-0 items-center justify-center rounded-sm border border-kiro-muted"
                    class:border-kiro-accent-500={on}
                    class:bg-kiro-accent-500={on}
                  >
                    {#if on}
                      <span class="text-[10px] font-bold leading-none text-kiro-base">✓</span>
                    {/if}
                  </span>
                  <div class="flex min-w-0 flex-1 flex-col gap-0.5">
                    <div class="flex items-center gap-2">
                      <code class="font-mono text-xs text-kiro-text">{t.name}</code>
                      {#if allowed}
                        <span
                          class="rounded-sm bg-kiro-accent-500/20 px-1.5 py-0.5 text-[9px] font-semibold uppercase tracking-wide text-kiro-accent-500"
                        >
                          auto-allow
                        </span>
                      {/if}
                    </div>
                    <div class="text-xs text-kiro-subtle">{t.summary}</div>
                  </div>
                </button>
                {#if on}
                  <label class="flex items-center gap-2 pl-6">
                    <span class="text-[10px] font-medium uppercase tracking-wide text-kiro-subtle">
                      Alias
                    </span>
                    <input
                      type="text"
                      class="flex-1 rounded-sm border border-kiro-muted bg-kiro-base px-2 py-1 font-mono text-xs text-kiro-text focus:border-kiro-accent-500 focus:outline-none"
                      placeholder="(none)"
                      value={aliasVal}
                      oninput={(e) => handleSetAlias(t.name, e.currentTarget.value)}
                    />
                  </label>
                {/if}
              </div>
            {/each}
          </div>
        </div>
      {/if}
    {/each}
  </section>

  <!-- Sub-region 3: External (MCP) -->
  <section class="flex flex-col gap-3">
    <div
      class="text-[10px] font-semibold uppercase tracking-wide text-kiro-subtle"
    >
      External (MCP)
    </div>
    <form
      class="flex items-start gap-2"
      onsubmit={(e) => {
        e.preventDefault();
        handleAddExternal();
      }}
    >
      <input
        type="text"
        class="flex-1 rounded-sm border border-kiro-muted bg-kiro-base px-2 py-1 font-mono text-xs text-kiro-text focus:border-kiro-accent-500 focus:outline-none"
        placeholder="e.g. @terraform-mcp/plan or @terraform-mcp"
        bind:value={externalDraft}
        oninput={() => (externalError = null)}
      />
      <button
        type="submit"
        class="rounded-sm border border-kiro-muted bg-kiro-base px-3 py-1 text-xs font-medium text-kiro-text hover:bg-kiro-overlay disabled:opacity-50"
        disabled={!externalDraft.trim().startsWith("@")}
      >
        Add
      </button>
    </form>
    {#if externalError}
      <div class="text-xs text-kiro-error">{externalError}</div>
    {/if}
    {#if externalTools.length === 0}
      <div class="text-xs text-kiro-subtle">No external tools added.</div>
    {:else}
      <div class="flex flex-col gap-1">
        {#each externalTools as name (name)}
          <div
            class="flex items-center gap-2 rounded-sm border border-kiro-muted bg-kiro-base px-2 py-1.5"
          >
            <code class="flex-1 font-mono text-xs text-kiro-text">{name}</code>
            <button
              type="button"
              class="rounded-sm px-1.5 py-0.5 text-xs text-kiro-subtle hover:bg-kiro-overlay hover:text-kiro-error"
              title="Remove"
              onclick={() => handleRemoveExternal(name)}
            >
              ✕
            </button>
          </div>
        {/each}
      </div>
    {/if}
  </section>
</div>

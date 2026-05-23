<script lang="ts">
  // Workflows > Agents — editor shell. Renders the topbar, 7-entry
  // section rail, and active panel slot. Owns the draft state, the
  // load-on-mount flow for edit mode, and the save flow that consumes
  // post-A1 `SaveOutcome` and forwards `orphan_left_behind` up to the
  // parent for toast rendering.
  //
  // S13 ships the SHELL only. The Identity panel content (S14) and
  // the System Prompt panel content (S15) are placeholders here —
  // their slices replace those `{:else if section === ...}` arms with
  // proper components. Tools / MCP / Resources / Hooks / Advanced
  // sections (S in slices 2-6) are visibly disabled.
  //
  // The S16 modal that asks "keep linked or detach?" before saving a
  // marketplace-tracked agent has not shipped yet. For S13, save in
  // edit mode hardcodes `detach: false` (preserves lineage). The
  // hardcode is replaced by a SaveChoice modal flow when S16 lands;
  // the editor's contract with the IPC layer is unchanged.

  import { commands } from "$lib/bindings";
  import type { CommandError, UserAgentRow } from "$lib/bindings";
  import type { AgentsTabMode } from "$lib/agent-list-helpers";

  // The editor only handles `new` and `edit` modes. The parent's
  // `{:else}` branch enforces that — `list` never reaches this
  // component. Excluding `list` at the prop boundary lets TS narrow
  // `mode.kind` correctly inside the effect / template without a
  // runtime guard that would never fire.
  type EditorMode = Exclude<AgentsTabMode, { kind: "list" }>;

  type Props = {
    mode: EditorMode;
    projectPath: string;
    onCancel: () => void;
    onSaved: (message: string, orphanPath: string | null) => void;
  };

  let { mode, projectPath, onCancel, onSaved }: Props = $props();

  type SectionId =
    | "identity"
    | "prompt"
    | "tools"
    | "mcp"
    | "resources"
    | "hooks"
    | "advanced";

  // Section rail definition. `enabled: false` rows are visible but
  // unclickable — disabled-section placeholders prevent the slice-N
  // implementer from discovering the rail is incomplete via testing
  // alone. Each disabled row notes its slice for navigation.
  const SECTIONS: ReadonlyArray<{
    id: SectionId;
    label: string;
    enabled: boolean;
    note: string;
  }> = [
    { id: "identity", label: "Identity", enabled: true, note: "" },
    { id: "prompt", label: "System Prompt", enabled: true, note: "" },
    { id: "tools", label: "Tools", enabled: false, note: "Slice 2" },
    { id: "mcp", label: "MCP Servers", enabled: false, note: "Slice 3" },
    { id: "resources", label: "Resources", enabled: false, note: "Slice 4" },
    { id: "hooks", label: "Hooks", enabled: false, note: "Slice 5" },
    { id: "advanced", label: "Advanced", enabled: false, note: "Slice 6" },
  ] as const;

  // The draft is the in-memory editable JSON. We store it as a
  // `Record<string, unknown>` rather than a typed shape because the
  // schema is evolving (slices 2-6 add fields) — the component must
  // round-trip unknown fields verbatim so saving an existing agent
  // doesn't lose data the panels haven't surfaced yet.
  let draft: Record<string, unknown> = $state({});
  let originalName: string = $state("");
  let section: SectionId = $state("identity");
  let loading: boolean = $state(false);
  let loadError: string | null = $state(null);
  let saving: boolean = $state(false);
  let saveError: string | null = $state(null);

  // Frontend mirror of the backend's `AgentName` regex
  // (`^[a-z0-9][a-z0-9-]*$`). The regex must match the Rust validator
  // in `validate_user_agent_name` — drift would let the UI accept
  // names the backend rejects, surfacing as a confusing IPC error
  // after a Save click. Tracked at design-slice-1.md § C3 / spec D14.
  const AGENT_NAME_REGEX = /^[a-z0-9][a-z0-9-]*$/;

  let isNew = $derived(mode.kind === "new");
  let editingRow: UserAgentRow | null = $derived(
    mode.kind === "edit" ? mode.row : null,
  );
  let draftName = $derived(
    typeof draft.name === "string" ? draft.name : "",
  );
  let titleLabel = $derived(
    isNew ? "New agent" : draftName || originalName || "Untitled agent",
  );

  // Load existing JSON on mount for edit mode. New mode starts from
  // an empty object — `name` is filled by the user via Identity (S14).
  $effect(() => {
    if (mode.kind === "new") {
      draft = {};
      originalName = "";
      loading = false;
      loadError = null;
      return;
    }
    // Edit mode: pull the file's bytes via the new IPC command so
    // the prompt / tools / etc. round-trip verbatim through save,
    // not just the summary fields available on UserAgentRow.
    const row = mode.row;
    originalName = row.name;
    loading = true;
    loadError = null;
    void (async () => {
      try {
        const result = await commands.loadUserAgentJson(
          row.name,
          projectPath,
        );
        if (result.status === "ok") {
          try {
            const parsed: unknown = JSON.parse(result.data);
            draft =
              parsed && typeof parsed === "object" && !Array.isArray(parsed)
                ? (parsed as Record<string, unknown>)
                : { name: row.name };
          } catch (parseErr) {
            // Malformed JSON on disk. Surface and keep the editor
            // open with a minimal draft so the user can still rename
            // and overwrite if they want; refusing to load would
            // strand them with no way to fix the file from the UI.
            loadError =
              parseErr instanceof Error
                ? `Could not parse agent JSON: ${parseErr.message}`
                : "Could not parse agent JSON";
            draft = { name: row.name };
          }
        } else {
          loadError = describeCommandError(result.error);
          draft = { name: row.name };
        }
      } catch (e) {
        loadError = e instanceof Error ? e.message : String(e);
        draft = { name: row.name };
      } finally {
        loading = false;
      }
    })();
  });

  function describeCommandError(err: CommandError): string {
    return err.message;
  }

  function trySetDraftName(value: string) {
    draft = { ...draft, name: value };
  }

  function validateNameOrError(name: string): string | null {
    if (!name) return "Name is required.";
    if (!AGENT_NAME_REGEX.test(name)) {
      return "Name must be lowercase letters, digits, or hyphens, and start with a letter or digit.";
    }
    return null;
  }

  async function handleSave() {
    if (saving) return;
    saveError = null;

    const validationError = validateNameOrError(draftName);
    if (validationError !== null) {
      saveError = validationError;
      section = "identity";
      return;
    }

    // Ensure the JSON's `name` field matches the filename stem (spec
    // D14). The backend enforces this too; doing it client-side
    // surfaces a clearer error before the IPC roundtrip.
    const finalDraft = { ...draft, name: draftName };
    const draftJson = JSON.stringify(finalDraft, null, 2);

    saving = true;
    try {
      if (mode.kind === "new") {
        const result = await commands.createUserAgent(
          draftName,
          draftJson,
          projectPath,
        );
        if (result.status === "ok") {
          onSaved(`Created “${draftName}”`, null);
        } else {
          saveError = describeCommandError(result.error);
        }
        return;
      }
      // Edit mode. S16 modal not yet shipped: hardcode keep-linked
      // (detach=false). When S16 lands, replace this with a
      // SaveChoice-driven `detach` value; everything else here is
      // already correct.
      const detach = false;
      const result = await commands.saveUserAgent(
        originalName,
        draftName,
        draftJson,
        detach,
        projectPath,
      );
      if (result.status === "ok") {
        // Post-A1: forward `orphan_left_behind` to the parent so the
        // toast can render the rename-orphan warning. Discarding the
        // outcome here would silently hide the partial-success state.
        const orphan = result.data.orphan_left_behind;
        const verb = originalName === draftName ? "Saved" : "Renamed to";
        onSaved(`${verb} “${draftName}”`, orphan);
      } else {
        saveError = describeCommandError(result.error);
      }
    } catch (e) {
      saveError = e instanceof Error ? e.message : String(e);
    } finally {
      saving = false;
    }
  }

  function selectSection(target: SectionId, enabled: boolean) {
    if (!enabled) return;
    section = target;
  }
</script>

<div class="flex flex-col h-full">
  <!-- Topbar -->
  <div
    class="flex items-center gap-3 px-4 py-3 border-b border-kiro-muted"
  >
    <button
      type="button"
      onclick={onCancel}
      class="text-sm text-kiro-text-secondary hover:text-kiro-text flex items-center gap-1 focus:outline-none focus:ring-2 focus:ring-kiro-accent-500 rounded px-1"
    >
      <svg class="w-3.5 h-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M15 19l-7-7 7-7" />
      </svg>
      <span>Agents</span>
    </button>
    <div class="flex items-center gap-2 flex-1 min-w-0">
      <svg class="w-4 h-4 text-kiro-accent-400 flex-shrink-0" fill="currentColor" viewBox="0 0 24 24">
        <path d="M12 2a3 3 0 013 3v1h1a4 4 0 014 4v8a4 4 0 01-4 4H8a4 4 0 01-4-4v-8a4 4 0 014-4h1V5a3 3 0 013-3z" />
      </svg>
      <span class="text-sm font-medium text-kiro-text truncate">{titleLabel}</span>
      {#if !isNew && draftName}
        <code class="text-[11px] font-mono text-kiro-subtle px-1.5 py-0.5 bg-kiro-overlay rounded truncate"
          >.kiro/agents/{draftName}.json</code
        >
      {/if}
    </div>
    <div class="flex items-center gap-2 flex-shrink-0">
      <button
        type="button"
        onclick={onCancel}
        disabled={saving}
        class="px-3 py-1.5 text-sm text-kiro-text-secondary hover:text-kiro-text disabled:opacity-50 disabled:cursor-not-allowed focus:outline-none focus:ring-2 focus:ring-kiro-accent-500 rounded"
      >
        Cancel
      </button>
      <button
        type="button"
        onclick={handleSave}
        disabled={saving || loading}
        class="px-3 py-1.5 text-sm font-medium bg-kiro-accent-700 hover:bg-kiro-accent-600 disabled:opacity-50 disabled:cursor-not-allowed text-white rounded focus:outline-none focus:ring-2 focus:ring-kiro-accent-500"
      >
        {#if saving}
          Saving…
        {:else if isNew}
          Create Agent
        {:else}
          Save Changes
        {/if}
      </button>
    </div>
  </div>

  <!-- Banners -->
  {#if loadError}
    <div
      class="mx-4 mt-3 px-4 py-2 rounded-md text-sm bg-kiro-error/10 border border-kiro-error/30 text-kiro-error flex items-start gap-3"
      role="alert"
    >
      <p class="flex-1">{loadError}</p>
      <button
        type="button"
        aria-label="Dismiss load error"
        onclick={() => (loadError = null)}
        class="opacity-70 hover:opacity-100 text-lg leading-none"
        >×</button
      >
    </div>
  {/if}
  {#if saveError}
    <div
      class="mx-4 mt-3 px-4 py-2 rounded-md text-sm bg-kiro-error/10 border border-kiro-error/30 text-kiro-error flex items-start gap-3"
      role="alert"
    >
      <p class="flex-1">{saveError}</p>
      <button
        type="button"
        aria-label="Dismiss save error"
        onclick={() => (saveError = null)}
        class="opacity-70 hover:opacity-100 text-lg leading-none"
        >×</button
      >
    </div>
  {/if}

  <!-- Body: rail + panel -->
  <div class="flex-1 flex min-h-0">
    <!-- Section rail -->
    <nav class="w-48 flex-shrink-0 border-r border-kiro-muted py-3 overflow-y-auto" aria-label="Editor sections">
      <ul class="flex flex-col gap-0.5 px-2">
        {#each SECTIONS as s (s.id)}
          <li>
            <button
              type="button"
              onclick={() => selectSection(s.id, s.enabled)}
              disabled={!s.enabled}
              aria-current={section === s.id ? "page" : undefined}
              title={s.enabled ? "" : `Coming in ${s.note}`}
              class="w-full flex items-center justify-between gap-2 px-2.5 py-1.5 text-[13px] rounded text-left transition-colors
                {section === s.id && s.enabled
                  ? 'bg-kiro-accent-900/30 text-kiro-accent-300'
                  : s.enabled
                    ? 'text-kiro-text-secondary hover:bg-kiro-overlay hover:text-kiro-text'
                    : 'text-kiro-subtle/60 cursor-not-allowed'}
                focus:outline-none focus:ring-2 focus:ring-kiro-accent-500"
            >
              <span class="truncate">{s.label}</span>
              {#if !s.enabled}
                <span class="text-[10px] uppercase tracking-wider text-kiro-subtle/70">{s.note}</span>
              {/if}
            </button>
          </li>
        {/each}
      </ul>
    </nav>

    <!-- Panel -->
    <div class="flex-1 overflow-y-auto p-6 min-w-0">
      {#if loading}
        <p class="text-sm text-kiro-subtle">Loading agent…</p>
      {:else if section === "identity"}
        <!-- Identity placeholder (S14 fills this in). Exposes a
             single `name` input so the save flow has a draft to send;
             description / model / shortcut / welcome message land in S14. -->
        <div class="max-w-xl flex flex-col gap-4">
          <h2 class="text-base font-semibold text-kiro-text">Identity</h2>
          <p class="text-xs text-kiro-subtle">
            Full Identity panel arrives in slice S14. For now, only the agent
            name is editable here — enough to wire create / rename through
            the save path.
          </p>
          <label class="flex flex-col gap-1.5">
            <span class="text-xs font-medium text-kiro-text-secondary">Name</span>
            <input
              type="text"
              value={draftName}
              oninput={(e) => trySetDraftName((e.currentTarget as HTMLInputElement).value)}
              placeholder="code-reviewer"
              class="px-3 py-1.5 text-sm font-mono bg-kiro-overlay border border-kiro-muted rounded text-kiro-text placeholder:text-kiro-subtle focus:outline-none focus:ring-2 focus:ring-kiro-accent-500"
            />
            <span class="text-[11px] text-kiro-subtle">
              Lowercase letters, digits, and hyphens. Used as the filename in
              <code class="font-mono">.kiro/agents/</code>.
            </span>
          </label>
          {#if !isNew && editingRow?.lineage}
            <div class="px-3 py-2 rounded text-xs bg-kiro-accent-900/20 border border-kiro-accent-800/40 text-kiro-accent-300">
              This agent was installed from
              <strong>{editingRow.lineage.marketplace}</strong> /
              <strong>{editingRow.lineage.plugin}</strong>. Saving keeps the
              marketplace link (slice S16 adds a keep-linked vs detach prompt).
            </div>
          {/if}
        </div>
      {:else if section === "prompt"}
        <!-- Prompt placeholder (S15 fills this in). -->
        <div class="max-w-xl flex flex-col gap-3">
          <h2 class="text-base font-semibold text-kiro-text">System Prompt</h2>
          <p class="text-xs text-kiro-subtle">
            Inline / file-mode prompt editor arrives in slice S15. The current
            agent's <code class="font-mono">prompt</code> field round-trips
            verbatim through save in the meantime.
          </p>
        </div>
      {:else}
        <p class="text-sm text-kiro-subtle">Section not yet implemented.</p>
      {/if}
    </div>
  </div>
</div>

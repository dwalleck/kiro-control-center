<script lang="ts">
  // Workflows > Agents — editor for user-authored agents.
  //
  // Owns:
  //   - the in-memory `draft` (a `name`-bearing open record so
  //     panels can round-trip fields they don't yet surface)
  //   - the load-on-mount flow for edit mode (with a `loadToken`
  //     guard and a `loadFailed` flag separate from `loadError` so
  //     a dismissed banner can't re-enable Save over a synthetic
  //     stub)
  //   - the save flow that consumes `SaveOutcome` and forwards
  //     `orphan_left_behind` to the parent via `onSaved`
  //   - the marketplace-prompt modal trigger (only mounts when
  //     the editing row carries lineage)
  //
  // Identity (`IdentityPanel`) and System Prompt (`PromptPanel`)
  // render in the active section slot. The other five sections
  // (Tools, MCP, Resources, Hooks, Advanced) are visibly disabled
  // until their slices land — see `SECTIONS` below.

  import { commands } from "$lib/bindings";
  import type { UserAgentRow } from "$lib/bindings";
  import type { AgentsTabMode } from "$lib/agent-list-helpers";
  import { validateAgentNameForSave } from "$lib/agent-name";
  import { normalizePromptForSave } from "$lib/prompt-mode";
  import {
    buildSaveParams,
    pickEditSavedVerb,
    shouldPromptForSaveChoice,
    type SaveChoice,
  } from "$lib/save-params";
  import IdentityPanel from "./editor/IdentityPanel.svelte";
  import PromptPanel from "./editor/PromptPanel.svelte";
  import MarketplaceSavePromptModal from "./editor/MarketplaceSavePromptModal.svelte";

  // The editor only handles `new` and `edit` modes. The parent's
  // `{:else}` branch enforces that — `list` never reaches this
  // component. Excluding `list` at the prop boundary lets TS narrow
  // `mode.kind` correctly inside the effect / template without a
  // runtime guard that would never fire.
  type EditorMode = Exclude<AgentsTabMode, { kind: "list" }>;

  // Compile-time exhaustiveness tripwire on EditorMode's `kind` arms.
  // Mirrors the canonical pattern in `agent-list-helpers.ts` (`_KINDS` /
  // `_AssertExhaustive` for `AgentsTabMode`). If a future
  // `AgentsTabMode` arm passes through `Exclude<…, { kind: "list" }>`
  // and lands here without `_EDITOR_MODE_KINDS` being updated, the
  // value-position `_assertEditorMode = true` fails to compile —
  // forcing the implementer to add an explicit case in the `switch`
  // statements below rather than silently routing through the
  // `default` never-arm at runtime.
  const _EDITOR_MODE_KINDS = ["new", "edit"] as const satisfies ReadonlyArray<
    EditorMode["kind"]
  >;
  type _AssertEditorModeExhaustive =
    Exclude<EditorMode["kind"], (typeof _EDITOR_MODE_KINDS)[number]> extends never
      ? true
      : never;
  const _assertEditorMode: _AssertEditorModeExhaustive = true;
  void _assertEditorMode;

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

  // Compile-time exhaustiveness tripwire on `SectionId`. Same pattern
  // as `_EDITOR_MODE_KINDS` above. If `SectionId` gains an eighth arm
  // and `SECTIONS` (or `_SECTION_IDS`) isn't updated, this fails to
  // compile — preventing the silent omission where a new section
  // type lands but the rail doesn't render it.
  const _SECTION_IDS = [
    "identity",
    "prompt",
    "tools",
    "mcp",
    "resources",
    "hooks",
    "advanced",
  ] as const satisfies ReadonlyArray<SectionId>;
  type _AssertSectionIdExhaustive =
    Exclude<SectionId, (typeof _SECTION_IDS)[number]> extends never
      ? true
      : never;
  const _assertSectionId: _AssertSectionIdExhaustive = true;
  void _assertSectionId;

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

  // The draft is the in-memory editable JSON. The shape is `name:
  // string` (always present, the canonical identity per spec D14)
  // intersected with an open record so panels in slices 2-6 can
  // round-trip any field they don't yet surface — saving an existing
  // agent must not lose schema fields the editor doesn't render.
  type Draft = { name: string } & Record<string, unknown>;
  let draft: Draft = $state({ name: "" });
  let originalName: string = $state("");
  let section: SectionId = $state("identity");
  let loading: boolean = $state(false);
  let loadError: string | null = $state(null);
  // **Separate from `loadError`.** The banner text (`loadError`) can
  // be dismissed by the user; this flag cannot. After a load failure
  // the in-memory `draft` is a synthetic stub that does NOT reflect
  // the file on disk — clicking Save would `JSON.stringify` the stub
  // and overwrite the user's content with `{"name": "<stem>"}`,
  // losing every field the panels haven't surfaced. Save is gated on
  // `!loadFailed` to defeat this one-click data-loss path. Per S13
  // review C2 (silent-failure-hunter).
  let loadFailed: boolean = $state(false);
  let saving: boolean = $state(false);
  let saveError: string | null = $state(null);

  // Save-time marketplace-prompt modal state. The modal opens when
  // the user clicks Save on a tracked agent and asks "keep linked
  // or detach?". `pendingDraftJson` snapshots the serialised draft
  // at the moment requestSave fires so the user's choice in the
  // modal applies to that snapshot, making the requestSave ->
  // performSaveEdit boundary deterministic regardless of any
  // background interaction. (The modal sets initial focus to the
  // safer Keep-linked button but does not implement a Tab-trap; the
  // snapshot is what guarantees correctness, not focus management.)
  let savePromptOpen: boolean = $state(false);
  let pendingDraftJson: string | null = $state(null);

  // Token guard against last-write-wins between concurrent loads.
  // Each `$effect` invocation increments this; post-await writes
  // check that the token still matches the current invocation before
  // mutating `draft` / `loadError`. Latent today (the parent unmounts
  // the editor on cancel/save so an in-flight load can't outlive a
  // mode flip) but turns into a real race once S14+ adds in-editor
  // refresh. Per S13 review I3.
  let loadToken = 0;

  // Mode discriminator derivations. Both go through `switch` with a
  // `never`-typed default so a future `EditorMode` arm causes a
  // compile error here rather than silently routing through the
  // existing branches (the `_EDITOR_MODE_KINDS` tripwire catches
  // type additions; these switches catch consumer-side drift).
  function deriveIsNew(m: EditorMode): boolean {
    switch (m.kind) {
      case "new":
        return true;
      case "edit":
        return false;
      default: {
        const _exhaustive: never = m;
        throw new Error(
          `AgentEditor.deriveIsNew: unhandled mode ${JSON.stringify(_exhaustive)}`,
        );
      }
    }
  }

  function deriveEditingRow(m: EditorMode): UserAgentRow | null {
    switch (m.kind) {
      case "new":
        return null;
      case "edit":
        return m.row;
      default: {
        const _exhaustive: never = m;
        throw new Error(
          `AgentEditor.deriveEditingRow: unhandled mode ${JSON.stringify(_exhaustive)}`,
        );
      }
    }
  }

  let isNew = $derived(deriveIsNew(mode));
  let editingRow: UserAgentRow | null = $derived(deriveEditingRow(mode));
  let draftName = $derived(draft.name);
  let titleLabel = $derived(
    isNew ? "New agent" : draftName || originalName || "Untitled agent",
  );

  // Load existing JSON on mount for edit mode. New mode starts from
  // an empty object — `name` is filled by the user via Identity (S14).
  $effect(() => {
    // Reset all transient state at effect entry. Without these
    // resets, a stale `saveError` from a prior mount could attach
    // to a different agent's draft on a future direct-mode-transition
    // flow. Latent today (parent unmounts editor on cancel/save) but
    // defensive against future direct-transition flows. Per S13
    // review I2.
    loadError = null;
    saveError = null;
    loadFailed = false;
    saving = false;
    // Defensive against future direct-mode-transition flows that
    // could run the effect with a save-prompt mid-flight. Today the
    // parent always unmounts the editor on cancel/save, so this is
    // belt-and-braces — costs nothing.
    savePromptOpen = false;
    pendingDraftJson = null;

    // Increment the load token so any previous in-flight load's
    // post-await write becomes a no-op. Per S13 review I3.
    const token = ++loadToken;

    switch (mode.kind) {
      case "new": {
        draft = { name: "" };
        originalName = "";
        loading = false;
        return;
      }
      case "edit": {
        const row = mode.row;
        originalName = row.name;
        loading = true;
        void (async () => {
          try {
            const result = await commands.loadUserAgentJson(
              row.name,
              projectPath,
            );
            // If a newer effect invocation has started while this
            // load was in flight, drop the result silently — the
            // newer effect owns `draft` / `loadError` from this
            // point on.
            if (token !== loadToken) return;
            if (result.status === "ok") {
              try {
                const parsed: unknown = JSON.parse(result.data);
                if (
                  parsed &&
                  typeof parsed === "object" &&
                  !Array.isArray(parsed)
                ) {
                  const obj = parsed as Record<string, unknown>;
                  draft = {
                    ...obj,
                    name:
                      typeof obj.name === "string" ? obj.name : row.name,
                  };
                } else {
                  // Top-level wasn't an object (a JSON array or
                  // primitive). Treat as a parse failure rather than
                  // silently dropping into a synthetic-draft state
                  // the user might Save over.
                  loadError =
                    "Agent file is not a JSON object; refusing to load.";
                  loadFailed = true;
                  draft = { name: row.name };
                }
              } catch (parseErr) {
                // Malformed JSON on disk. The synthetic-draft
                // fallback (`{ name: row.name }`) keeps the editor
                // open so the user sees the error and can navigate
                // away — but `loadFailed` blocks Save so a single
                // dismiss-then-click can't overwrite their content
                // with the stub. Fixed by S13 review C2.
                loadError =
                  parseErr instanceof Error
                    ? `Could not parse agent JSON: ${parseErr.message}`
                    : "Could not parse agent JSON";
                loadFailed = true;
                draft = { name: row.name };
              }
            } else {
              loadError = result.error.message;
              loadFailed = true;
              draft = { name: row.name };
            }
          } catch (e) {
            if (token !== loadToken) return;
            loadError = e instanceof Error ? e.message : String(e);
            loadFailed = true;
            draft = { name: row.name };
          } finally {
            if (token === loadToken) {
              loading = false;
            }
          }
        })();
        return;
      }
      default: {
        // Exhaustiveness sentinel — `_EDITOR_MODE_KINDS` above pins
        // this at compile time. If a third arm slips past the
        // tripwire (e.g., behind a casting bug) the throw makes the
        // failure loud rather than letting a future arm route
        // through the previous-arm's behavior.
        const _exhaustive: never = mode;
        throw new Error(
          `AgentEditor.$effect: unhandled mode arm ${JSON.stringify(_exhaustive)}`,
        );
      }
    }
  });

  function applyDraftPatch(patch: Record<string, string>) {
    // Identity panel + future panels emit string-only patches. The
    // editor folds them into `draft` here. Empty-string-to-null
    // normalisation happens at save time, NOT here, so the user can
    // clear-and-retype a field without losing focus through a
    // derived re-render.
    draft = { ...draft, ...patch };
  }

  // Fields the Identity panel displays as text inputs. The draft is
  // a `Record<string, unknown>` (it round-trips schema fields the
  // panels haven't surfaced yet); these helpers extract the string
  // representation, mapping null / missing / non-string to "" so
  // the inputs render an empty string rather than literal "null".
  function fieldOrEmpty(key: string): string {
    const v = draft[key];
    return typeof v === "string" ? v : "";
  }

  // Convert empty string back to null for save. This is the inverse
  // of `fieldOrEmpty` — every optional Identity field that the user
  // cleared should land in the saved JSON as `null`, not as `""`.
  // The agent-spec.json schema treats null and absent as equivalent;
  // an empty string would be a third state the schema doesn't model.
  // Mirrors the React reference's `handleSave` cleanup at
  // `Kiro Control Center Design System/.../AgentEditor.jsx` lines 41-46.
  function nullIfEmpty(value: unknown): unknown {
    return typeof value === "string" && value === "" ? null : value;
  }

  // The save flow is split into three pieces (per S16):
  //   - `requestSave()` — entry point, called from the Save button.
  //     Validates the draft, snapshots the JSON, then either calls
  //     the IPC directly (new mode, or edit mode without lineage)
  //     OR opens the marketplace-prompt modal (edit mode with
  //     lineage). Sets `saving = true` before either path so the
  //     button disables correctly through the modal.
  //   - `performSaveEdit(draftJson, detach)` — the IPC half for
  //     edit mode. Called either inline by requestSave or by the
  //     modal's onChoice handler.
  //   - `handleSavePromptChoice` / `handleSavePromptCancel` — the
  //     modal callbacks; close the modal and either continue the
  //     save with the chosen detach value or roll back `saving`.

  async function requestSave() {
    if (saving) return;
    // Refuse to save when the load left us with a synthetic-draft
    // stub. The dismiss button on the load-error banner clears the
    // banner text but NOT this gate — defeats the one-click data
    // loss path where the user dismisses the banner and clicks Save.
    // Per S13 review C2.
    if (loadFailed) {
      saveError =
        "Cannot save: the agent file failed to load and the in-memory draft does not reflect the on-disk content. Cancel and reopen, or fix the file out-of-band.";
      return;
    }
    saveError = null;

    const validationError = validateAgentNameForSave(draftName, originalName);
    if (validationError !== null) {
      saveError = validationError;
      section = "identity";
      return;
    }

    // Ensure the JSON's `name` field matches the filename stem (spec
    // D14). The backend enforces this too; doing it client-side
    // surfaces a clearer error before the IPC roundtrip.
    //
    // Normalise empty Identity-field strings to null on save (per the
    // React reference). The agent-spec.json schema treats null and
    // absent as equivalent for optional fields; passing "" would be
    // a third state the schema doesn't model. The prompt field's
    // file-mode null-coercion lives in `normalizePromptForSave` so
    // the whitespace-bypass case (`"file:// "`) has vitest coverage.
    const finalDraft: Record<string, unknown> = {
      ...draft,
      name: draftName,
      description: nullIfEmpty(draft.description),
      model: nullIfEmpty(draft.model),
      keyboardShortcut: nullIfEmpty(draft.keyboardShortcut),
      welcomeMessage: nullIfEmpty(draft.welcomeMessage),
      prompt: normalizePromptForSave(draft.prompt),
    };
    const draftJson = JSON.stringify(finalDraft, null, 2);

    saving = true;
    switch (mode.kind) {
      case "new": {
        try {
          const result = await commands.createUserAgent(
            draftName,
            draftJson,
            projectPath,
          );
          if (result.status === "ok") {
            onSaved(`Created “${draftName}”`, null);
          } else {
            saveError = result.error.message;
          }
        } catch (e) {
          saveError = e instanceof Error ? e.message : String(e);
        } finally {
          saving = false;
        }
        return;
      }
      case "edit": {
        if (shouldPromptForSaveChoice(mode.row)) {
          // Snapshot the draftJson; the modal's onChoice handler
          // picks it up. `saving` stays true so the Save button
          // remains disabled until the modal resolves (either via
          // a choice -> performSaveEdit, or via cancel ->
          // handleSavePromptCancel which clears it).
          pendingDraftJson = draftJson;
          savePromptOpen = true;
          return;
        }
        // No lineage — save directly with detach=false (preserves
        // the user-authored shape, which is the only legitimate
        // value when there's no lineage to detach from).
        await performSaveEdit(draftJson, false);
        return;
      }
      default: {
        const _exhaustive: never = mode;
        throw new Error(
          `AgentEditor.requestSave: unhandled mode arm ${JSON.stringify(_exhaustive)}`,
        );
      }
    }
  }

  async function performSaveEdit(draftJson: string, detach: boolean) {
    try {
      const result = await commands.saveUserAgent(
        originalName,
        draftName,
        draftJson,
        detach,
        projectPath,
      );
      if (result.status === "ok") {
        // Post-A1: forward `orphan_left_behind` to the parent so
        // the toast can render the rename-orphan warning.
        // Discarding the outcome here would silently hide the
        // partial-success state.
        const orphan = result.data.orphan_left_behind;
        const verb = pickEditSavedVerb(originalName, draftName);
        onSaved(`${verb} “${draftName}”`, orphan);
      } else {
        saveError = result.error.message;
      }
    } catch (e) {
      saveError = e instanceof Error ? e.message : String(e);
    } finally {
      saving = false;
    }
  }

  function handleSavePromptChoice(choice: SaveChoice) {
    savePromptOpen = false;
    if (pendingDraftJson === null) {
      // Defensive: the modal can only open after pendingDraftJson
      // is set in requestSave. If we reach this branch, something
      // out-of-flow set savePromptOpen=true; recover by clearing
      // saving so the Save button re-enables.
      saving = false;
      return;
    }
    const params = buildSaveParams(choice, originalName, pendingDraftJson);
    pendingDraftJson = null;
    void performSaveEdit(params.draftJson, params.detach);
  }

  function handleSavePromptCancel() {
    savePromptOpen = false;
    pendingDraftJson = null;
    saving = false;
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
        onclick={requestSave}
        disabled={saving || loading || loadFailed}
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
      <p class="flex-1">
        {loadError}
        {#if loadFailed}
          <span class="block mt-1 text-[12px] opacity-80">
            Save is disabled until the file loads successfully — the in-memory
            draft does not reflect the on-disk content. Cancel and reopen, or
            fix the file out-of-band.
          </span>
        {/if}
      </p>
      {#if !loadFailed}
        <!-- Banner is only dismissable when it does NOT also mean
             Save is gated — otherwise dismissing leaves the user with
             a disabled Save and no visible explanation. -->
        <button
          type="button"
          aria-label="Dismiss load error"
          onclick={() => (loadError = null)}
          class="opacity-70 hover:opacity-100 text-lg leading-none"
          >×</button
        >
      {/if}
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
        <div class="flex flex-col gap-4">
          <IdentityPanel
            name={draftName}
            {originalName}
            description={fieldOrEmpty("description")}
            model={fieldOrEmpty("model")}
            keyboardShortcut={fieldOrEmpty("keyboardShortcut")}
            welcomeMessage={fieldOrEmpty("welcomeMessage")}
            {isNew}
            onPatch={applyDraftPatch}
          />
          {#if !isNew && editingRow?.lineage}
            <div class="max-w-xl px-3 py-2 rounded text-xs bg-kiro-accent-900/20 border border-kiro-accent-800/40 text-kiro-accent-300">
              This agent was installed from
              <strong>{editingRow.lineage.marketplace}</strong> /
              <strong>{editingRow.lineage.plugin}</strong>. Saving will ask
              whether to keep the marketplace link or detach into a
              user-authored copy.
            </div>
          {/if}
        </div>
      {:else if section === "prompt"}
        <PromptPanel
          prompt={fieldOrEmpty("prompt")}
          onPatch={applyDraftPatch}
        />
      {:else}
        <p class="text-sm text-kiro-subtle">Section not yet implemented.</p>
      {/if}
    </div>
  </div>
</div>

<!-- Save-time marketplace prompt. The modal is mounted unconditionally
     so the `open` toggle drives mount/unmount of its content via the
     {#if open} inside the component — keeps focus management and key
     handlers tied to a single Svelte instance. -->
{#if !isNew && editingRow?.lineage}
  <MarketplaceSavePromptModal
    open={savePromptOpen}
    agentName={originalName}
    marketplace={editingRow.lineage.marketplace}
    plugin={editingRow.lineage.plugin}
    version={editingRow.lineage.version}
    onChoice={handleSavePromptChoice}
    onCancel={handleSavePromptCancel}
  />
{/if}

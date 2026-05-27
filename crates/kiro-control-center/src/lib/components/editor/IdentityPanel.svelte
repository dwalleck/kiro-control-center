<script lang="ts">
  // Identity section of the agent editor (slice S14). Five inputs:
  //   - name (required, kebab-case unless unchanged from original)
  //   - description
  //   - model
  //   - keyboardShortcut
  //   - welcomeMessage
  //
  // The parent (AgentEditor) owns the draft state. This panel is a
  // dumb consumer — it reads the current field values via props and
  // emits patches via the `onPatch` callback. Empty-string-to-null
  // normalisation happens at save time in the parent, not here, so
  // the user can clear-and-retype a field without losing focus
  // through a derived re-render.
  //
  // Inline name validation uses `validateAgentNameForSave` so the
  // split-policy escape hatch (per kiro-k9ok) renders correctly:
  // editing a marketplace-installed "Terraform Agent" shows no
  // validation error until the user starts to rename, at which point
  // the strict regex applies to the new name.

  import { validateAgentNameForSave } from "$lib/agent-name";

  type IdentityPatch = {
    name?: string;
    description?: string;
    model?: string;
    keyboardShortcut?: string;
    welcomeMessage?: string;
  };

  type Props = {
    name: string;
    originalName: string;
    description: string;
    model: string;
    keyboardShortcut: string;
    welcomeMessage: string;
    isNew: boolean;
    onPatch: (patch: IdentityPatch) => void;
  };

  let {
    name,
    originalName,
    description,
    model,
    keyboardShortcut,
    welcomeMessage,
    isNew,
    onPatch,
  }: Props = $props();

  // Inline validation message — null when name is acceptable.
  // Renders below the Name input as an inline hint; the AgentEditor
  // also runs the same check at save time so a user who ignores the
  // inline hint and clicks Save still hits the gate.
  let nameError = $derived(validateAgentNameForSave(name, originalName));

  // The "name unchanged" escape hatch can be invisible to the user
  // — they may not know why their not-quite-kebab-case name is
  // accepted. Surface a one-line explanation when it's active.
  let usingEscapeHatch = $derived(
    !isNew &&
      name !== "" &&
      name === originalName &&
      // Only mention the escape hatch when the regex would normally
      // reject the name — saying it for every unchanged kebab name
      // would be noise.
      validateAgentNameForSave(name, "") !== null,
  );
</script>

<div class="max-w-xl flex flex-col gap-5">
  <header class="flex flex-col gap-1">
    <h2 class="text-base font-semibold text-kiro-text">Identity</h2>
    <p class="text-xs text-kiro-subtle">
      How this agent is identified and surfaced in Kiro.
    </p>
  </header>

  <!-- Name -->
  <label class="flex flex-col gap-1.5">
    <span class="text-xs font-medium text-kiro-text-secondary">
      Name <span class="text-kiro-error">*</span>
    </span>
    <input
      type="text"
      value={name}
      oninput={(e) =>
        onPatch({ name: (e.currentTarget as HTMLInputElement).value })}
      placeholder="code-reviewer"
      aria-invalid={nameError !== null}
      aria-describedby={nameError ? "agent-name-error" : "agent-name-hint"}
      class="px-3 py-1.5 text-sm font-mono bg-kiro-overlay border rounded text-kiro-text placeholder:text-kiro-subtle focus:outline-none focus:ring-2 focus:ring-kiro-accent-500
        {nameError ? 'border-kiro-error/60' : 'border-kiro-muted'}"
    />
    {#if nameError}
      <span id="agent-name-error" class="text-[11px] text-kiro-error">
        {nameError}
      </span>
    {:else if usingEscapeHatch}
      <span id="agent-name-hint" class="text-[11px] text-kiro-subtle">
        Existing name kept as-is. Renaming will require lowercase
        letters, digits, or hyphens.
      </span>
    {:else}
      <span id="agent-name-hint" class="text-[11px] text-kiro-subtle">
        Lowercase letters, digits, and hyphens. Used as the filename in
        <code class="font-mono">.kiro/agents/</code>.
      </span>
    {/if}
  </label>

  <!-- Description -->
  <label class="flex flex-col gap-1.5">
    <span class="text-xs font-medium text-kiro-text-secondary">
      Description
    </span>
    <input
      type="text"
      value={description}
      oninput={(e) =>
        onPatch({
          description: (e.currentTarget as HTMLInputElement).value,
        })}
      placeholder="Short description for humans"
      class="px-3 py-1.5 text-sm bg-kiro-overlay border border-kiro-muted rounded text-kiro-text placeholder:text-kiro-subtle focus:outline-none focus:ring-2 focus:ring-kiro-accent-500"
    />
    <span class="text-[11px] text-kiro-subtle">
      Helps you tell agents apart. Not shown to the model.
    </span>
  </label>

  <!-- Model -->
  <label class="flex flex-col gap-1.5">
    <span class="text-xs font-medium text-kiro-text-secondary">Model</span>
    <input
      type="text"
      value={model}
      oninput={(e) =>
        onPatch({ model: (e.currentTarget as HTMLInputElement).value })}
      placeholder="claude-sonnet-4-5"
      class="px-3 py-1.5 text-sm font-mono bg-kiro-overlay border border-kiro-muted rounded text-kiro-text placeholder:text-kiro-subtle focus:outline-none focus:ring-2 focus:ring-kiro-accent-500"
    />
    <span class="text-[11px] text-kiro-subtle">
      Override the default model for this agent. Leave empty to use Kiro's
      default.
    </span>
  </label>

  <!-- Keyboard shortcut -->
  <label class="flex flex-col gap-1.5">
    <span class="text-xs font-medium text-kiro-text-secondary">
      Keyboard shortcut
    </span>
    <input
      type="text"
      value={keyboardShortcut}
      oninput={(e) =>
        onPatch({
          keyboardShortcut: (e.currentTarget as HTMLInputElement).value,
        })}
      placeholder="ctrl+shift+r"
      class="px-3 py-1.5 text-sm font-mono bg-kiro-overlay border border-kiro-muted rounded text-kiro-text placeholder:text-kiro-subtle focus:outline-none focus:ring-2 focus:ring-kiro-accent-500"
    />
    <span class="text-[11px] text-kiro-subtle">
      Quick-swap accelerator. Examples: <code class="font-mono">ctrl+shift+a</code>,
      <code class="font-mono">shift+tab</code>.
    </span>
  </label>

  <!-- Welcome message -->
  <label class="flex flex-col gap-1.5">
    <span class="text-xs font-medium text-kiro-text-secondary">
      Welcome message
    </span>
    <input
      type="text"
      value={welcomeMessage}
      oninput={(e) =>
        onPatch({
          welcomeMessage: (e.currentTarget as HTMLInputElement).value,
        })}
      placeholder="What should I help you with?"
      class="px-3 py-1.5 text-sm bg-kiro-overlay border border-kiro-muted rounded text-kiro-text placeholder:text-kiro-subtle focus:outline-none focus:ring-2 focus:ring-kiro-accent-500"
    />
    <span class="text-[11px] text-kiro-subtle">
      Displayed when switching to this agent.
    </span>
  </label>
</div>

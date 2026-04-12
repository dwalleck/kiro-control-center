<script lang="ts">
  import { commands } from "$lib/bindings";
  import type { SettingEntry, JsonValue } from "$lib/bindings";

  let { entry, onUpdate }: {
    entry: SettingEntry;
    onUpdate: (updated: SettingEntry) => void;
  } = $props();

  let displayValue = $derived(entry.current_value ?? entry.default_value);
  let isModified = $derived(entry.current_value !== null);

  let chipInput = $state("");
  let saving = $state(false);
  let error: string | null = $state(null);

  async function handleSet(value: JsonValue) {
    if (saving) return;
    saving = true;
    error = null;
    try {
      const result = await commands.setKiroSetting(entry.key, value);
      if (result.status === "ok") {
        onUpdate(result.data);
      } else {
        error = result.error.message;
      }
    } catch (e) {
      error = e instanceof Error
        ? `Failed to save: ${e.message}`
        : "Failed to save setting.";
    } finally {
      saving = false;
    }
  }

  async function handleReset() {
    if (saving) return;
    saving = true;
    error = null;
    try {
      const result = await commands.resetKiroSetting(entry.key);
      if (result.status === "ok") {
        onUpdate({ ...entry, current_value: null });
      } else {
        error = result.error.message;
      }
    } catch (e) {
      error = e instanceof Error
        ? `Failed to reset: ${e.message}`
        : "Failed to reset setting.";
    } finally {
      saving = false;
    }
  }

  function handleStringChange(e: Event) {
    const target = e.target as HTMLInputElement;
    handleSet(target.value);
  }

  function handleNumberChange(e: Event) {
    const target = e.target as HTMLInputElement;
    if (target.value === "") {
      handleReset();
      return;
    }
    const num = Number(target.value);
    if (Number.isNaN(num)) {
      error = "Invalid number";
      return;
    }
    handleSet(num);
  }

  function handleCharChange(e: Event) {
    const target = e.target as HTMLInputElement;
    handleSet(target.value.slice(0, 1));
  }

  function handleSelectChange(e: Event) {
    const target = e.target as HTMLSelectElement;
    handleSet(target.value);
  }

  function handleAddChip() {
    const trimmed = chipInput.trim();
    if (!trimmed) return;
    const current = Array.isArray(displayValue) ? (displayValue as string[]) : [];
    handleSet([...current, trimmed]);
    chipInput = "";
  }

  function handleChipKeydown(e: KeyboardEvent) {
    if (e.key === "Enter") {
      e.preventDefault();
      handleAddChip();
    }
  }

  function handleRemoveChip(index: number) {
    const current = Array.isArray(displayValue) ? (displayValue as string[]) : [];
    handleSet(current.filter((_, i) => i !== index));
  }
</script>

<div class="flex items-start justify-between gap-6 py-4 border-b border-kiro-muted last:border-b-0">
  <!-- Left: label, description, key -->
  <div class="flex-1 min-w-0">
    <div class="flex items-center gap-2">
      <span class="font-semibold text-sm text-kiro-text">{entry.label}</span>
      {#if isModified}
        <span class="inline-flex items-center px-1.5 py-0.5 text-xs font-medium rounded-full bg-kiro-accent-900/30 text-kiro-accent-400">
          Modified
        </span>
      {/if}
    </div>
    <p class="mt-0.5 text-sm text-kiro-text-secondary">{entry.description}</p>
    <p class="mt-0.5 font-mono text-[10px] text-kiro-subtle">{entry.key}</p>
    {#if error}
      <p class="mt-1 text-xs text-kiro-error">{error}</p>
    {/if}
  </div>

  <!-- Right: editor + reset -->
  <div class="flex items-center gap-2 flex-shrink-0">
    {#if entry.value_type.kind === "bool"}
      <!-- Toggle switch -->
      <button
        type="button"
        role="switch"
        aria-checked={displayValue === true}
        aria-label="Toggle {entry.label}"
        class="relative inline-flex h-6 w-11 items-center rounded-full transition-colors duration-200 focus:outline-none focus:ring-2 focus:ring-kiro-accent-500 focus:ring-offset-2 focus:ring-offset-kiro-base
          {displayValue === true ? 'bg-kiro-accent-600' : 'bg-kiro-muted'}"
        onclick={() => handleSet(displayValue !== true)}
      >
        <span
          class="inline-block h-4 w-4 transform rounded-full bg-white shadow transition-transform duration-200
            {displayValue === true ? 'translate-x-6' : 'translate-x-1'}"
        ></span>
      </button>

    {:else if entry.value_type.kind === "string"}
      <input
        type="text"
        value={typeof displayValue === "string" ? displayValue : ""}
        onchange={handleStringChange}
        class="w-48 px-2.5 py-1.5 text-sm rounded-md border border-kiro-muted bg-kiro-overlay text-kiro-text placeholder-kiro-subtle focus:outline-none focus:ring-2 focus:ring-kiro-accent-500 focus:border-transparent"
      />

    {:else if entry.value_type.kind === "number"}
      <input
        type="number"
        value={typeof displayValue === "number" ? displayValue : ""}
        placeholder={entry.default_value !== null ? String(entry.default_value) : "not set"}
        onchange={handleNumberChange}
        class="w-24 px-2.5 py-1.5 text-sm rounded-md border border-kiro-muted bg-kiro-overlay text-kiro-text focus:outline-none focus:ring-2 focus:ring-kiro-accent-500 focus:border-transparent"
      />

    {:else if entry.value_type.kind === "char"}
      <input
        type="text"
        maxlength={1}
        value={typeof displayValue === "string" ? displayValue : ""}
        onchange={handleCharChange}
        class="w-12 px-2.5 py-1.5 text-sm text-center rounded-md border border-kiro-muted bg-kiro-overlay text-kiro-text focus:outline-none focus:ring-2 focus:ring-kiro-accent-500 focus:border-transparent"
      />

    {:else if entry.value_type.kind === "string_array"}
      <div class="flex flex-col gap-2 w-64">
        <!-- Chips -->
        {#if Array.isArray(displayValue) && displayValue.length > 0}
          <div class="flex flex-wrap gap-1">
            {#each displayValue as chip, i (i)}
              <span class="inline-flex items-center gap-1 px-2 py-0.5 text-xs rounded-full bg-kiro-overlay border border-kiro-muted text-kiro-text-secondary">
                {chip}
                <button
                  type="button"
                  aria-label="Remove {chip}"
                  class="text-kiro-subtle hover:text-kiro-error transition-colors"
                  onclick={() => handleRemoveChip(i)}
                >
                  <svg class="w-3 h-3" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12" />
                  </svg>
                </button>
              </span>
            {/each}
          </div>
        {/if}
        <!-- Add input -->
        <div class="flex gap-1">
          <input
            type="text"
            placeholder="Add item..."
            bind:value={chipInput}
            onkeydown={handleChipKeydown}
            class="flex-1 px-2 py-1 text-xs rounded-md border border-kiro-muted bg-kiro-overlay text-kiro-text placeholder-kiro-subtle focus:outline-none focus:ring-1 focus:ring-kiro-accent-500 focus:border-transparent"
          />
          <button
            type="button"
            onclick={handleAddChip}
            aria-label="Add item to {entry.label}"
            class="px-2 py-1 text-xs rounded-md bg-kiro-overlay border border-kiro-muted text-kiro-text-secondary hover:bg-kiro-muted transition-colors"
          >
            +
          </button>
        </div>
      </div>

    {:else if entry.value_type.kind === "enum"}
      <select
        value={typeof displayValue === "string" ? displayValue : ""}
        onchange={handleSelectChange}
        class="w-40 px-2.5 py-1.5 text-sm rounded-md border border-kiro-muted bg-kiro-overlay text-kiro-text focus:outline-none focus:ring-2 focus:ring-kiro-accent-500 focus:border-transparent"
      >
        {#each entry.value_type.options as option (option)}
          <option value={option}>{option}</option>
        {/each}
      </select>

    {:else}
      <span class="text-sm text-kiro-subtle italic">Unsupported type</span>
    {/if}

    <!-- Reset button (always rendered to reserve space, invisible when unmodified) -->
    <button
      type="button"
      title="Reset to default"
      aria-label="Reset {entry.label} to default"
      onclick={handleReset}
      disabled={!isModified}
      class="p-1.5 rounded-md transition-colors
        {isModified
          ? 'text-kiro-subtle hover:text-kiro-text-secondary hover:bg-kiro-overlay'
          : 'invisible'}"
    >
      <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15" />
      </svg>
    </button>
  </div>
</div>

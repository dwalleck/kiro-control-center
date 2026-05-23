<script lang="ts">
  // System Prompt section of the agent editor (slice S15). Two
  // mutually-exclusive modes:
  //   - inline: the prompt content is the JSON's `prompt` field
  //     verbatim (typically Markdown). Renders as a textarea with
  //     a character count and a markdown-supported hint.
  //   - file: the JSON's `prompt` field is `file://path/to/prompt.md`.
  //     Renders as a composite input with the `file://` scheme as a
  //     non-editable chip + a path input + a Browse button that opens
  //     a Tauri file-picker dialog.
  //
  // **Switching modes clears the value** per the spec (and the React
  // design reference). The previous mode's content is gone — not
  // remembered. This is intentional: each mode produces a different
  // wire shape, and silently re-using a stale value would let inline
  // text appear as a file path or vice versa.
  //
  // The parent (AgentEditor) owns the draft state. This panel reads
  // `prompt` via props and emits patches via `onPatch`, mirroring
  // IdentityPanel's contract.

  import { open } from "@tauri-apps/plugin-dialog";

  import {
    buildFilePrompt,
    clearPromptOnModeSwitch,
    detectPromptMode,
    filePathFromPrompt,
    type PromptMode,
  } from "$lib/prompt-mode";

  type Props = {
    prompt: string;
    onPatch: (patch: { prompt: string }) => void;
  };

  let { prompt, onPatch }: Props = $props();

  let mode = $derived(detectPromptMode(prompt));
  let filePath = $derived(filePathFromPrompt(prompt));
  let charCount = $derived(prompt.length);

  function switchMode(target: PromptMode) {
    if (target === mode) return;
    onPatch({ prompt: clearPromptOnModeSwitch(target) });
  }

  function onInlineInput(value: string) {
    onPatch({ prompt: value });
  }

  function onFilePathInput(path: string) {
    onPatch({ prompt: buildFilePrompt(path) });
  }

  async function browseForPromptFile() {
    try {
      const selected = await open({
        multiple: false,
        title: "Select a prompt file",
        filters: [
          { name: "Markdown", extensions: ["md", "markdown"] },
          { name: "Text", extensions: ["txt"] },
          { name: "All files", extensions: ["*"] },
        ],
      });
      if (typeof selected === "string") {
        onPatch({ prompt: buildFilePrompt(selected) });
      }
      // `selected === null` (user cancelled) is the legitimate
      // no-op path; falling through with no patch is correct.
    } catch (e) {
      // The dialog plugin can fail (e.g., on a system without a
      // file dialog backend). Don't crash the editor — surface via
      // console.error so a developer running with DevTools open
      // sees it, and let the user fall back to typing the path.
      console.error("Browse dialog failed:", e);
    }
  }
</script>

<div class="max-w-3xl flex flex-col gap-4">
  <header class="flex flex-col gap-1">
    <h2 class="text-base font-semibold text-kiro-text">System Prompt</h2>
    <p class="text-xs text-kiro-subtle">
      The instructions that shape this agent's behavior. Kept inline in
      the agent JSON, or referenced from an external file.
    </p>
  </header>

  <!-- Mode toggle -->
  <div
    role="tablist"
    aria-label="Prompt mode"
    class="inline-flex self-start rounded-md bg-kiro-overlay border border-kiro-muted p-0.5"
  >
    <button
      type="button"
      role="tab"
      aria-selected={mode === "inline"}
      onclick={() => switchMode("inline")}
      class="px-3 py-1 text-xs font-medium rounded transition-colors focus:outline-none focus:ring-2 focus:ring-kiro-accent-500
        {mode === 'inline'
          ? 'bg-kiro-accent-700 text-white'
          : 'text-kiro-text-secondary hover:text-kiro-text'}"
    >
      Inline
    </button>
    <button
      type="button"
      role="tab"
      aria-selected={mode === "file"}
      onclick={() => switchMode("file")}
      class="px-3 py-1 text-xs font-medium rounded transition-colors focus:outline-none focus:ring-2 focus:ring-kiro-accent-500
        {mode === 'file'
          ? 'bg-kiro-accent-700 text-white'
          : 'text-kiro-text-secondary hover:text-kiro-text'}"
    >
      File
    </button>
  </div>

  {#if mode === "inline"}
    <!-- Inline mode: textarea + char count + markdown hint -->
    <div class="flex flex-col gap-1.5">
      <textarea
        value={prompt}
        oninput={(e) =>
          onInlineInput((e.currentTarget as HTMLTextAreaElement).value)}
        placeholder={"You are an expert at...\n\nWhen the user asks for X, do Y."}
        rows="14"
        class="px-3 py-2 text-sm font-mono bg-kiro-overlay border border-kiro-muted rounded text-kiro-text placeholder:text-kiro-subtle resize-y focus:outline-none focus:ring-2 focus:ring-kiro-accent-500"
      ></textarea>
      <div class="flex items-center justify-between text-[11px] text-kiro-subtle">
        <span>Markdown supported.</span>
        <span>{charCount} character{charCount === 1 ? "" : "s"}</span>
      </div>
    </div>
  {:else}
    <!-- File mode: composite input with the file:// chip + Browse -->
    <div class="flex flex-col gap-1.5">
      <div class="flex items-stretch border border-kiro-muted rounded overflow-hidden focus-within:ring-2 focus-within:ring-kiro-accent-500">
        <span
          class="flex items-center px-2.5 text-xs font-mono text-kiro-subtle bg-kiro-overlay border-r border-kiro-muted select-none"
          aria-hidden="true"
        >
          file://
        </span>
        <input
          type="text"
          value={filePath}
          oninput={(e) =>
            onFilePathInput((e.currentTarget as HTMLInputElement).value)}
          placeholder="path/to/prompt.md"
          aria-label="Prompt file path (without the file:// prefix)"
          class="flex-1 px-3 py-1.5 text-sm font-mono bg-kiro-overlay text-kiro-text placeholder:text-kiro-subtle focus:outline-none"
        />
        <button
          type="button"
          onclick={browseForPromptFile}
          class="px-3 text-xs font-medium text-kiro-text-secondary hover:text-kiro-text bg-kiro-overlay border-l border-kiro-muted focus:outline-none focus:ring-2 focus:ring-kiro-accent-500"
        >
          Browse…
        </button>
      </div>
      <span class="text-[11px] text-kiro-subtle">
        Path relative to the project root or absolute. Read at chat-start
        time, so edits to the referenced file take effect immediately.
      </span>
    </div>
  {/if}
</div>

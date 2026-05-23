// Prompt-mode helpers for the agent editor's System Prompt section.
//
// Native Kiro agents store the prompt as either:
//   - inline content (a JSON string with the prompt text), or
//   - an external file reference (`file://path/to/prompt.md`)
//
// The editor surfaces this as a segmented mode toggle between
// "inline" and "file"; switching modes clears the value (the spec
// makes this explicit, and the React design reference matches).
//
// `detectPromptMode` is the wire-format -> view-mode projection;
// `clearPromptOnModeSwitch` produces the post-switch initial value;
// `filePathFromPrompt` / `buildFilePrompt` round-trip the file mode's
// composite input (the displayed text minus the `file://` scheme
// chip).

export type PromptMode = "inline" | "file";

const FILE_PREFIX = "file://";

/**
 * Map a wire-format prompt value to its display mode.
 *
 * Empty strings, null, and any value not starting with the
 * canonical `file://` prefix render in inline mode.
 *
 * **Case-sensitive.** `"File://X"` does NOT count as file mode —
 * the wire format uses lowercase `file://`, and accepting other
 * casings would let the inline-mode textarea render literal
 * `File://` content as if it were a path picker. Pinned by test
 * `detectPromptMode_rejects_uppercase_scheme`.
 */
export function detectPromptMode(value: string | null): PromptMode {
  return (value ?? "").startsWith(FILE_PREFIX) ? "file" : "inline";
}

/**
 * Initial value when switching INTO a target mode.
 *
 * Switching to file mode produces `"file://"` (the user types the
 * path after the chip); switching to inline produces `""`.
 *
 * The spec's "switching modes clears the value" rule lives here:
 * callers do `onPatch({ prompt: clearPromptOnModeSwitch(target) })`
 * and the previous mode's content is gone. Confirmed-clear, not
 * remembered, mirroring the React design reference.
 */
export function clearPromptOnModeSwitch(target: PromptMode): string {
  return target === "file" ? FILE_PREFIX : "";
}

/**
 * Strip the `file://` prefix from a file-mode prompt value to
 * expose the bare path for the file-mode composite input.
 *
 * Returns the empty string when the value isn't actually in file
 * mode — the caller (the panel) only invokes this in file-mode
 * branches, but the defensive "" return prevents a stray
 * `slice(7)` from a future inline value from leaking.
 */
export function filePathFromPrompt(value: string): string {
  if (!value.startsWith(FILE_PREFIX)) return "";
  return value.slice(FILE_PREFIX.length);
}

/**
 * Compose a file-mode prompt value from a bare path. Inverse of
 * `filePathFromPrompt`. Roundtrip property:
 *   filePathFromPrompt(buildFilePrompt(p)) === p
 */
export function buildFilePrompt(path: string): string {
  return FILE_PREFIX + path;
}

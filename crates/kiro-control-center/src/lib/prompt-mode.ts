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

// Compile-time exhaustiveness tripwire on PromptMode. Mirrors the
// canonical pattern in AgentEditor.svelte's `_EDITOR_MODE_KINDS`.
// If a future PromptMode arm lands without `_PROMPT_MODES` being
// updated, the value-position `_assertPromptMode = true` fails to
// compile — forcing the implementer to add explicit cases in the
// switches below rather than silently routing through the existing
// arms.
const _PROMPT_MODES = ["inline", "file"] as const satisfies ReadonlyArray<
  PromptMode
>;
type _AssertPromptModeExhaustive =
  Exclude<PromptMode, (typeof _PROMPT_MODES)[number]> extends never
    ? true
    : never;
const _assertPromptMode: _AssertPromptModeExhaustive = true;
void _assertPromptMode;

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
  switch (target) {
    case "file":
      return FILE_PREFIX;
    case "inline":
      return "";
    default: {
      const _exhaustive: never = target;
      throw new Error(
        `clearPromptOnModeSwitch: unhandled PromptMode ${JSON.stringify(_exhaustive)}`,
      );
    }
  }
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

/**
 * Normalize the draft's `prompt` field at save time.
 *
 * Returns `null` when the value is functionally empty so the saved
 * agent JSON doesn't claim "my prompt is at <path that doesn't
 * exist>" — the agent-spec.json schema treats null and absent as
 * equivalent for optional fields. Three null-coerced cases:
 *   - empty string
 *   - bare `"file://"` (post-mode-switch state, no path typed)
 *   - `"file://<whitespace>"` (path component is all-whitespace —
 *     defeats the gap where typing a single space after switching
 *     to file mode bypassed the bare-`"file://"` exact-match check)
 *
 * Non-string values pass through unchanged so the draft round-trips
 * any field shape the panels haven't surfaced yet.
 *
 * The value is NOT trimmed — `"file:// foo"` is preserved verbatim
 * because that may be the user's actual path. Only fully-empty path
 * components are null-coerced.
 */
export function normalizePromptForSave(value: unknown): unknown {
  if (typeof value !== "string") return value;
  if (value === "") return null;
  if (value.startsWith(FILE_PREFIX)) {
    const path = value.slice(FILE_PREFIX.length);
    if (path.trim() === "") return null;
  }
  return value;
}

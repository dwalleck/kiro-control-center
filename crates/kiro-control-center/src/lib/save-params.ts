// Save-time helpers for the agent editor's marketplace-prompt flow.
//
// When the user clicks Save on an agent that has marketplace lineage
// (it was installed from a plugin), the editor must ask: keep the
// marketplace link (so future plugin updates can compare against the
// installed content), or detach the agent into a user-authored copy
// (which removes the lineage from `installed-agents.json`)?
//
// This module owns the two pure-logic pieces:
//   - `shouldPromptForSaveChoice(row)` — the editor's gate decision
//   - `buildSaveParams(choice, fromName, draftJson)` — the choice ->
//     IPC argument projection
//
// The modal component (MarketplaceSavePromptModal.svelte) is the UI
// presentation; AgentEditor wires the helpers to the modal and the
// IPC call.

import type { UserAgentRow } from "$lib/bindings";

export type SaveChoice = "keep-linked" | "detach";

// Compile-time exhaustiveness tripwire on SaveChoice. If a future
// arm (e.g., "preview-diff") lands without `_SAVE_CHOICES` being
// updated, the value-position `_assertSaveChoice = true` fails to
// compile — forcing the implementer to add an explicit case in
// buildSaveParams' switch rather than silently routing through the
// "not detach" branch and inverting the wire-format semantics.
const _SAVE_CHOICES = ["keep-linked", "detach"] as const satisfies ReadonlyArray<
  SaveChoice
>;
type _AssertSaveChoiceExhaustive =
  Exclude<SaveChoice, (typeof _SAVE_CHOICES)[number]> extends never
    ? true
    : never;
const _assertSaveChoice: _AssertSaveChoiceExhaustive = true;
void _assertSaveChoice;

export type SaveParams = {
  fromName: string;
  draftJson: string;
  /**
   * Wire-format `detach` boolean for `commands.saveUserAgent`.
   * `true` removes the agent's `installed-agents.json` entry as
   * part of the save (the agent becomes user-authored). `false`
   * preserves the lineage entry — future plugin updates can still
   * compare against `installed_hash`.
   */
  detach: boolean;
};

/**
 * Project a user's `SaveChoice` into the IPC argument shape for
 * `commands.saveUserAgent`. Pinned by the 2 plan vitest cases.
 */
export function buildSaveParams(
  choice: SaveChoice,
  fromName: string,
  draftJson: string,
): SaveParams {
  switch (choice) {
    case "keep-linked":
      return { fromName, draftJson, detach: false };
    case "detach":
      return { fromName, draftJson, detach: true };
    default: {
      const _exhaustive: never = choice;
      throw new Error(
        `buildSaveParams: unhandled SaveChoice ${JSON.stringify(_exhaustive)}`,
      );
    }
  }
}

/**
 * Should the editor open the keep-linked-vs-detach modal before
 * calling `saveUserAgent`?
 *
 * Returns `true` iff the row exists AND carries marketplace lineage.
 * For new agents (`row === null`) and user-authored agents
 * (`row.lineage === null`), the editor saves directly with
 * `detach: false` — there's no marketplace link to detach from,
 * so the modal would be a meaningless prompt.
 *
 * Pinned by vitest cases for this helper.
 */
export function shouldPromptForSaveChoice(
  row: UserAgentRow | null,
): boolean {
  return row !== null && row.lineage !== null;
}

/**
 * Pick the past-tense verb for the post-save toast in edit mode.
 * Returns `"Saved"` when the agent's name was unchanged, `"Renamed to"`
 * when the user changed it.
 *
 * The "Created" verb is exclusive to new-mode saves; the editor
 * composes that string inline because it has no rename branch.
 */
export function pickEditSavedVerb(
  originalName: string,
  draftName: string,
): "Saved" | "Renamed to" {
  return originalName === draftName ? "Saved" : "Renamed to";
}

/**
 * Compose the post-save toast text, appending the A1 orphan-path
 * warning when a rename succeeded but the post-rename unlink of the
 * old file failed.
 *
 * Without the suffix, a user who renames `foo` -> `bar` and sees the
 * toast briefly has no signal that `.kiro/agents/foo.json` remains
 * on disk — they might later delete `bar` thinking it's a duplicate
 * of the original. The orphan path makes the partial-success state
 * legible. Only `null` means "no orphan"; an empty string would still
 * render a (degenerate) suffix.
 */
export function formatSavedToast(
  message: string,
  orphanPath: string | null,
): string {
  if (orphanPath === null) return message;
  return `${message} (note: stale file remains at ${orphanPath})`;
}

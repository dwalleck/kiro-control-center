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
  return { fromName, draftJson, detach: choice === "detach" };
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
 * Pinned by the 3 vitest cases for this helper.
 */
export function shouldPromptForSaveChoice(
  row: UserAgentRow | null,
): boolean {
  return row !== null && row.lineage !== null;
}

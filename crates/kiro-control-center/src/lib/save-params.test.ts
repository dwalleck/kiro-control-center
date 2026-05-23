import { describe, expect, test } from "vitest";

import type { UserAgentRow } from "$lib/bindings";

import {
  buildSaveParams,
  formatSavedToast,
  pickEditSavedVerb,
  shouldPromptForSaveChoice,
} from "./save-params";

const ROW_BASE: UserAgentRow = {
  name: "agent",
  description: null,
  model: null,
  tools_count: 0,
  mcp_count: 0,
  resources_count: 0,
  hooks_count: 0,
  lineage: null,
};

describe("buildSaveParams", () => {
  // Plan cases 1-2.
  test("'keep-linked' produces detach: false", () => {
    expect(buildSaveParams("keep-linked", "agent", "{}")).toEqual({
      fromName: "agent",
      draftJson: "{}",
      detach: false,
    });
  });

  test("'detach' produces detach: true", () => {
    expect(buildSaveParams("detach", "agent", "{}")).toEqual({
      fromName: "agent",
      draftJson: "{}",
      detach: true,
    });
  });
});

describe("shouldPromptForSaveChoice", () => {
  // Plan case 3 broken into three explicit branches so the helper's
  // contract is verifiable without standing up a Svelte renderer.
  test("returns false for null (new-agent mode)", () => {
    expect(shouldPromptForSaveChoice(null)).toBe(false);
  });

  test("returns false when row has no lineage (user-authored)", () => {
    expect(shouldPromptForSaveChoice({ ...ROW_BASE, lineage: null })).toBe(
      false,
    );
  });

  test("returns true when row carries marketplace lineage", () => {
    // The bug class this defeats: a misimplementation that opens
    // the modal for every save (even user-authored agents) would
    // surface as a confusing always-on prompt. Conversely, an
    // implementation that never opens the modal would silently
    // keep the lineage on detach-intended saves.
    expect(
      shouldPromptForSaveChoice({
        ...ROW_BASE,
        lineage: {
          marketplace: "tutorials",
          plugin: "claude-skills",
          version: "1.2.3",
        },
      }),
    ).toBe(true);
  });
});

describe("pickEditSavedVerb", () => {
  test("returns 'Saved' when name unchanged", () => {
    expect(pickEditSavedVerb("agent", "agent")).toBe("Saved");
  });

  test("returns 'Renamed to' when name changed", () => {
    expect(pickEditSavedVerb("old-name", "new-name")).toBe("Renamed to");
  });

  test("treats empty originalName as a rename (defensive)", () => {
    // The editor only calls this in edit mode (originalName comes
    // from row.name, which is never ""), but the function shouldn't
    // depend on that invariant for correctness.
    expect(pickEditSavedVerb("", "new-name")).toBe("Renamed to");
  });
});

describe("formatSavedToast", () => {
  test("returns message verbatim when orphanPath is null", () => {
    expect(formatSavedToast("Saved foo", null)).toBe("Saved foo");
  });

  test("appends orphan-path suffix when orphanPath is provided", () => {
    // Bug class this defeats: a future refactor that drops the
    // orphan-path forwarding (the A1 plumbing in performSaveEdit)
    // would result in a save that LOOKS clean but leaves a stale
    // file the user has no signal about.
    expect(formatSavedToast("Renamed to bar", ".kiro/agents/foo.json")).toBe(
      "Renamed to bar (note: stale file remains at .kiro/agents/foo.json)",
    );
  });

  test("preserves curly quotes in the message", () => {
    // The editor composes messages with U+201C / U+201D smart quotes
    // around the agent name (e.g. `Saved "foo"`). The helper must
    // not normalise them — the e2e regex matchers depend on them
    // staying through unchanged.
    expect(formatSavedToast("Saved “foo”", null)).toBe(
      "Saved “foo”",
    );
  });

  test("empty string orphanPath still renders a (degenerate) suffix", () => {
    // Pinned: only `null` is the "no orphan" signal. An empty
    // string would be unusual coming from the backend but if it
    // ever did, treat it as a present-but-empty path so the bug
    // surfaces visibly rather than being silently absorbed.
    expect(formatSavedToast("Saved x", "")).toBe(
      "Saved x (note: stale file remains at )",
    );
  });
});

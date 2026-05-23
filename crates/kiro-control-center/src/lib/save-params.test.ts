import { describe, expect, test } from "vitest";

import type { UserAgentRow } from "$lib/bindings";

import {
  buildSaveParams,
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

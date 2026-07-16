import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, test } from "vitest";

import type { SettingCategory } from "./bindings";

// The settings UI is fully data-driven: `+page.svelte` derives the category
// nav from the `category` field of each `SettingEntry`, and `SettingControl`
// renders by `value_type.kind`. That means the *only* thing the frontend
// needs from the backend for a new category to appear is the regenerated
// `bindings.ts`. These tests guard that contract so a stale-bindings regen
// (or a dropped category) fails fast in the FE suite rather than silently
// hiding a whole settings group.

const bindingsSource = readFileSync(
  resolve(dirname(fileURLToPath(import.meta.url)), "bindings.ts"),
  "utf8",
);

describe("SettingCategory bindings", () => {
  test("includes the voice category", () => {
    // Compile-time guard: if `voice` is dropped from the generated union,
    // this assignment stops type-checking and `npm run check` fails.
    const voice: SettingCategory = "voice";
    expect(voice).toBe("voice");

    // Runtime guard against a stale committed bindings.ts (the file the
    // backend's generate_types test writes). svelte-check reads types, not
    // the committed string, so this catches a forgotten regen.
    expect(bindingsSource).toContain('"voice"');
  });

  test("exposes every category the registry groups settings under", () => {
    const expected: SettingCategory[] = [
      "telemetry",
      "chat",
      "knowledge",
      "key_bindings",
      "features",
      "api",
      "mcp",
      "app",
      "tool_search",
      "voice",
      "environment",
    ];
    for (const category of expected) {
      expect(bindingsSource).toContain(`"${category}"`);
    }
  });
});

import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, test } from "vitest";

import {
  CATEGORY_ORDER,
  type Tool,
  type ToolCategory,
  TOOLS_CATALOG,
} from "./tools-catalog";

// The probe-s2 locked answer is the oracle for catalog-port fidelity.
// Reading it at test time (rather than hardcoding the same constants
// twice) makes the dependency on the probe explicit — a re-run of the
// probe regenerates probe.out, and this test re-validates against the
// fresh answer without any manual mirroring.
const probePath = resolve(
  dirname(fileURLToPath(import.meta.url)),
  "..",
  "..",
  "..",
  "..",
  ".agents-view",
  "probe-s2",
  "probe.out",
);

type ProbeAnswer = {
  tool_count: number;
  category_count: number;
  categories: string[];
  tools: { name: string; category: string; summary: string }[];
};

const probe: ProbeAnswer = JSON.parse(readFileSync(probePath, "utf8"));

describe("TOOLS_CATALOG matches probe-s2 locked answer", () => {
  // C1 stress fixture case 1.
  test("length equals probe.tool_count", () => {
    expect(TOOLS_CATALOG).toHaveLength(probe.tool_count);
  });

  // C1 stress fixture case 2. Sorted comparison because TOOLS_CATALOG
  // intentionally preserves source order while the probe sorts
  // categories alphabetically.
  test("category set equals probe.categories", () => {
    const cats = [...new Set(TOOLS_CATALOG.map((t) => t.category))].sort();
    expect(cats).toEqual(probe.categories);
  });

  // C1 stress fixture case 3. it.each driven by the probe — a future
  // regression that drops a single tool or drifts a summary string
  // fails one row in this matrix WITHOUT touching the length assertion
  // above. Both halves of the falsifier are independent.
  test.each(probe.tools)(
    "tool '$name' matches probe",
    (probeTool) => {
      const found = TOOLS_CATALOG.find((t) => t.name === probeTool.name);
      expect(found, `missing tool: ${probeTool.name}`).toBeDefined();
      expect(found?.category).toBe(probeTool.category);
      expect(found?.summary).toBe(probeTool.summary);
    },
  );

  // Inverse direction: every entry in TOOLS_CATALOG appears in the
  // probe. Catches a regression where an EXTRA tool sneaks in beyond
  // the source-of-truth (a "helpful" addition during port).
  test.each(TOOLS_CATALOG)(
    "catalog entry '$name' appears in probe",
    (entry: Tool) => {
      const probeMatch = probe.tools.find((t) => t.name === entry.name);
      expect(probeMatch, `unknown tool in catalog: ${entry.name}`).toBeDefined();
    },
  );
});

describe("TOOLS_CATALOG immutability", () => {
  // C1 stress fixture case 4 — Object.freeze invariant.
  test("Object.isFrozen returns true", () => {
    expect(Object.isFrozen(TOOLS_CATALOG)).toBe(true);
  });

  test("mutation attempts throw in strict mode", () => {
    // Vitest runs Node in strict mode; attempting to mutate a frozen
    // array throws TypeError rather than silently no-oping.
    // The `as unknown as Tool[]` double-cast bypasses TS's
    // readonly type-level guard so the *runtime* freeze gets exercised
    // — a regression that drops `Object.freeze` but keeps the
    // `readonly Tool[]` type would silently allow runtime mutation.
    const mutable = TOOLS_CATALOG as unknown as Tool[];
    expect(() => {
      mutable.push({
        name: "rogue",
        category: "Meta",
        summary: "Should not land",
      });
    }).toThrow(TypeError);
  });
});

describe("CATEGORY_ORDER", () => {
  // Anchored separately from TOOLS_CATALOG so the visual grid order
  // doesn't drift on catalog re-sorts. Pairs with the per-entry test
  // above to lock the contract: every TOOLS_CATALOG category MUST
  // appear in CATEGORY_ORDER, and vice versa — otherwise a category
  // would render with no tools (or tools with no category render).
  test("contains every category from TOOLS_CATALOG", () => {
    const cataloged = new Set<ToolCategory>(
      TOOLS_CATALOG.map((t) => t.category),
    );
    const ordered = new Set<ToolCategory>(CATEGORY_ORDER);
    expect(ordered).toEqual(cataloged);
  });

  test("has the same length as the unique-category count", () => {
    const unique = new Set<ToolCategory>(TOOLS_CATALOG.map((t) => t.category));
    expect(CATEGORY_ORDER).toHaveLength(unique.size);
  });
});

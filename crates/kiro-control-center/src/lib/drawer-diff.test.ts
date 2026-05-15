import { describe, expect, it } from "vitest";
import { deriveDiff, deriveSectionState, pluralize } from "./drawer-diff";

// SvelteSet ≡ Set for the read-only contract (`.has`, `.size`) — vitest
// doesn't need the reactive shim, so use plain Set for fixtures.

describe("deriveSectionState", () => {
  it("empty: zero items returns 'empty' regardless of selected size", () => {
    expect(deriveSectionState([], new Set())).toBe("empty");
    expect(deriveSectionState([], new Set(["x"]))).toBe("empty");
  });

  it("none: zero selected returns 'none'", () => {
    expect(deriveSectionState([{ name: "a" }, { name: "b" }], new Set())).toBe("none");
  });

  it("all: every item selected returns 'all'", () => {
    expect(
      deriveSectionState([{ name: "a" }, { name: "b" }], new Set(["a", "b"])),
    ).toBe("all");
  });

  it("partial: some selected returns 'partial'", () => {
    expect(deriveSectionState([{ name: "a" }, { name: "b" }], new Set(["a"]))).toBe(
      "partial",
    );
  });

  it("partial when selected size matches via different names (size-only signal)", () => {
    // deriveSectionState only inspects `.size`, NOT membership; a
    // selected set whose `.size` equals items.length classifies as
    // "all" even if the names diverge. Documenting this edge so a
    // future change that adds membership-based check has a test to
    // update.
    expect(deriveSectionState([{ name: "a" }], new Set(["unrelated"]))).toBe("all");
  });
});

describe("deriveDiff", () => {
  it("empty inputs: empty diff", () => {
    expect(deriveDiff([], new Set())).toEqual({ install: [], remove: [] });
  });

  it("install: selected but not installed", () => {
    const items = [
      { name: "a", installed: false },
      { name: "b", installed: false },
    ];
    expect(deriveDiff(items, new Set(["a"]))).toEqual({
      install: ["a"],
      remove: [],
    });
  });

  it("remove: installed but not selected", () => {
    const items = [
      { name: "a", installed: true },
      { name: "b", installed: true },
    ];
    expect(deriveDiff(items, new Set(["a"]))).toEqual({
      install: [],
      remove: ["b"],
    });
  });

  it("no-op cases: selected+installed → noop; unselected+!installed → noop", () => {
    const items = [
      { name: "kept", installed: true },
      { name: "absent", installed: false },
    ];
    expect(deriveDiff(items, new Set(["kept"]))).toEqual({
      install: [],
      remove: [],
    });
  });

  it("mixed install + remove + noop", () => {
    const items = [
      { name: "add", installed: false },
      { name: "drop", installed: true },
      { name: "kept", installed: true },
      { name: "skipped", installed: false },
    ];
    expect(deriveDiff(items, new Set(["add", "kept"]))).toEqual({
      install: ["add"],
      remove: ["drop"],
    });
  });

  it("preserves item order in install/remove arrays", () => {
    const items = [
      { name: "z", installed: false },
      { name: "a", installed: false },
      { name: "m", installed: false },
    ];
    expect(deriveDiff(items, new Set(["z", "a", "m"]))).toEqual({
      install: ["z", "a", "m"],
      remove: [],
    });
  });

  it("selected name not in items list is silently ignored", () => {
    // A drawer state that holds a name no longer in entry.items
    // (catalog refresh shrunk the list while the drawer was open)
    // shouldn't crash or produce a phantom install entry — there's
    // no item to push.
    const items = [{ name: "a", installed: false }];
    expect(deriveDiff(items, new Set(["a", "phantom"]))).toEqual({
      install: ["a"],
      remove: [],
    });
  });
});

describe("pluralize", () => {
  it("returns singular for n=1", () => {
    expect(pluralize(1, "skill", "skills")).toBe("skill");
  });

  it("returns plural for n=0", () => {
    expect(pluralize(0, "skill", "skills")).toBe("skills");
  });

  it("returns plural for n>1", () => {
    expect(pluralize(2, "skill", "skills")).toBe("skills");
    expect(pluralize(99, "skill", "skills")).toBe("skills");
  });

  it("works with multi-word nouns", () => {
    expect(pluralize(1, "steering file", "steering files")).toBe("steering file");
    expect(pluralize(3, "steering file", "steering files")).toBe("steering files");
  });
});

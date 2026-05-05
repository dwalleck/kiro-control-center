import { describe, expect, it } from "vitest";
import type { PluginUpdateFailureKind } from "$lib/bindings";
import { remediationClass, kindLabel } from "./plugin-updates";

describe("remediationClass", () => {
  it("maps marketplace_unavailable to stale_cache", () => {
    const kind: PluginUpdateFailureKind = { kind: "marketplace_unavailable" };
    expect(remediationClass(kind)).toBe("stale_cache");
  });

  it("maps manifest_unreadable to stale_cache", () => {
    const kind: PluginUpdateFailureKind = { kind: "manifest_unreadable" };
    expect(remediationClass(kind)).toBe("stale_cache");
  });

  it("maps hash_failed to stale_cache", () => {
    const kind: PluginUpdateFailureKind = { kind: "hash_failed" };
    expect(remediationClass(kind)).toBe("stale_cache");
  });

  it("maps manifest_invalid to its own class", () => {
    const kind: PluginUpdateFailureKind = { kind: "manifest_invalid" };
    expect(remediationClass(kind)).toBe("manifest_invalid");
  });

  it("maps other to unknown", () => {
    const kind: PluginUpdateFailureKind = { kind: "other" };
    expect(remediationClass(kind)).toBe("unknown");
  });
});

describe("kindLabel", () => {
  it("returns a non-empty string for every PluginUpdateFailureKind variant", () => {
    const variants: PluginUpdateFailureKind[] = [
      { kind: "marketplace_unavailable" },
      { kind: "manifest_unreadable" },
      { kind: "manifest_invalid" },
      { kind: "hash_failed" },
      { kind: "other" },
    ];
    for (const v of variants) {
      const label = kindLabel(v);
      expect(label.length).toBeGreaterThan(0);
    }
  });

  it("produces distinct labels for distinct kinds", () => {
    const labels = new Set([
      kindLabel({ kind: "marketplace_unavailable" }),
      kindLabel({ kind: "manifest_unreadable" }),
      kindLabel({ kind: "manifest_invalid" }),
      kindLabel({ kind: "hash_failed" }),
      kindLabel({ kind: "other" }),
    ]);
    expect(labels.size).toBe(5);
  });
});

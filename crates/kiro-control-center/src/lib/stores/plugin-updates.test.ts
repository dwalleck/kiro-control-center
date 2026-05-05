import { describe, expect, it } from "vitest";
import type {
  MarketplaceName,
  PluginName,
  PluginUpdateFailure,
  PluginUpdateFailureKind,
} from "$lib/bindings";
import { remediationClass, kindLabel, groupFailures } from "./plugin-updates";
import type { FailureGroup } from "./plugin-updates";

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

function failure(
  marketplace: string,
  plugin: string,
  kind: PluginUpdateFailureKind,
): PluginUpdateFailure {
  return {
    marketplace: marketplace as MarketplaceName,
    plugin: plugin as PluginName,
    kind,
    reason: "test reason",
  };
}

describe("groupFailures", () => {
  it("returns empty array for empty input", () => {
    expect(groupFailures([])).toEqual([]);
  });

  it("collapses N stale-cache failures from one marketplace into one group", () => {
    const groups = groupFailures([
      failure("acme", "p1", { kind: "marketplace_unavailable" }),
      failure("acme", "p2", { kind: "manifest_unreadable" }),
      failure("acme", "p3", { kind: "hash_failed" }),
    ]);
    expect(groups).toHaveLength(1);
    expect(groups[0].remediation).toBe("stale_cache");
    expect(groups[0].marketplace).toBe("acme");
    expect(groups[0].plugins).toEqual(["p1", "p2", "p3"]);
  });

  it("produces separate groups for two marketplaces", () => {
    const groups = groupFailures([
      failure("acme", "p1", { kind: "marketplace_unavailable" }),
      failure("beta", "p2", { kind: "marketplace_unavailable" }),
    ]);
    expect(groups).toHaveLength(2);
    const marketplaces = new Set(groups.map((g) => g.marketplace));
    expect(marketplaces).toEqual(new Set(["acme", "beta"]));
  });

  it("produces separate groups for distinct remediation classes from same marketplace", () => {
    const groups = groupFailures([
      failure("acme", "p1", { kind: "marketplace_unavailable" }),
      failure("acme", "p2", { kind: "manifest_invalid" }),
    ]);
    expect(groups).toHaveLength(2);
    const remediations = new Set(groups.map((g) => g.remediation));
    expect(remediations).toEqual(new Set(["stale_cache", "manifest_invalid"]));
  });

  it("preserves plugin order within a group as the input was ordered", () => {
    const groups = groupFailures([
      failure("acme", "z-plugin", { kind: "marketplace_unavailable" }),
      failure("acme", "a-plugin", { kind: "marketplace_unavailable" }),
      failure("acme", "m-plugin", { kind: "marketplace_unavailable" }),
    ]);
    expect(groups[0].plugins).toEqual(["z-plugin", "a-plugin", "m-plugin"]);
  });

  it("each group carries a non-empty remediationHint", () => {
    const groups: FailureGroup[] = groupFailures([
      failure("acme", "p1", { kind: "marketplace_unavailable" }),
      failure("acme", "p2", { kind: "manifest_invalid" }),
      failure("acme", "p3", { kind: "other" }),
    ]);
    for (const g of groups) {
      expect(g.remediationHint.length).toBeGreaterThan(0);
    }
  });
});

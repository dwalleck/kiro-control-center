import { describe, expect, it } from "vitest";
import type {
  MarketplaceName,
  PluginName,
  PluginUpdateFailure,
  PluginUpdateFailureKind,
  PluginUpdateInfo,
} from "$lib/bindings";
import {
  actionUpdateLabel,
  groupFailures,
  kindLabel,
  projectUpdateCheckBanners,
  remediationClass,
  statusUpdateLabel,
} from "./plugin-updates";
import { updateCheckErrKey } from "../error-source";

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

  it("manifest_unreadable label covers read failure modes beyond 'missing'", () => {
    expect(kindLabel({ kind: "manifest_unreadable" })).toBe(
      "plugin.json couldn't be read from marketplace cache",
    );
  });
});

function update(
  partial: Partial<PluginUpdateInfo> & { change_signal: PluginUpdateInfo["change_signal"] },
): PluginUpdateInfo {
  return {
    marketplace: "acme" as MarketplaceName,
    plugin: "p" as PluginName,
    installed_version: null,
    available_version: null,
    ...partial,
  };
}

describe("statusUpdateLabel", () => {
  it("content_changed renders as a status sentence", () => {
    expect(
      statusUpdateLabel(update({ change_signal: { kind: "content_changed" } })),
    ).toBe("Content changed since install");
  });

  it("version_bumped with both versions known renders as 'vX → vY'", () => {
    expect(
      statusUpdateLabel(
        update({
          change_signal: { kind: "version_bumped" },
          installed_version: "1.0",
          available_version: "1.1",
        }),
      ),
    ).toBe("v1.0 → v1.1");
  });

  it("version_bumped with legacy install (installed_version null) renders as 'vN available'", () => {
    expect(
      statusUpdateLabel(
        update({
          change_signal: { kind: "version_bumped" },
          available_version: "1.1",
        }),
      ),
    ).toBe("v1.1 available");
  });

  it("version_bumped with neither version declared falls back to 'Update available'", () => {
    expect(
      statusUpdateLabel(update({ change_signal: { kind: "version_bumped" } })),
    ).toBe("Update available");
  });
});

describe("actionUpdateLabel", () => {
  it("content_changed renders as 'Update (content changed)'", () => {
    expect(
      actionUpdateLabel(update({ change_signal: { kind: "content_changed" } })),
    ).toBe("Update (content changed)");
  });

  it("version_bumped with available_version renders as 'Update → vN'", () => {
    expect(
      actionUpdateLabel(
        update({
          change_signal: { kind: "version_bumped" },
          installed_version: "1.0",
          available_version: "1.1",
        }),
      ),
    ).toBe("Update → v1.1");
  });

  it("version_bumped legacy install (installed_version null) still renders as 'Update → vN'", () => {
    expect(
      actionUpdateLabel(
        update({
          change_signal: { kind: "version_bumped" },
          available_version: "1.1",
        }),
      ),
    ).toBe("Update → v1.1");
  });

  it("version_bumped with no available_version falls back to 'Update'", () => {
    expect(
      actionUpdateLabel(update({ change_signal: { kind: "version_bumped" } })),
    ).toBe("Update");
  });
});

describe("status vs action label contract", () => {
  it("the column phrases state, the button phrases an action — they must differ", () => {
    const u = update({
      change_signal: { kind: "version_bumped" },
      installed_version: "1.0",
      available_version: "1.1",
    });
    expect(statusUpdateLabel(u)).toBe("v1.0 → v1.1");
    expect(actionUpdateLabel(u)).toBe("Update → v1.1");
    expect(statusUpdateLabel(u)).not.toBe(actionUpdateLabel(u));
  });

  it("content_changed: column reads as full sentence, button reads as parenthetical action", () => {
    const u = update({ change_signal: { kind: "content_changed" } });
    expect(statusUpdateLabel(u)).toBe("Content changed since install");
    expect(actionUpdateLabel(u)).toBe("Update (content changed)");
    expect(statusUpdateLabel(u)).not.toBe(actionUpdateLabel(u));
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

  it("returns groups in first-seen key order", () => {
    const groups = groupFailures([
      failure("beta", "p1", { kind: "marketplace_unavailable" }),
      failure("acme", "p2", { kind: "marketplace_unavailable" }),
    ]);
    expect(groups[0].marketplace).toBe("beta");
    expect(groups[1].marketplace).toBe("acme");
  });

  it("does not collide on marketplace names containing ':'", () => {
    // `validate_name` permits ':' in marketplace names; the group key
    // must use a separator that can never appear in either side.
    const groups = groupFailures([
      failure("time:zones", "p1", { kind: "marketplace_unavailable" }),
      failure("time", "p2", { kind: "marketplace_unavailable" }),
    ]);
    expect(groups).toHaveLength(2);
    const marketplaces = new Set(groups.map((g) => g.marketplace));
    expect(marketplaces).toEqual(new Set(["time:zones", "time"]));
  });

  it("each group carries a non-empty remediationHint", () => {
    const groups = groupFailures([
      failure("acme", "p1", { kind: "marketplace_unavailable" }),
      failure("acme", "p2", { kind: "manifest_invalid" }),
      failure("acme", "p3", { kind: "other" }),
    ]);
    for (const g of groups) {
      expect(g.remediationHint.length).toBeGreaterThan(0);
    }
  });
});

describe("projectUpdateCheckBanners", () => {
  it("returns empty upserts and no staleKeys for empty failures", () => {
    const { upserts, staleKeys } = projectUpdateCheckBanners([], []);
    expect(upserts.size).toBe(0);
    expect(staleKeys).toEqual([]);
  });

  it("produces one upsert per failure group", () => {
    const { upserts, staleKeys } = projectUpdateCheckBanners(
      [
        failure("acme", "p1", { kind: "marketplace_unavailable" }),
        failure("acme", "p2", { kind: "manifest_unreadable" }),
      ],
      [],
    );
    expect(upserts.size).toBe(1);
    expect(staleKeys).toEqual([]);
    expect([...upserts.keys()][0]).toBe(updateCheckErrKey("stale_cache", "acme"));
  });

  it("returns staleKeys for existing update-check keys not in the groups", () => {
    const { staleKeys } = projectUpdateCheckBanners(
      [],
      [updateCheckErrKey("stale_cache", "acme"), updateCheckErrKey("manifest_invalid", "beta")],
    );
    expect(staleKeys).toEqual([
      updateCheckErrKey("stale_cache", "acme"),
      updateCheckErrKey("manifest_invalid", "beta"),
    ]);
  });

  it("does NOT flag non-update-check keys as stale", () => {
    const { staleKeys } = projectUpdateCheckBanners([], ["some-other-key", "installed-plugins"]);
    expect(staleKeys).toEqual([]);
  });

  it("excludes keys that match a live group", () => {
    const existing = [
      updateCheckErrKey("stale_cache", "acme"),
      updateCheckErrKey("stale_cache", "beta"),
    ];
    const { staleKeys } = projectUpdateCheckBanners(
      [failure("acme", "p1", { kind: "marketplace_unavailable" })],
      existing,
    );
    expect(staleKeys).toEqual([updateCheckErrKey("stale_cache", "beta")]);
  });

  it("upsert messages include plugin count and list", () => {
    const { upserts } = projectUpdateCheckBanners(
      [
        failure("acme", "p1", { kind: "marketplace_unavailable" }),
        failure("acme", "p2", { kind: "marketplace_unavailable" }),
      ],
      [],
    );
    const msg = [...upserts.values()][0];
    expect(msg).toContain("2 plugins");
    expect(msg).toContain("(p1, p2)");
  });

  it("singular noun: 1 plugin reads '1 plugin' not '1 plugins'", () => {
    const { upserts } = projectUpdateCheckBanners(
      [
        failure("acme", "p1", { kind: "marketplace_unavailable" }),
      ],
      [],
    );
    const msg = [...upserts.values()][0];
    expect(msg).toContain("1 plugin");
    expect(msg).not.toContain("1 plugins");
    expect(msg).toContain("(p1)");
  });
});

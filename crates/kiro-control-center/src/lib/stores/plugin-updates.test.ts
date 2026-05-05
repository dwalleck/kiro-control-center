import { describe, expect, it } from "vitest";
import type { PluginUpdateFailureKind } from "$lib/bindings";
import { remediationClass } from "./plugin-updates";

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

import { beforeEach, describe, expect, it, vi } from "vitest";
import type {
  ErrorType,
  InstallPluginResult_Serialize,
  MarketplaceName,
  PluginName,
  RemovePluginResult,
} from "$lib/bindings";

import {
  runPluginInstall,
  runPluginRemove,
  type PluginActionContext,
  type PluginRemoveContext,
} from "./plugin-actions";

function emptyInstallResult(): InstallPluginResult_Serialize {
  return {
    marketplace: "acme" as MarketplaceName,
    plugin: "p" as PluginName,
    version: null,
    skills: { installed: [], skipped: [], failed: [], skipped_skills: [] },
    steering: { installed: [], failed: [], warnings: [] },
    agents: {
      installed: [],
      skipped: [],
      failed: [],
      warnings: [],
      installed_native: [],
      installed_companions: null,
    },
  };
}

function emptyRemoveResult(): RemovePluginResult {
  return {
    skills: { removed: [], failures: [] },
    steering: { removed: [], failures: [] },
    agents: { removed: [], failures: [] },
  };
}

function makeInstallCtx(
  overrides: Partial<PluginActionContext> = {},
): PluginActionContext {
  return {
    marketplace: "acme",
    plugin: "demo-plugin",
    projectPath: "/test/project",
    acceptMcp: false,
    refresh: () => Promise.resolve(),
    storeRefresh: () => Promise.resolve(),
    installPlugin: vi.fn(),
    ...overrides,
  };
}

function makeRemoveCtx(
  overrides: Partial<PluginRemoveContext> = {},
): PluginRemoveContext {
  return {
    marketplace: "acme",
    plugin: "demo-plugin",
    projectPath: "/test/project",
    refresh: () => Promise.resolve(),
    storeRefresh: () => Promise.resolve(),
    removePlugin: vi.fn(),
    ...overrides,
  };
}

beforeEach(() => {
  vi.clearAllMocks();
});

describe("runPluginInstall", () => {
  it("install success: kind=ok, banner.message populated, no errors, force=forceInstall", async () => {
    const r = emptyInstallResult();
    r.skills.installed = ["a", "b"];
    const installPlugin: PluginActionContext["installPlugin"] = vi
      .fn()
      .mockResolvedValue({ status: "ok", data: r });
    const storeRefresh = vi.fn().mockResolvedValue(undefined);

    const outcome = await runPluginInstall(
      makeInstallCtx({ installPlugin, storeRefresh }),
      { kind: "install", force: false },
    );

    expect(outcome.kind).toBe("ok");
    if (outcome.kind === "ok") {
      expect(outcome.banner.primary).toEqual({
        kind: "message",
        text: "Plugin demo-plugin: 2 skills",
      });
      expect(outcome.banner.warning).toBeNull();
      expect(outcome.banner.staleRefresh).toBeNull();
    }
    expect(installPlugin).toHaveBeenCalledWith(
      "acme",
      "demo-plugin",
      false,
      false,
      "/test/project",
    );
    expect(storeRefresh).toHaveBeenCalledOnce();
  });

  it("update mode: force=true (no force field on the update arm)", async () => {
    const r = emptyInstallResult();
    r.skills.installed = ["a"];
    const installPlugin: PluginActionContext["installPlugin"] = vi
      .fn()
      .mockResolvedValue({ status: "ok", data: r });

    await runPluginInstall(
      makeInstallCtx({ installPlugin }),
      { kind: "update" },
    );

    expect(installPlugin).toHaveBeenCalledWith(
      "acme",
      "demo-plugin",
      true,
      false,
      "/test/project",
    );
  });

  it("install mode: mode.force=true propagates to installPlugin", async () => {
    const r = emptyInstallResult();
    r.skills.installed = ["a"];
    const installPlugin: PluginActionContext["installPlugin"] = vi
      .fn()
      .mockResolvedValue({ status: "ok", data: r });

    await runPluginInstall(
      makeInstallCtx({ installPlugin }),
      { kind: "install", force: true },
    );

    expect(installPlugin).toHaveBeenCalledWith(
      "acme",
      "demo-plugin",
      true,
      false,
      "/test/project",
    );
  });

  it("anyFailed && !anyInstalled: banner.error populated", async () => {
    const r = emptyInstallResult();
    r.skills.failed = [
      { name: "broken", error: "oops", kind: { kind: "install_failed" } },
    ];
    const installPlugin: PluginActionContext["installPlugin"] = vi
      .fn()
      .mockResolvedValue({ status: "ok", data: r });

    const outcome = await runPluginInstall(
      makeInstallCtx({ installPlugin }),
      { kind: "install", force: false },
    );

    expect(outcome.kind).toBe("ok");
    if (outcome.kind === "ok") {
      expect(outcome.banner.primary).toEqual({
        kind: "error",
        text: "Plugin install failed for demo-plugin: 1 skill failed",
      });
      expect(outcome.banner.warning).toBeNull();
    }
  });

  it("partial success: some installed + some failed → banner.message, banner.error null", async () => {
    const r = emptyInstallResult();
    r.skills.installed = ["a"];
    r.skills.failed = [
      { name: "broken", error: "oops", kind: { kind: "install_failed" } },
    ];
    const installPlugin: PluginActionContext["installPlugin"] = vi
      .fn()
      .mockResolvedValue({ status: "ok", data: r });

    const outcome = await runPluginInstall(
      makeInstallCtx({ installPlugin }),
      { kind: "install", force: false },
    );

    expect(outcome.kind).toBe("ok");
    if (outcome.kind === "ok") {
      expect(outcome.banner.primary).toEqual({
        kind: "message",
        text: "Plugin demo-plugin: 1 skill · 1 skill failed",
      });
      expect(outcome.banner.warning).toBeNull();
    }
  });

  it("warnings: banner.warning populated alongside success message", async () => {
    const r = emptyInstallResult();
    r.skills.installed = ["a"];
    r.agents.warnings = [
      {
        kind: "mcp_servers_require_opt_in",
        agent: "scary",
        transports: ["stdio"],
      },
    ];
    const installPlugin: PluginActionContext["installPlugin"] = vi
      .fn()
      .mockResolvedValue({ status: "ok", data: r });

    const outcome = await runPluginInstall(
      makeInstallCtx({ installPlugin }),
      { kind: "install", force: false },
    );

    expect(outcome.kind).toBe("ok");
    if (outcome.kind === "ok") {
      expect(outcome.banner.primary).toEqual({
        kind: "message",
        text: "Plugin demo-plugin: 1 skill",
      });
      expect(outcome.banner.warning).toBe(
        "Plugin demo-plugin: agent 'scary' declares MCP servers [stdio] — re-run with --accept-mcp to install",
      );
    }
  });

  it("Tauri command returns error: kind=fail", async () => {
    const installPlugin: PluginActionContext["installPlugin"] = vi
      .fn()
      .mockResolvedValue({
        status: "error",
        error: { message: "network unreachable", error_type: "internal" as ErrorType },
      });

    const outcome = await runPluginInstall(
      makeInstallCtx({ installPlugin }),
      { kind: "install", force: false },
    );

    expect(outcome.kind).toBe("fail");
    if (outcome.kind === "fail") {
      expect(outcome.error).toBe(
        "Plugin install failed for demo-plugin: network unreachable",
      );
    }
  });

  it("Tauri command returns undefined message: falls back to 'Unknown error'", async () => {
    const installPlugin: PluginActionContext["installPlugin"] = vi
      .fn()
      .mockResolvedValue({
        status: "error",
        error: { message: undefined, error_type: "internal" as ErrorType },
      });

    const outcome = await runPluginInstall(
      makeInstallCtx({ installPlugin }),
      { kind: "install", force: false },
    );

    expect(outcome.kind).toBe("fail");
    if (outcome.kind === "fail") {
      expect(outcome.error).toBe(
        "Plugin install failed for demo-plugin: Unknown error",
      );
    }
  });

  it("Tauri command throws: kind=fail with error message from throw", async () => {
    const installPlugin: PluginActionContext["installPlugin"] = vi
      .fn()
      .mockRejectedValue(new Error("connection reset"));

    const outcome = await runPluginInstall(
      makeInstallCtx({ installPlugin }),
      { kind: "install", force: false },
    );

    expect(outcome.kind).toBe("fail");
    if (outcome.kind === "fail") {
      expect(outcome.error).toBe(
        "Plugin install failed for demo-plugin: connection reset",
      );
    }
  });

  it("update mode fail prefix uses 'Update failed'", async () => {
    const installPlugin: PluginActionContext["installPlugin"] = vi
      .fn()
      .mockResolvedValue({
        status: "error",
        error: { message: "disk full", error_type: "io_error" as ErrorType },
      });

    const outcome = await runPluginInstall(
      makeInstallCtx({ installPlugin }),
      { kind: "update" },
    );

    expect(outcome.kind).toBe("fail");
    if (outcome.kind === "fail") {
      expect(outcome.error).toBe("Update failed for demo-plugin: disk full");
    }
  });

  it("update mode throw uses 'Update failed' prefix", async () => {
    const installPlugin: PluginActionContext["installPlugin"] = vi
      .fn()
      .mockRejectedValue(new Error("crash"));

    const outcome = await runPluginInstall(
      makeInstallCtx({ installPlugin }),
      { kind: "update" },
    );

    expect(outcome.kind).toBe("fail");
    if (outcome.kind === "fail") {
      expect(outcome.error).toBe("Update failed for demo-plugin: crash");
    }
  });

  it("post-action store refresh throws: staleRefresh names store subsystem, tab refresh still runs", async () => {
    const r = emptyInstallResult();
    r.skills.installed = ["a"];
    const installPlugin: PluginActionContext["installPlugin"] = vi
      .fn()
      .mockResolvedValue({ status: "ok", data: r });
    const storeRefresh = vi.fn().mockRejectedValue(new Error("store boom"));

    let tabRan = false;
    const outcome = await runPluginInstall(
      makeInstallCtx({
        installPlugin,
        storeRefresh,
        refresh: async () => {
          tabRan = true;
        },
      }),
      { kind: "install", force: false },
    );

    expect(tabRan).toBe(true);
    expect(outcome.kind).toBe("ok");
    if (outcome.kind === "ok") {
      expect(outcome.banner.primary).toEqual({
        kind: "message",
        text: "Plugin demo-plugin: 1 skill",
      });
      expect(outcome.banner.staleRefresh).toBe(
        "Plugin-update state is stale after install: store boom",
      );
    }
  });

  it("post-action tab refresh throws: staleRefresh names list subsystem, message preserved", async () => {
    const r = emptyInstallResult();
    r.skills.installed = ["a"];
    const installPlugin: PluginActionContext["installPlugin"] = vi
      .fn()
      .mockResolvedValue({ status: "ok", data: r });

    const outcome = await runPluginInstall(
      makeInstallCtx({
        installPlugin,
        refresh: async () => {
          throw new Error("tab boom");
        },
      }),
      { kind: "install", force: false },
    );

    expect(outcome.kind).toBe("ok");
    if (outcome.kind === "ok") {
      expect(outcome.banner.primary).toEqual({
        kind: "message",
        text: "Plugin demo-plugin: 1 skill",
      });
      expect(outcome.banner.staleRefresh).toBe(
        "Installed-plugins list is stale after install: tab boom",
      );
    }
  });

  it("cascade ordering: storeRefresh called before ctx.refresh", async () => {
    const r = emptyInstallResult();
    r.skills.installed = ["a"];
    const installPlugin: PluginActionContext["installPlugin"] = vi
      .fn()
      .mockResolvedValue({ status: "ok", data: r });

    const calls: string[] = [];
    const storeRefresh = vi.fn().mockImplementation(async () => {
      calls.push("store-refresh");
    });

    await runPluginInstall(
      makeInstallCtx({
        installPlugin,
        storeRefresh,
        refresh: async () => {
          calls.push("tab-refresh");
        },
      }),
      { kind: "install", force: false },
    );

    expect(calls).toEqual(["store-refresh", "tab-refresh"]);
  });

  it("Tauri error: refresh is NOT called (store + tab)", async () => {
    const installPlugin: PluginActionContext["installPlugin"] = vi
      .fn()
      .mockResolvedValue({
        status: "error",
        error: { message: "network unreachable", error_type: "internal" as ErrorType },
      });
    const storeRefresh = vi.fn().mockResolvedValue(undefined);

    await runPluginInstall(
      makeInstallCtx({ installPlugin, storeRefresh }),
      { kind: "install", force: false },
    );

    expect(storeRefresh).not.toHaveBeenCalled();
  });

  it("Tauri throw: refresh is NOT called (store + tab)", async () => {
    const installPlugin: PluginActionContext["installPlugin"] = vi
      .fn()
      .mockRejectedValue(new Error("connection reset"));
    const storeRefresh = vi.fn().mockResolvedValue(undefined);

    await runPluginInstall(
      makeInstallCtx({ installPlugin, storeRefresh }),
      { kind: "install", force: false },
    );

    expect(storeRefresh).not.toHaveBeenCalled();
  });

  it("acceptMcp: true propagates to Tauri command", async () => {
    const r = emptyInstallResult();
    r.skills.installed = ["a"];
    const installPlugin: PluginActionContext["installPlugin"] = vi
      .fn()
      .mockResolvedValue({ status: "ok", data: r });

    await runPluginInstall(
      makeInstallCtx({ installPlugin, acceptMcp: true }),
      { kind: "install", force: false },
    );

    expect(installPlugin).toHaveBeenCalledWith(
      "acme",
      "demo-plugin",
      false,
      true,
      "/test/project",
    );
  });
});

describe("runPluginRemove", () => {
  it("remove success: kind=ok-removed, banner.message, removeResult present", async () => {
    const r = emptyRemoveResult();
    r.skills.removed = ["a", "b"];
    const removePlugin: PluginRemoveContext["removePlugin"] = vi
      .fn()
      .mockResolvedValue({ status: "ok", data: r });

    const outcome = await runPluginRemove(makeRemoveCtx({ removePlugin }));

    expect(outcome.kind).toBe("ok-removed");
    if (outcome.kind === "ok-removed") {
      expect(outcome.banner.primary).toEqual({
        kind: "message",
        text: "Removed plugin demo-plugin: 2 skills",
      });
      expect(outcome.removeResult).toBe(r);
    }
  });

  it("remove with partial failures: kind=ok-removed, banner.warning", async () => {
    const r = emptyRemoveResult();
    r.skills.removed = ["a"];
    r.steering.failures = [
      { item: "broken.md", error: "permission denied" },
    ];
    const removePlugin: PluginRemoveContext["removePlugin"] = vi
      .fn()
      .mockResolvedValue({ status: "ok", data: r });

    const outcome = await runPluginRemove(makeRemoveCtx({ removePlugin }));

    expect(outcome.kind).toBe("ok-removed");
    if (outcome.kind === "ok-removed") {
      expect(outcome.banner.primary).toBeNull();
      expect(outcome.banner.warning).toBe(
        "Removed plugin demo-plugin: 1 skill · 1 steering failed",
      );
    }
  });

  it("hasFailures && !hasItems: primary error 'Remove failed', no misleading 'Removed plugin' prefix, removeResult populated", async () => {
    // Total-fail: every removal attempt failed, nothing was actually removed.
    // Banner must be a red error ("Remove failed for X: ...") not an amber
    // "Removed plugin X: ..." (which would lie). removeResult stays populated
    // so the per-failure details panel can render — useful context for the
    // user about which items failed and why.
    const r = emptyRemoveResult();
    r.skills.failures = [
      { item: "a", error: "permission denied" },
      { item: "b", error: "locked file" },
    ];
    const removePlugin: PluginRemoveContext["removePlugin"] = vi
      .fn()
      .mockResolvedValue({ status: "ok", data: r });

    const outcome = await runPluginRemove(makeRemoveCtx({ removePlugin }));

    expect(outcome.kind).toBe("ok-removed");
    if (outcome.kind === "ok-removed") {
      expect(outcome.banner.primary).toEqual({
        kind: "error",
        text: "Remove failed for demo-plugin: 2 skills failed",
      });
      expect(outcome.banner.warning).toBeNull();
      expect(outcome.removeResult).toBe(r);
    }
  });

  it("empty remove: kind=ok-noop, banner shows 'nothing to remove'", async () => {
    const r = emptyRemoveResult();
    const removePlugin: PluginRemoveContext["removePlugin"] = vi
      .fn()
      .mockResolvedValue({ status: "ok", data: r });

    const outcome = await runPluginRemove(makeRemoveCtx({ removePlugin }));

    expect(outcome.kind).toBe("ok-noop");
    if (outcome.kind === "ok-noop") {
      expect(outcome.banner.primary).toEqual({
        kind: "message",
        text: "Removed plugin demo-plugin: nothing to remove",
      });
    }
  });

  it("Tauri command returns error: kind=fail", async () => {
    const removePlugin: PluginRemoveContext["removePlugin"] = vi
      .fn()
      .mockResolvedValue({
        status: "error",
        error: { message: "project not found", error_type: "not_found" as ErrorType },
      });

    const outcome = await runPluginRemove(makeRemoveCtx({ removePlugin }));

    expect(outcome.kind).toBe("fail");
    if (outcome.kind === "fail") {
      expect(outcome.error).toBe(
        "Remove failed for demo-plugin: project not found",
      );
    }
  });

  it("Tauri command throws: kind=fail with error message", async () => {
    const removePlugin: PluginRemoveContext["removePlugin"] = vi
      .fn()
      .mockRejectedValue(new Error("disk full"));

    const outcome = await runPluginRemove(makeRemoveCtx({ removePlugin }));

    expect(outcome.kind).toBe("fail");
    if (outcome.kind === "fail") {
      expect(outcome.error).toBe("Remove failed for demo-plugin: disk full");
    }
  });

  it("post-action refresh throws: staleRefresh joins both subsystem messages, removeResult still correct", async () => {
    const r = emptyRemoveResult();
    r.skills.removed = ["a"];
    const removePlugin: PluginRemoveContext["removePlugin"] = vi
      .fn()
      .mockResolvedValue({ status: "ok", data: r });
    const storeRefresh = vi.fn().mockRejectedValue(new Error("store boom"));

    const outcome = await runPluginRemove(
      makeRemoveCtx({
        removePlugin,
        storeRefresh,
        refresh: async () => {
          throw new Error("tab boom");
        },
      }),
    );

    expect(outcome.kind).toBe("ok-removed");
    if (outcome.kind === "ok-removed") {
      expect(outcome.banner.primary).toEqual({
        kind: "message",
        text: "Removed plugin demo-plugin: 1 skill",
      });
      expect(outcome.banner.staleRefresh).toBe(
        "Plugin-update state is stale after remove: store boom — Installed-plugins list is stale after remove: tab boom",
      );
      expect(outcome.removeResult).toBe(r);
    }
  });

  it("post-action store refresh throws: staleRefresh names store subsystem, tab refresh still runs", async () => {
    const r = emptyRemoveResult();
    r.skills.removed = ["a"];
    const removePlugin: PluginRemoveContext["removePlugin"] = vi
      .fn()
      .mockResolvedValue({ status: "ok", data: r });
    const storeRefresh = vi.fn().mockRejectedValue(new Error("store boom"));

    let tabRan = false;
    const outcome = await runPluginRemove(
      makeRemoveCtx({
        removePlugin,
        storeRefresh,
        refresh: async () => {
          tabRan = true;
        },
      }),
    );

    expect(tabRan).toBe(true);
    expect(outcome.kind).toBe("ok-removed");
    if (outcome.kind === "ok-removed") {
      expect(outcome.banner.staleRefresh).toBe(
        "Plugin-update state is stale after remove: store boom",
      );
    }
  });

  it("post-action tab refresh throws: staleRefresh names list subsystem", async () => {
    const r = emptyRemoveResult();
    r.skills.removed = ["a"];
    const removePlugin: PluginRemoveContext["removePlugin"] = vi
      .fn()
      .mockResolvedValue({ status: "ok", data: r });

    const outcome = await runPluginRemove(
      makeRemoveCtx({
        removePlugin,
        refresh: async () => {
          throw new Error("tab boom");
        },
      }),
    );

    expect(outcome.kind).toBe("ok-removed");
    if (outcome.kind === "ok-removed") {
      expect(outcome.banner.staleRefresh).toBe(
        "Installed-plugins list is stale after remove: tab boom",
      );
    }
  });

  it("cascade ordering: storeRefresh called before ctx.refresh", async () => {
    const r = emptyRemoveResult();
    r.skills.removed = ["a"];
    const removePlugin: PluginRemoveContext["removePlugin"] = vi
      .fn()
      .mockResolvedValue({ status: "ok", data: r });

    const calls: string[] = [];
    const storeRefresh = vi.fn().mockImplementation(async () => {
      calls.push("store-refresh");
    });

    await runPluginRemove(
      makeRemoveCtx({
        removePlugin,
        storeRefresh,
        refresh: async () => {
          calls.push("tab-refresh");
        },
      }),
    );

    expect(calls).toEqual(["store-refresh", "tab-refresh"]);
  });

  it("Tauri error: refresh is NOT called (store + tab)", async () => {
    const removePlugin: PluginRemoveContext["removePlugin"] = vi
      .fn()
      .mockResolvedValue({
        status: "error",
        error: { message: "project not found", error_type: "not_found" as ErrorType },
      });
    const storeRefresh = vi.fn().mockResolvedValue(undefined);

    await runPluginRemove(makeRemoveCtx({ removePlugin, storeRefresh }));

    expect(storeRefresh).not.toHaveBeenCalled();
  });

  it("Tauri throw: refresh is NOT called (store + tab)", async () => {
    const removePlugin: PluginRemoveContext["removePlugin"] = vi
      .fn()
      .mockRejectedValue(new Error("disk full"));
    const storeRefresh = vi.fn().mockResolvedValue(undefined);

    await runPluginRemove(makeRemoveCtx({ removePlugin, storeRefresh }));

    expect(storeRefresh).not.toHaveBeenCalled();
  });
});

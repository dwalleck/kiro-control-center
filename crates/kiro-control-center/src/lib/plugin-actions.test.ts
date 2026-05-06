import { beforeEach, describe, expect, it, vi } from "vitest";
import type {
  InstallPluginResult_Serialize,
  MarketplaceName,
  PluginName,
  RemovePluginResult,
  ErrorType,
} from "$lib/bindings";

const { mockRefresh, mockInstallPlugin, mockRemovePlugin } = vi.hoisted(() => ({
  mockRefresh: vi.fn<(projectPath: string) => Promise<void>>().mockResolvedValue(
    undefined,
  ),
  mockInstallPlugin: vi.fn(),
  mockRemovePlugin: vi.fn(),
}));

vi.mock("$lib/stores/plugin-updates.svelte", () => ({
  pluginUpdates: {
    refresh: mockRefresh,
  },
}));

vi.mock("$lib/bindings", () => ({
  commands: {
    installPlugin: (...args: unknown[]) => mockInstallPlugin(...args),
    removePlugin: (...args: unknown[]) => mockRemovePlugin(...args),
  },
}));

import { runPluginInstall, runPluginRemove } from "./plugin-actions";

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

const defaultCtx = {
  marketplace: "acme",
  plugin: "demo-plugin",
  projectPath: "/test/project",
  forceInstall: false,
  acceptMcp: false,
  refresh: () => Promise.resolve(),
};

beforeEach(() => {
  vi.clearAllMocks();
  mockRefresh.mockResolvedValue(undefined);
});

describe("runPluginInstall", () => {
  it("install success: kind=ok, banner.message populated, no errors, force=forceInstall", async () => {
    const r = emptyInstallResult();
    r.skills.installed = ["a", "b"];
    mockInstallPlugin.mockResolvedValue({ status: "ok", data: r });

    const outcome = await runPluginInstall(defaultCtx, "install");

    expect(outcome.kind).toBe("ok");
    if (outcome.kind === "ok") {
      expect(outcome.banner.primary).toEqual({
        kind: "message",
        text: "Plugin demo-plugin: 2 skills",
      });
      expect(outcome.banner.warning).toBeNull();
      expect(outcome.banner.staleRefresh).toBeNull();
    }
    expect(mockInstallPlugin).toHaveBeenCalledWith(
      "acme",
      "demo-plugin",
      false,
      false,
      "/test/project",
    );
    expect(mockRefresh).toHaveBeenCalledOnce();
  });

  it("update mode: force=true regardless of forceInstall setting", async () => {
    const r = emptyInstallResult();
    r.skills.installed = ["a"];
    mockInstallPlugin.mockResolvedValue({ status: "ok", data: r });

    const ctx = { ...defaultCtx, forceInstall: false };
    await runPluginInstall(ctx, "update");

    expect(mockInstallPlugin).toHaveBeenCalledWith(
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
    mockInstallPlugin.mockResolvedValue({ status: "ok", data: r });

    const outcome = await runPluginInstall(defaultCtx, "install");

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
    mockInstallPlugin.mockResolvedValue({ status: "ok", data: r });

    const outcome = await runPluginInstall(defaultCtx, "install");

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
    mockInstallPlugin.mockResolvedValue({ status: "ok", data: r });

    const outcome = await runPluginInstall(defaultCtx, "install");

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
    mockInstallPlugin.mockResolvedValue({
      status: "error",
      error: { message: "network unreachable", error_type: "internal" as ErrorType },
    });

    const outcome = await runPluginInstall(defaultCtx, "install");

    expect(outcome.kind).toBe("fail");
    if (outcome.kind === "fail") {
      expect(outcome.error).toBe(
        "Plugin install failed for demo-plugin: network unreachable",
      );
    }
  });

  it("Tauri command returns undefined message: falls back to 'Unknown error'", async () => {
    mockInstallPlugin.mockResolvedValue({
      status: "error",
      error: { message: undefined, error_type: "internal" as ErrorType },
    });

    const outcome = await runPluginInstall(defaultCtx, "install");

    expect(outcome.kind).toBe("fail");
    if (outcome.kind === "fail") {
      expect(outcome.error).toBe(
        "Plugin install failed for demo-plugin: Unknown error",
      );
    }
  });

  it("Tauri command throws: kind=fail with error message from throw", async () => {
    mockInstallPlugin.mockRejectedValue(new Error("connection reset"));

    const outcome = await runPluginInstall(defaultCtx, "install");

    expect(outcome.kind).toBe("fail");
    if (outcome.kind === "fail") {
      expect(outcome.error).toBe(
        "Plugin install failed for demo-plugin: connection reset",
      );
    }
  });

  it("update mode fail prefix uses 'Update failed'", async () => {
    mockInstallPlugin.mockResolvedValue({
      status: "error",
      error: { message: "disk full", error_type: "io_error" as ErrorType },
    });

    const outcome = await runPluginInstall(defaultCtx, "update");

    expect(outcome.kind).toBe("fail");
    if (outcome.kind === "fail") {
      expect(outcome.error).toBe("Update failed for demo-plugin: disk full");
    }
  });

  it("update mode throw uses 'Update failed' prefix", async () => {
    mockInstallPlugin.mockRejectedValue(new Error("crash"));

    const outcome = await runPluginInstall(defaultCtx, "update");

    expect(outcome.kind).toBe("fail");
    if (outcome.kind === "fail") {
      expect(outcome.error).toBe("Update failed for demo-plugin: crash");
    }
  });

  it("post-action store refresh throws: staleRefresh names store subsystem, tab refresh still runs", async () => {
    const r = emptyInstallResult();
    r.skills.installed = ["a"];
    mockInstallPlugin.mockResolvedValue({ status: "ok", data: r });

    let tabRan = false;
    mockRefresh.mockRejectedValue(new Error("store boom"));
    const ctx = {
      ...defaultCtx,
      refresh: async () => {
        tabRan = true;
      },
    };

    const outcome = await runPluginInstall(ctx, "install");

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
    mockInstallPlugin.mockResolvedValue({ status: "ok", data: r });

    mockRefresh.mockResolvedValue(undefined);
    const ctx = {
      ...defaultCtx,
      refresh: async () => {
        throw new Error("tab boom");
      },
    };

    const outcome = await runPluginInstall(ctx, "install");

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

  it("cascade ordering: pluginUpdates.refresh called before ctx.refresh", async () => {
    const r = emptyInstallResult();
    r.skills.installed = ["a"];
    mockInstallPlugin.mockResolvedValue({ status: "ok", data: r });

    const calls: string[] = [];
    mockRefresh.mockImplementation(async () => {
      calls.push("store-refresh");
    });
    const ctx = {
      ...defaultCtx,
      refresh: async () => {
        calls.push("tab-refresh");
      },
    };

    await runPluginInstall(ctx, "install");

    expect(calls).toEqual(["store-refresh", "tab-refresh"]);
  });

  it("Tauri error: refresh is NOT called (store + tab)", async () => {
    mockInstallPlugin.mockResolvedValue({
      status: "error",
      error: { message: "network unreachable", error_type: "internal" as ErrorType },
    });

    await runPluginInstall(defaultCtx, "install");

    expect(mockRefresh).not.toHaveBeenCalled();
  });

  it("Tauri throw: refresh is NOT called (store + tab)", async () => {
    mockInstallPlugin.mockRejectedValue(new Error("connection reset"));

    await runPluginInstall(defaultCtx, "install");

    expect(mockRefresh).not.toHaveBeenCalled();
  });

  it("acceptMcp: true propagates to Tauri command", async () => {
    const r = emptyInstallResult();
    r.skills.installed = ["a"];
    mockInstallPlugin.mockResolvedValue({ status: "ok", data: r });

    await runPluginInstall({ ...defaultCtx, acceptMcp: true }, "install");

    expect(mockInstallPlugin).toHaveBeenCalledWith(
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
    mockRemovePlugin.mockResolvedValue({ status: "ok", data: r });

    const outcome = await runPluginRemove({
      marketplace: "acme",
      plugin: "demo-plugin",
      projectPath: "/test/project",
      refresh: () => Promise.resolve(),
    });

    expect(outcome.kind).toBe("ok-removed");
    if (outcome.kind === "ok-removed") {
      expect(outcome.banner.primary).toEqual({
        kind: "message",
        text: "Removed plugin demo-plugin: 2 skills",
      });
      expect(outcome.removeResult).toBe(r);
    }
  });

  it("remove with failures: kind=ok-removed, banner.warning", async () => {
    const r = emptyRemoveResult();
    r.skills.removed = ["a"];
    r.steering.failures = [
      { item: "broken.md", error: "permission denied" },
    ];
    mockRemovePlugin.mockResolvedValue({ status: "ok", data: r });

    const outcome = await runPluginRemove({
      marketplace: "acme",
      plugin: "demo-plugin",
      projectPath: "/test/project",
      refresh: () => Promise.resolve(),
    });

    expect(outcome.kind).toBe("ok-removed");
    if (outcome.kind === "ok-removed") {
      expect(outcome.banner.primary).toBeNull();
      expect(outcome.banner.warning).toBe(
        "Removed plugin demo-plugin: 1 skill · 1 steering failed",
      );
    }
  });

  it("empty remove: kind=ok-noop, banner shows 'nothing to remove'", async () => {
    const r = emptyRemoveResult();
    mockRemovePlugin.mockResolvedValue({ status: "ok", data: r });

    const outcome = await runPluginRemove({
      marketplace: "acme",
      plugin: "demo-plugin",
      projectPath: "/test/project",
      refresh: () => Promise.resolve(),
    });

    expect(outcome.kind).toBe("ok-noop");
    if (outcome.kind === "ok-noop") {
      expect(outcome.banner.primary).toEqual({
        kind: "message",
        text: "Removed plugin demo-plugin: nothing to remove",
      });
    }
  });

  it("Tauri command returns error: kind=fail", async () => {
    mockRemovePlugin.mockResolvedValue({
      status: "error",
      error: { message: "project not found", error_type: "not_found" as ErrorType },
    });

    const outcome = await runPluginRemove({
      marketplace: "acme",
      plugin: "demo-plugin",
      projectPath: "/test/project",
      refresh: () => Promise.resolve(),
    });

    expect(outcome.kind).toBe("fail");
    if (outcome.kind === "fail") {
      expect(outcome.error).toBe(
        "Remove failed for demo-plugin: project not found",
      );
    }
  });

  it("Tauri command throws: kind=fail with error message", async () => {
    mockRemovePlugin.mockRejectedValue(new Error("disk full"));

    const outcome = await runPluginRemove({
      marketplace: "acme",
      plugin: "demo-plugin",
      projectPath: "/test/project",
      refresh: () => Promise.resolve(),
    });

    expect(outcome.kind).toBe("fail");
    if (outcome.kind === "fail") {
      expect(outcome.error).toBe("Remove failed for demo-plugin: disk full");
    }
  });

  it("post-action refresh throws: staleRefresh joins both subsystem messages, removeResult still correct", async () => {
    const r = emptyRemoveResult();
    r.skills.removed = ["a"];
    mockRemovePlugin.mockResolvedValue({ status: "ok", data: r });

    mockRefresh.mockRejectedValue(new Error("store boom"));
    const ctx = {
      marketplace: "acme",
      plugin: "demo-plugin",
      projectPath: "/test/project",
      refresh: async () => {
        throw new Error("tab boom");
      },
    };

    const outcome = await runPluginRemove(ctx);

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
    mockRemovePlugin.mockResolvedValue({ status: "ok", data: r });

    let tabRan = false;
    mockRefresh.mockRejectedValue(new Error("store boom"));
    const ctx = {
      marketplace: "acme",
      plugin: "demo-plugin",
      projectPath: "/test/project",
      refresh: async () => {
        tabRan = true;
      },
    };

    const outcome = await runPluginRemove(ctx);

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
    mockRemovePlugin.mockResolvedValue({ status: "ok", data: r });

    mockRefresh.mockResolvedValue(undefined);
    const ctx = {
      marketplace: "acme",
      plugin: "demo-plugin",
      projectPath: "/test/project",
      refresh: async () => {
        throw new Error("tab boom");
      },
    };

    const outcome = await runPluginRemove(ctx);

    expect(outcome.kind).toBe("ok-removed");
    if (outcome.kind === "ok-removed") {
      expect(outcome.banner.staleRefresh).toBe(
        "Installed-plugins list is stale after remove: tab boom",
      );
    }
  });

  it("cascade ordering: pluginUpdates.refresh called before ctx.refresh", async () => {
    const r = emptyRemoveResult();
    r.skills.removed = ["a"];
    mockRemovePlugin.mockResolvedValue({ status: "ok", data: r });

    const calls: string[] = [];
    mockRefresh.mockImplementation(async () => {
      calls.push("store-refresh");
    });
    const ctx = {
      marketplace: "acme",
      plugin: "demo-plugin",
      projectPath: "/test/project",
      refresh: async () => {
        calls.push("tab-refresh");
      },
    };

    await runPluginRemove(ctx);

    expect(calls).toEqual(["store-refresh", "tab-refresh"]);
  });

  it("Tauri error: refresh is NOT called (store + tab)", async () => {
    mockRemovePlugin.mockResolvedValue({
      status: "error",
      error: { message: "project not found", error_type: "not_found" as ErrorType },
    });

    await runPluginRemove({
      marketplace: "acme",
      plugin: "demo-plugin",
      projectPath: "/test/project",
      refresh: () => Promise.resolve(),
    });

    expect(mockRefresh).not.toHaveBeenCalled();
  });

  it("Tauri throw: refresh is NOT called (store + tab)", async () => {
    mockRemovePlugin.mockRejectedValue(new Error("disk full"));

    await runPluginRemove({
      marketplace: "acme",
      plugin: "demo-plugin",
      projectPath: "/test/project",
      refresh: () => Promise.resolve(),
    });

    expect(mockRefresh).not.toHaveBeenCalled();
  });
});

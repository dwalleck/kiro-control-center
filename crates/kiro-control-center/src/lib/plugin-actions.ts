import type {
  CommandError,
  InstallPluginResult_Serialize,
  RemovePluginResult,
} from "$lib/bindings";
import {
  formatCommandError,
  formatInstallPluginResult,
  formatRemovePluginResult,
} from "$lib/format";

// Mirrors the shape produced by `typedError<T, CommandError>` in bindings.ts.
// Re-declared here so this module stays pure-logic — no runtime import of
// `commands` or any Tauri IPC machinery; callers inject the IPC functions
// via context (CLAUDE.md "no Tauri-IPC mocks" — tests construct fakes
// directly without `vi.mock`).
type IpcResult<T> =
  | { status: "ok"; data: T }
  | { status: "error"; error: CommandError };

export type InstallPluginFn = (
  marketplace: string,
  plugin: string,
  force: boolean,
  acceptMcp: boolean,
  projectPath: string,
) => Promise<IpcResult<InstallPluginResult_Serialize>>;

export type RemovePluginFn = (
  marketplace: string,
  plugin: string,
  projectPath: string,
) => Promise<IpcResult<RemovePluginResult>>;

// Discriminated mode: `force` lives on the install arm because update mode
// always implies force (see the switch in runPluginInstall below). Before
// this refactor, `mode` was a bare string and `forceInstall` lived on the
// context — letting a caller pass `mode: "update"` together with a
// meaningless `forceInstall: false` that the body silently overrode.
// Encoding the per-arm field at the type level removes that dead state and
// lets the body discriminate on `mode.kind` with a compile-time
// exhaustiveness check.
export type PluginActionMode =
  | { kind: "install"; force: boolean }
  | { kind: "update" };

export type PluginActionContext = {
  marketplace: string;
  plugin: string;
  projectPath: string;
  // Security-critical: agents declaring mcp_servers are refused unless
  // acceptMcp is true. With acceptMcp = false the agent produces
  // InstallWarning::McpServersRequireOptIn and never lands in installed or
  // failed — the warning names the agent and its transports so the user
  // can re-run with the opt-in. Unlike `mode.force` (install-only), this
  // applies to both install and update modes — the FFI surface gates MCP
  // unconditionally (verified at crates/kiro-control-center/src-tauri/src/commands/plugins.rs).
  acceptMcp: boolean;
  refresh: () => Promise<void>;
  // Injected dependencies — see IpcResult comment above.
  installPlugin: InstallPluginFn;
  storeRefresh: (projectPath: string) => Promise<void>;
};

export type PluginRemoveContext = {
  marketplace: string;
  plugin: string;
  projectPath: string;
  refresh: () => Promise<void>;
  // Injected dependencies — see IpcResult comment above.
  removePlugin: RemovePluginFn;
  storeRefresh: (projectPath: string) => Promise<void>;
};

export type PluginBanner = {
  primary: { kind: "error"; text: string } | { kind: "message"; text: string } | null;
  warning: string | null;
  // Joined post-action refresh failures. The message text distinguishes
  // which subsystem is stale (store vs. installed-list); independent
  // dismiss per-subsystem isn't a UX requirement, so a single channel.
  staleRefresh: string | null;
};

export type PluginActionOutcome =
  | { kind: "ok"; banner: PluginBanner; installResult: InstallPluginResult_Serialize }
  | { kind: "fail"; error: string };

export type PluginRemoveOutcome =
  | { kind: "ok-removed"; banner: PluginBanner; removeResult: RemovePluginResult }
  | { kind: "ok-noop"; banner: PluginBanner }
  | { kind: "fail"; error: string };

export async function runPluginInstall(
  ctx: PluginActionContext,
  mode: PluginActionMode,
): Promise<PluginActionOutcome> {
  // Single switch makes a future `PluginActionMode` arm a compile error here
  // rather than silently falling through. Matches the project pattern used
  // in `format.ts` (formatSkippedSkill, formatSteeringWarning, etc.).
  let force: boolean;
  let failPrefix: string;
  let successPrefix: string;
  switch (mode.kind) {
    case "install":
      force = mode.force;
      failPrefix = `Plugin install failed for ${ctx.plugin}`;
      successPrefix = `Plugin ${ctx.plugin}`;
      break;
    case "update":
      force = true;
      failPrefix = `Update failed for ${ctx.plugin}`;
      successPrefix = `Updated ${ctx.plugin}`;
      break;
    default: {
      const _exhaustive: never = mode;
      throw new Error(
        `unhandled PluginActionMode: ${JSON.stringify(_exhaustive)}`,
      );
    }
  }

  try {
    const result = await ctx.installPlugin(
      ctx.marketplace,
      ctx.plugin,
      force,
      ctx.acceptMcp,
      ctx.projectPath,
    );
    if (result.status === "ok") {
      const { summary, warnings, anyInstalled, anyFailed } =
        formatInstallPluginResult(result.data);

      let primary: PluginBanner["primary"] = null;
      let warning: string | null = null;
      const staleParts: string[] = [];

      if (anyFailed && !anyInstalled) {
        primary = { kind: "error", text: `${failPrefix}: ${summary}` };
      } else {
        primary = { kind: "message", text: `${successPrefix}: ${summary}` };
      }
      if (warnings) {
        warning = `${successPrefix}: ${warnings}`;
      }

      // Plugin-updates store must refresh BEFORE the caller's local refresh
      // so updateFor / failureFor are based on fresh state when the caller
      // re-reads the project's installed list. If `ctx.refresh()` ran first,
      // the template would pair stale (pre-refresh) store data with the new
      // installed-plugins list, briefly showing "update available" badges
      // for plugins that were just installed.
      try {
        await ctx.storeRefresh(ctx.projectPath);
      } catch (e) {
        console.error(
          `[plugin-actions] pluginUpdates.refresh threw after ${mode.kind}`,
          e,
        );
        const reason = e instanceof Error ? e.message : String(e);
        staleParts.push(`Plugin-update state is stale after ${mode.kind}: ${reason}`);
      }

      try {
        await ctx.refresh();
      } catch (e) {
        console.error(
          `[plugin-actions] tab refresh threw after ${mode.kind}`,
          e,
        );
        const reason = e instanceof Error ? e.message : String(e);
        staleParts.push(`Installed-plugins list is stale after ${mode.kind}: ${reason}`);
      }

      const staleRefresh = staleParts.length > 0 ? staleParts.join(" — ") : null;
      return {
        kind: "ok",
        banner: { primary, warning, staleRefresh },
        installResult: result.data,
      };
    }
    return {
      kind: "fail",
      error: `${failPrefix}: ${formatCommandError(result.error)}`,
    };
  } catch (e) {
    const reason = e instanceof Error ? e.message : String(e);
    return { kind: "fail", error: `${failPrefix}: ${reason}` };
  }
}

export async function runPluginRemove(
  ctx: PluginRemoveContext,
): Promise<PluginRemoveOutcome> {
  try {
    const result = await ctx.removePlugin(
      ctx.marketplace,
      ctx.plugin,
      ctx.projectPath,
    );
    if (result.status === "ok") {
      const { summary, hasItems, hasFailures } =
        formatRemovePluginResult(result.data);

      let primary: PluginBanner["primary"] = null;
      let warning: string | null = null;
      const staleParts: string[] = [];

      // Always emit a banner — even "nothing to remove" gives the user
      // feedback that the action completed instead of feeling inert.
      // Three cases: total-fail (everything failed, nothing removed) →
      // red error banner because "Removed plugin X" would lie. Partial-fail
      // (some items removed, some failed) → amber warning. Clean success →
      // green message. The total-fail branch keeps `kind: "ok-removed"` so
      // the per-failure details panel still renders, giving the user
      // actionable context about which items failed and why.
      if (hasFailures && !hasItems) {
        primary = {
          kind: "error",
          text: `Remove failed for ${ctx.plugin}: ${summary}`,
        };
      } else if (hasFailures) {
        warning = `Removed plugin ${ctx.plugin}: ${summary}`;
      } else {
        primary = { kind: "message", text: `Removed plugin ${ctx.plugin}: ${summary}` };
      }

      try {
        await ctx.storeRefresh(ctx.projectPath);
      } catch (e) {
        console.error(
          "[plugin-actions] pluginUpdates.refresh threw after remove",
          e,
        );
        const reason = e instanceof Error ? e.message : String(e);
        staleParts.push(`Plugin-update state is stale after remove: ${reason}`);
      }

      try {
        await ctx.refresh();
      } catch (e) {
        console.error(
          "[plugin-actions] tab refresh threw after remove",
          e,
        );
        const reason = e instanceof Error ? e.message : String(e);
        staleParts.push(`Installed-plugins list is stale after remove: ${reason}`);
      }

      const banner: PluginBanner = {
        primary,
        warning,
        staleRefresh: staleParts.length > 0 ? staleParts.join(" — ") : null,
      };

      if (hasItems || hasFailures) {
        return {
          kind: "ok-removed",
          banner,
          removeResult: result.data,
        };
      }
      return { kind: "ok-noop", banner };
    }
    return {
      kind: "fail",
      error: `Remove failed for ${ctx.plugin}: ${result.error.message ?? "Unknown error"}`,
    };
  } catch (e) {
    const reason = e instanceof Error ? e.message : String(e);
    return {
      kind: "fail",
      error: `Remove failed for ${ctx.plugin}: ${reason}`,
    };
  }
}

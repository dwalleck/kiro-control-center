import { commands } from "$lib/bindings";
import type { RemovePluginResult } from "$lib/bindings";
import { formatInstallPluginResult, formatRemovePluginResult } from "$lib/format";
import { pluginUpdates } from "$lib/stores/plugin-updates.svelte";

export type PluginActionMode = "install" | "update";

export type PluginActionContext = {
  marketplace: string;
  plugin: string;
  projectPath: string;
  forceInstall: boolean;
  // Security-critical: agents declaring mcp_servers are refused unless
  // acceptMcp is true. With acceptMcp = false the agent produces
  // InstallWarning::McpServersRequireOptIn and never lands in installed or
  // failed — the warning names the agent and its transports so the user
  // can re-run with the opt-in.
  acceptMcp: boolean;
  refresh: () => Promise<void>;
};

export type PluginRemoveContext = {
  marketplace: string;
  plugin: string;
  projectPath: string;
  refresh: () => Promise<void>;
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
  | { kind: "ok"; banner: PluginBanner }
  | { kind: "fail"; error: string };

export type PluginRemoveOutcome =
  | { kind: "ok-removed"; banner: PluginBanner; removeResult: RemovePluginResult }
  | { kind: "ok-noop"; banner: PluginBanner }
  | { kind: "fail"; error: string };

export async function runPluginInstall(
  ctx: PluginActionContext,
  mode: PluginActionMode,
): Promise<PluginActionOutcome> {
  const force = mode === "update" ? true : ctx.forceInstall;
  const failPrefix =
    mode === "update"
      ? `Update failed for ${ctx.plugin}`
      : `Plugin install failed for ${ctx.plugin}`;
  const successPrefix =
    mode === "update" ? `Updated ${ctx.plugin}` : `Plugin ${ctx.plugin}`;

  try {
    const result = await commands.installPlugin(
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
        await pluginUpdates.refresh(ctx.projectPath);
      } catch (e) {
        console.error(
          `[plugin-actions] pluginUpdates.refresh threw after ${mode}`,
          e,
        );
        const reason = e instanceof Error ? e.message : String(e);
        staleParts.push(`Plugin-update state is stale after ${mode}: ${reason}`);
      }

      try {
        await ctx.refresh();
      } catch (e) {
        console.error(
          `[plugin-actions] tab refresh threw after ${mode}`,
          e,
        );
        const reason = e instanceof Error ? e.message : String(e);
        staleParts.push(`Installed-plugins list is stale after ${mode}: ${reason}`);
      }

      const staleRefresh = staleParts.length > 0 ? staleParts.join(" — ") : null;
      return {
        kind: "ok",
        banner: { primary, warning, staleRefresh },
      };
    }
    return {
      kind: "fail",
      error: `${failPrefix}: ${result.error.message ?? "Unknown error"}`,
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
    const result = await commands.removePlugin(
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
      if (hasFailures) {
        warning = `Removed plugin ${ctx.plugin}: ${summary}`;
      } else {
        primary = { kind: "message", text: `Removed plugin ${ctx.plugin}: ${summary}` };
      }

      try {
        await pluginUpdates.refresh(ctx.projectPath);
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

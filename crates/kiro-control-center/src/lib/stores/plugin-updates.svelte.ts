import { commands } from "$lib/bindings";
import type {
  DetectUpdatesResult,
  PluginUpdateFailure,
  PluginUpdateInfo,
} from "$lib/bindings";
import { groupFailures } from "./plugin-updates";

class PluginUpdatesStore {
  result = $state<DetectUpdatesResult | null>(null);
  loading = $state(false);
  fetchError = $state<string | null>(null);
  lastProjectPath = $state<string | null>(null);

  // Monotonic generation. Path equality alone lets A→B→A overwrite a
  // still-resolving newer A (same path, different generations).
  #latestRequestId = 0;

  failureGroups = $derived(
    this.result?.failures ? groupFailures(this.result.failures) : [],
  );

  updateFor(marketplace: string, plugin: string): PluginUpdateInfo | undefined {
    return this.result?.updates?.find(
      (u) => u.marketplace === marketplace && u.plugin === plugin,
    );
  }

  failureFor(marketplace: string, plugin: string): PluginUpdateFailure | undefined {
    return this.result?.failures?.find(
      (f) => f.marketplace === marketplace && f.plugin === plugin,
    );
  }

  async refresh(projectPath: string): Promise<void> {
    if (!projectPath) {
      this.result = null;
      this.fetchError = null;
      this.lastProjectPath = null;
      this.loading = false;
      return;
    }
    const reqId = ++this.#latestRequestId;
    this.loading = true;
    this.lastProjectPath = projectPath;
    try {
      const r = await commands.detectPluginUpdates(projectPath);
      if (this.#latestRequestId !== reqId) {
        console.warn("[pluginUpdates] discarding stale refresh result", { projectPath });
        return;
      }
      if (r.status === "ok") {
        this.result = r.data;
        this.fetchError = null;
      } else {
        this.result = null;
        this.fetchError = r.error.message;
      }
    } catch (e) {
      if (this.#latestRequestId !== reqId) {
        console.warn("[pluginUpdates] discarding stale refresh rejection", { projectPath, e });
        return;
      }
      this.result = null;
      this.fetchError = e instanceof Error ? e.message : String(e);
    } finally {
      if (this.#latestRequestId === reqId) this.loading = false;
    }
  }
}

export const pluginUpdates = new PluginUpdatesStore();

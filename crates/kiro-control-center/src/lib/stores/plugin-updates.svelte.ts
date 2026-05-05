import { commands } from "$lib/bindings";
import type {
  DetectUpdatesResult,
  PluginUpdateFailure,
  PluginUpdateInfo,
} from "$lib/bindings";
import { groupFailures, type FailureGroup } from "./plugin-updates";

/**
 *  Module-scoped reactive store wrapping `detectPluginUpdates`.
 *  Consumed by `BrowseTab` and `InstalledTab` — both `$effect` on
 *  `projectPath` and call `pluginUpdates.refresh(projectPath)`. The
 *  parent `+page.svelte` also wires `MarketplacesTab.onUpdated` to
 *  this store's `refresh` so a successful `kiro-market update`
 *  invalidates the cached scan.
 *
 *  Per Phase 2b design decision #2, the only re-fire triggers are:
 *  (1) projectPath change (each tab's existing $effect),
 *  (2) marketplace update (MarketplacesTab callback).
 *  No background polling, no manual rescan button.
 */
class PluginUpdatesStore {
  result = $state<DetectUpdatesResult | null>(null);
  loading = $state(false);
  // Toplevel error from `detectPluginUpdates` Result::Err — used when
  // the command itself failed (couldn't read tracking files at all).
  // Per-plugin failures live on `result.failures`, not here.
  fetchError = $state<string | null>(null);
  // Last project path the store refreshed against. Lets the consumer
  // tabs distinguish "not yet refreshed" from "refreshed and empty".
  lastProjectPath = $state<string | null>(null);

  failureGroups = $derived.by((): FailureGroup[] =>
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
    this.loading = true;
    this.lastProjectPath = projectPath;
    try {
      const r = await commands.detectPluginUpdates(projectPath);
      // Race guard: a newer refresh may have been started while we awaited.
      // Tauri commands don't support cancellation, so we compare paths and
      // discard our result if a newer call has already taken over.
      if (this.lastProjectPath !== projectPath) return;
      if (r.status === "ok") {
        this.result = r.data;
        this.fetchError = null;
      } else {
        this.result = null;
        this.fetchError = r.error.message;
      }
    } catch (e) {
      if (this.lastProjectPath !== projectPath) return;
      this.result = null;
      this.fetchError = e instanceof Error ? e.message : String(e);
    } finally {
      // Only clear loading if we're still the active call; a superseding
      // call has its own `loading = true` and will manage its own clear.
      if (this.lastProjectPath === projectPath) this.loading = false;
    }
  }
}

export const pluginUpdates = new PluginUpdatesStore();

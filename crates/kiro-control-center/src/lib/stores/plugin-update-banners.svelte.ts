import { untrack } from "svelte";
import { ERR_UPDATE_FETCH } from "$lib/error-source";
import type { UpdateCheckKey } from "$lib/error-source";
import { pluginUpdates } from "./plugin-updates.svelte";
import { projectUpdateCheckBanners } from "./plugin-updates";

/**
 * Reactive helper that projects plugin-update store state into a caller-owned
 * `fetchErrors` SvelteMap. Must be called once at component-init time (inside
 * the component's reactive scope) — it registers three `$effect` blocks under
 * the hood.
 *
 * The helper only ever writes two families of keys:
 *   - `UpdateCheckKey` (failure-group banners)
 *   - `typeof ERR_UPDATE_FETCH` (toplevel fetch error)
 * and removes stale `UpdateCheckKey` entries on each re-computation.
 *
 * The `fetchErrors` map may have a wider key type than what this helper writes,
 * but its `set`/`delete` methods must accept at least `UpdateCheckKey |
 * typeof ERR_UPDATE_FETCH` — structural typing (method-parameter
 * contravariance) means `SvelteMap<ErrorSource, string>` satisfies this
 * interface even when `ErrorSource` is a wider union.
 */
export function usePluginUpdateBanners(args: {
  projectPath: () => string;
  fetchErrors: {
    set(key: UpdateCheckKey | typeof ERR_UPDATE_FETCH, value: string): void;
    delete(key: UpdateCheckKey | typeof ERR_UPDATE_FETCH): void;
    keys(): Iterable<string>;
  };
  logPrefix: string;
}): void {
  $effect(() => {
    const p = args.projectPath();
    pluginUpdates
      .refresh(p)
      .catch((e) =>
        console.error(
          `[${args.logPrefix}] pluginUpdates.refresh threw`,
          e,
        ),
      );
  });

  $effect(() => {
    // Snapshot keys via `untrack` so this effect doesn't re-fire on its own
    // writes. `SvelteMap.keys()` is reactive in Svelte 5; without untrack,
    // the set/delete calls below would mutate the keyset the effect just
    // depended on, triggering re-runs that rely on Svelte's idempotent-write
    // optimization to terminate. The real signal that should drive banner
    // updates is `pluginUpdates.failureGroups` (read above).
    const existingKeys = untrack(() => Array.from(args.fetchErrors.keys()));
    const { upserts, staleKeys } = projectUpdateCheckBanners(
      pluginUpdates.failureGroups,
      existingKeys,
    );
    for (const [key, msg] of upserts) {
      args.fetchErrors.set(key, msg);
    }
    for (const k of staleKeys) {
      args.fetchErrors.delete(k);
    }
  });

  $effect(() => {
    if (pluginUpdates.fetchError) {
      args.fetchErrors.set(
        ERR_UPDATE_FETCH,
        `Couldn't check for updates: ${pluginUpdates.fetchError}`,
      );
    } else {
      args.fetchErrors.delete(ERR_UPDATE_FETCH);
    }
  });
}

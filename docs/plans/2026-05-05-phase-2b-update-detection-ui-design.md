# Phase 2b — Plugin Update Detection UI — Design

> **Status:** design draft. Implementation plan to be written next via the `superpowers:writing-plans` skill once this design is approved. Phase 2b ships as a frontend-only PR consuming the wire format defined in Phase 2a (`2026-04-30-phase-2a-update-detection-design.md` / PR #96).

## Problem

Phase 2a (PR #96) shipped the backend for plugin update detection: `detect_plugin_updates` Tauri command, `DetectUpdatesResult` / `PluginUpdateInfo` / `PluginUpdateFailure` / `PluginUpdateFailureKind` / `UpdateChangeSignal` types, and the per-content-type reshape of `RemovePluginResult`. `bindings.ts` regenerated and the Tauri crate compiles, but **no Svelte component consumes any of it yet**. Three concrete UI surfaces are missing:

1. **Update indicator on plugin cards.** A user looking at the Installed tab — or, more importantly, the Browse tab while shopping for new plugins — has no signal that one of their installed plugins has an update available. They re-install manually to find out, which is friction the Phase 1 design (`2026-04-29-plugin-first-install-design.md` lines 27-31, 185-233) explicitly called out as the "Phase 2" gap.
2. **Update button.** The action exists in the backend (`installPlugin(..., force=true, ...)`) but no UI calls it yet.
3. **Remove toast still shows opaque counts.** The `RemovePluginResult` reshape gave us per-content-type `removed: Vec<String>` and `failures: Vec<RemoveItemFailure>`, but `InstalledTab.svelte`'s current `removePlugin` swallows the success result entirely and only surfaces failures via `loadError`. The reshape's stated motivation ("user knows the magnitude but not which items") is unfulfilled.

Phase 2b lands all three surfaces in a single PR, plus narrowly-scoped vitest setup so the new pure-logic helpers get unit coverage.

## Approach

**Where indicators live: both BrowseTab and InstalledTab.** The PluginCard's "Installed" pill becomes an "Update → v1.1" button when an update is available; the InstalledTab table gains a Status column and an Update button next to Remove. Maximum discoverability — a user shopping in BrowseTab sees that an installed plugin needs an update without switching tabs.

**Scan trigger: eager on project mount + after marketplace fetch.** A new module-scoped Svelte store (`plugin-updates.svelte.ts`) calls `detect_plugin_updates(projectPath)` once whenever `projectPath` changes; a callback from `MarketplacesTab` invalidates the cached result after a successful marketplace update (the only time a new scan is materially different from the last one without project switching). The 2a design's "<100ms for realistic projects" estimate keeps this affordable; eager-on-mount avoids any flicker between "Installed" and "Update" pills on first paint.

**Update click: fire-and-forget, no confirmation modal, tooltip warning.** The Update button calls the existing `installPlugin(marketplace, plugin, force=true, acceptMcp=false, projectPath)` — the same code path Install uses, with `force` hard-coded to `true`. Mirrors the existing Install button's UX (also fire-and-forget). A `title="Update will replace local edits to plugin files"` tooltip is the only friction. Modals can be added later if a real data-loss incident motivates them; pulling them out is harder.

**Remove toast itemization: counts inline + collapsed `<details>`.** Native HTML `<details>`, closed-by-default on success (the user doesn't need to read 8 skill names when nothing went wrong), `<details open>` on failure (auto-expand so the user immediately sees what didn't get removed). Reuses the existing 3-banner pattern (`installError` red / `installMessage` green / `installWarning` amber); InstalledTab gains the same banner channels BrowseTab has today, plus the new `<details>` rendering pattern.

**Failure rendering: group by `(remediationClass, marketplace)`, shared "Update check failed" pill with kind in tooltip.** A pure function `remediationClass(kind: PluginUpdateFailureKind): "stale_cache" | "manifest_invalid" | "unknown"` collapses the 5-variant `PluginUpdateFailureKind` into 3 remediation paths; failures group by `(remediationClass, marketplace)` so 10 stale-cache failures from one marketplace produce one banner ("10 plugins from acme-marketplace couldn't be checked. Run `kiro-market update`"), not ten. Per-row pills share a single "Update check failed" treatment with the kind-specific copy in the `title=` tooltip — preserves the typed taxonomy on hover without amplifying it visually.

**Edge states.**
- *Scan in progress:* show the existing "Installed" pill optimistically (per Q6/A(ii)). Sub-100ms scan time + most-installed-plugins-have-no-update means flicker is barely perceptible. A skeleton placeholder on every card adds visual noise on every project switch for negligible signal gain.
- *`change_signal: ContentChanged`:* button label `"Update (content changed)"` instead of `"Update → v1.1"` since there's no new version to display.
- *Legacy install (`installed_version: None`) with `VersionBumped`:* button label stays `"Update → v1.1"` — the `→` reads as "to v1.1", not "from→to"; status column shows `"v1.1 available"` (no LHS to render).
- *`partial_load_warnings`:* same data is *already* surfaced by `listInstalledPlugins`'s own `partial_load_warnings`. The store dedupes by `tracking_file` field and surfaces the merged set via the existing `ERR_INSTALLED_PLUGINS` banner — no new channel.

**Vitest, narrowly scoped.** Phase 2a deferred vitest setup ("different category — separate phases"). Phase 2b reverses that decision because Phase 2b's pure-logic surface (`remediationClass`, `groupFailures`, `formatInstallPluginResult`, `formatRemovePluginResult`) is exactly the work-product that *needs* unit coverage, will not get it from Playwright (which is slow and gates on fixture marketplaces), and won't get it from `npm run check` (which catches only type errors, not behavior). Setup is intentionally minimal: no `@testing-library/svelte`, no `jsdom`, no Tauri-IPC mocks. Pure helpers live in non-`.svelte.ts` modules and are tested in `node` environment with vanilla vitest. Component-level testing remains future scope.

## User-locked decisions

These came out of the 2026-05-05 brainstorming conversation. Documented here so they don't drift during implementation:

1. **Surfaces: BrowseTab + InstalledTab.** PluginCard's "Installed" pill becomes an "Update → v1.1" button on update. InstalledTab table gains a Status column and Update action. Rationale: maximum discoverability; a user browsing for new plugins shouldn't have to switch tabs to learn one of their installed plugins needs an update.

2. **Scan trigger: eager on project mount + after marketplace fetch.** Two re-fire triggers only: (a) `projectPath` changes (existing `$effect` dependency in both tabs), (b) `MarketplacesTab.onMarketplacesUpdated` fires after a successful `kiro-market update`. No background polling (rejected in 2a, security-sensitive). No manual "Rescan" button (additive change later if motivated).

3. **Update click: fire-and-forget, no confirm modal.** Tooltip on hover (`title="Update will replace local edits to plugin files"`) is the only friction. The button is hardcoded to `force=true`, ignoring BrowseTab's global "Force reinstall" checkbox (the checkbox still drives the Install button's behavior; the two actions diverge intentionally).

4. **Remove toast: counts + collapsed `<details>`, auto-expand on failure.** Closed-by-default on success; `<details open>` on failure. Reuses the existing 3-banner pattern (`installError` / `installMessage` / `installWarning`). InstalledTab gains the same banner channels BrowseTab uses; the existing `loadError` channel narrows to fetch/refresh failures only.

5. **Failure rendering: grouped by `(remediationClass, marketplace)`.** Pure function `remediationClass(kind: PluginUpdateFailureKind)` maps 5 kinds → 3 remediation classes (`stale_cache` covers `marketplace_unavailable | manifest_unreadable | hash_failed`; `manifest_invalid` is its own class; `other` is `unknown`). Banner copy keys off the remediation class. Per-row pills share a generic "Update check failed" visual; the kind-specific copy lives in the row's `title=` tooltip.

6. **Edge state UX.** (a) Optimistic "Installed" pill during scan-in-progress; (b) ContentChanged button label is "Update (content changed)" not "Update → v1.1"; (c) legacy installs (`installed_version: None`) display `"v1.1 available"` in status; (d) `partial_load_warnings` from the scan dedup-merge with the same data already on `installedPlugins.partial_load_warnings` (single banner, keyed by `tracking_file`).

7. **Vitest in scope (narrow).** Pure-logic helpers only — `remediationClass`, `groupFailures`, `formatInstallPluginResult`, `formatRemovePluginResult`. No component tests, no jsdom, no Tauri mocking. Component-level testing remains future scope. Reverses 2a's "vitest deferred to a separate phase" because Phase 2b is the first PR with FE logic worth testing.

## Phase 2b architecture

### New module: `plugin-updates.svelte.ts` store

```ts
// crates/kiro-control-center/src/lib/stores/plugin-updates.svelte.ts
import { commands } from "$lib/bindings";
import type {
  DetectUpdatesResult,
  PluginUpdateInfo,
  PluginUpdateFailure,
} from "$lib/bindings";
import { groupFailures, type FailureGroup } from "./plugin-updates";

class PluginUpdatesStore {
  result = $state<DetectUpdatesResult | null>(null);
  loading = $state(false);
  // Toplevel error from detect_plugin_updates Result::Err — used when the
  // command itself failed (couldn't read tracking files at all). Per-plugin
  // failures live on result.failures.
  fetchError = $state<string | null>(null);
  lastProjectPath = $state<string | null>(null);

  failureGroups = $derived.by((): FailureGroup[] =>
    this.result?.failures ? groupFailures(this.result.failures) : []
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
      return;
    }
    this.loading = true;
    this.lastProjectPath = projectPath;
    try {
      const r = await commands.detectPluginUpdates(projectPath);
      if (r.status === "ok") {
        this.result = r.data;
        this.fetchError = null;
      } else {
        this.fetchError = r.error.message;
      }
    } catch (e) {
      this.fetchError = e instanceof Error ? e.message : String(e);
    } finally {
      this.loading = false;
    }
  }
}

export const pluginUpdates = new PluginUpdatesStore();
```

### New module: `plugin-updates.ts` (pure helpers, vitest-tested)

```ts
// crates/kiro-control-center/src/lib/stores/plugin-updates.ts
import type {
  PluginUpdateFailure,
  PluginUpdateFailureKind,
  MarketplaceName,
  PluginName,
} from "$lib/bindings";

export type RemediationClass = "stale_cache" | "manifest_invalid" | "unknown";

export function remediationClass(kind: PluginUpdateFailureKind): RemediationClass {
  switch (kind.kind) {
    case "marketplace_unavailable":
    case "manifest_unreadable":
    case "hash_failed":
      return "stale_cache";
    case "manifest_invalid":
      return "manifest_invalid";
    case "other":
      return "unknown";
  }
  // No default — TypeScript surfaces a compile error if PluginUpdateFailureKind
  // gains a new variant. Mirrors the CLAUDE.md classifier rule on the Rust side
  // ("classifier functions over error enums enumerate every variant").
}

export type FailureGroup = {
  remediation: RemediationClass;
  marketplace: MarketplaceName;
  plugins: PluginName[];
  remediationHint: string;
};

export function groupFailures(failures: PluginUpdateFailure[]): FailureGroup[] {
  const map = new Map<string, FailureGroup>();
  for (const f of failures) {
    const cls = remediationClass(f.kind);
    const groupKey = `${cls}${f.marketplace}`;
    let g = map.get(groupKey);
    if (!g) {
      g = {
        remediation: cls,
        marketplace: f.marketplace,
        plugins: [],
        remediationHint: hintFor(cls, f.marketplace),
      };
      map.set(groupKey, g);
    }
    g.plugins.push(f.plugin);
  }
  return Array.from(map.values());
}

function hintFor(cls: RemediationClass, marketplace: MarketplaceName): string {
  switch (cls) {
    case "stale_cache":
      return `Run \`kiro-market update\` to refresh ${marketplace}.`;
    case "manifest_invalid":
      return "Contact the marketplace owner — `plugin.json` failed to parse.";
    case "unknown":
      return "Update check failed — see browser console for the error chain.";
  }
}

export function kindLabel(kind: PluginUpdateFailureKind): string {
  switch (kind.kind) {
    case "marketplace_unavailable":
      return "Marketplace cache missing or plugin removed from manifest";
    case "manifest_unreadable":
      return "plugin.json missing in marketplace cache";
    case "manifest_invalid":
      return "plugin.json failed to parse";
    case "hash_failed":
      return "Failed to hash installed file";
    case "other":
      return "Update check failed — see console";
  }
}
```

### Updated component: `PluginCard.svelte`

Two new props: `update: PluginUpdateInfo | undefined`, `failure: PluginUpdateFailure | undefined`. The action area's render order:

1. `installing === true` → `"Installing…"` disabled button.
2. `updating === true` (new state — `pendingPluginActions[key] === "update"`) → `"Updating…"` disabled button.
3. `failure !== undefined` (and `installed === true`) → "Update check failed" red pill with `title={kindLabel(failure.kind)}`.
4. `update !== undefined` → orange "Update → v1.1" button (or "Update (content changed)" for `ContentChanged`); `onclick={onUpdate}`.
5. `installed === true` → existing green "Installed" pill.
6. Else → existing default "Install" button.

Update button label per the edge cases in design decision #6:
- `change_signal: VersionBumped` + both versions known → `"Update → v1.1"`
- `change_signal: VersionBumped` + `installed_version === null` → `"Update → v1.1"` (same; the `→` reads as "to" not "from→to")
- `change_signal: VersionBumped` + `available_version === null` → `"Update"` (manifest declares no version)
- `change_signal: ContentChanged` → `"Update (content changed)"`

### Updated component: `InstalledTab.svelte`

Table header gains a `Status` column between `Version` and `Contents`. The action cell becomes `[Update] [Remove]` (Update only present when `pluginUpdates.updateFor(p.marketplace, p.plugin) !== undefined`).

```svelte
<th class="px-4 py-2">Status</th>
...
<td class="px-4 py-3">
  {#if updateInfo}
    <span class="px-2 py-0.5 text-[11px] font-medium text-kiro-warning border
                 border-kiro-warning/40 rounded">
      {updateLabelFor(updateInfo)}
    </span>
  {:else if failure}
    <span class="px-2 py-0.5 text-[11px] font-medium text-kiro-error border
                 border-kiro-error/40 rounded"
          title={kindLabel(failure.kind)}>
      Update check failed
    </span>
  {:else}
    <span class="text-kiro-success text-[11px]">Up to date</span>
  {/if}
</td>
```

The action cell renders both buttons when an update is available; `pendingPluginActions: SvelteMap<string, "install" | "update" | "remove">` drives the disabled+label logic. The existing `removingKey: string | null` is replaced by reads from this map.

### Banner stack

A new `BannerStack.svelte` component extracted from BrowseTab's existing render block (`BrowseTab.svelte:1066-1132`). Both tabs render `<BannerStack errors={fetchErrors} message={installMessage} warning={installWarning} fatalError={installError} ondismiss={(key) => fetchErrors.delete(key)} />` (Svelte 5 callback-prop syntax, not the legacy `on:` directive).

The shared component owns: 3-cap-with-overflow rendering for the `fetchErrors` map, dismiss buttons, and the green/amber/red color treatment.

InstalledTab adopts `fetchErrors: SvelteMap<ErrorSource, string>` for failure groups, with an `ErrorSource` union extended to include `update-check<remediation><marketplace>`. The existing `loadError` and `loadWarning` channels narrow to "couldn't load installed plugins" — they no longer carry remove-action results.

### Update flow

Each tab implements `updatePlugin` with the skeleton below; the only divergence is the post-action refresh call — BrowseTab calls its own `fetchInstalledPlugins()`, InstalledTab calls its own `refresh()`. Pulled out as a parameter so the body is otherwise identical:

```ts
// per-tab; postRefresh differs between tabs.
async function updatePlugin(
  marketplace: string,
  plugin: string,
  postRefresh: () => Promise<void>,
) {
  const key = pluginKey(marketplace, plugin);
  if (pendingPluginActions.has(key)) return;
  pendingPluginActions.set(key, "update");
  installError = null;
  installMessage = null;
  installWarning = null;
  try {
    const result = await commands.installPlugin(
      marketplace, plugin,
      /*force=*/ true,
      /*acceptMcp=*/ false,
      projectPath,
    );
    if (result.status === "ok") {
      const { summary, warnings } = formatInstallPluginResult(result.data, plugin);
      installMessage = `Updated ${plugin}: ${summary}`;
      if (warnings) installWarning = `Updated ${plugin}: ${warnings}`;
      await pluginUpdates.refresh(projectPath);
      await postRefresh();
    } else {
      installError = `Update failed for ${plugin}: ${result.error.message}`;
    }
  } catch (e) {
    installError =
      `Update failed for ${plugin}: ${e instanceof Error ? e.message : String(e)}`;
  } finally {
    pendingPluginActions.delete(key);
  }
}
```

`formatInstallPluginResult` is extracted from `BrowseTab.installWholePlugin` (lines 744-841) into `src/lib/format.ts` so both Install and Update reuse the same summarization logic.

### Remove toast

`InstalledTab.removePlugin` switches from "discard success result" to "format and surface". Adds a new state var:

```ts
let removeResult: RemovePluginResult | null = $state(null);
```

`removeResult` carries the data for the inline `<details>` block independently of the banner channels (which carry the human summary). When set non-null and there are items, the `<details>` block renders below the banner stack.

The action handler:

```ts
async function removePlugin(marketplace: string, plugin: string) {
  const key = pluginKey(marketplace, plugin);
  if (pendingPluginActions.has(key)) return;
  pendingPluginActions.set(key, "remove");
  installError = null;
  installMessage = null;
  installWarning = null;
  try {
    const result = await commands.removePlugin(marketplace, plugin, projectPath);
    if (result.status === "ok") {
      const { summary, hasItems, hasFailures } =
        formatRemovePluginResult(result.data, plugin);
      removeResult = result.data; // drives the <details> render
      if (hasFailures) {
        installWarning = `Removed plugin ${plugin}: ${summary}`;
      } else {
        installMessage = `Removed plugin ${plugin}: ${summary}`;
      }
      await refresh();
      await pluginUpdates.refresh(projectPath);
    } else {
      installError = `Remove failed for ${plugin}: ${result.error.message}`;
    }
  } catch (e) {
    installError = `Remove failed for ${plugin}: ${e instanceof Error ? e.message : String(e)}`;
  } finally {
    pendingPluginActions.delete(key);
  }
}
```

The toast renders inline in InstalledTab below the BannerStack; the `<details>` body iterates `removeResult.skills.removed`, `.steering.removed`, `.agents.removed` and (if `hasFailures`) the corresponding `failures` arrays with `{item}: {error}` lines.

### Failure → banner mapping

The `pluginUpdates.failureGroups` `$derived` produces `FailureGroup[]`. Both tabs reactively project these into the `fetchErrors` map under the new key family:

```ts
$effect(() => {
  // Clear previous update-check banners for keys not in the new group set.
  const seen = new Set<ErrorSource>();
  for (const group of pluginUpdates.failureGroups) {
    const key: ErrorSource =
      `update-check${group.remediation}${group.marketplace}`;
    seen.add(key);
    const pluginList = group.plugins.join(", ");
    fetchErrors.set(
      key,
      `${group.plugins.length} plugin${group.plugins.length === 1 ? "" : "s"} ` +
      `from ${group.marketplace}: ${group.remediationHint}` +
      ` (${pluginList})`,
    );
  }
  for (const k of fetchErrors.keys()) {
    if (k.startsWith("update-check") && !seen.has(k)) fetchErrors.delete(k);
  }
});
```

`partial_load_warnings` get merged into the existing `ERR_INSTALLED_PLUGINS` banner. The merge dedupes by `tracking_file` since both `listInstalledPlugins` and `detectPluginUpdates` read the same files; a single corrupt `installed-skills.json` shouldn't produce two banners that say the same thing.

**Toplevel `fetchError` (rare).** When `commands.detectPluginUpdates` itself fails (`Result::Err` — backend couldn't read tracking files at all), `pluginUpdates.fetchError` carries the message. Both tabs surface this via a new `ERR_UPDATE_FETCH` key on `fetchErrors`:

```ts
$effect(() => {
  if (pluginUpdates.fetchError) {
    fetchErrors.set("update-fetch", `Couldn't check for updates: ${pluginUpdates.fetchError}`);
  } else {
    fetchErrors.delete("update-fetch");
  }
});
```

Distinct from the per-group `update-check<remediation><marketplace>` keys: this one is "scan didn't run at all," they're "scan ran, some plugins failed."

### Cross-marketplace same-plugin idempotency edge

The 2a design footnote (`2026-04-30-phase-2a-update-detection-design.md` line 257) flagged this as "may matter for Phase 2b UI work but doesn't block 2a backend." Investigation: every UI surface is keyed on `(marketplace, plugin)` via `pluginKey()` (`BrowseTab.svelte:9-19`); `installPlugin(marketplace, plugin, ...)` takes marketplace as a param; `installedPluginKeys` is `Set<pluginKey>` not `Set<plugin>`. Two marketplaces shipping the same plugin name produce two distinct rows in InstalledTab and two distinct cards in BrowseTab. **No UI-side work required**; the edge is a backend concern that doesn't surface in 2b's design space.

## Wire format / FFI

Phase 2b introduces **zero new types** crossing FFI. It consumes:

- `DetectUpdatesResult { updates?, failures?, partial_load_warnings? }`
- `PluginUpdateInfo { marketplace, plugin, installed_version, available_version, change_signal }`
- `PluginUpdateFailure { marketplace, plugin, kind, reason }`
- `PluginUpdateFailureKind` (5-variant tagged union)
- `UpdateChangeSignal` (`version_bumped` | `content_changed`)
- `RemovePluginResult { skills, steering, agents }` and the three sub-result types
- `RemoveItemFailure { item, error }`

All shipped in PR #96 (`bindings.ts` regenerated at `b5cbd97`). No backend changes; no new Tauri commands.

## Module map

| File | Status | Responsibility |
|---|---|---|
| `crates/kiro-control-center/src/lib/stores/plugin-updates.svelte.ts` | New | Reactive store wrapping `detectPluginUpdates`; consumed by both tabs |
| `crates/kiro-control-center/src/lib/stores/plugin-updates.ts` | New | Pure helpers (`remediationClass`, `groupFailures`, `kindLabel`); vitest-tested |
| `crates/kiro-control-center/src/lib/stores/plugin-updates.test.ts` | New | Vitest cases for the pure helpers |
| `crates/kiro-control-center/src/lib/format.ts` | Modify | Add `formatInstallPluginResult`, `formatRemovePluginResult` (extracted from BrowseTab) |
| `crates/kiro-control-center/src/lib/format.test.ts` | New | Vitest cases for the extracted formatters |
| `crates/kiro-control-center/src/lib/components/PluginCard.svelte` | Modify | Two new props (`update`, `failure`); 3 new action-area states |
| `crates/kiro-control-center/src/lib/components/InstalledTab.svelte` | Modify | New `Status` column; Update button; consume reshape; multi-banner stack; `<details>` toast |
| `crates/kiro-control-center/src/lib/components/BrowseTab.svelte` | Modify | Wire store; pass `update` + `failure` props to PluginCard; refactor `pendingPluginInstalls` → `pendingPluginActions`; extract banner block |
| `crates/kiro-control-center/src/lib/components/MarketplacesTab.svelte` | Modify | New `onMarketplacesUpdated` callback prop; fired after successful `kiro-market update` |
| `crates/kiro-control-center/src/lib/components/BannerStack.svelte` | New | Extracted from BrowseTab; reused by InstalledTab |
| `crates/kiro-control-center/src/routes/+page.svelte` | Modify | Wire `MarketplacesTab.onMarketplacesUpdated` → `pluginUpdates.refresh(projectPath)` |
| `crates/kiro-control-center/vite.config.ts` | Modify | Add `test: { include: ['src/**/*.test.ts'], environment: 'node' }` block |
| `crates/kiro-control-center/package.json` | Modify | New devDep `vitest`; new script `"test:unit": "vitest run"` |
| `CLAUDE.md` (top-level) | Modify | Add `npm run test:unit` to the pre-commit list; note the FE-pure-logic boundary for vitest |

## Testing strategy

### Vitest (new — narrow scope)

- `plugin-updates.test.ts`:
  - `remediationClass` — every `PluginUpdateFailureKind.kind` value maps to a non-default `RemediationClass`. (Compile-time exhaustiveness via switch is the primary guard; this runtime test backs it up.)
  - `groupFailures` — empty input → empty output; N stale-cache failures from one marketplace → one group; failures from two marketplaces → two groups; `manifest_invalid` and `stale_cache` from same marketplace → two distinct groups (different remediation); plugin order within group preserved.
  - `kindLabel` — every variant returns a non-empty human-readable string.
- `format.test.ts`:
  - `formatInstallPluginResult` — happy path with all 3 sub-results populated; failures-only; warnings-only (e.g. MCP-gated agents); empty (zero installs / zero failures); idempotent reinstalls (steering's `installed[].kind === "idempotent"`).
  - `formatRemovePluginResult` — happy path (3 sub-results, no failures); per-content-type failures land in the right summary bucket; empty cascade (all three sub-results empty).

### Playwright (e2e)

`tests/e2e/app.spec.ts` already gates on `FIXTURE_MARKETPLACE_PATH`. New scenarios:

- **Update available, golden path:** project + fixture marketplace where one plugin's manifest version increments → BrowseTab card shows "Update → v{N}", InstalledTab row shows update pill in Status column.
- **Click Update → success:** click the Update button → toast appears with green "Updated {plugin}: …"; the indicator clears on the post-action `pluginUpdates.refresh()`.
- **Stale-cache failure:** project with installed plugin from a marketplace not present in cache → amber banner with grouped copy; rows show "Update check failed" pill.
- **Remove with sub-results:** remove a plugin with mixed content types → toast appears with `<details>` containing item names; expand verifies named items are present.
- **Remove with cascade failure:** fixture where steering removal fails mid-cascade → amber toast with `<details open>` showing the failed item and error.

### Manual UI smoke

Per CLAUDE.md "For UI or frontend changes, start the dev server and use the feature in a browser before reporting the task as complete." `npm run dev` walkthrough covering:
- No installed plugins → tab loads, no Update affordances anywhere.
- Scan-in-progress → cards/rows render "Installed" pill optimistically; flicker on transition.
- ContentChanged label.
- Legacy install (manually edit `installed-*.json` to remove `version` fields).
- Marketplace offline (rename/move marketplace cache dir under `~/.kiro/marketplaces/`) → grouped amber banner, "Update check failed" rows.
- Remove with no items, with items only, with failures only, with mixed.

### TypeScript / `npm run check`

`svelte-check` continues to gate the merge. The new exhaustive switches (`remediationClass`, `kindLabel`) become compile-time gates against future `PluginUpdateFailureKind` additions.

## Out of scope

Documented here so they don't drift into the plan:

- **Component-level testing** (`@testing-library/svelte`, `jsdom`, Tauri-IPC mocking). Phase 2b adds vitest only for pure-logic helpers. Component tests are a separate scope.
- **Manual "Rescan" button.** Eager-on-mount + after-marketplace-update covers the common cases; an explicit rescan affordance is additive and can ship later if motivated.
- **Tab-level count badge** (option C from Q1). Rejected during brainstorming in favor of inline indicators on both surfaces.
- **Confirmation modal on Update.** Tooltip-only friction; modal is an additive change later if data-loss incidents motivate.
- **Auto-update / background polling.** Already rejected in 2a (security: a malicious marketplace could push a hostile MCP server).
- **Per-content-type Update / partial-Update.** "Update only the steering files" or similar; plugins remain coherent bundles per Phase 1 design.
- **Cross-marketplace same-plugin idempotency edge.** Backend concern; UI keys on `(marketplace, plugin)` everywhere already, so 2b doesn't surface or worsen the edge.
- **Hash memoization in marketplace cache.** Performance optimization; deferred per 2a.

## 6-Gates self-review

### Gate 1 — Grounding

**Real incident driving this work?** Yes:
1. The 2a design's stated goal was a complete plugin lifecycle, of which "users see updates and can act on them" is the keystone. 2a backend without 2b UI is a typed wire format with zero consumers — the value is unrealized until a user can click Update in the UI.
2. The `RemovePluginResult` reshape was specifically motivated by "the toast says '3 skills, 1 steering file, 2 agents' — the user knows the magnitude but not which items" (2a design line 10). Currently `InstalledTab.removePlugin` *discards* the result; the reshape is unfulfilled until 2b consumes it.
3. The vitest scope reversal traces to: 2a deferred vitest because there was no pure FE logic worth testing. 2b introduces such logic (`remediationClass`, `groupFailures`, format helpers) — the deferral's preconditions no longer hold.

### Gate 2 — Threat Model

**Untrusted inputs:** none new at the FFI boundary; Phase 2b is a consumer of types whose `Deserialize` impls already enforce parse-don't-validate at deserialization. The wire-format types (`PluginUpdateInfo`, `PluginUpdateFailure`, `RemovePluginResult` and sub-results) flow `serde_json::from_slice` → typed values → component props. No new Tauri command surface to validate.

**Destructive UI actions:**
- *Update button*: calls `installPlugin(force=true)` which **overwrites** the user's local plugin files. UX mitigation: tooltip warning. Considered: confirmation modal — rejected for symmetry with the existing Install fire-and-forget UX. Tradeoff documented in user-locked decision #3; reversible if data-loss incidents materialize.
- *Remove button*: existing behavior; 2b only changes how the result is rendered.

**Tooltip / banner content** carries `PluginUpdateFailure.reason` strings, which are `error_full_chain` outputs from the backend. Rendered as text nodes (not innerHTML); Svelte's default escaping handles this.

### Gate 3 — Wire Format / FFI

Zero new types crossing FFI. All consumed types ship in PR #96. `bindings.ts` already regenerated. No `specta::Type` derives, no `serde(tag = "kind")` discriminators, no JSON-shape locks to add — all of those landed in 2a.

The `remediationClass` switch over `PluginUpdateFailureKind.kind` is the FE's single point of fanout from a typed enum to UX. If 2a-and-beyond add a new kind, TypeScript surfaces a compile error here (no `default:` arm). Mirrors the CLAUDE.md classifier rule on the Rust side.

### Gate 4 — External Type Boundary

N/A — no Rust crate boundary changes. The TypeScript code consumes already-translated types; no `serde_json::Error` or `gix::Error` surfaces in 2b's code paths.

### Gate 5 — Type Design

**Newtypes preserved:** `MarketplaceName` and `PluginName` are TypeScript-side aliases (`= string`) per `bindings.ts`. The 2b code threads them as opaque values; equality checks use direct string compares since the TS aliases erase. The newtype guarantee lives on the backend; 2b consumes the values at face value.

**Discriminated unions consumed correctly:** `PluginUpdateFailureKind` and `UpdateChangeSignal` are both `{ kind: ... }` tagged. The 2b code switches on `.kind` strings — type-safe, matches the FFI wire format. No positional access (no `tuple[0]` patterns).

**No magic-value sentinels added.** ContentChanged with no `available_version` is encoded by the `change_signal` discriminator + `Option<String>` versions, not by any sentinel string in 2b code. The UI's "Update (content changed)" label is a render-side decision; the wire format remains structurally typed.

### Gate 6 — Reference vs Transcription

**Was the plan review tested against actual code, not transcribed from prose?**

- Module map is anchored to existing file paths verified during exploration: `crates/kiro-control-center/src/lib/components/{PluginCard,InstalledTab,BrowseTab,MarketplacesTab}.svelte`, `crates/kiro-control-center/src/lib/{format.ts,bindings.ts}`, `crates/kiro-control-center/src/lib/stores/project.svelte.ts`. None invented.
- The "extract `formatInstallPluginResult` from BrowseTab" plan cites `BrowseTab.svelte:744-841` — read during exploration, not transcribed.
- The "InstalledTab adopts the BrowseTab banner-stack pattern" plan is anchored to actual code at `BrowseTab.svelte:1066-1132`, verified during exploration.
- The cross-marketplace footnote was investigated by reading `project.rs` directly (the line numbers in the 2a footnote no longer match — the codebase shifted). Conclusion ("UI keys on `(marketplace, plugin)` already; no UI work") is grounded in current file state, not the stale 2a line ref.
- The vitest setup plan is conservative because actual research surfaced the Svelte-5-runes + vitest interaction nuance (testing reactive `$state`/`$derived` requires `flushSync` and the svelte runtime initialization). The plan dodges that complexity by extracting helpers to non-`.svelte.ts` modules — a structural choice, not a transcribed best-practice.

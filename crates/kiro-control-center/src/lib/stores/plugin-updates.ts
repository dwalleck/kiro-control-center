import type {
  MarketplaceName,
  PluginName,
  PluginUpdateFailure,
  PluginUpdateFailureKind,
  PluginUpdateInfo,
} from "$lib/bindings";
import { DELIM } from "$lib/keys";
import {
  isUpdateCheckKey,
  updateCheckErrKey,
} from "$lib/error-source";
import type {
  RemediationClass,
  UpdateCheckKey,
} from "$lib/error-source";

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
}

export function kindLabel(kind: PluginUpdateFailureKind): string {
  switch (kind.kind) {
    case "marketplace_unavailable":
      return "Marketplace cache missing or plugin removed from manifest";
    case "manifest_unreadable":
      return "plugin.json couldn't be read from marketplace cache";
    case "manifest_invalid":
      return "plugin.json failed to parse";
    case "hash_failed":
      return "Failed to hash installed file";
    case "other":
      return "Update check failed — see console";
  }
}

// Unexported on purpose — `remediationHint` is derivable from
// `(remediation, marketplace)` via `hintFor` below, but this type doesn't
// encode that derivation. Hiding the name discourages hand-constructed
// literals with a mismatched hint, but isn't a hard barrier: consumers
// can still reconstruct the shape via `ReturnType<typeof groupFailures>[number]`
// and pass it through `projectUpdateCheckBanners`. Convention: always go
// through `groupFailures` to construct.
type FailureGroup = {
  remediation: RemediationClass;
  marketplace: MarketplaceName;
  plugins: PluginName[];
  remediationHint: string;
};

export function groupFailures(failures: PluginUpdateFailure[]): FailureGroup[] {
  const map = new Map<string, FailureGroup>();
  for (const f of failures) {
    const cls = remediationClass(f.kind);
    // Use DELIM for consistency with ErrorSource keys; `:` is permitted in
    // marketplace names by `validate_name` (only control chars + path
    // separators are rejected).
    const groupKey = `${cls}${DELIM}${f.marketplace}`;
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

// String union, not a tagged-object union. The BrowseAction / InstalledAction
// Extract<> aliases below filter by literal type — if this ever grows object
// payloads (e.g. { kind: "install"; force: boolean }), Extract<> silently
// changes meaning and the narrowings break in non-obvious ways. Switch to
// Exclude<> of the other arms in that case. Mixed unions (some literal arms
// remain alongside new object arms) need re-thinking the alias strategy
// entirely — neither Extract<> nor Exclude<> rescues per-tab subsets.
export type PluginAction = "install" | "update" | "remove";

export type BrowseAction = Extract<PluginAction, "install" | "update">;
export type InstalledAction = Extract<PluginAction, "remove" | "update">;

// Compile-time guard: lists the canonical values once and `satisfies` fails
// the moment any arm becomes non-string-literal — `"install"` won't satisfy
// `{ kind: "install"; ... }`. Also catches arm removal (an absent value
// fails to satisfy the narrower union). The `const _assertStringUnion = true`
// at the bottom forces type-check evaluation: an unused type alias resolving
// to `never` is valid TS, so a value-position assignment is what makes the
// tripwire actually fire. Pairs with `_AssertNarrow` in error-source.ts.
const _PLUGIN_ACTION_VALUES = ["install", "update", "remove"] as const satisfies readonly PluginAction[];
type _AssertStringUnion = (typeof _PLUGIN_ACTION_VALUES)[number] extends string ? true : never;
const _assertStringUnion: _AssertStringUnion = true;

// Column = state sentence; companion `actionUpdateLabel` = button action.
export function statusUpdateLabel(u: PluginUpdateInfo): string {
  if (u.change_signal.kind === "content_changed") return "Content changed since install";
  if (u.installed_version && u.available_version) {
    return `v${u.installed_version} → v${u.available_version}`;
  }
  if (u.available_version) return `v${u.available_version} available`;
  return "Update available";
}

export function actionUpdateLabel(u: PluginUpdateInfo): string {
  if (u.change_signal.kind === "content_changed") return "Update (content changed)";
  if (u.available_version) return `Update → v${u.available_version}`;
  return "Update";
}

/**
 * Pure projection of failureGroups into an upsert+stale-delete pair.
 * Keys are branded as UpdateCheckKey so consumers know the namespace
 * they operate in.
 */
export function projectUpdateCheckBanners(
  groups: FailureGroup[],
  existingKeys: Iterable<string>,
): { upserts: Map<UpdateCheckKey, string>; staleKeys: UpdateCheckKey[] } {
  const upserts = new Map<UpdateCheckKey, string>();
  const seen = new Set<UpdateCheckKey>();
  for (const group of groups) {
    const key = updateCheckErrKey(group.remediation, group.marketplace);
    seen.add(key);
    const noun = group.plugins.length === 1 ? "plugin" : "plugins";
    const list = group.plugins.join(", ");
    upserts.set(
      key,
      `${group.plugins.length} ${noun} from ${group.marketplace}: ${group.remediationHint} (${list})`,
    );
  }
  const staleKeys: UpdateCheckKey[] = [];
  for (const k of existingKeys) {
    if (isUpdateCheckKey(k) && !seen.has(k)) {
      staleKeys.push(k);
    }
  }
  return { upserts, staleKeys };
}

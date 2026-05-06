import type {
  MarketplaceName,
  PluginName,
  PluginUpdateFailure,
  PluginUpdateFailureKind,
  PluginUpdateInfo,
} from "$lib/bindings";
import { DELIM } from "$lib/keys";

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

export type PluginAction = "install" | "update" | "remove";

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

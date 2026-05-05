import type {
  MarketplaceName,
  PluginName,
  PluginUpdateFailure,
  PluginUpdateFailureKind,
} from "$lib/bindings";

/**
 *  Three remediation paths the FE distinguishes among the five
 *  `PluginUpdateFailureKind` variants. The grouping function in
 *  `groupFailures` keys off this so 10 stale-cache failures from one
 *  marketplace produce one banner, not ten.
 *
 *  - `stale_cache` — `marketplace_unavailable`, `manifest_unreadable`,
 *    `hash_failed`. Remediation: run `kiro-market update`.
 *  - `manifest_invalid` — broken `plugin.json` upstream. Remediation:
 *    contact the marketplace owner.
 *  - `unknown` — `other` catch-all. Remediation: see error chain.
 */
export type RemediationClass = "stale_cache" | "manifest_invalid" | "unknown";

/**
 *  Map a `PluginUpdateFailureKind` to its remediation class. The switch
 *  has no `default:` arm — TypeScript surfaces a compile error if the
 *  bindings ever grow a new variant. Mirrors the CLAUDE.md classifier
 *  rule on the Rust side ("classifier functions over error enums
 *  enumerate every variant").
 */
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

/**
 *  Per-kind human-readable tooltip copy for "Update check failed" pills.
 *  Lives on hover, not in the banner — the banner uses the remediation
 *  hint (`hintFor`) instead so 10 failures with the same remediation
 *  produce one banner.
 *
 *  No `default:` arm — same exhaustiveness contract as `remediationClass`.
 */
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

/**
 *  A grouped failure surface. One per `(remediation, marketplace)` —
 *  the natural unit for banner copy, since plugins sharing the same
 *  remediation from the same marketplace want a single combined
 *  "N plugins from acme couldn't be checked" banner instead of N.
 */
export type FailureGroup = {
  remediation: RemediationClass;
  marketplace: MarketplaceName;
  // Plugin names ordered as the scan returned them.
  plugins: PluginName[];
  // Human-readable remediation hint (e.g. "Run `kiro-market update`...").
  remediationHint: string;
};

/**
 *  Collapse a flat `failures: PluginUpdateFailure[]` into per-group
 *  rows. Order of groups in the returned array reflects first-seen
 *  ordering of group keys; plugin order within a group is input order.
 */
export function groupFailures(failures: PluginUpdateFailure[]): FailureGroup[] {
  const map = new Map<string, FailureGroup>();
  for (const f of failures) {
    const cls = remediationClass(f.kind);
    // Composite key — a literal "<remediation>:<marketplace>" suffices
    // because remediationClass values never contain ":" and marketplace
    // names go through a backend newtype that forbids control chars.
    const groupKey = `${cls}:${f.marketplace}`;
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

/**
 *  Per-plugin in-flight action discriminator. Each tab narrows this
 *  to the actions it actually performs (BrowseTab: install/update,
 *  InstalledTab: remove/update) but the union is exported so the
 *  Map shapes stay nameable from one place.
 */
export type PluginAction = "install" | "update" | "remove";

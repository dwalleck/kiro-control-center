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

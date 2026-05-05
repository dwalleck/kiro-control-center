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

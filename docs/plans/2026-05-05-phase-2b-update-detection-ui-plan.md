# Phase 2b — Plugin Update Detection UI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Land the frontend consumer for Phase 2a's update-detection backend: Update indicators + buttons on both BrowseTab and InstalledTab, refreshed Remove toast that itemizes the per-content-type sub-results, plus narrowly-scoped vitest setup so the new pure-logic helpers get unit coverage.

**Architecture:** A module-scoped `pluginUpdates` Svelte store wraps `commands.detectPluginUpdates` and is consumed by both tabs; re-fires on project change + after a marketplace update. Pure helpers (`remediationClass`, `groupFailures`, format helpers) live in non-`.svelte.ts` modules so vitest tests them in `node` env without `@testing-library/svelte` or `jsdom`. Per-plugin failures group by `(remediationClass, marketplace)` and surface in the existing 3-banner pattern (`installError` red / `installMessage` green / `installWarning` amber). The `RemovePluginResult` reshape is consumed via an inline `<details>` block below the banner stack on InstalledTab.

**Tech Stack:** Svelte 5 (runes), TypeScript (strict), Vite, SvelteKit (static adapter), Tauri 2 + tauri-specta-generated bindings, Vitest (new), Playwright.

**Source-of-truth references** (cite-don't-transcribe per Gate 6):
- Wire-format types: `crates/kiro-control-center/src/lib/bindings.ts` (regenerated PR #96, commit `b5cbd97`).
  - `DetectUpdatesResult`: `bindings.ts:252-256`
  - `PluginUpdateInfo`: `bindings.ts:1077-1100`
  - `PluginUpdateFailure`: `bindings.ts:1018-1023`
  - `PluginUpdateFailureKind` (5-variant tagged union): `bindings.ts:1040-1069`
  - `UpdateChangeSignal` (2-variant tagged union): `bindings.ts:1523-1533`
  - `RemovePluginResult` + sub-results + `RemoveItemFailure`: `bindings.ts:1125-1193`
  - `InstallPluginResult_Serialize` (returned by `installPlugin`): `bindings.ts:657-664`
  - `TrackingLoadWarning`: `bindings.ts:1492-1506`
  - `InstalledPluginInfo`: `bindings.ts:785-799`
- Existing summarization logic to extract: `BrowseTab.svelte:734-852` (`installWholePlugin`).
- Existing banner-stack render block to extract: `BrowseTab.svelte:1066-1132`.
- Composite-key helpers (already shared): `src/lib/keys.ts`.
- Existing format-helper conventions (assertNever pattern over discriminated unions): `src/lib/format.ts`.
- Pre-commit suite: `CLAUDE.md` (top-level) — `cargo fmt --check`, `cargo test --workspace`, `cargo clippy --workspace --tests -- -D warnings`. Phase 2b adds `npm run check` and `npm run test:unit` to the FE-tier pre-commit list.

---

## Task 1: Vitest setup

Establishes the test runner with minimum config so subsequent tasks can TDD pure helpers. No `@testing-library/svelte`, no `jsdom`, no Tauri-IPC mocking — just `vitest` running TypeScript in a `node` environment. The first real test is part of Task 2 (the smoke test happens via that work).

**Files:**
- Modify: `crates/kiro-control-center/package.json`
- Modify: `crates/kiro-control-center/vite.config.js`

- [ ] **Step 1.1: Add vitest devDependency**

Edit `crates/kiro-control-center/package.json`. The current devDependencies block ends after `"vite": "^6.0.3"` (see existing file). Add `vitest`:

```diff
   "devDependencies": {
     "@playwright/test": "^1.59.1",
     "@sveltejs/adapter-static": "^3.0.6",
     "@sveltejs/kit": "^2.9.0",
     "@sveltejs/vite-plugin-svelte": "^5.0.0",
     "@tauri-apps/cli": "^2",
     "@types/node": "^25.5.2",
     "svelte": "^5.0.0",
     "svelte-check": "^4.0.0",
     "typescript": "~5.6.2",
-    "vite": "^6.0.3"
+    "vite": "^6.0.3",
+    "vitest": "^2.1.0"
   }
```

- [ ] **Step 1.2: Add `test:unit` npm script**

Edit `crates/kiro-control-center/package.json`'s `scripts` block:

```diff
   "scripts": {
     "dev": "vite dev",
     "build": "vite build",
     "preview": "vite preview",
     "check": "svelte-kit sync && svelte-check --tsconfig ./tsconfig.json",
     "check:watch": "svelte-kit sync && svelte-check --tsconfig ./tsconfig.json --watch",
     "tauri": "tauri",
-    "test:e2e": "playwright test"
+    "test:e2e": "playwright test",
+    "test:unit": "vitest run"
   },
```

- [ ] **Step 1.3: Add a `test:` block to vite.config.js**

Replace the entirety of `crates/kiro-control-center/vite.config.js` with:

```js
import { defineConfig } from "vite";
import { sveltekit } from "@sveltejs/kit/vite";

const host = process.env.TAURI_DEV_HOST;

// https://vite.dev/config/
export default defineConfig(async () => ({
  plugins: [sveltekit()],

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
  // 2. tauri expects a fixed port, fail if that port is not available
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      // 3. tell Vite to ignore watching `src-tauri`
      ignored: ["**/src-tauri/**"],
    },
  },

  // Vitest config — pure-logic tests only (no jsdom, no @testing-library/svelte).
  // `environment: 'node'` keeps DOM concerns out of scope. Test files colocated
  // next to the source they exercise (`*.test.ts`); component-level testing is
  // intentionally future scope (see docs/plans/2026-05-05-phase-2b-...-design.md).
  test: {
    include: ["src/**/*.test.ts"],
    environment: "node",
  },
}));
```

- [ ] **Step 1.4: Install the new devDep**

Run from `crates/kiro-control-center/`:

```bash
npm install
```

Expected: `vitest` installs, no errors. `package-lock.json` updates.

- [ ] **Step 1.5: Verify `vitest run` is wired**

Run from `crates/kiro-control-center/`:

```bash
npm run test:unit
```

Expected: `vitest` runs and reports `No test files found, exiting with code 0` (or similar) — no test files exist yet. The command itself MUST succeed (exit 0); if it errors out vitest is misconfigured.

- [ ] **Step 1.6: Commit**

```bash
git add crates/kiro-control-center/package.json \
        crates/kiro-control-center/package-lock.json \
        crates/kiro-control-center/vite.config.js
git commit -m "feat(test): add vitest for FE pure-logic helpers (Phase 2b prep)"
```

---

## Task 2: Pure helpers — `remediationClass` (TDD)

The first real vitest test. Drives a thin module that holds the `PluginUpdateFailureKind` → `RemediationClass` mapping. Establishes the file pattern (`plugin-updates.ts` next to the future `plugin-updates.svelte.ts` store).

**Files:**
- Create: `crates/kiro-control-center/src/lib/stores/plugin-updates.ts`
- Create: `crates/kiro-control-center/src/lib/stores/plugin-updates.test.ts`

- [ ] **Step 2.1: Write the failing test**

Create `crates/kiro-control-center/src/lib/stores/plugin-updates.test.ts`:

```ts
import { describe, expect, it } from "vitest";
import type { PluginUpdateFailureKind } from "$lib/bindings";
import { remediationClass } from "./plugin-updates";

describe("remediationClass", () => {
  it("maps marketplace_unavailable to stale_cache", () => {
    const kind: PluginUpdateFailureKind = { kind: "marketplace_unavailable" };
    expect(remediationClass(kind)).toBe("stale_cache");
  });

  it("maps manifest_unreadable to stale_cache", () => {
    const kind: PluginUpdateFailureKind = { kind: "manifest_unreadable" };
    expect(remediationClass(kind)).toBe("stale_cache");
  });

  it("maps hash_failed to stale_cache", () => {
    const kind: PluginUpdateFailureKind = { kind: "hash_failed" };
    expect(remediationClass(kind)).toBe("stale_cache");
  });

  it("maps manifest_invalid to its own class", () => {
    const kind: PluginUpdateFailureKind = { kind: "manifest_invalid" };
    expect(remediationClass(kind)).toBe("manifest_invalid");
  });

  it("maps other to unknown", () => {
    const kind: PluginUpdateFailureKind = { kind: "other" };
    expect(remediationClass(kind)).toBe("unknown");
  });
});
```

- [ ] **Step 2.2: Run the test, verify it fails**

```bash
npm run test:unit -- plugin-updates.test
```

Expected: FAIL — module `./plugin-updates` cannot be resolved.

- [ ] **Step 2.3: Write minimal implementation**

Create `crates/kiro-control-center/src/lib/stores/plugin-updates.ts`:

```ts
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
```

- [ ] **Step 2.4: Run the tests, verify they pass**

```bash
npm run test:unit -- plugin-updates.test
```

Expected: 5 tests pass.

- [ ] **Step 2.5: Commit**

```bash
git add crates/kiro-control-center/src/lib/stores/plugin-updates.ts \
        crates/kiro-control-center/src/lib/stores/plugin-updates.test.ts
git commit -m "feat(updates): add remediationClass helper + tests"
```

---

## Task 3: Pure helpers — `kindLabel`

Per-kind tooltip copy. Drives the row pill's `title=` attribute.

**Files:**
- Modify: `crates/kiro-control-center/src/lib/stores/plugin-updates.ts`
- Modify: `crates/kiro-control-center/src/lib/stores/plugin-updates.test.ts`

- [ ] **Step 3.1: Write the failing tests**

Append to `crates/kiro-control-center/src/lib/stores/plugin-updates.test.ts`:

```ts
import { kindLabel } from "./plugin-updates";

describe("kindLabel", () => {
  it("returns a non-empty string for every PluginUpdateFailureKind variant", () => {
    const variants: PluginUpdateFailureKind[] = [
      { kind: "marketplace_unavailable" },
      { kind: "manifest_unreadable" },
      { kind: "manifest_invalid" },
      { kind: "hash_failed" },
      { kind: "other" },
    ];
    for (const v of variants) {
      const label = kindLabel(v);
      expect(label.length).toBeGreaterThan(0);
    }
  });

  it("produces distinct labels for distinct kinds", () => {
    const labels = new Set([
      kindLabel({ kind: "marketplace_unavailable" }),
      kindLabel({ kind: "manifest_unreadable" }),
      kindLabel({ kind: "manifest_invalid" }),
      kindLabel({ kind: "hash_failed" }),
      kindLabel({ kind: "other" }),
    ]);
    expect(labels.size).toBe(5);
  });
});
```

Also extend the existing top-of-file import to bring in `kindLabel`:

```diff
-import { remediationClass } from "./plugin-updates";
+import { remediationClass, kindLabel } from "./plugin-updates";
```

(or merge into the new `import { kindLabel } from "./plugin-updates";` block if the engineer prefers per-suite imports — the existing `remediationClass` import stays where it is.)

- [ ] **Step 3.2: Run the tests, verify they fail**

```bash
npm run test:unit -- plugin-updates.test
```

Expected: FAIL — `kindLabel` is not exported.

- [ ] **Step 3.3: Add the implementation**

Append to `crates/kiro-control-center/src/lib/stores/plugin-updates.ts`:

```ts
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
```

- [ ] **Step 3.4: Run the tests, verify they pass**

```bash
npm run test:unit -- plugin-updates.test
```

Expected: 7 tests pass (5 from Task 2 + 2 new).

- [ ] **Step 3.5: Commit**

```bash
git add crates/kiro-control-center/src/lib/stores/plugin-updates.ts \
        crates/kiro-control-center/src/lib/stores/plugin-updates.test.ts
git commit -m "feat(updates): add kindLabel helper + tests"
```

---

## Task 4: Pure helpers — `groupFailures`

The grouping pipeline. Collapses N stale-cache failures from one marketplace into one banner-bound `FailureGroup`.

**Files:**
- Modify: `crates/kiro-control-center/src/lib/stores/plugin-updates.ts`
- Modify: `crates/kiro-control-center/src/lib/stores/plugin-updates.test.ts`

- [ ] **Step 4.1: Write the failing tests**

Append to `crates/kiro-control-center/src/lib/stores/plugin-updates.test.ts`:

```ts
import { groupFailures, type FailureGroup } from "./plugin-updates";

function failure(
  marketplace: string,
  plugin: string,
  kind: PluginUpdateFailureKind,
): PluginUpdateFailure {
  return {
    marketplace: marketplace as MarketplaceName,
    plugin: plugin as PluginName,
    kind,
    reason: "test reason",
  };
}

describe("groupFailures", () => {
  it("returns empty array for empty input", () => {
    expect(groupFailures([])).toEqual([]);
  });

  it("collapses N stale-cache failures from one marketplace into one group", () => {
    const groups = groupFailures([
      failure("acme", "p1", { kind: "marketplace_unavailable" }),
      failure("acme", "p2", { kind: "manifest_unreadable" }),
      failure("acme", "p3", { kind: "hash_failed" }),
    ]);
    expect(groups).toHaveLength(1);
    expect(groups[0].remediation).toBe("stale_cache");
    expect(groups[0].marketplace).toBe("acme");
    expect(groups[0].plugins).toEqual(["p1", "p2", "p3"]);
  });

  it("produces separate groups for two marketplaces", () => {
    const groups = groupFailures([
      failure("acme", "p1", { kind: "marketplace_unavailable" }),
      failure("beta", "p2", { kind: "marketplace_unavailable" }),
    ]);
    expect(groups).toHaveLength(2);
    const marketplaces = new Set(groups.map((g) => g.marketplace));
    expect(marketplaces).toEqual(new Set(["acme", "beta"]));
  });

  it("produces separate groups for distinct remediation classes from same marketplace", () => {
    const groups = groupFailures([
      failure("acme", "p1", { kind: "marketplace_unavailable" }),
      failure("acme", "p2", { kind: "manifest_invalid" }),
    ]);
    expect(groups).toHaveLength(2);
    const remediations = new Set(groups.map((g) => g.remediation));
    expect(remediations).toEqual(new Set(["stale_cache", "manifest_invalid"]));
  });

  it("preserves plugin order within a group as the input was ordered", () => {
    const groups = groupFailures([
      failure("acme", "z-plugin", { kind: "marketplace_unavailable" }),
      failure("acme", "a-plugin", { kind: "marketplace_unavailable" }),
      failure("acme", "m-plugin", { kind: "marketplace_unavailable" }),
    ]);
    expect(groups[0].plugins).toEqual(["z-plugin", "a-plugin", "m-plugin"]);
  });

  it("each group carries a non-empty remediationHint", () => {
    const groups: FailureGroup[] = groupFailures([
      failure("acme", "p1", { kind: "marketplace_unavailable" }),
      failure("acme", "p2", { kind: "manifest_invalid" }),
      failure("acme", "p3", { kind: "other" }),
    ]);
    for (const g of groups) {
      expect(g.remediationHint.length).toBeGreaterThan(0);
    }
  });
});
```

Also extend the test file's imports near the top:

```diff
-import type { PluginUpdateFailureKind } from "$lib/bindings";
+import type {
+  MarketplaceName,
+  PluginName,
+  PluginUpdateFailure,
+  PluginUpdateFailureKind,
+} from "$lib/bindings";
```

- [ ] **Step 4.2: Run the tests, verify they fail**

```bash
npm run test:unit -- plugin-updates.test
```

Expected: FAIL — `groupFailures` and `FailureGroup` are not exported.

- [ ] **Step 4.3: Add the implementation**

Append to `crates/kiro-control-center/src/lib/stores/plugin-updates.ts`:

```ts
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
```

- [ ] **Step 4.4: Run the tests, verify they pass**

```bash
npm run test:unit -- plugin-updates.test
```

Expected: 13 tests pass (5 + 2 + 6).

- [ ] **Step 4.5: Run typecheck**

```bash
npm run check
```

Expected: 0 errors.

- [ ] **Step 4.6: Commit**

```bash
git add crates/kiro-control-center/src/lib/stores/plugin-updates.ts \
        crates/kiro-control-center/src/lib/stores/plugin-updates.test.ts
git commit -m "feat(updates): add groupFailures + FailureGroup + tests"
```

---

## Task 5: Format helper — `formatInstallPluginResult` (extract + TDD)

Extract the install-result summarization currently inlined in `BrowseTab.svelte:734-852` (specifically the summary-building logic at 753-841) into `format.ts` so both the existing Install flow and the new Update flow consume the same helper. Tests-first against the type structure of `InstallPluginResult_Serialize` (`bindings.ts:657-664`).

**Files:**
- Modify: `crates/kiro-control-center/src/lib/format.ts`
- Create: `crates/kiro-control-center/src/lib/format.test.ts`

- [ ] **Step 5.1: Write the failing tests**

Create `crates/kiro-control-center/src/lib/format.test.ts`:

```ts
import { describe, expect, it } from "vitest";
import type {
  InstallPluginResult_Serialize,
  MarketplaceName,
  PluginName,
} from "$lib/bindings";
import { formatInstallPluginResult } from "./format";

// Field names + structure tracked from bindings.ts (see plan
// "Source-of-truth references"):
//  - InstallSkillsResult:             bindings.ts:667-686
//  - InstallSteeringResult_Serialize: bindings.ts:699-703
//  - InstallAgentsResult_Serialize:   bindings.ts:522-560
//  - FailedSkill:                     bindings.ts:352-356
//  - InstalledSteeringOutcome:        bindings.ts:853-859
//  - InstallOutcomeKind:              bindings.ts:568-581
function emptyInstallResult(): InstallPluginResult_Serialize {
  return {
    marketplace: "acme" as MarketplaceName,
    plugin: "p" as PluginName,
    version: null,
    skills: { installed: [], skipped: [], failed: [], skipped_skills: [] },
    steering: { installed: [], failed: [], warnings: [] },
    // InstallAgentsResult_Serialize requires installed_native +
    // installed_companions (bindings.ts:553, :559). Both default to
    // empty/null in this fixture.
    agents: {
      installed: [],
      skipped: [],
      failed: [],
      warnings: [],
      installed_native: [],
      installed_companions: null,
    },
  };
}

describe("formatInstallPluginResult", () => {
  it("happy path: counts all 3 sub-results", () => {
    const r = emptyInstallResult();
    r.skills.installed = ["a", "b"];
    r.steering.installed = [
      { source: "s.md", destination: "s.md", kind: "installed", source_hash: "h", installed_hash: "h" },
    ];
    // installed: string[] (bindings.ts:527), not an object array.
    r.agents.installed = ["g"];
    const out = formatInstallPluginResult(r, "p");
    expect(out.summary).toContain("2 skill");
    expect(out.summary).toContain("1 steering");
    expect(out.summary).toContain("1 agent");
    expect(out.anyInstalled).toBe(true);
    expect(out.anyFailed).toBe(false);
  });

  it("failures-only: anyInstalled=false, anyFailed=true", () => {
    const r = emptyInstallResult();
    // FailedSkill requires `kind: FailedSkillReason` (bindings.ts:352-356).
    r.skills.failed = [
      { name: "broken", error: "oops", kind: { kind: "install_failed" } },
    ];
    const out = formatInstallPluginResult(r, "p");
    expect(out.anyInstalled).toBe(false);
    expect(out.anyFailed).toBe(true);
    expect(out.summary).toContain("1 skill failed");
  });

  it("warnings-only (e.g. MCP-gated agent): warnings string present, no failure flag", () => {
    const r = emptyInstallResult();
    r.agents.warnings = [
      { kind: "mcp_servers_require_opt_in", agent: "scary", transports: ["stdio"] },
    ];
    const out = formatInstallPluginResult(r, "p");
    expect(out.warnings).not.toBeNull();
    expect(out.warnings).toContain("scary");
    expect(out.anyFailed).toBe(false);
  });

  it("empty: summary reads 'nothing to install'", () => {
    const r = emptyInstallResult();
    const out = formatInstallPluginResult(r, "p");
    expect(out.summary).toBe("nothing to install");
    expect(out.anyInstalled).toBe(false);
    expect(out.anyFailed).toBe(false);
  });

  it("skipped (idempotent skill): counted as 'already installed'", () => {
    const r = emptyInstallResult();
    r.skills.skipped = ["a", "b"];
    const out = formatInstallPluginResult(r, "p");
    expect(out.summary).toContain("2 skills already installed");
  });
});
```

- [ ] **Step 5.2: Run the tests, verify they fail**

```bash
npm run test:unit -- format.test
```

Expected: FAIL — `formatInstallPluginResult` is not exported.

- [ ] **Step 5.3: Add the implementation**

Append to `crates/kiro-control-center/src/lib/format.ts`:

```ts
import type { InstallPluginResult_Serialize } from "$lib/bindings";

/**
 *  Summarized view of an `InstallPluginResult_Serialize` for banner
 *  rendering. Extracted from `BrowseTab.installWholePlugin` so the new
 *  Update flow (which also calls `installPlugin`, just with `force=true`)
 *  reuses the same summarization rather than duplicating it.
 *
 *  - `summary`: human-readable mid-dot-separated count phrase
 *    (e.g. "2 skills · 1 steering · 1 agent"). Reads "nothing to install"
 *    when nothing happened.
 *  - `warnings`: pipe-separated warning lines (steering-scan warnings,
 *    MCP-gated agents, per-skill skipped_skills) or `null` when empty.
 *  - `anyInstalled` / `anyFailed`: caller uses these to decide which
 *    banner channel (success vs. error vs. warning) to route to.
 */
export type FormattedInstallPluginResult = {
  summary: string;
  warnings: string | null;
  anyInstalled: boolean;
  anyFailed: boolean;
};

export function formatInstallPluginResult(
  r: InstallPluginResult_Serialize,
  _plugin: string,
): FormattedInstallPluginResult {
  const summaryParts: string[] = [];
  const warningParts: string[] = [];

  // Skills sub-result.
  {
    const skills = r.skills;
    if (skills.installed.length > 0) {
      const noun = skills.installed.length === 1 ? "skill" : "skills";
      summaryParts.push(`${skills.installed.length} ${noun}`);
    }
    if (skills.failed.length > 0) {
      const noun = skills.failed.length === 1 ? "skill" : "skills";
      summaryParts.push(`${skills.failed.length} ${noun} failed`);
    }
    if (skills.skipped.length > 0) {
      const noun = skills.skipped.length === 1 ? "skill" : "skills";
      summaryParts.push(`${skills.skipped.length} ${noun} already installed`);
    }
    if (skills.skipped_skills.length > 0) {
      warningParts.push(formatSkippedSkillsForPlugin(skills.skipped_skills));
    }
  }

  // Steering sub-result. Idempotent reinstalls land in `installed` with
  // `kind: idempotent` (not a separate field) — the current banner shape
  // counts them as installed; per-content breakdown is future scope.
  {
    const steering = r.steering;
    if (steering.installed.length > 0) {
      const noun = steering.installed.length === 1 ? "file" : "files";
      summaryParts.push(`${steering.installed.length} steering ${noun}`);
    }
    if (steering.failed.length > 0) {
      summaryParts.push(`${steering.failed.length} steering failed`);
    }
    for (const w of steering.warnings) {
      warningParts.push(formatSteeringWarning(w));
    }
  }

  // Agents sub-result.
  {
    const agents = r.agents;
    if (agents.installed.length > 0) {
      const noun = agents.installed.length === 1 ? "agent" : "agents";
      summaryParts.push(`${agents.installed.length} ${noun}`);
    }
    if (agents.failed.length > 0) {
      const noun = agents.failed.length === 1 ? "agent" : "agents";
      summaryParts.push(`${agents.failed.length} ${noun} failed`);
    }
    if (agents.skipped.length > 0) {
      const noun = agents.skipped.length === 1 ? "agent" : "agents";
      summaryParts.push(`${agents.skipped.length} ${noun} already installed`);
    }
    for (const w of agents.warnings) {
      warningParts.push(formatInstallWarning(w));
    }
  }

  const anyFailed =
    r.skills.failed.length + r.steering.failed.length + r.agents.failed.length > 0;
  const anyInstalled =
    r.skills.installed.length +
      r.steering.installed.length +
      r.agents.installed.length >
    0;
  const summary = summaryParts.length > 0 ? summaryParts.join(" · ") : "nothing to install";
  const warnings = warningParts.length > 0 ? warningParts.join(" | ") : null;

  return { summary, warnings, anyInstalled, anyFailed };
}
```

- [ ] **Step 5.4: Run the tests, verify they pass**

```bash
npm run test:unit -- format.test
```

Expected: 5 tests pass.

- [ ] **Step 5.5: Run typecheck**

```bash
npm run check
```

Expected: 0 errors.

- [ ] **Step 5.6: Commit**

```bash
git add crates/kiro-control-center/src/lib/format.ts \
        crates/kiro-control-center/src/lib/format.test.ts
git commit -m "feat(format): extract formatInstallPluginResult helper + tests"
```

---

## Task 6: Format helper — `formatRemovePluginResult` (TDD)

Mirrors Task 5 but for the new `RemovePluginResult` shape. Drives the new Remove toast.

**Files:**
- Modify: `crates/kiro-control-center/src/lib/format.ts`
- Modify: `crates/kiro-control-center/src/lib/format.test.ts`

- [ ] **Step 6.1: Write the failing tests**

Append to `crates/kiro-control-center/src/lib/format.test.ts`:

```ts
import type { RemovePluginResult } from "$lib/bindings";
import { formatRemovePluginResult } from "./format";

// Field names + structure tracked from bindings.ts:
//  - RemovePluginResult:  bindings.ts:1171-1175
//  - RemoveSkillsResult:  bindings.ts:1181-1184
//  - RemoveSteeringResult: bindings.ts:1190-1193
//  - RemoveAgentsResult:  bindings.ts:1134-1137
//  - RemoveItemFailure:   bindings.ts:1145-1156
function emptyRemoveResult(): RemovePluginResult {
  return {
    skills: { removed: [], failures: [] },
    steering: { removed: [], failures: [] },
    agents: { removed: [], failures: [] },
  };
}

describe("formatRemovePluginResult", () => {
  it("happy path: counts all 3 sub-results", () => {
    const r = emptyRemoveResult();
    r.skills.removed = ["a", "b", "c"];
    r.steering.removed = ["s.md"];
    r.agents.removed = ["g1", "g2"];
    const out = formatRemovePluginResult(r, "p");
    expect(out.summary).toContain("3 skill");
    expect(out.summary).toContain("1 steering");
    expect(out.summary).toContain("2 agent");
    expect(out.hasItems).toBe(true);
    expect(out.hasFailures).toBe(false);
  });

  it("steering failure lands in summary (failed count) and hasFailures=true", () => {
    const r = emptyRemoveResult();
    r.steering.failures = [{ item: "broken.md", error: "permission denied" }];
    const out = formatRemovePluginResult(r, "p");
    expect(out.hasFailures).toBe(true);
    expect(out.summary).toContain("1 steering failed");
  });

  it("empty (zero items, zero failures): hasItems=false, hasFailures=false", () => {
    const r = emptyRemoveResult();
    const out = formatRemovePluginResult(r, "p");
    expect(out.hasItems).toBe(false);
    expect(out.hasFailures).toBe(false);
    expect(out.summary).toBe("nothing to remove");
  });

  it("treats undefined removed/failures as empty arrays", () => {
    // The wire format makes both fields optional (#[serde(default)]).
    const r: RemovePluginResult = {
      skills: {},
      steering: {},
      agents: {},
    };
    const out = formatRemovePluginResult(r, "p");
    expect(out.hasItems).toBe(false);
    expect(out.hasFailures).toBe(false);
  });
});
```

- [ ] **Step 6.2: Run the tests, verify they fail**

```bash
npm run test:unit -- format.test
```

Expected: FAIL — `formatRemovePluginResult` is not exported.

- [ ] **Step 6.3: Add the implementation**

Append to `crates/kiro-control-center/src/lib/format.ts`:

```ts
import type { RemovePluginResult } from "$lib/bindings";

/**
 *  Summarized view of a `RemovePluginResult` for banner + `<details>`
 *  rendering on the InstalledTab.
 *
 *  - `summary`: human-readable mid-dot-separated count phrase
 *    (mirrors `formatInstallPluginResult`'s shape — "3 skills · 1
 *    steering · 2 agents" for happy path; appends "N <type> failed"
 *    for partial failure). Reads "nothing to remove" on empty.
 *  - `hasItems`: at least one removed-list is non-empty. Drives the
 *    decision whether to render the `<details>` block at all.
 *  - `hasFailures`: at least one failures-list is non-empty. Drives
 *    the choice of banner channel (amber vs. green) and the `<details
 *    open>` auto-expand.
 */
export type FormattedRemovePluginResult = {
  summary: string;
  hasItems: boolean;
  hasFailures: boolean;
};

export function formatRemovePluginResult(
  r: RemovePluginResult,
  _plugin: string,
): FormattedRemovePluginResult {
  // Sub-result fields are optional per the wire format
  // (RemoveSkillsResult.removed?: string[], etc.). Default to empty
  // arrays so the rest of this function can do plain `.length` reads.
  const skillsRemoved = r.skills.removed ?? [];
  const skillsFailures = r.skills.failures ?? [];
  const steeringRemoved = r.steering.removed ?? [];
  const steeringFailures = r.steering.failures ?? [];
  const agentsRemoved = r.agents.removed ?? [];
  const agentsFailures = r.agents.failures ?? [];

  const summaryParts: string[] = [];

  if (skillsRemoved.length > 0) {
    const noun = skillsRemoved.length === 1 ? "skill" : "skills";
    summaryParts.push(`${skillsRemoved.length} ${noun}`);
  }
  if (steeringRemoved.length > 0) {
    const noun = steeringRemoved.length === 1 ? "file" : "files";
    summaryParts.push(`${steeringRemoved.length} steering ${noun}`);
  }
  if (agentsRemoved.length > 0) {
    const noun = agentsRemoved.length === 1 ? "agent" : "agents";
    summaryParts.push(`${agentsRemoved.length} ${noun}`);
  }
  if (skillsFailures.length > 0) {
    const noun = skillsFailures.length === 1 ? "skill" : "skills";
    summaryParts.push(`${skillsFailures.length} ${noun} failed`);
  }
  if (steeringFailures.length > 0) {
    summaryParts.push(`${steeringFailures.length} steering failed`);
  }
  if (agentsFailures.length > 0) {
    const noun = agentsFailures.length === 1 ? "agent" : "agents";
    summaryParts.push(`${agentsFailures.length} ${noun} failed`);
  }

  const hasItems =
    skillsRemoved.length + steeringRemoved.length + agentsRemoved.length > 0;
  const hasFailures =
    skillsFailures.length + steeringFailures.length + agentsFailures.length > 0;
  const summary = summaryParts.length > 0 ? summaryParts.join(" · ") : "nothing to remove";

  return { summary, hasItems, hasFailures };
}
```

- [ ] **Step 6.4: Run the tests, verify they pass**

```bash
npm run test:unit
```

Expected: all unit tests pass (Task 2-6 cumulative — 13 + 5 + 4 = 22 tests).

- [ ] **Step 6.5: Run typecheck**

```bash
npm run check
```

Expected: 0 errors.

- [ ] **Step 6.6: Commit**

```bash
git add crates/kiro-control-center/src/lib/format.ts \
        crates/kiro-control-center/src/lib/format.test.ts
git commit -m "feat(format): add formatRemovePluginResult helper + tests"
```

---

## Task 7: `pluginUpdates` Svelte store

The reactive store wrapping `commands.detectPluginUpdates`. Both tabs consume it; pure helpers from Task 4 power the `failureGroups` derived. No tests in this task — reactive-state testing is out of scope per the design ("the store's reactive layer stays untested in this PR — that's the line").

**Files:**
- Create: `crates/kiro-control-center/src/lib/stores/plugin-updates.svelte.ts`

- [ ] **Step 7.1: Create the store module**

Create `crates/kiro-control-center/src/lib/stores/plugin-updates.svelte.ts`:

```ts
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

- [ ] **Step 7.2: Verify svelte-check passes**

```bash
npm run check
```

Expected: 0 errors. (No consumer of the store yet — just verifying the new module typechecks.)

- [ ] **Step 7.3: Commit**

```bash
git add crates/kiro-control-center/src/lib/stores/plugin-updates.svelte.ts
git commit -m "feat(updates): add pluginUpdates Svelte store"
```

---

## Task 8: Extract `BannerStack.svelte` from BrowseTab

Pull the existing banner render block from `BrowseTab.svelte:1066-1132` into a shared component so InstalledTab can adopt the same UX in Task 11. No behavior change — verbatim extraction. Does NOT modify BrowseTab in this task; Task 9 swaps BrowseTab to consume the new component.

**Files:**
- Create: `crates/kiro-control-center/src/lib/components/BannerStack.svelte`

- [ ] **Step 8.1: Create the BannerStack component**

Create `crates/kiro-control-center/src/lib/components/BannerStack.svelte`:

```svelte
<script lang="ts" generics="K extends string">
  import type { SvelteMap } from "svelte/reactivity";

  // Extracted from BrowseTab.svelte (the prior render block lived at
  // lines 1066-1132). The shared component owns: 3-cap-with-overflow
  // rendering for the `errors` map, dismiss buttons (calls back via
  // `ondismiss`), and the green/amber/red color treatment for the
  // remaining three banner channels.
  //
  // The `errors` map is generic over its key type via Svelte 5's
  // `generics="K extends string"` script-tag attribute — `K` flows
  // through the dismiss callback so the consumer can branch type-
  // safely on which key was dismissed without a cast.
  type Props = {
    errors: SvelteMap<K, string>;
    message: string | null;
    warning: string | null;
    fatalError: string | null;
    // `errLabel` is per-tab because the screen-reader label depends
    // on the consumer's ErrorSource taxonomy (e.g. "Dismiss error
    // for acme/foo" vs "Dismiss installed-plugins error").
    errLabel: (key: K) => string;
    // Svelte 5 callback prop, not the legacy `on:dismiss` directive.
    ondismiss: (key: K) => void;
    onmessageDismiss?: () => void;
    onwarningDismiss?: () => void;
    onfatalErrorDismiss?: () => void;
  };

  let {
    errors,
    message,
    warning,
    fatalError,
    errLabel,
    ondismiss,
    onmessageDismiss,
    onwarningDismiss,
    onfatalErrorDismiss,
  }: Props = $props();
</script>

<!-- Banners render newest-first (reverse insertion order) and cap at 3 so
     a storm of broken plugins doesn't push the grid off-screen. Dismissing
     a banner or resolving its source surfaces the next-newest below. -->
{#each [...errors].reverse().slice(0, 3) as [key, msg] (key)}
  <div
    data-testid="fetch-error"
    class="mx-4 mt-3 px-4 py-3 rounded-md bg-kiro-error/10 border border-kiro-error/30 flex items-start gap-3"
  >
    <p class="text-sm text-kiro-error flex-1">{msg}</p>
    <button
      type="button"
      onclick={() => ondismiss(key)}
      aria-label={errLabel(key)}
      class="text-kiro-error/70 hover:text-kiro-error text-lg leading-none flex-shrink-0 focus:outline-none focus:ring-2 focus:ring-kiro-accent-500 rounded"
    >
      ×
    </button>
  </div>
{/each}
{#if errors.size > 3}
  <div
    data-testid="fetch-error-overflow"
    class="mx-4 mt-3 px-4 py-2 text-xs text-kiro-subtle text-center border border-kiro-muted/50 rounded-md bg-kiro-surface/30"
  >
    +{errors.size - 3} more {errors.size - 3 === 1 ? "error" : "errors"} — dismiss or resolve above to see the rest
  </div>
{/if}

{#if fatalError}
  <div
    data-testid="install-error"
    class="mx-4 mt-3 px-4 py-3 rounded-md bg-kiro-error/10 border border-kiro-error/30 flex items-start gap-3"
  >
    <p class="text-sm text-kiro-error flex-1">{fatalError}</p>
    <button
      type="button"
      onclick={() => onfatalErrorDismiss?.()}
      aria-label="Dismiss install error"
      class="text-kiro-error/70 hover:text-kiro-error text-lg leading-none flex-shrink-0 focus:outline-none focus:ring-2 focus:ring-kiro-accent-500 rounded"
    >
      ×
    </button>
  </div>
{/if}

{#if message}
  <div class="mx-4 mt-3 px-4 py-3 rounded-md bg-kiro-success/10 border border-kiro-success/30 flex items-start gap-3">
    <p class="text-sm text-kiro-success flex-1">{message}</p>
    {#if onmessageDismiss}
      <button
        type="button"
        onclick={() => onmessageDismiss?.()}
        aria-label="Dismiss success message"
        class="text-kiro-success/70 hover:text-kiro-success text-lg leading-none flex-shrink-0 focus:outline-none focus:ring-2 focus:ring-kiro-accent-500 rounded"
      >
        ×
      </button>
    {/if}
  </div>
{/if}

{#if warning}
  <div
    data-testid="install-warning"
    class="mx-4 mt-3 px-4 py-3 rounded-md bg-kiro-warning/10 border border-kiro-warning/30 flex items-start gap-3"
  >
    <p class="text-sm text-kiro-warning flex-1">{warning}</p>
    <button
      type="button"
      onclick={() => onwarningDismiss?.()}
      aria-label="Dismiss install warning"
      class="text-kiro-warning/70 hover:text-kiro-warning text-lg leading-none flex-shrink-0 focus:outline-none focus:ring-2 focus:ring-kiro-accent-500 rounded"
    >
      ×
    </button>
  </div>
{/if}
```

- [ ] **Step 8.2: Verify it typechecks**

```bash
npm run check
```

Expected: 0 errors. (Component isn't consumed yet but must compile standalone.)

- [ ] **Step 8.3: Commit**

```bash
git add crates/kiro-control-center/src/lib/components/BannerStack.svelte
git commit -m "feat(ui): extract BannerStack component from BrowseTab"
```

---

## Task 9: BrowseTab — adopt BannerStack (no behavior change)

Replace the inline banner render block (current `BrowseTab.svelte:1066-1132`) with a `<BannerStack>` invocation. Verifies the extraction is faithful before we layer Update-related behavior on top.

**Files:**
- Modify: `crates/kiro-control-center/src/lib/components/BrowseTab.svelte`

- [ ] **Step 9.1: Add import**

In `crates/kiro-control-center/src/lib/components/BrowseTab.svelte`, add to the imports near the top of the `<script>` block (alongside the existing `import SkillCard from "./SkillCard.svelte";` / `import PluginCard from "./PluginCard.svelte";`):

```svelte
  import BannerStack from "./BannerStack.svelte";
```

- [ ] **Step 9.2: Replace the inline banner block**

In `BrowseTab.svelte`, locate the existing render block from the comment "Banners render newest-first..." through the closing `{#if installWarning}` block (currently lines ~1066-1132 — search for the `data-testid="fetch-error"` and `data-testid="install-warning"` markers).

Replace that entire block with a single component invocation:

```svelte
  <BannerStack
    errors={fetchErrors}
    message={installMessage}
    warning={installWarning}
    fatalError={installError}
    errLabel={errLabel}
    ondismiss={(key) => fetchErrors.delete(key)}
    onmessageDismiss={() => (installMessage = null)}
    onwarningDismiss={() => (installWarning = null)}
    onfatalErrorDismiss={() => (installError = null)}
  />
```

(No `as ErrorSource` cast on `ondismiss` — Svelte 5's `generics="K extends string"` on `BannerStack` propagates the `ErrorSource` type through the callback.)

- [ ] **Step 9.3: Run typecheck**

```bash
npm run check
```

Expected: 0 errors.

- [ ] **Step 9.4: Manual smoke — banners still render**

```bash
npm run dev
```

Open `http://localhost:1420`. Verify the existing banner UX still works (e.g. add a marketplace, see success banner; dismiss it). Then close the dev server.

- [ ] **Step 9.5: Run unit tests**

```bash
npm run test:unit
```

Expected: still 22 passing — no new tests in this task.

- [ ] **Step 9.6: Commit**

```bash
git add crates/kiro-control-center/src/lib/components/BrowseTab.svelte
git commit -m "refactor(browse): consume BannerStack component"
```

---

## Task 10: PluginCard — Update + failure states

Add the two new props (`update`, `failure`) and the new `onUpdate` callback. Implement the action-area state machine per the design (`design.md:205-218`). The card stays usable for callers that don't pass the new props (BrowseTab is the only caller and will pass them in Task 11).

**Files:**
- Modify: `crates/kiro-control-center/src/lib/components/PluginCard.svelte`

- [ ] **Step 10.1: Update the script block**

Replace the entirety of `crates/kiro-control-center/src/lib/components/PluginCard.svelte`'s `<script>` block (current lines 1-30) with:

```svelte
<script lang="ts">
  import type {
    PluginInfo,
    PluginUpdateFailure,
    PluginUpdateInfo,
  } from "$lib/bindings";
  import { kindLabel } from "$lib/stores/plugin-updates";
  import { skillCountLabel, skillCountTitle } from "$lib/format";

  type Props = {
    plugin: PluginInfo;
    marketplace: string;
    installed: boolean;
    installing: boolean;
    updating: boolean;
    update: PluginUpdateInfo | undefined;
    failure: PluginUpdateFailure | undefined;
    projectPicked: boolean;
    onInstall: () => void;
    onUpdate: () => void;
  };

  let {
    plugin,
    marketplace,
    installed,
    installing,
    updating,
    update,
    failure,
    projectPicked,
    onInstall,
    onUpdate,
  }: Props = $props();

  const installTitle = $derived(
    !projectPicked
      ? "Pick a project first"
      : installed
        ? `${plugin.name} is already installed in this project`
        : `Install ${plugin.name} (skills + steering + agents) into the active project`,
  );

  // Update button label per Phase 2b design decision #6:
  //   - VersionBumped + both versions known      → "Update → vN"
  //   - VersionBumped + installed_version null   → "Update → vN" (legacy install; → reads as "to vN")
  //   - VersionBumped + available_version null   → "Update"      (manifest declares no version)
  //   - ContentChanged                            → "Update (content changed)"
  const updateLabel = $derived.by(() => {
    if (!update) return "Update";
    if (update.change_signal.kind === "content_changed") return "Update (content changed)";
    if (update.available_version) return `Update → v${update.available_version}`;
    return "Update";
  });
</script>
```

- [ ] **Step 10.2: Update the action-area markup**

Replace the existing action-area block (current lines ~57-80, the `<div class="flex flex-col items-end gap-1.5 flex-shrink-0">...</div>` block at the right side of the card) with:

```svelte
  <div class="flex flex-col items-end gap-1.5 flex-shrink-0">
    {#if installing}
      <button
        type="button"
        disabled
        aria-busy="true"
        aria-label="Installing {plugin.name}"
        class="px-3 py-1.5 text-xs font-medium rounded-md bg-kiro-muted text-kiro-subtle border border-transparent cursor-not-allowed"
      >
        Installing…
      </button>
    {:else if updating}
      <button
        type="button"
        disabled
        aria-busy="true"
        aria-label="Updating {plugin.name}"
        class="px-3 py-1.5 text-xs font-medium rounded-md bg-kiro-muted text-kiro-subtle border border-transparent cursor-not-allowed"
      >
        Updating…
      </button>
    {:else if failure && installed}
      <span
        class="px-2 py-0.5 text-[11px] font-medium text-kiro-error border border-kiro-error/40 rounded"
        title={kindLabel(failure.kind)}
      >
        Update check failed
      </span>
    {:else if update}
      <button
        type="button"
        onclick={onUpdate}
        disabled={!projectPicked}
        title="Update will replace local edits to plugin files"
        aria-label="Update {plugin.name}"
        class="px-3 py-1.5 text-xs font-medium rounded-md bg-kiro-warning/10 border border-kiro-warning/40 text-kiro-warning hover:bg-kiro-warning/15 transition-colors"
      >
        {updateLabel}
      </button>
    {:else if installed}
      <span
        class="px-2 py-0.5 text-[11px] font-medium text-kiro-success border border-kiro-success/40 rounded"
      >
        Installed
      </span>
    {:else}
      <button
        type="button"
        onclick={onInstall}
        disabled={!projectPicked}
        aria-busy={installing}
        title={installTitle}
        aria-label="Install {plugin.name}"
        class="px-3 py-1.5 text-xs font-medium rounded-md transition-colors
          {projectPicked
            ? 'bg-kiro-overlay border border-kiro-muted text-kiro-accent-300 hover:bg-kiro-muted hover:text-kiro-accent-200'
            : 'bg-kiro-muted text-kiro-subtle border border-transparent cursor-not-allowed'}"
      >
        Install
      </button>
    {/if}
  </div>
```

- [ ] **Step 10.3: Run typecheck**

```bash
npm run check
```

Expected: errors — BrowseTab still calls PluginCard with the old prop set. That's expected; Task 11 fixes BrowseTab. Don't commit yet.

- [ ] **Step 10.4: Note the breakage and proceed**

The `npm run check` failure here is expected and isolated (only BrowseTab's two-place PluginCard invocation is affected). Task 11 closes the loop. Skip the typecheck pass-requirement for this task only.

- [ ] **Step 10.5: Commit**

```bash
git add crates/kiro-control-center/src/lib/components/PluginCard.svelte
git commit -m "feat(ui): add Update / failure / updating states to PluginCard"
```

---

## Task 11: BrowseTab — wire pluginUpdates store + Update flow + new PluginCard props

The big BrowseTab change. Refactors the per-plugin in-flight tracker from `pendingPluginInstalls: SvelteSet<string>` to `pendingPluginActions: SvelteMap<string, "install" | "update">`, consumes `pluginUpdates`, projects `failureGroups` into `fetchErrors`, surfaces `pluginUpdates.fetchError`, and adds the `updatePlugin` handler. Closes the typecheck breakage from Task 10.

**Files:**
- Modify: `crates/kiro-control-center/src/lib/components/BrowseTab.svelte`

- [ ] **Step 11.1: Add new imports**

Add to the existing imports block at the top of `BrowseTab.svelte`'s `<script>`:

```svelte
  import { pluginUpdates } from "$lib/stores/plugin-updates.svelte";
  import { formatInstallPluginResult } from "$lib/format";
```

- [ ] **Step 11.2: Extend `ErrorSource`**

Find the existing `ErrorSource` literal-union type (currently around `BrowseTab.svelte:46-55`). Extend it with the two new key families:

```diff
   const ERR_INSTALLED_PLUGINS = "installed-plugins" as const;
+  const ERR_UPDATE_FETCH = "update-fetch" as const;
+  const UPDATE_CHECK_PREFIX = "update-check" as const;
   type ErrorSource =
     | typeof ERR_MARKETPLACES
     | typeof ERR_INSTALLED_PLUGINS
+    | typeof ERR_UPDATE_FETCH
+    | `${typeof UPDATE_CHECK_PREFIX}${string}${string}`
     | `${typeof PLUGINS_ERR_PREFIX}${string}`
     | `${typeof SKILLS_ERR_PREFIX}${string}${typeof DELIM}${string}`
     | `${typeof BULK_SKILLS_ERR_PREFIX}${string}`;
```

- [ ] **Step 11.3: Extend `errLabel` for the new keys**

Find the existing `errLabel` function (currently around `BrowseTab.svelte:64-75`). Add branches for the two new keys before the existing `if (key.startsWith(PLUGINS_ERR_PREFIX))` branch:

```ts
  function errLabel(key: ErrorSource): string {
    if (key === ERR_MARKETPLACES) return "Dismiss marketplaces error";
    if (key === ERR_INSTALLED_PLUGINS) return "Dismiss installed-plugins error";
    if (key === ERR_UPDATE_FETCH) return "Dismiss update-check error";
    if (key.startsWith(UPDATE_CHECK_PREFIX)) {
      const rest = key.slice(UPDATE_CHECK_PREFIX.length);
      const sepIdx = rest.indexOf("");
      const marketplace = sepIdx >= 0 ? rest.slice(sepIdx + 1) : rest;
      return `Dismiss update-check banner for ${marketplace}`;
    }
    if (key.startsWith(PLUGINS_ERR_PREFIX)) {
      return `Dismiss error for ${key.slice(PLUGINS_ERR_PREFIX.length)}`;
    }
    if (key.startsWith(BULK_SKILLS_ERR_PREFIX)) {
      return `Dismiss error for ${key.slice(BULK_SKILLS_ERR_PREFIX.length)}`;
    }
    const { marketplace, plugin } = parsePluginKey(key.slice(SKILLS_ERR_PREFIX.length));
    return `Dismiss error for ${marketplace}/${plugin}`;
  }
```

- [ ] **Step 11.4: Replace `pendingPluginInstalls` with `pendingPluginActions`**

Find the existing declaration (currently around `BrowseTab.svelte:94`):

Add to the existing `<script>` imports:

```svelte
  import type { PluginAction } from "$lib/stores/plugin-updates";
```

Then update the in-flight tracker declaration:

```diff
-  let pendingPluginInstalls = new SvelteSet<string>();
+  // Per-plugin in-flight tracker keyed by pluginKey(marketplace, plugin).
+  // Narrows the shared PluginAction union to the actions BrowseTab actually
+  // performs (Install + Update — never Remove, that's InstalledTab's surface).
+  let pendingPluginActions = new SvelteMap<string, Extract<PluginAction, "install" | "update">>();
```

Now find every site reading `pendingPluginInstalls`:

- The `installWholePlugin` body (currently `BrowseTab.svelte:734-852`): replace `pendingPluginInstalls.has(key)` / `pendingPluginInstalls.add(key)` / `pendingPluginInstalls.delete(key)` with `pendingPluginActions.has(key)` / `pendingPluginActions.set(key, "install")` / `pendingPluginActions.delete(key)`.
- The `installing` prop passed to `PluginCard` (currently in the `{#each availablePlugins}` block around `BrowseTab.svelte:1215-1226`): replace `installing={pendingPluginInstalls.has(key)}` with `installing={pendingPluginActions.get(key) === "install"}`.

(Cargo for the second point: in Task 11.7 below the same `{#each}` block gains the new props.)

- [ ] **Step 11.5: Add `updatePlugin` handler**

Append a new function below `installWholePlugin` in `BrowseTab.svelte`'s `<script>` (right after the closing brace of `installWholePlugin`):

```ts
  // Update an installed plugin. Calls the same `installPlugin` command
  // used by Install but with `force=true` hard-coded — the existing
  // global `forceInstall` checkbox stays bound to the Install button
  // path. `pluginUpdates.refresh` re-runs the scan after the update so
  // the indicator clears; `fetchInstalledPlugins` refreshes the
  // installed-set so any new content type the update added shows up.
  async function updatePlugin(marketplace: string, plugin: string) {
    const key = pluginKey(marketplace, plugin);
    if (pendingPluginActions.has(key)) return;
    pendingPluginActions.set(key, "update");
    installError = null;
    installMessage = null;
    installWarning = null;
    try {
      const result = await commands.installPlugin(
        marketplace,
        plugin,
        /*force=*/ true,
        /*acceptMcp=*/ false,
        projectPath,
      );
      if (result.status === "ok") {
        const { summary, warnings, anyInstalled, anyFailed } =
          formatInstallPluginResult(result.data, plugin);
        if (anyFailed && !anyInstalled) {
          installError = `Update failed for ${plugin}: ${summary}`;
        } else {
          installMessage = `Updated ${plugin}: ${summary}`;
        }
        if (warnings) {
          installWarning = `Updated ${plugin}: ${warnings}`;
        }
        await pluginUpdates.refresh(projectPath);
        await fetchInstalledPlugins();
      } else {
        installError = `Update failed for ${plugin}: ${result.error.message}`;
      }
    } catch (e) {
      const reason = e instanceof Error ? e.message : String(e);
      installError = `Update failed for ${plugin}: ${reason}`;
    } finally {
      pendingPluginActions.delete(key);
    }
  }
```

- [ ] **Step 11.6: Wire pluginUpdates.refresh to projectPath changes**

Find the existing `$effect` block that fans out on `projectPath` changes (around `BrowseTab.svelte:433-437`, `void projectPath; fetchInstalledPlugins();`). Add a sibling `$effect`:

```ts
  // Phase 2b: re-run update-scan whenever projectPath changes. The
  // store tracks `lastProjectPath` itself so a no-op call (same path
  // repeatedly) is harmless. Re-runs after a marketplace fetch are
  // triggered by `+page.svelte` via `MarketplacesTab.onUpdated`, not
  // here.
  $effect(() => {
    void projectPath;
    pluginUpdates.refresh(projectPath);
  });
```

- [ ] **Step 11.7: Project failureGroups + fetchError into fetchErrors**

Add a sibling `$effect` to the same script block (after the one above):

```ts
  // Project per-marketplace failure groups into the fetchErrors banner
  // map. Re-runs whenever pluginUpdates.failureGroups changes; clears
  // any previously-projected `update-check<...>` keys not in the
  // new group set so a recovered marketplace's banner disappears.
  $effect(() => {
    const seen = new Set<ErrorSource>();
    for (const group of pluginUpdates.failureGroups) {
      const key: ErrorSource =
        `${UPDATE_CHECK_PREFIX}${group.remediation}${group.marketplace}` as ErrorSource;
      seen.add(key);
      const noun = group.plugins.length === 1 ? "plugin" : "plugins";
      const list = group.plugins.join(", ");
      fetchErrors.set(
        key,
        `${group.plugins.length} ${noun} from ${group.marketplace}: ${group.remediationHint} (${list})`,
      );
    }
    for (const k of fetchErrors.keys()) {
      if (k.startsWith(UPDATE_CHECK_PREFIX) && !seen.has(k)) {
        fetchErrors.delete(k);
      }
    }
  });

  // Surface the toplevel pluginUpdates.fetchError as its own banner.
  // Distinct from the per-group keys above: this is "scan didn't run
  // at all" (couldn't read tracking files), not "scan ran, some
  // plugins failed".
  $effect(() => {
    if (pluginUpdates.fetchError) {
      fetchErrors.set(
        ERR_UPDATE_FETCH,
        `Couldn't check for updates: ${pluginUpdates.fetchError}`,
      );
    } else {
      fetchErrors.delete(ERR_UPDATE_FETCH);
    }
  });
```

- [ ] **Step 11.8: Pass new props to PluginCard**

Find the `<PluginCard ...>` invocation inside the `{#each availablePlugins}` block (around `BrowseTab.svelte:1215-1226`). Replace it with:

```svelte
            <PluginCard
              plugin={ap.plugin}
              marketplace={ap.marketplace}
              installed={installedPluginKeys.has(key)}
              installing={pendingPluginActions.get(key) === "install"}
              updating={pendingPluginActions.get(key) === "update"}
              update={pluginUpdates.updateFor(ap.marketplace, ap.plugin.name)}
              failure={pluginUpdates.failureFor(ap.marketplace, ap.plugin.name)}
              projectPicked={!!projectPath}
              onInstall={() => installWholePlugin(ap.marketplace, ap.plugin.name)}
              onUpdate={() => updatePlugin(ap.marketplace, ap.plugin.name)}
            />
```

- [ ] **Step 11.9: Run typecheck**

```bash
npm run check
```

Expected: 0 errors. (Closes the breakage from Task 10.)

- [ ] **Step 11.10: Run unit tests**

```bash
npm run test:unit
```

Expected: 22 still passing.

- [ ] **Step 11.11: Manual smoke**

```bash
npm run dev
```

Open `http://localhost:1420`. Navigate to Browse. Verify cards still render, Install still works on a non-installed plugin (if the dev environment has a marketplace fixture), and no errors in the browser console. Then close the dev server.

- [ ] **Step 11.12: Commit**

```bash
git add crates/kiro-control-center/src/lib/components/BrowseTab.svelte
git commit -m "feat(browse): wire pluginUpdates store + Update button + failure banners"
```

---

## Task 12: InstalledTab — adopt BannerStack + pendingPluginActions

Mirror BrowseTab's banner channels in InstalledTab. Replace `removingKey: string | null` with `pendingPluginActions: SvelteMap<string, "remove" | "update">`. Adds `installError | installMessage | installWarning` channels; the existing `loadError` narrows to fetch/refresh failures.

**Files:**
- Modify: `crates/kiro-control-center/src/lib/components/InstalledTab.svelte`

- [ ] **Step 12.1: Update the script block scaffolding**

Replace the existing `<script>` block opener and state declarations in `crates/kiro-control-center/src/lib/components/InstalledTab.svelte` (currently lines 1-25). Replace:

```svelte
<script lang="ts">
  import { onMount } from "svelte";
  import { commands } from "$lib/bindings";
  import type {
    InstalledSkillInfo,
    InstalledPluginInfo,
  } from "$lib/bindings";
  import { pluginKey } from "$lib/keys";

  let { projectPath }: { projectPath: string } = $props();

  let plugins: InstalledPluginInfo[] = $state([]);
  let skills: InstalledSkillInfo[] = $state([]);
  let loading: boolean = $state(true);
  let loadError: string | null = $state(null);
  // Non-fatal partial-load detail (one or more `installed-*.json` tracking
  // files failed to parse; the others loaded). Distinct from `loadError`
  // (red, fatal) — the table still has rows from the files that DID load.
  let loadWarning: string | null = $state(null);
  // Single removal in flight at a time — `remove_plugin` reads/writes the
  // installed-skills/steering/agents tracking files, so racing two removes
  // could clobber each other. Disabling all Remove buttons while one is
  // pending is the simplest correctness-preserving UI.
  let removingKey: string | null = $state(null);
```

with:

```svelte
<script lang="ts">
  import { onMount } from "svelte";
  import { SvelteMap } from "svelte/reactivity";
  import { commands } from "$lib/bindings";
  import type {
    InstalledSkillInfo,
    InstalledPluginInfo,
    PluginUpdateInfo,
    PluginUpdateFailure,
    RemovePluginResult,
  } from "$lib/bindings";
  import { pluginKey } from "$lib/keys";
  import { pluginUpdates } from "$lib/stores/plugin-updates.svelte";
  import { kindLabel } from "$lib/stores/plugin-updates";
  import type { PluginAction } from "$lib/stores/plugin-updates";
  import { formatInstallPluginResult, formatRemovePluginResult } from "$lib/format";
  import BannerStack from "./BannerStack.svelte";

  let { projectPath }: { projectPath: string } = $props();

  let plugins: InstalledPluginInfo[] = $state([]);
  let skills: InstalledSkillInfo[] = $state([]);
  let loading: boolean = $state(true);
  // `loadError` narrows in 2b: only fetch/refresh failures land here.
  // Remove-action failures route to `installError` (matching BrowseTab).
  let loadError: string | null = $state(null);

  // 3-banner pattern mirrored from BrowseTab. installError = red fatal,
  // installMessage = green success, installWarning = amber non-fatal.
  let installError: string | null = $state(null);
  let installMessage: string | null = $state(null);
  let installWarning: string | null = $state(null);

  // Per-plugin in-flight tracker, keyed by pluginKey(marketplace, plugin).
  // Narrows the shared PluginAction union to the actions InstalledTab
  // performs (Remove + Update — never Install, that's BrowseTab's surface).
  let pendingPluginActions = new SvelteMap<string, Extract<PluginAction, "remove" | "update">>();

  // The most recent RemovePluginResult — drives the inline <details>
  // block below the BannerStack. Stays set until the next Remove or
  // a project change clears it.
  let removeResult: RemovePluginResult | null = $state(null);
  let removeResultPlugin: string | null = $state(null);
  let removeResultHasFailures: boolean = $state(false);

  // Banner-stack typing — InstalledTab's ErrorSource union is small for
  // 2b: only the new update-check + update-fetch keys plus a single
  // installed-plugins key for partial-load warnings (existing).
  const ERR_INSTALLED_PLUGINS = "installed-plugins" as const;
  const ERR_UPDATE_FETCH = "update-fetch" as const;
  const UPDATE_CHECK_PREFIX = "update-check" as const;
  type ErrorSource =
    | typeof ERR_INSTALLED_PLUGINS
    | typeof ERR_UPDATE_FETCH
    | `${typeof UPDATE_CHECK_PREFIX}${string}${string}`;
  let fetchErrors = new SvelteMap<ErrorSource, string>();

  function errLabel(key: ErrorSource): string {
    if (key === ERR_INSTALLED_PLUGINS) return "Dismiss installed-plugins warning";
    if (key === ERR_UPDATE_FETCH) return "Dismiss update-check error";
    if (key.startsWith(UPDATE_CHECK_PREFIX)) {
      const rest = key.slice(UPDATE_CHECK_PREFIX.length);
      const sepIdx = rest.indexOf("");
      const marketplace = sepIdx >= 0 ? rest.slice(sepIdx + 1) : rest;
      return `Dismiss update-check banner for ${marketplace}`;
    }
    return "Dismiss banner";
  }
```

- [ ] **Step 12.2: Update the `removePlugin` handler**

Replace the existing `removePlugin` function (current lines 64-81) with:

```ts
  async function removePlugin(marketplace: string, plugin: string) {
    const key = pluginKey(marketplace, plugin);
    if (pendingPluginActions.has(key)) return;
    pendingPluginActions.set(key, "remove");
    installError = null;
    installMessage = null;
    installWarning = null;
    removeResult = null;
    removeResultPlugin = null;
    removeResultHasFailures = false;
    try {
      const result = await commands.removePlugin(marketplace, plugin, projectPath);
      if (result.status === "ok") {
        const { summary, hasItems, hasFailures } =
          formatRemovePluginResult(result.data, plugin);
        if (hasItems || hasFailures) {
          removeResult = result.data;
          removeResultPlugin = plugin;
          removeResultHasFailures = hasFailures;
        }
        if (hasFailures) {
          installWarning = `Removed plugin ${plugin}: ${summary}`;
        } else {
          installMessage = `Removed plugin ${plugin}: ${summary}`;
        }
        // Order: pluginUpdates.refresh first, local refresh second.
        // Matches BrowseTab.updatePlugin (Step 11.5) and InstalledTab
        // .updatePlugin (Step 12.3) — uniform across all action handlers.
        await pluginUpdates.refresh(projectPath);
        await refresh();
      } else {
        installError = `Remove failed for ${plugin}: ${result.error.message}`;
      }
    } catch (e) {
      const reason = e instanceof Error ? e.message : String(e);
      installError = `Remove failed for ${plugin}: ${reason}`;
    } finally {
      pendingPluginActions.delete(key);
    }
  }
```

- [ ] **Step 12.3: Add the `updatePlugin` handler**

Append to the `<script>` block (right after `removePlugin`'s closing brace):

```ts
  async function updatePlugin(marketplace: string, plugin: string) {
    const key = pluginKey(marketplace, plugin);
    if (pendingPluginActions.has(key)) return;
    pendingPluginActions.set(key, "update");
    installError = null;
    installMessage = null;
    installWarning = null;
    removeResult = null;
    try {
      const result = await commands.installPlugin(
        marketplace,
        plugin,
        /*force=*/ true,
        /*acceptMcp=*/ false,
        projectPath,
      );
      if (result.status === "ok") {
        const { summary, warnings, anyInstalled, anyFailed } =
          formatInstallPluginResult(result.data, plugin);
        if (anyFailed && !anyInstalled) {
          installError = `Update failed for ${plugin}: ${summary}`;
        } else {
          installMessage = `Updated ${plugin}: ${summary}`;
        }
        if (warnings) {
          installWarning = `Updated ${plugin}: ${warnings}`;
        }
        await pluginUpdates.refresh(projectPath);
        await refresh();
      } else {
        installError = `Update failed for ${plugin}: ${result.error.message}`;
      }
    } catch (e) {
      const reason = e instanceof Error ? e.message : String(e);
      installError = `Update failed for ${plugin}: ${reason}`;
    } finally {
      pendingPluginActions.delete(key);
    }
  }
```

- [ ] **Step 12.4: Update the existing `refresh` function — narrow loadError, route partial-load to fetchErrors**

Replace the existing `refresh()` body (current lines 26-62) with:

```ts
  async function refresh() {
    loading = true;
    loadError = null;
    try {
      const [pluginsResult, skillsResult] = await Promise.all([
        commands.listInstalledPlugins(projectPath),
        commands.listInstalledSkills(projectPath),
      ]);
      if (pluginsResult.status === "ok") {
        plugins = pluginsResult.data.plugins;
        const warnings = pluginsResult.data.partial_load_warnings ?? [];
        if (warnings.length > 0) {
          const summary = warnings
            .map((w) => `${w.tracking_file}: ${w.error}`)
            .join("; ");
          fetchErrors.set(
            ERR_INSTALLED_PLUGINS,
            `Installed plugins partially loaded — ${summary}`,
          );
        } else {
          fetchErrors.delete(ERR_INSTALLED_PLUGINS);
        }
      } else {
        loadError = pluginsResult.error.message;
      }
      if (skillsResult.status === "ok") {
        skills = skillsResult.data;
      } else if (loadError === null) {
        loadError = skillsResult.error.message;
      }
    } catch (e) {
      const reason = e instanceof Error ? e.message : String(e);
      loadError = `Failed to load installed state: ${reason}`;
    } finally {
      loading = false;
    }
  }
```

(Note: `partial_load_warnings` previously drove a standalone `loadWarning` banner; Step 12.1 already excluded that state from the new script block, and the new `refresh()` routes the warning through `fetchErrors[ERR_INSTALLED_PLUGINS]` for symmetry with BrowseTab.)

- [ ] **Step 12.5: Add the pluginUpdates wiring effects**

Append to the `<script>` block (after the existing `$effect` and `onMount(refresh)` calls — preserving them):

```ts
  // Eager scan on project mount + on every projectPath change.
  $effect(() => {
    void projectPath;
    pluginUpdates.refresh(projectPath);
  });

  // Project per-marketplace failure groups into the fetchErrors map.
  $effect(() => {
    const seen = new Set<ErrorSource>();
    for (const group of pluginUpdates.failureGroups) {
      const key: ErrorSource =
        `${UPDATE_CHECK_PREFIX}${group.remediation}${group.marketplace}` as ErrorSource;
      seen.add(key);
      const noun = group.plugins.length === 1 ? "plugin" : "plugins";
      const list = group.plugins.join(", ");
      fetchErrors.set(
        key,
        `${group.plugins.length} ${noun} from ${group.marketplace}: ${group.remediationHint} (${list})`,
      );
    }
    for (const k of fetchErrors.keys()) {
      if (k.startsWith(UPDATE_CHECK_PREFIX) && !seen.has(k)) {
        fetchErrors.delete(k);
      }
    }
  });

  // Surface the toplevel pluginUpdates.fetchError as its own banner.
  $effect(() => {
    if (pluginUpdates.fetchError) {
      fetchErrors.set(
        ERR_UPDATE_FETCH,
        `Couldn't check for updates: ${pluginUpdates.fetchError}`,
      );
    } else {
      fetchErrors.delete(ERR_UPDATE_FETCH);
    }
  });

  // Helpers used by the table render block.
  function statusUpdateLabel(u: PluginUpdateInfo): string {
    // ContentChanged: phrased as a status (full sentence) rather than the
    // PluginCard button's action label ("Update (content changed)").
    // The two surfaces intentionally differ — column = state; button = action.
    if (u.change_signal.kind === "content_changed") return "Content changed since install";
    // VersionBumped: prefer the explicit "v_old → v_new" form when both
    // versions are known; fall back to "vN available" for legacy installs
    // (installed_version: None) and to a bare "Update available" when the
    // marketplace manifest declares no version.
    if (u.installed_version && u.available_version) {
      return `v${u.installed_version} → v${u.available_version}`;
    }
    if (u.available_version) return `v${u.available_version} available`;
    return "Update available";
  }

  function updateInfoFor(p: InstalledPluginInfo): PluginUpdateInfo | undefined {
    return pluginUpdates.updateFor(p.marketplace, p.plugin);
  }

  function failureFor(p: InstalledPluginInfo): PluginUpdateFailure | undefined {
    return pluginUpdates.failureFor(p.marketplace, p.plugin);
  }
```

- [ ] **Step 12.6: Verify `loadWarning` is fully gone**

Step 12.1's `<script>` replacement block already excluded `let loadWarning`; Step 12.4's refresh body no longer touches it. This step is a no-op verification:

```bash
grep -n loadWarning crates/kiro-control-center/src/lib/components/InstalledTab.svelte
```

Expected: no matches. If any references remain, delete them — they're dead.

In the markup (replaced wholesale in Task 13), the standalone `data-testid="installed-load-warning"` block is also gone — Task 13's full-markup replacement supersedes it.

- [ ] **Step 12.7: Run typecheck**

```bash
npm run check
```

Expected: 0 errors. (Markup updates land in Task 13; the script changes alone should typecheck because removeResult is read in the markup that's still about to change. Cargo: if `removeResult` triggers an unused-state warning, ignore — Task 13 reads it.)

If `npm run check` complains about unused declarations, ignore — they get consumed in Task 13.

- [ ] **Step 12.8: Commit**

```bash
git add crates/kiro-control-center/src/lib/components/InstalledTab.svelte
git commit -m "feat(installed): wire pluginUpdates + extend banner channels (script)"
```

---

## Task 13: InstalledTab — Status column, Update button, BannerStack, `<details>` toast

Mirror Task 12 in markup. Adds the Status column, the Update action button, the inline `<details>` block for `removeResult`, and replaces the legacy banner UI with `<BannerStack>`.

**Files:**
- Modify: `crates/kiro-control-center/src/lib/components/InstalledTab.svelte`

- [ ] **Step 13.1: Replace the entire `<div class="flex flex-col h-full">` template block**

Replace everything from `<div class="flex flex-col h-full">` to the end of the file (current lines 106-216) with:

```svelte
<div class="flex flex-col h-full">
  <BannerStack
    errors={fetchErrors}
    message={installMessage}
    warning={installWarning}
    fatalError={installError}
    errLabel={errLabel}
    ondismiss={(key) => fetchErrors.delete(key)}
    onmessageDismiss={() => (installMessage = null)}
    onwarningDismiss={() => (installWarning = null)}
    onfatalErrorDismiss={() => (installError = null)}
  />

  {#if removeResult && removeResultPlugin}
    <div
      class="mx-4 mt-3 px-4 py-3 rounded-md text-sm flex items-start gap-3
        {removeResultHasFailures
          ? 'bg-kiro-warning/10 border border-kiro-warning/30 text-kiro-warning'
          : 'bg-kiro-success/10 border border-kiro-success/30 text-kiro-success'}"
    >
      <details
        class="flex-1"
        open={removeResultHasFailures}
      >
        <summary class="cursor-pointer text-xs opacity-85">
          {removeResultHasFailures ? "Show items + failures" : "Show items"}
        </summary>
        <div class="mt-2 pl-3 border-l-2 border-current/40 text-xs space-y-1">
          {#if (removeResult.skills.removed ?? []).length > 0}
            <div><b>Skills removed:</b> {(removeResult.skills.removed ?? []).join(", ")}</div>
          {/if}
          {#if (removeResult.steering.removed ?? []).length > 0}
            <div><b>Steering removed:</b> {(removeResult.steering.removed ?? []).join(", ")}</div>
          {/if}
          {#if (removeResult.agents.removed ?? []).length > 0}
            <div><b>Agents removed:</b> {(removeResult.agents.removed ?? []).join(", ")}</div>
          {/if}
          {#each removeResult.skills.failures ?? [] as f (f.item)}
            <div><b>Skill failed:</b> {f.item} — {f.error}</div>
          {/each}
          {#each removeResult.steering.failures ?? [] as f (f.item)}
            <div><b>Steering failed:</b> {f.item} — {f.error}</div>
          {/each}
          {#each removeResult.agents.failures ?? [] as f (f.item)}
            <div><b>Agent failed:</b> {f.item} — {f.error}</div>
          {/each}
        </div>
      </details>
      <button
        type="button"
        onclick={() => { removeResult = null; removeResultPlugin = null; }}
        aria-label="Dismiss remove summary"
        class="opacity-70 hover:opacity-100 text-lg leading-none flex-shrink-0 focus:outline-none focus:ring-2 focus:ring-kiro-accent-500 rounded"
      >
        ×
      </button>
    </div>
  {/if}

  <div class="flex-1 overflow-y-auto p-4">
    {#if loading}
      <div class="flex flex-col items-center justify-center h-full text-kiro-subtle gap-3">
        <svg class="w-8 h-8 text-kiro-accent-800 animate-pulse" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="1.5"
            d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15" />
        </svg>
        <p class="text-sm">Loading installed state...</p>
      </div>
    {:else if loadError}
      <div class="px-4 py-3 rounded-md bg-kiro-error/10 border border-kiro-error/30">
        <p class="text-sm text-kiro-error">{loadError}</p>
      </div>
    {:else}
      <section class="mb-6">
        <h2 class="text-sm font-semibold text-kiro-text mb-3">Installed plugins</h2>
        {#if plugins.length === 0}
          <p class="text-sm text-kiro-subtle">No plugins installed in this project.</p>
        {:else}
          <table class="w-full text-sm">
            <thead>
              <tr class="text-left text-[11px] uppercase tracking-wider text-kiro-subtle border-b border-kiro-muted">
                <th class="px-4 py-2">Plugin</th>
                <th class="px-4 py-2">Marketplace</th>
                <th class="px-4 py-2">Version</th>
                <th class="px-4 py-2">Status</th>
                <th class="px-4 py-2">Contents</th>
                <th class="px-4 py-2">Installed at</th>
                <th class="px-4 py-2"></th>
              </tr>
            </thead>
            <tbody>
              {#each plugins as p (pluginKey(p.marketplace, p.plugin))}
                {@const key = pluginKey(p.marketplace, p.plugin)}
                {@const updateInfo = updateInfoFor(p)}
                {@const failure = failureFor(p)}
                {@const action = pendingPluginActions.get(key)}
                <tr class="border-b border-kiro-muted/50">
                  <td class="px-4 py-3 font-medium text-kiro-text">{p.plugin}</td>
                  <td class="px-4 py-3 text-kiro-text-secondary">{p.marketplace}</td>
                  <td class="px-4 py-3 text-kiro-text-secondary">{p.installed_version ?? "—"}</td>
                  <td class="px-4 py-3">
                    {#if updateInfo}
                      <span
                        class="px-2 py-0.5 text-[11px] font-medium text-kiro-warning border border-kiro-warning/40 rounded"
                      >
                        {statusUpdateLabel(updateInfo)}
                      </span>
                    {:else if failure}
                      <span
                        class="px-2 py-0.5 text-[11px] font-medium text-kiro-error border border-kiro-error/40 rounded"
                        title={kindLabel(failure.kind)}
                      >
                        Update check failed
                      </span>
                    {:else}
                      <span class="text-kiro-success text-[11px]">Up to date</span>
                    {/if}
                  </td>
                  <td class="px-4 py-3 text-kiro-text-secondary">{contentSummary(p)}</td>
                  <td class="px-4 py-3 text-kiro-text-secondary">{formatDate(p.latest_install)}</td>
                  <td class="px-4 py-3 text-right">
                    <div class="inline-flex gap-2">
                      {#if updateInfo}
                        <button
                          type="button"
                          onclick={() => updatePlugin(p.marketplace, p.plugin)}
                          disabled={action !== undefined}
                          aria-busy={action === "update"}
                          title="Update will replace local edits to plugin files"
                          class="px-2 py-0.5 text-[11px] text-kiro-warning hover:text-kiro-warning/80 disabled:cursor-not-allowed disabled:opacity-50"
                        >
                          {action === "update" ? "Updating…" : "Update"}
                        </button>
                      {/if}
                      <button
                        type="button"
                        onclick={() => removePlugin(p.marketplace, p.plugin)}
                        disabled={action !== undefined}
                        aria-busy={action === "remove"}
                        class="px-2 py-0.5 text-[11px] text-kiro-subtle hover:text-kiro-error disabled:cursor-not-allowed disabled:opacity-50"
                      >
                        {action === "remove" ? "Removing…" : "Remove"}
                      </button>
                    </div>
                  </td>
                </tr>
              {/each}
            </tbody>
          </table>
        {/if}
      </section>

      <details class="mb-6">
        <summary class="cursor-pointer text-sm font-medium text-kiro-text-secondary hover:text-kiro-text">
          All installed skills (flat view)
        </summary>
        <div class="mt-3">
          {#if skills.length === 0}
            <p class="text-sm text-kiro-subtle">No skills installed.</p>
          {:else}
            <table class="w-full text-sm">
              <thead>
                <tr class="text-left text-[11px] uppercase tracking-wider text-kiro-subtle border-b border-kiro-muted">
                  <th class="px-4 py-2">Name</th>
                  <th class="px-4 py-2">Marketplace</th>
                  <th class="px-4 py-2">Plugin</th>
                  <th class="px-4 py-2">Version</th>
                  <th class="px-4 py-2">Installed</th>
                </tr>
              </thead>
              <tbody>
                {#each skills as skill (skill.name)}
                  <tr class="border-b border-kiro-muted/50">
                    <td class="px-4 py-3 text-kiro-text">{skill.name}</td>
                    <td class="px-4 py-3 text-kiro-text-secondary">{skill.marketplace}</td>
                    <td class="px-4 py-3 text-kiro-text-secondary">{skill.plugin}</td>
                    <td class="px-4 py-3 text-kiro-text-secondary">{skill.version ?? "—"}</td>
                    <td class="px-4 py-3 text-kiro-text-secondary">{formatDate(skill.installed_at)}</td>
                  </tr>
                {/each}
              </tbody>
            </table>
          {/if}
        </div>
      </details>
    {/if}
  </div>
</div>
```

(`formatDate` and `contentSummary` are existing functions in InstalledTab; the `<script>` block kept them intact in Task 12. The `loadWarning` block previously rendered standalone is gone — `partial_load_warnings` now route through `fetchErrors`.)

- [ ] **Step 13.2: Run typecheck**

```bash
npm run check
```

Expected: 0 errors.

- [ ] **Step 13.3: Run unit tests**

```bash
npm run test:unit
```

Expected: 22 still passing.

- [ ] **Step 13.4: Manual smoke**

```bash
npm run dev
```

Open `http://localhost:1420`. Navigate to Installed. Verify:
- The table renders Plugin / Marketplace / Version / **Status** / Contents / **Installed at** / actions columns.
- Status reads "Up to date" for plugins with no available update.
- Remove still works (will produce the new toast with `<details>`).

Then close the dev server.

- [ ] **Step 13.5: Commit**

```bash
git add crates/kiro-control-center/src/lib/components/InstalledTab.svelte
git commit -m "feat(installed): Status column + Update button + details toast"
```

---

## Task 14: MarketplacesTab — `onUpdated` callback prop

Wire a callback that fires after a successful `commands.updateMarketplace`. The parent (`+page.svelte`) calls `pluginUpdates.refresh(store.projectPath)` from this callback so the scan re-fires when marketplace cache content changes.

**Files:**
- Modify: `crates/kiro-control-center/src/lib/components/MarketplacesTab.svelte`

- [ ] **Step 14.1: Add the `onUpdated` callback prop**

Edit `crates/kiro-control-center/src/lib/components/MarketplacesTab.svelte`. Replace the top of the `<script>` block (currently lines 1-3):

```diff
 <script lang="ts">
   import { commands } from "$lib/bindings";
   import type { MarketplaceInfo, GitProtocol } from "$lib/bindings";
+
+  // Phase 2b: parent (`+page.svelte`) wires this to
+  // `pluginUpdates.refresh(store.projectPath)` so a successful marketplace
+  // update invalidates the cached update-detection scan.
+  type Props = {
+    onUpdated?: (marketplaceName: string) => void;
+  };
+  let { onUpdated }: Props = $props();
```

- [ ] **Step 14.2: Fire the callback on successful update**

Find the existing `updateMarketplace` function (lines 48-66). Append the callback fire inside the success branch:

```diff
     const result = await commands.updateMarketplace(name);
     if (result.status === "ok") {
       const { updated, failed, skipped } = result.data;
       const parts: string[] = [];
       if (updated.length > 0) parts.push(`Updated: ${updated.join(", ")}`);
       if (skipped.length > 0) parts.push(`Skipped: ${skipped.join(", ")}`);
       if (failed.length > 0) parts.push(`Failed: ${failed.map((f) => `${f.name} (${f.error})`).join(", ")}`);
       successMessage = parts.join(" | ");
       await loadMarketplaces();
+      onUpdated?.(name);
     } else {
       error = result.error.message;
     }
```

- [ ] **Step 14.3: Run typecheck**

```bash
npm run check
```

Expected: 0 errors. (`+page.svelte` doesn't pass the prop yet — that's Task 15. Optional callback prop means missing callers compile.)

- [ ] **Step 14.4: Commit**

```bash
git add crates/kiro-control-center/src/lib/components/MarketplacesTab.svelte
git commit -m "feat(marketplaces): add onUpdated callback prop"
```

---

## Task 15: `+page.svelte` — wire MarketplacesTab callback

Threads the callback to `pluginUpdates.refresh(store.projectPath)`.

**Files:**
- Modify: `crates/kiro-control-center/src/routes/+page.svelte`

- [ ] **Step 15.1: Add import + wire the prop**

Edit `crates/kiro-control-center/src/routes/+page.svelte`. Add the store import near the top of the `<script>` block (alongside the existing `import { store, initialize } from "$lib/stores/project.svelte";`):

```diff
   import { store, initialize } from "$lib/stores/project.svelte";
+  import { pluginUpdates } from "$lib/stores/plugin-updates.svelte";
```

Then find the `<MarketplacesTab />` mount (current line 106) and pass the new prop:

```diff
-        {:else if activeTab === "Marketplaces"}
-          <MarketplacesTab />
+        {:else if activeTab === "Marketplaces"}
+          <MarketplacesTab
+            onUpdated={() => {
+              if (store.projectPath) {
+                pluginUpdates.refresh(store.projectPath);
+              }
+            }}
+          />
```

- [ ] **Step 15.2: Run typecheck**

```bash
npm run check
```

Expected: 0 errors.

- [ ] **Step 15.3: Commit**

```bash
git add crates/kiro-control-center/src/routes/+page.svelte
git commit -m "feat(routes): refresh pluginUpdates after marketplace update"
```

---

## Task 16: CLAUDE.md — pre-commit gate update

Add `npm run test:unit` to the FE-tier pre-commit list and document the vitest scope boundary.

**Files:**
- Modify: `CLAUDE.md` (top-level — `/home/dwalleck/repos/kiro-marketplace-cli/CLAUDE.md`)

- [ ] **Step 16.1: Add to the Pre-commit section**

Edit `CLAUDE.md`. Find the `## Pre-commit` section (currently around line 21):

```diff
 ## Pre-commit
 Run all three before committing — CI enforces each:
 - `cargo fmt --all --check`
 - `cargo test --workspace`
 - `cargo clippy --workspace --tests -- -D warnings`
+
+For changes under `crates/kiro-control-center/` also run:
+- `cd crates/kiro-control-center && npm run check`
+- `cd crates/kiro-control-center && npm run test:unit`
+
+Vitest covers pure-logic helpers only (no jsdom, no `@testing-library/svelte`,
+no Tauri-IPC mocks). Component-level testing is intentionally future scope.
+If you find yourself wanting to test a `.svelte` file or a reactive store's
+`$state`/`$derived`, factor the testable logic out into a non-`.svelte.ts`
+module and test the helper instead.
```

- [ ] **Step 16.2: Verify CLAUDE.md still reads cleanly**

```bash
grep -A3 "## Pre-commit" CLAUDE.md
```

Expected: the new lines appear in the Pre-commit section, no broken markdown.

- [ ] **Step 16.3: Commit**

```bash
git add CLAUDE.md
git commit -m "docs(claude): add npm run test:unit to FE pre-commit list"
```

---

## Task 17: Full verification pass

Single sweep: every gate the project enforces.

- [ ] **Step 17.1: Run cargo fmt check**

```bash
cargo fmt --all --check
```

Expected: 0 changes (no Rust changes in 2b).

- [ ] **Step 17.2: Run cargo test**

```bash
cargo test --workspace
```

Expected: all passing (no Rust changes in 2b; this verifies 2a's tests still pass against the new bindings consumers).

- [ ] **Step 17.3: Run cargo clippy**

```bash
cargo clippy --workspace --tests -- -D warnings
```

Expected: 0 warnings.

- [ ] **Step 17.4: Run npm run check**

```bash
cd crates/kiro-control-center && npm run check
```

Expected: 0 errors.

- [ ] **Step 17.5: Run npm run test:unit**

```bash
cd crates/kiro-control-center && npm run test:unit
```

Expected: 22 passing (5 + 2 + 6 + 5 + 4 across remediationClass / kindLabel / groupFailures / formatInstallPluginResult / formatRemovePluginResult).

- [ ] **Step 17.6: Manual smoke — golden path**

```bash
cd crates/kiro-control-center && npm run dev
```

Open `http://localhost:1420`. Walk through:
1. Pick a project. Browse tab loads. Cards render existing "Installed" pills.
2. Switch to Installed tab. Table renders with the new Status column showing "Up to date" for installed plugins. Remove a plugin → toast appears with `<details>` containing item names.
3. (If a fixture marketplace with newer plugin versions is available) Verify the Update button appears on a card with an available update, click it, observe the success toast and the indicator clear.
4. (If able to simulate marketplace offline) Move/rename a marketplace cache dir under `~/.kiro/marketplaces/`, switch project (or click Update Marketplaces) → grouped amber banner reading "N plugins from {marketplace} couldn't be checked. Run `kiro-market update`...". Restore the cache dir → banner clears on next refresh.

Close the dev server when done.

- [ ] **Step 17.7: Run plan-lint to confirm no Rust lint regressions**

```bash
TETHYS_BIN=/home/dwalleck/repos/rivets/target/release/tethys cargo xtask plan-lint
```

Expected: 0 findings. (Phase 2b is FE-only so this is a no-op check, but the design references the lint and CI runs it.)

---

## Task 18: Playwright e2e — Update available + Remove with sub-results

Append two new test scenarios to the existing e2e suite. Both gate on `FIXTURE_MARKETPLACE_PATH` per the existing pattern at `tests/e2e/app.spec.ts`.

**Files:**
- Modify: `crates/kiro-control-center/tests/e2e/app.spec.ts`

- [ ] **Step 18.1: Add the new test cases**

Append to `crates/kiro-control-center/tests/e2e/app.spec.ts` (at the end of the file, before the final closing brace if present, or as a new top-level `test.describe` block):

```ts
test.describe("Phase 2b — update detection UI", () => {
  test("Status column appears on Installed tab", async ({ page }) => {
    await page.goto("/");

    const fixturePath = process.env.FIXTURE_MARKETPLACE_PATH;
    if (!fixturePath) {
      test.skip(true, "FIXTURE_MARKETPLACE_PATH not set");
      return;
    }

    await page.getByRole("button", { name: "Installed", exact: true }).click();

    // Status column appears in the header even when there are zero
    // installed plugins (its presence is a static UI guarantee).
    await expect(page.getByRole("columnheader", { name: "Status" })).toBeVisible();
    await expect(page.getByRole("columnheader", { name: "Installed at" })).toBeVisible();
  });
});
```

(The "Remove toast surfaces with collapsed details" scenario from the design's Playwright section is intentionally NOT shipped here — it requires a fixture pipeline that supports an install-then-remove sequence, which doesn't exist yet. Manual smoke at Task 17.6 covers it. When a fixture pipeline lands, replace nothing instead of replacing a `test.skip` stub.)

- [ ] **Step 18.2: Run e2e if fixture is set**

```bash
cd crates/kiro-control-center && npm run test:e2e
```

Expected: existing tests pass; new tests skip if `FIXTURE_MARKETPLACE_PATH` is unset (matching the existing `test.skip` pattern).

- [ ] **Step 18.3: Commit**

```bash
git add crates/kiro-control-center/tests/e2e/app.spec.ts
git commit -m "test(e2e): add Phase 2b update-detection UI scenarios"
```

---

## Task 19: Branch hygiene + PR

- [ ] **Step 19.1: Sanity-check the diff**

```bash
git log --oneline origin/main..HEAD
```

Expected: ~14 commits, one per Task (1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 18). The Task 17 verification pass intentionally produces no commit.

- [ ] **Step 19.2: One last full pre-commit pass**

```bash
cargo fmt --all --check && \
  cargo test --workspace && \
  cargo clippy --workspace --tests -- -D warnings && \
  cd crates/kiro-control-center && npm run check && npm run test:unit
```

Expected: every command exits 0.

- [ ] **Step 19.3: Open the PR**

```bash
git push -u origin <branch-name>
gh pr create --title "feat(ui): plugin update detection UI (Phase 2b)" --body "$(cat <<'EOF'
## Summary

Implements Phase 2b of the plugin update detection rollout — the FE
consumer for the wire format Phase 2a (PR #96) shipped.

- Update indicator + button on PluginCard (BrowseTab) and Status column
  + Update action on InstalledTab.
- New module-scoped `pluginUpdates` Svelte store; eager scan on project
  mount + after marketplace fetch.
- `RemovePluginResult` reshape consumed via inline `<details>` block on
  InstalledTab — the toast now names the items it removed.
- Failures group by `(remediationClass, marketplace)` — N stale-cache
  failures from one marketplace produce one banner, not N.
- Vitest setup (narrow scope): pure-logic helpers tested in `node` env;
  no jsdom, no `@testing-library/svelte`. Component-level testing
  intentionally future scope.

## Design + plan
- Design: `docs/plans/2026-05-05-phase-2b-update-detection-ui-design.md`
- Plan: `docs/plans/2026-05-05-phase-2b-update-detection-ui-plan.md`

## Test plan
- [x] `cargo fmt --all --check`
- [x] `cargo test --workspace`
- [x] `cargo clippy --workspace --tests -- -D warnings`
- [x] `cd crates/kiro-control-center && npm run check`
- [x] `cd crates/kiro-control-center && npm run test:unit` (22 tests)
- [x] Manual smoke per Task 17.6 (golden + edge cases)
- [ ] Playwright e2e with `FIXTURE_MARKETPLACE_PATH` set (Task 18 — fixture-dependent)

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

---

## Plan review — 6 gates self-check

Per `docs/plan-review-checklist.md`. Run before declaring this plan implementation-ready.

### Gate 1 — Grounding

- Every `bindings.ts` line reference verified against the current file (`bindings.ts:139`, `:160`, `:252-256`, `:657-664`, `:785-799`, `:1018-1023`, `:1040-1069`, `:1077-1100`, `:1125-1193`, `:1492-1506`, `:1523-1533`).
- `vite.config.js` (NOT `.ts`) — verified by `ls`.
- `MarketplacesTab.svelte:48-66` `updateMarketplace` success branch — verified.
- `+page.svelte:106` `<MarketplacesTab />` mount — verified.
- `BrowseTab.svelte:734-852` `installWholePlugin` body for the extraction in Task 5 — verified by direct read.
- `BrowseTab.svelte:1066-1132` banner block — verified during Task 8 plan write.
- `format.ts` existing helpers (`formatSkippedSkillsForPlugin`, `formatSteeringWarning`, `formatInstallWarning`) — verified.
- The `assertNever` exhaustiveness pattern in `format.ts` — observed and matched in the new helpers (no `default:` arm in `remediationClass` / `kindLabel`).
- `pluginKey` / `parsePluginKey` (`keys.ts`) — verified; reused throughout.
- Existing `pendingPluginInstalls: SvelteSet<string>` (BrowseTab) and `removingKey: string | null` (InstalledTab) — refactored to a unified `pendingPluginActions: SvelteMap<string, "install" | "update" | "remove">`.
- `InstallPluginResult_Serialize` (NOT bare `InstallPluginResult`) — verified to be the type returned by `installPlugin` per `bindings.ts:139`.

### Gate 2 — Threat Model

- No new untrusted byte sources — Phase 2b consumes typed values that `serde_json::from_slice` already validated on the backend (PR #96).
- Update button is **destructive** (force-installs over user-edited plugin files). Mitigation: `title=` tooltip "Update will replace local edits to plugin files" on every Update affordance (PluginCard action button + InstalledTab row button). This is explicit per design decision #3 — confirmation modals deferred.
- `PluginUpdateFailure.reason` strings are `error_full_chain` outputs from the backend. Rendered as text via Svelte's default escaping (no `{@html ...}` anywhere in 2b).
- `accept_mcp` stays hard-coded to `false` for Update calls — same security posture as Install. The "user opt-in to MCP servers" UX is out of scope per Phase 1.

### Gate 3 — Wire Format / FFI

- Zero new types crossing FFI. The plan introduces only TS-side types (`RemediationClass`, `FailureGroup`, `FormattedInstallPluginResult`, `FormattedRemovePluginResult`) — none derive `Serialize`/`specta::Type`.
- `FailureGroup.marketplace: MarketplaceName` and `.plugins: PluginName[]` thread the wire-format newtypes (TS aliases for `string` per `bindings.ts:905`, `:997`) without erasure into bare `string`.
- The grouping function's composite map key (`${cls}:${f.marketplace}`) uses a literal colon — safe because `RemediationClass` values are fixed and don't contain `:`, and marketplace names go through the backend `MarketplaceName::new` validator. Documented in the comment alongside the key construction (Task 4 step 4.3).

### Gate 4 — External Type Boundary

- N/A — no Rust crate boundary changes. No `serde_json::Error` / `gix::Error` / etc. surfaces in 2b's TypeScript code (those are caught at the Tauri command boundary on the backend and surface as already-classified strings or typed enums).

### Gate 5 — Type Design

- `RemediationClass` is a 3-state literal union — collapses 5 `PluginUpdateFailureKind` variants into 3 remediation paths. Switch is exhaustive (no `default:`).
- `kindLabel` switch is exhaustive (no `default:`). Mirrors the CLAUDE.md classifier rule.
- `pendingPluginActions: SvelteMap<string, "install" | "update" | "remove">` replaces the prior boolean-pair pattern. The `(install: true, update: true)` state is unrepresentable — at most one action is in flight per plugin key.
- `FormattedInstallPluginResult.anyInstalled` + `.anyFailed` are computed from the input data — each is a real semantic axis (could there be no installs but failures? yes; vice versa? yes; both? yes — partial install). Not collapsible to one boolean.
- `FormattedRemovePluginResult.hasItems` + `.hasFailures` — same: orthogonal axes (a remove that emptied the cascade with no failures has `hasItems=false, hasFailures=false` — that's the "nothing to remove" case).
- The `update?: PluginUpdateInfo | undefined` and `failure?: PluginUpdateFailure | undefined` props on `PluginCard` are mutually exclusive in practice (the backend never produces both for one plugin) but typed as independent `undefined`-able values for forward-compat. Action-area state machine handles all four `(update, failure) ∈ { (∅,∅), (info,∅), (∅,fail), (info,fail) }` cases — the last is rendered as `failure` precedence (the row "we don't know what's available" matters more than a stale "we knew yesterday's update").

### Gate 6 — Reference vs Transcription

- The plan cites `BrowseTab.svelte:734-852` as the source of `formatInstallPluginResult` rather than transcribing the summarization logic — Task 5's implementation is the extraction, with the test cases describing the contract the *result* must satisfy.
- The plan cites `bindings.ts:line` ranges for every wire-format type rather than reproducing struct shapes inline. Plan-time consumers read the bindings; the plan describes how to consume them.
- The 5 `PluginUpdateFailureKind` variants ARE enumerated in the switch arms (Task 2.3, 3.3) — that's not transcription, that's the implementation. The transcription risk would be paraphrasing the variants in prose; the plan instead cites `bindings.ts:1040-1069` for the canonical list.
- Hint copy (`hintFor`) and tooltip copy (`kindLabel`) are net-new strings introduced by this plan — they're not transcribed from existing code; they're the FE's UX contract for 2b.
- The banner-stack render block in Task 8 is a near-verbatim extraction of `BrowseTab.svelte:1066-1132`. The duplication is intentional during the extraction step; Task 9 deletes the original. After Task 9 the only copy is in `BannerStack.svelte`.

---

## Self-review

Walked through each spec section against the task list:

| Spec section | Tasks |
|---|---|
| `pluginUpdates` store (`design.md` §"New module: plugin-updates.svelte.ts") | Task 7 |
| Pure helpers `remediationClass`, `kindLabel`, `groupFailures`, `hintFor` (`design.md` §"New module: plugin-updates.ts") | Tasks 2, 3, 4 |
| `formatInstallPluginResult` (`design.md` §"Update flow") | Task 5 |
| `formatRemovePluginResult` (`design.md` §"Remove toast") | Task 6 |
| PluginCard new states (`design.md` §"Updated component: PluginCard.svelte") | Task 10 |
| InstalledTab Status column + Update + reshape (`design.md` §"Updated component: InstalledTab.svelte" + §"Remove toast") | Tasks 12, 13 |
| Banner stack extraction (`design.md` §"Banner stack") | Tasks 8, 9 |
| MarketplacesTab callback + page wiring (`design.md` §"User-locked decisions" #2) | Tasks 14, 15 |
| Failure → banner $effect (`design.md` §"Failure → banner mapping") | Tasks 11, 12 |
| `partial_load_warnings` dedupe via existing `ERR_INSTALLED_PLUGINS` (`design.md` §"Failure → banner mapping") | Task 12 (`refresh()` change routes to `fetchErrors[ERR_INSTALLED_PLUGINS]`) |
| Toplevel `fetchError` $effect (`design.md` §"Toplevel fetchError (rare)") | Tasks 11, 12 |
| Vitest setup (`design.md` §"Vitest, narrowly scoped") | Task 1 |
| CLAUDE.md pre-commit update (`design.md` Module map) | Task 16 |
| Playwright e2e (`design.md` §"Playwright (e2e)") | Task 18 |
| Manual smoke (`design.md` §"Manual UI smoke") | Task 17.6 |

No gaps found.

**Placeholder scan:** searched for "TBD", "TODO", "fill in", "appropriate error handling" — none.

**Type consistency:** Shared `PluginAction` union exported from `plugin-updates.ts`; per-tab maps narrow via `Extract<PluginAction, "install" | "update">` (BrowseTab) and `Extract<PluginAction, "remove" | "update">` (InstalledTab). `PluginCard`'s `update`/`failure` props match the bindings types in every reference. `formatInstallPluginResult` parameter type is `InstallPluginResult_Serialize` (not the union); `formatRemovePluginResult` parameter is `RemovePluginResult` (the only non-split type). `removeResult: RemovePluginResult | null` declared once and read consistently. `kindLabel` named consistently across imports.

Single naming inconsistency caught and fixed: an early draft mixed `formatInstallResult` and `formatInstallPluginResult` — settled on `formatInstallPluginResult` everywhere.

---

## Plan-review record

This plan was originally drafted with an off-board amendments doc holding the post-review corrections; that doc was consolidated inline (the dual-doc pattern is fragile under subagent dispatch — context truncation can drop the smaller corrections doc). What follows preserves the audit trail for future plan-review passes: which gates fired, what the finding was, why it was fixed, where the fix landed.

Two-pass review applied per `docs/plan-review-checklist.md`:
- **LSP-first pass** — `documentSymbol` / `workspaceSymbol` queries against `bindings.ts`, `format.ts`, `keys.ts` to verify every cited type/function/method/prop exists at the SHA the plan was written against.
- **Code-reviewer-style pass** — separate subagent walked each task asking "does this do the right thing?" — semantic / behavioral coverage for things LSP can't see.

### Findings (consolidated, with disposition)

| ID | Severity | Gate | Disposition | Where the fix lives |
|---|---|---|---|---|
| P2b-1 | blocker | Gate 1 | **Fixed inline** | Task 5.1 fixture: `FailedSkill.kind` field added; `InstallAgentsResult_Serialize.installed_native` + `installed_companions` added; `agents.installed` corrected to `string[]` |
| P2b-2 (×3) | — | — | **False positive** | Code-reviewer claimed `update-check<...>` ErrorSource keys lack a delimiter and `errLabel` parser is broken via `indexOf("")`. The plan actually uses literal `` (DELIM) bytes in the template strings; the Read tool renders control chars as nothing, fooling both reviewers. Verified via `cat -A` on the byte stream — separator is correct, type narrows correctly, parser finds the right index. No code change needed. **Lesson: when reviewing template-literal types, check the actual bytes — `cat -A` or `xxd` — not the rendered text.** |
| P2b-3 | blocker | Gate 5 | **Fixed inline** | Task 8.1 BannerStack `<script>` opener uses `generics="K extends string"` (Svelte 5 syntax); Tasks 9.2 + 13.1 drop their `as ErrorSource` casts on the `ondismiss` callback |
| P2b-4 | major | Gate 1 | **Fixed inline** | Task 12.1's `<script>` replacement no longer declares `loadWarning`; Task 12.4's refresh body no longer touches it; Task 12.6 became a verify-only grep |
| P2b-5 | major | Gate 1 | **Fixed inline** | Task 12.2's cascade reordered to `pluginUpdates.refresh` first, then local `refresh()` — uniform with Task 11.5 (BrowseTab updatePlugin) and Task 12.3 (InstalledTab updatePlugin) |
| P2b-6 | major | Gate 5 | **Fixed inline** | New `PluginAction` type alias exported from `plugin-updates.ts` (Task 4.3); per-tab maps use `Extract<PluginAction, ...>` to narrow (Tasks 11.4, 12.1) |
| P2b-7 | minor | Gate 1 | **Fixed inline** | Task 12.5's `statusUpdateLabel` for ContentChanged returns `"Content changed since install"` — a status sentence distinct from PluginCard's button label `"Update (content changed)"`; comment documents the intentional divergence (column = state, button = action) |
| P2b-8 | minor | Gate 6 | **Fixed inline** | Tasks 5.1 + 6.1 fixtures now cite `bindings.ts:NNN` for every type they construct |
| P2b-9 | minor | scope | **Fixed inline** | Task 18 dropped its second `test.skip` stub — only the "Status column appears" e2e remains; the missing toast scenario is documented as fixture-dependent future scope |

### Non-amendment observation (D-4 from code-reviewer pass)

**Project-switch race in `pluginUpdates` store.** If a user switches projects A → B fast, two `refresh()` calls overlap; A's promise can resolve after B's, overwriting B's data with A's. The store has no version/abort guard. Not fixed in this plan (YAGNI for Phase 2b — sub-100ms scan time + the design's "Out of scope: hash memoization" line — but worth a 4-line monotonic-token guard if real users hit it).

### Process note for future plan reviews

The biggest lesson from this pass was the false positive on the ErrorSource separator (P2b-2). The Read tool's silent rendering of control characters (`` → invisible) is a subtle hazard for plan-review work. Two protections going forward:

1. **When a finding involves a template literal that "looks wrong"**, immediately verify the byte stream with `cat -A`, `xxd`, or `od -c` before believing the finding. The display layer can lie.
2. **Subagent reviewers can't see the bytes either** — they read the same rendered text we do. A finding that depends on "this string has no separator" is fragile; it should be paired with a byte-level check before being elevated to "blocker."

### What landed: gate-by-gate

- **Gate 1 (Grounding):** P2b-1, P2b-4, P2b-5, P2b-7
- **Gate 2 (Threat Model):** No findings
- **Gate 3 (Wire Format):** No findings (Phase 2b introduces zero new types crossing FFI; all consumed types ship in PR #96)
- **Gate 4 (External Type Boundary):** N/A (no Rust crate boundary changes)
- **Gate 5 (Type Design):** P2b-3, P2b-6
- **Gate 6 (Reference vs Transcription):** P2b-8

---

## Plan complete

**Plan saved to `docs/plans/2026-05-05-phase-2b-update-detection-ui-plan.md`.**

Two execution options:

1. **Subagent-Driven (recommended)** — Dispatch a fresh subagent per task, review between tasks, fast iteration. Each task block is self-contained.
2. **Inline Execution** — Execute tasks in this session using `superpowers:executing-plans`, batch execution with checkpoints.

Which approach?

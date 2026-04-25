# Plan Review Findings

Findings from a consistency / contradiction / underspecification / assumption
review of the three stage plans against the actual `kiro-market-core` code.

**Plans reviewed:**
- `2026-04-23-stage1-content-hash-primitive-plan.md`
- `2026-04-23-stage2-native-kiro-cli-agent-import-plan.md`
- `2026-04-23-stage3-steering-import-plan.md`

**Spec under review:**
- `2026-04-23-kiro-cli-native-plugin-import-design.md`

**Review evidence:** direct reads of `crates/kiro-market-core/src/error.rs`,
`service/mod.rs`, `service/test_support.rs`, `project.rs`. Findings are
verified against actual code state on commit `f81ae86`, not assumed.

---

## Status Legend

- ✅ **FIXED** — addressed inline in the Stage 1 plan revision committed
  alongside this doc.
- ⚠️ **STAGE 2 REVISION REQUIRED** — needs explicit decision + plan edit
  before Stage 2 implementation begins.
- ⚠️ **STAGE 3 REVISION REQUIRED** — needs explicit decision + plan edit
  before Stage 3 implementation begins.
- 📝 **DOCUMENTATION ONLY** — no code change, but worth noting in the
  implementer's mental model.

---

## Critical assumption mismatches against actual code

### 1. `install_plugin_agents` signature is completely different from what Stage 2 plans against

**Status:** ⚠️ STAGE 2 REVISION REQUIRED

**Evidence:** `crates/kiro-market-core/src/service/mod.rs:1161`:

```rust
pub fn install_plugin_agents(
    &self,
    project: &KiroProject,
    plugin_dir: &Path,
    scan_paths: &[String],
    mode: InstallMode,                  // NOT force: bool
    accept_mcp: bool,
    marketplace: &str,
    plugin: &str,
    version: Option<&str>,
) -> InstallAgentsResult
```

Stage 2 Task 18 plans against a fictional signature:

```rust
svc.install_plugin_agents(&project, "marketplace-x", &ctx, AgentInstallOptions { force, accept_mcp })
```

**Impact:** Stage 2's "wire dispatch" task doesn't fit the existing
function. Three options:
- **(A)** Refactor the existing signature to take `&PluginInstallContext`
  and `AgentInstallOptions` first (substantial scope expansion that
  affects existing translated-path callers).
- **(B)** Add a sibling method `install_plugin_agents_v2(...)` with the
  new shape and route the legacy method to it (or vice versa) until the
  CLI catches up. Allows incremental migration.
- **(C)** Keep existing positional params; have Stage 2's dispatch
  resolve `format` separately and fork inside the existing function
  body. Smallest blast radius.

**Recommendation:** Option C for v1 (tightest scope). Plans should be
updated to reflect this — the dispatch reads `manifest.format` directly
inside `install_plugin_agents` body rather than depending on a
pre-resolved context field.

---

### 2. `AgentError` has only THREE variants today, not 5+

**Status:** ⚠️ STAGE 2 REVISION REQUIRED

**Evidence:** `crates/kiro-market-core/src/error.rs:269`:

```rust
pub enum AgentError {
    AlreadyInstalled { name },
    NotInstalled { name },
    ParseFailed { path, failure },
}
```

Stage 2 Task 6 lists 5 *new* variants. Stage 2 Task 15 *also* needs
`PathOwnedByOtherPlugin` and `OrphanFileAtDestination` (referenced in
the `install_native_companions` impl with a hand-wavy "add these
alongside Task 6's variants if not already there" note). And Task 18
references `McpRequiresAccept` and `ManifestReadFailed` — the latter
exists on `PluginError` (`error.rs:135`) but NOT on `AgentError`.

**Net:** Task 6 needs to add **9 new variants**, not 5:

```rust
NativeManifestParseFailed { path, source: serde_json::Error },
NativeManifestMissingName { path },
NativeManifestInvalidName { path, reason },
NameClashWithOtherPlugin { name, owner },
ContentChangedRequiresForce { name },
PathOwnedByOtherPlugin { path, owner },
OrphanFileAtDestination { path },
McpRequiresAccept { name },                    // OR reuse Warning shape
ManifestReadFailed { path, source: io::Error },// for native parse I/O
```

Plus: `from_plugin_error` and `remediation_hint` (`error.rs:206`)
classifiers must add explicit arms for each — no `_ =>` per CLAUDE.md.

---

### 3. MCP gate behavior: native plan diverges from existing translated path

**Status:** ⚠️ STAGE 2 REVISION REQUIRED

**Evidence:** `crates/kiro-market-core/src/service/mod.rs:1213` (existing
translated path):

```rust
if !accept_mcp && !def.mcp_servers.is_empty() {
    let transports: Vec<String> = def.mcp_servers.values()
        .map(|cfg| cfg.transport_label().to_owned())
        .collect();
    result.warnings.push(InstallWarning::McpServersRequireOptIn {
        agent: def.name.clone(),
        transports,
    });
    // ...skip into result.skipped...
}
```

The existing code:
- Gates **all** MCP transports (Stdio + Http + Sse), not just Stdio
- Routes to `warnings` + `skipped`, not `failed`
- Surfaces via the typed `InstallWarning::McpServersRequireOptIn`
  variant

Stage 2 Task 18 plans:

```rust
let has_stdio = bundle.mcp_servers.values().any(|s| s.is_stdio());
if has_stdio && !opts.accept_mcp { /* fail */ }
```

**Impact:** Different policy + different result-bucket = different UX
between native and translated agent installs that bring MCP servers.
Spec is silent on which is canonical.

**Recommendation:** Match existing translated behavior in native path
(gate any MCP, route to `warnings` + `skipped`). If the user later wants
to be more aggressive, they can tighten both paths together.

---

### 4. `test_marketplace_service()` doesn't exist

**Status:** ⚠️ STAGE 2 REVISION REQUIRED + ⚠️ STAGE 3 REVISION REQUIRED

**Evidence:** `crates/kiro-market-core/src/service/test_support.rs:64`:

```rust
pub fn temp_service() -> (TempDir, MarketplaceService) { ... }
```

No `test_marketplace_service()` function exists. Multiple service-layer
test cases in Stage 2 (Tasks 18, 20) and Stage 3 (Tasks 9, 11) call the
non-existent fixture.

**Recommendation:** Replace `test_marketplace_service()` with
`temp_service()` throughout. The returned `(TempDir, MarketplaceService)`
shape requires a small destructure adjustment in each test:

```rust
let (_tempdir, svc) = crate::service::test_support::temp_service();
```

---

### 5. `DiscoveryWarning` doesn't exist; actual type is `InstallWarning`

**Status:** ⚠️ STAGE 3 REVISION REQUIRED

**Evidence:** Stage 3 Task 2's `steering/types.rs` declares:

```rust
pub warnings: Vec<crate::service::DiscoveryWarning>,
```

Actual type is `InstallWarning` (variants: `AgentParseFailed`,
`McpServersRequireOptIn`, etc. — domain is "install-time observations,"
not "discovery-time observations").

**Recommendation:** Either reuse `InstallWarning` directly (creates a
cross-module dep that's awkward for the steering module) or define a
small steering-specific `SteeringWarning` enum in `steering/types.rs`.
The latter is cleaner and matches the "sibling-shaped module" intent.

---

### 6. `FailedAgent` actual shape doesn't match plan

**Status:** ⚠️ STAGE 2 REVISION REQUIRED

**Evidence:** `crates/kiro-market-core/src/service/mod.rs:~1199`:

```rust
result.failed.push(FailedAgent {
    name: path.display().to_string(),               // String, not Option<String>
    error: crate::error::error_full_chain(&e),     // pre-rendered String
});
```

Stage 2 Type Changes section plans:

```rust
pub struct FailedAgent {
    pub name: Option<String>,
    pub source_path: PathBuf,
    pub error: AgentError,                          // typed
}
```

**Impact:** Either keep the existing pre-rendered shape (simpler, but
loses typed-error fidelity for future programmatic handling) or upgrade
the existing struct to the typed shape (refactor that touches every
caller).

**Recommendation:** Upgrade to typed shape in Stage 2 — small cost
(one-time refactor of FailedAgent's existing call sites), big benefit
(future error-classifier code can pattern-match instead of substring-
matching the rendered message).

---

## Cross-plan contradictions

### 7. Translated install must populate `native_companions` — was missing in both Stage 1 AND Stage 2

**Status:** ✅ FIXED in Stage 1

**Evidence:** Spec section "Translated-agent path also gets hashes (and
a companion entry)" calls for translated installs to write a synthesized
`native_companions` entry so the cross-format collision model is
uniform. Original Stage 1 plan only added hash fields; original Stage 2
Task 8 added the `native_companions` map but didn't update
`install_agent_inner` to populate it. **Result of either oversight:** a
translated agent's `prompts/code-reviewer.md` looks like an *orphan
file* to a subsequent native plugin install, which would refuse it
without `--force`.

**Fix applied:** Stage 1 now owns the schema additions
(`InstalledNativeCompanionsMeta` + `native_companions` field) AND
extends `install_agent_inner` to maintain a per-plugin companion entry
listing every prompt file the translated install of this plugin wrote.
See Stage 1 Tasks 12 + 14.

**Stage 2 Task 8** becomes a no-op (the schema already exists). The
implementer should skip Task 8 during Stage 2 execution.

**Known v1 limitation documented in Stage 1's commit message:** if a
translated agent is later overwritten by a different plugin via
`--force`, the prior plugin's `native_companions` entry still lists the
prompt path. Cross-plugin transfer logic lives in Stage 2's native
install paths; backporting it to translated path is out of scope for
v1.

---

### 8. Native install's "verbatim copy" claim is broken by JSON re-serialization

**Status:** ⚠️ STAGE 2 REVISION REQUIRED

**Evidence:** Stage 2 Task 10's `install_native_agent` impl:

```rust
let pretty = serde_json::to_vec_pretty(&bundle.raw_json)?;
std::fs::write(&staging_json, &pretty)?;
```

Stage 2 parses JSON into `serde_json::Value`, then re-serializes pretty
when writing the destination. If the source isn't already
pretty-printed (different whitespace, different field ordering), the
installed bytes differ from the source bytes byte-for-byte. The spec's
"Out of Scope" section explicitly says **"v1 preserves them verbatim."**

**Two fixes:**

- **(A)** Store `raw_bytes: Vec<u8>` on the bundle alongside `raw_json:
  Value`, and `fs::write(&dest, &bundle.raw_bytes)` (or `fs::copy(&src,
  &dest)` if the bundle keeps a path). True verbatim.
- **(B)** Update the spec to say "semantic-equivalent (re-serialized to
  canonical pretty form)." Documents the actual behavior.

**Recommendation:** **(A)** — the user explicitly said "preserve
verbatim." Adds one field to `NativeAgentBundle`.

---

## Underspecification

### 9. Companion bundle reinstall leaves orphan files on disk

**Status:** ⚠️ STAGE 2 REVISION REQUIRED

**Evidence:** Stage 2 Task 15's `install_native_companions`:

```rust
installed.native_companions.insert(plugin.to_string(),
    InstalledNativeCompanionsMeta { ..., files: entries.iter()... });
```

This OVERWRITES the prior entry's `files` list with the new bundle's
files. If Plugin A previously shipped `[a, b, c]` and now ships
`[b, c, d]`, file `a` lingers on disk untracked — a true orphan with no
ownership trail.

**Fix:** Before the rename phase, compute the diff between the prior
entry's `files` and the new entry's `files`. Files in the prior set but
not the new set are removed from disk during the install. Use the same
per-file lock to protect the removal.

```rust
let to_remove: Vec<&PathBuf> = old_files.iter()
    .filter(|p| !new_files.contains(p))
    .collect();
for f in to_remove {
    let abs = self.kiro_dir().join("agents").join(f);
    let _ = std::fs::remove_file(&abs);  // best-effort
}
```

---

### 10. Stage 2 Task 18 assumes single scan_root for companion source_hash

**Status:** ⚠️ STAGE 2 REVISION REQUIRED

**Evidence:** Stage 2 Task 18's plan:

```rust
let scan_root = companion_files[0].scan_root.clone();
let rel_paths: Vec<PathBuf> = companion_files.iter()
    .map(|f| f.source.strip_prefix(&f.scan_root)...)
    .collect();
let source_hash = crate::hash::hash_artifact(&scan_root, &rel_paths)?;
```

This computes the hash relative to `companion_files[0].scan_root`. If
the plugin declares `agents: ["./agents/", "./extra-agents/"]`, the
companion files come from BOTH scan roots with different `scan_root`
values. The strip-prefix would produce wrong relative paths (or `Err`)
for files from the second scan root.

**Fix:** Group companion files by `scan_root` and either (a) reject
multi-scan-root native plugins with a clear error, or (b) compute a
combined hash by feeding each (scan_root, rel) pair as
`scan_root_str || 0x00 || rel_str || 0x00 || file_bytes || 0x00`.

**Recommendation:** (a) for v1 — return a typed
`AgentError::MultipleScanRootsNotSupported { paths }` if companion files
span more than one scan_root. Single-scan-root is the common case (and
the only case the starter-kit needs). (b) is a follow-up if a real
plugin needs it.

---

### 11. `uuid_or_pid()` in Stage 3 Task 7 is undefined

**Status:** ⚠️ STAGE 3 REVISION REQUIRED

**Evidence:** Stage 3 Task 7's `install_steering_file` impl:

```rust
let staging = self.steering_dir().join(format!(".staging-{}", uuid_or_pid()));
```

The plan says "if the project doesn't already have such a helper, use
`std::process::id().to_string()` or similar." Engineer is left to
invent.

**Fix:** Hard-code one approach. Recommendation:

```rust
let staging = self.steering_dir().join(format!(
    ".staging-{}-{}",
    std::process::id(),
    chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0),
));
```

(`with_file_lock` already serializes installs of the same tracking
file, so the unique name is purely defensive against leftover staging
from prior crashed processes.)

---

### 12. CLI presenter field renames are breaking changes I didn't flag

**Status:** ⚠️ STAGE 2 REVISION REQUIRED

**Evidence:** Stage 2 Task 19 says to render `result.installed_agents`
(new field name). Existing CLI code consumes `result.installed`
(per the existing `InstallAgentsResult` shape).

**Impact:** Renaming `installed` → `installed_agents` is a breaking
change that touches every existing consumer (CLI, future Tauri). Plan
doesn't acknowledge or scope this.

**Fix:** Either:
- **(A)** Keep the existing `installed` field name; the native install
  path populates it with the same shape (carrying the unified outcome).
  Add a new `installed_companions: Option<...>` field for the
  per-plugin companion bundle outcome.
- **(B)** Rename and migrate all consumers as part of Stage 2.

**Recommendation:** **(A)** for minimum disruption. The unified
`installed` field is fine if the per-agent outcome shape carries the
needed metadata for both translated and native paths.

---

### 13. Hash failure during install — caller behavior

**Status:** 📝 DOCUMENTATION ONLY (low risk)

**Evidence:** Stage 1's `install_skill_from_dir` returns
`Err(HashError)` if hashing fails. Existing CLI code expects skill
install errors to be specific `SkillError` variants.

**Analysis:** Stage 1 Task 3 step 3 adds `HashError` as
`#[error(transparent)] Hash(#[from] HashError)` on the top-level
`Error`. Existing callers using `?` propagation are unaffected.
Callers using `match Error { ... }` exhaustively will get a compile
error (caught at build time). No silent failures.

**Documentation note:** Stage 1 Task 15 (final verification) catches
this via `cargo test --workspace`. The plan should explicitly call out
"adding a new top-level `Error` variant may force exhaustive `match`
callers to add an arm — fix any compile errors that surface."

**No code fix needed.** Add a one-line note to Stage 1 Task 15 if
revising.

---

## Other inconsistencies

### 14. `force: bool` vs `InstallMode` enum

**Status:** ⚠️ STAGE 2 REVISION REQUIRED

**Evidence:** Existing `install_plugin_agents` takes `mode: InstallMode`.
Plans use `force: bool` everywhere.

**Recommendation:** Use `InstallMode` throughout new code (consistent
with existing). The `AgentInstallOptions` / `SteeringInstallOptions`
structs (if kept — see #1) should embed `InstallMode`, not a bare
`force: bool`.

---

### 15. `InstalledAgents` extension affects struct-literal callers

**Status:** 📝 DOCUMENTATION ONLY (already partially noted in plans)

**Evidence:** Stage 1 Task 12's new `native_companions` field on
`InstalledAgents` will cause any test that does `InstalledAgents {
agents: ... }` (struct literal without `..Default::default()`) to fail
to compile.

**Status:** Stage 1 Task 12 step 6 already says "Any test that compares
`InstalledAgents` literally may need a `..Default::default()` to pick
up the new field — fix any such test."

**No additional fix needed.**

---

### 16. JSON parsed twice + pretty-printed once = three passes

**Status:** 📝 DOCUMENTATION ONLY (acceptable trade-off)

**Evidence:** Stage 2 Task 5's `parse_native_kiro_agent_file`
deserializes the bytes twice (once into `NativeAgentProjection`, once
into `serde_json::Value`). Stage 2 Task 10 then re-serializes the
`Value` into pretty bytes for write-out.

**Analysis:** Three passes over the JSON for one install. Acceptable
for typical agent JSONs (sub-kilobyte). If #8 is fixed (verbatim
copy), the second deserialize-into-Value goes away and the
re-serialize is replaced with `fs::copy` — net 1.5 passes (one
parse for projection, one byte-copy).

**No code fix needed beyond #8.**

---

### 17. `SteeringError` mixes typed-domain variants with bare infrastructure pass-throughs

**Status:** ⚠️ STAGE 3 REVISION REQUIRED

**Evidence:** Stage 3 Task 2's `SteeringError` declares:

```rust
#[error(transparent)] Hash(#[from] HashError),
#[error(transparent)] Io(#[from] io::Error),
#[error(transparent)] Json(#[from] serde_json::Error),
```

Per CLAUDE.md ("don't lose context at adapter boundaries"), bare `Io`
and `Json` arms invite callers to lose the location of the failure —
the user sees "no such file or directory" without knowing which
file/operation failed.

**Fix:** Wrap each in a typed variant:

```rust
#[error("hash computation failed at `{path}`")]
HashFailed { path: PathBuf, #[source] source: HashError },
#[error("I/O failed at `{path}`")]
IoFailed { path: PathBuf, #[source] source: io::Error },
#[error("steering tracking JSON malformed at `{path}`")]
TrackingMalformed { path: PathBuf, #[source] source: serde_json::Error },
```

The existing `SourceReadFailed` and `TrackingIoFailed` variants
already follow this pattern — extending it across the rest of the
infrastructure-error surface.

---

## Summary

| # | Finding | Status |
|---|---|---|
| 1 | `install_plugin_agents` actual signature differs | ⚠️ Stage 2 |
| 2 | `AgentError` needs 9 new variants, not 5 | ⚠️ Stage 2 |
| 3 | MCP gate native vs translated divergence | ⚠️ Stage 2 |
| 4 | `test_marketplace_service` doesn't exist | ⚠️ Stage 2 + 3 |
| 5 | `DiscoveryWarning` should be `InstallWarning` (or new type) | ⚠️ Stage 3 |
| 6 | `FailedAgent` actual shape differs | ⚠️ Stage 2 |
| 7 | Translated install must populate `native_companions` | ✅ Fixed Stage 1 |
| 8 | "Verbatim copy" broken by JSON re-serialization | ⚠️ Stage 2 |
| 9 | Companion reinstall leaves disk orphans | ⚠️ Stage 2 |
| 10 | Multi-scan_root companion source_hash assumption | ⚠️ Stage 2 |
| 11 | `uuid_or_pid()` undefined | ⚠️ Stage 3 |
| 12 | CLI field rename is a breaking change | ⚠️ Stage 2 |
| 13 | Hash failure surfacing | 📝 Doc only |
| 14 | `force: bool` vs `InstallMode` | ⚠️ Stage 2 |
| 15 | Struct-literal callers may break | 📝 Already noted |
| 16 | JSON parsed three times (acceptable) | 📝 Doc only |
| 17 | `SteeringError` infrastructure variants | ⚠️ Stage 3 |

**Tally:** 1 fixed, 11 require Stage 2/3 revisions, 4 documentation
only / acceptable.

---

## Recommended next moves

**1. Stage 1 is now executable as-is.** The fixes for #7 are inline.
Items #13 and #15 are documentation-only / already acknowledged. The
remaining items are all Stage 2 / Stage 3 concerns that don't block
Stage 1.

**2. Before executing Stage 2, do a focused Stage 2 plan revision
session.** Items #1, #2, #3, #4, #6, #8, #9, #10, #12, #14 all need
explicit decisions baked into the plan. Estimated scope: 2-3 hours of
plan revision (not implementation), grounded in a real read of the
existing `install_plugin_agents` body to understand what stays vs.
what changes.

**3. Stage 3 plan revisions are smaller** (#4, #5, #11, #17). 30-45
min after Stage 2 plan is settled, since some of Stage 3's shape
depends on Stage 2's final decisions on #1 / #14.

**4. Alternative — execute Stage 1, then re-derive Stages 2 and 3
plans from scratch.** Once Stage 1's hash primitive and tracking
schema are real code, the Stage 2 and 3 plans can be written against
known-good APIs instead of assumed ones. This is a valid path if you'd
rather spend the time implementing Stage 1 than revising plans.

The user's stated preference during planning was "split this larger
work into smaller, but still very well defined steps if needed." This
review surfaces that Stage 2 in particular is closer to "loose
sketch" than "well defined" against actual code. Either revising the
plans before execution or deriving Stages 2-3 fresh after Stage 1
respects that preference.

---

## Self-review note

This review is itself a check that the original "self-review against
spec" step in the writing-plans skill was inadequate — it caught
internal-consistency issues (type names match across plans) but did
not catch grounding issues (plans assume APIs that don't exist).
Future plan-writing for this project should explicitly include a
"grep / read against actual code" pass alongside the spec-coverage
pass.

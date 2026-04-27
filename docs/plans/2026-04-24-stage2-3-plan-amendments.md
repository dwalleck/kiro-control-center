# Stage 2 + Stage 3 Plan Amendments

**Generated:** 2026-04-24, after Stage 1 landed on branch `feat/native-kiro-plugin-import` (PR #48 in flight).

**Companion docs:**
- `2026-04-23-stage2-native-kiro-cli-agent-import-plan.md` — Stage 2 plan as originally drafted (read first).
- `2026-04-23-stage3-steering-import-plan.md` — Stage 3 plan as originally drafted (read first).
- `2026-04-23-kiro-cli-native-plugin-import-design.md` — design intent (still authoritative for WHAT we build; this doc reconciles HOW).
- `2026-04-23-plan-review-findings.md` — review findings the original plans never folded back in.

## How to use this doc

The Stage 2 and Stage 3 plans were drafted alongside Stage 1's plan in a single earlier session and not grounded against the actual `kiro-market-core` API surface. Stage 1 has now shipped — and shipped with several refinements not in the original plan (staging-before-rename, `CompanionInput<'a>` param-bundles, helper extraction, `skip_serializing_if` on empty collection fields, etc.). This doc captures every drift item between the plans and the **actual landed code** so the Stage 2 / Stage 3 implementer can apply the corrections inline as they execute each task.

When executing a Stage 2 / Stage 3 task, **read the corresponding amendment block here first**, then read the original plan section. Apply the amendment in place of (or alongside) the plan text. The original plan files are not modified; this doc is the delta.

Amendment IDs:
- `S2-N` — patches `2026-04-23-stage2-native-kiro-cli-agent-import-plan.md`.
- `S3-N` — patches `2026-04-23-stage3-steering-import-plan.md`.
- `P-N` — Stage 1 PATTERNS to apply uniformly across both plans where relevant.

Severity:
- **BLOCKING** — plan reference is wrong; executing as written will fail to compile or violate CLAUDE.md.
- **INHERIT** — plan re-implements something Stage 1 already landed; skip the duplication.
- **COSMETIC** — plan is technically correct but stylistically inconsistent with the post-Stage-1 codebase; apply for consistency, defer if pressed for time.

---

## Stage 1 patterns to apply uniformly (referenced by S2/S3 amendments)

### P-1: Staging-before-rename atomicity for `installed_hash`

Stage 1 commits `a8cd6b2` (skill) and `19e97c3` (agent) moved `installed_hash` computation from **after** the destructive `remove_dir_all + rename` block to **before** it, hashing the staged copy directly. Without this fix, a hash failure in `--force` mode after the old install was unlinked would leave the user with no install on disk at all. The hash value is bytewise identical because `copy_dir_recursive` (skill) and the `<name>.json + prompts/<name>.md` write path (agent) produce the same content at staging as will land at the final location.

**Pattern:**

```rust
// Stage all source bytes into a fresh staging dir.
copy_dir_recursive(source_dir, &staging_dir)?;  // OR fs::write to staging

// Compute installed_hash on staging BEFORE the destructive swap.
let installed_hash = match crate::hash::hash_<artifact|dir_tree>(&staging_dir, ...) {
    Ok(h) => h,
    Err(e) => {
        warn!(name, error = %e, "installed_hash computation failed on staging; removing staging dir");
        if let Err(cleanup_err) = fs::remove_dir_all(&staging_dir) {
            warn!(path = %staging_dir.display(), error = %cleanup_err, "failed to clean up staging directory");
        }
        return Err(e.into());
    }
};

// Only NOW do the destructive rename.
if dir.exists() {
    fs::remove_dir_all(&dir)?;     // force-mode old-content removal
}
fs::rename(&staging_dir, &dir)?;
```

For agent installs, the staged JSON file must be named `<name>.json` (NOT `agent.json`) so the staging layout mirrors `agents_root` and the hash relative-path list is the same in both bases. See Stage 1 commit `19e97c3`.

### P-2: `CompanionInput<'a>`-style param-bundle structs

Stage 1 commit `925990f` introduced a private bundle struct to avoid `#[allow(clippy::too_many_arguments)]` on a helper that took 8 immutable refs. CLAUDE.md is unconditional on no `#[allow(...)]` in production code.

**Pattern** (`project.rs:257-272`):

```rust
/// Input bundle for [`KiroProject::synthesize_companion_entry`]. Groups the
/// 7 immutable refs that the helper needs so the public-ish signature stays
/// at two parameters (the `&mut InstalledAgents` plus the bundle), avoiding
/// a `#[allow(clippy::too_many_arguments)]` waiver that would otherwise be
/// required.
struct CompanionInput<'a> {
    marketplace: &'a str,
    plugin: &'a str,
    version: Option<&'a str>,
    agents_root: &'a Path,
    prompt_rel: &'a Path,
    json_target: &'a Path,
    prompt_target: &'a Path,
}
```

**Where to apply in S2/S3:** any helper extracted from `install_native_agent`, `install_native_companions`, `install_steering_file`, `install_native_kiro_cli_agents_inner`, or `install_plugin_steering` that ends up with 7+ parameters. Do NOT add `#[allow(too_many_arguments)]` — bundle the immutable refs into a private input struct alongside any `&mut`-state parameter that stays standalone.

### P-3: Private helper extraction discipline

Stage 1 `install_agent_inner` was kept under the clippy `too_many_lines` threshold by extracting three helpers: `stage_agent_files`, `promote_staged_agent`, `synthesize_companion_entry`. The outer function reads as a thin orchestrator (validate → emit → source_hash → lock { load → check → stage → promote → insert → synthesize → write_tracking }).

**Pattern** — for any new install function (skill/native-agent/companion-bundle/steering) that is going to be more than ~100 lines, split into:

- `stage_<thing>_files(...)` — create staging, write or copy, compute `installed_hash` pre-destructive. Returns `(staging, rel_paths..., installed_hash)`.
- `promote_<thing>(...)` — force-clear existing targets, perform renames with cross-file rollback, clean up empty staging. Returns the absolute destination path(s).
- `synthesize_<bundle>_entry(...)` — update tracking-side state with the new ownership facts; rollback on any sub-step failure mirrors the existing patterns.

Do NOT add `#[allow(clippy::too_many_lines)]` on the outer function. If extraction doesn't get under threshold, escalate to the plan author rather than suppressing.

### P-4: `skip_serializing_if` on new optional collection / option fields

Stage 1 commit `16c049f` added `#[serde(default, skip_serializing_if = "HashMap::is_empty")]` to `InstalledAgents.native_companions` after the original Stage 1 plan deliberately omitted it. The omission caused round-trip noise: any project with no native companions would gain `"native_companions": {}` on the next tracking write, silently mutating the on-disk file.

**Pattern** — every new optional collection or `Option<T>` field on a tracking struct gets BOTH `#[serde(default)]` AND `#[serde(skip_serializing_if = "<predicate>")]`:

```rust
#[serde(default, skip_serializing_if = "HashMap::is_empty")]
pub native_companions: HashMap<String, InstalledNativeCompanionsMeta>,

#[serde(default, skip_serializing_if = "Option::is_none")]
pub source_hash: Option<String>,
```

Applies in S3 to `InstalledSteering.files` and any future tracking maps Stage 2 / Stage 3 introduce.

### P-5: Inherited from `hash_artifact` automatically (no plan changes needed)

Stage 1 commits `56de6d4` (forward-slash path normalization) and `db6535b` (symlink TOCTOU re-check before `fs::read`) both live inside `hash_artifact`. Any code path that hashes via `hash_artifact` or `hash_dir_tree` inherits these properties. **Do not** introduce sibling helpers in Stage 2 / Stage 3 that re-implement path-to-string conversion or `fs::read` against discovered files — route everything through the primitives.

### P-6: Backup-then-swap atomicity for cross-plugin `--force` (deferred Stage 1 → Stage 2)

Stage 1's `synthesize_companion_entry` documented a residual risk: the per-plugin companion hash still runs **post-rename**, and if it fails in force mode, the rollback removes the just-renamed files but the previous plugin's files were already deleted at the destructive step. Stage 1 deferred the full `.old`-sibling backup-then-swap fix to Stage 2 because Stage 2 owns cross-plugin ownership transfer.

**Where to apply in S2:** any install path in Stage 2 that performs `fs::remove_file(&existing) + fs::rename(&staged, &existing)` in a way that **could** be followed by a fallible step (hash, tracking write, etc.) **must** adopt:

```rust
// Backup phase (only when overwriting existing tracked content):
let backup = existing.with_extension("kiro-bak");
if existing.exists() {
    fs::rename(&existing, &backup)?;
}

// Promote phase:
fs::rename(&staged, &existing)?;

// Validate phase (any fallible work):
let result = run_post_rename_work();

match result {
    Ok(_) => {
        // Commit phase:
        if backup.exists() {
            let _ = fs::remove_file(&backup);  // best-effort cleanup
        }
    }
    Err(e) => {
        // Rollback phase:
        let _ = fs::remove_file(&existing);
        if backup.exists() {
            if let Err(restore_err) = fs::rename(&backup, &existing) {
                warn!(...);
            }
        }
        return Err(e.into());
    }
}
```

For Stage 2's `install_native_companions` (Task 15), this affects every per-file rename in the bundle's promote phase — backup each existing file before its rename. For `install_native_agent` (Task 10), backup the existing `<name>.json` if present in force mode.

---

## Stage 2 amendments

### S2-1: Skip Task 8 entirely

**Severity:** INHERIT

**Affected plan section:** Stage 2 plan, Task 8 (`Add InstalledNativeCompanionsMeta and native_companions map`), lines 1124-1216.

**What the plan says:** Task 8 step 3 adds a new `InstalledNativeCompanionsMeta` struct, step 4 extends `InstalledAgents` with `native_companions: HashMap<String, InstalledNativeCompanionsMeta>`, step 7 commits with message `feat(core): add InstalledNativeCompanionsMeta + native_companions map`.

**What's actually true:** Stage 1 already landed both the new struct AND the field. `crates/kiro-market-core/src/project.rs:88-114` defines `InstalledNativeCompanionsMeta` (with `marketplace, plugin, version: Option<String>, installed_at: DateTime<Utc>, files: Vec<PathBuf>, source_hash: String, installed_hash: String`). `crates/kiro-market-core/src/project.rs:116-124` extends `InstalledAgents` with `native_companions: HashMap<String, InstalledNativeCompanionsMeta>` already with `#[serde(default, skip_serializing_if = "HashMap::is_empty")]` (per P-4). The backward-compat test `installed_agents_loads_legacy_json_without_native_companions` already lives in the project.rs test module.

**Required revision:** Skip Task 8 entirely — do not write a duplicate struct, do not write a duplicate test, do not produce a commit. Note in your task tracker that Stage 2 Task 8 is a no-op at execution time. Proceed directly from Task 7 to Task 9.

**Reasoning:** Stage 1 absorbed this work as part of fix #7 from the plan-review-findings doc. The original Stage 2 plan was written before that fix was scoped into Stage 1.

---

### S2-2: `AgentError` needs 9 new variants (not 5)

**Severity:** BLOCKING

**Affected plan section:** Task 6 (`Add five new AgentError variants`), lines 932-1041. Also affects Tasks 15 (`install_native_companions`, lines 1762-2083) and 18 (`install_native_kiro_cli_agents_inner`, lines 2237-2563) which reference variants Task 6 doesn't add.

**What the plan says:** Task 6 step 3 adds five variants: `NativeManifestParseFailed`, `NativeManifestMissingName`, `NativeManifestInvalidName`, `NameClashWithOtherPlugin`, `ContentChangedRequiresForce`. Tasks 15 and 18 then reference `PathOwnedByOtherPlugin`, `OrphanFileAtDestination`, `McpRequiresAccept`, `ManifestReadFailed` without ever adding them. Task 18 step 6 acknowledges "Add `AgentError::McpRequiresAccept` if not present" but is non-binding.

**What's actually true:** `crates/kiro-market-core/src/error.rs:269-287` defines `AgentError` with exactly 3 variants today: `AlreadyInstalled { name }`, `NotInstalled { name }`, `ParseFailed { path, failure }`. None of the 9 referenced variants exist. CLAUDE.md mandates classifier functions enumerate every variant explicitly (no `_ =>` defaults), so each new variant ALSO requires arms in `SkippedReason::from_plugin_error` (`crates/kiro-market-core/src/service/browse.rs`) and `PluginError::remediation_hint` (`crates/kiro-market-core/src/error.rs:206`).

**Required revision:** Rewrite Task 6 step 3 to add all 9 variants in a single commit:

```rust
// Native-import parsing failures:
#[error("native agent JSON `{path}` failed to parse")]
NativeManifestParseFailed {
    path: PathBuf,
    #[source]
    source: serde_json::Error,
},
#[error("native agent at `{path}` is missing the required `name` field")]
NativeManifestMissingName { path: PathBuf },
#[error("native agent at `{path}` has an invalid `name`: {reason}")]
NativeManifestInvalidName { path: PathBuf, reason: String },

// Manifest I/O (parallels PluginError::ManifestReadFailed):
#[error("could not read native agent manifest at {path}")]
ManifestReadFailed {
    path: PathBuf,
    #[source]
    source: io::Error,
},

// Cross-plugin / collision:
#[error(
    "native agent name `{name}` would clobber an agent owned by plugin \
     `{owner}`; pass --force to transfer ownership"
)]
NameClashWithOtherPlugin { name: String, owner: String },
#[error("path `{path}` is owned by plugin `{owner}`; pass --force to transfer")]
PathOwnedByOtherPlugin { path: PathBuf, owner: String },
#[error(
    "file exists at `{path}` but has no tracking entry; \
     remove it manually or pass --force"
)]
OrphanFileAtDestination { path: PathBuf },

// Reinstall-with-changed-content:
#[error(
    "agent `{name}` content has changed since last install; \
     pass --force to overwrite"
)]
ContentChangedRequiresForce { name: String },

// MCP gate (see also S2-12 for whether this stays as a typed AgentError or
// becomes an InstallWarning::McpServersRequireOptIn route — pending S2-12
// outcome, leave the variant defined; if S2-12's translated-parity decision
// wins, this variant will be unused at the install layer and can be dropped
// in a follow-up):
#[error(
    "agent `{name}` brings MCP servers; re-run with --accept-mcp to install"
)]
McpRequiresAccept { name: String },
```

Then in Task 6 step 5 ("Audit `SkippedReason::from_plugin_error` and `remediation_hint`") add explicit arms for ALL 9 new variants, not just the 5 the original plan mentioned. The classifier audit grows correspondingly. Task 6's commit message becomes `feat(core): add nine native-agent variants to AgentError`.

**Reasoning:** Splitting variant additions across Tasks 6, 15, 18 leaves intermediate commits with broken classifier exhaustiveness or undefined-symbol references. One commit-on-Task-6 keeps every later task self-consistent.

---

### S2-3: `install_plugin_agents` actual signature

**Severity:** BLOCKING

**Affected plan section:** Task 18 step 1 (test fixture, lines 2244-2299), step 5 (dispatch wrapper, lines 2514-2541), step 4 (native inner body — passes ctx + opts shape, lines 2341-2511). Also Task 19 (CLI presenter, lines 2566+) which consumes the result type.

**What the plan says:** The plan envisions a refactored signature `pub fn install_plugin_agents(&self, project, marketplace, ctx: &PluginInstallContext, opts: AgentInstallOptions) -> InstallAgentsResult` and a thin format-dispatch body that calls either `install_native_kiro_cli_agents_inner` or `install_translated_agents_inner`. Tests call `svc.install_plugin_agents(&project, "marketplace-x", &ctx, AgentInstallOptions { force: false, accept_mcp: false })`. Task 18 step 5 includes a parenthetical "(If the existing function signature differs ...)" caveat but does not specify the resolution.

**What's actually true:** `crates/kiro-market-core/src/service/mod.rs:1163-1173` defines:

```rust
pub fn install_plugin_agents(
    &self,
    project: &crate::project::KiroProject,
    plugin_dir: &Path,
    scan_paths: &[String],
    mode: InstallMode,
    accept_mcp: bool,
    marketplace: &str,
    plugin: &str,
    version: Option<&str>,
) -> InstallAgentsResult {
```

— 8 positional non-self parameters, taking `mode: InstallMode` (an enum, not a bool), and **not** taking a `PluginInstallContext` at all. The body iterates `discover_agents_in_dirs(plugin_dir, scan_paths)` and calls `install_agent` / `install_agent_force` per file. Existing CLI and test callers pass each parameter individually.

**Required revision:** Adopt review-findings option C — **keep the existing positional signature**, dispatch internally:

```rust
pub fn install_plugin_agents(
    &self,
    project: &crate::project::KiroProject,
    plugin_dir: &Path,
    scan_paths: &[String],
    mode: InstallMode,
    accept_mcp: bool,
    marketplace: &str,
    plugin: &str,
    version: Option<&str>,
    format: Option<crate::plugin::PluginFormat>,  // NEW — only addition
) -> InstallAgentsResult {
    match format {
        Some(crate::plugin::PluginFormat::KiroCli) => self.install_native_kiro_cli_agents_inner(
            project, plugin_dir, scan_paths, mode, accept_mcp, marketplace, plugin, version,
        ),
        None => self.install_translated_agents_inner(
            project, plugin_dir, scan_paths, mode, accept_mcp, marketplace, plugin, version,
        ),
    }
}
```

Replace the existing `install_plugin_agents` body verbatim with the call to a renamed private `install_translated_agents_inner` that takes the same 8 positional params (this is purely a rename + indirection — no body change). Add `install_native_kiro_cli_agents_inner` with the **same** 8 positional params. The signatures match so the dispatch is one match arm per arm body.

The CLI and tests update by adding one argument: `format: ctx.format` (read from `PluginInstallContext.format` per Task 7) at every existing call site. Estimated affected sites: 1 in CLI (`crates/kiro-market/src/commands/install.rs`), N in tests (`grep "install_plugin_agents(" crates/`). The change is purely additive — no existing-caller signature breakage beyond appending one arg.

Update Task 18's test fixture call to:

```rust
let result = svc.install_plugin_agents(
    &project,
    tmp.path(),                 // plugin_dir
    &["./agents/".to_string()], // scan_paths
    crate::service::InstallMode::New,  // see S2-4
    false,                      // accept_mcp
    "marketplace-x",
    "p",                        // plugin name from manifest
    None,                       // version
    Some(crate::plugin::PluginFormat::KiroCli),  // NEW
);
```

Replace `AgentInstallOptions { force, accept_mcp }` with the positional `mode + accept_mcp` pair throughout Stage 2 plan tests. If you want a struct façade for ergonomics, keep it as a build-the-args-locally pattern in the test, NOT as a public type.

**Reasoning:** Refactoring the existing positional signature into `(ctx, opts)` form (review-findings option A) cascades through every CLI callsite and into Tauri (when wired), expanding scope by an order of magnitude. The dispatcher-on-format addition (option C) costs one new parameter on one function, and CLI / Tauri callers update by reading `ctx.format` and passing it. This matches the actual codebase convention of explicit positional params with `#[allow(clippy::too_many_arguments)]` only when justified (see `service/mod.rs:1159`'s precedent).

---

### S2-4: `force: bool` → `mode: InstallMode` everywhere

**Severity:** BLOCKING

**Affected plan section:** Task 18 step 3 (`AgentInstallOptions` definition, lines 2308-2317), Task 18 step 4 (`install_native_kiro_cli_agents_inner` body — uses `opts.force`, lines 2341-2511), Task 18 step 5 (dispatch wrapper, lines 2514-2541), Task 10 step 3 (`install_native_agent` body — takes `force: bool`, lines 1346-1483), Task 15 step 3 (`install_native_companions` body — takes `force: bool`, lines 1843-2053).

**What the plan says:** Plan uses `force: bool` everywhere, and introduces `pub struct AgentInstallOptions { pub force: bool, pub accept_mcp: bool }` at Task 18 step 3.

**What's actually true:** `crates/kiro-market-core/src/service/mod.rs:206-227` defines:

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InstallMode {
    New,
    Force,
}

impl InstallMode {
    #[must_use]
    pub fn is_force(self) -> bool {
        matches!(self, Self::Force)
    }
}

impl From<bool> for InstallMode {
    fn from(force: bool) -> Self {
        if force { Self::Force } else { Self::New }
    }
}
```

`install_plugin_agents` already takes `mode: InstallMode` (see S2-3). Existing translated-path internal calls dispatch via `if mode.is_force() { install_agent_force(...) } else { install_agent(...) }`.

**Required revision:**

1. **Drop the `AgentInstallOptions` struct** from Task 18 step 3. Stage 2's service-layer entrypoint already takes `mode + accept_mcp` positionally; do not introduce a wrapper type.

2. **Project-layer install methods take `mode: InstallMode`, not `force: bool`.** Update the planned signatures:

```rust
// Task 10 — install_native_agent:
pub fn install_native_agent(
    &self,
    bundle: &crate::agent::NativeAgentBundle,
    marketplace: &str,
    plugin: &str,
    version: Option<&str>,
    source_hash: &str,
    mode: crate::service::InstallMode,   // was: force: bool
) -> Result<InstalledNativeAgentOutcome, AgentError> {
    // Inside body, replace `if force { ... }` with `if mode.is_force() { ... }`.
    // Replace `forced_overwrite = true; if !force { return Err(...) }` patterns
    // with `if !mode.is_force() { return Err(...) } else { forced_overwrite = true; }`.
    ...
}

// Task 15 — install_native_companions: same shape change.
// Task 7 (Stage 3) — install_steering_file: same shape change (see S3-7).
```

3. **`install_native_kiro_cli_agents_inner` and `install_translated_agents_inner` both take `mode + accept_mcp` positionally** (matching `install_plugin_agents`). Inside, pass `mode` straight through to `project.install_native_agent` / `install_agent`.

4. **Tests construct `InstallMode::New` / `InstallMode::Force` explicitly** rather than `force: false` / `force: true`. The `From<bool>` impl exists for CLI ergonomics but tests should use the typed values.

**Reasoning:** Mixing `force: bool` (new) with `mode: InstallMode` (existing) creates two parallel conventions for the same concept. The codebase already standardized on `InstallMode`; Stage 2 should extend that, not fork.

---

### S2-5: `InstallAgentsResult` field shape (option A — additive, not rename)

**Severity:** BLOCKING

**Affected plan section:** Task 18 step 3 (lines 2319-2330), Task 19 (CLI presenter — consumes `result.installed_agents`, lines 2566+).

**What the plan says:** The plan redefines `InstallAgentsResult`:

```rust
pub struct InstallAgentsResult {
    pub installed_agents: Vec<crate::project::InstalledNativeAgentOutcome>,
    pub installed_companions: Option<crate::project::InstalledNativeCompanionsOutcome>,
    pub skipped: Vec<SkippedAgent>,
    pub failed: Vec<FailedAgent>,
    pub warnings: Vec<DiscoveryWarning>,
}
```

Renames `installed: Vec<String>` → `installed_agents: Vec<InstalledNativeAgentOutcome>`, adds `installed_companions: Option<...>`, references `SkippedAgent` and `DiscoveryWarning` types.

**What's actually true:** `crates/kiro-market-core/src/service/mod.rs:365-376`:

```rust
#[derive(Clone, Debug, Default, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct InstallAgentsResult {
    pub installed: Vec<String>,
    pub skipped: Vec<String>,
    pub failed: Vec<FailedAgent>,
    pub warnings: Vec<InstallWarning>,
}
```

— field is `installed: Vec<String>` (not `installed_agents`), `skipped: Vec<String>` (not `Vec<SkippedAgent>`), `warnings: Vec<InstallWarning>` (not `DiscoveryWarning`). `SkippedAgent` doesn't exist as a type (skipped tracks names only, matching `installed`); `DiscoveryWarning` doesn't exist (see S2-7).

**Required revision:** Adopt option (A) from the review-findings — **additive, not breaking**. Extend `InstallAgentsResult` with new fields rather than renaming:

```rust
#[derive(Clone, Debug, Default, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct InstallAgentsResult {
    /// Agent names successfully installed (translated path) OR
    /// names of native agents successfully installed. Matches existing
    /// CLI / Tauri consumers; populated by both translated and native
    /// dispatch arms.
    pub installed: Vec<String>,
    /// Agent names that were already installed (translated path's
    /// `AlreadyInstalled` route OR native path's `was_idempotent: true`
    /// outcomes). Same name-only shape for backward compat.
    pub skipped: Vec<String>,
    pub failed: Vec<FailedAgent>,
    pub warnings: Vec<InstallWarning>,
    // NEW fields (Stage 2):
    /// Per-native-agent rich outcome (with `forced_overwrite`, `was_idempotent`,
    /// hashes). Only populated by the native install path; empty for translated.
    /// CLI presenters that want the rich detail consume this; legacy presenters
    /// keep using `installed: Vec<String>`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub installed_native: Vec<crate::project::InstalledNativeAgentOutcome>,
    /// Per-plugin native companion bundle outcome. None for translated plugins
    /// or for native plugins with zero companion files.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installed_companions: Option<crate::project::InstalledNativeCompanionsOutcome>,
}
```

`installed_native` is the new place for rich per-agent outcomes; `installed` stays as the name-only list for both paths so existing presenters keep working without changes. The native dispatch arm pushes to BOTH `installed` (the name) and `installed_native` (the outcome) for each successful install.

**For Task 19 (CLI presenter):** The minimum change is to render `installed_companions` if present:

```rust
// Existing rendering of `result.installed` (per-name) stays unchanged.
// NEW: companion bundle row.
if let Some(companions) = &result.installed_companions {
    let suffix = if companions.was_idempotent { " (unchanged)" }
                  else if companions.forced_overwrite { " (forced)" }
                  else { "" };
    println!(
        "  {} companion bundle for {} ({} file{}){}",
        "✓".green(),
        companions.plugin,
        companions.files.len(),
        if companions.files.len() == 1 { "" } else { "s" },
        suffix
    );
}
```

If a UI consumer wants the rich per-agent detail later, it reads `installed_native` instead of (or alongside) `installed`. No existing call site breaks.

**Reasoning:** Renaming `installed` → `installed_agents` is a breaking change touching every CLI / Tauri / test callsite that reads the field. The additive shape lets Stage 2 ship without reworking presenter code, and the rename can happen in a later focused PR if there's appetite.

---

### S2-6: `FailedAgent` actual shape

**Severity:** BLOCKING

**Affected plan section:** Task 18 step 3 (`FailedAgent` shape, lines 2332-2336), Task 18 step 4 (constructions of `FailedAgent` inside the native inner body — many sites, lines 2403-2461 + 2484-2506), Task 19 (CLI presenter consumption).

**What the plan says:**

```rust
pub struct FailedAgent {
    pub name: Option<String>,
    pub source_path: std::path::PathBuf,
    pub error: crate::error::AgentError,   // typed
}
```

Constructions in Task 18 step 4 use `FailedAgent { name: Some(bundle.name.clone()), source_path: f.source.clone(), error: ... }`.

**What's actually true:** `crates/kiro-market-core/src/service/mod.rs:378-386`:

```rust
#[derive(Clone, Debug, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct FailedAgent {
    pub name: String,                  // NOT Option<String>
    pub error: String,                 // pre-rendered, NOT typed
}
```

— two fields, both `String`. The pre-rendered `error: String` is constructed via `crate::error::error_full_chain(&e)` at the existing translated-path call site (`service/mod.rs:1201-1204`).

**Required revision:** Upgrade `FailedAgent` to the typed shape (review-findings option) — this IS a breaking change for the existing translated path's construction site at `service/mod.rs:1201`, but the translated path has exactly one such site so the migration cost is bounded.

Update `FailedAgent`:

```rust
#[derive(Clone, Debug, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct FailedAgent {
    /// Agent name if parse / discovery reached the point of having one;
    /// otherwise `None` (caller falls back to displaying source_path).
    pub name: Option<String>,
    /// Source file or directory the failure originated at. Always available;
    /// even pre-parse failures know where the file lived on disk.
    pub source_path: std::path::PathBuf,
    /// Typed error so frontends can branch on cause rather than substring-match
    /// a pre-rendered string. Render via `error_full_chain(&self.error)` for
    /// human display.
    pub error: crate::error::AgentError,
}
```

**Migration of existing translated-path callsite (`service/mod.rs:1201-1204`):**

Before:
```rust
result.failed.push(FailedAgent {
    name: path.display().to_string(),
    error: crate::error::error_full_chain(&e),
});
```

After:
```rust
result.failed.push(FailedAgent {
    name: None,                         // pre-parse failure path
    source_path: path.clone(),
    error: match e {
        crate::error::Error::Agent(agent_err) => agent_err,
        // Wrap other top-level errors into a generic AgentError variant:
        other => crate::error::AgentError::ParseFailed {
            path: path.clone(),
            failure: crate::agent::ParseFailure::Io(other.to_string()),
        },
    },
});
```

The wrapping of non-`Error::Agent` variants requires a small extension to `ParseFailure` (or accepting that this site will need a new `AgentError` variant for "wrapped infrastructure failure"). Alternative: introduce one more `AgentError` variant `InstallFailed { path: PathBuf, source: Box<crate::error::Error> }` and use it as the catch-all — this keeps the migration purely additive.

**Cost estimate:** 1 production callsite update + ~5 test assertions that read `failed[0].error` as a string (now: render via `error_full_chain` for the same string output). Net: ~10 line touches.

**For Task 18 step 4:** Construct `FailedAgent` exactly as planned (`name: Option<String>`, `source_path: PathBuf`, `error: AgentError`). The plan was written for the future shape; just add the migration step for the existing translated callsite as a Task 6 sibling commit (so all `FailedAgent` constructions in the codebase use the new shape from one commit forward).

**Reasoning:** Upgrading to typed errors is a one-time refactor that pays off every time the frontend wants to programmatically branch on error cause. CLAUDE.md's typed-error discipline already pushed in this direction; the existing pre-rendered shape is a pre-Stage-1 artifact that Stage 2 is the right time to clean up.

---

### S2-7: `DiscoveryWarning` doesn't exist; use `InstallWarning`

**Severity:** BLOCKING

**Affected plan section:** Task 18 step 3 (`InstallAgentsResult.warnings: Vec<DiscoveryWarning>`, line 2329).

**What the plan says:** Plan references `DiscoveryWarning` as the warnings field type. Same in Stage 3 (see S3-2).

**What's actually true:** `crates/kiro-market-core/src/service/mod.rs:394-421`:

```rust
#[derive(Clone, Debug, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[non_exhaustive]
pub enum InstallWarning {
    UnmappedTool { agent: String, tool: String, reason: crate::agent::tools::UnmappedReason },
    AgentParseFailed { path: PathBuf, failure: crate::agent::ParseFailure },
    McpServersRequireOptIn { agent: String, transports: Vec<String> },
}
```

— that's the actual type. `DiscoveryWarning` doesn't exist anywhere in the codebase.

**Required revision:** In Task 18 step 3 (and per S2-5, in the kept-additive `InstallAgentsResult`):

```rust
pub warnings: Vec<InstallWarning>,    // was: Vec<DiscoveryWarning>
```

Inside `install_native_kiro_cli_agents_inner`, push warnings using existing variants:

- For native parse failures classified as warnings (rare — most native parse failures are `failed`, not `warnings`), `InstallWarning::AgentParseFailed { path, failure: ParseFailure::... }`. Note that `ParseFailure` may need new variants to represent native-parse failure modes (`InvalidJson`, `MissingName` from native context). Either extend `ParseFailure` with native-aware variants, or — recommended — keep `ParseFailure` as the translated-path-only failure type and route native parse failures through `result.failed` only (typed `AgentError::NativeManifestParseFailed` etc per S2-2). The latter avoids cross-pollution.

- For MCP gate (per S2-12 below), `InstallWarning::McpServersRequireOptIn { agent, transports }` is the existing variant — reuse it directly.

**Reasoning:** `DiscoveryWarning` was a planning placeholder. The real type already covers everything Stage 2 needs.

---

### S2-8: `test_marketplace_service()` doesn't exist; use `temp_service()`

**Severity:** BLOCKING

**Affected plan section:** Task 18 step 1 (test fixture, line 2269), Task 20 (end-to-end integration test fixture, lines 2652+).

**What the plan says:** Tests do `let svc = crate::service::test_support::test_marketplace_service();` and assume it returns `MarketplaceService` directly.

**What's actually true:** `crates/kiro-market-core/src/service/test_support.rs:64`:

```rust
pub fn temp_service() -> (TempDir, MarketplaceService) {
    ...
}
```

— actual function name + returns a tuple.

**Required revision:** Replace every test fixture call:

```rust
let (_tempdir, svc) = crate::service::test_support::temp_service();
```

The `_tempdir` keeps the temp marketplace cache directory alive for the test's lifetime; dropping it cleans up. The leading underscore quiets unused-variable warnings if the test doesn't otherwise touch the tempdir.

**Reasoning:** Mechanical rename. The actual fixture also enforces a "no network" stub backend, which Stage 2 / Stage 3 tests should rely on (their fixtures don't need network access).

---

### S2-9: `NativeAgentBundle` must store raw bytes for verbatim copy

**Severity:** BLOCKING

**Affected plan section:** Task 5 (`parse_native.rs` and `NativeAgentBundle`, lines 711-931), Task 10 step 3 (uses `serde_json::to_vec_pretty(&bundle.raw_json)?` to write the destination JSON, line 1428).

**What the plan says:** `NativeAgentBundle` carries `pub raw_json: serde_json::Value`; the install path re-serializes via `serde_json::to_vec_pretty(&bundle.raw_json)` and writes that to the destination.

**What's actually true (design intent):** `2026-04-23-kiro-cli-native-plugin-import-design.md` § "Out of Scope" line 47 explicitly says **"v1 preserves them verbatim."** Re-serialization changes byte content (whitespace, field ordering, escape choices) for any source JSON not already in canonical pretty form. Two installs of the same source on the same machine could produce different `installed_hash` values depending on source whitespace.

**Required revision:**

1. **Add `raw_bytes: Vec<u8>` to `NativeAgentBundle`** (Task 5 step 3 — schema):

```rust
#[derive(Debug, Clone)]
pub struct NativeAgentBundle {
    pub agent_json_source: PathBuf,
    pub scan_root: PathBuf,
    pub name: String,
    pub mcp_servers: BTreeMap<String, McpServerConfig>,
    /// Parsed JSON, used for projection / validation only. Not the source
    /// of truth for what lands on disk.
    pub raw_json: serde_json::Value,
    /// Source bytes preserved exactly. The install path writes these to
    /// the destination so the installed file matches the source byte-for-byte
    /// (per design doc § "Out of Scope": v1 preserves verbatim).
    pub raw_bytes: Vec<u8>,
}
```

In `parse_native_kiro_agent_file`, populate both: read the file bytes once, parse `raw_json` from the bytes (zero-copy via `serde_json::from_slice`), store both:

```rust
pub fn parse_native_kiro_agent_file(
    json_path: &Path,
    scan_root: &Path,
) -> Result<NativeAgentBundle, NativeParseFailure> {
    let raw_bytes = std::fs::read(json_path).map_err(NativeParseFailure::IoError)?;
    let raw_json: serde_json::Value =
        serde_json::from_slice(&raw_bytes).map_err(NativeParseFailure::InvalidJson)?;
    // ... extract name, mcp_servers, validate ...
    Ok(NativeAgentBundle {
        agent_json_source: json_path.to_path_buf(),
        scan_root: scan_root.to_path_buf(),
        name,
        mcp_servers,
        raw_json,
        raw_bytes,
    })
}
```

2. **Task 10 step 3 — staging write** uses `raw_bytes`:

```rust
// Inside install_native_agent's stage_native_agent_file helper (per P-3):
let staging_json = staging.join(format!("{}.json", &bundle.name));
fs::write(&staging_json, &bundle.raw_bytes)?;     // verbatim, NOT to_vec_pretty(&raw_json)
```

3. **Task 16 doctest verifying verbatim preservation** (add a new test in Task 10 or Task 11 — not in the plan, but worth pinning): construct a source JSON with non-canonical whitespace (e.g., extra indent, fields in unusual order), install it, read the destination, assert byte-equality with the source.

**Reasoning:** The verbatim preservation is the design's explicit promise to native plugin authors — they author Kiro's native JSON exactly as they want it stored, and the marketplace pipeline doesn't reformat it. Without `raw_bytes`, every cross-machine install produces drift in `installed_hash` based on parse / re-emit nondeterminism.

---

### S2-10: Task 10 (`install_native_agent`) bundle of fixes

**Severity:** BLOCKING

**Affected plan section:** Task 10, especially step 3 body (lines 1346-1483).

**What the plan says (in summary):** Body inlines staging-write + force-clear + rename + post-rename hash + tracking-write in one ~140-line function. Uses `staging.join("agent.json")`, re-serializes `raw_json`, hashes via `hash_artifact(&self.agents_dir(), &[<name>.json])` AFTER the rename, and uses `let _ = std::fs::remove_dir_all(&staging);` for cleanup.

**What's actually true (Stage 1 patterns to inherit):**

Per **P-1**, Stage 1's translated-agent install (`install_agent_inner`) computes `installed_hash` against staging BEFORE the destructive rename. The agent is staged with the final-layout filename `<name>.json` (NOT `agent.json`) so the relative-path list is identical in both bases (`crates/kiro-market-core/src/project.rs:577-583`).

Per **P-3**, `install_agent_inner` is decomposed into `stage_agent_files`, `promote_staged_agent`, `synthesize_companion_entry`. A monolithic `install_native_agent` will trigger `clippy::too_many_lines` and lead to `#[allow(...)]` (CLAUDE.md violation).

CLAUDE.md forbids `let _ = ...` discarding a `Result`; use either `if let Err(e) = ... { warn!(...) }` or propagate via `?`.

**Required revision:** Restructure Task 10 step 3 to mirror Stage 1's translated-path shape:

1. **Stage with final filename, hash pre-rename** — inside the file-lock closure:

```rust
let staging = self.fresh_agent_staging_dir(&bundle.name);
let staging_json = staging.join(format!("{}.json", &bundle.name));
fs::create_dir_all(&staging)?;
if let Err(e) = fs::write(&staging_json, &bundle.raw_bytes) {  // see S2-9
    if let Err(cleanup_err) = fs::remove_dir_all(&staging) {
        warn!(path = %staging.display(), error = %cleanup_err,
              "failed to clean staging after write failure");
    }
    return Err(e.into());
}

// Compute installed_hash on staging BEFORE any destructive op.
let json_rel = std::path::PathBuf::from(format!("{}.json", &bundle.name));
let installed_hash = match crate::hash::hash_artifact(&staging, &[json_rel.clone()]) {
    Ok(h) => h,
    Err(e) => {
        warn!(name = %bundle.name, error = %e,
              "installed_hash computation failed on staging; cleaning up");
        if let Err(cleanup_err) = fs::remove_dir_all(&staging) {
            warn!(path = %staging.display(), error = %cleanup_err,
                  "failed to clean staging after hash failure");
        }
        return Err(e.into());
    }
};
```

2. **Force-clear + rename block** runs only after hash succeeds. Adopt **P-6 backup-then-swap** for force mode:

```rust
fs::create_dir_all(self.agents_dir())?;
let json_target = self.agents_dir().join(format!("{}.json", &bundle.name));
let backup_target = json_target.with_extension("json.kiro-bak");

// Backup phase (only when overwriting).
let mut had_backup = false;
if mode.is_force() && json_target.exists() {
    if let Err(e) = fs::rename(&json_target, &backup_target) {
        if let Err(cleanup_err) = fs::remove_dir_all(&staging) {
            warn!(...);
        }
        return Err(e.into());
    }
    had_backup = true;
} else if !mode.is_force() && json_target.exists() {
    if let Err(cleanup_err) = fs::remove_dir_all(&staging) {
        warn!(...);
    }
    return Err(AgentError::OrphanFileAtDestination { path: json_target });
}

// Promote phase.
if let Err(e) = fs::rename(&staging_json, &json_target) {
    // Restore backup if we made one.
    if had_backup {
        if let Err(restore_err) = fs::rename(&backup_target, &json_target) {
            warn!(error = %restore_err,
                  "failed to restore backup after rename failure");
        }
    }
    if let Err(cleanup_err) = fs::remove_dir_all(&staging) {
        warn!(...);
    }
    return Err(e.into());
}
if let Err(cleanup_err) = fs::remove_dir_all(&staging) {
    warn!(path = %staging.display(), error = %cleanup_err,
          "failed to clean empty staging dir");
}
```

3. **Tracking write commits the new state OR restores the backup:**

```rust
installed.agents.insert(bundle.name.clone(), InstalledAgentMeta { ..., dialect: AgentDialect::Native, source_hash: Some(source_hash.to_string()), installed_hash: Some(installed_hash.clone()) });

if let Err(e) = self.write_agent_tracking(&installed) {
    // Tracking write failure: restore backup so the user keeps the old install.
    if had_backup {
        // Remove the new file we just renamed in.
        if let Err(rb_err) = fs::remove_file(&json_target) {
            warn!(...);
        }
        if let Err(restore_err) = fs::rename(&backup_target, &json_target) {
            warn!(error = %restore_err, "failed to restore backup after tracking failure");
        }
    } else {
        // No prior install — just clean up the new file.
        if let Err(rb_err) = fs::remove_file(&json_target) {
            warn!(...);
        }
    }
    return Err(e.into());
}

// Commit phase: drop the backup.
if had_backup {
    if let Err(e) = fs::remove_file(&backup_target) {
        warn!(path = %backup_target.display(), error = %e,
              "failed to clean up backup after successful install (orphan .kiro-bak file left)");
    }
}
```

4. **Decompose into helpers per P-3** — the body above is ~80 lines and getting tight. Extract:

```rust
fn stage_native_agent_file(
    &self,
    bundle: &NativeAgentBundle,
) -> crate::error::Result<(PathBuf, PathBuf, String)> {
    // returns (staging_dir, json_rel, installed_hash)
}

fn promote_native_agent(
    &self,
    staging: &Path,
    json_rel: &Path,
    json_target: &Path,
    mode: InstallMode,
) -> Result<bool, AgentError> {  // returns had_backup
}
```

Then `install_native_agent` orchestrates: stage → check tracking-side conflicts → promote → write tracking (with rollback). Outer function stays under the line threshold without `#[allow]`.

5. **Tests in Task 10 step 1 are mostly fine but add an assertion** for verbatim preservation (per S2-9): write a non-canonical source JSON, install, read destination, assert byte-equality.

**Reasoning:** The original Task 10 was written before Stage 1 ironed out the staging-before-rename and helper-extraction patterns. Adopting them prevents the same data-loss regression Stage 1 had to fix mid-PR (commits `19e97c3` + `925990f`).

---

### S2-11: Task 15 (`install_native_companions`) bundle of fixes

**Severity:** BLOCKING

**Affected plan section:** Task 15 step 3 (lines 1843-2053).

**What the plan says (in summary):**

- Iterates `files`, computes per-file relative paths via `f.source.strip_prefix(&f.scan_root).map_err(|_| io::Error::new(InvalidInput, ...))`.
- Stages all files in a single `.staging-companions-{plugin}` directory.
- Per-file rename loop: `std::fs::remove_file(&dest)` (unconditional) → `std::fs::rename(&staged, &dest)`.
- Computes `installed_hash` over the placed files at `agents_dir` AFTER the rename loop.
- Inserts new `InstalledNativeCompanionsMeta` and overwrites any prior entry's `files` list — old files NOT in the new set linger on disk.
- `let _ = std::fs::remove_dir_all(&staging);` and `let _ = std::fs::remove_dir_all(&staging);` cleanup.
- Function takes 6 params; will trigger `clippy::too_many_arguments`.

**What's actually true / what should change:**

A. **Multi-scan-root assumption (review-findings #10)**. The body assumes all `files` share the scan_root passed to `hash_artifact`. The service-layer caller (Task 18 step 4 lines 2469-2491) passes `companion_files[0].scan_root` and computes rel paths from `f.scan_root` per-file — these can disagree when `manifest.agents = ["./agents/", "./extra-agents/"]`.

B. **Orphan files on disk (review-findings #9)**. Replacing `meta.files = new_files` without removing files in `(old_files \ new_files)` leaks old files.

C. **`let _ = ...` discards Result (CLAUDE.md violation)** — applies to staging cleanup at the end.

D. **`.expect(...)` in production code (CLAUDE.md violation)** — Task 18 step 4 line 2475 has `.expect("companion source under scan_root")` and line 2431 has `.expect("agent file has a name")`.

E. **Post-rename hash without backup (P-1 + P-6)**. If `hash_artifact` over the placed files fails (transient I/O), rollback removes the new files but Plugin A's previous bundle was already deleted at the per-file rename step. Force-mode data loss.

F. **Param count → adopt CompanionInput pattern (P-2)**. Function takes `(files: &[DiscoveredNativeFile], marketplace, plugin, version, source_hash, force_or_mode)` — 6 params plus `&self`. Bundle into a `NativeCompanionsInput<'a>`.

G. **Helper decomposition (P-3)**. Stage 2 monolithic function will need `#[allow(too_many_lines)]` without extraction.

**Required revision:**

1. **Reject multi-scan-root native plugins** (review-findings option (a) for #10). Add to `AgentError` (per S2-2 you already added 9 variants; add a 10th):

```rust
#[error(
    "native plugin spans multiple agent scan roots ({}); v1 supports a \
     single scan root only",
    .roots.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(", ")
)]
MultipleScanRootsNotSupported { roots: Vec<PathBuf> },
```

In `install_native_kiro_cli_agents_inner` (Task 18), check at the top of the companion-install block:

```rust
let unique_roots: std::collections::HashSet<&PathBuf> =
    companion_files.iter().map(|f| &f.scan_root).collect();
if unique_roots.len() > 1 {
    let roots: Vec<PathBuf> = unique_roots.into_iter().cloned().collect();
    result.failed.push(FailedAgent {
        name: None,
        source_path: ctx.plugin_dir.clone(),
        error: AgentError::MultipleScanRootsNotSupported { roots },
    });
} else if !companion_files.is_empty() {
    // Single-root path — call install_native_companions.
}
```

`install_native_companions` itself takes a single `scan_root: &Path` parameter (not a `&[DiscoveredNativeFile]`-with-implicit-roots), and a `&[PathBuf]` of relative paths under that root. Caller responsibility to assemble.

2. **Diff-and-remove for orphan cleanup (review-findings #9)**:

```rust
// Inside the lock, BEFORE staging:
let to_remove: Vec<PathBuf> = installed.native_companions
    .get(plugin)
    .map(|prior| prior.files.iter()
        .filter(|p| !new_rel_set.contains(*p))
        .cloned()
        .collect())
    .unwrap_or_default();
```

After the promote phase succeeds, walk `to_remove` and best-effort delete each file under `agents_root`. Log warnings on failure but don't abort — the new install succeeded; orphan cleanup is opportunistic.

3. **Replace `.expect()` with proper error returns**:

```rust
// Task 18 step 4 line 2475 — companion rel path:
let rel = match f.source.strip_prefix(&f.scan_root) {
    Ok(p) => p.to_path_buf(),
    Err(_) => {
        result.failed.push(FailedAgent {
            name: None,
            source_path: f.source.clone(),
            error: AgentError::PathOwnedByOtherPlugin {
                // Or whichever variant best describes "discovered file is not under its declared scan_root".
                // This shouldn't happen if discovery is correct — return a typed error rather than panic.
                path: f.source.clone(),
                owner: "<discovery-error>".to_string(),
            },
        });
        continue;
    }
};

// Task 18 step 4 line 2431 — agent filename:
let filename = match f.source.file_name() {
    Some(n) => std::path::PathBuf::from(n),
    None => {
        result.failed.push(FailedAgent {
            name: None,
            source_path: f.source.clone(),
            error: AgentError::NativeManifestInvalidName {
                path: f.source.clone(),
                reason: "source path has no file name".into(),
            },
        });
        continue;
    }
};
```

4. **Replace `let _ = remove_dir_all(...)` with `if let Err(e) = ... { warn!(...) }`** at every cleanup site.

5. **Adopt P-6 backup-then-swap for force mode** in the per-file rename loop. Each existing destination file gets a `.kiro-bak` sibling rename before the staging rename, restored on any later-step failure, deleted on success. The hash + tracking write happen between promote and commit; failure restores backups.

6. **Adopt P-2 param-bundle struct** for `install_native_companions`:

```rust
struct NativeCompanionsInput<'a> {
    scan_root: &'a Path,
    rel_paths: &'a [PathBuf],
    marketplace: &'a str,
    plugin: &'a str,
    version: Option<&'a str>,
    source_hash: &'a str,
    mode: crate::service::InstallMode,
}

pub fn install_native_companions(
    &self,
    input: &NativeCompanionsInput<'_>,
) -> Result<InstalledNativeCompanionsOutcome, AgentError> { ... }
```

7. **Decompose into helpers per P-3**:

- `stage_native_companion_files` — copy each file into `.staging-companions-{plugin}` preserving rel layout, returns `(staging, installed_hash_pre_rename)`.
- `promote_native_companions` — backup existing destinations, rename staged into place, return `(placed: Vec<PathBuf>, backups: Vec<PathBuf>)` so the orchestrator can roll back.
- `synthesize_companion_tracking_entry` — update `installed.native_companions` with new files + hashes; the existing `synthesize_companion_entry` from Stage 1 (translated path) is similar but per-translated-agent — keep them separate, both use the same `InstalledNativeCompanionsMeta` type.

**Reasoning:** Task 15 as originally drafted has six independent CLAUDE.md / correctness violations. Restructuring to mirror Stage 1's translated-agent install pattern eliminates them all and keeps the function shape consistent with the rest of the project layer.

---

### S2-12: MCP gate parity with translated path

**Severity:** BLOCKING (semantic divergence) / COSMETIC (depending on whether divergence is intentional)

**Affected plan section:** Task 18 step 4 — MCP gate inside `install_native_kiro_cli_agents_inner` (lines 2413-2427).

**What the plan says:**

```rust
let has_stdio = bundle.mcp_servers.values().any(|s| s.is_stdio());
if has_stdio && !opts.accept_mcp {
    result.failed.push(FailedAgent {
        name: Some(bundle.name.clone()),
        source_path: f.source.clone(),
        error: crate::error::AgentError::McpRequiresAccept { name: bundle.name.clone() },
    });
    continue;
}
```

— gates only Stdio transports, routes the gated agent to `failed`.

**What's actually true:** Existing translated path (`crates/kiro-market-core/src/service/mod.rs:1209-1228`) gates ANY non-empty `mcp_servers` (Stdio + Http + Sse) and routes through `warnings: Vec<InstallWarning>` with the `InstallWarning::McpServersRequireOptIn { agent, transports }` variant + `continue` (which routes to neither `installed` nor `failed` — effectively skipped):

```rust
if !accept_mcp && !def.mcp_servers.is_empty() {
    let transports: Vec<String> = def.mcp_servers.values()
        .map(|cfg| cfg.transport_label().to_owned()).collect();
    result.warnings.push(InstallWarning::McpServersRequireOptIn {
        agent: def.name.clone(), transports,
    });
    continue;
}
```

The user UX difference is significant: translated MCP plugins produce a non-fatal warning; the install completes successfully with one item skipped. Native MCP plugins under the planned shape produce a hard failure that turns CLI exit code non-zero.

**Required revision:** Match the translated-path behavior in the native path (review-findings recommendation):

```rust
if !accept_mcp && !bundle.mcp_servers.is_empty() {
    let transports: Vec<String> = bundle.mcp_servers.values()
        .map(|cfg| cfg.transport_label().to_owned()).collect();
    result.warnings.push(InstallWarning::McpServersRequireOptIn {
        agent: bundle.name.clone(),
        transports,
    });
    continue;
}
```

Drop the `AgentError::McpRequiresAccept` variant from S2-2's list (it becomes unused). The existing `InstallWarning::McpServersRequireOptIn` variant is already in scope and visible to existing CLI presenters; no UX disruption.

**Reasoning:** Two different MCP UX behaviors for two install paths is user-hostile. A single user installing both translated and native plugins should see one MCP gate convention. If a future stricter policy is desired, tighten BOTH paths together in a focused PR.

---

### S2-13: Out-of-scope items deliberately deferred from Stage 2

The following Stage 2 plan items are intentionally NOT amended here; they remain valid as-written or are explicitly out of scope for the post-Stage-1 PR:

- **Task 1, 2, 3, 4** — `PluginFormat` enum, `AgentDialect::Native` variant, discovery helpers. These don't touch any drift items; execute as planned.
- **Task 5** — `parse_native.rs`. Modify only per S2-9 (add `raw_bytes` field).
- **Task 7** — `PluginInstallContext` extension. Execute as planned (add `format: Option<PluginFormat>`).
- **Task 9** — Outcome struct definitions. Execute as planned.
- **Tasks 11-14** — `install_native_agent` collision tests. Update `force: bool` → `mode: InstallMode` per S2-4; otherwise execute as planned.
- **Task 16** — `install_native_companions` collision tests. Update `force: bool` → `mode: InstallMode`; otherwise execute as planned.
- **Task 17** — MCP gate enforcement at the service layer. Per S2-12, this becomes a no-op (the warning route is in `install_native_kiro_cli_agents_inner` directly).
- **Task 20, 21** — End-to-end integration test + final verification. Update fixture call per S2-8; otherwise execute as planned.

---

## Stage 3 amendments

### S3-1: `SteeringError` infrastructure variants

**Severity:** BLOCKING

**Affected plan section:** Task 2 step 1 (`SteeringError` definition, lines 142-185).

**What the plan says:**

```rust
#[derive(Debug, Error)]
pub enum SteeringError {
    SourceReadFailed { path: PathBuf, #[source] source: io::Error },
    PathOwnedByOtherPlugin { rel: PathBuf, owner: String },
    OrphanFileAtDestination { path: PathBuf },
    ContentChangedRequiresForce { rel: PathBuf },
    TrackingIoFailed { path: PathBuf, #[source] source: io::Error },

    #[error(transparent)]
    Hash(#[from] crate::hash::HashError),

    #[error(transparent)]
    Io(#[from] io::Error),

    #[error(transparent)]
    Json(#[from] serde_json::Error),
}
```

**What's actually true (CLAUDE.md):** "no `let _ = ...` discarding a `Result`" and "Map external errors at the adapter boundary" — bare `#[error(transparent)] Io(#[from] io::Error)` lets a top-level "no such file or directory" surface to the user with no indication of which file or which operation. The codebase's existing pattern (`PluginError::ManifestReadFailed`, `PluginError::DirectoryUnreadable`, `Stage 1's HashError`) wraps every infrastructure error in a typed variant carrying `path: PathBuf` + `#[source] source: io::Error`.

**Required revision:** Drop the bare `Hash` / `Io` / `Json` variants. Replace with typed wrappers that carry the operation's `path` context:

```rust
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum SteeringError {
    // Domain variants stay as in the original plan:
    #[error("steering source `{path}` could not be read")]
    SourceReadFailed {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("steering file `{rel}` would clobber a file owned by plugin `{owner}`; pass --force to transfer ownership")]
    PathOwnedByOtherPlugin { rel: PathBuf, owner: String },
    #[error("steering file exists at `{path}` but has no tracking entry; remove it manually or pass --force")]
    OrphanFileAtDestination { path: PathBuf },
    #[error("steering file `{rel}` content has changed since last install; pass --force to overwrite")]
    ContentChangedRequiresForce { rel: PathBuf },
    #[error("steering tracking I/O failed at `{path}`")]
    TrackingIoFailed {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    // Replace bare transparent variants with typed wrappers:
    #[error("hash computation failed at `{path}`")]
    HashFailed {
        path: PathBuf,
        #[source]
        source: crate::hash::HashError,
    },
    #[error("steering staging file `{path}` could not be written")]
    StagingWriteFailed {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("steering destination directory `{path}` could not be prepared")]
    DestinationDirFailed {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("steering tracking JSON malformed at `{path}`")]
    TrackingMalformed {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
}
```

Add `#[non_exhaustive]` to match every other public error enum in `error.rs`.

Update Task 7's `install_steering_file` body to construct the typed variants instead of relying on `?`-style `From` conversion. E.g.:

```rust
let installed_hash = crate::hash::hash_artifact(&staging_dir, &[rel_path.clone()])
    .map_err(|source| SteeringError::HashFailed { path: staging_dir.clone(), source })?;

fs::write(&staging, source_bytes)
    .map_err(|source| SteeringError::StagingWriteFailed { path: staging.clone(), source })?;
```

**Reasoning:** Bare `#[from]` infrastructure variants are a known anti-pattern in this codebase (see PluginError, AgentError, GitError — none have them). Stage 3's plan inadvertently re-introduced the pattern; aligning with codebase convention is small mechanical work and saves debugging confusion downstream.

---

### S3-2: `DiscoveryWarning` doesn't exist; introduce `SteeringWarning`

**Severity:** BLOCKING

**Affected plan section:** Task 2 step 1 (`InstallSteeringResult.warnings: Vec<crate::service::DiscoveryWarning>`, line 213).

**What the plan says:**

```rust
pub struct InstallSteeringResult {
    pub installed: Vec<InstalledSteeringOutcome>,
    pub failed: Vec<FailedSteeringFile>,
    pub warnings: Vec<crate::service::DiscoveryWarning>,
}
```

(With a parenthetical "if it doesn't exist, replace with whatever warning type the service layer uses".)

**What's actually true:** `DiscoveryWarning` doesn't exist. The service-layer warning type is `InstallWarning` (`crates/kiro-market-core/src/service/mod.rs:394-421`). Reusing `InstallWarning` for steering would force `crate::steering` to depend on warning variants that have nothing to do with steering (`UnmappedTool`, `AgentParseFailed`, `McpServersRequireOptIn`).

**Required revision:** Define `SteeringWarning` local to the steering module (review-findings recommendation):

```rust
// In steering/types.rs alongside SteeringError and InstalledSteeringOutcome:

/// Non-fatal issues raised during steering install (discovery or per-file
/// problems that don't abort the batch).
#[derive(Clone, Debug, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[non_exhaustive]
pub enum SteeringWarning {
    /// A steering scan path was declared but doesn't exist or isn't a
    /// directory. Surfaces so authors can fix manifest typos.
    ScanPathInvalid {
        path: PathBuf,
        reason: String,
    },
    /// A discovered candidate looked like steering but failed validation
    /// at parse / discovery time without rising to a per-file error
    /// (e.g. README-style markdown skipped).
    Skipped {
        path: PathBuf,
        reason: String,
    },
}

impl std::fmt::Display for SteeringWarning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SteeringWarning::ScanPathInvalid { path, reason } => {
                write!(f, "skipped scan path {}: {}", path.display(), reason)
            }
            SteeringWarning::Skipped { path, reason } => {
                write!(f, "skipped steering candidate {}: {}", path.display(), reason)
            }
        }
    }
}
```

Then:

```rust
pub struct InstallSteeringResult {
    pub installed: Vec<InstalledSteeringOutcome>,
    pub failed: Vec<FailedSteeringFile>,
    pub warnings: Vec<SteeringWarning>,
}
```

The `#[non_exhaustive]` is mandatory per the codebase convention for every public enum in error / warning surfaces.

**Reasoning:** Local types are clearer than cross-module reuse here — steering has different warning categories than agents. The cost is one small enum + Display impl; the benefit is the steering module stays self-contained.

---

### S3-3: `uuid_or_pid()` undefined helper *(SUPERSEDED by S3-9)*

> **STATUS:** Superseded after Stage 2 Tier 1.4 cleanup retired the
> `STAGING_COUNTER` + manual `cleanup_leftover_*` pattern in favour of
> `tempfile::TempDir`. **Do not implement this amendment as written.**
> Apply [S3-9](#s3-9-staging-uses-tempfiletempdir-no-staging_counter-no-leftover-sweep)
> instead. The original text below is preserved for historical context.

**Severity:** BLOCKING

**Affected plan section:** Task 7 step 3 (`install_steering_file` body, line 976).

**What the plan says:**

```rust
let staging = self
    .steering_dir()
    .join(format!(".staging-{}", uuid_or_pid()));
```

With note: "if the project doesn't already have such a helper, use `std::process::id().to_string()` or similar".

**What was true at amendment-write time (Stage 1):** No `uuid_or_pid` helper existed. The codebase used `STAGING_COUNTER` (atomic) + `pid` for unique staging names, swept by `cleanup_leftover_agent_staging`. Stage 2 commit `47bee9a` replaced the whole pattern with `tempfile::TempDir`, so this amendment's "use `STAGING_COUNTER`" instruction is now stale. See S3-9 for the current pattern.

---

### S3-4: `install_steering_file` adopts staging-before-rename (P-1)

**Severity:** BLOCKING

**Affected plan section:** Task 7 step 3 (lines 886-1021).

**What the plan says (in summary):** Body computes `installed_hash` AFTER the destructive `if dest.exists() { fs::remove_file(&dest)?; }` + `fs::rename(&staging, &dest)?;` block (lines 977-990). Same data-loss class as the issue Stage 1 fixed for skill / agent installs.

**Required revision:** Apply P-1: hash on staging BEFORE the destructive swap. Adopt P-6 backup-then-swap for force mode since steering CAN have a force-mode collision (cross-plugin or content-changed). Concretely:

```rust
// Inside the file-lock closure:

// (existing collision check stays as planned — no destructive ops yet)

// Stage the file.
fs::create_dir_all(self.steering_dir())
    .map_err(|source| SteeringError::DestinationDirFailed { path: self.steering_dir(), source })?;
let staging = self.fresh_steering_staging(...);  // S3-3
let source_bytes = fs::read(&source.source)
    .map_err(|source| SteeringError::SourceReadFailed { path: source.source.clone(), source })?;
fs::write(&staging, &source_bytes)
    .map_err(|src| SteeringError::StagingWriteFailed { path: staging.clone(), source: src })?;

// Compute installed_hash on staging BEFORE the destructive swap (P-1).
let installed_hash = match crate::hash::hash_artifact(
    staging.parent().unwrap_or(&self.steering_dir()),
    &[std::path::PathBuf::from(staging.file_name().unwrap_or_default())],
) {
    Ok(h) => h,
    Err(source) => {
        if let Err(cleanup_err) = fs::remove_file(&staging) {
            warn!(path = %staging.display(), error = %cleanup_err, "failed to clean staging");
        }
        return Err(SteeringError::HashFailed { path: staging.clone(), source });
    }
};

// Backup-then-swap promote phase (P-6).
let backup = dest.with_extension("md.kiro-bak");
let mut had_backup = false;
if dest.exists() {
    fs::rename(&dest, &backup)
        .map_err(|source| SteeringError::DestinationDirFailed { path: dest.clone(), source })?;
    had_backup = true;
}
if let Some(parent) = dest.parent() {
    fs::create_dir_all(parent)
        .map_err(|source| SteeringError::DestinationDirFailed { path: parent.to_path_buf(), source })?;
}
if let Err(rename_err) = fs::rename(&staging, &dest) {
    if had_backup {
        if let Err(restore) = fs::rename(&backup, &dest) {
            warn!(error = %restore, "failed to restore steering backup after rename failure");
        }
    }
    if let Err(cleanup) = fs::remove_file(&staging) { warn!(...); }
    return Err(SteeringError::DestinationDirFailed { path: dest.clone(), source: rename_err });
}

// Tracking write — rollback on failure restores backup.
installed.files.insert(rel_path.clone(), InstalledSteeringMeta { ... });
if let Err(track_err) = self.write_steering_tracking(&installed) {
    if had_backup {
        if let Err(rb) = fs::remove_file(&dest) { warn!(...); }
        if let Err(restore) = fs::rename(&backup, &dest) { warn!(...); }
    } else {
        if let Err(rb) = fs::remove_file(&dest) { warn!(...); }
    }
    return Err(SteeringError::TrackingIoFailed { path: self.steering_tracking_path(), source: io::Error::other(track_err.to_string()) });
}

// Commit phase: drop backup.
if had_backup {
    if let Err(e) = fs::remove_file(&backup) {
        warn!(path = %backup.display(), error = %e, "leftover .kiro-bak file");
    }
}
```

The `staging.parent()` / `staging.file_name()` calls assume staging lives at `steering_dir/.steering-staging-...`; per S3-3 it does, so `parent()` returns `Some(steering_dir)` reliably. If you're uncomfortable with the `.unwrap_or(...)`, restructure `fresh_steering_staging` to return a `(PathBuf, PathBuf)` tuple of `(staging_dir, file_name_only_pathbuf)` so the hash call can use both directly.

**Reasoning:** Without this restructure, Stage 3 ships with the same data-loss bug Stage 1 had to fix mid-PR. Steering installs are rarer than agent installs but the failure mode is just as user-hostile.

---

### S3-5: `test_marketplace_service()` → `temp_service()`

**Severity:** BLOCKING

**Affected plan section:** Task 9 step 1 (line 1228), Task 11 (end-to-end test fixture, lines 1450+).

**Required revision:** Same as S2-8 — replace every `crate::service::test_support::test_marketplace_service()` call with `let (_tempdir, svc) = crate::service::test_support::temp_service();` and adjust subsequent code that referenced the old single-value binding.

---

### S3-6: `install_steering_file` and `install_plugin_steering` use `mode: InstallMode`

**Severity:** BLOCKING

**Affected plan section:** Task 7 step 3 (`install_steering_file` signature, line 902 — `force: bool`), Task 9 step 3 (`install_plugin_steering` signature, line 1276 — `opts: SteeringInstallOptions { force: bool }`).

**Required revision:** Same as S2-4 — adopt `mode: InstallMode` throughout. Drop `SteeringInstallOptions` from Task 2 step 1 (lines 217-220) — Stage 3's service-layer entrypoint takes `mode: InstallMode` directly, no wrapper:

```rust
// Task 7:
pub fn install_steering_file(
    &self,
    source: &crate::agent::DiscoveredNativeFile,
    marketplace: &str,
    plugin: &str,
    version: Option<&str>,
    source_hash: &str,
    mode: crate::service::InstallMode,
) -> Result<InstalledSteeringOutcome, SteeringError> { ... }

// Task 9:
pub fn install_plugin_steering(
    &self,
    project: &crate::project::KiroProject,
    plugin_dir: &Path,
    scan_paths: &[String],
    mode: crate::service::InstallMode,
    marketplace: &str,
    plugin: &str,
    version: Option<&str>,
) -> InstallSteeringResult { ... }
```

Mirrors `install_plugin_agents`'s positional shape. CLI callsite passes `mode: cli_force_flag.into()` per the existing `From<bool>` impl on `InstallMode`.

---

### S3-8: `InstalledSteeringOutcome` uses `kind: InstallOutcomeKind`, not the bool pair

**Severity:** BLOCKING

**Affected plan section:** Task 2 step 1 (`InstalledSteeringOutcome` struct definition, lines 188–198), Task 7 step 1 (test assertions on `outcome.was_idempotent`, line 868), Task 7 step 3 (struct-literal construction sites, lines 936–941 and 1010–1017), Task 8 (every collision-test assertion that branches on `was_idempotent` / `forced_overwrite`), Task 9 (`assert!(again.installed.iter().all(|o| o.was_idempotent))`, line 1257), Task 10 step 3 (CLI presenter at lines 1391–1397), Task 11 (line 1545).

**What the plan says:**

```rust
pub struct InstalledSteeringOutcome {
    pub source: PathBuf,
    pub destination: PathBuf,
    /// True if `--force` overwrote a tracked path (orphan or other plugin).
    pub forced_overwrite: bool,
    /// True if the install was a no-op because tracking matched
    /// `source_hash` exactly (idempotent reinstall).
    pub was_idempotent: bool,
    pub source_hash: String,
    pub installed_hash: String,
}
```

**What's actually true (commit `e65e314`, Issue #59):** The `(was_idempotent: bool, forced_overwrite: bool)` pair was retired across `InstalledNativeAgentOutcome` and `InstalledNativeCompanionsOutcome` in favour of a 3-state enum. The `(true, true)` state was unrepresentable — encoding it as a single enum makes that explicit and forces presenters to match exhaustively. The shared enum is already `pub` in `crate::project`:

```rust
// crates/kiro-market-core/src/project.rs:128
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum InstallOutcomeKind {
    /// Verified no-op — `source_hash` matched the existing tracking
    /// entry's `source_hash`. No bytes were written.
    Idempotent,
    /// Clean first install — no prior tracking entry, no orphan on disk.
    Installed,
    /// Force-mode overwrote a tracked path (same plugin's prior content,
    /// another plugin's content via ownership transfer, or an orphan
    /// without tracking).
    ForceOverwrote,
}
```

**Required revision:** Replace the bool pair with `kind: InstallOutcomeKind` on `InstalledSteeringOutcome`. **Do not** introduce a parallel `SteeringOutcomeKind` — reuse the existing enum.

```rust
// In steering/types.rs:
use crate::project::InstallOutcomeKind;

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct InstalledSteeringOutcome {
    pub source: PathBuf,
    pub destination: PathBuf,
    pub kind: InstallOutcomeKind,
    pub source_hash: String,
    pub installed_hash: String,
}
```

Mechanical follow-throughs:

- **Task 7 step 3 idempotent branch:** `kind: InstallOutcomeKind::Idempotent` (replaces `forced_overwrite: false, was_idempotent: true`).
- **Task 7 step 3 success branch:** `kind: if forced_overwrite { InstallOutcomeKind::ForceOverwrote } else { InstallOutcomeKind::Installed }` (replaces `forced_overwrite, was_idempotent: false`). The `forced_overwrite: bool` local stays as the internal control variable threaded through staging — only the *outcome* shape changes.
- **Task 7 step 1 test:** `assert_eq!(outcome.kind, InstallOutcomeKind::Installed);` instead of `assert!(!outcome.was_idempotent);`. Add `use kiro_market_core::project::InstallOutcomeKind;`.
- **Task 8 collision tests:** every `assert!(outcome.forced_overwrite)` becomes `assert_eq!(outcome.kind, InstallOutcomeKind::ForceOverwrote)`; every `assert!(outcome.was_idempotent)` becomes `assert_eq!(outcome.kind, InstallOutcomeKind::Idempotent)`.
- **Task 9 step 1 test:** `assert!(again.installed.iter().all(|o| o.kind == InstallOutcomeKind::Idempotent))`.
- **Task 10 step 3 CLI presenter:** match-based suffix instead of two `if`s, exhaustive over the 3-variant enum (the 3-variant exhaustive match is the *point* of the refactor — don't preserve the cascading `if/else if` shape):

  ```rust
  let suffix = match outcome.kind {
      InstallOutcomeKind::Idempotent => " (unchanged)".dimmed(),
      InstallOutcomeKind::ForceOverwrote => " (forced)".yellow(),
      InstallOutcomeKind::Installed => "".normal(),
  };
  ```

- **Task 11 end-to-end test:** `assert!(steering_again.installed.iter().all(|o| o.kind == InstallOutcomeKind::Idempotent))`.

**Reasoning:** Without this, Stage 3 ships a fresh instance of the exact bug pattern Issue #59 just retired. The enum is already public and already used by both other native install outcomes; reusing it keeps the wire format consistent (Specta export will produce one `InstallOutcomeKind` TS type, not three).

---

### S3-9: Staging uses `tempfile::TempDir`, no `STAGING_COUNTER`, no leftover sweep

**Severity:** BLOCKING

**Affected plan section:** Task 7 step 3 (`install_steering_file` body, lines 972–984), supersedes [S3-3](#s3-3-uuid_or_pid-undefined-helper-superseded-by-s3-9).

**What the plan says:**

```rust
std::fs::create_dir_all(self.steering_dir())?;
let staging = self
    .steering_dir()
    .join(format!(".staging-{}", uuid_or_pid()));
std::fs::write(&staging, std::fs::read(&source.source)?)?;
if dest.exists() {
    std::fs::remove_file(&dest)?;
}
if let Some(parent) = dest.parent() {
    std::fs::create_dir_all(parent)?;
}
std::fs::rename(&staging, &dest)?;
```

**What's actually true (commit `47bee9a`, Stage 2 Tier 1.4):** The `STAGING_COUNTER` + `cleanup_leftover_*_staging` pattern was retired in Stage 2 in favour of `tempfile::TempDir`. RAII `Drop` cleans up even when the install panics or returns early — no leftover-sweep needed, no atomic counter, no pid encoding. Both `stage_native_agent_file` and `stage_native_companion_files` now follow this pattern:

```rust
// crates/kiro-market-core/src/project.rs:1182 (stage_native_agent_file)
let staging = tempfile::Builder::new()
    .prefix(&format!("_installing-agent-{name}-"))
    .tempdir_in(self.kiro_dir())?;
let json_rel = PathBuf::from(format!("{name}.json"));
let staging_json = staging.path().join(&json_rel);
fs::write(&staging_json, raw_bytes)?;

let installed_hash =
    crate::hash::hash_artifact(staging.path(), std::slice::from_ref(&json_rel))?;
// staging is a TempDir; drops at scope exit and cleans up the staging dir.
```

`tempfile` is already a workspace dep with both runtime (`[dependencies]`) and dev usage in `kiro-market-core` (Stage 2 promoted it from dev-only to runtime).

**Required revision:** Drop S3-3 entirely. Refactor `install_steering_file` to stage into a `tempfile::TempDir` rooted under `self.kiro_dir()` (NOT under `self.steering_dir()` — staging directories must not live inside the destination directory the install is writing to, because a subsequent `fs::create_dir_all(self.steering_dir())` race could cause it to nest). Hash the staging copy BEFORE the destructive promote (P-1, see S3-4), then `fs::rename` the single staged file into place. Let `staging` drop at end-of-scope.

Sketch — combine with S3-4's backup-then-swap:

```rust
// Inside the file-lock closure, AFTER the collision check decided we proceed:

// Stage. Create staging dir under .kiro/ (NOT inside steering_dir/).
fs::create_dir_all(self.kiro_dir())
    .map_err(|source| SteeringError::DestinationDirFailed {
        path: self.kiro_dir(),
        source,
    })?;
let staging = tempfile::Builder::new()
    .prefix(&format!("_installing-steering-{}-", file_stem(&rel_path)))
    .tempdir_in(self.kiro_dir())
    .map_err(|source| SteeringError::StagingWriteFailed {
        path: self.kiro_dir(),
        source,
    })?;

// Read source + write to staging at the FINAL filename so hashing the
// staged copy gives the same value as hashing after promotion.
let staged_file = staging.path().join(rel_path.file_name().unwrap_or_default());
let source_bytes = fs::read(&source.source)
    .map_err(|src| SteeringError::SourceReadFailed {
        path: source.source.clone(),
        source: src,
    })?;
fs::write(&staged_file, &source_bytes)
    .map_err(|src| SteeringError::StagingWriteFailed {
        path: staged_file.clone(),
        source: src,
    })?;

// Compute installed_hash on staging BEFORE the destructive promote (P-1).
let staged_rel = std::path::PathBuf::from(rel_path.file_name().unwrap_or_default());
let installed_hash = crate::hash::hash_artifact(staging.path(), std::slice::from_ref(&staged_rel))
    .map_err(|src| SteeringError::HashFailed {
        path: staged_file.clone(),
        source: src,
    })?;

// Backup-then-swap promote (P-6). See S3-4 for the rollback semantics.
// ...
fs::rename(&staged_file, &dest).map_err(|src| ...)?;
// staging (TempDir) drops here and cleans up the now-empty staging dir.
```

The `cleanup_leftover_steering_staging` helper from S3-3 is **not needed**. Do not add it. (TempDir's Drop sweeps on the happy path; on an OS-level kill, future installs cannot race on the same staging path because each gets its own random suffix from `tempfile::Builder`.)

**Reasoning:** S3-3 was correct against the Stage 1 codebase but went stale within a few commits. Adopting the now-canonical `TempDir` pattern keeps the three native-install staging paths consistent and removes one nontrivial chunk of crash-recovery code from the steering module before it ships.

---

### S3-10: Reuse generic `CollisionDecision<T>`; extract a `classify_steering_collision` helper

**Severity:** COSMETIC (recommended; not strictly blocking)

**Affected plan section:** Task 7 step 3 (the inline `let mut forced_overwrite = false; if let Some(existing) = ...` ladder at lines 929–970).

**What the plan says:** A 40-line inline collision ladder threaded through the file-lock body, with three error branches and one `forced_overwrite = true` proceed signal, all fused with the staging + promote + tracking code in a single function.

**What's actually true (commit `e65e314`, Issue #60):** Stage 2 extracted both `classify_native_collision` and `classify_companion_collision` into private associated functions that return a generic `CollisionDecision<T>`. The classifiers stay short and exhaustive over the same-plugin / cross-plugin / orphan-on-disk / clean-install matrix; the install body becomes a `match` over the decision plus a linear staging-promote-track sequence. The shared enum lives in `crates/kiro-market-core/src/project.rs:163`:

```rust
enum CollisionDecision<T> {
    Idempotent(Box<T>),
    Proceed { forced_overwrite: bool },
}
```

(Private to `project.rs`. Steering code lives in the same crate, so the visibility is fine — but it's currently `pub(crate)`-by-default with no `pub` modifier, so verify the `enum` line is reachable from steering's caller before reusing. If reuse requires bumping it to `pub(crate)`, do that in the same commit.)

**Required revision:** Refactor Task 7 step 3 to:

```rust
fn classify_steering_collision(
    installed: &InstalledSteering,
    rel_path: &Path,
    plugin: &str,
    source_hash: &str,
    dest: &Path,
    mode: crate::service::InstallMode,
) -> Result<CollisionDecision<InstalledSteeringOutcome>, SteeringError> { ... }
```

Then `install_steering_file`'s body shrinks to:

```rust
let forced_overwrite = match Self::classify_steering_collision(
    &installed, &rel_path, plugin, source_hash, &dest, mode,
)? {
    CollisionDecision::Idempotent(outcome) => return Ok(*outcome),
    CollisionDecision::Proceed { forced_overwrite } => forced_overwrite,
};
// stage + hash + promote + tracking
```

The classifier must be exhaustive (no `_ => default` — see the CLAUDE.md "classifiers enumerate every variant" rule). Steering's collision matrix has exactly four states:
1. Tracked + same plugin + same hash → `Idempotent`
2. Tracked + same plugin + different hash → `ContentChangedRequiresForce` or `Proceed { forced_overwrite: true }`
3. Tracked + different plugin → `PathOwnedByOtherPlugin` or `Proceed { forced_overwrite: true }`
4. Untracked + on-disk → `OrphanFileAtDestination` or `Proceed { forced_overwrite: true }`
5. Untracked + clean → `Proceed { forced_overwrite: false }`

**Reasoning:** Marked COSMETIC rather than BLOCKING because the inline ladder is correct as-written. But every previously-shipped install path in this crate now follows the classifier-decision shape; landing steering with the older inline shape diverges the codebase right as it grows a third install target. If pressed for time, ship inline and follow up — but the refactor takes ~30 lines of net change and lifts the whole install-body comprehension cost.

---

### S3-11: Multi-scan-root steering is allowed (unlike companion bundles)

**Severity:** COSMETIC (documentation amendment + one explicit test)

**Affected plan section:** Task 3 (`discover_steering_files_in_dirs` semantics), Task 9 (`install_plugin_steering` body), and Task 11 (end-to-end test scenarios).

**What the plan says:** The plan does not address what happens when `manifest.steering` declares multiple paths (e.g. `["./guidance/", "./extras/"]`). Since steering files have no shared identity (the per-file relative path under `.kiro/steering/` is the tracking key), there's no analogue to the companion bundle's "single-scan-root" invariant.

**What's actually true (commit `085b48b`):** Stage 2 added `multiple_companion_scan_roots` rejection BEFORE installing any agents, because companion `rel_paths` derivation is ambiguous when files come from two different scan roots — the same `prompts/foo.md` from `./agents/` vs `./extras/` would silently overwrite. Steering does not have this problem: each file's `rel` is computed against its own `scan_root` independently, and the destination key is purely the filename. Two files named `process.md` in two different scan roots WOULD collide at `.kiro/steering/process.md`, but that collision surfaces naturally through `PathOwnedByOtherPlugin` / `ContentChangedRequiresForce` / `OrphanFileAtDestination` — exactly the same way it surfaces for a single-scan-root duplicate.

**Required revision:** Add an explicit note in Task 9's body and a test in Task 11 (or Task 8) that exercises a multi-scan-root steering manifest. The note in Task 9:

```
// Multi-scan-root is supported — each file's rel under its own scan_root
// is the tracking key. Same-name files from different scan roots
// surface as a normal cross-rel collision via the standard collision
// matrix, no upstream rejection needed.
```

The test:

```rust
#[test]
fn install_plugin_steering_handles_multi_scan_root_without_special_case() {
    // plugin manifest declares ["./a/", "./b/"] with distinct .md files
    // in each. Both install. No upstream rejection.
}
```

**Reasoning:** The asymmetry with `MultipleScanRootsNotSupported` is intentional and load-bearing — the Stage 2 reviewer asked about it, and the answer "steering doesn't need the same rejection because the rel-key derivation is unambiguous" should be documented in the plan and pinned by a test rather than living only in this amendments doc.

---

### S3-12: Use `rstest` `#[fixture]` for collision-test setup

**Severity:** COSMETIC

**Affected plan section:** Task 8 (collision tests for `install_steering_file`).

**What's actually true (Stage 2):** The Stage 2 `install_native_agent` and `install_native_companions` collision tests use `rstest` `#[fixture]` (`NativeRev`, `CompanionBundle`) to share multi-step setup across 3+ tests. The pattern is documented in MEMORY.md (`feedback_test_fixtures.md`): "when 3+ tests share multi-step setup, extract a `#[fixture]` returning a resource-owning struct."

**Required revision:** Task 8 will have ≥4 collision tests (idempotent / content-changed / cross-plugin / orphan). Stage 3 should pre-declare a `SteeringFile` fixture mirroring `CompanionBundle`'s shape:

```rust
struct SteeringFile {
    scratch: tempfile::TempDir,
    project: KiroProject,
    scan_root: PathBuf,
    rel_path: PathBuf,
    source_hash: String,
}

impl SteeringFile {
    fn rewrite_source(&mut self, body: &[u8]) { /* re-stage + re-hash */ }
}

#[fixture]
fn steering_file() -> SteeringFile { /* tempdir + project + stage one .md */ }

fn install_steering(
    f: &SteeringFile,
    plugin: &str,
    mode: InstallMode,
) -> Result<InstalledSteeringOutcome, SteeringError> { /* convenience wrapper */ }
```

Same shape as `crates/kiro-market-core/src/project.rs:3399` (`CompanionBundle`).

**Reasoning:** The fixture pattern is already on disk; following it makes the diff smaller and trains the next reader to look for fixtures rather than copy-paste setup.

---

### S3-13: CLI errors render via `error_full_chain`, not `to_string()`

**Severity:** BLOCKING

**Affected plan section:** Task 10 (CLI integration of `install_plugin_steering` results, lines 1370–1409).

**What's actually true (CLAUDE.md):** > At Tauri/log boundaries, AND in any wire-format `reason`/`error: String` field that crosses the FFI, use `error_full_chain(&err)` — not `err.to_string()`, which drops the source chain.

The Stage 2 CLI presenter (`crates/kiro-market/src/commands/install.rs:298`) already follows this:

```rust
let rendered = kiro_market_core::error::error_full_chain(&failed.error);
eprintln!(...);
```

**Required revision:** When Task 10 renders `result.failed` entries, use `error_full_chain(&entry.error)`. Do not use `entry.error.to_string()` or `format!("{}", entry.error)` — either drops `#[source]` chain context (e.g. the underlying `io::Error` reason for a `SourceReadFailed`).

```rust
for failed in &steering_result.failed {
    let rendered = kiro_market_core::error::error_full_chain(&failed.error);
    eprintln!(
        "  {} steering {}: {}",
        "✗".red().bold(),
        failed.source.display(),
        rendered
    );
}
```

**Reasoning:** Stage 1 set this rule explicitly because a previous `to_string()` call dropped the inner `io::Error` reason and produced "tracking I/O failed" without saying which file or which OS error code. Stage 2's CLI follows it; Stage 3's must too.

---

### S3-7: Per-task amendment recap (appears last because it cross-references all other S3 amendments)

> **Reading order note:** S3-7 was authored in pass 1 as a per-task recap; pass 2 inserted S3-8 through S3-13 *before* it on the page so the new amendments group with the others numerically. As a result the on-page section order is `S3-1 … S3-6, S3-8 … S3-13, S3-7`. S3-7 still works as a recap because its bullets reference the new S3-N amendments by number.

- **Tasks 1, 3, 4, 5, 6** — `PluginManifest` extension, `discover_steering_files_in_dirs`, `PluginInstallContext.steering_scan_paths`, `InstalledSteering` / `InstalledSteeringMeta`, load/save helpers. Execute as planned. Note Task 5 should add `#[serde(default, skip_serializing_if = "HashMap::is_empty")]` to `InstalledSteering.files` per P-4 (the original plan only has `#[serde(default)]`).
- **Task 7** — happy path. Apply S3-4 (P-1 + P-6 atomicity), S3-8 (`InstallOutcomeKind`), S3-9 (`tempfile::TempDir` staging), and S3-10 (extract classifier).
- **Task 8** — collision tests. Apply S3-6 (`force: bool` → `mode: InstallMode`), S3-8 (`kind: InstallOutcomeKind` assertions), and S3-12 (rstest `#[fixture]`).
- **Task 9** — `install_plugin_steering`. Apply S3-2 (`SteeringWarning`), S3-5 (`temp_service`), S3-6 (`mode: InstallMode`), S3-8 (`InstallOutcomeKind`), and S3-11 (multi-scan-root note + test).
- **Task 10** — CLI integration. Apply S3-6 (drop `SteeringInstallOptions`), S3-8 (3-variant match in presenter), and S3-13 (`error_full_chain`).
- **Tasks 11, 12** — End-to-end test + final verification. Apply S3-5 (fixture call), S3-8 (idempotent assertion shape), and S3-11 (consider extending the multi-scan-root scenario into the integration test).

---

## Cross-stage notes

- **PluginInstallContext propagation** — Stage 2 Task 7 adds `format: Option<PluginFormat>`; Stage 3 Task 4 adds `steering_scan_paths`. Both should land in the SAME plan-revision pass (extending the same struct in two adjacent commits). The CLI call sites that build the context need to consume both new fields together. *(Stage 2 has shipped; Stage 3 is the only remaining edit to `PluginInstallContext`.)*

- **`PluginInstallContext` as parameter struct vs positional** — Multiple S2 amendments above kept the codebase's positional-arg convention through Stage 2. After Stage 2 landed, Issue #61 (commit `e65e314`) introduced `AgentInstallContext<'a>` as a Copy-able bundle for the agent-install chain's shared `(mode, accept_mcp, marketplace, plugin, version)`. Stage 3's `install_plugin_steering` is currently planned as 4 positional args (`project, marketplace, ctx, opts`), which stays under the `clippy::too_many_arguments` threshold — leaving it positional is fine. If you want consistency with `AgentInstallContext`, introduce a sibling `SteeringInstallContext { mode, marketplace, plugin, version }` (no `accept_mcp` — steering has no MCP gate); it adds ~5 lines and a Copy derive. Either choice is defensible; flagged here so the executor doesn't reach for the now-unconventional bundle shape used in Stage 1.

- **Stage 1's residual companion-hash gap** — The Stage 1 `synthesize_companion_entry` (translated path) leaves a force-mode data-loss window for the per-plugin companion hash. P-6 backup-then-swap should be retroactively applied to that helper too, in a focused commit either alongside Stage 2 (since Stage 2's `install_native_companions` adopts P-6 from the start) or as a Stage 1.5 cleanup PR. Out of scope for this amendments doc; flag for the executor. *(Status check before starting Stage 3: verify whether commit `f203a1b` "extract synthesize_companion_entry helper" carried the P-6 fix or only extracted the helper. If still P-1-only, treat as a parallel cleanup before Stage 3 ships.)*

- **Stage 2 patterns now established as canonical** — Issue #59 (`InstallOutcomeKind`), Issue #60 (`CollisionDecision<T>` + `CompanionPromotion` struct), Issue #61 (`AgentInstallContext` bundle), and the `tempfile::TempDir` migration (commit `47bee9a`) all landed AFTER the original Stage 3 plan was written. The new amendments S3-8 through S3-13 cover the steering-side fall-through; Issue #60's `CollisionDecision<T>` is the most reusable artifact (S3-10). Per-issue tracking lives at GitHub issues #59/#60/#61, all closed by `e65e314`.

- **`tempfile` is now a runtime dep** — Stage 2 promoted `tempfile` from `[dev-dependencies]` to `[dependencies]` in `crates/kiro-market-core/Cargo.toml`. Stage 3's S3-9 staging refactor relies on this — no Cargo.toml edit needed.

- **Self-cycle dev-dep activates `test-support` for integration tests** — Stage 2 added a self-referential `kiro-market-core = { path = ".", features = ["test-support"] }` line under `[dev-dependencies]` so integration tests in `tests/` can reach `service::test_support::temp_service`. Stage 3's Task 11 inherits this — extending `tests/integration_native_install.rs` Just Works. If Stage 3 instead creates a new `tests/integration_steering.rs`, the same feature activation applies (it's per-crate, not per-test-file).

- **Test count expectations (revised)** — Pre-Stage-3 baseline: `cargo test -p kiro-market-core` reports **587 lib tests + 2 integration tests + 2 doc tests** as of commit `e65e314`. Stage 3 should add ~15–25 lib tests (Tasks 1, 3, 4, 5, 6, 7, 8) plus 1–2 integration tests (Task 11). Target total after Stage 3: **~605–615 lib tests**. (The earlier "620–650" target in this doc's previous revision assumed Stage 2 would add 35–50 tests; the actual Stage 2 increment was smaller because several planned tests collapsed into shared `rstest` fixtures.)

---

## Self-review note

This amendments doc was synthesized in two passes:

**Pass 1 (2026-04-24, S3-1 through S3-7):** synthesized by reading the Stage 2 + Stage 3 + design + review-findings + Stage 1 plan files in full; the Stage-1-era landed code in `crates/kiro-market-core/src/{error,project,service/mod,service/test_support,hash}.rs`; and the Stage 1 PR commit history (`56de6d4`, `495755d`, `db6535b`, `a8cd6b2`, `19e97c3`, `f203a1b`, `925990f`).

**Pass 2 (2026-04-26, S3-8 through S3-13 + cross-stage revisions):** added after Stage 2 shipped (PR #48 merged via `617a16a`) and the post-merge cleanup work (issues #59/#60/#61) closed via commit `e65e314` on `feat/native-kiro-plugin-import`. The new amendments are grounded against `crates/kiro-market-core/src/{project,service/mod,steering — N/A yet}.rs` AND `crates/kiro-market/src/commands/install.rs` at HEAD-of-branch as of that commit. S3-3 was retained (with a SUPERSEDED banner) rather than deleted so the audit trail of "why doesn't Stage 3 use STAGING_COUNTER" stays in the doc. Snippets are quoted verbatim from the live code; if a snippet disagrees with `git show <sha> -- <path>`, trust the live code.

Items intentionally NOT amended that the original review-findings doc flagged as "documentation only" or "already noted":
- #13 (Hash failure surfacing) — Stage 1 wired `Error::Hash(#[from])`, callers using `?` propagation are unaffected; documented in Stage 1 Task 15 already.
- #15 (struct-literal callers may break) — Stage 1 plan Task 12 step 6 already acknowledges the fix-up rule.
- #16 (JSON parsed three times) — resolved by S2-9 fix to use `raw_bytes` (eliminates the third re-serialize pass).

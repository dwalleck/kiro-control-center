# Plugin-First Install — Plan Amendments

> **Status:** plan-review pass. Fixes drift between
> `2026-04-29-plugin-first-install-plan.md` and the actual SHA at
> `de59270` (post-PR-92 `main`). Format follows the precedent set by
> `2026-04-24-stage2-3-plan-amendments.md`.

The original self-review section in the plan listed gate names but
didn't do the work each gate prescribes. A second-pass `grep` against
the current source tree (Gate 1 specifically) caught five drift points
between plan code and reality, plus one threat-model gap (Gate 2) that
the surface-level review missed.

Each amendment cites the gate that fired, names the original plan
text, gives the amended text, and explains the rationale. Apply these
during execution; they don't require re-opening the design conversation.

---

## A-1 — Gate 1: `install_plugin_agents` is an associated function, not a method

**Original (plan Task 1, Step 4):**

```rust
let agents = if ctx.agent_scan_paths.is_empty() {
    None
} else {
    Some(self.install_plugin_agents(
        project,
        &ctx.plugin_dir,
        &ctx.agent_scan_paths,
        ctx.format,
        AgentInstallContext { ... },
    ))
};
```

**Drift.** `service/mod.rs:1299` declares
`pub fn install_plugin_agents(project: &KiroProject, ...)` — no `&self`
receiver. The plan's `self.install_plugin_agents(...)` won't compile.

**Amended.** Use the associated-function form. Same fix for
`install_plugin_steering` (already at `service/mod.rs:1330`, no `&self`):

```rust
let agents = if ctx.agent_scan_paths.is_empty() {
    None
} else {
    Some(Self::install_plugin_agents(
        project,
        &ctx.plugin_dir,
        &ctx.agent_scan_paths,
        ctx.format,
        AgentInstallContext { ... },
    ))
};

let steering = if ctx.steering_scan_paths.is_empty() {
    None
} else {
    Some(Self::install_plugin_steering(
        project,
        &ctx.plugin_dir,
        &ctx.steering_scan_paths,
        crate::steering::SteeringInstallContext { ... },
    ))
};
```

`install_skills` is unchanged because it *is* a method (`pub fn install_skills(&self, ...)` at `service/mod.rs:1021`).

**Rationale.** Mechanical compile-error catch. PR-64-style: the original
plan referenced an API signature that didn't match reality. The April 23
plan-review precedent caught four of these (`test_marketplace_service`,
`uuid_or_pid`, etc.); this is the same class.

---

## A-2 — Gate 1: `KiroProject::remove_steering_file` and `remove_agent` do not exist

**Original (plan Task 3, Step 4):**

> "Confirm whether `remove_steering_file(&Path)` and `remove_agent(&str)`
> exist on `KiroProject`. If not, add them following `remove_skill`'s
> shape. **Sub-task:** if either is missing, write a separate failing
> test first..."

**Drift.** Verified by `grep -n "pub fn remove_" crates/kiro-market-core/src/project.rs`:

```
562:    pub fn remove_skill(&self, name: &str) -> crate::error::Result<()> {
```

That is the *only* `pub fn remove_*` on `KiroProject`. Both
`remove_steering_file` and `remove_agent` are absent. My "if missing"
hedge soft-pedals what is in fact a hard prerequisite — `remove_plugin`
cannot exist without these two methods.

**Amended.** Promote the prerequisite to two named tasks before the
cascade lands. Insert into the plan as Tasks 3a and 3b, before what is
currently Task 3:

### Task 3a (new): `KiroProject::remove_steering_file(rel: &Path)`

Follow the shape of `remove_skill` at `project.rs:562`:

1. Acquire `with_file_lock` on `installed-steering.json`'s tracking path.
2. Load `InstalledSteering`, look up `files.get(rel)`.
3. If absent → return `SteeringError::NotInstalled` (or whatever the
   project's not-found-on-remove convention is — check `remove_skill`
   for the pattern; CLAUDE.md "user-owned tracking files" applies).
4. Compute the on-disk destination path
   (`self.steering_dir().join(rel)` — if `steering_dir()` doesn't exist
   yet, use `self.kiro_dir().join("steering").join(rel)`).
5. Validate the joined path stays under `steering_dir` after `canonicalize`
   (Gate 2 — see A-4 below).
6. Unlink the destination. `io::ErrorKind::NotFound` is non-fatal — the
   tracking entry may outlive the file if a user manually deleted it.
7. Remove the entry from `InstalledSteering.files`.
8. Atomically rewrite the tracking JSON.

Test: `remove_steering_file_unlinks_and_updates_tracking` — seed
`installed-steering.json` + an on-disk file, call remove, assert both gone.

### Task 3b (new): `KiroProject::remove_agent(name: &str)`

Mirror the steering shape but for agent prompts in `.kiro/agents/prompts/`
(see `agent_prompts_dir()` at `project.rs:669`). Account for the
`InstalledAgents.native_companions` per-plugin map — see A-3.

Test: `remove_agent_unlinks_prompt_and_updates_tracking`.

### Task 3 (was 3, now 3c): cascade in `remove_plugin`

Unchanged from the original plan's Task 3 except the body now relies
on Tasks 3a/3b being present rather than conditionally creating them.

**Rationale.** "If missing, write the test first" is a hedge I made
because I hadn't verified absence at write time. The 5-gates checklist
explicitly calls out this failure mode ("plan referenced
`test_marketplace_service()` — actual helper was `temp_service()`").
Promoting the prerequisites to first-class tasks removes the conditional.

---

## A-3 — Gate 1: `InstalledAgents.native_companions` map needs cleanup in `remove_plugin`

**Original.** `remove_plugin` cascade in plan Task 3 step 3 only iterates
`InstalledAgents.agents` entries.

**Drift.** `project.rs:113-119`:

```rust
pub struct InstalledAgents {
    pub agents: HashMap<String, InstalledAgentMeta>,
    pub native_companions: HashMap<String, InstalledNativeCompanionsMeta>,
}
```

`native_companions` is keyed by **plugin name** (one entry per plugin
that ships native-format companions). When `remove_plugin` removes
every entry from a plugin, the corresponding `native_companions` entry
must also go — otherwise the tracking file accumulates orphan
companion records.

**Amended.** Extend the cascade in Task 3c:

```rust
// After removing per-agent entries, also drop the per-plugin
// native_companions map entry if present. The companions map is
// keyed by plugin name (not marketplace-qualified), so a plugin
// reinstall under a different marketplace would leave a stale
// companions record otherwise.
let agents_post = self.load_installed_agents()?;
if agents_post.native_companions.contains_key(plugin) {
    // Use whichever helper KiroProject exposes for native_companions
    // tracking updates. If none exists, write a small one-shot
    // helper in this same impl block — pattern: with_file_lock the
    // tracking path, mutate the map, save.
    self.remove_native_companions_for_plugin(plugin)?;
}
```

If `remove_native_companions_for_plugin` doesn't already exist (it
likely doesn't — the cascade is the first caller that needs it), add
it as Task 3b.5 alongside `remove_agent`.

**Rationale.** Gate 1 ("every API the plan references actually exists")
— the original plan didn't grep `InstalledAgents` to discover the
`native_companions` field, so the cascade silently skipped the cleanup
that's required for a clean uninstall.

---

## A-4 — Gate 2: Tracking-file path validation on remove

**Original.** No threat-model treatment of tracking-file content
during `remove_plugin`. My grep pass also missed the existing
mitigation — see **Update via LSP** below.

**Threat scenario.** A malicious plugin (or a corrupted previous
install state) writes a tracking entry like:

```json
{ "files": { "../../etc/passwd": { ... } } }
```

`remove_plugin` calling `steering_dir.join(rel)` followed by an unlink
on the result would resolve to `/etc/passwd` because `Path::join`
does NOT collapse `..`.

**Update via LSP `documentSymbol` on `project.rs`** — this work
should have happened *before* writing the original plan. The
codebase already mitigates the threat at the load boundary, and
already has a path-validation helper:

- `validate_tracking_path_entry(rel: &Path) -> Result<(), &'static str>` at `project.rs:409` — the project-wide helper for "is this tracking-file path safe to use?"
- `validate_tracking_steering_files(installed: &InstalledSteering, tracking_path: &Path)` at `project.rs:476` — runs at load time. Returns `Err` if any entry fails the path check. Tested by `load_installed_steering_rejects_path_traversal_in_files_key` (`project.rs:2737`).
- `validate_tracking_companion_files(installed: &InstalledAgents, tracking_path: &Path)` at `project.rs:445` — same shape for native companions. Tested by `load_installed_agents_rejects_path_traversal_in_companion_files` (`project.rs:2780`).

So the moment `KiroProject::load_installed_steering()` returns `Ok`,
the `InstalledSteering.files` map has been validated against
traversal. By the time `remove_plugin` reads it, the bad entries
have already turned into `Err`s on `load`. **The original threat
scenario doesn't materialize** — `remove_plugin` would propagate the
load-time Err before ever reaching an unlink.

**Amended.** The earlier plan asked for canonicalize-and-prefix-check
inside the cascade. That was the right shape but the wrong layer —
the load boundary already does it. Two narrower asks for the actual
implementation:

1. Add an explicit test that `remove_plugin` propagates the load-time
   error: seed a tracking file with a traversal entry, call
   `remove_plugin`, assert it returns the same `Err` shape as
   `load_installed_steering` would have. Locks the contract that
   removal can't accidentally widen the threat surface by reading
   tracking entries some other way.

2. The new `remove_steering_file(&Path)` / `remove_agent(&str)`
   methods (Tasks 3a / 3b) MUST reuse `validate_tracking_path_entry`
   rather than rolling their own canonicalization. Defense in depth
   against future refactors that bypass the load helpers — e.g., a
   path that constructs an `InstalledSteering` in memory without
   going through `load_installed_steering`. Mirror the
   `remove_skill_rejects_path_traversal` pattern (`project.rs:3902`).

**Rationale.** Original Gate 2 instinct was correct (the threat
exists in principle). LSP-first analysis showed the codebase
already addresses it at the load boundary, with a tested helper.
The amendment narrows the ask from "add new validation" to "reuse
the existing helper and add a regression test that the cascade path
inherits the protection."

---

## A-5 — Gate 3: JSON shape lock only covers all-`None`, not `Some` cases

**Original.** Plan Task 1 step 6 has one rstest case asserting the
all-`None` JSON shape:

```rust
let result = InstallPluginResult {
    plugin: "p".into(),
    version: Some("1.0.0".into()),
    skills: None,
    steering: None,
    agents: None,
};
```

**Drift.** This locks the field-presence contract for "no content
applicable." It does NOT lock the shape when one or more sub-results
are populated. A regression that, say, mistakenly serialized
`Some(InstallSteeringResult)` as a flattened struct (vs. nested) would
slip past this test.

**Amended.** Add a second rstest case:

```rust
#[test]
fn install_plugin_result_json_shape_with_populated_subresult() {
    let result = InstallPluginResult {
        plugin: "p".into(),
        version: Some("1.0.0".into()),
        skills: Some(crate::service::InstallSkillsResult {
            installed: vec!["alpha".into()],
            skipped: vec![],
            failed: vec![],
            skipped_skills: vec![],
        }),
        steering: None,
        agents: None,
    };
    let json = serde_json::to_value(&result).expect("serialize");
    let skills = json.pointer("/skills").expect("skills field exists");
    assert!(
        skills.is_object(),
        "skills must serialize as a nested object, not flatten into the parent",
    );
    assert_eq!(
        skills.pointer("/installed").and_then(|v| v.as_array()).map(Vec::len),
        Some(1),
    );
}
```

**Rationale.** Gate 3 fail signature: "fields constructed in two paths
must produce semantically equivalent values." `Option<T>` constructed
as `None` vs. `Some(default)` would both serialize cleanly today, but a
serde-attribute change (e.g., `#[serde(flatten)]` accidentally
appearing) would break the second only. Single-test coverage of the
all-`None` case is insufficient.

---

## A-6 — Gate 5: "Highest version" via lexicographic string comparison is unreliable

**Original.** Plan Task 2 step 4, `update_version_and_dates`:

```rust
fn update_version_and_dates(
    acc: &mut Acc,
    version: Option<&str>,
    installed_at: chrono::DateTime<chrono::Utc>,
) {
    if let Some(v) = version {
        // Latest wins by string comparison (no semver).
        if acc.version.as_deref().map_or(true, |existing| existing < v) {
            acc.version = Some(v.to_string());
        }
    }
    ...
}
```

**Drift.** Plain `&str` `<` is *lexicographic*, not semantic.
`"0.10.0" < "0.9.0"` is true lexicographically (`'1'` < `'9'`). The
design doc says "no semver comparison — strict string inequality is
enough" but that's the *update-detection* (Phase 2) rule —
`detect_plugin_updates` only needs equality. The *aggregator*
(`installed_plugins`) wants the version of the user's latest install,
which is a different semantic.

**Amended.** Sidestep version comparison entirely by tracking the
version of the **most recent install by `installed_at` timestamp**:

```rust
#[derive(Default)]
struct Acc {
    /// (installed_at, version) of the most recent tracking entry seen
    /// across the three content types. The version of the latest
    /// install — semantically what the UI wants to show under "this
    /// plugin's installed version."
    latest: Option<(chrono::DateTime<chrono::Utc>, Option<String>)>,
    skills: Vec<String>,
    steering: Vec<std::path::PathBuf>,
    agents: Vec<String>,
    earliest: Option<chrono::DateTime<chrono::Utc>>,
}

fn update_latest(
    acc: &mut Acc,
    version: Option<&str>,
    installed_at: chrono::DateTime<chrono::Utc>,
) {
    let new_version = version.map(str::to_string);
    let should_replace = acc
        .latest
        .as_ref()
        .map_or(true, |(when, _)| installed_at >= *when);
    if should_replace {
        acc.latest = Some((installed_at, new_version));
    }
    acc.earliest = Some(acc.earliest.map_or(installed_at, |e| e.min(installed_at)));
}
```

In the final `InstalledPluginInfo` construction:

```rust
let (latest_install, installed_version) = acc
    .latest
    .map_or_else(|| (now, None), |(t, v)| (t, v));
```

**Rationale.** Gate 5 ("encode invariants in types"): the original code
relied on lexicographic string ordering being semantic for version
strings. It isn't. Switching to "latest by timestamp" is both more
correct *and* requires no version-format assumption — it just defers
the question of "which version is current?" to the order in which the
user actually ran installs, which is the UX-correct answer anyway.

---

## A-7 (meta) — process: actually run gates, don't just write section headers

**Drift.** The original plan's "5-Gates self-review" section listed
each gate and wrote a paragraph that read like a gate result — but
several paragraphs were *what I expected the gate to find*, not *what
the gate actually surfaced after running it*. Specifically:

- Gate 1 paragraph asserted "no drift" without grepping the actual SHA.
- Gate 2 paragraph listed `validate_kiro_project_path` and `accept_mcp`
  but missed the tracking-file path-traversal vector (A-4).
- Gate 4 paragraph asserted "no new external errors" without checking
  whether `remove_plugin`'s new failure modes might re-introduce one.

**Mitigation.** Treat the gate paragraphs as *outputs of running the
gate*, not *predictions*. For Gate 1 specifically: run the grep, paste
the matching lines into the paragraph, then write the conclusion. For
Gate 2: enumerate every byte source the new code reads from and walk
the (source × capability) table explicitly. The next plan's self-review
section should cite the actual `grep` command output, the actual
`cargo xtask plan-lint --gate gate-4` output, etc.

The PR-64 plan-review precedent (`2026-04-23-plan-review-findings.md`)
followed this discipline — its findings cite specific line numbers in
the source tree and quote the drift verbatim. That's the bar.

---

## A-10 — Gate 1: Task 8's inline `pluginKey` drops the US-delimiter

**Original (plan Task 8, Step 1):**

```typescript
function pluginKey(mp: string, plugin: string): string {
  return `${mp}${plugin}`;
}
```

**Drift.** BrowseTab.svelte (PR 92) already defines `pluginKey` with an
ASCII Unit-Separator (``) delimiter precisely so a marketplace
literally named `"foo"` + plugin `"bar"` cannot collide with a
marketplace `"fooba"` + plugin `"r"`. From `BrowseTab.svelte:108-111`:

```typescript
const DELIM = "";
const pluginKey = (mp: string, plugin: string) => `${mp}${DELIM}${plugin}`;
```

My Task 8 InstalledTab redefinition uses naive concatenation, breaking
the collision-safe contract. The two files would then key the same
plugin differently — anything that round-trips a key between the
components (none does today, but the temptation is real once
`installedPluginKeys` proves useful) would silently mismatch.

**Amended.** Two equivalent fixes; pick one:

**(a) Inline the delimiter** in Task 8's script block:

```typescript
const DELIM = "";
function pluginKey(mp: string, plugin: string): string {
  return `${mp}${DELIM}${plugin}`;
}
```

**(b) Extract to `$lib/keys.ts`** (preferred — matches Task 6's
extraction-first pattern and avoids any future drift):

```typescript
// crates/kiro-control-center/src/lib/keys.ts
const DELIM = "";
export const pluginKey = (mp: string, plugin: string): string =>
  `${mp}${DELIM}${plugin}`;
export const skillKey = (mp: string, plugin: string, name: string): string =>
  `${mp}${DELIM}${plugin}${DELIM}${name}`;
export const parsePluginKey = (key: string): { marketplace: string; plugin: string } => {
  const [marketplace, plugin] = key.split(DELIM);
  return { marketplace, plugin };
};
```

Then BOTH BrowseTab.svelte and InstalledTab.svelte import from
`$lib/keys`, and the BrowseTab in-script copy of these helpers
(currently at `BrowseTab.svelte:108-118`) gets deleted.

**Recommendation:** (b). The `pluginKey` helper is the kind of
single-source-of-truth utility that earns its keep across the
codebase — and the lift is mechanical, ~6 lines of diff plus the
imports. Setting the precedent here also paves the way for the
future Phase 1.5 / Phase 2 work that'll need the same key shape.

**Rationale.** Gate 1 — "every API the plan references actually
exists at the SHA the plan was written against" expanded to "and
matches the existing contract." A `pluginKey` whose collision-safety
silently differs from BrowseTab's is a contract violation even
though the symbol exists.

---

## A-11 — Gate 1: Task 7's conditional structure under-specifies how `browseView` composes with the existing empty-state branch

**Original (plan Task 7, Step 5):**

```svelte
{:else if browseView === "skills"}
  <div class="grid gap-3 grid-cols-1 lg:grid-cols-2">
    {#each filteredSkills as skill ... }
      <SkillCard ... />
    {/each}
  </div>
{:else}
  {#if availablePlugins.length === 0}
    ... (plugin empty state)
  {:else}
    ... (plugin grid)
  {/if}
{/if}
```

Plus a free-floating note:

> "the existing `{:else if filteredSkills.length === 0}` empty-state
> block (PR 92's per-plugin steering install in the empty state)
> needs to be removed in favor of the new Plugins view"

**Drift.** The note is correct but the code block doesn't show how to
compose with the parent `{#if showLoadingSpinner}` chain. The
implementer has to figure out where the existing
`{:else if filteredSkills.length === 0}` branch goes — keep it gated
on `browseView === "skills"`, or remove it entirely? Both are
defensible; the plan should pick one and show the full structure.

**Amended.** Show the complete conditional, with PR 92's empty-state
removed (the Plugins view is the new home for plugin-based install):

```svelte
{#if showLoadingSpinner}
  <div class="flex flex-col items-center justify-center h-full text-kiro-subtle gap-3">
    <!-- existing loading spinner unchanged -->
  </div>
{:else if initialLoadFailed}
  <div class="flex flex-col items-center justify-center h-full text-kiro-subtle gap-3">
    <!-- existing initial-load-failed message unchanged -->
  </div>
{:else if browseView === "skills"}
  {#if filteredSkills.length === 0}
    <div class="flex flex-col items-center justify-center h-full text-kiro-subtle gap-3">
      <svg class="w-10 h-10 text-kiro-accent-800" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="1.5"
          d="M21 21l-6-6m2-5a7 7 0 11-14 0 7 7 0 0114 0z" />
      </svg>
      <p class="text-sm">
        {#if filterText}
          No skills match the filter
        {:else if fetchErrors.size > 0}
          Skills unavailable due to errors above
        {:else}
          No skills available — try the Plugins view to install plugins that ship steering.
        {/if}
      </p>
    </div>
  {:else}
    <div class="grid gap-3 grid-cols-1 lg:grid-cols-2">
      {#each filteredSkills as skill (skillKey(skill.marketplace, skill.plugin, skill.name))}
        {@const key = skillKey(skill.marketplace, skill.plugin, skill.name)}
        <SkillCard
          {skill}
          selected={selectedSkills.has(key)}
          onToggle={() => toggleSkill(key)}
        />
      {/each}
    </div>
  {/if}
{:else}
  <!-- browseView === "plugins" -->
  {#if availablePlugins.length === 0}
    <div class="flex flex-col items-center justify-center h-full text-kiro-subtle gap-3">
      <p class="text-sm">No plugins available — pick a marketplace from Filters.</p>
    </div>
  {:else}
    <div class="grid gap-3 grid-cols-1 lg:grid-cols-2">
      {#each availablePlugins as ap (pluginKey(ap.marketplace, ap.plugin.name))}
        {@const key = pluginKey(ap.marketplace, ap.plugin.name)}
        <PluginCard
          plugin={ap.plugin}
          marketplace={ap.marketplace}
          installed={installedPluginKeys.has(key)}
          installing={pendingPluginInstalls.has(key)}
          projectPicked={!!projectPath}
          onInstall={() => installWholePlugin(ap.marketplace, ap.plugin.name)}
        />
      {/each}
    </div>
  {/if}
{/if}
```

**Concretely deleted by this amendment** — the current BrowseTab
empty-state plugin-card list added in PR 92 commit `0102ec5` (the
`{:else if !filterText && availablePlugins.length > 0}` branch
introduced when the user reported "I don't see anything in the UI
to install steering"). That UI moves into the new Plugins view as
the *primary* surface; the empty-state is just "No skills available
— try the Plugins view" text now.

**Rationale.** Gate 1 — the plan's "remove PR 92's empty-state"
note moved a non-trivial chunk of working code into "implementer's
discretion." Showing the explicit before/after structure removes
the ambiguity. Also lands a small UX win: the skill empty-state
now points users at the Plugins view, completing the discovery
loop.

---

## A-9 — Performance: hoist `Utc::now()` out of `installed_plugins`'s map closure

**Original (plan Task 2, Step 4):**

```rust
Ok(by_pair
    .into_iter()
    .map(|((marketplace, plugin), acc)| {
        let now = chrono::Utc::now();   // <- called per plugin
        InstalledPluginInfo {
            ...
            earliest_install: acc.earliest.unwrap_or(now),
            latest_install: acc.latest.unwrap_or(now),
        }
    })
    .collect())
```

**Drift.** `chrono::Utc::now()` is a syscall (reads the system clock).
Calling it inside the `map` closure means one syscall per plugin in the
result vec. For a project with 50 installed plugins that's 50 redundant
clock reads, all returning effectively the same value. The whole vec is
built in well under a millisecond — semantically the timestamps would
differ by nanoseconds at most — but the code shape is wasteful, and the
fix is mechanical. Caught by gemini-code-assist on PR #93.

**Amended.** Capture the time once before the loop, reuse the binding:

```rust
let now = chrono::Utc::now();
Ok(by_pair
    .into_iter()
    .map(|((marketplace, plugin), acc)| {
        InstalledPluginInfo {
            ...
            earliest_install: acc.earliest.unwrap_or(now),
            latest_install: acc.latest.unwrap_or(now),
        }
    })
    .collect())
```

Same hoist applies to the A-6 rewrite — the `now` in
`acc.latest.map_or_else(|| (now, None), |(t, v)| (t, v))` should come
from a single binding outside the closure too.

**Rationale.** Strictly speaking this is a perf nit; the cost is
negligible at expected plugin counts. But the `unwrap_or(now)` arm is
also semantically suspect — a missing `installed_at` from the tracking
file is a degenerate state (every install path writes one), so we're
substituting "right now" for "we don't actually know" and presenting it
to the UI as a real install timestamp. The hoisting fix sidesteps the
multi-call concern AND makes the substitution explicit at one site
where a future reviewer can ask "is this fallback right?" rather than
having it scattered across N closure invocations.

---

## A-12 — Gate 1: `remove_plugin` cascade aborts on orphan tracking entries

**Original (plan Task 3, Step 3):**

```rust
for name in &to_remove {
    self.remove_skill(name)?;     // <- propagates SkillError::NotInstalled
    result.skills_removed = result.skills_removed.saturating_add(1);
}
// ...steering and agents follow the same pattern
```

**Drift.** `KiroProject::remove_skill` returns `SkillError::NotInstalled`
when the on-disk skill *directory* is absent (verified at
`project.rs:568-572`) — a normal state-divergence case the user can
reach by running `rm -rf .kiro/skills/<name>/` manually. The
cascade's `?` aborts on the first such error, dropping the
`result.skills_removed` count and skipping steering + agents
entirely. Result: a single orphan tracking entry stalls every
subsequent removal in the same plugin. The user has no path to
recover except hand-editing tracking JSON.

**Amended.** Treat per-entry "not installed" as a *recoverable*
case — log + count it as removed, continue the cascade:

```rust
for name in &to_remove {
    match self.remove_skill(name) {
        Ok(()) => {
            result.skills_removed = result.skills_removed.saturating_add(1);
        }
        // Orphan tracking entry — directory was already gone. The
        // cascade's job is to drive the project to "no entries from
        // this plugin"; absent directory is a step in the right
        // direction, not an error.
        Err(crate::error::Error::Skill(SkillError::NotInstalled { .. })) => {
            tracing::warn!(
                skill = %name, plugin, marketplace,
                "remove_plugin: skill tracking entry had no on-disk directory; \
                 treating as already-removed",
            );
            result.skills_removed = result.skills_removed.saturating_add(1);
        }
        Err(e) => return Err(e),
    }
}
```

Mirror the same `match` pattern in the steering and agent loops.
Other error variants (`Skill(_)`, `Plugin(_)`, `Io(_)`) still abort
the cascade — they indicate genuine problems, not state divergence.

**Rationale.** Gate 2 / Gate 5 (semantic correctness): the `?`
collapses two distinct cases into one error path, losing the
recoverable-vs-fatal distinction that the typed error variants
were designed to encode. `remove_plugin` is a "drive toward
absence" operation; finding things already absent is not a
failure.

---

## A-13 — Gate 1: `SteeringError::NotInstalled` does not exist

**Original (plan Task 3a, Step 3):**

> "If absent → return `SteeringError::NotInstalled` (or whatever
> the project's not-found-on-remove convention is)..."

**Drift.** Verified by `grep "NotInstalled"` against
`crates/kiro-market-core/src/steering/types.rs`: `SteeringError`
has no `NotInstalled` variant. Variants present:
`SourceReadFailed`, `SourceHardlinked`, `PathOwnedByOtherPlugin`,
`OrphanFileAtDestination`, `ContentChangedRequiresForce`,
`TrackingIoFailed`, `HashFailed`, `StagingWriteFailed`,
`DestinationDirFailed`, `TrackingMalformed`. The "not installed"
case isn't represented today because `install_plugin_steering`
never had a corresponding `remove_*` operation.

**Amended.** Task 3a step 3 must add a new typed variant before
the `remove_steering_file` method can return it:

```rust
// crates/kiro-market-core/src/steering/types.rs — add to SteeringError
#[non_exhaustive]
#[error("steering file `{rel}` is not tracked in installed-steering.json")]
NotInstalled { rel: PathBuf },
```

Update the existing `cargo xtask plan-lint --gate
ffi-enum-serde-tag` cycle: the new variant is a unit-payload-bearing
addition, so the `tag = "kind"` discriminant attribute stays valid;
the JSON-shape rstest at `steering/types.rs:298-333` should grow a
case for `NotInstalled`.

Same pattern for `AgentError::NotInstalled` already exists at
`error.rs:276` — confirm by reading the variant before reusing.
The agent path's Task 3b can use the existing `AgentError::NotInstalled`
without adding anything.

**Rationale.** Gate 1 — naming a fictitious variant in a code block
is the exact failure mode the checklist is designed to catch.
A-2's "if missing, write the test first" hedge passed the buck;
this amendment names the variant that needs to land.

---

## A-14 — Gate 1: Task 4's wrapper has the same `Self::` drift as A-1

**Original (plan Task 4, Step 1, the example wrapper body):**

```rust
fn install_plugin_agents_impl(
    svc: &MarketplaceService,
    ...
) -> Result<InstallAgentsResult, CommandError> {
    ...
    Ok(svc.install_plugin_agents(   // <- method call on associated function
        &project,
        &ctx.plugin_dir,
        ...
    ))
}
```

**Drift.** `MarketplaceService::install_plugin_agents` is an
associated function (no `&self`) at `service/mod.rs:1299`, same
shape as `install_plugin_steering`. The plan's example uses
`svc.install_plugin_agents(...)` — method-call syntax —
which won't compile.

A-1 covered the same drift in Task 1's orchestrator but not in
Task 4's standalone Tauri command. Cross-task consistency check
missed.

**Amended.** Task 4's `_impl` should call the function via its
absolute path:

```rust
Ok(MarketplaceService::install_plugin_agents(
    &project,
    &ctx.plugin_dir,
    &ctx.agent_scan_paths,
    ctx.format,
    AgentInstallContext { ... },
))
```

Strip `svc` from the `_impl` signature too — the function doesn't
use the receiver. The wrapper still calls `make_service()?` for
its own error path but the `_impl` doesn't need it. Or: keep
`svc` in the signature for symmetry with the steering `_impl` and
just unused-bind it; the existing `commands/steering.rs::install_plugin_steering_impl`
takes `svc: &MarketplaceService` even though it calls
`MarketplaceService::install_plugin_steering(...)` as an
associated function — same shape applies here.

**Rationale.** Gate 1 — cross-task consistency. When fixing one
instance of a drift pattern, search for the same pattern in
sibling tasks. This is a "did you grep for the bug shape, not just
the bug instance" failure.

---

## A-15 — Gate 1: orchestrator's `is_empty()` branches are unreachable dead code

**Original (plan Task 1, Step 4):**

```rust
let agents = if ctx.agent_scan_paths.is_empty() {
    None
} else {
    Some(Self::install_plugin_agents(...))  // (A-1's fix applied)
};
// same shape for skills and steering
```

**Drift.** `agent_scan_paths_for_plugin` at `service/browse.rs:884-901`
falls back to `crate::DEFAULT_AGENT_PATHS` (`["./agents/"]`)
whenever the manifest is absent or its `agents` list is empty.
`steering_scan_paths_for_plugin` at `:907-916` does the same with
`DEFAULT_STEERING_PATHS`. Both functions ALWAYS return at least
one element. Therefore `ctx.agent_scan_paths.is_empty()` and
`ctx.steering_scan_paths.is_empty()` are tautologically false —
the `None` branches never fire.

The skills branch is similarly affected: `ctx.skill_dirs` comes
from a different code path (registry-driven discovery), but for
plugins that declare no `skills/` directory, the discovery yields
an empty `Vec` — so the skills `is_empty()` MIGHT fire. Need to
verify with LSP / read.

**Amended.** Drop the conditional entirely for steering and
agents — always run those install paths, let them return empty
`installed`/`failed`/`warnings` vecs when nothing's there. Update
`InstallPluginResult` to make `skills`/`steering`/`agents` non-
optional:

```rust
#[derive(Debug, Default, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct InstallPluginResult {
    pub plugin: String,
    pub version: Option<String>,
    pub skills: InstallSkillsResult,
    pub steering: InstallSteeringResult,
    pub agents: InstallAgentsResult,
}
```

Frontend consumers check `result.skills.installed.length > 0`
etc. directly — no `Option` unwrap needed. The "did this plugin
have any of X content" question is implicitly answered by
`installed.is_empty() && failed.is_empty() && warnings.is_empty()`.

Updates the JSON-shape lock from A-5: instead of asserting `null`
for unattempted content types, assert empty vecs.

**Rationale.** Gate 1 + Gate 5 — the original design's `Option<T>`
distinction ("applicable vs. attempted but empty") was based on a
pre-condition check (`is_empty()`) that doesn't actually catch
the "not applicable" case because of the default fallback. The
distinction was illusory; encoding it in the type was wrong.
Removing the `Option` simplifies the wire format AND eliminates
the dead branches.

---

## A-16 — Gate 1: `native_companions` cleanup over-deletes across marketplaces

**Original (A-3 amendment text):**

```rust
let agents_post = self.load_installed_agents()?;
if agents_post.native_companions.contains_key(plugin) {
    self.remove_native_companions_for_plugin(plugin)?;
}
```

**Drift.** `InstalledAgents.native_companions: HashMap<String,
InstalledNativeCompanionsMeta>` is keyed by **plugin name only**
(`project.rs:114`). `InstalledNativeCompanionsMeta` carries a
`marketplace: String` field at `project.rs:97` precisely so two
different marketplaces shipping a plugin with the same name can
coexist.

A-3's `contains_key(plugin)` test ignores the marketplace
distinction. If marketplace A and marketplace B both ship a plugin
named `"code-reviewer"`, removing marketplace A's entry would
match-and-delete marketplace B's `native_companions` record too
— a cross-marketplace data corruption.

**Amended.** Match on BOTH plugin name AND marketplace:

```rust
let agents_post = self.load_installed_agents()?;
if let Some(meta) = agents_post.native_companions.get(plugin) {
    if meta.marketplace == marketplace {
        self.remove_native_companions_for_plugin(plugin, marketplace)?;
    }
    // else: this `plugin` name belongs to a different marketplace's
    // record. Leave it alone — the `(marketplace, plugin)` pair we
    // were asked to remove is not represented in native_companions.
}
```

The new `remove_native_companions_for_plugin` helper signature
takes `(plugin: &str, marketplace: &str)` and only removes when
both match. The HashMap keying-by-plugin-only is a pre-existing
data-model quirk that this fix works around at the read site;
restructuring `native_companions` to be keyed on `(marketplace,
plugin)` is a larger change and out of scope.

**Rationale.** Gate 1 + Gate 2 (data-corruption threat from a
plausible same-name-across-marketplaces scenario). A-3 surfaced
the field but didn't grep its `Meta` struct to discover the
disambiguator. LSP `documentSymbol` would have shown
`InstalledNativeCompanionsMeta.marketplace` immediately, but A-3
predated the A-8 LSP-first discipline.

---

## A-17 — Gate 5: `>=` tie-break in `update_latest` is non-deterministic

**Original (A-6 amendment code):**

```rust
let should_replace = acc
    .latest
    .as_ref()
    .map_or(true, |(when, _)| installed_at >= *when);
```

**Drift.** `install_plugin` runs the three install paths in a
single call, all sharing the `Utc::now()` snapshot — so when the
aggregator iterates the three tracking maps, the `installed_at`
timestamps for entries from one `install_plugin` call are equal.
With `>=`, equal timestamps always replace, so the
`installed_version` that wins depends on which content type's
`HashMap` was iterated last. `HashMap` iteration order is
non-deterministic in Rust (per `std::collections::HashMap` docs).

User-visible effect: the `installed_version` field on
`InstalledPluginInfo` flickers between content-type values across
process restarts when the three sub-results' versions differ
(e.g. plugin manifest's `version` is one of them, but the entries
might have been written by older code paths with stale versions).

**Amended.** Use `>` instead — first-seen wins on ties:

```rust
let should_replace = acc
    .latest
    .as_ref()
    .map_or(true, |(when, _)| installed_at > *when);
```

Document the iteration order explicitly: the aggregator iterates
skills, then steering, then agents — so on tied timestamps, the
*first* of those three with a tracking entry contributes the
displayed `installed_version`. Stable across process restarts.

**Rationale.** Gate 5 (encode invariants in semantics, not in
HashMap-iteration-order accidents). `>=` looks innocent but
silently couples the result to a non-deterministic substrate;
`>` makes the iteration order load-bearing in a documented way.

---

## A-18 — Gate 2: Task 5's `install_plugin` Tauri wrapper drops `accept_mcp`

**Original (plan Task 5, Step 2):**

> "Same wrapper + `_impl` pattern as steering. Calls
> `svc.install_plugin(...)` from Task 1."

**Drift.** Steering's `install_plugin_steering` Tauri command
takes `(marketplace, plugin, force, project_path)` — no
`accept_mcp`. Following that template literally for `install_plugin`
gives the same signature, but `svc.install_plugin(...)` from
Task 1 takes `accept_mcp: bool` as its 5th parameter. The
implementer would either (a) omit `accept_mcp` from the wrapper
and hardcode `false` in the `_impl`, silently bypassing the
user's MCP opt-in, or (b) hand-add it without reading the design
doc's Gate 2 action item that explicitly calls this out.

**Amended.** Task 5 Step 2's wrapper signature MUST include
`accept_mcp: bool`:

```rust
#[tauri::command]
#[specta::specta]
pub async fn install_plugin(
    marketplace: String,
    plugin: String,
    force: bool,
    accept_mcp: bool,
    project_path: String,
) -> Result<InstallPluginResult, CommandError> {
    let svc = make_service()?;
    install_plugin_impl(
        &svc,
        &marketplace,
        &plugin,
        InstallMode::from(force),
        accept_mcp,
        &project_path,
    )
}

fn install_plugin_impl(
    svc: &MarketplaceService,
    marketplace: &str,
    plugin: &str,
    mode: InstallMode,
    accept_mcp: bool,
    project_path: &str,
) -> Result<InstallPluginResult, CommandError> {
    validate_kiro_project_path(project_path)?;
    let project = KiroProject::new(PathBuf::from(project_path));
    svc.install_plugin(&project, marketplace, plugin, mode, accept_mcp)
        .map_err(CommandError::from)
}
```

Frontend `commands.installPlugin(marketplace, plugin, force,
acceptMcp, projectPath)` — `acceptMcp` defaults to `false` at the
Svelte caller (per the design doc's `accept_mcp` is opt-in
default-deny rule). When/where the user toggles MCP consent in
the UI is a Phase 2 concern; for Phase 1, ship the binding with
`acceptMcp: false` hardcoded at the BrowseTab call site and wire
a real toggle later.

**Rationale.** Gate 2 — the design doc explicitly listed
"plumb `accept_mcp` through `install_plugin` from the Tauri
layer" as an action item. The plan's Task 5 silently dropped
it. CLAUDE.md "frontend error-path rigor" applies: a security-
sensitive flag the design doc said to plumb cannot be left
implicit.

---

## A-19 — Gate 1: `list_installed_plugins` and `remove_plugin` don't follow the `_impl(svc, ...)` pattern; existing `list_installed_skills` doesn't either

**Original (plan Task 5, Steps 3-4):**

```rust
fn list_installed_plugins_impl(
    project_path: &str,
) -> Result<Vec<InstalledPluginInfo>, CommandError> { ... }

fn remove_plugin_impl(
    marketplace: &str,
    plugin: &str,
    project_path: &str,
) -> Result<RemovePluginResult, CommandError> { ... }
```

**Drift.** CLAUDE.md says `_impl(svc: &MarketplaceService, ...)`.
My code drops `svc` because these commands operate on
`KiroProject` only — no service needed. Claude's review flagged
this as a CLAUDE.md violation.

**Update via LSP read of existing `commands/installed.rs`:**
`list_installed_skills` and `remove_skill` are BOTH defined
without an `_impl` — the body is inline in the wrapper:

```rust
pub async fn list_installed_skills(
    project_path: String,
) -> Result<Vec<InstalledSkillInfo>, CommandError> {
    let project = KiroProject::new(PathBuf::from(&project_path));
    let installed = project.load_installed().map_err(CommandError::from)?;
    // ... transform + return
}
```

So the existing convention is "no `_impl` for non-service-
consuming commands." CLAUDE.md's wrapper-`_impl`-svc rule
applies to commands that consume `MarketplaceService`; it doesn't
apply to project-only reads.

**Amended.** Two equivalent shapes — pick one and document the
choice:

**(a) Match the existing `list_installed_skills` precedent** — no
`_impl`, body inline:

```rust
#[tauri::command]
#[specta::specta]
pub async fn list_installed_plugins(
    project_path: String,
) -> Result<Vec<InstalledPluginInfo>, CommandError> {
    validate_kiro_project_path(&project_path)?;
    let project = KiroProject::new(PathBuf::from(&project_path));
    project.installed_plugins().map_err(CommandError::from)
}
```

**(b) Keep the `_impl` for testability without `svc`** — document
the deviation from CLAUDE.md's rule with a one-line comment:

```rust
// `_impl` exists for direct unit-test access without spinning up
// a full MarketplaceService. CLAUDE.md's wrapper-`_impl`-svc rule
// applies to service-consuming commands; this one operates on
// KiroProject only.
fn list_installed_plugins_impl(project_path: &str) -> Result<...>
```

**Recommendation:** (a). Match the existing precedent.
`commands/installed.rs::tests` already shows that wrapper-only
commands can be unit-tested by calling them directly (it's
async, but `#[tokio::test]` handles that). No `_impl` needed.

`remove_plugin` (Task 5 Step 4) gets the same treatment — body
inline in the wrapper.

**Rationale.** Gate 1 — Claude's reading of CLAUDE.md was strict;
the actual project convention is more permissive. Verifying via
LSP-read of an existing peer command (`commands/installed.rs`)
gives the right answer instead of guessing from the doc text.
A-8's LSP-first discipline applies: when the rule and the
practice diverge, read the practice.

---

## A-20 — Gate 1: `make_kiro_project` test helper is private to `commands/steering.rs::tests`

**Original (plan Task 5, Step 1 test code):**

```rust
let project_path = make_kiro_project(dir.path());
```

**Drift.** `make_kiro_project` is defined at
`commands/steering.rs:88-92` (per LSP `documentSymbol`) inside
`#[cfg(test)] mod tests` — it's private to that module. Tests in
a NEW `commands/plugins.rs` file cannot import it. The PR 92
review's `code-simplifier` agent flagged the same helper as
duplicated between `steering.rs::tests` and `browse.rs::tests`
and recommended hoisting to `kiro_market_core::service::test_support`,
but that recommendation was deferred.

**Amended.** Add a sub-task before Task 5's tests can be written:

### Task 5.0 (new, prerequisite to 5.1+ tests): Hoist `make_kiro_project` to `service::test_support`

1. Add `pub fn make_kiro_project(dir: &Path) -> String` to
   `crates/kiro-market-core/src/service/test_support.rs` (gated
   `#[cfg(any(test, feature = "test-support"))]` per the existing
   pattern). Same body as the inline helper in `steering.rs:88-92`.
2. Update `commands/steering.rs::tests` to import from
   `kiro_market_core::service::test_support::make_kiro_project`
   instead of defining locally.
3. Update `commands/browse.rs::tests` (which defines an identical
   inline copy at `browse.rs:451-455` per the PR 92 review) to
   import the same.
4. The new `commands/plugins.rs::tests` (Task 5.1+) imports from
   the same location.
5. Test: existing `steering.rs::tests` and `browse.rs::tests` all
   continue to pass.
6. Commit: `refactor(test): hoist make_kiro_project to service::test_support`.

This pre-task closes a real PR-92 follow-up (the
`code-simplifier`'s "high-value simplification #1") AND unblocks
Task 5's tests in one stroke. Roughly ~10 lines moved across
three files.

**Rationale.** Gate 1 — naming a function that doesn't exist at
the call site. The original Task 5 plan wrote test code referencing
`make_kiro_project` as if it were globally available; LSP
`documentSymbol` would have shown it scoped to `steering.rs::tests`
only. Test code is code; the same naming-resolution rules apply.

---

## A-21 — Gate 1 (process): A-1 over-claimed scope

**Original (A-1 amendment):**

> "Same fix for `install_plugin_steering` (already at
> `service/mod.rs:1330`, no `&self`)..."

**Drift.** The original Task 1 plan code already used
`MarketplaceService::install_plugin_steering(...)` (associated-
function form, correct). My A-1 amendment text claimed "same fix
for steering" implying drift; there was no drift in steering's
call site. The agents arm (`self.install_plugin_agents`) was the
sole drift A-1 addressed.

**Amended.** Strike the steering reference from A-1's body. The
amendment applies to the agents arm only. The steering call site
is correct as originally written.

**Rationale.** Process — amendments are statements of fact about
plan drift. Over-broad amendment text dilutes the audit trail
and may cause the implementer to "fix" already-correct code.
Cross-reference Claude's review at PR #93 (round 3) for the
original flag.

---

## A-23 — Gate 1 + Gate 3: `DateTime<Utc>` fields are incompatible with the project's `specta` feature config

**Original (plan Task 2, Step 3):**

```rust
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct InstalledPluginInfo {
    ...
    pub earliest_install: chrono::DateTime<chrono::Utc>,
    pub latest_install: chrono::DateTime<chrono::Utc>,
}
```

**Drift.** `kiro-market-core/Cargo.toml:30` declares:

```toml
specta = { version = "2.0.0-rc.24", optional = true, features = ["derive", "serde_json"] }
```

The `"chrono"` feature is NOT in that list. Without it,
`chrono::DateTime<chrono::Utc>` does not implement `specta::Type`.
The Tauri crate always enables the `specta` feature on
`kiro-market-core` (since its commands call `specta::specta` to
generate bindings), so compiling with the new
`InstalledPluginInfo` would fail with `the trait specta::Type is
not implemented for chrono::DateTime<chrono::Utc>`.

The existing precedent at `commands/installed.rs:17-25`
(`InstalledSkillInfo.installed_at: String`) shows the project's
established pattern: convert to `String` at the FFI boundary,
filled via `.to_rfc3339()` in the wrapper. Every `DateTime<Utc>`
field in `project.rs` lives on a struct that intentionally does
NOT derive `specta::Type` (`InstalledSkillMeta`,
`InstalledSteeringMeta`, `InstalledAgentMeta`,
`InstalledNativeCompanionsMeta` — verified via LSP
`documentSymbol`).

`InstalledPluginInfo` is meant to cross the FFI (it's the return
type of `commands.listInstalledPlugins`), so it MUST follow the
String-at-boundary pattern.

**Amended.** Two changes to Task 2:

1. **Field types**: `earliest_install` and `latest_install` become
   `String`:

```rust
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct InstalledPluginInfo {
    pub marketplace: String,
    pub plugin: String,
    pub installed_version: Option<String>,
    pub skill_count: u32,
    pub steering_count: u32,
    pub agent_count: u32,
    pub installed_skills: Vec<String>,
    pub installed_steering: Vec<std::path::PathBuf>, // PathBuf has specta::Type
    pub installed_agents: Vec<String>,
    /// RFC3339-formatted timestamp. Matches the shape used by
    /// [`commands/installed.rs::InstalledSkillInfo`].
    pub earliest_install: String,
    pub latest_install: String,
}
```

2. **Construction in `installed_plugins()`**: keep the
   `DateTime<Utc>` internally in the `Acc` struct (so A-17's
   timestamp-based "latest wins" tie-break still works), then
   convert at the final field assignment:

```rust
let now = chrono::Utc::now();
Ok(by_pair
    .into_iter()
    .map(|((marketplace, plugin), acc)| {
        let (latest_install_dt, installed_version) = acc
            .latest
            .map_or_else(|| (now, None), |(t, v)| (t, v));
        let earliest_install_dt = acc.earliest.unwrap_or(now);
        InstalledPluginInfo {
            marketplace,
            plugin,
            installed_version,
            skill_count: u32::try_from(acc.skills.len()).unwrap_or(u32::MAX),
            steering_count: u32::try_from(acc.steering.len()).unwrap_or(u32::MAX),
            agent_count: u32::try_from(acc.agents.len()).unwrap_or(u32::MAX),
            installed_skills: acc.skills,
            installed_steering: acc.steering,
            installed_agents: acc.agents,
            earliest_install: earliest_install_dt.to_rfc3339(),
            latest_install: latest_install_dt.to_rfc3339(),
        }
    })
    .collect())
```

The frontend's `formatDate(p.latest_install)` already does
`new Date(iso)` — works directly with the RFC3339 string. No
frontend changes needed.

**Verified `PathBuf` survives unchanged.** `installed_steering:
Vec<PathBuf>` is fine — `PathBuf` does implement `specta::Type`
out of the box (specta serializes it as a TS `string`). Confirmed
by the existing `InstalledSteeringOutcome.source: PathBuf` /
`destination: PathBuf` at `steering/types.rs:142-148`, which DOES
derive `specta::Type` and renders as `string` in `bindings.ts:351-358`.

**Rationale.** Gate 1 + Gate 3 — the field type doesn't
implement the trait the derive requires, with the project's
specific Cargo.toml config. Catching this at plan-review is
cheaper than at first-build of Task 2. Also a Gate 5 win:
String-at-boundary makes the wire format more predictable for
the frontend (no chrono-specific JS deserialization needed).

---

## A-22 — Process: code-reviewer-style audit catches what LSP-first cannot

**Observation.** After A-1 through A-11 (rigorous LSP-first gate
review), Claude's three-round review on PR #93 surfaced 9
substantive findings I missed:

- A-12 (`remove_plugin` cascade abort on orphan) — behavioral
  semantic concern about a `?` operator
- A-13 (`SteeringError::NotInstalled` fictitious) — variant
  enumeration
- A-14 (Task 4 `Self::` drift) — same shape as A-1 in a sibling
  task
- A-15 (dead `is_empty()` branches) — logical reachability
- A-16 (`native_companions` over-delete) — cross-marketplace
  ambiguity given a HashMap-key-by-name-only data shape
- A-17 (`>=` tie-break) — non-determinism from HashMap iteration
- A-18 (`accept_mcp` plumbing) — design-doc action-item drift
- A-19 (`_impl(svc, ...)` rule mismatch with practice) — CLAUDE.md
  vs. actual existing-code convention
- A-20 (`make_kiro_project` private helper) — module-scope
  visibility

**Pattern.** LSP-first answers "does this symbol exist?" and
"what's its signature?" — strong on shape, weak on semantics.
The 9 findings above need a different review lens:

- **Reachability analysis** (which branches are actually taken):
  A-15
- **Semantic equivalence** (do these two error paths have the
  same recoverable-vs-fatal contract?): A-12, A-19
- **Cross-task / cross-instance consistency** (when fixing
  pattern X in task N, is the same pattern in tasks M and P also
  fixed?): A-14, A-21
- **Data-shape correctness across hidden disambiguators** (is
  this HashMap keyed by enough fields?): A-16, A-17
- **Action-item-vs-task linkage** (did the design doc say to do
  X, and does the plan actually do X?): A-18
- **Test-code naming resolution** (is this helper visible from
  where the test calls it?): A-20

**Mitigation going forward.** After the LSP-first Gate 1 pass,
schedule a code-reviewer-style second pass that walks each task
and asks the six questions above explicitly. This is what
Claude did on PR #93 — the rigor isn't unique to a bot; it's
the *checklist of review angles* that's the artifact. The next
plan-review pass should run both:

1. LSP-first (A-8): catches signature drift, missing exports,
   field-access typos.
2. Code-reviewer-style (A-22): catches behavioral semantics,
   cross-task drift, action-item linkage.

These are complementary, not substitutable. A plan that passes
only one is half-reviewed.

---

## A-8 — Tooling: use LSP `documentSymbol` for Gate 1, not grep

**Process drift.** The original Gate 1 pass used `grep -n` to spot
APIs the plan referenced. That found the most obvious drift but
missed two existing helpers I should have been reusing:

- `validate_tracking_path_entry` (free fn at `project.rs:409`)
- `validate_tracking_steering_files` / `validate_tracking_companion_files`
  (`project.rs:445, 476`) and their already-passing
  `*_rejects_path_traversal_in_files_key` regression tests at
  `project.rs:2737, 2780`

**Mitigation.** A single LSP `documentSymbol` call on `project.rs`
returned the full symbol map — every `pub fn`, every struct field
with its type, every test, in one query. The full call:

```
LSP operation=documentSymbol
    filePath=crates/kiro-market-core/src/project.rs
```

returned ~200 symbols including the validation helpers, the existing
`remove_skill` traversal test (`project.rs:3902`), and the
`InstalledAgents.native_companions` field that drove A-3. None of
these were reachable by `grep "pub fn remove"` alone — `documentSymbol`
sees the whole structural surface and returns it grouped by type.

**Rule going forward.** First step of any plan-review Gate 1 pass:
`documentSymbol` on every file the plan modifies. Then `grep` for
any specific names the plan introduces (it's still useful for
"does this exact identifier exist?" — LSP `workspaceSymbol` is the
better answer there too). The PR-64-era plans pre-dated
LSP-as-a-tool; the new gold standard is LSP-first, grep-as-fallback.

This amendment is process-only — no code change. Captures the
lesson so future plan-reviews start with the right tool.

---

## A-25 — Implementation finding: `tauri-specta` 2.0.0-rc.24 rejects `skip_serializing_if` on FFI types

**Surfaced during.** Task 4 implementation (commit `34c96e0`). Hit when registering `install_plugin_agents` triggered the binding-gen pass.

**Drift.** `InstallAgentsResult` (in `service/mod.rs:387`) had `#[serde(default, skip_serializing_if = "Vec::is_empty")]` on `installed_native` and `#[serde(default, skip_serializing_if = "Option::is_none")]` on `installed_companions`. These pre-dated the type crossing FFI. Registering it as a Tauri command return failed validation:

```
Specta Serde validation failed for command 'install_plugin_agents' result:
Invalid phased type usage at 'InstallAgentsResult_Serialize.installed_native':
`skip_serializing_if` requires `apply_phases` because unified mode cannot
represent conditional omission
```

**Why `tauri-specta` rejects this.** Unified mode generates one TypeScript type that has to be valid for both serialization and deserialization. `skip_serializing_if` makes a field's *presence* conditional — meaning the deserialization input may have fewer fields than the serialization output. The unified TS type can't represent that without phase-splitting (`InstallAgentsResult_Serialize` vs `_Deserialize`), which `apply_phases` would enable but the project doesn't currently configure.

**Wire-format peers don't have this issue.** `InstallSkillsResult` and `InstallSteeringResult` — both already exported through `bindings.ts` since PRs 83 and 92 — don't use `skip_serializing_if` on any field. The pattern at this codebase is "always serialize, always deserialize, default values for missing fields." `InstallAgentsResult`'s `skip_serializing_if` was an outlier predating its FFI exposure.

**Amended.** Drop `skip_serializing_if` from both fields. Keep `serde(default)` for legacy-JSON tolerance:

```rust
#[serde(default)]
pub installed_native: Vec<crate::project::InstalledNativeAgentOutcome>,
#[serde(default)]
pub installed_companions: Option<crate::project::InstalledNativeCompanionsOutcome>,
```

**Wire format change.** Empty `installed_native` now serializes as `[]` instead of being omitted; `installed_companions: None` now serializes as `null` instead of being omitted. No Rust callers asserted on the omitted shape; no tests asserted on the omitted shape. The runtime contract for Rust consumers is preserved. The TypeScript wire shape becomes consistent with the sibling result types.

**Forward-looking rule.** Validation/result types that flow through Tauri must avoid `skip_serializing_if` — match `InstallSkillsResult`'s shape. This is a sibling rule to CLAUDE.md's existing "validation newtypes flowing through Tauri bindings need `#[cfg_attr(feature = "specta", derive(specta::Type))]`" guidance. Worth adding to CLAUDE.md once Phase 1 ships.

**Rationale.** Captured as audit trail per the "forward motion + amendments" execution rule. The implementer correctly identified the root cause, made the minimal fix, and preserved the legacy-JSON tolerance contract. The amendment names the rule for future reviewers.

---

## A-24 — Implementation finding: `remove_skill`'s `NotInstalled` leaves tracking row stale; A-12 cascade counts but doesn't truly drive to absence

**Surfaced during.** Task 3 implementation (commit `7a4718d`). Captured per the "forward motion + audit trail" rule from the Phase 1 execution session — not actioned in Task 3.

**Drift.** A-12's "log + count + continue" recipe assumes that catching `*Error::NotInstalled` from a per-content remove is equivalent to "the entry is now gone." That holds for `remove_steering_file` and `remove_agent` (Tasks 3b/3c — both methods drop the tracking entry as the FIRST step, then unlink, so a missing on-disk file is still success and the tracking row is gone). It does NOT hold for the existing `remove_skill` (`project.rs:562`): when `!skill_dir.exists()`, `remove_skill` returns `Err(SkillError::NotInstalled)` *without* mutating `installed-skills.json`. The orphan tracking row persists.

**User-visible effect.** A user clicks "Remove plugin" on a plugin whose `.kiro/skills/<name>/` was hand-deleted. The cascade reports `skills_removed: 1` and the UI redraws as "removed." But `installed_plugins()` will still surface that orphan tracking row on the next list call — the plugin reappears.

**Why out-of-scope for Task 3.** Fixing `remove_skill` is a behavioral change to a pre-existing public API on `KiroProject` — the asymmetry vs. `remove_steering_file` / `remove_agent` (which Task 3 introduced) is a pre-existing design quirk. Tightening it touches the existing `remove_skill_*` test suite and may surface invariants we'd rather not change mid-implementation.

**Two equally valid future fixes — pick during follow-up:**

1. **Tighten `remove_skill`** to drop the tracking entry on `!dir.exists()` instead of returning `NotInstalled`. Symmetric with the new sibling methods. Risk: a caller relying on the current "tracking unchanged on NotInstalled" contract would break — needs a grep for `SkillError::NotInstalled` consumers to gauge the blast radius.

2. **Tighten the cascade** to manually drop the skill tracking entry when it catches `SkillError::NotInstalled`, via direct `with_file_lock` + `load_installed` + remove + save. The `remove_skill` API stays unchanged; only the cascade's recovery path becomes complete. Risk: the cascade gains intimate knowledge of tracking-file shape that lives elsewhere.

**Recommendation:** option 1, tracked for a Phase 1.5 follow-up PR after Phase 1 ships. The orphan-skill scenario is real but rare (user manually `rm -rf`'d a skill), and the user can recover by clicking "Remove" again after a `Refresh` (which would now show no entry — wait, no, the entry persists; never mind, *the user can hand-edit the tracking JSON*, which is the documented escape hatch per CLAUDE.md "tracking files are user-owned").

The Task 3 orphan-recovery test (`remove_plugin_recovers_from_orphan_skill_tracking_entry`) was written as a regression lock for the cascade's no-abort behavior — NOT as a "drives to absence" test. The test asserts `result.skills_removed == 1` and that the cascade returns `Ok`. It does NOT assert that the tracking is empty afterward, because today it isn't. Future-fix PR should expand that test once `remove_skill` (or the cascade) is tightened.

**Rationale.** Captured for audit trail per the "23 amendments + forward motion" rule. The implementer correctly chose to leave `remove_skill` alone — A-12's recipe didn't mandate "drive to absence," only "log + count + continue." The semantic gap is real but doesn't block Phase 1 from shipping; Phase 1.5 can close it.

---

## Summary of changes

- A-1: Single line fix in Task 1 step 4 (`self.` → `Self::`).
- A-2: Two new tasks (3a, 3b) before the cascade. Real prerequisites,
  not "if missing."
- A-3: Add `native_companions` cleanup in the cascade. Possibly a new
  helper at Task 3b.5.
- A-4: Reuse existing `validate_tracking_path_entry` helper in new
  `remove_steering_file` / `remove_agent` methods. Add regression
  test that `remove_plugin` propagates the load-time validation Err.
  (Original ask was canonicalize-and-prefix; LSP-first analysis showed
  the helper already exists.)
- A-5: Second JSON-shape rstest case for the populated-subresult shape.
- A-6: Replace lexicographic version comparison with "version of the
  latest install by timestamp."
- A-7: Process correction — future plan-reviews must run each gate,
  not paraphrase.
- A-8: Tooling correction — use LSP `documentSymbol` for Gate 1,
  not grep. Spot fix that LSP would have surfaced earlier and at
  lower cost.
- A-9: Hoist `chrono::Utc::now()` out of `installed_plugins`'s map
  closure. Caught by gemini-code-assist on PR #93. Surfaces a
  separate semantic concern — `unwrap_or(now)` may be papering over
  a malformed-tracking case that should fail loudly instead.
- A-10: Task 8's inline `pluginKey` drops BrowseTab's `` delim,
  breaking the collision-safe contract. Lift `pluginKey`/`skillKey`/
  `parsePluginKey` to `$lib/keys.ts`; both files import.
- A-11: Task 7's conditional structure under-specifies how
  `browseView` composes with the existing chain. Explicit before/
  after rendering tree shown. PR 92's empty-state plugin cards
  retired in favor of the new Plugins view as primary surface.
- A-12: `remove_plugin` cascade aborts on orphan tracking entries.
  Use `match` + log + count-as-removed for `NotInstalled`; only
  abort on genuine fs/parse errors.
- A-13: `SteeringError::NotInstalled` is fictitious. Add the
  variant before Task 3a can use it. `AgentError::NotInstalled`
  already exists.
- A-14: Task 4's wrapper has the same `Self::` drift as Task 1.
  A-1 didn't grep sibling tasks for the bug shape.
- A-15: `is_empty()` checks on `agent_scan_paths`/`steering_scan_paths`
  are dead code (defaults always populate). Drop `Option<...>` from
  `InstallPluginResult` sub-results — always populate.
- A-16: `native_companions` cleanup over-deletes when two
  marketplaces ship same-named plugins. Match on
  `(plugin, marketplace)`, not `plugin` alone.
- A-17: `>=` tie-break in `update_latest` is non-deterministic
  given equal timestamps + HashMap iteration order. Use `>` for
  stable first-seen-wins.
- A-18: Task 5's wrapper drops `accept_mcp`. Plumb it through
  per the design doc's Gate 2 action item.
- A-19: `list_installed_plugins_impl` and `remove_plugin_impl`
  don't follow CLAUDE.md's `_impl(svc, ...)` rule. The existing
  `list_installed_skills` precedent is "no `_impl` for non-
  service-consuming commands." Match precedent (a).
- A-20: `make_kiro_project` is private to `commands/steering.rs::tests`.
  Hoist to `service::test_support::make_kiro_project` as
  prerequisite Task 5.0.
- A-21: A-1's amendment text over-claimed scope — the steering
  call was already correct, only the agents arm needed the fix.
  Strike the steering reference.
- A-22: Process — Claude's three-round review on PR #93 surfaced
  9 findings (A-12 to A-20) that the LSP-first pass missed. The
  rigor lives in the checklist of review angles, not in any one
  tool. Future plan-reviews run BOTH LSP-first AND
  code-reviewer-style.
- A-23: `DateTime<Utc>` fields on `InstalledPluginInfo` are
  incompatible with `kiro-market-core`'s specta feature config
  (no `"chrono"` flag). Convert to `String` (RFC3339) at the FFI
  boundary, matching the existing `InstalledSkillInfo.installed_at`
  precedent. `PathBuf` survives — verified `InstalledSteeringOutcome`
  uses it under specta::Type today.
- A-24: Implementation finding from Task 3. `remove_skill`'s
  `NotInstalled` doesn't drop the tracking row, so the cascade's
  A-12 "count as removed" path leaves stale tracking. Out of scope
  for Phase 1; deferred to Phase 1.5 follow-up. Captured here as
  audit trail per the "forward motion + amendments" execution rule.
- A-25: Implementation finding from Task 4. `tauri-specta` 2.0.0-rc.24
  rejects `skip_serializing_if` on Tauri-exposed result types in
  unified mode. Drop the directives from `InstallAgentsResult.installed_native`
  and `InstallAgentsResult.installed_companions`; the wire format
  becomes `[]`/`null` for empty/None instead of omitted, matching
  `InstallSkillsResult` and `InstallSteeringResult`. Forward-looking
  rule worth adding to CLAUDE.md.

No design-doc revisions required. The amendments are all execution-time
corrections; the architecture in
`2026-04-29-plugin-first-install-design.md` stands as written.

## References

- `docs/plan-review-checklist.md` — the 5 gates this pass applied
- `2026-04-24-stage2-3-plan-amendments.md` — format precedent
- `2026-04-23-plan-review-findings.md` — bar for "what good looks like"
- Source SHA at review time: `de59270` (post-PR-92 main)

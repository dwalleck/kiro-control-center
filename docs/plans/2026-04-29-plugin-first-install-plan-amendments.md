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

No design-doc revisions required. The amendments are all execution-time
corrections; the architecture in
`2026-04-29-plugin-first-install-design.md` stands as written.

## References

- `docs/plan-review-checklist.md` — the 5 gates this pass applied
- `2026-04-24-stage2-3-plan-amendments.md` — format precedent
- `2026-04-23-plan-review-findings.md` — bar for "what good looks like"
- Source SHA at review time: `de59270` (post-PR-92 main)

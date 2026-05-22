# Budgeted Plan — agents-view slice 1

Source: design at `./design-slice-1.md` (8 claims, cheapest falsifier C2 passed).
Spec: `./spec.md` (14 decisions, 14 edge cases, 8 success criteria).
Probe + oracle: `./probe/README.md` (AGREE on 3 fixtures).
Follow-up issues filed: kiro-6g6r (CI gate), kiro-vgnw, kiro-gwo4, kiro-3ll2,
kiro-ttew, kiro-zqci (slices 2-6), kiro-fd40 (broken-row UX, deferred).

Each sub-slice below is independently reviewable. The contract is the slice's
**Claim / Oracle / Stress fixture / Loop budget / Files / Verification**
fields — the code blocks under "Code (advisory)" are suggestions, not
dictation. If the implementer finds a cleaner shape that still passes the
fixture and oracle, they take it.

Implementation order is dependency-driven: each slice's tests assume the
previous slice's code is present. The whole sequence is one PR; sub-slices
become individual commits for reviewable bisection.

---

## Backend (Rust, in `crates/kiro-market-core/` and `crates/kiro-control-center/src-tauri/`)

### S1 — Vendor `agent-spec.json` into the workspace

**Claim:** C8 part 1. The workspace owns its validation target; the design
bundle directory can be deleted post-port without breaking tests.

**Oracle:** SHA-256 byte equality between the new vendored copy and the design
bundle's original. The schemas are confirmed authoritative per spec
decision #11.

**Stress fixture:** N/A — pure file copy, no logic. (Per skill rule 4: pure
schema slices are exempt from fixtures.)

**Loop budget:** N/A — no loop.

**Wall budget:** N/A — not an always-on phase.

**Files:**
- `crates/kiro-market-core/schemas/agent-spec.json` (new)

**Code (advisory):**
```
$ mkdir crates/kiro-market-core/schemas
$ cp "Kiro Control Center Design System/design_handoff_agents/schemas/agent-spec.json" \
     crates/kiro-market-core/schemas/agent-spec.json
```

**Verification:**
- [ ] `crates/kiro-market-core/schemas/agent-spec.json` exists
- [ ] SHA-256 matches the design bundle's original
- [ ] `cargo build --workspace` still passes (vendoring a JSON file shouldn't change anything; this verifies it's not accidentally pulled into `include_str!` somewhere it breaks)

---

### S2 — Wire-format types: `UserAgentRow`, `UserAgentLineage`

**Claim:** Precursor to C1 — the shape `list_user_agents` produces.

**Oracle:** A unit test serializes both shapes (one with `lineage: Some(...)`,
one with `lineage: None`) to JSON and asserts the field set matches the
TypeScript shape the frontend (slice 10+) will read from `bindings.ts`. The
oracle is the *target JSON shape* documented in the design's input-shapes
table — written down before this code is.

**Stress fixture:** Build a `UserAgentRow` with:
- `name = "agent-with-üñîçødé"` (Unicode validity at type level)
- `description = None`
- `model = None`
- All counts = `usize::MAX` (boundary value — would catch a `u32` truncation bug if someone "tightened" the type)
- `lineage = Some(UserAgentLineage { marketplace: "m".into(), plugin: "p".into(), version: Some("0.0.0-pre".into()) })`

Serialize, deserialize, assert structural equality. If a future change
makes `name: String` non-Unicode-safe or downgrades counts to `u32`, the
fixture fails.

**Loop budget:** N/A — no loops; pure type definitions.

**Files:**
- `crates/kiro-market-core/src/user_agent.rs` (new, ~50 LOC)
- `crates/kiro-market-core/src/lib.rs` (1 line: `pub mod user_agent;`)

**Code (advisory):**
```rust
//! User-authored agent surface for the kiro-control-center "Workflows >
//! Agents" view. Distinct from the marketplace-install path in
//! [`crate::agent::parse_native`] — this module is read/write shapes only.

use serde::{Deserialize, Serialize};

/// One row of the agents list-page payload. See
/// [`crate::project::KiroProject::list_user_agents`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct UserAgentRow {
    pub name: String,
    pub description: Option<String>,
    pub model: Option<String>,
    pub tools_count: usize,
    pub mcp_count: usize,
    pub resources_count: usize,
    pub hooks_count: usize,
    pub lineage: Option<UserAgentLineage>,
}

/// Marketplace lineage badge data. Present iff the agent's name appears in
/// `installed-agents.json#/agents`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct UserAgentLineage {
    pub marketplace: String,
    pub plugin: String,
    pub version: Option<String>,
}
```

**Verification:**
- [ ] `cargo test -p kiro-market-core user_agent::` passes (stress fixture round-trip)
- [ ] `cargo check -p kiro-market-core --features specta` passes (specta derive works)
- [ ] No `pub fn` added — module is types-only at this slice

---

### S3 — `KiroProject::list_user_agents`

**Claim:** C1 (list output spec) + C2 (untyped JSON, never `parse_native`).

**Oracle:** `probe.py` from `.agents-view/probe/probe.py`. Run on the same
fixture; row sets must match. The probe is independent of the Rust
implementation (different language, different JSON parser) and already
agrees with `oracle.ps1`.

**Stress fixture:** The existing fixture at
`.agents-view/probe/fixture/.kiro/`, copied into a `tempfile::tempdir()` for
the test. Contains:
1. `marketplace-tracked.json` (tracked + on disk → row with lineage)
2. `user-authored.json` (untracked + on disk → row, no lineage)
3. `no-name.json` (untracked + on disk + NO `name` field → row with `name = "no-name"` from filename stem; would crash `parse_native::parse_native_kiro_agent_file`)
4. `orphan-tracking` is in `installed-agents.json` but no file → ABSENT from output (spec D12)

Expected output: 3 rows, sorted by name, exactly matching `probe.py`'s
output for the same fixture.

**Loop budget:** Single directory walk over `agents_dir().read_dir()`. **O(F)
where F = files in `.kiro/agents/`.** Production scale: F ≤ 200 (a user with
200 distinct agents is unrealistic; the existing seed has 7). Per-file work:
one `fs::read`, one `serde_json::from_slice`, one `HashMap::get` against
tracking. Total ops at scale: 200 × ~4 = 800 ops. **Well within 10^6
budget.**

**Wall budget:** N/A — called on user action (tab open / post-write
refresh), not always-on. Spec criterion S6 sets ≤50 ms for SAVE latency;
LIST is structurally simpler.

**Files:**
- `crates/kiro-market-core/src/project.rs` (append ~60 LOC; existing file)
- `crates/kiro-market-core/src/user_agent.rs` (none — type already defined)

**Code (advisory):**
```rust
impl KiroProject {
    /// Build the list-page payload for the agents view.
    ///
    /// Reads every `*.json` file in `agents_dir()`, projects each to a
    /// [`UserAgentRow`], and attaches marketplace lineage from
    /// [`Self::load_installed_agents`] when the agent's name is tracked.
    ///
    /// **Untyped JSON.** This method parses each file via
    /// `serde_json::from_slice::<serde_json::Value>` — NOT
    /// [`crate::agent::parse_native::parse_native_kiro_agent_file`].
    /// The install path's security checks (symlink refusal, 1 MiB byte
    /// cap, required `name` field) are appropriate when copying
    /// untrusted marketplace bytes; the list path operates on files the
    /// user already owns and would only be hampered by them. Pinned by
    /// design claim C2 and the no-name fixture in
    /// `.agents-view/probe/fixture/`.
    ///
    /// Creates `agents_dir()` if absent (idempotent `create_dir_all`).
    /// Empty list returned when the directory exists but contains no
    /// `*.json` files. Files that fail JSON parsing are logged
    /// (`tracing::warn!`) and excluded — see spec D13.
    ///
    /// Output is sorted by `name` ascending for stable comparison
    /// against [`probe.py`].
    ///
    /// # Errors
    ///
    /// I/O failure reading the directory or the tracking file.
    pub fn list_user_agents(&self) -> crate::error::Result<Vec<UserAgentRow>> {
        let dir = self.agents_dir();
        fs::create_dir_all(&dir)?;

        let tracking = self.load_installed_agents()?;

        let mut rows = Vec::new();
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            let bytes = match fs::read(&path) {
                Ok(b) => b,
                Err(e) => {
                    warn!(path = %path.display(), error = %e, "skipping unreadable agent file");
                    continue;
                }
            };
            let value: serde_json::Value = match serde_json::from_slice(&bytes) {
                Ok(v) => v,
                Err(e) => {
                    warn!(path = %path.display(), error = %e, "skipping unparseable agent file (spec D13)");
                    continue;
                }
            };
            let name = value
                .get("name")
                .and_then(|v| v.as_str())
                .map(String::from)
                .unwrap_or_else(|| {
                    path.file_stem()
                        .and_then(|s| s.to_str())
                        .map_or_else(String::new, String::from)
                });
            let lineage = tracking.agents.get(&name).map(|m| UserAgentLineage {
                marketplace: m.marketplace.to_string(),
                plugin: m.plugin.to_string(),
                version: m.version.clone(),
            });
            rows.push(UserAgentRow {
                name,
                description: value.get("description").and_then(|v| v.as_str()).map(String::from),
                model: value.get("model").and_then(|v| v.as_str()).map(String::from),
                tools_count: value.get("tools").and_then(|v| v.as_array()).map_or(0, Vec::len),
                mcp_count: value.get("mcpServers").and_then(|v| v.as_object()).map_or(0, |o| o.len()),
                resources_count: value.get("resources").and_then(|v| v.as_array()).map_or(0, Vec::len),
                hooks_count: value
                    .get("hooks")
                    .and_then(|v| v.as_object())
                    .map_or(0, |o| o.values().filter_map(|v| v.as_array()).map(Vec::len).sum()),
                lineage,
            });
        }
        rows.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(rows)
    }
}
```

**Verification:**
- [ ] `cargo test list_user_agents_against_probe_fixture` passes — output equals `probe.py` on the same fixture (assert via JSON string equality after sorting both by name)
- [ ] Loop visits each `*.json` file exactly once (instrument with a counter in the test if necessary)
- [ ] `tracing::warn!` is emitted for the unparseable case (capture via `tracing-test` or similar; alternatively, accept as untested-but-documented)
- [ ] **Doc-comment-as-contract:** "Creates agents_dir if absent" is load-bearing for correctness (a missing dir would yield `NotFound`); enforced by the runtime `fs::create_dir_all` call (not `debug_assert!`)
- [ ] **Output stream:** rows are returned as data via `Result<Vec<_>>`; logs go to stderr via `tracing::warn!`. Correct data/diagnostic split

---

### S4 — `KiroProject::create_user_agent`

**Claim:** C3 (atomic, validating, collision-rejecting create).

**Oracle:** Filesystem inspection — before/after content hashes
(`hash::BlakeHash` already exists in the crate). The pre-existing file's
hash MUST NOT change when the create returns `NameCollision`.

**Stress fixture:** rstest with 4 cases:
1. **Happy path**: name `"new-agent"`, no existing file → file written with given bytes; second list call shows the row
2. **Name validation — empty**: name `""` → `AgentSaveError::InvalidName`, no file written
3. **Name validation — regex-violating**: names `"Bad"`, `"-leads"`, `".dotted"`, `"has spaces"` → each `InvalidName`
4. **Collision**: pre-write `foo.json` with byte content `b"original"`, call `create_user_agent` with name `"foo"` and bytes `b"replacement"` → `NameCollision`; assert `BlakeHash::of(&fs::read("foo.json")?) == BlakeHash::of(b"original")`

A naive implementation that uses `fs::write` (which silently overwrites)
falsifies case 4. A regex-skipping implementation falsifies cases 2-3.

**Loop budget:** N/A — single file write, single regex match in
`AgentName::new`.

**Files:**
- `crates/kiro-market-core/src/project.rs` (append ~40 LOC)
- `crates/kiro-market-core/src/user_agent.rs` (append `AgentSaveError` enum, ~25 LOC; or extend `crate::error::MarketplaceError` — implementer's call)

**Code (advisory):**
```rust
impl KiroProject {
    /// Atomically write a new user-authored agent JSON file.
    ///
    /// Validates the draft's `name` via [`AgentName::new`] and rejects
    /// existing-file collision with `NameCollision` *before* writing.
    /// File write goes through [`crate::cache::atomic_write`] so a
    /// crash mid-write leaves the target either fully written or
    /// fully absent.
    ///
    /// # Errors
    /// - `InvalidName` — `draft_name` fails the [`AgentName`] regex
    /// - `NameCollision` — `<agents_dir>/<name>.json` already exists
    /// - I/O failure from atomic_write
    pub fn create_user_agent(
        &self,
        draft_name: &str,
        draft_bytes: &[u8],
    ) -> Result<(), AgentSaveError> {
        let name = AgentName::new(draft_name)
            .map_err(|e| AgentSaveError::InvalidName { reason: e.to_string() })?;
        let target = self.agents_dir().join(format!("{}.json", name.as_str()));
        // Load-bearing collision check: silent overwrite would lose user data.
        if target.exists() {
            return Err(AgentSaveError::NameCollision { name: name.into_inner() });
        }
        fs::create_dir_all(self.agents_dir())?;
        crate::cache::atomic_write(&target, draft_bytes)?;
        Ok(())
    }
}
```

**Verification:**
- [ ] All 4 rstest cases pass
- [ ] Pre/post hash assertion in case 4 succeeds (collision did not overwrite)
- [ ] **Doc-comment-as-contract:** "rejects existing-file collision *before* writing" is load-bearing; enforced by the runtime `target.exists()` check (NOT `debug_assert!`)
- [ ] No `.unwrap()`/`.expect()` in the implementation (CLAUDE.md zero-tolerance)

---

### S5 — `KiroProject::save_user_agent` (in-place + rename + detach)

**Claim:** C4. Transactional save with optional rename and optional detach.

**Oracle:** Filesystem inspection + `load_installed_agents()` post-call.

**Stress fixture:** rstest with 5 cases (each runs the save under a fresh
`tempfile::tempdir()`):
1. **In-place save** (`from_name == draft.name`, untracked): pre-write `foo.json` with `b"v1"`, call `save("foo", b"v2", detach=false)`. Assert `foo.json` content == `b"v2"`, no `bar.json`, tracking unchanged
2. **Rename, no collision**: pre-write `foo.json`, call `save("foo", draft_with_name_bar, detach=false)`. Assert `bar.json` exists with new content, `foo.json` absent
3. **Rename, collision** (adversarial — the bug class is "implementation forgets to check before writing"): pre-write `foo.json` AND `bar.json` (DIFFERENT content), call `save("foo", draft_with_name_bar, detach=false)`. Assert BOTH files unchanged (hash equality pre/post), `NameCollision` error returned
4. **Detach=true on tracked agent**: pre-install a marketplace agent `m-agent` via `install_native_agent` (so tracking entry exists), call `save("m-agent", draft, detach=true)`. Assert `m-agent.json` has new content AND `tracking.agents.contains_key("m-agent") == false`
5. **Detach=false on tracked agent**: same setup as case 4, but `detach=false`. Assert `m-agent.json` has new content AND `tracking.agents.contains_key("m-agent") == true` (lineage preserved)

A naive write-old-unlink-then-write-new implementation fails case 3 (the
write happens before the collision check). An implementation that drops
tracking unconditionally fails case 5.

**Loop budget:** N/A — no loops. Single `with_file_lock` block; 2-4 fs
operations within.

**Files:**
- `crates/kiro-market-core/src/project.rs` (append ~70 LOC — over the 50-LOC heuristic; justified because the transactional logic doesn't split cleanly across slices)

**Code (advisory):**
```rust
impl KiroProject {
    /// Save an edited user-authored agent. Handles three orthogonal shapes:
    /// in-place edit, rename, and detach-from-marketplace.
    ///
    /// `from_name` is the filename stem of the agent being edited (i.e.,
    /// the existing `<from_name>.json`). `draft_name` is the post-edit
    /// name (may equal `from_name` for in-place; may differ for rename).
    ///
    /// All work occurs under [`crate::file_lock::with_file_lock`] on
    /// the tracking file path, even for untracked agents — this
    /// serializes concurrent saves on the same project and is the same
    /// lock used by `install_native_agent` and `remove_agent`.
    ///
    /// Ordering inside the lock (file-first; matches `install_native_agent`):
    /// 1. If `from_name != draft_name` and `<draft_name>.json` already
    ///    exists, return `NameCollision`. **No writes.**
    /// 2. Write `<draft_name>.json` atomically.
    /// 3. If `from_name != draft_name`, unlink `<from_name>.json`
    ///    (best-effort; unlink failure yields a warn log but returns
    ///    `Ok(())` — the new file is correctly in place).
    /// 4. If `detach && tracking.agents.contains_key(from_name)`,
    ///    remove the tracking entry and persist tracking.
    ///
    /// Crash semantics:
    /// - Crash between (2) and (3) leaves both files; user sees the new
    ///   one and a stale old one; re-saving cleans up.
    /// - Crash between (3) and (4) leaves a renamed-but-still-tracked
    ///   agent — handled by existing `ContentChangedRequiresForce` on
    ///   next marketplace update attempt.
    pub fn save_user_agent(
        &self,
        from_name: &str,
        draft_name: &str,
        draft_bytes: &[u8],
        detach: bool,
    ) -> Result<(), AgentSaveError> {
        let from = AgentName::new(from_name).map_err(|e| AgentSaveError::InvalidName { reason: e.to_string() })?;
        let target = AgentName::new(draft_name).map_err(|e| AgentSaveError::InvalidName { reason: e.to_string() })?;
        let agents_dir = self.agents_dir();
        let from_path = agents_dir.join(format!("{}.json", from.as_str()));
        let target_path = agents_dir.join(format!("{}.json", target.as_str()));

        crate::file_lock::with_file_lock(&self.agent_tracking_path(), || {
            // 1. Collision check (only when renaming).
            if from.as_str() != target.as_str() && target_path.exists() {
                return Err(AgentSaveError::NameCollision { name: target.clone().into_inner() });
            }
            fs::create_dir_all(&agents_dir)?;
            // 2. Atomic write of the new content.
            crate::cache::atomic_write(&target_path, draft_bytes)?;
            // 3. Unlink the old file iff renaming. Best-effort.
            if from.as_str() != target.as_str() {
                if let Err(e) = fs::remove_file(&from_path) {
                    if e.kind() != std::io::ErrorKind::NotFound {
                        warn!(path = %from_path.display(), error = %e, "post-rename unlink failed; old file orphaned");
                    }
                }
            }
            // 4. Detach from tracking if requested AND lineage exists.
            if detach {
                let mut installed = self.load_installed_agents()?;
                if installed.agents.remove(from.as_str()).is_some() {
                    self.write_agent_tracking(&installed)?;
                }
            }
            Ok(())
        })
    }
}
```

**Verification:**
- [ ] All 5 rstest cases pass
- [ ] Case 3's pre/post hash equality assertion succeeds on BOTH files (no collision-induced corruption)
- [ ] Case 4 asserts tracking entry absent after save
- [ ] Case 5 asserts tracking entry preserved
- [ ] **Doc-comment-as-contract:** All four numbered preconditions are load-bearing; each is enforced by an explicit runtime check (target.exists, file_lock, atomic_write, conditional fs::remove_file)
- [ ] **LOC overage** acknowledged: ~70 LOC vs 50 LOC heuristic; the transactional block can't split cleanly without losing the atomicity claim

---

### S6 — `KiroProject::delete_user_agent`

**Claim:** C5 (tracking-aware delete; reuses existing `remove_agent`).

**Oracle:** Filesystem + `load_installed_agents()` post-call.

**Stress fixture:** rstest with 3 cases:
1. **Untracked delete**: pre-write `foo.json` (NO tracking entry), call delete. Assert `foo.json` absent, tracking unchanged (still no `foo` entry)
2. **Tracked delete**: pre-install marketplace agent `m-agent` (so tracking entry + JSON + prompt file all exist), call delete. Assert `m-agent.json` absent, `prompts/m-agent.md` absent, `tracking.agents.contains_key("m-agent") == false`
3. **Idempotent on missing file**: pre-state has no `foo.json` and no tracking entry. Call delete. Assert `Ok(())` returned (NOT an error)

A naive `KiroProject::remove_agent`-only implementation falsifies case 1
(it'd return `AgentError::NotInstalled` for the untracked agent). An
implementation that fails on missing files falsifies case 3.

**Loop budget:** N/A — single match + one of two branches.

**Files:**
- `crates/kiro-market-core/src/project.rs` (append ~20 LOC)

**Code (advisory):**
```rust
impl KiroProject {
    /// Delete a user-visible agent.
    ///
    /// Tracking-aware: if the agent has an `InstalledAgents` entry,
    /// delegates to [`Self::remove_agent`] (file lock + tracking update
    /// + file unlink with rollback). Otherwise, performs a direct
    /// `fs::remove_file`. Both paths are idempotent on
    /// `ErrorKind::NotFound`.
    pub fn delete_user_agent(&self, name: &str) -> Result<(), AgentDeleteError> {
        let name = AgentName::new(name).map_err(|e| AgentDeleteError::InvalidName { reason: e.to_string() })?;
        let installed = self.load_installed_agents()?;
        if installed.agents.contains_key(name.as_str()) {
            self.remove_agent(name.as_str())?;
            return Ok(());
        }
        let target = self.agents_dir().join(format!("{}.json", name.as_str()));
        match fs::remove_file(&target) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(AgentDeleteError::IoFailure(e)),
        }
    }
}
```

**Verification:**
- [ ] All 3 rstest cases pass
- [ ] Case 2 verifies the rollback-on-unlink-failure path is preserved (since delegation to `remove_agent` inherits it for free)

---

### S7 — `KiroProject::duplicate_user_agent`

**Claim:** C6 (collision-walking, lineage-stripping duplicate).

**Oracle:** Filesystem listing + `load_installed_agents()` post-call.

**Stress fixture:** rstest with 4 cases:
1. **`-copy` free**: source `foo.json` exists; `foo-copy.json` does not. Call duplicate. Assert returns `"foo-copy"`, `foo-copy.json` exists with same content as source
2. **Chain occupied** (adversarial — the bug class is "naively use -copy and overwrite"): pre-write `foo.json`, `foo-copy.json`, `foo-copy-2.json`. Call duplicate. Assert returns `"foo-copy-3"`, `foo-copy-3.json` exists
3. **Source missing**: no `foo.json`. Call duplicate. Assert `AgentDuplicateError::SourceNotFound`
4. **Source marketplace-tracked → duplicate is user-authored**: pre-install marketplace agent `m-agent`. Call duplicate(`m-agent`). Assert returns `"m-agent-copy"`; the duplicate is NOT in `tracking.agents`; the original IS

A naive implementation that always uses `-copy` falsifies case 2 (overwrite
of existing `foo-copy.json`). An implementation that copies tracking
metadata into the duplicate falsifies case 4.

**Loop budget:** Find-next-free loop. **O(N) where N = count of taken
`<source>-copy[-X]` names.** Production scale: N ≤ 10 realistically (a
user who duplicates an agent more than ten times is doing something
unusual; we'd notice). Per-iteration cost: one `Path::exists` check (one
syscall). Total: ≤ 10 syscalls. **Well within 10^3 syscall budget.**

**Files:**
- `crates/kiro-market-core/src/project.rs` (append ~45 LOC)

**Code (advisory):**
```rust
impl KiroProject {
    /// Duplicate a user-visible agent. Walks `<source>-copy`,
    /// `<source>-copy-2`, `<source>-copy-3`, ... finding the first
    /// unused name. The duplicate is always user-authored — never
    /// carries marketplace lineage even if the source has it.
    pub fn duplicate_user_agent(&self, source_name: &str) -> Result<String, AgentDuplicateError> {
        let source = AgentName::new(source_name).map_err(|e| AgentDuplicateError::InvalidName { reason: e.to_string() })?;
        let source_path = self.agents_dir().join(format!("{}.json", source.as_str()));
        if !source_path.exists() {
            return Err(AgentDuplicateError::SourceNotFound { name: source.into_inner() });
        }
        let source_bytes = fs::read(&source_path)?;

        // Walk -copy, -copy-2, -copy-3, ... up to a sanity cap of 10000.
        let target_name = (1..=10_000)
            .find_map(|k| {
                let candidate = if k == 1 {
                    format!("{}-copy", source.as_str())
                } else {
                    format!("{}-copy-{}", source.as_str(), k)
                };
                let path = self.agents_dir().join(format!("{candidate}.json"));
                if path.exists() { None } else { Some(candidate) }
            })
            .ok_or(AgentDuplicateError::NameSpaceExhausted)?;

        // Rewrite the JSON's `name` field to the new name (so the file is
        // self-consistent with its filename).
        let mut value: serde_json::Value = serde_json::from_slice(&source_bytes)
            .map_err(|e| AgentDuplicateError::ParseFailure { reason: e.to_string() })?;
        if let Some(obj) = value.as_object_mut() {
            obj.insert("name".into(), serde_json::Value::String(target_name.clone()));
        }
        let new_bytes = serde_json::to_vec_pretty(&value)?;

        let target_path = self.agents_dir().join(format!("{target_name}.json"));
        crate::cache::atomic_write(&target_path, &new_bytes)?;
        // Intentionally NOT inserting into `InstalledAgents` — see C11.
        Ok(target_name)
    }
}
```

**Verification:**
- [ ] All 4 rstest cases pass
- [ ] Loop runs ≤10 iterations on case 2 fixture (instrument with a counter)
- [ ] **NameSpaceExhausted** sanity cap is documented; the 10,000 bound was chosen because real-world users won't hit it, while the loop must terminate

---

### S8 — Tauri command wrappers in `agents_authoring.rs`

**Claim:** C7 (project-only commands; no `MarketplaceService`).

**Oracle:** Static grep on the new file:
`grep -E '(make_service|MarketplaceService)' crates/kiro-control-center/src-tauri/src/commands/agents_authoring.rs`
must return zero matches. This is the manual form of the future CI gate
(kiro-6g6r).

**Stress fixture:** Each command has a `#[cfg(test)] mod tests` that calls
the wrapper directly with constructed primitives (no Tauri runtime) and
asserts the result. The fixture verifies:
- `list_user_agents` returns 0 rows on a fresh `tempfile::tempdir()`
- `create_user_agent` with a colliding name returns `CommandError::Validation` with the message containing `"NameCollision"` (or equivalent typed surface)
- Each wrapper validates `project_path` via the existing `validate_kiro_project_path` helper (the negative test: passing a nonexistent path returns a typed error, not a panic)

**Loop budget:** N/A — wrappers are thin pass-throughs.

**Files:**
- `crates/kiro-control-center/src-tauri/src/commands/agents_authoring.rs` (new, ~120 LOC across 5 wrappers + a tests module)

**Code (advisory):**
```rust
//! User-authored agent CRUD commands. PROJECT-ONLY — none of these
//! construct or accept a `MarketplaceService`. Per CLAUDE.md, the body
//! inlines in the wrapper (no `_impl(svc, ...)` pattern).
//!
//! CI gate kiro-6g6r will enforce the no-MarketplaceService invariant
//! once it lands; until then, the manual grep on this file is the fence.

use kiro_market_core::project::KiroProject;
use kiro_market_core::user_agent::UserAgentRow;
// ... etc.

#[tauri::command]
#[specta::specta]
pub async fn list_user_agents(project_path: String) -> Result<Vec<UserAgentRow>, CommandError> {
    let project_root = validate_kiro_project_path(&project_path)?;
    let project = KiroProject::new(project_root);
    project.list_user_agents().map_err(CommandError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn create_user_agent(
    name: String,
    draft_json: String,
    project_path: String,
) -> Result<(), CommandError> {
    let project_root = validate_kiro_project_path(&project_path)?;
    let project = KiroProject::new(project_root);
    project
        .create_user_agent(&name, draft_json.as_bytes())
        .map_err(CommandError::from)
}

// ... save_user_agent, delete_user_agent, duplicate_user_agent
```

**Verification:**
- [ ] Grep returns zero matches for `MarketplaceService` / `make_service` in `agents_authoring.rs`
- [ ] All 5 commands have at least one unit test in `#[cfg(test)] mod tests`
- [ ] Each wrapper validates `project_path` via `validate_kiro_project_path` (negative test: malformed path returns typed error, not panic)
- [ ] **LOC overage** acknowledged: 120 LOC across 5 wrappers (~24 LOC each); each individual command is well under 50 LOC, and they live in one file for cohesion

---

### S9 — Register commands + regenerate `bindings.ts`

**Claim:** C8 part 2 (idempotent bindings regen).

**Oracle:** `cargo test -p kiro-control-center --lib generate_types -- --exact --ignored`
runs twice in succession; the second run produces zero `git diff` on
`bindings.ts`.

**Stress fixture:** The idempotency check itself is the fixture. The
"plausible bug" is a non-deterministic specta export (e.g., HashMap field
order); running twice catches it.

**Loop budget:** N/A.

**Files:**
- `crates/kiro-control-center/src-tauri/src/commands/mod.rs` (1 line: `pub mod agents_authoring;`)
- `crates/kiro-control-center/src-tauri/src/lib.rs` (add 5 commands to the `collect_commands![]` macro)
- `crates/kiro-control-center/src/lib/bindings.ts` (regenerated; reviewable diff)

That's 3 files modified. The `bindings.ts` change is mechanical/generated;
the reviewable code change is in `mod.rs` + `lib.rs` (2 files).

**Code (advisory):**
```rust
// in lib.rs, inside collect_commands!:
commands::agents_authoring::list_user_agents,
commands::agents_authoring::create_user_agent,
commands::agents_authoring::save_user_agent,
commands::agents_authoring::delete_user_agent,
commands::agents_authoring::duplicate_user_agent,
```

**Verification:**
- [ ] `cargo test -p kiro-control-center --lib generate_types -- --exact --ignored` exits 0
- [ ] Run twice; second run yields `git diff --exit-code crates/kiro-control-center/src/lib/bindings.ts` exit 0
- [ ] `bindings.ts` contains `UserAgentRow`, `UserAgentLineage`, and the 5 new command bindings
- [ ] `cargo test --workspace` continues to pass

---

## Frontend (Svelte 5 + Tailwind v4, in `crates/kiro-control-center/src/`)

### S10 — List-page helpers (pure logic, vitest)

**Claim:** Per CLAUDE.md vitest discipline, the testable logic of the list
page (filtering, formatting) lives in a pure `.ts` module that vitest
covers; the `.svelte` consumer (S12) is a dumb renderer.

**Oracle:** vitest assertions against constructed `UserAgentRow` arrays.

**Stress fixture:** vitest cases:
1. **Empty query returns all rows** — including rows with `description = null` (would crash a naive `.toLowerCase().includes(query)` if `null` slips through)
2. **Case-insensitive name match**: query `"REVIEWER"` matches row `name: "code-reviewer"`
3. **Description match**: query `"orchestrator"` matches row whose name doesn't contain it but description does
4. **Model match**: query `"opus"` matches a row with `model: "claude-opus-4-7"` (would catch an implementation that only searches `name`)
5. **Unicode**: query `"üñîçødé"` correctly matches a row with that in its name

A naive `row.name.includes(q)` falsifies case 3, 4. A null-unsafe
implementation falsifies case 1.

**Loop budget:** O(rows × (|name| + |desc| + |model|)) per keystroke.
Production scale: rows ≤ 200, total string length ≤ ~500 chars per row.
Per-keystroke ops: ≤ 100k char comparisons. Within budget for input-event
work.

**Files:**
- `crates/kiro-control-center/src/lib/agents/agent-list-helpers.ts` (new, ~30 LOC)
- `crates/kiro-control-center/src/lib/agents/agent-list-helpers.test.ts` (new, ~50 LOC)

**Code (advisory):**
```ts
import type { UserAgentRow } from "$lib/bindings";

export function filterAgentRows(rows: readonly UserAgentRow[], query: string): UserAgentRow[] {
  if (!query) return [...rows];
  const q = query.toLowerCase();
  return rows.filter((r) =>
    r.name.toLowerCase().includes(q) ||
    (r.description ?? "").toLowerCase().includes(q) ||
    (r.model ?? "").toLowerCase().includes(q)
  );
}

export function formatLineageBadge(lineage: UserAgentRow["lineage"]): string | null {
  if (!lineage) return null;
  return lineage.version
    ? `${lineage.marketplace} · ${lineage.plugin} · ${lineage.version}`
    : `${lineage.marketplace} · ${lineage.plugin}`;
}
```

**Verification:**
- [ ] All 5 vitest cases pass
- [ ] `npm run check` passes (TS type-checks against bindings.ts)
- [ ] **Output stream:** helpers return values (data); no console.log

---

### S11 — NavRail "Workflows" group + Tab type extension

**Claim:** Spec B1 — the entry point.

**Oracle:** TypeScript type-check: `Tab` union must include `"Agents"`. The
running app shows the new nav item in the sidebar (manual visual check
against `screenshots/01-list.png`).

**Stress fixture:** vitest is overkill for this; the type-check IS the
fixture. If `Tab` doesn't include `"Agents"`, the `AgentsTab` rendering
branch in `App.svelte` (or whatever the dispatcher is) won't compile.

**Loop budget:** N/A.

**Files:**
- `crates/kiro-control-center/src/lib/types.ts` (1 line modified)
- `crates/kiro-control-center/src/lib/components/NavRail.svelte` (1 line modified — add the group)

**Code (advisory):**
```ts
// types.ts
export type Tab = "Browse" | "Installed" | "Marketplaces" | "Agents" | "Kiro Settings";
```
```svelte
<!-- NavRail.svelte: navGroups array -->
const navGroups: { label: string; items: { id: Tab; hasSubItems?: boolean }[] }[] = [
  { label: "Skills", items: [{ id: "Browse" }, { id: "Installed" }] },
  { label: "Sources", items: [{ id: "Marketplaces" }] },
  { label: "Workflows", items: [{ id: "Agents" }] },
  { label: "Configuration", items: [{ id: "Kiro Settings", hasSubItems: true }] },
];
```

**Verification:**
- [ ] `npm run check` passes
- [ ] App dispatcher (in `App.svelte` or equivalent) is updated to render the new tab (could be done in this slice or S12; implementer's call)

---

### S12 — `AgentsTab.svelte` (list page)

**Claim:** Spec B1, B3, B4, B5, B10, B11 — list rendering, filter, create button, duplicate icon, delete icon.

**Oracle:** Visual fidelity to `screenshots/01-list.png`. The pure-logic
helpers from S10 carry the verifiable correctness.

**Stress fixture:** Manual visual check + the e2e test in S17 exercises
this against real Tauri commands.

**Loop budget:** `{#each rows as row}` — O(rows). Production scale ≤ 200.
Trivial.

**Files:**
- `crates/kiro-control-center/src/lib/components/AgentsTab.svelte` (new, ~150 LOC including markup)

**Code (advisory):** See design bundle's `AgentsTab.jsx` for the React reference. Port to Svelte 5 runes per the README's component map.

**Verification:**
- [ ] Component renders without TS errors
- [ ] Filter input wires to `filterAgentRows` from S10
- [ ] Create button calls a callback that the parent uses to swap to editor mode
- [ ] Marketplace-tracked rows show a lineage badge (uses `formatLineageBadge` from S10)
- [ ] **LOC overage** acknowledged: ~150 LOC mostly Tailwind markup; testable logic lives in S10

---

### S13 — `AgentEditor.svelte` (editor shell, section rail, topbar)

**Claim:** Spec B5–B9, B13 — shell that hosts the panels.

**Oracle:** Visual fidelity to `screenshots/02-edit-identity.png`.

**Stress fixture:** Manual visual check. The section rail must show all 7
sections (Identity, System Prompt, Tools, MCP Servers, Resources, Hooks,
Advanced) with sections 3-7 visibly disabled (they're slices 2-6). A
plausible bug: implementer omits the disabled sections, then slice 2
implementer has to discover the rail is incomplete. Disabled-section
placeholders catch this.

**Loop budget:** N/A.

**Files:**
- `crates/kiro-control-center/src/lib/components/AgentEditor.svelte` (new, ~180 LOC mostly markup)

**Code (advisory):** See `AgentEditor.jsx` in the design bundle (the `AgentEditor` top-level component). Section rail renders all 7 entries; the active-section state is `$state<SectionId>("identity")`; non-implemented sections set `disabled` on the button.

**Verification:**
- [ ] Renders without TS errors
- [ ] Cancel button discards (calls parent callback; no autosave per B13)
- [ ] Save button calls parent with the draft + `from_name` + `detach`
- [ ] Disabled sections are visible but not clickable
- [ ] **LOC overage** acknowledged: ~180 LOC, mostly markup

---

### S14 — `IdentityPanel.svelte`

**Claim:** Spec § 3 of design — the five Identity fields.

**Oracle:** vitest on the `isValidAgentName` helper (extracted to S14's
`.ts` module).

**Stress fixture:** vitest cases for `isValidAgentName`:
1. `""` → `false`
2. `"good-name"` → `true`
3. `"Bad"` (capital) → `false`
4. `"-leads"` (leading hyphen) → `false`
5. `"a"` (single char) → `true`
6. `"a"` * 200 → `true` (long-but-valid)
7. `"has space"` → `false`
8. `"with.dot"` → `false`
9. `"naïve"` → `false` (Unicode rejected per `^[a-z0-9][a-z0-9-]*$`)

A naive `query.length > 0` check falsifies cases 3, 4, 7, 8, 9.

**Loop budget:** N/A.

**Files:**
- `crates/kiro-control-center/src/lib/components/editor/IdentityPanel.svelte` (new, ~100 LOC)
- `crates/kiro-control-center/src/lib/agents/agent-name.ts` + `.test.ts` (new, ~30 LOC + ~40 LOC tests)

**Code (advisory):**
```ts
// agent-name.ts
const AGENT_NAME_REGEX = /^[a-z0-9][a-z0-9-]*$/;

export function isValidAgentName(name: string): boolean {
  return AGENT_NAME_REGEX.test(name);
}
```

**Verification:**
- [ ] All 9 vitest cases pass
- [ ] Panel renders 5 inputs (name, description, model, keyboardShortcut, welcomeMessage)
- [ ] Name field shows inline validation error when invalid
- [ ] **Doc-comment-as-contract:** the regex is documented as MATCHING the backend's `AgentName::new` regex; a mismatch would let the UI accept names the backend rejects (load-bearing). Enforcement: a comment + a parity test that imports the regex source from a shared constant if practical. If not practical (Rust regex vs JS regex syntax differ), document the parity as a maintenance burden in the helper file

---

### S15 — `PromptPanel.svelte` (dual-mode inline/file)

**Claim:** Spec § 4 of design — segmented mode toggle with "Switching modes clears the value."

**Oracle:** vitest on the `detectPromptMode` + `clearPromptOnModeSwitch` helpers.

**Stress fixture:** vitest cases:
1. `detectPromptMode(null)` → `"inline"` (default)
2. `detectPromptMode("")` → `"inline"` (empty inline, not file)
3. `detectPromptMode("Hello")` → `"inline"`
4. `detectPromptMode("file://path/to/file.md")` → `"file"`
5. `detectPromptMode("file://")` → `"file"` (just the scheme; weird but matches `.startsWith`)
6. **Adversarial — non-canonical prefix**: `detectPromptMode("File://X")` → `"inline"` (case-sensitive per spec)
7. `clearPromptOnModeSwitch` from inline to file → returns `"file://"`
8. `clearPromptOnModeSwitch` from file to inline → returns `""`

A naive `startsWith("file:")` falsifies case 5/6 differently than the spec intends.

**Loop budget:** N/A.

**Files:**
- `crates/kiro-control-center/src/lib/components/editor/PromptPanel.svelte` (new, ~130 LOC)
- `crates/kiro-control-center/src/lib/agents/prompt-mode.ts` + `.test.ts` (new, ~20 LOC + ~40 LOC tests)

**Code (advisory):**
```ts
export type PromptMode = "inline" | "file";

export function detectPromptMode(value: string | null): PromptMode {
  return (value ?? "").startsWith("file://") ? "file" : "inline";
}

export function clearPromptOnModeSwitch(target: PromptMode): string {
  return target === "file" ? "file://" : "";
}
```

**Verification:**
- [ ] All 8 vitest cases pass
- [ ] Panel renders both modes; toggling clears the value
- [ ] Inline mode shows character count + markdown hint
- [ ] File mode shows the `file://` chip + composite input

---

### S16 — Save-time marketplace-prompt modal

**Claim:** Spec B8 — the keep-linked vs detach choice at save time.

**Oracle:** vitest on the choice-to-save-params helper (`buildSaveParams`).

**Stress fixture:** vitest cases:
1. `buildSaveParams("keep-linked", from, draft)` → `{ fromName: from, draft, detach: false }`
2. `buildSaveParams("detach", from, draft)` → `{ fromName: from, draft, detach: true }`
3. The modal only opens when `originalRow.lineage !== null`; for user-authored agents, save proceeds directly with `detach: false`

**Loop budget:** N/A.

**Files:**
- `crates/kiro-control-center/src/lib/components/editor/MarketplaceSavePromptModal.svelte` (new, ~80 LOC)
- `crates/kiro-control-center/src/lib/agents/save-params.ts` + `.test.ts` (new, ~15 LOC + ~30 LOC tests)

**Code (advisory):**
```ts
export type SaveChoice = "keep-linked" | "detach";

export function buildSaveParams(choice: SaveChoice, fromName: string, draftJson: string) {
  return { fromName, draftJson, detach: choice === "detach" };
}
```

**Verification:**
- [ ] All 3 vitest cases pass
- [ ] Modal renders only when the row has lineage
- [ ] Two clearly-labeled buttons; choice routes through `buildSaveParams`

---

### S17 — Playwright e2e CRUD round-trip

**Claim:** Spec success criterion S1 — "1 Playwright e2e test passes."

**Oracle:** Filesystem inspection inside the test (Playwright can read
disk before and after each user action).

**Stress fixture:** The test itself is the fixture. Sequence:
1. Open the app, point at a fresh `tempfile::tempdir()`-style project
2. Navigate to Workflows > Agents
3. Empty state visible; click "+ Create Agent"
4. Fill Identity (name = `e2e-test`, description, model), submit; assert `<project>/.kiro/agents/e2e-test.json` exists on disk
5. Return to list; click Edit on the new row; change description; save; assert file content updated
6. Click Duplicate; assert `e2e-test-copy.json` exists on disk
7. Click Delete on the copy; confirm; assert file absent

Adversarial assertions interleaved: between (4) and (5), assert no
`installed-agents.json` entry was created (user-authored, no lineage).

**Loop budget:** N/A (single test path).

**Wall budget:** The test itself should run ≤ 30 s on CI (Playwright app
startup dominates). No tighter bound needed.

**Files:**
- `crates/kiro-control-center/tests/e2e/agents.spec.ts` (new, ~120 LOC)

**Code (advisory):** Mirror the shape of the existing
`tests/e2e/app.spec.ts`, including the `FIXTURE_MARKETPLACE_PATH`
gating pattern (gate on a fresh project tmpdir, `test.skip` if the
env var convention isn't set up for agents-view fixtures yet).

**Verification:**
- [ ] Test passes against a clean tmpdir project
- [ ] Each filesystem assertion runs after the corresponding UI action
- [ ] Test handles the marketplace-agent case in a separate `test.describe` block (assert keep-linked vs detach pathways)

---

## Plan Self-Review

Per skill rule 7, five lists with no gaps:

### 1. Every loop in the plan

| Slice | Loop | Complexity | Production scale | Total ops | Within budget? |
|---|---|---|---|---|---|
| S3 | Read each `*.json` in `agents_dir` | O(F) | F ≤ 200 | ~800 ops | ✓ (≪ 10^6) |
| S7 | Find next free `-copy[-N]` | O(N) | N ≤ 10 | ≤ 10 syscalls | ✓ (≪ 10^3 syscalls) |
| S10 | `filterAgentRows`: rows × fields | O(R × L) | R ≤ 200, L ≤ 500 chars | ~100k char-cmp per keystroke | ✓ |
| S12 | `{#each rows}` in markup | O(R) | R ≤ 200 | trivial | ✓ |
| S17 | (Playwright sequence, not a loop) | — | — | — | — |

No loop is `O(?)` or unbounded.

### 2. Every fixture — adversarial, not happy-path

| Slice | Bug class the fixture would catch |
|---|---|
| S2 | Truncating `usize` counts to `u32` would lose precision; Unicode in `name` would break a non-`String` type |
| S3 | A `parse_native`-based implementation would crash on `no-name.json`; orphan tracking entries should be excluded |
| S4 | Silent overwrite on name collision; missing regex validation |
| S5 | Write-before-collision-check would corrupt the target on rename collision (case 3); unconditional tracking drop would falsify case 5 |
| S6 | `KiroProject::remove_agent`-only delegation falsifies case 1 (untracked); brittle on missing files falsifies case 3 |
| S7 | Naive "always use `-copy`" overwrites; copying tracking metadata into the duplicate |
| S8 | A future contributor adds `let svc = make_service()?` to a wrapper; grep catches it |
| S9 | Non-deterministic specta export (HashMap field order); second-run diff catches it |
| S10 | Null-unsafe `description.includes`; name-only search misses model field |
| S11 | (Type-check IS the fixture) |
| S14 | Naive `length > 0` misses regex constraints |
| S15 | Mode detection that doesn't preserve case-sensitivity |
| S16 | Modal that opens for user-authored agents (wrong condition) |
| S17 | CRUD operations that don't actually touch disk; tracking accidentally created for user-authored |

S1 (vendor schema), S13 (editor shell) are exempt — pure data / pure
markup. S12 has visual-only fixture (acceptable per spec criterion S7).

### 3. Every doc-comment precondition

| Slice | Precondition | Classification | Enforcement |
|---|---|---|---|
| S3 | "Creates agents_dir if absent" | Load-bearing | Runtime `fs::create_dir_all` |
| S3 | "Untyped JSON, never parse_native" | Load-bearing (design C2) | Tested empirically by stress fixture's no-name file |
| S4 | "Rejects collision *before* writing" | Load-bearing | Runtime `target.exists()` check |
| S5 | All 4 ordering steps | Load-bearing | Runtime checks for each (collision, write, unlink, tracking) |
| S5 | "from_name is the editable agent's stem" | Sanity hint | `debug_assert!(from_path.exists())` at function entry; release builds tolerate (caller is the UI which won't be confused) |
| S6 | "Idempotent on NotFound" | Load-bearing | Match arm explicitly handles `ErrorKind::NotFound` |
| S7 | "Duplicate is always user-authored" | Load-bearing | Code path simply does not call into `installed.agents.insert` — enforcement is by ABSENCE rather than by check |
| S7 | "NameSpaceExhausted sanity cap" | Sanity hint | Loop bound of 10,000; documented; would only fire under deliberate abuse |
| S14 | "JS regex matches Rust AgentName regex" | Load-bearing | Parity test (or, if regex syntax differs irreconcilably, documented as maintenance burden in helper file) |

### 4. Every write target — data vs diagnostic

| Slice | Target | Class | Correct? |
|---|---|---|---|
| S3 | `Result<Vec<UserAgentRow>>` return value | data | ✓ |
| S3 | `tracing::warn!` for unparseable / orphan | diagnostic (stderr) | ✓ |
| S4 | `Result<()>` return | data | ✓ |
| S5 | `tracing::warn!` for post-rename unlink failure | diagnostic | ✓ |
| S6 | `Result<()>` return | data | ✓ |
| S7 | `Result<String>` (new name) return | data | ✓ |
| S8 | Tauri command return values | data (serialized over IPC) | ✓ |
| S9 | `bindings.ts` regen | generated source artifact | ✓ (committed; not stdout/stderr) |

No unexamined `println!` / `eprintln!` introduced.

### 5. Every tracker reference — resolves to existing issue

| Reference | Issue | Covers? |
|---|---|---|
| "future CI gate (kiro-6g6r)" in S8 | kiro-6g6r | ✓ — issue is precisely about the CI gate |
| "deferred to follow-up issue (kiro-fd40)" implicit in S3's spec D13 logging | kiro-fd40 | ✓ — broken-row UX follow-up |
| "slice 2 implementer" in S13 | kiro-vgnw | ✓ — slice 2 Tools section |
| Slice 2-6 disabled rail entries in S13 | kiro-vgnw, kiro-gwo4, kiro-3ll2, kiro-ttew, kiro-zqci | ✓ — each issue covers one slice |

All seven follow-up issues verified existing in rivets as of 2026-05-22.

---

## Hard gate (skill requirement) — status

- [x] Every slice has all mandatory fields filled (Claim / Oracle / Stress fixture / Loop budget / Files / Verification)
- [x] Every loop has a complexity statement (table in self-review item 1)
- [x] Every slice has a stress fixture (or is exempt as pure-schema / pure-markup)
- [x] Plan's claim coverage matches design's claim list (C1 → S3, C2 → S3, C3 → S4, C4 → S5, C5 → S6, C6 → S7, C7 → S8, C8 → S1 + S9)
- [x] Every tracker reference resolves to an existing issue (table in self-review item 5)

Plan ready for hand-off to `checkpointed-build`.

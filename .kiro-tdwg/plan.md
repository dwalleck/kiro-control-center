# Budgeted plan — Surface remediation_hint (kiro-tdwg)

Design claims: `design.md#claims`. Core helper `format_error_for_surface` + C1 test already committed in `f2b72cb`. Remaining claims: C2 (Ui surface test), C3 (non-remote pass-through test), C4 (CommandError.remediation populated), C5 (CommandError.remediation None for others), C6 (bindings determinism), C7 (existing test regression), C8 (CLI smoke).

## Slice 1: Complete core tests (C2, C3)

**Claim:** `format_error_for_surface` with `Surface::Ui` yields UI-specific hint; non-remote errors pass through unchanged.
**Oracle:** Assertion on the function output (same as C1 pattern).
**Stress fixture:** Parameterized non-remote test covering `NotFound`, `InvalidManifest`, `ManifestNotFound`, `SymlinkRefused`, `DirectoryUnreadable`, `NoSkills`, `ManifestReadFailed`.
**Smallest change:** Add two `#[test]` functions after C1 in `error.rs`.
**Loop budget:** N/A (no loops introduced).
**Wall budget:** N/A (test-only).
**Files:** `crates/kiro-market-core/src/error.rs`
**Verification:**
- [ ] `cargo test -p kiro-market-core --lib format_error_for_surface` passes all three tests

---

## Slice 2: Tauri CommandError.remediation field (C4, C5)

**Claim:** `CommandError` carries `remediation: Option<String>` populated from `remediation_hint(Surface::Ui)` in `From<CoreError>`.
**Oracle:** Assert `CommandError.remediation` is `Some(...)` for `RemoteSourceNotLocal` (containing "detail page", NOT "kiro-market install"), and `None` for `NotFound`.
**Stress fixture:** `RemoteSourceNotLocal` with both `GitHub` and `GitUrl` sources — both must produce `Some(remediation)`.
**Smallest change:**
1. Add `pub remediation: Option<String>` to `CommandError` struct
2. Populate in `From<CoreError>`: extract via `remediation_hint(Surface::Ui)` before consuming `err`
3. Update `new()` constructor default
4. Add two unit tests
**Loop budget:** N/A.
**Wall budget:** N/A.
**Files:** `crates/kiro-control-center/src-tauri/src/error.rs`
**Verification:**
- [ ] `cargo test -p kiro-control-center --lib command_error` passes
- [ ] Existing tests still pass (`cargo test -p kiro-control-center --lib`)

---

## Slice 3: Bindings regeneration + CLI interception (C6, C8)

**Claim:** Generated TS bindings carry `remediation: string | null` on `CommandError`; CLI main.rs intercepts `RemoteSourceNotLocal` and appends the CLI hint.
**Oracle (bindings):** Run `generate_types` test twice, diff output, assert zero diff. Parse TS AST, assert `CommandError` has `remediation: string | null`.
**Oracle (CLI):** Build CLI, run against remote-only marketplace, capture stderr, assert "kiro-market install" in output.
**Stress fixture (bindings):** Two consecutive generator runs; any non-deterministic output fails.
**Stress fixture (CLI):** Error chain without `RemoteSourceNotLocal` (e.g., `NotFound`) — must NOT contain CLI remediation text.
**Smallest change:**
1. Run binding generator test
2. Modify `crates/kiro-market/src/main.rs`: wrap body in `try_main() -> Result<()>`, intercept error, downcast to `Error::Plugin(PluginError::RemoteSourceNotLocal)`, append `remediation_hint(Surface::Cli)`.
**Loop budget:** N/A.
**Wall budget:** N/A.
**Files:** `crates/kiro-control-center/src/lib/bindings.ts`, `crates/kiro-market/src/main.rs`
**Verification:**
- [ ] `cargo test -p kiro-control-center --lib -- --ignored generate_types` passes
- [ ] Bindings diff is deterministic (re-run, zero diff)
- [ ] `cargo build -p kiro-market`
- [ ] CLI smoke: `kiro-market list` against remote marketplace shows CLI hint in stderr
- [ ] Full gate sweep (nextest, clippy, fmt, doctests, frontend)

---

## Plan self-review

### 1. Every loop
None introduced. All changes are error-formatting additions and field additions.

### 2. Every fixture

| Slice | Fixture | Bug class |
|---|---|---|
| S1 | 7 non-remote variants parameterized | Pass-through regression — if format_error_for_surface accidentally matches on more than RemoteSourceNotLocal, the test catches it |
| S2 | GitHub + GitUrl source variants | Source-type regression — if the hint extraction only works for one source type |
| S3 | Two-run binding diff | Non-deterministic code generation |
| S3 | NotFound error in CLI | Over-matching — if the CLI interceptor appends hint to non-remote errors |

### 3. Every doc-comment precondition
`format_error_for_surface` doc-comment says "Errors that are not `Error::Plugin(PluginError::RemoteSourceNotLocal)` pass through unchanged" — enforced by C3 parameterized test (runtime verification, not compile-time).

### 4. Every write target
- `format_error_for_surface` → returns `String` (data, caller routes to stderr or UI)
- CLI `eprintln!` → stderr (diagnostic)

### 5. Every tracker reference
None deferred. The `remediation_hint` method and `Surface` enum are not refactored. No new issues filed.

### Hard-gate checklist

- [x] Every slice has claim, oracle, stress fixture, smallest change, loop/wall budget, files, verification
- [x] No loop introduced
- [x] Every fixture designed against a specific bug class
- [x] Claim coverage: S1 = C2+C3, S2 = C4+C5, S3 = C6+C8. C1 already passed, C7 = existing test suite
- [x] No tracker deferrals
- [x] Each slice touches 1-2 files

Ready for checkpointed build.

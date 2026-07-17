# Prove-it findings — kiro-tdwg

## Smallest question

Is `PluginError::remediation_hint` reachable from any production surface?

## Probe

`.kiro-tdwg/probe.py` walks every `.rs` file in `crates/` with Python's `pathlib` and checks for `.remediation_hint(` call sites, classifying each as test or production by `#[cfg(test)]` context. It also checks whether CLI source files or the Tauri error module reference `remediation_hint`. Result:

```json
{
  "production_call_sites": 0,
  "test_call_sites": 4,
  "cli_surface_calls_remediation_hint": false,
  "tauri_surface_calls_remediation_hint": false,
  "remediation_is_dead_code": true
}
```

## Oracle

`.kiro-tdwg/oracle.mjs` computes the same five fields through an independent mechanism — `grep -rn` pipelines plus per-file `#[cfg(test)]` context detection — rather than Python's file-tree walk. Result:

```json
{
  "production_call_sites": 0,
  "test_call_sites": 4,
  "cli_files_with_remediation": 0,
  "tauri_error_remediation_mentions": 0,
  "is_dead_code": true
}
```

Probe and oracle **AGREE** on all five fields.

## Additional verification

The 4 test-only call sites are inside `crates/kiro-market-core/src/error.rs` under `#[cfg(test)] mod tests` — the unit tests for `remediation_hint` itself. No production file in `crates/kiro-market/src/` or `crates/kiro-control-center/src-tauri/src/error.rs` references the method.

## Surfaces that should carry remediation

| Surface | Error path | Current behavior | Remediation source |
|---|---|---|---|
| CLI | `main.rs` → anyhow `Result` → Display chain | Error message only, no next step | `remediation_hint(Surface::Cli)` |
| Tauri | `error.rs` → `CommandError { message, error_type }` | Error message + type only, no remediation field | `remediation_hint(Surface::Ui)` |

## What I learned

The method and its tests are correct. The `Surface` enum and surface-specific hint strings exist and are tested. The gap is purely wiring: no error projection path in either CLI or Tauri calls `remediation_hint` before surfacing the error to the user.

The CLI path is simpler — `main.rs` returns `anyhow::Result<()>`, and anyhow prints the error chain's Display. The cheap fix is to wrap the core error in an anyhow context that appends the CLI remediation hint. The Tauri path needs a new optional field on `CommandError`.

## Design constraints established

1. `remediation_hint` and `Surface` are correct as designed; do not refactor them.
2. CLI surface: append remediation text to the error chain before returning.
3. Tauri surface: add optional `remediation: Option<String>` to `CommandError`.
4. Both surfaces must call `remediation_hint` with their respective `Surface` variant.
5. The existing unit tests for `remediation_hint` are the regression fence.

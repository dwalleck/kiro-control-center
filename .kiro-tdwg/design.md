# Falsifiable design — Surface remediation_hint (kiro-tdwg)

## Purpose

Wire `PluginError::remediation_hint` to the two surfaces that display errors to
users: the CLI (`Surface::Cli`) and the Tauri desktop app (`Surface::Ui`). The
method, `Surface` enum, and per-surface hint strings already exist and are tested;
the gap is purely wiring — no production code calls `remediation_hint` today.

## Architecture

One new core-level helper, two surface-level wiring changes:

```
core: format_error_for_surface(&CoreError, Surface) -> String
 │
 ├─ CLI: each command calls format_error_for_surface before returning
 │        (or a wrapper interposes on the anyhow chain)
 │
 └─ Tauri: From<CoreError> populates new CommandError.remediation field
           via format_error_for_surface or direct remediation_hint call
```

### `format_error_for_surface`

New public function in `kiro_market_core::error`:

```rust
pub fn format_error_for_surface(err: &CoreError, surface: Surface) -> String {
    let chain = error_full_chain(err);
    if let CoreError::Plugin(e) = err {
        if let Some(hint) = e.remediation_hint(surface) {
            return format!("{chain}\n\n{hint}");
        }
    }
    chain
}
```

CLI commands that currently return `anyhow::Error` wrapping a `CoreError` call
this before constructing the anyhow error, producing a single message that
includes the remediation. Commands that don't touch `resolve_local_plugin_dir`
(list, info, search, install --skill) need no change — their error paths can't
produce `RemoteSourceNotLocal`.

Tauri's `From<CoreError> for CommandError` calls `remediation_hint(Surface::Ui)`
directly and stores it as `remediation: Option<String>`.

## Input shapes (step 2)

| Shape | Example | Covered by claim |
|---|---|---|
| `RemoteSourceNotLocal` + `Surface::Cli` | CLI list of remote-only marketplace plugin | C1 |
| `RemoteSourceNotLocal` + `Surface::Ui` | Tauri browse catalog with remote source | C2 |
| `PluginError::NotFound` + any Surface | Missing plugin | C3 |
| `PluginError::InvalidManifest` + any Surface | Malformed plugin.json | C3 |
| `CoreError::Agent(_)` + any Surface | Agent install failure | C3 |
| `CoreError::Io(_)` + any Surface | Disk error | C3 |
| `Surface::Cli` × `Surface::Ui` distinction | Both surfaces, same error | C4 |

## Negative space

1. **Not adding remediation for any error variant other than `RemoteSourceNotLocal`.** The method returns `None` for all other variants; the wiring respects that.
2. **Not changing the Display impl of `RemoteSourceNotLocal`.** The existing test fence `remote_source_not_local_display_has_no_cli_hint` proves Display contains no remediation verbs. Remediation is appended by the surface, not embedded in Display.
3. **Not wiring remediation into `SkippedReason::RemoteSourceNotLocal`.** That browse-catalog variant already carries `source: StructuredSource` for the frontend to build its own "clone" button; it's the catalog layer's responsibility, not the error layer's.
4. **Not changing `error_full_chain`'s signature.** It remains `&dyn Error -> String` so existing test assertions continue to pass.
5. **Not surfacing remediation in `PluginUpdateFailure` or `InstallWarning`.** Those are informational paths, not error paths; the existing `mcp_servers_require_opt_in` warning pattern is the right model there.

## Claims

| # | Claim | Falsifier | Oracle | Cost | Status |
|---|---|---|---|---|---|
| C1 | `format_error_for_surface` with `RemoteSourceNotLocal` + `Surface::Cli` returns a string containing both the Display error and the CLI-specific hint ("\`kiro-market install\`") | Construct the error, call the function, assert the output contains "clone" | Unit test on the function | 2m | pending |
| C2 | `format_error_for_surface` with `RemoteSourceNotLocal` + `Surface::Ui` returns a string containing the UI-specific hint ("detail page") but NOT the CLI-specific hint ("\`kiro-market install\`") | Same error, `Surface::Ui`, assert contains "detail page" and does NOT contain "kiro-market install" | Unit test | 2m | pending |
| C3 | `format_error_for_surface` with any non-`RemoteSourceNotLocal` error returns `error_full_chain(err)` unchanged (no hint appended) | Parameterized test with `NotFound`, `InvalidManifest`, `Io`, `Agent::AlreadyInstalled` | Unit test | 5m | pending |
| C4 | Tauri `CommandError` carries `remediation: Option<String>` populated from `remediation_hint(Surface::Ui)` | Inject `RemoteSourceNotLocal` into `From<CoreError>`, assert `remediation` is `Some(...)` and does NOT contain "kiro-market install" | Unit test on `From<CoreError>` impl | 5m | pending |
| C5 | Tauri `CommandError` for non-remediation errors has `remediation: None` | Inject `NotFound` into `From<CoreError>`, assert `remediation` is `None` | Unit test | 2m | pending |
| C6 | Generated TypeScript bindings carry `remediation: string | null` on the `CommandError` type | Run the binding generator test twice; diff must be empty; TS AST must find the field | Existing `generate_types` test + probe/oracle extension | 5m | pending |
| C7 | Existing tests for `remediation_hint` still pass (no regression) | `cargo test -p kiro-market-core --lib error` | CI | 1m | pending |
| C8 | CLI commands that can produce `RemoteSourceNotLocal` surface the remediation in their output | Build CLI, run `kiro-market list` against a remote-only marketplace, capture stderr, assert it contains the CLI hint | Manual CLI invocation (no CI harness for CLI output) | 10m | pending |

### Cheapest falsifier

C1 (2 min): `format_error_for_surface` unit test. Run before design approval.

### Regression fences

| Claim | Fence |
|---|---|
| C1 | `format_error_for_surface_cli_includes_install_hint` unit test in `error.rs` |
| C2 | `format_error_for_surface_ui_excludes_cli_verb` unit test in `error.rs` |
| C3 | `format_error_for_surface_non_remote_unchanged` parameterized test in `error.rs` |
| C4 | `commanderror_remote_source_not_local_carries_remediation` in `error.rs` (tauri) |
| C5 | `commanderror_non_remote_remediation_is_none` in `error.rs` (tauri) |
| C6 | Existing `generate_types` ignored test |
| C7 | Existing `remediation_hint_*` tests |
| C8 | Manual CLI verification (approval required for manual fence) |

## Decisions requiring approval

1. **Core helper vs per-surface wiring.** Recommend `format_error_for_surface` in core so both surfaces share the composition logic. Alternative: each surface independently calls `remediation_hint` and composes the message — duplicates the "append \n\n + hint" logic.
2. **Manual fence for C8.** The repo has no CLI output test harness. C8 is a manual verification fence — approve or replace with a different approach.

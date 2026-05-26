# Review Notes

<!-- tags: review, gaps, consistency -->

## Consistency Check

No cross-document contradictions found. The following terms are used consistently across all files:

- Plugin reference format: `plugin@marketplace` (split on first `@`) — consistent in `interfaces.md`, `components.md`, `codebase_info.md`
- Feature flag names (`cli`, `specta`, `test-support`) — consistent in `architecture.md`, `codebase_info.md`, `dependencies.md`
- Default scan paths (`./skills/`, `./agents/`, `./steering/`) — consistent in `codebase_info.md` and `workflows.md`
- BLAKE3 hash format (`blake3:<hex>`) — consistent in `data_models.md` and `dependencies.md`
- `GixCliBackend` as the production `GitBackend` implementation — consistent in `interfaces.md` and `components.md`

## Completeness Check

### Gaps Identified

**1. Frontend state management detail**
`components.md` lists the Svelte stores but does not describe the `$state` module pattern in depth. The Svelte 5 runes pattern (mutations via deep state proxy on a `const` object) is mentioned in `architecture.md` but not elaborated. Agents working on frontend state should consult the Svelte MCP server tools for authoritative Svelte 5 documentation.

**2. xtask plan-lint allowlist format**
`architecture.md` mentions the allowlist mechanism for `plan-lint` gates but does not document the allowlist file format or location. This is an internal tool detail; consult `xtask/src/plan_lint.rs` directly if needed.

**3. `.agents-view/` directory**
The `.agents-view/` directory (containing `spec.md`, `design-slice-1.md`, probe scripts) is not documented. It appears to be a work-in-progress feature design area (plugin catalog view). It is not part of the shipped codebase and is excluded from documentation intentionally.

**4. `Kiro Control Center Design System` directories**
Two design handoff directories exist at the repo root (`Kiro Control Center Design System/` and `Kiro Control Center Design System -plugins/`). These contain Figma/design artifacts and are not part of the Rust/Svelte codebase. Excluded from documentation intentionally.

**5. `.prove-it/` directory**
Contains probe/oracle artifacts from the `prove-it-prototype` skill workflow. Not part of the shipped codebase. Excluded intentionally.

**6. `docs/plans/` and `docs/reviews/`**
Extensive design documents and review notes exist under `docs/`. These are historical artifacts. Not documented in the summary files; agents needing design rationale should read them directly.

**7. Native companion files**
The `native_companions` tracking structure in `installed-agents.json` is documented in `data_models.md` but the install workflow for native companions (multi-file Kiro-native plugins) is not traced in `workflows.md`. This is a complex sub-workflow; consult `project.rs::install_native_companions` and `service/mod.rs::install_native_companions_for_plugin` directly.

**8. `resolve_local_path_restricted` in CacheDir**
This security function (restricts path resolution to a set of allowed roots, rejects symlink escapes) is mentioned in `components.md` but not detailed in `interfaces.md` or `architecture.md`. It is used for local marketplace path validation.

### Language Coverage Limitations

- The `.github/scripts/post-review-comments.py` Python script is documented at the component level in `components.md` (implicitly, as a GitHub Actions script) but its internal structure is not covered. It handles PR review comment posting and is not part of the core application.
- The probe scripts in `.agents-view/probe/` (Python + PowerShell) are not documented.

## Recommendations

1. If adding a new Tauri command, update `interfaces.md` and regenerate `bindings.ts`.
2. If adding a new tracking field to any `.kiro/*.json` file, update `data_models.md`.
3. If the native companion install workflow becomes a common change area, add a workflow trace to `workflows.md`.
4. The `Custom Instructions` section in `AGENTS.md` is the right place for operational gotchas discovered during development — add them there rather than to these summary files.

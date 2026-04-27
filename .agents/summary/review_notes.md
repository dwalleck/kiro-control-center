# Review Notes

## Consistency Check

All documentation files were cross-referenced for consistency. Findings:

### ✅ Consistent

- **Crate names and paths** — consistent across all files (kiro-market-core, kiro-market, kiro-control-center)
- **Feature flags** — `cli`, `specta`, `test-support` documented identically in codebase_info.md and architecture.md
- **Security invariants** — path validation, MCP opt-in, TLS enforcement documented consistently in architecture.md and data_models.md
- **Dependency versions** — match Cargo.toml workspace definitions
- **IPC command list** — matches actual `collect_commands!` registration in lib.rs (17 commands)
- **CLI command structure** — matches clap derive definitions in cli.rs
- **On-disk file formats** — JSON schemas in data_models.md match actual serde derives in source

### ⚠️ Minor Notes

1. **Workspace lint level**: AGENTS.md says `unsafe_code = "deny"` but Cargo.toml actually uses `unsafe_code = "forbid"` (stricter). The documentation should use "forbid" consistently.
   - **Affected files**: Existing AGENTS.md (will be corrected in consolidation step)

---

## Completeness Check

### Well-Documented Areas

- ✅ Core service layer (MarketplaceService) — thoroughly covered
- ✅ Agent parsing pipeline (all dialects) — complete
- ✅ Installation workflows — detailed sequence diagrams
- ✅ Error hierarchy — full type tree documented
- ✅ Security model — all invariants captured
- ✅ CI pipeline — all jobs listed
- ✅ Dependencies — rationale provided for non-obvious choices

### Gaps Identified

| Area | Gap | Severity | Recommendation |
|------|-----|----------|----------------|
| E2E tests | Playwright test structure not documented | Low | Add test patterns section to components.md if test coverage grows |
| `.claude/` directory | Claude Code skills, commands, and agents in this repo not documented | Low | These are development aids, not part of the product. Document if they become part of the workflow |
| `.kiro/` project agents | The 7 installed Kiro agents (code-reviewer, etc.) are not documented as project tooling | Low | These are operational tooling; mention in AGENTS.md Custom Instructions if relevant |
| Hash change detection | The BLAKE3 source/installed hash comparison workflow could be more explicit | Low | Add a subsection to workflows.md explaining when hashes are compared |
| Svelte 5 state patterns | The `$state` module pattern in `project.svelte.ts` is mentioned but not explained | Low | Add a brief pattern explanation to components.md frontend section |
| Release workflow | `.github/workflows/release.yml` exists but release process not documented | Medium | Document release tagging and artifact generation |
| `kiro-review.yml` workflow | Review automation workflow with `post-review-comments.py` not documented | Medium | Document the automated PR review pipeline |

### Language Support Limitations

- **Python scripts** (`.github/scripts/`): Analyzed structurally but not deeply. These are CI support scripts for posting review comments, not core product code.
- **Svelte components**: Analyzed by file structure and naming. Deep component logic (reactive state, event handling) would require reading each `.svelte` file.

---

## Recommendations

1. **Correct `unsafe_code` level** in AGENTS.md consolidation: use "forbid" (matches Cargo.toml)
2. **Document release process** if the project reaches regular release cadence
3. **Document the review automation pipeline** (`kiro-review.yml` + `post-review-comments.py`) as it's a significant piece of project infrastructure
4. **Consider adding a testing.md** if test patterns become complex enough to warrant separate documentation

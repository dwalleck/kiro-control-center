# Review Notes

## Consistency Check

### ✅ Passed

- **Terminology**: All documents consistently use "marketplace", "plugin", "skill", "agent" hierarchy
- **Type names**: Types referenced across documents match actual Rust source (verified against `error.rs`, `service.rs`, `project.rs`, `cache.rs`)
- **Architecture claims**: The shared-core pattern described in architecture.md is confirmed by Cargo.toml dependency declarations
- **CLI commands**: Commands listed in interfaces.md match the `cli.rs` clap definitions exactly
- **Tauri commands**: IPC functions listed match `lib.rs` `collect_commands!` macro invocation
- **File paths**: Cache structure, project structure, and config paths are consistent across all documents
- **Security model**: Path traversal prevention, MCP gating, and TLS enforcement are described consistently in architecture.md, components.md, and workflows.md

### ⚠️ Minor Notes

- **GitBackend trait**: Documented as a trait in interfaces.md but the actual implementation uses a generic parameter `impl GitBackend` rather than trait objects. This is accurate but could be clearer about the compile-time dispatch.
- **Error type `Io` and `Json`**: The data_models.md error diagram shows `Io(io::Error)` and `Json(serde_json::Error)` as variants. These are `From` conversions rather than named variants in the actual enum — the diagram is a simplification.

## Completeness Check

### Well-Documented Areas

- Core library architecture and module responsibilities
- CLI command structure and arguments
- Tauri IPC interface
- Security model and validation
- Git operations and dual-backend strategy
- CI pipeline structure
- File format specifications
- Dependency rationale

### Gaps Identified

| Area | Gap | Severity | Recommendation |
|------|-----|----------|----------------|
| Frontend components | Individual Svelte component props/events not documented | Low | Components are straightforward UI; bindings.ts provides the contract |
| E2E tests | Playwright test coverage and patterns not described | Low | Only one test file exists; document when test suite grows |
| Release workflow | `release.yml` not analyzed | Medium | Document release process (likely tag-triggered builds) |
| `claude.yml` / `claude-code-review.yml` | Claude Code automation workflows not documented | Low | These are likely PR review automation; document if relevant to contributors |
| Agent tool mapping | Specific Claude→Kiro and Copilot→Kiro tool mappings not enumerated | Medium | Would help contributors understand what tools are supported |
| Kiro settings registry | Specific setting keys, categories, and defaults not listed | Medium | Would help frontend contributors understand available settings |
| Error recovery | How the system recovers from partial failures (leftover staging, interrupted clones) | Low | Covered at high level in workflows; implementation details in code comments |
| Multi-file skill merging | The process of merging companion `.md` references into a single SKILL.md | Medium | Mentioned in README but not detailed in workflows |

### Language Support Limitations

- **Svelte 5**: Runes-mode `$state`/`$derived` syntax is relatively new; tooling support for deep analysis is limited
- **Tauri-specta**: RC-version library; API may shift before stable release

## Recommendations

1. **Document release workflow** — Analyze `release.yml` and add a release process section to workflows.md
2. **Enumerate tool mappings** — Add a table of Claude/Copilot tool names → Kiro equivalents in interfaces.md
3. **List settings registry** — Document available setting keys with their types and defaults
4. **Detail skill merging** — Add a workflow diagram showing how multi-file skills are consolidated
5. **Keep docs updated** — Re-run documentation generation after significant architectural changes

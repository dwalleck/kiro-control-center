# Related rivets issues (prior art scan)

Searched `rivets list --status open` for agent-related work on 2026-05-18.
None of the open issues are this feature; all are follow-ups to the existing
**marketplace-install** path for agents. They are relevant background, not duplicates.

## Active agent-touching issues

- **kiro-zx73** (P2, ★ ready) — *Per-item steering/agent install/remove Tauri commands for Customize drawer.* Adds `install_agent` / `remove_agent` per-name commands. **Relevance:** this user-authoring feature needs **different** create/save/delete commands (no marketplace/plugin lineage), but the `_impl(svc, ...)` shape, validation newtypes, and bindings-fence discipline established by kiro-zx73 are the pattern to mirror.

- **kiro-0pbb** (P2, ●) — Dedup steering+agent names within a plugin (catalog reads). Marketplace-side.

- **kiro-2cu2** (P2) — MCP opt-in UI surface missing from BrowseTab install paths. **Relevance:** the editor's MCP Servers section needs to decide whether saving an agent with MCP servers in its `mcpServers` field requires an opt-in dialog (parallel question to install-time opt-in).

- **kiro-19zq** (P3, ◆) — FailedAgent wire-format follow-ups F1-F5. Touches `FailedAgent` discriminator; if user-author save can fail in structured ways (e.g. name collision with installed agent), it may share or parallel this enum.

- **kiro-jmgb** (P3) — SkippedAgentReason typed enum. Marketplace catalog side; relevant precedent for typed-error discipline on the new save path.

- **kiro-bury** (P3, ●) — FailedAgent::InvalidName variant for CLI-supplied malformed agent names. **Relevance:** the editor's name validation (`^[a-z0-9][a-z0-9-]*$` per design doc) needs to align with `AgentName` newtype constraints in `validation.rs`.

- **kiro-deph** (P3) — RequestedButNotFound name-type discipline. Cross-cutting agent-naming concern.

- **kiro-wks5** (P3) — Surface native-agent description in catalog (parser widening).

- **kiro-i3ll** (P4) — `#[non_exhaustive]` on `PluginCatalogEntry`/`SteeringItemInfo`/`AgentItemInfo`.

- **kiro-afl7** (P4) — FailedAgent variant naming asymmetry.

## Bug-class issues touching shared paths

- **kiro-ti4x** / **kiro-ql6l** (P2/P3) — `applyDrawerDiff` parallel install batches overwrite `installError`. Frontend pattern not specific to agents but the BrowseTab's `installErrors: string[]` migration is the parallel-error-handling precedent.

## Conclusion

No issue currently exists for **user-authored agents** (the surface this feature creates). Interrogation should produce a spec that, when handed off, becomes a new rivets issue (or sequence of issues, if decomposed into slices).

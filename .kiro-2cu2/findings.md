# Prove-it findings — kiro-2cu2

## Smallest question

Can BrowseTab know before an install that a catalog agent declares MCP servers, and what consent value do its two current install paths submit?

## Probe

`.kiro-2cu2/probe.py` independently scans the generated TypeScript wire contract and BrowseTab source. It reported:

```json
{
  "agent_catalog_fields": [
    "name",
    "description",
    "plugin",
    "marketplace",
    "installed",
    "dialect"
  ],
  "catalog_has_preinstall_mcp_signal": false,
  "drawer_accept_mcp": "false",
  "post_install_warning_available": true,
  "whole_plugin_accept_mcp": "false"
}
```

## Oracle

`.kiro-2cu2/oracle.mjs` computes the same answer through independent parsers: Svelte's compiler AST for BrowseTab and TypeScript's AST for `bindings.ts`. Probe and oracle output compared equal: **AGREE** on all five fields.

The backend behavior was checked separately against the real service code:

```text
cargo test -p kiro-market-core --lib install_plugin_agents_skips_mcp_agents_without_opt_in
cargo test -p kiro-market-core --lib install_plugin_agents_installs_mcp_agents_when_opted_in
```

Both tests passed. `false` skips the agent, emits `McpServersRequireOptIn`, and writes no agent JSON; `true` installs it with the normalized `mcpServers` block.

## What I learned

The catalog cannot currently drive a pre-install conditional affordance: `AgentItemInfo` has six fields and none carries MCP presence or transport metadata. The only MCP signal reaches the frontend after an attempted install through `InstallWarning::McpServersRequireOptIn`. Therefore the ticket's proposed “when catalog refresh reports” behavior requires either a catalog wire-format addition or a deliberate post-warning retry UX; merely replacing the two `false` literals is insufficient.

## Design constraints established

1. Default remains fail-closed (`acceptMcp = false`) until an explicit per-plugin action.
2. Both BrowseTab paths must consume the same consent decision; separate uncoordinated booleans would drift.
3. The backend gate already works and needs no semantic change unless the design chooses pre-install catalog metadata.
4. Whole-plugin and drawer warning renderers already preserve `mcp_servers_require_opt_in`; the feature must not suppress that fallback signal.
5. InstalledTab update remains out of scope and is tracked as **kiro-yr2f**.

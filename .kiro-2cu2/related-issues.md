# Related issues — kiro-2cu2

Tracker search terms: `acceptMcp`, `accept_mcp`, `MCP opt`, `mcp_servers_require_opt_in`, `BrowseTab`, and `InstalledTab`.

- **kiro-2cu2** — primary issue. BrowseTab passes `false` on both drawer and whole-plugin install paths, leaving no consent path.
- **kiro-zx73** — shipped substrate. Per-item steering/agent commands and interactive drawer are already on `main`; the issue was stale-open and was closed after targeted verification.
- **kiro-gwo4** — related terminology only. Adds MCP-server authoring UI to the separate Agents editor; it does not control marketplace-install consent.
- **kiro-yr2f** — discovered and filed during this probe. InstalledTab's update path also hardcodes `acceptMcp: false`; intentionally deferred because kiro-2cu2 is scoped to BrowseTab install/customize behavior.

No duplicate issue covers BrowseTab's missing MCP consent affordance.

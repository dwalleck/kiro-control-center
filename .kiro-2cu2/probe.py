#!/usr/bin/env python3
"""Measure what MCP opt-in information BrowseTab has before install."""

from __future__ import annotations

import json
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
BROWSE = ROOT / "crates/kiro-control-center/src/lib/components/BrowseTab.svelte"
BINDINGS = ROOT / "crates/kiro-control-center/src/lib/bindings.ts"

browse = BROWSE.read_text(encoding="utf-8")
bindings = BINDINGS.read_text(encoding="utf-8")

agent_match = re.search(
    r"export type AgentItemInfo = \{(?P<body>.*?)\n\};", bindings, re.DOTALL
)
if agent_match is None:
    raise SystemExit("AgentItemInfo binding not found")

agent_fields = re.findall(r"^\s*([a-zA-Z_][a-zA-Z0-9_]*):", agent_match["body"], re.MULTILINE)
drawer_values = re.findall(r"/\* acceptMcp \*/\s*(true|false)", browse)
whole_values = re.findall(r"acceptMcp:\s*(true|false)", browse)
warning_present = 'kind: "mcp_servers_require_opt_in"' in bindings

if len(drawer_values) != 1 or len(whole_values) != 1:
    raise SystemExit(
        f"expected one drawer and one whole-plugin value, got {drawer_values=} {whole_values=}"
    )

result = {
    "agent_catalog_fields": agent_fields,
    "catalog_has_preinstall_mcp_signal": any("mcp" in field for field in agent_fields),
    "drawer_accept_mcp": drawer_values[0],
    "whole_plugin_accept_mcp": whole_values[0],
    "post_install_warning_available": warning_present,
}
print(json.dumps(result, indent=2, sort_keys=True))

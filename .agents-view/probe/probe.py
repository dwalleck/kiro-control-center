#!/usr/bin/env python3
"""Probe: list-page payload for the agents view (slice 1).

Source-of-truth strategy: FILESYSTEM-FIRST.
  1. Walk .kiro/agents/*.json.
  2. Parse each file as JSON.
  3. Project to row shape (name, description, model, counts).
  4. Look up lineage in .kiro/installed-agents.json by agent name; null if absent.

Output: JSON array of rows, sorted by name. Stable for diff against oracle.

Usage: python probe.py <project_path>
"""
import json
import sys
from pathlib import Path


def hook_count(hooks):
    if not isinstance(hooks, dict):
        return 0
    return sum(len(v) for v in hooks.values() if isinstance(v, list))


def main(project):
    root = Path(project)
    agents_dir = root / ".kiro" / "agents"
    tracking_file = root / ".kiro" / "installed-agents.json"

    tracking = {}
    if tracking_file.exists():
        tracking = json.loads(tracking_file.read_text(encoding="utf-8")).get("agents", {})

    rows = []
    for jf in sorted(agents_dir.glob("*.json")):
        agent = json.loads(jf.read_text(encoding="utf-8"))
        # 2026-05-22 update (spec D14 revision): row identity is the
        # filename stem, not the JSON `name` field. Original probe used
        # `agent.get("name", jf.stem)` to mirror the now-superseded
        # "JSON name first, stem fallback" policy. Switched to always
        # using the stem so the probe stays in lockstep with the Rust
        # binary. See PR #120 / kiro-78io.
        name = jf.stem
        track = tracking.get(name)
        rows.append({
            "name": name,
            "description": agent.get("description"),
            "model": agent.get("model"),
            "tools_count": len(agent.get("tools") or []),
            "mcp_count": len(agent.get("mcpServers") or {}),
            "resources_count": len(agent.get("resources") or []),
            "hooks_count": hook_count(agent.get("hooks") or {}),
            "lineage": {
                "marketplace": track["marketplace"],
                "plugin": track["plugin"],
                "version": track["version"],
            } if track else None,
        })

    print(json.dumps(sorted(rows, key=lambda r: r["name"]), indent=2, sort_keys=True))


if __name__ == "__main__":
    main(sys.argv[1] if len(sys.argv) > 1 else ".")

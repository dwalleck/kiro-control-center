#!/usr/bin/env python3
"""Probe for kiro-vgnw (slice 2): extract AGENT_TOOLS from the design bundle.

Reads `Kiro Control Center Design System/design_handoff_agents/source/agents-data.js`,
isolates the `window.AGENT_TOOLS = [ ... ]` array literal via regex, and parses
each row with a tolerant JSON-ish reader (the JS uses unquoted keys so we
quote them before handing to json.loads).

Emits a normalized JSON document on stdout:

    {
      "tool_count": <int>,
      "category_count": <int>,
      "categories": ["Cloud", "Code", ...],   # sorted unique
      "tools": [
        {"name": "...", "category": "...", "summary": "..."},
        ...                                    # sorted by name
      ]
    }

Independent of the future `tools-catalog.ts` — this probe answers
"what does the design bundle's AGENT_TOOLS actually contain" without
consulting any port.
"""

import json
import re
import sys
from pathlib import Path


def main() -> int:
    if len(sys.argv) != 2:
        print("usage: probe.py <agents-data.js>", file=sys.stderr)
        return 2
    src = Path(sys.argv[1]).read_text(encoding="utf-8")

    m = re.search(
        r"window\.AGENT_TOOLS\s*=\s*\[(?P<body>.*?)\];",
        src,
        flags=re.DOTALL,
    )
    if not m:
        print("AGENT_TOOLS array literal not found", file=sys.stderr)
        return 2

    # JS object literals use unquoted keys; quote them so json.loads accepts.
    body = m.group("body")
    quoted = re.sub(r"([{,]\s*)([A-Za-z_][A-Za-z_0-9]*)\s*:", r'\1"\2":', body)
    # Strip trailing comma before `]` if any (JS tolerates, JSON does not).
    quoted = re.sub(r",(\s*$)", r"\1", quoted)
    array_text = f"[{quoted}]"

    tools = json.loads(array_text)
    categories = sorted({t["category"] for t in tools})
    tools_sorted = sorted(tools, key=lambda t: t["name"])

    out = {
        "tool_count": len(tools),
        "category_count": len(categories),
        "categories": categories,
        "tools": tools_sorted,
    }
    # Bypass Python's text-mode CRLF translation on Windows so the output
    # diffs byte-for-byte against oracle.mjs (Node writes LF). The earlier
    # probe-s13 hit the same CRLF gotcha; keep both probes consistent.
    payload = json.dumps(out, indent=2, sort_keys=True) + "\n"
    sys.stdout.buffer.write(payload.encode("utf-8"))
    return 0


if __name__ == "__main__":
    sys.exit(main())

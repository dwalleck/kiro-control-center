#!/usr/bin/env python3
"""Probe: Python file-tree walk to find all remediation_hint call sites."""
import json, sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CRATES = ROOT / "crates"

prod_calls = []
test_calls = []
cli_surface = False
tauri_surface = False

for rs in CRATES.rglob("*.rs"):
    text = rs.read_text()
    rel = str(rs.relative_to(CRATES))

    # Count remediation_hint call sites (method call, not definition or doc-comment)
    for lineno, line in enumerate(text.split("\n"), 1):
        if ".remediation_hint(" not in line:
            continue
        if "fn remediation_hint" in line:
            continue
        if line.strip().startswith("///") or line.strip().startswith("//"):
            continue
        if line.strip().startswith("#[doc"):
            continue

        is_test = (
            rel.endswith("_test.rs")
            or "/tests/" in rel
            or "#[cfg(test)]" in text[:text.find(line)]
        )
        entry = f"{rel}:{lineno}"
        if is_test:
            test_calls.append(entry)
        else:
            prod_calls.append(entry)

    # Surface check
    if "kiro-market/src/" in rel and "remediation_hint" in text:
        cli_surface = True
    if "error.rs" in rel and "src-tauri" in rel and "remediation_hint" in text:
        tauri_surface = True

result = {
    "production_call_sites": len(prod_calls),
    "test_call_sites": len(test_calls),
    "cli_surface_calls_remediation_hint": cli_surface,
    "tauri_surface_calls_remediation_hint": tauri_surface,
    "remediation_is_dead_code": (
        len(prod_calls) == 0 and not cli_surface and not tauri_surface
    ),
}

json.dump(result, sys.stdout, indent=2)

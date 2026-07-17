#!/usr/bin/env python3
"""Oracle: shell-pipeline + per-file cfg(test) context check.

Two independent mechanisms from the probe:
1. grep for call sites (not Python file-tree walk)
2. per-file cfg(test) detection via grep for #[cfg(test)] before the call line
"""
import subprocess, json, sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CRATES = ROOT / "crates"

# 1. All .remediation_hint( call sites
r = subprocess.run(
    ["grep", "-rn", r"\.remediation_hint(", str(CRATES)],
    capture_output=True, text=True
)
call_lines = [l.strip() for l in r.stdout.split("\n") if l.strip()]
call_lines = [l for l in call_lines if "fn remediation_hint" not in l]

def is_in_test(fp: str, lineno: int) -> bool:
    """Check if line N in file is inside a #[cfg(test)] module."""
    p = Path(fp)
    if not p.exists():
        return False
    # Check file path convention first
    rel = str(p)
    if rel.endswith("_test.rs") or "/tests/" in rel:
        return True
    # Check if #[cfg(test)] appears before lineno
    with open(p) as fh:
        for i, line in enumerate(fh, 1):
            if i >= lineno:
                break
            if "#[cfg(test)]" in line:
                return True
    return False

prod = []
test_only = []
for line in call_lines:
    parts = line.split(":")
    fp = parts[0]
    lineno = int(parts[1])
    if is_in_test(fp, lineno):
        test_only.append(line)
    else:
        prod.append(line)

# 2. CLI surface
r2 = subprocess.run(
    ["grep", "-rl", "remediation_hint", str(CRATES / "kiro-market" / "src")],
    capture_output=True, text=True
)
cli_files = [f for f in r2.stdout.strip().split("\n") if f]

# 3. Tauri surface
r3 = subprocess.run(
    ["grep", "-c", "remediation_hint",
     str(CRATES / "kiro-control-center" / "src-tauri" / "src" / "error.rs")],
    capture_output=True, text=True
)
tauri_count = int(r3.stdout.strip() or 0)

result = {
    "method_call_sites_total": len(call_lines),
    "production_call_sites": len(prod),
    "test_call_sites": len(test_only),
    "cli_files_with_remediation": len(cli_files),
    "tauri_error_remediation_mentions": tauri_count,
    "is_dead_code": len(prod) == 0 and len(cli_files) == 0 and tauri_count == 0,
}

json.dump(result, sys.stdout, indent=2)

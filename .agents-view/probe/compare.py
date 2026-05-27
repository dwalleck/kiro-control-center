#!/usr/bin/env python3
"""Structural comparison between probe and oracle outputs.

Text diff fails on key-order differences between Python (alphabetical sort)
and PowerShell (insertion order). Compare semantically: load both, sort,
deep-equal.

Exit 0 on agreement, 1 on disagreement with a row-by-row report.
"""
import json
import sys


def normalize(rows):
    return sorted(rows, key=lambda r: r["name"])


def main(probe_path, oracle_path):
    with open(probe_path, encoding="utf-8-sig") as f:
        probe = normalize(json.load(f))
    with open(oracle_path, encoding="utf-8-sig") as f:
        oracle = normalize(json.load(f))

    if probe == oracle:
        print(f"AGREE on {len(probe)} rows: {[r['name'] for r in probe]}")
        sys.exit(0)

    print(f"DISAGREE: probe has {len(probe)} rows, oracle has {len(oracle)} rows")
    probe_names = {r["name"] for r in probe}
    oracle_names = {r["name"] for r in oracle}
    if probe_names - oracle_names:
        print(f"  only in probe:  {sorted(probe_names - oracle_names)}")
    if oracle_names - probe_names:
        print(f"  only in oracle: {sorted(oracle_names - probe_names)}")
    for p, o in zip(probe, oracle):
        if p != o:
            print(f"  row {p['name']}: probe != oracle")
            for k in sorted(set(p) | set(o)):
                if p.get(k) != o.get(k):
                    print(f"    {k}: probe={p.get(k)!r} oracle={o.get(k)!r}")
    sys.exit(1)


if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2])

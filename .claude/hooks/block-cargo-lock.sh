#!/bin/bash
# Block direct edits to Cargo.lock unless the user has set KIRO_ALLOW_LOCKFILE_EDIT=1.
# Rationale: the workspace Cargo.toml pins `curl = "0.4"` as a feature-unification
# shim for gix-transport. Silent Cargo.lock churn from unrelated edits can shift
# curl-sys feature resolution and break Windows HTTPS clones. Lockfile changes
# should come from explicit `cargo update` / dep bumps, not side effects.
set -euo pipefail

INPUT=$(cat)
FILE_PATH=$(echo "$INPUT" | jq -r '.tool_input.file_path // empty')

if [[ "$FILE_PATH" != *Cargo.lock ]]; then
  exit 0
fi

if [ "${KIRO_ALLOW_LOCKFILE_EDIT:-0}" = "1" ]; then
  exit 0
fi

cat <<'MSG' >&2
Blocked: direct edit to Cargo.lock.

The workspace Cargo.toml pins `curl = "0.4"` as a feature-unification shim for
gix-transport. Lockfile churn from unrelated edits can shift curl-sys's TLS
feature resolution and break Windows HTTPS clones.

To proceed:
  1. If this lockfile change is the result of a dep bump, regenerate it via
     `cargo update -p <crate>` instead of editing directly.
  2. To override this guard for one session, export KIRO_ALLOW_LOCKFILE_EDIT=1
     and retry.
MSG
exit 2

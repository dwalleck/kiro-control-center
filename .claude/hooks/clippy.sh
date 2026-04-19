#!/bin/bash
# Run clippy scoped to the package containing the edited .rs file.
# Surfaces lint failures back to Claude on stdout; never blocks the edit.
set -euo pipefail

INPUT=$(cat)
FILE_PATH=$(echo "$INPUT" | jq -r '.tool_input.file_path // empty')

if [ -z "$FILE_PATH" ] || [[ "$FILE_PATH" != *.rs ]]; then
  exit 0
fi
if [ ! -f "$FILE_PATH" ]; then
  exit 0
fi

# Derive the package name from the path: crates/<name>/src/...
PKG=""
case "$FILE_PATH" in
  */crates/kiro-market-core/*) PKG="kiro-market-core" ;;
  */crates/kiro-market/*)      PKG="kiro-market" ;;
  */crates/kiro-control-center/src-tauri/*) PKG="kiro-control-center" ;;
esac

if [ -z "$PKG" ]; then
  exit 0
fi

cd "$CLAUDE_PROJECT_DIR"

# --no-deps keeps output tight; --message-format=short gives one-line-per-issue.
# Cap to 40 lines so large failures don't flood context.
OUTPUT=$(cargo clippy --package "$PKG" --no-deps --message-format=short -- -D warnings 2>&1 | tail -n 40 || true)

if echo "$OUTPUT" | grep -qE '^(error|warning):'; then
  echo "clippy ($PKG) flagged issues:"
  echo "$OUTPUT"
fi

exit 0

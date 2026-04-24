#!/usr/bin/env python3
"""Parse Kiro review-orchestrator output and post as GitHub PR review comments.

Primary path: parse orchestrator markdown into structured findings, then post
inline review comments for findings whose line is inside the PR diff. Findings
on lines outside the diff are appended to the review summary body.

Fallback paths exist for when parsing fails (no findings extracted) or when
GitHub's review API rejects the payload; both degrade to a plain issue comment
so the review content is never lost silently.
"""

import argparse
import json
import re
import subprocess
import sys
from pathlib import Path


# The orchestrator and all specialists emit the per-finding block defined in
# .kiro/steering/review-process.md:214-237:
#
#   #### <severity-emoji> <Severity> — <brief title> [source: agent-name]
#
#   **File:** `path/to/file.ext:line-range`
#
#   **Code:** ...
#   **Problem:** ...
#   **Fix direction:** ...
#
# Each block begins with a level-4 heading and runs until the next level-≤4
# heading (or EOF). The severity word is one of the buckets from
# review-process.md:128-138. The emoji is optional so minor format drift
# (missing glyph, unicode substitution) doesn't silently drop findings.
_SEVERITY_WORDS = r"Critical|Important|Suggestion|Nitpick|Verified|Uncertain"
FINDING_HEADER_RE = re.compile(
    r"^####\s+"
    r"(?:\S+\s+)?"                                   # optional severity emoji
    r"(?P<severity>" + _SEVERITY_WORDS + r")\s+"
    r"[—–\-]\s+"                           # em-dash, en-dash, or hyphen
    r"(?P<rest>.+?)\s*$"
)

# Kiro CLI's `chat --no-interactive` output contains ANSI color sequences in
# two forms:
#   * Real CSI escapes (\x1b[...m) that propagate when the CLI's color
#     renderer runs despite redirection.
#   * Literal caret-bracket strings (`^[[...m`, where `^` and `[` are
#     printable ASCII) that the CLI emits when it detects a non-TTY stdout
#     but still encodes color sequences into the stream.
# Both forms wrap every markdown heading and break `^####` anchoring. Also
# matches non-`m` CSI sequences (`\x1b[K` erase-line, cursor moves) that
# some terminal renderers inject during long-running output.
ANSI_RE = re.compile(r"(?:\x1b|\^\[)\[[0-9;?]*[a-zA-Z]")

# The orchestrator prompt (review-orchestrator.md:110) pins the start of
# the actual review at a level-1 `# Code Review —` heading. Everything
# before that heading is `kiro-cli chat` session narration (tool calls,
# tool output, assistant thinking) and must not leak into the parsed
# findings or the raw-output fallback comment.
REVIEW_START_RE = re.compile(r"^#\s+Code Review\b", re.MULTILINE)

# `File:` is the required locator from review-process.md:220. The spec
# form is `**File:** `path:line``, but kiro-cli renders markdown for its
# terminal output: bold markers become ANSI bold (\[1m ... \[22m) and
# inline code becomes colored text (\[38;5;10m...\[0m). After ANSI
# stripping, what remains is plain `File: path:line` — no `**` markers,
# no backticks. This pattern accepts both shapes so the parser works
# regardless of whether `NO_COLOR`/`TERM=dumb` convinces the CLI to
# preserve the raw markdown. Anchored on line-start so it doesn't match
# `File: foo.ts` mentions inside a Problem paragraph.
FILE_LINE_RE = re.compile(
    r"^\s*(?:\*\*)?File:(?:\*\*)?\s+`?(?P<path>[^\s`]+?):(?P<line>\d+)(?:-\d+)?`?\s*$",
    re.MULTILINE,
)

# Orchestrator annotates each block with [source: agent-name] per
# review-orchestrator.md:129. Placement isn't pinned to the title line, so
# we search the whole block.
SOURCE_TAG_RE = re.compile(r"\[source:\s*(?P<agent>[^\]]+?)\s*\]")

# A line whose trimmed form starts with ```` ``` ```` or `~~~` opens or closes
# a fenced code block. Tracking this prevents `####` that appears inside a
# quoted code sample from being mistaken for a new finding heading.
FENCE_RE = re.compile(r"^\s*(?:`{3,}|~{3,})")

# A level-1-to-4 heading (outside a fence) terminates the previous finding
# block. `##` and `###` delimit section grouping (Critical/Important/...);
# `#` is the top-level title; `####` is the next finding. Anything deeper
# (`#####`+) is allowed inside a finding body.
HEADING_TERMINATOR_RE = re.compile(r"^#{1,4}\s")

# GitHub's PR review API rejects comments whose body exceeds ~65k chars and
# rejects whole review payloads over the same limit. Cap each finding body so
# a single verbose finding can't bust the limit for the whole review.
MAX_BODY_CHARS = 2000


def _strip_ansi(text):
    """Remove ANSI/CSI escape sequences, including the literal `^[` encoding.

    Returns the text unchanged if no matches are found, so the cost on a
    clean input is a single regex scan.
    """
    return ANSI_RE.sub("", text)


def preprocess_review_text(text):
    """Clean and trim raw kiro-cli output to the review proper.

    Two passes, both required:
      1. Strip ANSI sequences so `^####` anchors match heading lines.
      2. Trim everything before `# Code Review` so tool-call narration
         doesn't pollute parsed findings or the raw-output fallback.

    If the review start marker is absent — which typically means the
    orchestrator crashed before producing its final report — returns the
    ANSI-stripped text as-is so callers can still post it via the fallback
    path rather than silently dropping the run.
    """
    stripped = _strip_ansi(text)
    match = REVIEW_START_RE.search(stripped)
    if match is None:
        return stripped
    return stripped[match.start():]


def _iter_finding_blocks(text):
    """Yield (header_line, block_text) for every level-4 heading in ``text``.

    Respects fenced code blocks: a ```` ``` ```` fence toggles "inside fence"
    state, and headings inside a fence are treated as literal content rather
    than block boundaries. Without this, a specialist that quotes code
    containing `####` comments (e.g. Python or shell) would see its finding
    body truncated at the fake heading.
    """
    lines = text.splitlines()
    in_fence = False
    start = None
    for i, line in enumerate(lines):
        if FENCE_RE.match(line):
            in_fence = not in_fence
            continue
        if in_fence:
            continue
        if line.startswith("#### "):
            if start is not None:
                yield lines[start], "\n".join(lines[start:i])
            start = i
        elif HEADING_TERMINATOR_RE.match(line) and start is not None:
            # A higher-level heading (1-3 hashes) closes the current block
            # without starting a new one.
            yield lines[start], "\n".join(lines[start:i])
            start = None
    if start is not None:
        yield lines[start], "\n".join(lines[start:])


def parse_findings(text):
    r"""Extract findings with file:line references from orchestrator markdown.

    The orchestrator emits the level-4 per-finding block from
    review-process.md. Findings missing a `**File:** `path:line`` locator
    cannot be posted inline and are skipped — they typically belong to
    narrative sections (Holistic Assessment, Verified Findings summaries)
    that don't need inline commenting anyway.
    """
    findings = []
    for header, block in _iter_finding_blocks(text):
        header_match = FINDING_HEADER_RE.match(header)
        if header_match is None:
            continue

        file_match = FILE_LINE_RE.search(block)
        if file_match is None:
            continue

        source_match = SOURCE_TAG_RE.search(block)
        agent = source_match.group("agent").strip() if source_match else "unknown"

        body = block.strip()
        if len(body) > MAX_BODY_CHARS:
            body = body[:MAX_BODY_CHARS].rstrip() + "\n\n… (truncated)"

        findings.append({
            "agent": agent,
            "severity": header_match.group("severity"),
            "path": file_match.group("path").strip(),
            "line": int(file_match.group("line")),
            "body": body,
        })
    return findings


# Single-character C escapes used in git's quoted-path form. Octal triplets
# (\NNN) for arbitrary bytes are handled separately because they're the
# encoding of non-ASCII UTF-8 bytes and must be reassembled before decoding.
_C_ESCAPE = {
    "a": 0x07, "b": 0x08, "t": 0x09, "n": 0x0A,
    "v": 0x0B, "f": 0x0C, "r": 0x0D,
    '"': 0x22, "\\": 0x5C,
}


def _unquote_diff_path(inner):
    """Decode the inner body of a git-quoted diff path into a str.

    Git's quoted form (documented in `git config core.quotepath`) wraps the
    path in double quotes and emits:
      - C escapes (\\t, \\n, \\\", \\\\, etc.)
      - Three-digit octal triplets (\\303\\251) for each non-ASCII byte.

    The octal bytes assemble into the UTF-8 encoding of the original path,
    so we build a bytearray and decode it as UTF-8 at the end. Anything we
    don't recognize is preserved verbatim; a broken path is still better
    than silently skipping a finding.
    """
    buf = bytearray()
    i = 0
    n = len(inner)
    while i < n:
        c = inner[i]
        if c != "\\":
            buf.extend(c.encode("utf-8"))
            i += 1
            continue
        if i + 1 >= n:
            buf.append(ord("\\"))
            break
        nxt = inner[i + 1]
        if nxt.isdigit() and i + 3 < n and inner[i + 2].isdigit() and inner[i + 3].isdigit():
            try:
                buf.append(int(inner[i + 1:i + 4], 8))
                i += 4
                continue
            except ValueError:
                pass
        if nxt in _C_ESCAPE:
            buf.append(_C_ESCAPE[nxt])
            i += 2
            continue
        buf.extend(nxt.encode("utf-8"))
        i += 2
    return buf.decode("utf-8", errors="replace")


def _decode_diff_path(raw):
    """Decode a path token from a `+++ ` diff header line.

    Git quotes paths containing special characters (spaces, tabs, non-ASCII)
    when `core.quotepath` is true — the default. We also call git with
    `-c core.quotepath=false` in `get_diff_lines` to avoid quoting entirely,
    but `parse_diff_hunks` is exposed for testing and may be handed diffs
    produced elsewhere — so it handles both shapes.
    """
    if len(raw) >= 2 and raw[0] == '"' and raw[-1] == '"':
        raw = _unquote_diff_path(raw[1:-1])
    if raw.startswith(("a/", "b/")):
        return raw[2:]
    return raw


def parse_diff_hunks(diff_text):
    """Return {filepath: set(line_numbers)} parsed from a unified-diff string.

    Expects `git diff -U0` output so @@ hunk ranges describe only changed lines.
    GitHub's PR review API rejects inline comments on lines outside the diff,
    so this set is the allowlist for routing findings inline vs. to the summary.

    Handles both plain (`+++ b/path`) and quoted (`+++ "b/path with space"`)
    path forms so paths with spaces or non-ASCII characters still participate
    in inline commenting.
    """
    diff_lines = {}
    current_file = None
    for line in diff_text.splitlines():
        if line.startswith("+++ "):
            token = line[4:]
            if token == "/dev/null":
                current_file = None
            else:
                current_file = _decode_diff_path(token)
                diff_lines.setdefault(current_file, set())
        elif line.startswith("@@") and current_file:
            m = re.search(r"\+(\d+)(?:,(\d+))?", line)
            if m:
                start = int(m.group(1))
                count = int(m.group(2)) if m.group(2) else 1
                for i in range(start, start + count):
                    diff_lines[current_file].add(i)
    return diff_lines


def get_diff_lines(base_ref):
    """Shell out to `git diff -U0` and parse the result. Raises on git failure."""
    if not base_ref:
        raise ValueError("base_ref is empty; cannot compute diff range")
    # -U0 strips context lines so @@ hunk ranges cover only changed lines —
    # GitHub only accepts inline review comments on changed lines, not context.
    # core.quotepath=false disables C-style quoting of non-ASCII paths so
    # parse_diff_hunks sees raw UTF-8 filenames and matches finding paths
    # that were never quoted in the first place.
    result = subprocess.run(
        [
            "git", "-c", "core.quotepath=false",
            "diff", "-U0", f"origin/{base_ref}...HEAD",
        ],
        capture_output=True, text=True,
    )
    if result.returncode != 0:
        raise RuntimeError(
            f"git diff failed (exit {result.returncode}): {result.stderr.strip()}"
        )
    return parse_diff_hunks(result.stdout)


def looks_like_findings(text):
    """Heuristic: does the text look like it should contain parseable findings?

    Used to distinguish "orchestrator reported no issues" from "parser broke" —
    if the text contains level-4 severity headings but zero were parsed out,
    the regex is likely out of sync with the orchestrator prompt.
    """
    return bool(
        re.search(
            rf"^####\s+(?:\S+\s+)?(?:{_SEVERITY_WORDS})\s+[—–\-]\s",
            text,
            re.MULTILINE,
        )
    )


def gh_issue_comment(repo, pr, body):
    """Post a plain issue comment. Returns (returncode, stdout, stderr).

    Passes the body through stdin as a JSON payload via `gh api --input -`.
    Sending large bodies through `-f body=...` puts the text on argv, which
    is bounded by ARG_MAX and fragile for content containing shell-special
    sequences. `gh api` writes 4xx/5xx response bodies to stdout (not stderr),
    so callers need both streams to diagnose failures.
    """
    result = subprocess.run(
        ["gh", "api", f"/repos/{repo}/issues/{pr}/comments", "--input", "-"],
        input=json.dumps({"body": body}),
        capture_output=True, text=True,
    )
    return result.returncode, result.stdout, result.stderr


def _format_summary_entry(path, line, body):
    """Render an out-of-diff finding as a single markdown list item.

    Indents body continuation lines by two spaces so multi-line finding bodies
    render as one list item in GitHub's markdown. Without this, only the
    first line attaches to the bullet and the rest renders as top-level prose.
    """
    indented = body.replace("\n", "\n  ")
    return f"- `{path}:{line}` — {indented}"


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--review-file", required=True)
    parser.add_argument("--repo", required=True)
    parser.add_argument("--pr", required=True, type=int)
    parser.add_argument("--sha", required=True)
    parser.add_argument("--base-ref", required=True,
                        help="Base branch name for diff range (e.g. 'main')")
    args = parser.parse_args()

    raw_text = Path(args.review_file).read_text(encoding="utf-8")
    if not raw_text.strip():
        print("::error::Review output is empty — the orchestrator likely failed.",
              file=sys.stderr)
        sys.exit(1)

    # Strip ANSI sequences and trim to the `# Code Review` H1 so downstream
    # parsing — and any raw-output fallback — never sees the chat transcript.
    review_text = preprocess_review_text(raw_text)

    findings = parse_findings(review_text)
    if not findings:
        if looks_like_findings(review_text):
            print("::warning::Review text contains finding-like headings but "
                  "parse_findings extracted none — the orchestrator output "
                  "format may have drifted from the parser's regex.",
                  file=sys.stderr)
        rc, out, err = gh_issue_comment(
            args.repo, args.pr, f"## Kiro Review\n\n{review_text}"
        )
        if rc != 0:
            print(f"::error::Failed to post raw-output fallback comment. "
                  f"stderr={err} stdout={out}", file=sys.stderr)
            sys.exit(1)
        print("No inline findings parsed. Posted raw output as PR comment.")
        return

    diff_lines = get_diff_lines(args.base_ref)

    inline_comments = []
    body_comments = []
    for f in findings:
        in_diff = f["path"] in diff_lines and f["line"] in diff_lines[f["path"]]
        if in_diff:
            inline_comments.append({
                "path": f["path"],
                "line": f["line"],
                "body": f["body"],
            })
        else:
            body_comments.append(_format_summary_entry(f["path"], f["line"], f["body"]))

    body_parts = ["## Kiro Review Summary"]
    body_parts.append(
        f"Found **{len(findings)}** findings "
        f"({len(inline_comments)} inline, {len(body_comments)} in summary).\n"
    )
    if body_comments:
        body_parts.append("### Findings outside diff range\n")
        body_parts.extend(body_comments)

    review_payload = {
        "commit_id": args.sha,
        "body": "\n".join(body_parts),
        "event": "COMMENT",
        "comments": inline_comments,
    }

    result = subprocess.run(
        ["gh", "api", f"/repos/{args.repo}/pulls/{args.pr}/reviews",
         "--input", "-"],
        input=json.dumps(review_payload),
        capture_output=True, text=True,
    )

    if result.returncode == 0:
        print(f"Posted review with {len(inline_comments)} inline comments.")
        return

    print(f"::warning::Primary review post failed. "
          f"stderr={result.stderr} stdout={result.stdout}", file=sys.stderr)
    fallback = "\n".join(body_parts)
    if inline_comments:
        fallback += "\n\n### Inline findings\n"
        fallback += "\n".join(
            _format_summary_entry(c["path"], c["line"], c["body"])
            for c in inline_comments
        )
    rc, out, err = gh_issue_comment(args.repo, args.pr, fallback)
    if rc != 0:
        print(f"::error::Both primary review and fallback issue comment failed. "
              f"Fallback stderr={err} stdout={out}", file=sys.stderr)
        sys.exit(1)
    print("Fell back to plain PR comment.")


if __name__ == "__main__":
    main()

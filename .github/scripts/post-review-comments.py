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
# `File: foo.ts` mentions inside a Problem paragraph. The `(?!\w+://)`
# lookahead rejects URLs (e.g. `File: http://x.com:8080`) that would
# otherwise parse as `path=http://x.com`, `line=8080`.
FILE_LINE_RE = re.compile(
    r"^\s*(?:\*\*)?File:(?:\*\*)?\s+`?(?!\w+://)(?P<path>[^\s`]+?):(?P<line>\d+)(?:-\d+)?`?\s*$",
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

# The orchestrator emits a structured JSON manifest after the markdown
# review (see review-orchestrator.md "Machine-Readable Manifest" section).
# These regexes locate the section and the fenced JSON payload inside it,
# so we can parse findings deterministically instead of regexing prose.
MANIFEST_SECTION_RE = re.compile(
    r"^##\s+Machine-Readable Findings\b\s*$", re.MULTILINE
)
MANIFEST_FENCE_RE = re.compile(
    r"```(?:json)?\s*\n(?P<payload>.*?)\n```", re.DOTALL
)

# Severity → emoji for reconstructing a finding's heading line when
# rendering a JSON-manifest entry as an inline comment body. Kept in sync
# with review-process.md's severity buckets; unknown severities fall
# through with no emoji rather than being rejected.
_SEVERITY_EMOJI = {
    "Critical": "❌",
    "Important": "⚠️",
    "Suggestion": "💡",
    "Nitpick": "📝",
    "Verified": "✅",
    "Uncertain": "⚠️",
}

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


def _validate_manifest_item(item):
    """Return True iff a JSON-manifest entry passes schema checks.

    Required shape per the review-orchestrator prompt:
      severity: str (one of the six buckets; not enforced here so new
                     orchestrator severities don't break parsing)
      agent:    str
      path:     str, forward-slash
      line:     int >= 1
      title:    str
      body:     str

    Any missing field, wrong type, or empty required string fails the
    check. The caller's contract is "all items valid or fall back to
    regex" — one bad entry invalidates the whole manifest rather than
    producing a partial list that silently drops findings.
    """
    if not isinstance(item, dict):
        return False
    required = {"severity", "agent", "path", "line", "title", "body"}
    if not required.issubset(item):
        return False
    if not isinstance(item["line"], int) or item["line"] < 1:
        return False
    for key in ("severity", "agent", "path", "title", "body"):
        if not isinstance(item[key], str) or not item[key]:
            return False
    return True


def _render_manifest_body(item):
    """Reconstruct a markdown heading + body from a JSON manifest entry.

    The manifest keeps `title` and `body` separate so the orchestrator
    doesn't have to emit the heading twice; rendering prepends the
    conventional `#### <emoji> <severity> — <title> [source: <agent>]`
    heading used by the regex path so inline comment bodies look
    identical regardless of which parser produced them.
    """
    severity = item["severity"]
    emoji = _SEVERITY_EMOJI.get(severity, "")
    emoji_prefix = f"{emoji} " if emoji else ""
    header = f"#### {emoji_prefix}{severity} — {item['title']} [source: {item['agent']}]"
    body = f"{header}\n\n{item['body']}"
    if len(body) > MAX_BODY_CHARS:
        body = body[:MAX_BODY_CHARS].rstrip() + "\n\n… (truncated)"
    return body


def parse_json_manifest(text):
    """Extract findings from the orchestrator's JSON manifest, or None.

    Returns:
      list[dict] — when a valid manifest section exists. May be empty
                   if the orchestrator legitimately found no issues.
      None       — when the manifest section is absent, the fenced JSON
                   block is missing, JSON parsing fails, or any item
                   fails schema validation. Signals "fall back to the
                   regex parser" — callers must distinguish this from
                   "valid empty manifest" so a clean review doesn't
                   accidentally trigger format-drift warnings.

    The JSON path is preferred over the regex parser because it bypasses
    markdown-rendering drift (bold markers eaten, backticks stripped,
    emoji substitution) that the regex parser has to tolerate.
    """
    section_match = MANIFEST_SECTION_RE.search(text)
    if section_match is None:
        return None
    fence_match = MANIFEST_FENCE_RE.search(text, pos=section_match.end())
    if fence_match is None:
        print("::warning::Machine-Readable Findings section found but no "
              "fenced JSON block inside it. Falling back to regex parser.",
              file=sys.stderr)
        return None
    try:
        payload = json.loads(fence_match.group("payload"))
    except json.JSONDecodeError as exc:
        print(f"::warning::JSON manifest failed to parse ({exc}). "
              f"Falling back to regex parser.", file=sys.stderr)
        return None
    if not isinstance(payload, list):
        print(f"::warning::JSON manifest is not a list "
              f"(got {type(payload).__name__}). "
              f"Falling back to regex parser.", file=sys.stderr)
        return None

    findings = []
    for index, item in enumerate(payload):
        if not _validate_manifest_item(item):
            print(f"::warning::JSON manifest item {index} failed schema "
                  f"validation. Falling back to regex parser.",
                  file=sys.stderr)
            return None
        findings.append({
            "agent": item["agent"],
            "severity": item["severity"],
            "path": item["path"],
            "line": item["line"],
            "body": _render_manifest_body(item),
        })
    return findings


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


def _render_fallback_summary(findings):
    """Render all findings as a single issue-comment body.

    Used when `get_diff_lines` fails and we can't determine which findings
    are inside the PR diff for inline commenting — we still want the
    review content on the PR, just not anchored to lines.
    """
    parts = [
        "## Kiro Review Summary",
        f"Found **{len(findings)}** findings. "
        f"Unable to determine diff ranges — posting all findings here "
        f"rather than as inline review comments.\n",
        "### Findings\n",
    ]
    parts.extend(
        _format_summary_entry(f["path"], f["line"], f["body"])
        for f in findings
    )
    return "\n".join(parts)


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
    if not review_text.strip():
        # preprocess_review_text returns the ANSI-stripped text even when
        # no `# Code Review` marker is found, so a whitespace-only result
        # means the original was either all-ANSI or all-whitespace. Either
        # way, nothing to post — exit loudly rather than sending an empty
        # comment that looks like the pipeline succeeded.
        print("::error::Preprocessed review is empty — orchestrator output "
              "contained no reviewable content after ANSI strip / transcript trim.",
              file=sys.stderr)
        sys.exit(1)

    # Prefer the JSON manifest path. When the orchestrator follows the
    # prompt, the manifest is deterministic and bypasses every regex
    # fragility (markdown rendering, bold marker eating, emoji drift).
    # `None` from parse_json_manifest means "manifest missing/invalid —
    # fall through"; an empty list means "orchestrator validly reported
    # no findings."
    findings = parse_json_manifest(review_text)
    parser_source = "json-manifest"
    if findings is None:
        findings = parse_findings(review_text)
        parser_source = "regex"
    print(f"Using {parser_source} parser → {len(findings)} findings")

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
        print("No findings parsed. Posted raw output as PR comment.")
        return

    # get_diff_lines can raise RuntimeError (git failure) or ValueError
    # (empty base_ref). An unhandled exception here would discard all
    # parsed findings and leave the PR without a review comment. Instead,
    # fall through to the issue-comment fallback with every finding in
    # the summary body — no findings lost, just posted less prettily.
    try:
        diff_lines = get_diff_lines(args.base_ref)
    except (RuntimeError, ValueError) as exc:
        print(f"::error::get_diff_lines failed ({exc}). Falling back to "
              f"issue comment with all {len(findings)} findings in summary.",
              file=sys.stderr)
        rc, out, err = gh_issue_comment(
            args.repo, args.pr, _render_fallback_summary(findings)
        )
        if rc != 0:
            print(f"::error::Fallback issue comment also failed. "
                  f"stderr={err} stdout={out}", file=sys.stderr)
            sys.exit(1)
        print(f"Posted {len(findings)} findings via issue-comment fallback "
              f"(get_diff_lines failed).")
        return

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

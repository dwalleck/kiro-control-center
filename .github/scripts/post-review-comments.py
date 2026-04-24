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


FINDING_PATTERN = re.compile(
    r"\d+\.\s+\*\*\[([^\]]+)\]\*\*\s+`([^`]+?):(\d+)`\s*\n\s+(.+?)"
    r"(?=\n\s*\d+\.\s+\*\*\[|\n#{2,}\s|\n---|\Z)",
    re.DOTALL,
)

# GitHub's PR review API rejects comments whose body exceeds ~65k chars and
# rejects whole review payloads over the same limit. Cap each finding body so
# a single verbose finding can't bust the limit for the whole review.
MAX_BODY_CHARS = 2000


def parse_findings(text):
    """Extract findings with file:line references from orchestrator markdown output.

    Orchestrator emits findings as numbered list items:
        1. **[agent-name]** `path/to/file:42`
           Description text, possibly multi-line.
    The terminator stops at the next numbered finding, the next `## H2` header,
    or end-of-string — keeps descriptions from bleeding across findings.
    """
    findings = []
    for m in FINDING_PATTERN.finditer(text):
        description = m.group(4).strip()
        if len(description) > MAX_BODY_CHARS:
            description = description[:MAX_BODY_CHARS] + "… (truncated)"
        findings.append({
            "agent": m.group(1).strip(),
            "path": m.group(2).strip(),
            "line": int(m.group(3)),
            "body": f"**[{m.group(1).strip()}]** {description}",
        })
    return findings


def parse_diff_hunks(diff_text):
    """Return {filepath: set(line_numbers)} parsed from a unified-diff string.

    Expects `git diff -U0` output so @@ hunk ranges describe only changed lines.
    GitHub's PR review API rejects inline comments on lines outside the diff,
    so this set is the allowlist for routing findings inline vs. to the summary.
    """
    diff_lines = {}
    current_file = None
    for line in diff_text.splitlines():
        if line.startswith("+++ b/"):
            current_file = line[6:]
            diff_lines.setdefault(current_file, set())
        elif line.startswith("+++ /dev/null"):
            current_file = None
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
    result = subprocess.run(
        ["git", "diff", "-U0", f"origin/{base_ref}...HEAD"],
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
    if the text contains numbered `**[agent]**` markers but zero parsed out,
    the regex is likely out of sync with the orchestrator prompt.
    """
    return bool(re.search(r"^\s*\d+\.\s+\*\*\[", text, re.MULTILINE))


def gh_issue_comment(repo, pr, body):
    """Post a plain issue comment. Returns (returncode, stdout, stderr).

    `gh api` writes 4xx/5xx response bodies to stdout (not stderr), so callers
    need both streams to diagnose failures.
    """
    result = subprocess.run(
        ["gh", "api", f"/repos/{repo}/issues/{pr}/comments",
         "-f", f"body={body}"],
        capture_output=True, text=True,
    )
    return result.returncode, result.stdout, result.stderr


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--review-file", required=True)
    parser.add_argument("--repo", required=True)
    parser.add_argument("--pr", required=True, type=int)
    parser.add_argument("--sha", required=True)
    parser.add_argument("--base-ref", required=True,
                        help="Base branch name for diff range (e.g. 'main')")
    args = parser.parse_args()

    review_text = Path(args.review_file).read_text(encoding="utf-8")
    if not review_text.strip():
        print("::error::Review output is empty — the orchestrator likely failed.",
              file=sys.stderr)
        sys.exit(1)

    findings = parse_findings(review_text)
    if not findings:
        if looks_like_findings(review_text):
            print("::warning::Review text contains finding-like markers but "
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
            body_comments.append(f"`{f['path']}:{f['line']}` — {f['body']}")

    body_parts = ["## Kiro Review Summary"]
    body_parts.append(
        f"Found **{len(findings)}** findings "
        f"({len(inline_comments)} inline, {len(body_comments)} in summary).\n"
    )
    if body_comments:
        body_parts.append("### Findings outside diff range\n")
        body_parts.extend(f"- {c}" for c in body_comments)

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
            f"- `{c['path']}:{c['line']}` — {c['body']}" for c in inline_comments
        )
    rc, out, err = gh_issue_comment(args.repo, args.pr, fallback)
    if rc != 0:
        print(f"::error::Both primary review and fallback issue comment failed. "
              f"Fallback stderr={err} stdout={out}", file=sys.stderr)
        sys.exit(1)
    print("Fell back to plain PR comment.")


if __name__ == "__main__":
    main()

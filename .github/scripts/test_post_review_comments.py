"""Smoke tests for post-review-comments.py.

The script's filename uses hyphens and isn't a valid Python identifier, so we
load it via importlib. Run with: python3 -m pytest .github/scripts/
"""

import importlib.util
import sys
from pathlib import Path

import pytest


_SCRIPT = Path(__file__).parent / "post-review-comments.py"
_spec = importlib.util.spec_from_file_location("post_review_comments", _SCRIPT)
assert _spec is not None and _spec.loader is not None
_mod = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(_mod)

parse_findings = _mod.parse_findings
parse_diff_hunks = _mod.parse_diff_hunks
looks_like_findings = _mod.looks_like_findings


ORCHESTRATOR_SAMPLE = """# PR Review Summary

| Severity | Count | Agents |
|----------|-------|--------|
| Critical | 2 | code-reviewer, silent-failure-hunter |
| Suggestion | 1 | comment-analyzer |

## Critical Issues

1. **[code-reviewer]** `src/auth/login.ts:45`
   Missing null check on session token before database write.

2. **[silent-failure-hunter]** `src/middleware/session.ts:88`
   Catch block swallows TimeoutException without logging.

## Suggestions

1. **[comment-analyzer]** `src/utils/helpers.ts:30`
   Comment describes old behavior — update to match current logic.

## Positive Observations

- Good test coverage for edge cases.
"""


class TestParseFindings:
    def test_extracts_all_findings(self):
        findings = parse_findings(ORCHESTRATOR_SAMPLE)
        assert len(findings) == 3

    def test_structured_fields_match_input(self):
        findings = parse_findings(ORCHESTRATOR_SAMPLE)
        assert findings[0] == {
            "agent": "code-reviewer",
            "path": "src/auth/login.ts",
            "line": 45,
            "body": "**[code-reviewer]** Missing null check on session token "
                    "before database write.",
        }

    def test_finding_across_sections_does_not_bleed(self):
        findings = parse_findings(ORCHESTRATOR_SAMPLE)
        # The last Critical finding must stop at the ## Suggestions header,
        # not swallow the Suggestions list below.
        assert "Comment describes" not in findings[1]["body"]
        assert findings[2]["agent"] == "comment-analyzer"

    def test_empty_input_returns_empty(self):
        assert parse_findings("") == []

    def test_prose_without_structure_returns_empty(self):
        assert parse_findings("This is a paragraph about nothing.") == []

    def test_missing_backticks_returns_empty(self):
        # Without backtick-wrapped path:line, the regex should not match.
        malformed = "1. **[agent]** path/to/file:45\n   description body."
        assert parse_findings(malformed) == []

    def test_multiline_body_is_captured(self):
        text = (
            "1. **[code-reviewer]** `a.py:10`\n"
            "   First sentence.\n"
            "   Second sentence continues the description."
        )
        findings = parse_findings(text)
        assert len(findings) == 1
        assert "Second sentence" in findings[0]["body"]

    def test_body_containing_numbered_list_is_not_split(self):
        # The terminator requires `\d+.` followed by `**[agent]**` — a plain
        # numbered list inside a body should not be mistaken for the next finding.
        text = (
            "1. **[code-reviewer]** `a.py:10`\n"
            "   Multiple issues:\n"
            "   1. First cause\n"
            "   2. Second cause\n"
        )
        findings = parse_findings(text)
        assert len(findings) == 1
        assert "Second cause" in findings[0]["body"]

    def test_h3_subheader_terminates_finding(self):
        # ### headers delimit subsections within a severity group; the body
        # should stop there rather than swallowing the next section.
        text = (
            "1. **[code-reviewer]** `a.py:10`\n"
            "   Finding body.\n"
            "\n"
            "### Positive Observations\n"
            "- Something good\n"
        )
        findings = parse_findings(text)
        assert len(findings) == 1
        assert "Positive" not in findings[0]["body"]

    def test_body_exceeding_limit_is_truncated(self):
        long_body = "x" * 5000
        text = f"1. **[code-reviewer]** `a.py:10`\n   {long_body}\n"
        findings = parse_findings(text)
        assert len(findings) == 1
        assert findings[0]["body"].endswith("… (truncated)")
        assert len(findings[0]["body"]) < 2500


class TestParseDiffHunks:
    def test_basic_hunk_with_count(self):
        diff = (
            "diff --git a/file.py b/file.py\n"
            "--- a/file.py\n"
            "+++ b/file.py\n"
            "@@ -10,0 +11,2 @@\n"
            "+new line 1\n"
            "+new line 2\n"
        )
        assert parse_diff_hunks(diff) == {"file.py": {11, 12}}

    def test_missing_count_defaults_to_one(self):
        diff = "+++ b/a.py\n@@ -5 +5 @@\n-old\n+new\n"
        assert parse_diff_hunks(diff) == {"a.py": {5}}

    def test_pure_deletion_hunk_has_empty_range(self):
        # +start,0 means the hunk only removed lines — no added lines to comment on.
        diff = "+++ b/a.py\n@@ -10,3 +9,0 @@\n"
        assert parse_diff_hunks(diff) == {"a.py": set()}

    def test_deleted_file_is_skipped(self):
        diff = (
            "--- a/removed.py\n"
            "+++ /dev/null\n"
            "@@ -1,3 +0,0 @@\n"
            "-line\n"
            "+++ b/kept.py\n"
            "@@ -0,0 +1,1 @@\n"
            "+new\n"
        )
        result = parse_diff_hunks(diff)
        assert "removed.py" not in result
        assert result["kept.py"] == {1}

    def test_empty_input(self):
        assert parse_diff_hunks("") == {}

    def test_multiple_hunks_in_same_file(self):
        diff = (
            "+++ b/a.py\n"
            "@@ -5 +5,2 @@\n"
            "+x\n+y\n"
            "@@ -20 +22,1 @@\n"
            "+z\n"
        )
        assert parse_diff_hunks(diff) == {"a.py": {5, 6, 22}}


class TestLooksLikeFindings:
    def test_recognizes_finding_markers(self):
        assert looks_like_findings(ORCHESTRATOR_SAMPLE)

    def test_empty_returns_false(self):
        assert not looks_like_findings("")

    def test_plain_prose_returns_false(self):
        assert not looks_like_findings("No structured content here.")

    def test_numbered_list_without_agent_brackets_returns_false(self):
        assert not looks_like_findings("1. Just a regular numbered list item.")


if __name__ == "__main__":
    sys.exit(pytest.main([__file__, "-v"]))

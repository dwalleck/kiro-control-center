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
_decode_diff_path = _mod._decode_diff_path
_format_summary_entry = _mod._format_summary_entry


# Fixture mirrors the per-finding block from .kiro/steering/review-process.md
# with the [source: agent-name] tag the orchestrator adds per
# review-orchestrator.md:129. Keep this in sync with review-process.md —
# if that doc changes the finding layout, this sample (and the parser) must
# change with it.
ORCHESTRATOR_SAMPLE = """# Code Review — PR #123

## Holistic Assessment

**Motivation:** No concern.
**Scope:** No concern.

---

## Detailed Findings

### ❌ Critical

#### ❌ Critical — Missing null check on session token [source: code-reviewer]

**File:** `src/auth/login.ts:45-47`

**Code:**
```typescript
const token = req.session.token;
db.write(token);
```

**Problem:** The session token may be undefined when the session has expired,
leading to a database write with null and a runtime error downstream.

**Failure scenario:** Request arrives after session timeout; `token` is
`undefined`; `db.write(undefined)` throws in the driver layer.

**Verified with:** `code find_references file=src/auth/login.ts row=45 column=15`

**Fix direction:** This needs design discussion, not a mechanical fix.
Remediation is a separate task after the approach is agreed.

---

### ⚠️ Important

#### ⚠️ Important — TimeoutException swallowed silently [source: silent-failure-hunter]

**File:** `src/middleware/session.ts:88`

**Problem:** Catch block swallows TimeoutException without logging.

**Verified with:** `rg "catch.*Timeout" src/middleware/`

**Fix direction:** Re-raise or at minimum log at warn level so timeouts are
visible in production telemetry.

---

### 💡 Suggestion

#### 💡 Suggestion — Comment describes stale behavior [source: comment-analyzer]

**File:** `src/utils/helpers.ts:30`

**Problem:** Comment describes old behavior — update to match current logic.

**Verified with:** `read src/utils/helpers.ts:25-40`

**Fix direction:** Rewrite the comment to describe what the function
actually does today.

---

## Verified Findings

No claims to verify.
"""


class TestParseFindings:
    def test_extracts_all_findings(self):
        findings = parse_findings(ORCHESTRATOR_SAMPLE)
        assert len(findings) == 3

    def test_structured_fields_match_input(self):
        findings = parse_findings(ORCHESTRATOR_SAMPLE)
        first = findings[0]
        assert first["agent"] == "code-reviewer"
        assert first["severity"] == "Critical"
        assert first["path"] == "src/auth/login.ts"
        assert first["line"] == 45
        assert "Missing null check" in first["body"]
        assert "**File:** `src/auth/login.ts:45-47`" in first["body"]

    def test_body_does_not_bleed_across_blocks(self):
        findings = parse_findings(ORCHESTRATOR_SAMPLE)
        # The Critical finding's body must not include the Important block's
        # "TimeoutException" description that follows it.
        assert "TimeoutException" not in findings[0]["body"]
        # And the Important finding must not pull in the Suggestion below.
        assert "Comment describes" not in findings[1]["body"]

    def test_severity_parsed_per_block(self):
        findings = parse_findings(ORCHESTRATOR_SAMPLE)
        assert [f["severity"] for f in findings] == [
            "Critical", "Important", "Suggestion",
        ]

    def test_empty_input_returns_empty(self):
        assert parse_findings("") == []

    def test_prose_without_structure_returns_empty(self):
        assert parse_findings("This is a paragraph about nothing.") == []

    def test_block_without_file_locator_is_skipped(self):
        # Holistic-assessment-style prose uses #### headings but has no
        # `**File:**` line — those can't be posted inline and must be skipped.
        text = (
            "#### ❌ Critical — Narrative observation [source: code-reviewer]\n"
            "\n"
            "Some prose without a file reference.\n"
        )
        assert parse_findings(text) == []

    def test_missing_source_tag_defaults_to_unknown(self):
        text = (
            "#### 💡 Suggestion — No tag present\n"
            "\n"
            "**File:** `a.py:10`\n"
            "\n"
            "**Problem:** body.\n"
        )
        findings = parse_findings(text)
        assert len(findings) == 1
        assert findings[0]["agent"] == "unknown"

    def test_hash_inside_fenced_code_does_not_split_block(self):
        # Specialists quote user code in **Code:** blocks. If that code
        # contains `####` (shell/Python comment), a naive parser would
        # mistake it for a new finding heading and truncate the body.
        text = (
            "#### ❌ Critical — Shell injection risk [source: code-reviewer]\n"
            "\n"
            "**File:** `script.sh:5`\n"
            "\n"
            "**Code:**\n"
            "```bash\n"
            "#### this looks like a heading but it's a shell comment\n"
            "rm -rf $USER_INPUT\n"
            "```\n"
            "\n"
            "**Problem:** User input flows into rm -rf.\n"
        )
        findings = parse_findings(text)
        assert len(findings) == 1
        assert "User input flows" in findings[0]["body"]
        assert "shell comment" in findings[0]["body"]

    def test_tilde_fence_is_respected(self):
        # GitHub markdown also supports ~~~ fences. Verify we track both.
        text = (
            "#### ❌ Critical — YAML issue [source: code-reviewer]\n"
            "\n"
            "**File:** `config.yml:10`\n"
            "\n"
            "**Code:**\n"
            "~~~yaml\n"
            "#### fake heading inside tilde fence\n"
            "key: value\n"
            "~~~\n"
            "\n"
            "**Problem:** Misconfigured.\n"
        )
        findings = parse_findings(text)
        assert len(findings) == 1
        assert "Misconfigured" in findings[0]["body"]

    def test_section_heading_terminates_block(self):
        # A `### ⚠️ Important` grouping heading must end the preceding
        # `#### Critical` block without starting a new one.
        text = (
            "### ❌ Critical\n"
            "\n"
            "#### ❌ Critical — First [source: code-reviewer]\n"
            "\n"
            "**File:** `a.py:10`\n"
            "\n"
            "**Problem:** crit.\n"
            "\n"
            "### 💡 Suggestion\n"
            "\n"
            "#### 💡 Suggestion — Second [source: comment-analyzer]\n"
            "\n"
            "**File:** `b.py:20`\n"
            "\n"
            "**Problem:** sugg.\n"
        )
        findings = parse_findings(text)
        assert len(findings) == 2
        assert "sugg" not in findings[0]["body"]
        assert findings[1]["severity"] == "Suggestion"

    def test_body_exceeding_limit_is_truncated(self):
        long_body = "x" * 5000
        text = (
            "#### 💡 Suggestion — Verbose [source: comment-analyzer]\n"
            "\n"
            "**File:** `a.py:10`\n"
            "\n"
            "**Problem:** " + long_body + "\n"
        )
        findings = parse_findings(text)
        assert len(findings) == 1
        assert findings[0]["body"].endswith("… (truncated)")
        assert len(findings[0]["body"]) < 2500

    def test_line_range_uses_first_line(self):
        text = (
            "#### ❌ Critical — Range [source: code-reviewer]\n"
            "\n"
            "**File:** `a.py:42-48`\n"
            "\n"
            "**Problem:** body.\n"
        )
        findings = parse_findings(text)
        assert findings[0]["line"] == 42

    def test_hyphen_dash_variants_are_accepted(self):
        # Title separator should tolerate em-dash, en-dash, or ASCII hyphen
        # because AI agents occasionally substitute dash characters.
        for dash in ("—", "–", "-"):
            text = (
                f"#### ❌ Critical {dash} Title [source: code-reviewer]\n"
                "\n"
                "**File:** `a.py:1`\n"
                "\n"
                "**Problem:** body.\n"
            )
            findings = parse_findings(text)
            assert len(findings) == 1, f"dash variant {dash!r} not accepted"


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

    def test_quoted_path_with_spaces(self):
        # Git quotes paths that contain spaces when core.quotepath is true
        # (the default). Without decoding, an inline finding referencing
        # `path with spaces/file.py` would never match the diff entry and
        # would be demoted to the out-of-diff summary.
        diff = (
            '+++ "b/path with spaces/file.py"\n'
            "@@ -0,0 +1 @@\n"
            "+new line\n"
        )
        result = parse_diff_hunks(diff)
        assert "path with spaces/file.py" in result
        assert result["path with spaces/file.py"] == {1}

    def test_quoted_path_with_non_ascii(self):
        # Non-ASCII bytes are octal-escaped inside the quoted form. 'café'
        # encodes as \303\251 for the 'é' byte; decoding must reassemble
        # that back into UTF-8 text.
        diff = (
            '+++ "b/caf\\303\\251/file.py"\n'
            "@@ -0,0 +1 @@\n"
            "+new line\n"
        )
        result = parse_diff_hunks(diff)
        assert "café/file.py" in result


class TestDecodeDiffPath:
    def test_plain_path_strips_prefix(self):
        assert _decode_diff_path("b/src/file.py") == "src/file.py"
        assert _decode_diff_path("a/src/file.py") == "src/file.py"

    def test_plain_path_without_prefix(self):
        assert _decode_diff_path("src/file.py") == "src/file.py"

    def test_quoted_path_unescapes(self):
        assert _decode_diff_path('"b/with space.py"') == "with space.py"

    def test_quoted_path_decodes_octal(self):
        assert _decode_diff_path('"b/caf\\303\\251.py"') == "café.py"

    def test_quoted_path_decodes_tab_escape(self):
        assert _decode_diff_path('"b/with\\ttab.py"') == "with\ttab.py"


class TestLooksLikeFindings:
    def test_recognizes_finding_headings(self):
        assert looks_like_findings(ORCHESTRATOR_SAMPLE)

    def test_empty_returns_false(self):
        assert not looks_like_findings("")

    def test_plain_prose_returns_false(self):
        assert not looks_like_findings("No structured content here.")

    def test_heading_without_severity_returns_false(self):
        assert not looks_like_findings("#### Just a generic subheader")

    def test_heading_with_severity_word_but_no_dash_returns_false(self):
        # The severity word alone isn't enough — the trailing dash separator
        # distinguishes finding headings from prose that happens to mention
        # a severity bucket in passing.
        assert not looks_like_findings("#### Critical path analysis follows")


class TestFormatSummaryEntry:
    def test_single_line_body(self):
        out = _format_summary_entry("a.py", 10, "single line")
        assert out == "- `a.py:10` — single line"

    def test_multiline_body_indents_continuation_lines(self):
        # Without the two-space indent, only the first line attaches to
        # the bullet — subsequent lines render as top-level prose and break
        # the list structure in GitHub's markdown renderer.
        out = _format_summary_entry("a.py", 10, "line one\nline two\nline three")
        assert out == "- `a.py:10` — line one\n  line two\n  line three"


if __name__ == "__main__":
    sys.exit(pytest.main([__file__, "-v"]))

"""Smoke tests for post-review-comments.py.

The script's filename uses hyphens and isn't a valid Python identifier, so we
load it via importlib. Run with: python3 -m pytest .github/scripts/
"""

import importlib.util
import json
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
preprocess_review_text = _mod.preprocess_review_text
parse_json_manifest = _mod.parse_json_manifest
_strip_ansi = _mod._strip_ansi
_decode_diff_path = _mod._decode_diff_path
_format_summary_entry = _mod._format_summary_entry
_render_fallback_summary = _mod._render_fallback_summary
_validate_manifest_item = _mod._validate_manifest_item


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
        # Tight bound: MAX_BODY_CHARS (2000) plus the 15-char truncation
        # marker "\n\n… (truncated)". A looser bound would hide a
        # regression in the truncation logic.
        assert len(findings[0]["body"]) <= _mod.MAX_BODY_CHARS + 15

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

    def test_rendered_markdown_file_line_matches(self):
        # In production, kiro-cli renders `**File:** `path:line`` through
        # its terminal markdown formatter, which eats the bold and
        # backticks when we strip the resulting ANSI. The parser must
        # accept the plain `File: path:line` form too — this is the
        # regression observed on PR #48 where the spec-form-only parser
        # extracted zero findings from real output.
        text = (
            "#### ⚠️ Important — .expect() in production code\n"
            "\n"
            "File: crates/kiro-market-core/src/hash.rs:122-126\n"
            "\n"
            "Problem: panic would abort the install.\n"
        )
        findings = parse_findings(text)
        assert len(findings) == 1
        assert findings[0]["path"] == "crates/kiro-market-core/src/hash.rs"
        assert findings[0]["line"] == 122
        assert findings[0]["severity"] == "Important"


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


class TestStripAnsi:
    def test_removes_real_esc_color_codes(self):
        # Real CSI color sequences with the actual 0x1b byte.
        text = "\x1b[38;5;252m\x1b[1m#### Critical — Title\x1b[0m"
        assert _strip_ansi(text) == "#### Critical — Title"

    def test_removes_literal_caret_bracket_form(self):
        # Kiro CLI was observed emitting escape codes as printable
        # `^[[...m` text (three ASCII characters `^`, `[`, `[` …) when
        # stdout is redirected. Parser must strip this form too.
        text = "^[[38;5;252m^[[1m#### Critical — Title^[[0m^[[0m"
        assert _strip_ansi(text) == "#### Critical — Title"

    def test_removes_non_color_csi_sequences(self):
        # Some terminal renderers inject cursor/erase sequences during
        # long-running output (e.g. `\x1b[K` erase-line, `\x1b[2A` up 2).
        text = "line one\x1b[Kline two\x1b[2A"
        assert _strip_ansi(text) == "line oneline two"

    def test_clean_text_unchanged(self):
        clean = "#### Critical — Title\n\n**File:** `a.py:10`"
        assert _strip_ansi(clean) == clean


class TestFileLineRe:
    # FILE_LINE_RE's URL-rejection lookahead is exercised indirectly via
    # parse_findings since parse_findings is the only caller — these cases
    # pin the false-positive class Kiro's own review flagged on PR #49.

    def test_url_with_port_is_not_matched_as_file_line(self):
        # Before the URL-rejection lookahead, `File: http://example.com:8080`
        # parsed as `path=http://example.com`, `line=8080`.
        text = (
            "#### ⚠️ Important — Mention of a URL in docs\n"
            "\n"
            "File: http://example.com:8080\n"
            "\n"
            "Problem: body.\n"
        )
        assert parse_findings(text) == []

    def test_https_url_is_not_matched(self):
        text = (
            "#### ⚠️ Important — HTTPS URL\n"
            "\n"
            "File: https://github.com:443\n"
            "\n"
            "Problem: body.\n"
        )
        assert parse_findings(text) == []

    def test_git_url_is_not_matched(self):
        # Non-http schemes must be rejected too — `\w+://` is the guard.
        text = (
            "#### ⚠️ Important — SSH URL\n"
            "\n"
            "File: git://example.com:22/repo\n"
            "\n"
            "Problem: body.\n"
        )
        assert parse_findings(text) == []

    def test_plain_path_with_colons_in_body_still_matches(self):
        # Regression guard: the lookahead must not reject normal paths
        # just because they contain word-characters.
        text = (
            "#### ⚠️ Important — Real file\n"
            "\n"
            "File: crates/kiro-market-core/src/hash.rs:42\n"
            "\n"
            "Problem: body.\n"
        )
        findings = parse_findings(text)
        assert len(findings) == 1
        assert findings[0]["path"] == "crates/kiro-market-core/src/hash.rs"


class TestParseJsonManifest:
    _VALID = {
        "severity": "Important",
        "agent": "code-reviewer",
        "path": "src/auth.ts",
        "line": 42,
        "title": "Missing null check",
        "body": "**Problem:** token may be undefined.",
    }

    def _wrap(self, payload_text):
        return (
            "# Code Review — PR #1\n\n"
            "## Detailed Findings\n\n"
            "(markdown review here)\n\n"
            "## Machine-Readable Findings\n\n"
            "```json\n"
            f"{payload_text}\n"
            "```\n"
        )

    def test_happy_path_extracts_findings(self):
        text = self._wrap(json.dumps([self._VALID]))
        findings = parse_json_manifest(text)
        assert findings is not None
        assert len(findings) == 1
        assert findings[0]["path"] == "src/auth.ts"
        assert findings[0]["line"] == 42
        assert findings[0]["severity"] == "Important"
        assert findings[0]["agent"] == "code-reviewer"
        # Body should include the reconstructed heading + the JSON body.
        assert findings[0]["body"].startswith("#### ⚠️ Important — Missing null check")
        assert "[source: code-reviewer]" in findings[0]["body"]
        assert "**Problem:** token may be undefined." in findings[0]["body"]

    def test_missing_section_returns_none(self):
        # No `## Machine-Readable Findings` heading → signal fallback.
        text = "# Code Review — PR #1\n\nJust prose.\n"
        assert parse_json_manifest(text) is None

    def test_section_without_array_returns_none(self):
        # No `[` after the heading → signal fallback. raw_decode can't
        # locate a JSON array if there isn't one; this can happen if the
        # orchestrator emitted the heading but crashed before producing
        # findings, or wrote a prose apology instead.
        text = (
            "# Code Review — PR #1\n\n"
            "## Machine-Readable Findings\n\n"
            "The orchestrator failed; no findings extracted.\n"
        )
        assert parse_json_manifest(text) is None

    def test_malformed_json_returns_none(self):
        # Trailing comma is a common LLM mistake; must fall back, not crash.
        text = self._wrap('[{"severity": "Important",}]')
        assert parse_json_manifest(text) is None

    def test_non_list_payload_returns_none(self):
        text = self._wrap(json.dumps({"severity": "Important"}))
        assert parse_json_manifest(text) is None

    def test_empty_list_returns_empty_list_not_none(self):
        # Crucial: an empty `[]` manifest is "orchestrator validly found
        # nothing," which must NOT fall through to regex parsing. None is
        # reserved for "manifest absent or invalid."
        text = self._wrap("[]")
        result = parse_json_manifest(text)
        assert result == []
        assert result is not None

    def test_item_missing_required_field_returns_none(self):
        # Missing `line` should invalidate the whole manifest so the
        # caller falls back to regex rather than silently dropping.
        broken = dict(self._VALID)
        del broken["line"]
        text = self._wrap(json.dumps([broken]))
        assert parse_json_manifest(text) is None

    def test_line_as_string_returns_none(self):
        broken = dict(self._VALID)
        broken["line"] = "42"  # JSON string, not int
        text = self._wrap(json.dumps([broken]))
        assert parse_json_manifest(text) is None

    def test_line_zero_is_rejected(self):
        broken = dict(self._VALID)
        broken["line"] = 0
        text = self._wrap(json.dumps([broken]))
        assert parse_json_manifest(text) is None

    def test_unfenced_output_from_kiro_cli_still_parses(self):
        # Kiro CLI's markdown renderer consumes ``` fence markers before
        # writing stdout (PR #51 confirmed this by capturing the actual
        # orchestrator output). After ANSI stripping, a block emitted as
        # ```json\n[...]\n``` arrives as `json\n[...]` with no fences.
        # raw_decode must handle this since the fence-free shape is what
        # we see in production, not the shape a markdown-preserving tool
        # would emit.
        text = (
            "# Code Review — PR #1\n\n"
            "## Machine-Readable Findings\n\n"
            "json\n"
            f"{json.dumps([self._VALID])}\n"
        )
        findings = parse_json_manifest(text)
        assert findings is not None
        assert len(findings) == 1
        assert findings[0]["path"] == "src/auth.ts"

    def test_trailing_content_after_array_is_ignored(self):
        # raw_decode parses one JSON value and stops. Any content after
        # the closing `]` (e.g. the next section heading or orchestrator
        # verdict text) must not interfere.
        text = (
            "## Machine-Readable Findings\n\n"
            f"{json.dumps([self._VALID])}\n\n"
            "## Verdict\n\n"
            "✅ LGTM — all findings addressed.\n"
        )
        findings = parse_json_manifest(text)
        assert findings is not None
        assert len(findings) == 1

    def test_body_exceeding_limit_is_truncated(self):
        item = dict(self._VALID, body="x" * 5000)
        text = self._wrap(json.dumps([item]))
        findings = parse_json_manifest(text)
        assert findings is not None
        assert findings[0]["body"].endswith("… (truncated)")
        assert len(findings[0]["body"]) <= _mod.MAX_BODY_CHARS + 15

    def test_unknown_severity_renders_without_emoji(self):
        # Forward-compatible: a new severity bucket shouldn't crash; we
        # just render without an emoji rather than reject.
        item = dict(self._VALID, severity="Blocker")
        text = self._wrap(json.dumps([item]))
        findings = parse_json_manifest(text)
        assert findings is not None
        assert findings[0]["severity"] == "Blocker"
        # Heading should still render, just without the emoji prefix.
        assert findings[0]["body"].startswith("#### Blocker — Missing null check")


class TestValidateManifestItem:
    _BASE = {
        "severity": "Important",
        "agent": "code-reviewer",
        "path": "a.py",
        "line": 1,
        "title": "t",
        "body": "b",
    }

    def test_valid_item_passes(self):
        assert _validate_manifest_item(self._BASE)

    def test_non_dict_fails(self):
        assert not _validate_manifest_item("not a dict")
        assert not _validate_manifest_item(42)
        assert not _validate_manifest_item([])

    def test_missing_field_fails(self):
        for key in self._BASE:
            bad = dict(self._BASE)
            del bad[key]
            assert not _validate_manifest_item(bad), f"missing {key} should fail"

    def test_empty_string_field_fails(self):
        for key in ("severity", "agent", "path", "title", "body"):
            bad = dict(self._BASE, **{key: ""})
            assert not _validate_manifest_item(bad), f"empty {key} should fail"

    def test_bool_line_is_rejected(self):
        # True is the dangerous case: `isinstance(True, int)` is True
        # (bool subclasses int) AND `True >= 1` is True, so without an
        # explicit bool reject, `{"line": true}` passes as line=1 and
        # fabricates an inline comment on line 1 of whatever file the
        # orchestrator pointed at. False would actually be caught by the
        # existing `< 1` check (False >= 1 is False), so it's already
        # rejected — but we pin both cases for symmetry so a future
        # refactor that removes the `< 1` check doesn't silently accept
        # `{"line": false}`.
        bad_true = dict(self._BASE, line=True)
        bad_false = dict(self._BASE, line=False)
        assert not _validate_manifest_item(bad_true)
        assert not _validate_manifest_item(bad_false)


class TestGetDiffLinesErrorNormalization:
    # get_diff_lines is the only boundary where subprocess.run can raise
    # FileNotFoundError — if the git binary is missing from the runner's
    # PATH, the exception type differs from the RuntimeError/ValueError
    # the rest of the module uses. Verify the function normalizes it to
    # RuntimeError so main()'s except clause catches every git failure
    # without a second exception type.

    def test_missing_git_binary_raises_runtime_error(self, monkeypatch):
        # `_mod.subprocess` is the same object as the top-level
        # `subprocess` module (verified: `_mod.subprocess is subprocess`
        # → True), so patching `_mod.subprocess.run` is sufficient. The
        # module uses attribute lookup (`subprocess.run(...)`) at call
        # time, not at import time, so this patch takes effect even
        # though the import happened during module load.
        def fake_run(*args, **kwargs):
            raise FileNotFoundError(
                "[Errno 2] No such file or directory: 'git'"
            )

        monkeypatch.setattr(_mod.subprocess, "run", fake_run)

        with pytest.raises(RuntimeError, match="git binary not found"):
            _mod.get_diff_lines("main")

    def test_empty_base_ref_still_raises_value_error(self):
        # Regression guard: normalizing FileNotFoundError must not
        # accidentally swallow the ValueError for an empty base_ref,
        # which is a distinct misconfiguration.
        with pytest.raises(ValueError, match="base_ref is empty"):
            _mod.get_diff_lines("")


class TestGhApiPostJson:
    # _gh_api_post_json wraps subprocess.run to normalize missing-gh
    # failures into a (rc, stdout, stderr) triple instead of a raised
    # FileNotFoundError. Without this, call sites in main() (the review
    # POST at line ~657 and the gh_issue_comment fallback) would need
    # exception-handling duplicated at each site, and a missing `gh`
    # would escape as an uncaught Python traceback.

    def test_missing_gh_binary_returns_minus_one(self, monkeypatch):
        def fake_run(*args, **kwargs):
            raise FileNotFoundError(
                "[Errno 2] No such file or directory: 'gh'"
            )

        monkeypatch.setattr(_mod.subprocess, "run", fake_run)

        rc, out, err = _mod._gh_api_post_json("/repos/x/y/issues/1/comments", {})
        assert rc == -1
        assert out == ""
        assert "gh binary not found" in err

    def test_subprocess_success_passes_through(self, monkeypatch):
        # Happy path: subprocess returns normally, _gh_api_post_json
        # forwards returncode/stdout/stderr unchanged.
        class FakeResult:
            returncode = 0
            stdout = '{"id":42}'
            stderr = ""

        monkeypatch.setattr(_mod.subprocess, "run", lambda *a, **kw: FakeResult())

        rc, out, err = _mod._gh_api_post_json("/repos/x/y/issues/1/comments", {"body": "hi"})
        assert rc == 0
        assert out == '{"id":42}'
        assert err == ""


class TestRenderFallbackSummary:
    def test_includes_headers_count_and_entries(self):
        findings = [
            {"path": "a.py", "line": 10, "body": "first finding"},
            {"path": "b.py", "line": 20, "body": "second finding"},
        ]
        out = _render_fallback_summary(findings)
        # Top-level heading signals this as the Kiro review output to
        # readers who see it in a PR comment thread mixed with other bots.
        assert "## Kiro Review Summary" in out
        # Subsection header delimits the finding list.
        assert "### Findings" in out
        assert "Found **2** findings" in out
        assert "- `a.py:10` — first finding" in out
        assert "- `b.py:20` — second finding" in out

    def test_empty_list_renders_coherently(self):
        # Unit-level contract: main() never actually calls
        # _render_fallback_summary with an empty list (the no-findings
        # branch fires earlier and returns before reaching the
        # get_diff_lines exception path that renders the fallback). This
        # test is a contract check on the function in isolation — it
        # should produce coherent output if ever called with [], rather
        # than crashing or dropping the heading.
        out = _render_fallback_summary([])
        assert "Found **0** findings" in out
        assert "## Kiro Review Summary" in out


class TestPreprocessReviewText:
    def test_trims_kiro_cli_narration_before_code_review_h1(self):
        # Realistic kiro-cli transcript: the CLI narrates tool calls
        # before producing the actual review. Every heading is wrapped in
        # the literal caret-bracket ANSI form. preprocess_review_text must
        # strip the color codes and drop everything before the `# Code
        # Review` H1 so the parser never sees the narration.
        raw = (
            "^[[38;5;141m> ^[[0mI'll start by gathering code context.^[[0m\n"
            "I will run the following command: ^[[38;5;141mgit log --oneline -5^[[0m\n"
            "abc1234 some commit\n"
            "\n"
            "^[[38;5;252m^[[1m# Code Review — PR #42^[[0m\n"
            "\n"
            "^[[38;5;252m^[[1m## Holistic Assessment^[[0m\n"
            "\n"
            "^[[38;5;252m^[[1m#### ❌ Critical — Bug [source: code-reviewer]^[[0m\n"
            "\n"
            "**File:** `a.py:10`\n"
            "\n"
            "**Problem:** body.\n"
        )
        out = preprocess_review_text(raw)
        assert out.startswith("# Code Review — PR #42")
        assert "I'll start by gathering" not in out
        assert "git log" not in out
        # Findings should now parse after preprocessing.
        findings = parse_findings(out)
        assert len(findings) == 1
        assert findings[0]["severity"] == "Critical"
        assert findings[0]["path"] == "a.py"

    def test_returns_stripped_text_when_no_review_marker(self):
        # If the orchestrator crashed before producing its final report,
        # there's no `# Code Review` marker. Return the ANSI-stripped text
        # so the fallback path can still post something — silent drop
        # would be worse than a noisy comment.
        raw = "^[[38;5;141m> ^[[0mI'll start.^[[0m\nbut then I crashed"
        out = preprocess_review_text(raw)
        assert out == "> I'll start.\nbut then I crashed"

    def test_is_idempotent_on_clean_input(self):
        clean = (
            "# Code Review — PR #1\n\n"
            "#### ❌ Critical — X [source: y]\n\n"
            "**File:** `a.py:1`\n\n"
            "**Problem:** body.\n"
        )
        assert preprocess_review_text(clean) == clean


if __name__ == "__main__":
    sys.exit(pytest.main([__file__, "-v"]))

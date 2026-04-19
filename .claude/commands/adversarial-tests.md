---
description: Propose rstest cases covering the adversarial branches of a function (symlinks, traversal, concurrency, malformed input)
---

Argument: `$ARGUMENTS` — a file path, `file:function`, or `file:line` span identifying the code under scrutiny.

Read the target code. Enumerate its adversarial branches — the input shapes that a marketplace author or attacker could craft to reach edge-case behavior. Propose `rstest`-style test cases that exercise each branch. Do NOT write tests to disk unless the user explicitly asks — your job is to surface the gap list and the proposed cases.

## Adversarial branch catalogue (check each against the target)

**Path / filesystem inputs**
- `..` component traversal
- Backslash traversal on Unix (`sub\..\..\x`) — Rust `Path::components()` doesn't split on `\` on Unix
- Symlinks (file and directory)
- Hardlinks pointing outside the source tree (`metadata.nlink() > 1`)
- Windows reparse points / junctions (if `cfg(windows)`)
- Leading `/`, leading `~`, absolute paths where relative was expected
- NUL bytes (`\0`) — panic on OS calls
- Case collisions on case-insensitive filesystems (`Foo` vs `foo`)
- Very long paths (>255 bytes), paths with trailing dots/spaces (Windows)
- Non-UTF8 paths (OsStr, `OsString::from_encoded_bytes_unchecked`)

**Name / identifier inputs**
- Empty string, whitespace-only
- Leading `-` (argument injection)
- ASCII control characters (`\x01`..`\x1f`)
- RTL override codepoints (U+202E), zero-width joiners, combining marks
- Windows reserved names (CON, PRN, AUX, NUL, COM1–9, LPT1–9, with and without extensions)
- Mixed-script homoglyphs (Latin `a` vs Cyrillic `а`)
- Unnormalized Unicode (NFC vs NFD)

**Concurrency**
- Two concurrent `add` calls with the same derived name
- Reader during writer (cache prune while install running)
- Lock contention — same path acquired by two processes
- TOCTOU between `exists()` / `stat()` and the subsequent operation

**Subprocess / external**
- `git` unreachable (no network, bad URL)
- `git` returns non-zero with arbitrary stderr
- Credential helper prompts (simulate via `GIT_ASKPASS=/bin/false`)
- Signal interrupt (Ctrl-C mid-clone) — partial state cleanup

**Deserialization**
- Malformed JSON / YAML — missing field, wrong type, nested beyond depth limit
- Unicode BOM, UTF-16
- Duplicate keys, extra fields (is `#[serde(deny_unknown_fields)]` set?)
- Huge inputs (10MB JSON, deeply nested)

**Supply chain**
- Manifest declares a SHA that doesn't match the clone
- `http://` URL (plaintext)
- Redirect from HTTPS to HTTP mid-clone (curl backend)
- 1-character SHA prefix (`"a"`)

## Output format

```markdown
## Adversarial test gaps — <target>

### Branches currently untested
- **<branch name>**: <description>. Expected behavior: <error type or invariant>.

### Proposed rstest cases

<!-- Use #[rstest] + #[case] where possible. Match the project's existing test
     style (look at neighboring test files for helper / fixture patterns). -->

```rust
#[rstest]
#[case::unix_backslash_traversal("sub\\..\\..\\etc\\passwd")]
#[case::nul_byte("foo\0bar")]
#[case::leading_dash("-rf")]
fn validate_name_rejects_hostile(#[case] name: &str) {
    assert!(validate_name(name).is_err(), "expected reject for {name:?}");
}
```

### Low-value cases (excluded and why)
- <cases that don't apply to this function and the reasoning>
```

Keep proposed test bodies short. Favor `#[rstest] #[case]` over repeated functions when the assertion is uniform. Cite the line in the target where each branch lives (or where the branch is *missing* — that's the finding).

If the target has no meaningful adversarial surface (e.g. a pure `Display` impl), say so and stop.

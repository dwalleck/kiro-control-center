//! `cargo xtask comment-lint` — flag PR / issue / rivets-ID references inside
//! source-file comments.
//!
//! CLAUDE.md mandates that "current task / fix / callers" references belong in
//! the PR description, not in committed comments — references like
//! `kiro-uphh`, `PR #119`, or `issue #66` rot the moment the referenced work
//! is closed, leaving readers to grep `.rivets/issues.jsonl` (often archived)
//! or GitHub for stale context. This gate scans `//` line comments under
//! `crates/` and flags those patterns so a future violation can't slip into
//! the tree.
//!
//! Block comments (`/* ... */`) and HTML/Svelte comments are NOT scanned —
//! the bulk of comment density is line-comment style and adding block-comment
//! tracking would multiply complexity without meaningful coverage gain.
//! Strings literals are not scanned: a `"kiro-XXXX"` literal in production
//! code is rare and almost always intentional (e.g. a test fixture name).
//!
//! [`ALLOWED_SITES`] documents deliberate, long-lived rationale references
//! (gate descriptions citing the originating PR, etc.) the same way
//! `plan_lint::ALLOWED_SITES` documents zero-tolerance panic-point exceptions.
//! Adding to the list requires a code change reviewed in PR.

use std::collections::HashSet;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

/// A line in a source file that contains a forbidden comment reference.
#[derive(Debug, PartialEq, Eq, Clone)]
struct Finding {
    /// Repo-relative path, forward-slash separated.
    path: String,
    line: u32,
    /// The matched pattern label (`kiro-id`, `pr-ref`, `issue-ref`).
    kind: &'static str,
    /// The exact substring matched, for the reviewer.
    matched: String,
}

/// A scanned source-tree extension.
#[derive(Debug, Clone, Copy)]
struct Extension {
    suffix: &'static str,
}

/// Extensions the gate scans. Only file types that carry `//` line comments;
/// `.svelte` files include script blocks with `//` comments so they're in.
const SCANNED_EXTENSIONS: &[Extension] = &[
    Extension { suffix: ".rs" },
    Extension { suffix: ".ts" },
    Extension { suffix: ".tsx" },
    Extension { suffix: ".js" },
    Extension { suffix: ".svelte" },
];

/// Directories under the workspace root the gate walks.
const SCANNED_ROOTS: &[&str] = &["crates", "xtask"];

/// Directory basenames that are pruned at any depth during the walk.
const PRUNED_DIRS: &[&str] = &["target", "node_modules", ".git"];

/// Reviewer-agent names that, when they appear in a `//` comment, signal a
/// "closes <agent> finding" / "per <agent> review" smell. The smell shape:
/// the comment attributes the change to the reviewer process rather than
/// describing the invariant the code enforces. CLAUDE.md's rule treats
/// these the same as PR/issue references — they rot once the reviewer pass
/// is forgotten and leave readers grepping transcripts that don't exist.
///
/// All entries are kebab-case multi-word (no English-word collisions).
/// String-literal uses of these names (e.g. `pn("code-reviewer")` in test
/// fixtures) are not flagged because the scanner is `//`-comment-scoped.
const REVIEWER_AGENT_NAMES: &[&str] = &[
    "code-reviewer",
    "code-simplifier",
    "comment-analyzer",
    "pr-test-analyzer",
    "silent-failure-hunter",
    "type-design-analyzer",
    "marketplace-security-reviewer",
    "tauri-ipc-auditor",
    "plugin-validator",
    "skill-reviewer",
    "gemini-code-assist",
];

/// Process-reference keywords / phrases. Same smell class as PR/issue/
/// reviewer-agent references: the comment attributes the code to a
/// process artifact (plan amendment, slice number from the budgeted-plan)
/// rather than describing the invariant the code enforces.
///
/// Cases:
/// - `amendment` — bare word, case-insensitive. References plan
///   amendments like "Per A1 amendment", "amendment A2". A real English
///   word, but every observed use in this codebase is a process
///   reference; legitimate prose uses are rare enough that allowlist
///   bookkeeping is cheaper than under-detecting.
/// - `per A<digits>` — shorthand attribution like "Per A1: …" naming
///   a plan amendment by its identifier.
const PROCESS_REF_KEYWORDS: &[&str] = &["amendment"];

/// Files (matched by basename) that the gate skips:
///
/// - `bindings.ts` — regenerated from Rust via specta; manual cleanup there
///   would be erased on every regen.
/// - `comment_lint.rs` — the gate's own module necessarily references the
///   patterns it detects (docstring examples, unit-test fixtures).
/// - `plan_lint.rs` — gate-query rationale anchored to originating PRs
///   (e.g. "PR #64 was the failure that drove gate-4"); each reference is
///   the documented historical justification for a long-lived query shape.
const SKIPPED_FILES: &[&str] = &["bindings.ts", "comment_lint.rs", "plan_lint.rs"];

/// A `(path, line, reason)` exception. Same shape and discipline as
/// `plan_lint::ALLOWED_SITES`: keep the audit trail in source so adding a
/// row shows up in `git blame`.
struct AllowedSite {
    path: &'static str,
    line: u32,
    #[expect(
        dead_code,
        reason = "human-only documentation; reviewer audit, not a runtime field"
    )]
    reason: &'static str,
}

/// Historical rationale comments — PR / issue / rivets-ID references that
/// pre-date the gate's introduction and which the project has not yet
/// retroactively cleaned. New violations land as findings; cleanup of the
/// baseline shrinks this list. Line numbers shift with edits; the runner
/// reports stale entries (the same way `plan_lint`'s allowlist does)
/// forcing coordinated updates.
const LEGACY_BASELINE_REASON: &str =
    "Legacy baseline at gate introduction. Cleanup tracked in follow-up; do not extend.";

/// String-literal reference to a domain plugin / agent name (e.g. a
/// fixture `pn("code-reviewer")` whose surrounding comment quotes the
/// name in prose). Not a reviewer-attribution smell — the gate's
/// word-boundary check sees a bare token, but the comment treats it
/// as a domain identifier.
const FIXTURE_NAME_REASON: &str =
    "Fixture plugin/agent name referenced in comment (not a reviewer attribution).";

const ALLOWED_SITES: &[AllowedSite] = &[
    AllowedSite {
        path: "crates/kiro-control-center/src-tauri/src/commands/agents.rs",
        line: 550,
        reason: LEGACY_BASELINE_REASON,
    },
    AllowedSite {
        path: "crates/kiro-control-center/src-tauri/src/commands/agents.rs",
        line: 599,
        reason: LEGACY_BASELINE_REASON,
    },
    AllowedSite {
        path: "crates/kiro-control-center/src-tauri/src/commands/browse.rs",
        line: 179,
        reason: LEGACY_BASELINE_REASON,
    },
    AllowedSite {
        path: "crates/kiro-control-center/src-tauri/src/commands/browse.rs",
        line: 894,
        reason: LEGACY_BASELINE_REASON,
    },
    AllowedSite {
        path: "crates/kiro-control-center/src-tauri/src/commands/browse.rs",
        line: 943,
        reason: LEGACY_BASELINE_REASON,
    },
    AllowedSite {
        path: "crates/kiro-control-center/src-tauri/src/commands/steering.rs",
        line: 73,
        reason: LEGACY_BASELINE_REASON,
    },
    AllowedSite {
        path: "crates/kiro-control-center/src-tauri/src/commands/steering.rs",
        line: 568,
        reason: LEGACY_BASELINE_REASON,
    },
    AllowedSite {
        path: "crates/kiro-control-center/src-tauri/src/commands/steering.rs",
        line: 615,
        reason: LEGACY_BASELINE_REASON,
    },
    AllowedSite {
        path: "crates/kiro-control-center/src-tauri/src/commands/steering.rs",
        line: 721,
        reason: LEGACY_BASELINE_REASON,
    },
    AllowedSite {
        path: "crates/kiro-control-center/src-tauri/src/error.rs",
        line: 84,
        reason: LEGACY_BASELINE_REASON,
    },
    AllowedSite {
        path: "crates/kiro-control-center/src-tauri/src/error.rs",
        line: 321,
        reason: LEGACY_BASELINE_REASON,
    },
    AllowedSite {
        path: "crates/kiro-control-center/src-tauri/src/lib.rs",
        line: 112,
        reason: LEGACY_BASELINE_REASON,
    },
    AllowedSite {
        path: "crates/kiro-control-center/src-tauri/src/lib.rs",
        line: 138,
        reason: LEGACY_BASELINE_REASON,
    },
    AllowedSite {
        path: "crates/kiro-control-center/src/lib/agent-name.test.ts",
        line: 65,
        reason: LEGACY_BASELINE_REASON,
    },
    AllowedSite {
        path: "crates/kiro-control-center/src/lib/agent-name.ts",
        line: 8,
        reason: LEGACY_BASELINE_REASON,
    },
    AllowedSite {
        path: "crates/kiro-control-center/src/lib/agent-name.ts",
        line: 27,
        reason: LEGACY_BASELINE_REASON,
    },
    AllowedSite {
        path: "crates/kiro-control-center/src/lib/components/BrowseTab.svelte",
        line: 696,
        reason: LEGACY_BASELINE_REASON,
    },
    AllowedSite {
        path: "crates/kiro-control-center/src/lib/components/CustomizeDrawer.svelte",
        line: 18,
        reason: LEGACY_BASELINE_REASON,
    },
    AllowedSite {
        path: "crates/kiro-control-center/src/lib/components/editor/IdentityPanel.svelte",
        line: 17,
        reason: LEGACY_BASELINE_REASON,
    },
    AllowedSite {
        path: "crates/kiro-control-center/src/lib/format.test.ts",
        line: 367,
        reason: LEGACY_BASELINE_REASON,
    },
    AllowedSite {
        path: "crates/kiro-market-core/src/project.rs",
        line: 1419,
        reason: LEGACY_BASELINE_REASON,
    },
    AllowedSite {
        path: "crates/kiro-market-core/src/project.rs",
        line: 1582,
        reason: LEGACY_BASELINE_REASON,
    },
    AllowedSite {
        path: "crates/kiro-market-core/src/project.rs",
        line: 1863,
        reason: LEGACY_BASELINE_REASON,
    },
    AllowedSite {
        path: "crates/kiro-market-core/src/project.rs",
        line: 10016,
        reason: LEGACY_BASELINE_REASON,
    },
    AllowedSite {
        path: "crates/kiro-market-core/src/project.rs",
        line: 10891,
        reason: LEGACY_BASELINE_REASON,
    },
    AllowedSite {
        path: "crates/kiro-market-core/src/service/browse.rs",
        line: 166,
        reason: LEGACY_BASELINE_REASON,
    },
    AllowedSite {
        path: "crates/kiro-market-core/src/service/mod.rs",
        line: 3690,
        reason: LEGACY_BASELINE_REASON,
    },
    AllowedSite {
        path: "crates/kiro-market-core/src/service/mod.rs",
        line: 4718,
        reason: LEGACY_BASELINE_REASON,
    },
    AllowedSite {
        path: "crates/kiro-market-core/src/service/mod.rs",
        line: 5051,
        reason: LEGACY_BASELINE_REASON,
    },
    AllowedSite {
        path: "crates/kiro-market-core/src/service/mod.rs",
        line: 5101,
        reason: LEGACY_BASELINE_REASON,
    },
    AllowedSite {
        path: "crates/kiro-market-core/src/service/mod.rs",
        line: 8768,
        reason: LEGACY_BASELINE_REASON,
    },
    AllowedSite {
        path: "crates/kiro-market/src/commands/install.rs",
        line: 41,
        reason: LEGACY_BASELINE_REASON,
    },
    // ── reviewer-agent legacy baseline (added with the reviewer-agent detector) ──
    AllowedSite {
        path: "crates/kiro-control-center/src/lib/components/AgentEditor.svelte",
        line: 140,
        reason: LEGACY_BASELINE_REASON,
    },
    AllowedSite {
        path: "crates/kiro-market-core/src/agent/parse_native.rs",
        line: 501,
        reason: LEGACY_BASELINE_REASON,
    },
    AllowedSite {
        path: "crates/kiro-market-core/src/kiro_settings.rs",
        line: 1032,
        reason: LEGACY_BASELINE_REASON,
    },
    AllowedSite {
        path: "crates/kiro-market-core/src/project.rs",
        line: 3859,
        reason: LEGACY_BASELINE_REASON,
    },
    AllowedSite {
        path: "crates/kiro-market-core/src/project.rs",
        line: 4185,
        reason: LEGACY_BASELINE_REASON,
    },
    AllowedSite {
        path: "crates/kiro-market-core/src/project.rs",
        line: 4595,
        reason: LEGACY_BASELINE_REASON,
    },
    AllowedSite {
        path: "crates/kiro-market-core/src/project.rs",
        line: 4767,
        reason: LEGACY_BASELINE_REASON,
    },
    AllowedSite {
        path: "crates/kiro-market-core/src/project.rs",
        line: 6283,
        reason: LEGACY_BASELINE_REASON,
    },
    AllowedSite {
        path: "crates/kiro-market-core/src/project.rs",
        line: 6858,
        reason: FIXTURE_NAME_REASON,
    },
    AllowedSite {
        path: "crates/kiro-market-core/src/project.rs",
        line: 6885,
        reason: FIXTURE_NAME_REASON,
    },
    AllowedSite {
        path: "crates/kiro-market-core/src/project.rs",
        line: 8914,
        reason: LEGACY_BASELINE_REASON,
    },
    AllowedSite {
        path: "crates/kiro-market-core/src/project.rs",
        line: 8926,
        reason: LEGACY_BASELINE_REASON,
    },
    AllowedSite {
        path: "crates/kiro-market-core/src/project.rs",
        line: 8995,
        reason: LEGACY_BASELINE_REASON,
    },
    AllowedSite {
        path: "crates/kiro-market-core/src/project.rs",
        line: 9225,
        reason: LEGACY_BASELINE_REASON,
    },
    AllowedSite {
        path: "crates/kiro-market-core/src/project.rs",
        line: 9625,
        reason: LEGACY_BASELINE_REASON,
    },
    AllowedSite {
        path: "crates/kiro-market-core/src/service/browse.rs",
        line: 1898,
        reason: LEGACY_BASELINE_REASON,
    },
    // ── process-ref legacy baseline (added with the process-ref detector) ──
    AllowedSite {
        path: "crates/kiro-control-center/src/lib/components/editor/MarketplaceSavePromptModal.svelte",
        line: 19,
        reason: LEGACY_BASELINE_REASON,
    },
    AllowedSite {
        path: "crates/kiro-control-center/tests/e2e/agents.spec.ts",
        line: 303,
        reason: LEGACY_BASELINE_REASON,
    },
    AllowedSite {
        path: "crates/kiro-market-core/src/service/mod.rs",
        line: 7063,
        reason: LEGACY_BASELINE_REASON,
    },
    AllowedSite {
        path: "crates/kiro-market-core/src/steering/types.rs",
        line: 235,
        reason: LEGACY_BASELINE_REASON,
    },
];

pub fn run(args: impl Iterator<Item = String>) -> Result<usize> {
    let opts = Options::parse(args)?;

    let mut findings: Vec<Finding> = Vec::new();
    for root in SCANNED_ROOTS {
        let dir = opts.workspace.join(root);
        if !dir.is_dir() {
            continue;
        }
        scan_dir(&dir, &opts.workspace, &mut findings)?;
    }
    findings.sort_by(|a, b| a.path.cmp(&b.path).then(a.line.cmp(&b.line)));

    let mut matched_allowlist: HashSet<usize> = HashSet::new();
    let (allowed, real): (Vec<_>, Vec<_>) = findings.into_iter().partition(|f| {
        if let Some(idx) = find_allowlist_index(ALLOWED_SITES, f) {
            matched_allowlist.insert(idx);
            true
        } else {
            false
        }
    });

    if real.is_empty() {
        if allowed.is_empty() {
            println!("comment-lint OK");
        } else {
            println!(
                "comment-lint OK ({} allowlisted exception{})",
                allowed.len(),
                if allowed.len() == 1 { "" } else { "s" }
            );
        }
    } else {
        println!(
            "comment-lint — PR / issue / rivets-ID reference in source comment ({} finding{})",
            real.len(),
            if real.len() == 1 { "" } else { "s" },
        );
        for f in &real {
            println!("    {}:{}", f.path, f.line);
            println!("        [{}] {}", f.kind, f.matched);
        }
    }

    let stale: Vec<&AllowedSite> = ALLOWED_SITES
        .iter()
        .enumerate()
        .filter(|(i, _)| !matched_allowlist.contains(i))
        .map(|(_, s)| s)
        .collect();
    let mut total = real.len();
    if !stale.is_empty() {
        total += stale.len();
        println!(
            "stale-allowlist — {} entr{} in ALLOWED_SITES matched no finding",
            stale.len(),
            if stale.len() == 1 { "y" } else { "ies" }
        );
        for site in &stale {
            println!("    {}:{}", site.path, site.line);
        }
    }

    if total > 0 {
        eprintln!("comment-lint found {total} violation(s)");
    }
    Ok(total)
}

struct Options {
    workspace: PathBuf,
}

impl Options {
    fn parse(mut args: impl Iterator<Item = String>) -> Result<Self> {
        let mut workspace = None;
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--workspace" => {
                    workspace = Some(PathBuf::from(
                        args.next().context("--workspace needs a path")?,
                    ));
                }
                "-h" | "--help" => {
                    print_help();
                    std::process::exit(0);
                }
                other => bail!("unknown comment-lint flag: {other}"),
            }
        }
        let workspace = workspace
            .or_else(|| env::var_os("CLAUDE_PROJECT_DIR").map(PathBuf::from))
            .or_else(|| env::current_dir().ok())
            .context("could not determine workspace root")?;
        Ok(Self { workspace })
    }
}

fn print_help() {
    println!(
        "cargo xtask comment-lint — flag PR / issue / rivets-ID refs in `//` comments

USAGE:
    cargo xtask comment-lint [--workspace <PATH>]

OPTIONS:
    --workspace <PATH>   workspace root (default: $CLAUDE_PROJECT_DIR or cwd)

EXIT CODES:
    0  no findings
    1  one or more comments contain forbidden references (CI gate fails)
    2  internal error (file walk failed, etc.)"
    );
}

/// Recursively walk `dir`, scanning files whose extensions are in
/// [`SCANNED_EXTENSIONS`] and pruning the directories named in
/// [`PRUNED_DIRS`].
fn scan_dir(dir: &Path, workspace: &Path, out: &mut Vec<Finding>) -> Result<()> {
    let entries =
        fs::read_dir(dir).with_context(|| format!("reading directory {}", dir.display()))?;
    for entry in entries {
        let entry = entry.with_context(|| format!("entry under {}", dir.display()))?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .with_context(|| format!("file type of {}", path.display()))?;
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if file_type.is_dir() {
            if PRUNED_DIRS.contains(&name) {
                continue;
            }
            scan_dir(&path, workspace, out)?;
        } else if file_type.is_file() && is_scanned_file(name) {
            scan_file(&path, workspace, out);
        }
    }
    Ok(())
}

fn is_scanned_file(name: &str) -> bool {
    if SKIPPED_FILES.contains(&name) {
        return false;
    }
    SCANNED_EXTENSIONS
        .iter()
        .any(|e| name.len() > e.suffix.len() && name.ends_with(e.suffix))
}

fn scan_file(path: &Path, workspace: &Path, out: &mut Vec<Finding>) {
    let text = match fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => {
            // Non-UTF-8 source isn't a lint target — log and continue.
            eprintln!("comment-lint: skipping {}: {e}", path.display());
            return;
        }
    };
    let rel = relative_path(path, workspace);
    for (idx, line) in text.lines().enumerate() {
        let Some(comment) = extract_line_comment(line) else {
            continue;
        };
        // A source file with >4 billion lines is implausible; saturating
        // truncation keeps the gate honest if it somehow happens.
        let line_no = u32::try_from(idx + 1).unwrap_or(u32::MAX);
        for f in scan_comment(&rel, line_no, comment) {
            out.push(f);
        }
    }
}

/// Convert an absolute (or relative-to-cwd) path into a workspace-relative
/// path with `/` separators so findings are stable across OSes and CI logs.
fn relative_path(path: &Path, workspace: &Path) -> String {
    let rel = path.strip_prefix(workspace).unwrap_or(path);
    rel.to_string_lossy().replace('\\', "/")
}

/// Extract the `//`-comment substring from a source line, or `None` if the
/// line has no line-comment. Returns the substring *after* the `//`.
///
/// Heuristic for skipping `//` that appears inside string literals: a naive
/// rfind/find would flag `let url = "https://example.com";` as a comment.
/// We scan left-to-right, toggling a "string-state" flag at unescaped `"`
/// and skipping `//` that lives inside the active string.
///
/// This is intentionally simple: it does NOT handle Rust's raw strings
/// (`r#"..."#`), char literals (`'/'`), nested string escapes beyond
/// backslash, or TypeScript template strings. False negatives in those
/// shapes are acceptable — the gate flags only what it confidently
/// identifies as comment text, accepting that a future violation tucked
/// inside a raw string slips through.
fn extract_line_comment(line: &str) -> Option<&str> {
    let bytes = line.as_bytes();
    let mut in_string = false;
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if in_string {
            if c == b'\\' && i + 1 < bytes.len() {
                i += 2;
                continue;
            }
            if c == b'"' {
                in_string = false;
            }
            i += 1;
            continue;
        }
        match c {
            b'"' => in_string = true,
            b'/' if i + 1 < bytes.len() && bytes[i + 1] == b'/' => {
                return Some(&line[i + 2..]);
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Scan a single comment substring for all forbidden patterns. Returns a
/// vector because a single comment line could carry multiple references
/// (e.g. "kiro-uphh / PR #119") and the user benefits from seeing each one.
fn scan_comment(path: &str, line_no: u32, comment: &str) -> Vec<Finding> {
    let mut out = Vec::new();
    if let Some(m) = find_kiro_id(comment) {
        out.push(Finding {
            path: path.to_string(),
            line: line_no,
            kind: "kiro-id",
            matched: m.to_string(),
        });
    }
    if let Some(m) = find_pr_ref(comment) {
        out.push(Finding {
            path: path.to_string(),
            line: line_no,
            kind: "pr-ref",
            matched: m.to_string(),
        });
    }
    if let Some(m) = find_issue_ref(comment) {
        out.push(Finding {
            path: path.to_string(),
            line: line_no,
            kind: "issue-ref",
            matched: m.to_string(),
        });
    }
    if let Some(m) = find_reviewer_agent(comment) {
        out.push(Finding {
            path: path.to_string(),
            line: line_no,
            kind: "reviewer-agent",
            matched: m.to_string(),
        });
    }
    if let Some(m) = find_process_ref(comment) {
        out.push(Finding {
            path: path.to_string(),
            line: line_no,
            kind: "process-ref",
            matched: m.to_string(),
        });
    }
    out
}

/// Find a process-reference phrase: bare `amendment` (case-insensitive)
/// or `per A<digits>` shorthand. Both attribute code to a planning
/// artifact rather than describing the invariant the code enforces —
/// the comment's reader has to consult the plan doc to understand
/// what changed, and the plan doc rots once the slice ships.
fn find_process_ref(text: &str) -> Option<&str> {
    if let Some(m) = find_keyword(text, PROCESS_REF_KEYWORDS) {
        return Some(m);
    }
    find_per_amendment_shorthand(text)
}

/// Generic case-insensitive whole-word match against a list of keywords.
/// Returns the substring of `text` that matched (preserving original
/// case for the reviewer's eyes).
fn find_keyword<'a>(text: &'a str, keywords: &[&str]) -> Option<&'a str> {
    let lower = text.to_ascii_lowercase();
    let lb = lower.as_bytes();
    for kw in keywords {
        let kb = kw.as_bytes();
        let mut i = 0;
        while i + kb.len() <= lb.len() {
            if &lb[i..i + kb.len()] == kb {
                let left_ok = i == 0 || !lb[i - 1].is_ascii_alphanumeric();
                let right_idx = i + kb.len();
                let right_ok = right_idx >= lb.len() || !lb[right_idx].is_ascii_alphanumeric();
                if left_ok && right_ok {
                    return Some(&text[i..right_idx]);
                }
            }
            i += 1;
        }
    }
    None
}

/// Match `per A<digits>` (case-insensitive). The `A` is uppercase in
/// the canonical slice convention (`A1`, `A2`), but contributors type
/// it in various cases — flag all forms.
fn find_per_amendment_shorthand(text: &str) -> Option<&str> {
    let lower = text.to_ascii_lowercase();
    let lb = lower.as_bytes();
    let needle = b"per ";
    let mut i = 0;
    while i + needle.len() < lb.len() {
        if &lb[i..i + needle.len()] == needle {
            let left_ok = i == 0 || !lb[i - 1].is_ascii_alphanumeric();
            let mut j = i + needle.len();
            // Skip extra spaces after "per".
            while j < lb.len() && lb[j] == b' ' {
                j += 1;
            }
            if left_ok && j < lb.len() && lb[j] == b'a' {
                let mut k = j + 1;
                let digits_start = k;
                while k < lb.len() && lb[k].is_ascii_digit() {
                    k += 1;
                }
                if k > digits_start {
                    // At least one digit consumed. Right-boundary
                    // check so `per A123x` does not match (the `x`
                    // would change the semantic).
                    let right_ok = k >= lb.len() || !lb[k].is_ascii_alphanumeric();
                    if right_ok {
                        return Some(&text[i..k]);
                    }
                }
            }
        }
        i += 1;
    }
    None
}

/// Find a known reviewer-agent name in `text`. Word-boundary anchored on
/// both sides so `kiro-code-reviewer-v2` (a legitimate plugin name) does
/// NOT match `code-reviewer`. Returns the first match.
fn find_reviewer_agent(text: &str) -> Option<&str> {
    let bytes = text.as_bytes();
    for name in REVIEWER_AGENT_NAMES {
        let needle = name.as_bytes();
        let mut i = 0;
        while i + needle.len() <= bytes.len() {
            if &bytes[i..i + needle.len()] == needle {
                let left_ok = i == 0 || !is_id_continuation(bytes[i - 1]);
                let right_idx = i + needle.len();
                let right_ok = right_idx >= bytes.len() || !is_id_continuation(bytes[right_idx]);
                if left_ok && right_ok {
                    return Some(&text[i..right_idx]);
                }
            }
            i += 1;
        }
    }
    None
}

/// Find a `kiro-XXXX` rivets ID (exactly 4 lowercase alphanumeric chars).
/// Word-boundary aware: `kiro-uphh` matches; `mykiro-uphh` and
/// `kiro-uphhx` do not.
fn find_kiro_id(text: &str) -> Option<&str> {
    let bytes = text.as_bytes();
    let needle = b"kiro-";
    let mut i = 0;
    while i + needle.len() + 4 <= bytes.len() {
        if &bytes[i..i + needle.len()] == needle {
            // Left boundary: previous byte must not be alphanumeric / -.
            let left_ok = i == 0 || !is_id_continuation(bytes[i - 1]);
            // Tail must be exactly 4 lowercase alphanumeric.
            let tail_start = i + needle.len();
            let tail = &bytes[tail_start..tail_start + 4];
            let tail_ok = tail
                .iter()
                .all(|&c| c.is_ascii_lowercase() || c.is_ascii_digit());
            // Right boundary: byte after the 4-char tail must NOT be alphanumeric.
            let right_idx = tail_start + 4;
            let right_ok = right_idx >= bytes.len() || !bytes[right_idx].is_ascii_alphanumeric();
            if left_ok && tail_ok && right_ok {
                return Some(&text[i..right_idx]);
            }
        }
        i += 1;
    }
    None
}

fn is_id_continuation(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'-' || b == b'_'
}

/// Find `PR #N` / `pr#NNN` / `Pr #42` (case-insensitive). `N` must be one
/// or more digits. The `#` is mandatory — bare `PR 123` (rare) is not
/// flagged.
fn find_pr_ref(text: &str) -> Option<&str> {
    find_keyword_hash_digits(text, "pr")
}

/// Find `issue #N`. Same shape as [`find_pr_ref`].
fn find_issue_ref(text: &str) -> Option<&str> {
    find_keyword_hash_digits(text, "issue")
}

/// Scan for `<keyword>[whitespace]#<digits>` case-insensitively, anchored
/// on a word boundary at the keyword. Returns the matched substring.
fn find_keyword_hash_digits<'a>(text: &'a str, keyword: &str) -> Option<&'a str> {
    let lower = text.to_ascii_lowercase();
    let lb = lower.as_bytes();
    let kb = keyword.as_bytes();
    let mut i = 0;
    while i + kb.len() <= lb.len() {
        if &lb[i..i + kb.len()] == kb {
            let left_ok = i == 0 || !lb[i - 1].is_ascii_alphanumeric();
            let after = i + kb.len();
            // Skip optional whitespace between keyword and `#`.
            let mut j = after;
            while j < lb.len() && (lb[j] == b' ' || lb[j] == b'\t') {
                j += 1;
            }
            if left_ok && j < lb.len() && lb[j] == b'#' {
                let mut k = j + 1;
                while k < lb.len() && lb[k].is_ascii_digit() {
                    k += 1;
                }
                if k > j + 1 {
                    // At least one digit consumed — real match.
                    return Some(&text[i..k]);
                }
            }
        }
        i += 1;
    }
    None
}

fn find_allowlist_index(sites: &[AllowedSite], f: &Finding) -> Option<usize> {
    sites
        .iter()
        .position(|s| s.path == f.path && s.line == f.line)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_kiro_id_in_simple_comment() {
        let m = find_kiro_id("closes kiro-uphh follow-up");
        assert_eq!(m, Some("kiro-uphh"));
    }

    #[test]
    fn finds_kiro_id_with_digits() {
        assert_eq!(find_kiro_id("see kiro-1ah3 next"), Some("kiro-1ah3"));
    }

    #[test]
    fn does_not_match_kiro_with_three_chars() {
        assert_eq!(find_kiro_id("kiro-abc next"), None);
    }

    #[test]
    fn does_not_match_kiro_with_five_chars() {
        // 5 alphanumeric chars after `kiro-` violates the format.
        assert_eq!(find_kiro_id("kiro-uphhx"), None);
    }

    #[test]
    fn does_not_match_kiro_with_uppercase() {
        // Rivets ids are lowercase only — `kiro-UPHH` is not a real id.
        assert_eq!(find_kiro_id("kiro-UPHH"), None);
    }

    #[test]
    fn does_not_match_kiro_inside_word() {
        assert_eq!(find_kiro_id("mykiro-uphh"), None);
    }

    #[test]
    fn does_not_match_kiro_with_trailing_alphanumeric_extension() {
        // Word boundary on the right: must not be followed by alphanumeric.
        // (Trailing `-` is fine — it's a separator.)
        assert_eq!(find_kiro_id("kiro-uphh9 inside"), None);
        assert_eq!(find_kiro_id("kiro-uphh-extended"), Some("kiro-uphh"));
    }

    #[test]
    fn finds_pr_ref_with_space() {
        assert_eq!(find_pr_ref("from PR #119 review"), Some("PR #119"));
    }

    #[test]
    fn finds_pr_ref_without_space() {
        assert_eq!(find_pr_ref("see pr#42"), Some("pr#42"));
    }

    #[test]
    fn finds_pr_ref_case_insensitive() {
        assert_eq!(find_pr_ref("Pr #7"), Some("Pr #7"));
    }

    #[test]
    fn does_not_match_pr_without_hash() {
        assert_eq!(find_pr_ref("press the button"), None);
    }

    #[test]
    fn does_not_match_pr_inside_word() {
        // `apr #3` — `pr` follows `a`, no word boundary.
        assert_eq!(find_pr_ref("apr #3"), None);
    }

    #[test]
    fn finds_issue_ref() {
        assert_eq!(find_issue_ref("see issue #66"), Some("issue #66"));
    }

    #[test]
    fn finds_reviewer_agent_in_attribution_phrase() {
        assert_eq!(
            find_reviewer_agent("Closes silent-failure-hunter #1"),
            Some("silent-failure-hunter"),
        );
        assert_eq!(
            find_reviewer_agent("per code-reviewer feedback"),
            Some("code-reviewer"),
        );
        assert_eq!(
            find_reviewer_agent("flagged by marketplace-security-reviewer"),
            Some("marketplace-security-reviewer"),
        );
    }

    #[test]
    fn does_not_match_reviewer_agent_inside_longer_kebab_name() {
        // `kiro-code-reviewer-v2` is a legitimate plugin-name string literal
        // used in tests. The word-boundary check on both sides must keep it
        // from matching the embedded `code-reviewer`.
        assert_eq!(find_reviewer_agent("kiro-code-reviewer-v2 plugin"), None);
        assert_eq!(find_reviewer_agent("see code-reviewer-extra notes"), None);
    }

    #[test]
    fn finds_bare_amendment_word_case_insensitive() {
        assert_eq!(find_process_ref("Per A1 amendment"), Some("amendment"));
        assert_eq!(find_process_ref("Per the AMENDMENT"), Some("AMENDMENT"));
        assert_eq!(find_process_ref("see plan amendment"), Some("amendment"));
    }

    #[test]
    fn does_not_match_amendment_inside_word() {
        // The inner letters of a longer alphanumeric token must not match.
        assert_eq!(find_process_ref("amendments9 trailing alphanumeric"), None);
        assert_eq!(find_process_ref("preamendment leading alphanumeric"), None);
    }

    #[test]
    fn finds_per_a_digits_shorthand() {
        assert_eq!(find_process_ref("Per A1: foo"), Some("Per A1"));
        assert_eq!(find_process_ref("per a2 — note"), Some("per a2"));
        assert_eq!(find_process_ref("(per A14)"), Some("per A14"));
    }

    #[test]
    fn does_not_match_per_a_when_not_amendment_shape() {
        // No digits after the A → not the amendment shorthand.
        assert_eq!(find_process_ref("per Apple"), None);
        // Trailing alphanumeric on the digit run → ambiguous, refuse.
        assert_eq!(find_process_ref("per A1x"), None);
        // The "per" must be word-boundaried.
        assert_eq!(find_process_ref("superA1"), None);
    }

    #[test]
    fn extract_line_comment_returns_after_slashes() {
        assert_eq!(
            extract_line_comment("let x = 1; // a comment"),
            Some(" a comment")
        );
    }

    #[test]
    fn extract_line_comment_skips_double_slash_in_string() {
        // The defining false-positive shape: a URL in a string literal.
        assert_eq!(
            extract_line_comment(r#"let url = "https://example.com";"#),
            None
        );
    }

    #[test]
    fn extract_line_comment_handles_escaped_quote_in_string() {
        // The `\"` inside the string must not terminate the string-state
        // tracker prematurely (which would expose `//` to the scanner).
        assert_eq!(
            extract_line_comment(r#"let x = "say \"hi\""; // real comment"#),
            Some(" real comment")
        );
    }

    #[test]
    fn extract_line_comment_returns_none_for_pure_code() {
        assert_eq!(extract_line_comment("let x = 1 + 2;"), None);
    }

    #[test]
    fn scan_comment_emits_multiple_findings_for_multiple_patterns() {
        let findings = scan_comment("f.rs", 1, " closes kiro-uphh from PR #119");
        assert_eq!(findings.len(), 2);
        let kinds: Vec<&str> = findings.iter().map(|f| f.kind).collect();
        assert!(kinds.contains(&"kiro-id"));
        assert!(kinds.contains(&"pr-ref"));
    }

    #[test]
    fn is_scanned_file_accepts_rust_and_typescript_and_svelte() {
        assert!(is_scanned_file("foo.rs"));
        assert!(is_scanned_file("foo.ts"));
        assert!(is_scanned_file("foo.svelte"));
    }

    #[test]
    fn is_scanned_file_rejects_other_extensions() {
        assert!(!is_scanned_file("foo.json"));
        assert!(!is_scanned_file("foo.md"));
        assert!(!is_scanned_file("foo.toml"));
    }

    #[test]
    fn is_scanned_file_skips_bindings_ts() {
        // Auto-generated; manual edits would be erased on regen.
        assert!(!is_scanned_file("bindings.ts"));
    }
}

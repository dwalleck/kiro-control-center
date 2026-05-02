//! `cargo xtask plan-lint` — run structural lint queries against the tethys index.
//!
//! Each lint is a [`Gate`] — a name, a description, and a SQL query that
//! returns one row per violation. Findings are formatted with the file path
//! and line of the offending symbol so reviewers can jump straight to the
//! source.
//!
//! Gate 4 is the canonical motivating example: "no `pub` enum variant
//! carrying an external crate's error type via `#[source]`". CLAUDE.md
//! line ~57 documents the rule; PR #64 was the failure that drove it.
//! The grep that previously encoded this rule (in
//! `docs/plan-review-checklist.md`) silently exits with no matches because
//! `\n` in BRE/ERE is a literal — this command runs the same intent as a
//! SQL query against the tethys index, which fails loud rather than
//! silently clean.

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use rusqlite::Connection;

/// A structural lint backed by a single SQL query against the tethys index.
///
/// The query MUST select `path, line, qualified_name, signature` in that
/// order so [`Gate::run`] can decode rows uniformly. Other columns are
/// ignored; queries are free to JOIN whatever tables they need.
struct Gate {
    name: &'static str,
    description: &'static str,
    sql: &'static str,
}

#[derive(Debug, PartialEq, Eq)]
struct Finding {
    path: String,
    line: u32,
    qualified_name: String,
    signature: Option<String>,
}

/// External crate paths that must not appear behind `#[source]` on any
/// field of a public enum or struct. CLAUDE.md says these errors should
/// be mapped at the adapter boundary into typed `ErrorKind` variants
/// with `reason: String` payloads, never leaked through the public API.
///
/// `io::` is intentionally absent — it's std and CLAUDE.md explicitly
/// allows it.
///
/// Visibility check is on the *parent type*, not the field itself: Rust
/// syntax doesn't allow `pub` on enum variant fields (`pub enum E { V {
/// pub x: T } }` is a compile error), so they always extract as
/// `Visibility::Private`. A `WHERE s.visibility = 'public'` predicate
/// would therefore skip every legitimate Gate 4 violation — the
/// canonical PR #64 case `NativeParseFailure::InvalidJson(#[source]
/// serde_json::Error)` has a private field syntactically. What matters
/// is whether the *enclosing enum or struct* is public — `pub(crate)`
/// or fully private parents don't leak through `kiro-market-core`'s
/// public API.
///
/// The `EXISTS` clause uses `qualified_name`'s `Parent::*` prefix to
/// walk up to the parent symbol, since `parent_symbol_id` is NULL on
/// the current tethys schema (a known gap — see the reference memory).
///
/// The prefix anchor is `parent.qualified_name || '::%'`, NOT
/// `parent.name || '::%'`. For an enum nested inside a `mod foo`
/// block, `parent.name = 'Bar'` but its variants/fields carry
/// `qualified_name = 'foo::Bar::...'`. A `LIKE 'Bar::%'` pattern
/// would miss every nested-module case, silently exempting real
/// gate-4 violations (caught by gemini-code-assist on PR #91).
const GATE_4_SQL: &str = "\
SELECT f.path, s.line, s.qualified_name, s.signature
FROM symbols s
JOIN attributes a ON a.symbol_id = s.id
JOIN files f ON f.id = s.file_id
WHERE s.kind = 'struct_field'
  AND a.name = 'source'
  AND (s.signature LIKE '%serde_json::%'
       OR s.signature LIKE '%gix::%'
       OR s.signature LIKE '%reqwest::%'
       OR s.signature LIKE '%toml::%')
  AND EXISTS (
      SELECT 1 FROM symbols parent
      WHERE parent.kind IN ('enum', 'struct')
        AND parent.visibility = 'public'
        AND parent.file_id = s.file_id
        AND s.qualified_name LIKE parent.qualified_name || '::%'
  )
ORDER BY f.path, s.line";

/// CLAUDE.md "Zero-tolerance in production code (tests are exempt): no
/// `.unwrap()`, no `.expect()`". This query finds calls to those methods
/// in production code, where "production" means:
///
/// - The containing symbol is a function or method (skips uses in
///   constants, type expressions, etc. that aren't runtime panic points).
/// - The containing symbol is not itself marked as a test (`#[test]`,
///   `#[tokio::test]`, `#[rstest]`, ...).
/// - The file is not under a `tests/` or `benches/` directory (Cargo's
///   conventional locations for integration tests / benchmarks) and is
///   not a `test_support` / `test_utils` module.
///
/// `signature` carries the call name (`unwrap` or `expect`) so the same
/// `Finding` shape works for this gate and Gate 4.
// `'/' || f.path` prepends a slash so the LIKE pattern matches paths
// that start with `tests/` or `benches/` at the workspace root the same
// way it matches `crates/foo/tests/...`. Without this, a contributor
// adding `tests/integration.rs` at workspace root would have its
// .unwrap() calls flagged as production violations.
const NO_UNWRAP_SQL: &str = "\
SELECT f.path, r.line, s.qualified_name, r.reference_name
FROM refs r
JOIN symbols s ON s.id = r.in_symbol_id
JOIN files f ON f.id = r.file_id
WHERE r.reference_name IN ('unwrap', 'expect')
  AND s.kind IN ('function', 'method')
  AND s.is_test = 0
  AND '/' || f.path NOT LIKE '%/tests/%'
  AND '/' || f.path NOT LIKE '%/benches/%'
  AND f.path NOT LIKE '%test_support%'
  AND f.path NOT LIKE '%test_utils%'
ORDER BY f.path, r.line";

/// Catches `panic!`, `todo!`, and `unimplemented!` macro invocations in
/// non-test production code. Same JOIN shape as `no-unwrap-in-production`,
/// just a different `reference_name` list — these macros are not method
/// calls but tree-sitter-rust extracts `macro_invocation` references the
/// same way it extracts method calls, so the query is identical in shape.
///
/// `unreachable!()` is *not* in the list — it's the canonical replacement
/// when restructuring code to satisfy zero-tolerance, and treating it as
/// a violation would defeat its purpose. If `unreachable!()` ever shows
/// up at a runtime-reachable site, that's a code-review concern, not a
/// gate concern.
const NO_PANIC_SQL: &str = "\
SELECT f.path, r.line, s.qualified_name, r.reference_name
FROM refs r
JOIN symbols s ON s.id = r.in_symbol_id
JOIN files f ON f.id = r.file_id
WHERE r.reference_name IN ('panic', 'todo', 'unimplemented')
  AND s.kind IN ('function', 'method')
  AND s.is_test = 0
  AND '/' || f.path NOT LIKE '%/tests/%'
  AND '/' || f.path NOT LIKE '%/benches/%'
  AND f.path NOT LIKE '%test_support%'
  AND f.path NOT LIKE '%test_utils%'
ORDER BY f.path, r.line";

/// CLAUDE.md "Dependencies point inward" — `kiro-market-core` is the
/// domain core and must stay free of UI / Tauri / async-runtime / frontend
/// deps. Adding `use tauri::...`, `use tauri_plugin_*::...`, or `use tokio::...`
/// to a file in `kiro-market-core/src/` would violate this even before
/// `Cargo.toml` notices.
///
/// Queries the `imports` table (one row per `use` statement) and flags
/// any import in `kiro-market-core/src/` whose `source_module` starts
/// with one of the forbidden prefixes. `cargo-deny` solves the
/// crate-Cargo.toml version of this rule; this gate solves the
/// per-file version, catching the bad import the moment it lands.
// `imports` does not carry a line column (one row per
// (file_id, symbol_name, source_module) tuple), so we report `line = 0`
// as a sentinel and the reviewer greps the file for the import. The
// path + (symbol_name, source_module) signature is enough to locate it.
const NO_FRONTEND_DEPS_IN_CORE_SQL: &str = "\
SELECT f.path, 0 AS line, i.symbol_name AS qualified_name, i.source_module AS signature
FROM imports i
JOIN files f ON f.id = i.file_id
WHERE f.path LIKE '%kiro-market-core/src/%'
  AND f.path NOT LIKE '%/tests/%'
  AND (i.source_module LIKE 'tauri::%'
       OR i.source_module = 'tauri'
       OR i.source_module LIKE 'tauri_plugin_%'
       OR i.source_module LIKE 'tokio::%'
       OR i.source_module = 'tokio')
ORDER BY f.path, i.symbol_name";

/// CLAUDE.md mandates `#[non_exhaustive]` on error enums so adding a new
/// variant in core doesn't silently break downstream pattern matches.
/// This gate flags any public enum in `kiro-market-core` whose name ends
/// in `Error` and that lacks the attribute.
///
/// **Heuristic limitation:** the SQL matches on `LIKE '%Error'` only.
/// Error-like enums named `*Failure`, `*Kind`, or `*Reason` (e.g.
/// `NativeParseFailure`, `ParseFailure`) deriving `thiserror::Error` are
/// invisible to this gate and must carry `#[non_exhaustive]` by manual
/// review. Widening the SQL would require joining the `attributes` table
/// to detect `thiserror::Error` derives regardless of name; the current
/// `*Error` convention catches the common case and the existing
/// `*Failure` enums in core already opt in.
///
/// Scoped to `kiro-market-core` because the rule is about the public
/// surface of the *library* — Tauri command errors and CLI binary
/// errors don't carry the same downstream-stability requirement.
///
/// `signature` is `NULL` (no useful per-finding text); the gate's
/// description carries the explanation.
const NON_EXHAUSTIVE_ERROR_ENUM_SQL: &str = "\
SELECT f.path, s.line, s.qualified_name, NULL AS signature
FROM symbols s
JOIN files f ON f.id = s.file_id
WHERE s.kind = 'enum'
  AND s.visibility = 'public'
  AND s.name LIKE '%Error'
  AND f.path LIKE '%kiro-market-core/src/%'
  AND f.path NOT LIKE '%/tests/%'
  AND NOT EXISTS (
      SELECT 1 FROM attributes a
      WHERE a.symbol_id = s.id
        AND a.name = 'non_exhaustive'
  )
ORDER BY f.path, s.line";

/// CLAUDE.md "Validation newtypes flowing through Tauri bindings need
/// `cfg_attr(specta::Type)`" — and any pub enum that crosses the FFI must
/// carry an explicit serde tagging directive. PR #83 commit 1 shipped
/// `SteeringWarning` with default external tagging; the bindings.ts shape
/// it generated was an awkward intersection type
/// (`({ Foo: ... }) & { Bar?: never }`) that frontend code patterns like
/// `if (warning.kind === "...")` would silently never match. Caught at
/// review, fixed in commit 2 with `#[serde(tag = "kind", rename_all =
/// "snake_case")]`.
///
/// This gate flags any public enum that:
/// - derives both `Serialize` AND `specta::Type` (the tells that it
///   crosses the FFI),
/// - has at least one non-unit variant (default external tagging is
///   structurally fine for unit-only enums — they serialize as plain
///   strings and TS sees a clean union of string literals), AND
/// - lacks an explicit `#[serde(tag = "...")]` or `#[serde(untagged)]`
///   directive.
///
/// Detecting `specta::Type` covers both unconditional `derive(specta::
/// Type)` and the codebase's canonical `cfg_attr(feature = "specta",
/// derive(specta::Type))` form — both store the type name in
/// `attributes.args` regardless of whether the wrapping attribute is
/// `derive` or `cfg_attr`.
///
/// The non-unit-variant check uses tethys's sub-symbol extraction (rivets
/// PR #58): enum variants are stored as `enum_variant` rows whose
/// `signature` is NULL/empty for unit variants and carries the field list
/// otherwise. Without this filter, every all-unit `pub enum` deriving
/// `Serialize + specta::Type` (`SourceType`, `ErrorType`, etc.) would be
/// a false positive — they don't need tagging because external tagging
/// produces plain string literals for them.
///
/// `parent_symbol_id` is currently NULL on `enum_variant` rows in tethys
/// (a known indexer gap), so the variant→parent link uses
/// `qualified_name LIKE s.qualified_name || '::%'`. The anchor is the
/// parent enum's full `qualified_name`, NOT its short `name` — a nested
/// `mod foo { pub enum Bar { ... } }` has `s.name = 'Bar'` but variants
/// at `qualified_name = 'foo::Bar::Variant'`, and `LIKE 'Bar::%'` would
/// silently miss them (caught by gemini-code-assist on PR #91).
const FFI_ENUM_TAG_SQL: &str = "\
SELECT f.path, s.line, s.qualified_name, NULL AS signature
FROM symbols s
JOIN files f ON f.id = s.file_id
WHERE s.kind = 'enum'
  AND s.visibility = 'public'
  AND EXISTS (
      SELECT 1 FROM attributes a
      WHERE a.symbol_id = s.id
        AND ((a.name = 'derive' AND a.args LIKE '%Serialize%')
             OR (a.name = 'cfg_attr' AND a.args LIKE '%Serialize%'))
  )
  AND EXISTS (
      SELECT 1 FROM attributes a
      WHERE a.symbol_id = s.id
        AND ((a.name = 'derive' AND a.args LIKE '%specta::Type%')
             OR (a.name = 'cfg_attr' AND a.args LIKE '%specta::Type%'))
  )
  AND EXISTS (
      SELECT 1 FROM symbols v
      WHERE v.kind = 'enum_variant'
        AND v.file_id = s.file_id
        AND v.qualified_name LIKE s.qualified_name || '::%'
        AND v.signature IS NOT NULL
        AND v.signature != ''
  )
  AND NOT EXISTS (
      SELECT 1 FROM attributes a
      WHERE a.symbol_id = s.id
        AND a.name = 'serde'
        AND (a.args LIKE '%tag = %' OR a.args LIKE '%untagged%')
  )
ORDER BY f.path, s.line";

const ALL_GATES: &[Gate] = &[
    Gate {
        name: "gate-4-external-error-boundary",
        description: "external crate error type behind #[source] on a field of a public enum/struct",
        sql: GATE_4_SQL,
    },
    Gate {
        name: "no-unwrap-in-production",
        description: ".unwrap() or .expect() in non-test production code",
        sql: NO_UNWRAP_SQL,
    },
    Gate {
        name: "no-panic-in-production",
        description: "panic!, todo!, or unimplemented! in non-test production code",
        sql: NO_PANIC_SQL,
    },
    Gate {
        name: "non-exhaustive-error-enum",
        description: "pub *Error-named enum in kiro-market-core missing #[non_exhaustive] (heuristic: name LIKE '%Error' only)",
        sql: NON_EXHAUSTIVE_ERROR_ENUM_SQL,
    },
    Gate {
        name: "no-frontend-deps-in-core",
        description: "tauri / tokio import in kiro-market-core (dependencies-point-inward)",
        sql: NO_FRONTEND_DEPS_IN_CORE_SQL,
    },
    Gate {
        name: "ffi-enum-serde-tag",
        description: "pub Serialize+specta::Type enum with non-unit variants missing #[serde(tag = \"...\")] / #[serde(untagged)]",
        sql: FFI_ENUM_TAG_SQL,
    },
];

/// A `(gate, path, line)` triple acknowledged as a deliberate exception.
///
/// CLAUDE.md zero-tolerance is the *default*; this list is the register
/// of cases where an idiomatic Rust pattern requires `.expect()` (or
/// similar) and refactoring would only relocate the panic. Each entry
/// must carry a `reason` long enough for a future reviewer to evaluate
/// without re-deriving the rationale.
///
/// The mechanism deliberately keeps the allowlist *in source code* (not
/// a TOML / JSON file): adding an exception requires a code change and
/// shows up in `git blame`, the same audit trail that protects every
/// other rule in this codebase.
struct AllowedSite {
    gate: &'static str,
    path: &'static str,
    line: u32,
    /// Why this site is acknowledged. Read at PR review time.
    #[expect(
        dead_code,
        reason = "human-only documentation; reviewer audit, not a runtime field"
    )]
    reason: &'static str,
}

// Line numbers shift with edits to the target file; when an unrelated edit
// pushes a registered `.expect()` to a different line, the runner's
// `stale_allowlist_entries` check fails CI and forces a coordinated
// `ALLOWED_SITES` update in the same PR. That failure is intentional —
// it keeps the audit trail current rather than letting orphaned rows
// silently exempt some unrelated future panic that lands on the old line.
const ALLOWED_SITES: &[AllowedSite] = &[
    AllowedSite {
        gate: "no-unwrap-in-production",
        path: "crates/kiro-control-center/src-tauri/src/lib.rs",
        line: 55,
        reason: "Tauri scaffolding pattern — debug-only `specta_typescript::Typescript::default().export(...)` failure at app startup. Refactoring to `?` propagation would only move the panic into `fn main()`. Idiomatic Rust at the binary entry point.",
    },
    AllowedSite {
        gate: "no-unwrap-in-production",
        path: "crates/kiro-control-center/src-tauri/src/lib.rs",
        line: 66,
        reason: "Tauri scaffolding pattern — `tauri::Builder::run` failure at app startup. Replacing with Result propagation would only move the panic into `fn main()`. Idiomatic Rust at the binary entry point.",
    },
];

/// Test-only convenience wrapper preserved so the four existing
/// `is_allowed_*` regression tests keep their original names. Production
/// code uses [`find_allowlist_index`] directly so it can record which
/// entries actually matched (powering stale-allowlist detection).
#[cfg(test)]
fn is_allowed(gate: &str, finding: &Finding) -> bool {
    find_allowlist_index(ALLOWED_SITES, gate, finding).is_some()
}

/// Locate the [`ALLOWED_SITES`] entry matching `(gate, finding.path,
/// finding.line)`, returning its index. Returning the index (rather than a
/// boolean) lets the runner record *which* allowlist entries actually
/// matched a finding this run, so a follow-up pass can surface stale
/// entries — sites whose acknowledged `.expect()` was refactored away or
/// shifted to a different line, leaving the allowlist row protecting
/// nothing.
fn find_allowlist_index(sites: &[AllowedSite], gate: &str, finding: &Finding) -> Option<usize> {
    sites
        .iter()
        .position(|s| s.gate == gate && s.path == finding.path && s.line == finding.line)
}

/// Return references to allowlist entries that were *not* matched by any
/// finding during a run, restricted to gates that were actually executed.
/// An entry whose gate was filtered out by `--gate <name>` is correctly
/// excluded — we cannot tell whether it would have matched without
/// running its gate.
///
/// A non-empty result indicates the allowlist has accumulated stale
/// rows: the panic-bearing call site moved/disappeared but the
/// acknowledgement stayed. The runner treats stale entries as findings
/// so the audit trail can't rot silently.
fn stale_allowlist_entries<'a>(
    sites: &'a [AllowedSite],
    matched_indices: &std::collections::HashSet<usize>,
    gates_run: &std::collections::HashSet<&str>,
) -> Vec<&'a AllowedSite> {
    sites
        .iter()
        .enumerate()
        .filter(|(idx, site)| gates_run.contains(site.gate) && !matched_indices.contains(idx))
        .map(|(_, s)| s)
        .collect()
}

impl Gate {
    fn run(&self, conn: &Connection) -> Result<Vec<Finding>> {
        let mut stmt = conn
            .prepare(self.sql)
            .with_context(|| format!("preparing gate {} SQL", self.name))?;
        let rows = stmt
            .query_map([], |row| {
                Ok(Finding {
                    path: row.get(0)?,
                    line: row.get(1)?,
                    qualified_name: row.get(2)?,
                    signature: row.get(3)?,
                })
            })
            .with_context(|| format!("running gate {} SQL", self.name))?
            .collect::<Result<Vec<_>, _>>()
            .with_context(|| format!("collecting gate {} rows", self.name))?;
        Ok(rows)
    }
}

/// CLI options for `plan-lint`.
struct Options {
    workspace: PathBuf,
    skip_reindex: bool,
    gate_filter: Option<String>,
}

impl Options {
    fn parse(mut args: impl Iterator<Item = String>) -> Result<Self> {
        let mut workspace = None;
        let mut skip_reindex = false;
        let mut gate_filter = None;
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--workspace" => {
                    workspace = Some(PathBuf::from(
                        args.next().context("--workspace needs a path")?,
                    ));
                }
                "--no-reindex" => skip_reindex = true,
                "--gate" => {
                    gate_filter = Some(args.next().context("--gate needs a name")?);
                }
                "-h" | "--help" => {
                    print_help();
                    std::process::exit(0);
                }
                other => bail!("unknown plan-lint flag: {other}"),
            }
        }
        let workspace = workspace
            .or_else(|| env::var_os("CLAUDE_PROJECT_DIR").map(PathBuf::from))
            .or_else(|| env::current_dir().ok())
            .context("could not determine workspace root")?;
        Ok(Self {
            workspace,
            skip_reindex,
            gate_filter,
        })
    }

    fn db_path(&self) -> PathBuf {
        self.workspace
            .join(".rivets")
            .join("index")
            .join("tethys.db")
    }
}

fn print_help() {
    // Help text on stdout per UNIX convention so
    // `cargo xtask plan-lint --help | grep TETHYS_BIN` works.
    println!(
        "cargo xtask plan-lint — run structural lint queries against the tethys index

USAGE:
    cargo xtask plan-lint [--workspace <PATH>] [--no-reindex] [--gate <NAME>]

OPTIONS:
    --workspace <PATH>   workspace root (default: $CLAUDE_PROJECT_DIR or cwd)
    --no-reindex         skip the `tethys index` step (use the existing DB as-is)
    --gate <NAME>        run only the named gate (default: run every gate)

ENVIRONMENT:
    TETHYS_BIN           path to the tethys binary (default: `tethys` on PATH)

EXIT CODES:
    0  no findings
    1  one or more gates produced findings (CI gate fails)
    2  internal error (couldn't reach tethys, malformed DB, etc.)"
    );
}

/// Reject an unknown `--gate <name>` value with a list of registered
/// gates. Without this check, a typo like `--gate gate4-external-...`
/// (missing hyphen) would skip every gate, return `Ok(())`, and exit 0
/// — the exact silent-pass failure mode the broken plan-review-checklist
/// grep had. Validating up front means a CI typo fails loud.
fn validate_gate_filter(filter: Option<&str>, gates: &[Gate]) -> Result<()> {
    let Some(name) = filter else {
        return Ok(());
    };
    if gates.iter().any(|g| g.name == name) {
        return Ok(());
    }
    let known: Vec<&str> = gates.iter().map(|g| g.name).collect();
    bail!("unknown gate `{name}`; known gates: {}", known.join(", "));
}

/// Run plan-lint and return the count of violations found across all
/// non-allowlisted gate hits. The caller is responsible for mapping the
/// count to a process exit code (per the documented contract: 0 for
/// clean, 1 for findings, 2 for internal error). `Err` returns are
/// internal errors — propagate them with exit code 2.
pub fn run(args: impl Iterator<Item = String>) -> Result<usize> {
    let opts = Options::parse(args)?;

    // Validate `--gate <name>` before the expensive index step.
    validate_gate_filter(opts.gate_filter.as_deref(), ALL_GATES)?;

    if !opts.skip_reindex {
        ensure_tethys_index(&opts.workspace).context("re-indexing with tethys failed")?;
    }

    let db_path = opts.db_path();
    if !db_path.is_file() {
        bail!(
            "tethys index not found at {} — run `tethys index` first or omit --no-reindex",
            db_path.display()
        );
    }
    let conn = Connection::open(&db_path)
        .with_context(|| format!("opening tethys index at {}", db_path.display()))?;

    let mut total_findings = 0usize;
    let mut matched_allowlist_indices: std::collections::HashSet<usize> =
        std::collections::HashSet::new();
    let mut gates_run: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for gate in ALL_GATES {
        if let Some(name) = &opts.gate_filter
            && gate.name != name
        {
            continue;
        }
        gates_run.insert(gate.name);
        let raw = gate.run(&conn)?;
        let mut allowed: Vec<Finding> = Vec::new();
        let mut findings: Vec<Finding> = Vec::new();
        for f in raw {
            if let Some(idx) = find_allowlist_index(ALLOWED_SITES, gate.name, &f) {
                matched_allowlist_indices.insert(idx);
                allowed.push(f);
            } else {
                findings.push(f);
            }
        }

        if findings.is_empty() {
            if allowed.is_empty() {
                println!("{} OK", gate.name);
            } else {
                println!(
                    "{} OK ({} allowlisted exception{})",
                    gate.name,
                    allowed.len(),
                    if allowed.len() == 1 { "" } else { "s" },
                );
            }
            continue;
        }
        total_findings += findings.len();
        println!(
            "{} — {} ({} finding{}{})",
            gate.name,
            gate.description,
            findings.len(),
            if findings.len() == 1 { "" } else { "s" },
            if allowed.is_empty() {
                String::new()
            } else {
                format!(
                    ", {} allowlisted exception{}",
                    allowed.len(),
                    if allowed.len() == 1 { "" } else { "s" },
                )
            },
        );
        for f in &findings {
            print_finding(f);
        }
    }

    // Surface allowlist entries whose `.expect()` site moved or vanished.
    // Without this check the audit trail rots silently — a panic at
    // `lib.rs:49` shifts to `lib.rs:51` after an unrelated edit, the new
    // line is flagged as a real violation, and the original line-49 entry
    // outlives its rationale, ready to silently exempt some unrelated
    // future panic that happens to land on line 49.
    let stale = stale_allowlist_entries(ALLOWED_SITES, &matched_allowlist_indices, &gates_run);
    if !stale.is_empty() {
        total_findings += stale.len();
        println!(
            "stale-allowlist — {} entr{} in ALLOWED_SITES matched no finding (line shifted or panic refactored away)",
            stale.len(),
            if stale.len() == 1 { "y" } else { "ies" },
        );
        for site in &stale {
            println!("    {}:{}  ({})", site.path, site.line, site.gate);
        }
    }

    if total_findings > 0 {
        // Stderr message; stdout already carries the per-gate output.
        // The caller (main) maps this count to exit code 1.
        eprintln!("plan-lint found {total_findings} violation(s)");
    }
    Ok(total_findings)
}

fn print_finding(f: &Finding) {
    let signature = f.signature.as_deref().unwrap_or("<no signature>");
    println!("    {}", format_path_line(&f.path, f.line));
    println!("        {} : {}", f.qualified_name, signature);
}

/// Format the `path:line` prefix shown for a finding. The
/// `no-frontend-deps-in-core` gate uses `line = 0` as a sentinel because
/// the `imports` tethys table has no line column; rendering that as
/// `path:0` invites editors and CI log parsers to treat it as a real
/// line reference. This helper substitutes a human-readable hint so the
/// reviewer knows to grep the file for the import.
fn format_path_line(path: &str, line: u32) -> String {
    if line == 0 {
        format!("{path} (line unknown — grep for import)")
    } else {
        format!("{path}:{line}")
    }
}

fn ensure_tethys_index(workspace: &Path) -> Result<()> {
    let bin = env::var_os("TETHYS_BIN").map_or_else(|| PathBuf::from("tethys"), PathBuf::from);
    let status = Command::new(&bin)
        .args([
            "--workspace",
            workspace
                .to_str()
                .context("workspace path must be UTF-8 for tethys CLI")?,
            "index",
        ])
        .status()
        .with_context(|| {
            format!(
                "failed to invoke tethys binary at {} (set TETHYS_BIN to override)",
                bin.display()
            )
        })?;
    if !status.success() {
        bail!("tethys index exited with {status}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal tethys schema needed for the queries here. If tethys's schema
    /// changes shape, update this — and prefer adding an integration test
    /// that exercises a real index over expanding this fixture.
    const TEST_SCHEMA: &str = "
        CREATE TABLE files (
            id INTEGER PRIMARY KEY,
            path TEXT NOT NULL UNIQUE,
            language TEXT NOT NULL,
            mtime_ns INTEGER NOT NULL DEFAULT 0,
            size_bytes INTEGER NOT NULL DEFAULT 0,
            content_hash INTEGER,
            indexed_at INTEGER NOT NULL DEFAULT 0
        );
        CREATE TABLE symbols (
            id INTEGER PRIMARY KEY,
            file_id INTEGER NOT NULL,
            name TEXT NOT NULL,
            module_path TEXT NOT NULL DEFAULT '',
            qualified_name TEXT NOT NULL,
            kind TEXT NOT NULL,
            line INTEGER NOT NULL,
            column INTEGER NOT NULL DEFAULT 0,
            end_line INTEGER,
            end_column INTEGER,
            signature TEXT,
            visibility TEXT NOT NULL DEFAULT 'public',
            parent_symbol_id INTEGER,
            is_test INTEGER NOT NULL DEFAULT 0
        );
        CREATE TABLE refs (
            id INTEGER PRIMARY KEY,
            symbol_id INTEGER,
            file_id INTEGER NOT NULL,
            kind TEXT NOT NULL DEFAULT 'call',
            line INTEGER NOT NULL,
            column INTEGER NOT NULL DEFAULT 0,
            end_line INTEGER,
            end_column INTEGER,
            in_symbol_id INTEGER,
            reference_name TEXT
        );
        CREATE TABLE attributes (
            id INTEGER PRIMARY KEY,
            symbol_id INTEGER NOT NULL,
            name TEXT NOT NULL,
            args TEXT,
            line INTEGER NOT NULL
        );
        CREATE TABLE imports (
            file_id INTEGER NOT NULL,
            symbol_name TEXT NOT NULL,
            source_module TEXT NOT NULL,
            alias TEXT,
            PRIMARY KEY (file_id, symbol_name, source_module)
        );
    ";

    fn fresh_db() -> Connection {
        let conn = Connection::open_in_memory().expect("in-memory open should succeed");
        conn.execute_batch(TEST_SCHEMA)
            .expect("schema setup should succeed");
        conn.execute(
            "INSERT INTO files (id, path, language) VALUES (1, 'src/error.rs', 'rust')",
            [],
        )
        .expect("file insert");
        conn
    }

    /// Seed a parent enum row for Gate-4 fixtures with a caller-managed
    /// id. Both kind values ('enum' and 'struct') satisfy the gate's
    /// EXISTS clause; we pick 'enum' because that's the canonical Gate-4
    /// shape (variants with `#[source]` fields).
    fn seed_parent_enum(
        conn: &Connection,
        sym_id: i64,
        file_id: i64,
        name: &str,
        visibility: &str,
    ) {
        conn.execute(
            "INSERT INTO symbols (id, file_id, name, qualified_name, kind, line, visibility)
             VALUES (?1, ?2, ?3, ?3, 'enum', 1, ?4)",
            rusqlite::params![sym_id, file_id, name, visibility],
        )
        .expect("parent enum insert");
    }

    /// Seed a `struct_field` row plus a public parent enum derived from
    /// the qualified name's first `::`-segment. Convenience wrapper for
    /// the common Gate-4 fixture shape; use
    /// [`insert_field_under_parent`] when the test needs a non-public
    /// parent.
    fn insert_field(
        conn: &Connection,
        id: i64,
        qualified: &str,
        signature: &str,
        line: u32,
        with_source_attr: bool,
    ) {
        insert_field_under_parent(
            conn,
            id,
            qualified,
            signature,
            line,
            with_source_attr,
            "public",
        );
    }

    /// Seed a `struct_field` row plus its parent enum at the requested
    /// visibility. Parent ids occupy the 1000+ range so they cannot
    /// collide with the lower field ids that existing tests use; calling
    /// this twice with the same parent name is fine — only the first
    /// call seeds the parent (the second's INSERT silently succeeds via
    /// a different `parent_id`, which is harmless because the EXISTS
    /// clause matches by name+file, not by id).
    fn insert_field_under_parent(
        conn: &Connection,
        id: i64,
        qualified: &str,
        signature: &str,
        line: u32,
        with_source_attr: bool,
        parent_visibility: &str,
    ) {
        let parent_name = qualified
            .split("::")
            .next()
            .expect("qualified name has at least one segment");
        // Allocate a non-conflicting parent id derived from the field id.
        let parent_id: i64 = 1000 + id;
        // Skip if this parent_id is already taken (e.g. multiple fields
        // under one parent within the same test): the EXISTS clause
        // matches by (file_id, name) so any pre-existing row works.
        let already_seeded: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM symbols WHERE file_id = 1 AND name = ?1 AND kind = 'enum'",
                [parent_name],
                |r| r.get(0),
            )
            .expect("count query");
        if already_seeded == 0 {
            seed_parent_enum(conn, parent_id, 1, parent_name, parent_visibility);
        }
        conn.execute(
            "INSERT INTO symbols (id, file_id, name, qualified_name, kind, line, signature)
             VALUES (?1, 1, '0', ?2, 'struct_field', ?3, ?4)",
            rusqlite::params![id, qualified, line, signature],
        )
        .expect("symbol insert");
        if with_source_attr {
            conn.execute(
                "INSERT INTO attributes (symbol_id, name, args, line)
                 VALUES (?1, 'source', NULL, ?2)",
                rusqlite::params![id, line],
            )
            .expect("attr insert");
        }
    }

    fn gate_4() -> &'static Gate {
        ALL_GATES
            .iter()
            .find(|g| g.name == "gate-4-external-error-boundary")
            .expect("gate 4 should be registered")
    }

    #[test]
    fn gate_4_flags_external_serde_json_error_with_source_attr() {
        let conn = fresh_db();
        insert_field(
            &conn,
            1,
            "NativeParseFailure::InvalidJson::0",
            "serde_json::Error",
            42,
            true,
        );

        let findings = gate_4().run(&conn).expect("gate query should succeed");
        assert_eq!(findings.len(), 1);
        assert_eq!(
            findings[0].qualified_name,
            "NativeParseFailure::InvalidJson::0"
        );
        assert_eq!(findings[0].signature.as_deref(), Some("serde_json::Error"));
        assert_eq!(findings[0].line, 42);
    }

    #[test]
    fn gate_4_ignores_io_error_even_with_source_attr() {
        let conn = fresh_db();
        insert_field(
            &conn,
            1,
            "PluginError::ManifestReadFailed::source",
            "io::Error",
            10,
            true,
        );

        let findings = gate_4().run(&conn).expect("gate query should succeed");
        assert!(
            findings.is_empty(),
            "io::Error is std and explicitly allowed; got: {findings:?}",
        );
    }

    #[test]
    fn gate_4_ignores_external_error_without_source_attr() {
        // Even if the type name matches, no #[source] attribute means no
        // gate-4 violation — the rule is specifically about typed `Error`
        // sources flowing through the public error chain.
        let conn = fresh_db();
        insert_field(
            &conn,
            1,
            "Wrapped::field",
            "serde_json::Error",
            5,
            false, // no #[source]
        );

        let findings = gate_4().run(&conn).expect("gate query should succeed");
        assert!(findings.is_empty());
    }

    #[test]
    fn gate_4_flags_each_external_crate_in_the_canonical_list() {
        let conn = fresh_db();
        let crates = [
            "serde_json::Error",
            "gix::Error",
            "reqwest::Error",
            "toml::de::Error",
        ];
        for (i, sig) in crates.iter().enumerate() {
            let id = i64::try_from(i).expect("enumerated index over 4-element slice") + 1;
            let line = u32::try_from(i).expect("enumerated index over 4-element slice") + 1;
            insert_field(&conn, id, &format!("E::V{i}::0"), sig, line, true);
        }

        let findings = gate_4().run(&conn).expect("gate query should succeed");
        assert_eq!(findings.len(), 4);
    }

    #[test]
    fn gate_4_findings_ordered_by_path_then_line() {
        let conn = fresh_db();
        // Second file so we can verify ordering across files. file_id=1 is
        // 'src/error.rs' (set up by fresh_db); file_id=2 is 'src/agent.rs'.
        conn.execute(
            "INSERT INTO files (id, path, language) VALUES (2, 'src/agent.rs', 'rust')",
            [],
        )
        .expect("second file insert");
        // Parent enum 'A' lives in file 1; parent enum 'B' lives in file 2.
        // Both public so Gate 4 fires.
        conn.execute(
            "INSERT INTO symbols (file_id, name, qualified_name, kind, line, visibility)
             VALUES (1, 'A', 'A', 'enum', 1, 'public'),
                    (2, 'B', 'B', 'enum', 1, 'public')",
            [],
        )
        .expect("parent enums");
        // Insert fields in reverse path order to detect that ORDER BY fires.
        conn.execute(
            "INSERT INTO symbols (id, file_id, name, qualified_name, kind, line, signature)
             VALUES (10, 1, '0', 'A::0', 'struct_field', 99, 'serde_json::Error'),
                    (11, 2, '0', 'B::0', 'struct_field', 1, 'gix::Error')",
            [],
        )
        .expect("fields");
        conn.execute(
            "INSERT INTO attributes (symbol_id, name, args, line)
             VALUES (10, 'source', NULL, 99), (11, 'source', NULL, 1)",
            [],
        )
        .expect("attrs");

        let findings = gate_4().run(&conn).expect("gate query should succeed");
        assert_eq!(findings.len(), 2);
        // src/agent.rs sorts before src/error.rs alphabetically.
        assert_eq!(findings[0].path, "src/agent.rs");
        assert_eq!(findings[1].path, "src/error.rs");
    }

    #[test]
    fn gate_4_skips_field_when_parent_is_private() {
        // The exact pattern PR #72's review flagged: an internal type
        // with #[source] serde_json::Error. CLAUDE.md scopes the rule to
        // the *public API*; a private parent doesn't violate it.
        let conn = fresh_db();
        insert_field_under_parent(
            &conn,
            1,
            "InternalIndexError::Bad::0",
            "serde_json::Error",
            10,
            true,
            "private",
        );

        let findings = gate_4().run(&conn).expect("gate query should succeed");
        assert!(
            findings.is_empty(),
            "private parent enum is not public API; got {findings:?}",
        );
    }

    #[test]
    fn gate_4_skips_field_when_parent_is_pub_crate() {
        // pub(crate) is reachable within the crate but not part of the
        // *external* public API of `kiro-market-core`. CLAUDE.md scopes
        // the rule to the public API.
        let conn = fresh_db();
        insert_field_under_parent(
            &conn,
            1,
            "InternalIndexError::Bad::0",
            "serde_json::Error",
            10,
            true,
            "crate",
        );

        let findings = gate_4().run(&conn).expect("gate query should succeed");
        assert!(
            findings.is_empty(),
            "pub(crate) parent enum is not public API; got {findings:?}",
        );
    }

    #[test]
    fn gate_4_flags_field_inside_nested_module_enum() {
        // gemini-code-assist (PR #91): for an enum nested inside a
        // `mod foo` block, `parent.name = 'Bar'` but the field's
        // `qualified_name = 'foo::Bar::field'`. The earlier prefix
        // anchor `parent.name || '::%'` would have silently exempted
        // every nested-module gate-4 violation. Anchor on
        // `parent.qualified_name` so nesting depth is irrelevant.
        let conn = fresh_db();
        // Parent enum with module-qualified name (kind 'enum', public).
        conn.execute(
            "INSERT INTO symbols (id, file_id, name, qualified_name, kind, line, visibility)
             VALUES (1000, 1, 'Inner', 'foo::Inner', 'enum', 1, 'public')",
            [],
        )
        .expect("nested parent enum insert");
        // Field whose qualified_name carries the same module prefix.
        conn.execute(
            "INSERT INTO symbols (id, file_id, name, qualified_name, kind, line, signature)
             VALUES (1, 1, '0', 'foo::Inner::Bad::0', 'struct_field', 42, 'serde_json::Error')",
            [],
        )
        .expect("nested field insert");
        conn.execute(
            "INSERT INTO attributes (symbol_id, name, args, line)
             VALUES (1, 'source', NULL, 42)",
            [],
        )
        .expect("source attr insert");

        let findings = gate_4().run(&conn).expect("gate query should succeed");
        assert_eq!(
            findings.len(),
            1,
            "nested-module enum field with #[source] external error must be flagged; got: {findings:?}"
        );
        assert_eq!(findings[0].qualified_name, "foo::Inner::Bad::0");
    }

    // ─── no-unwrap-in-production ────────────────────────────────────────

    fn no_unwrap() -> &'static Gate {
        ALL_GATES
            .iter()
            .find(|g| g.name == "no-unwrap-in-production")
            .expect("no-unwrap gate should be registered")
    }

    /// Seeds a `files` row plus a containing function symbol and returns
    /// the symbol id so callers can attach unwrap refs to it.
    fn seed_function(
        conn: &Connection,
        file_id: i64,
        path: &str,
        sym_id: i64,
        qualified: &str,
        is_test: bool,
    ) {
        conn.execute(
            "INSERT OR IGNORE INTO files (id, path, language) VALUES (?1, ?2, 'rust')",
            rusqlite::params![file_id, path],
        )
        .expect("file insert");
        conn.execute(
            "INSERT INTO symbols (id, file_id, name, qualified_name, kind, line, signature, is_test)
             VALUES (?1, ?2, ?3, ?4, 'function', 1, 'fn x()', ?5)",
            rusqlite::params![sym_id, file_id, qualified, qualified, i64::from(is_test)],
        )
        .expect("symbol insert");
    }

    fn insert_panic_ref(
        conn: &Connection,
        file_id: i64,
        in_symbol_id: i64,
        line: u32,
        which: &str,
    ) {
        conn.execute(
            "INSERT INTO refs (symbol_id, file_id, in_symbol_id, line, reference_name)
             VALUES (NULL, ?1, ?2, ?3, ?4)",
            rusqlite::params![file_id, in_symbol_id, line, which],
        )
        .expect("ref insert");
    }

    #[test]
    fn no_unwrap_flags_unwrap_in_production_function() {
        let conn = fresh_db();
        seed_function(&conn, 2, "crates/core/src/git.rs", 10, "run_git", false);
        insert_panic_ref(&conn, 2, 10, 42, "unwrap");

        let findings = no_unwrap().run(&conn).expect("query should succeed");
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].path, "crates/core/src/git.rs");
        assert_eq!(findings[0].line, 42);
        assert_eq!(findings[0].qualified_name, "run_git");
        assert_eq!(findings[0].signature.as_deref(), Some("unwrap"));
    }

    #[test]
    fn no_unwrap_flags_expect_too() {
        let conn = fresh_db();
        seed_function(&conn, 2, "crates/core/src/git.rs", 10, "run_git", false);
        insert_panic_ref(&conn, 2, 10, 42, "expect");

        let findings = no_unwrap().run(&conn).expect("query should succeed");
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].signature.as_deref(), Some("expect"));
    }

    #[test]
    fn no_unwrap_skips_unwrap_or_and_unwrap_or_else() {
        let conn = fresh_db();
        seed_function(&conn, 2, "crates/core/src/git.rs", 10, "run_git", false);
        // These call shapes are safe (provide defaults). They must not be
        // flagged — IN ('unwrap', 'expect') is exact-match only.
        insert_panic_ref(&conn, 2, 10, 5, "unwrap_or");
        insert_panic_ref(&conn, 2, 10, 6, "unwrap_or_else");
        insert_panic_ref(&conn, 2, 10, 7, "unwrap_or_default");

        let findings = no_unwrap().run(&conn).expect("query should succeed");
        assert!(
            findings.is_empty(),
            "default-providing variants are not panic points; got {findings:?}"
        );
    }

    #[test]
    fn no_unwrap_skips_test_marked_functions() {
        // A function with `is_test = 1` (because tethys saw `#[test]` etc.)
        // is exempt — CLAUDE.md zero-tolerance applies to production only.
        let conn = fresh_db();
        seed_function(&conn, 2, "crates/core/src/lib.rs", 10, "test_thing", true);
        insert_panic_ref(&conn, 2, 10, 42, "unwrap");

        let findings = no_unwrap().run(&conn).expect("query should succeed");
        assert!(
            findings.is_empty(),
            "test-marked fn exempt; got {findings:?}"
        );
    }

    #[test]
    fn no_unwrap_skips_files_under_tests_dir() {
        // Cargo's integration-tests convention: anything under `tests/` is
        // test code by virtue of where it lives, even if the function
        // itself isn't `#[test]`-marked (helpers in tests/common/, etc.).
        let conn = fresh_db();
        seed_function(
            &conn,
            2,
            "crates/core/tests/common/fixtures.rs",
            10,
            "make_fixture",
            false,
        );
        insert_panic_ref(&conn, 2, 10, 42, "unwrap");

        let findings = no_unwrap().run(&conn).expect("query should succeed");
        assert!(
            findings.is_empty(),
            "files under tests/ are exempt; got {findings:?}"
        );
    }

    #[test]
    fn no_unwrap_skips_workspace_root_tests_and_benches() {
        // Regression test for PR #72 review feedback: a file at workspace
        // root like `tests/integration.rs` (no leading slash, no `/tests/`
        // substring) was previously NOT excluded by the LIKE pattern.
        // The `'/' || f.path` prepend in NO_UNWRAP_SQL handles both
        // workspace-root and crate-nested paths uniformly.
        let conn = fresh_db();
        seed_function(&conn, 2, "tests/integration.rs", 10, "test_helper", false);
        seed_function(&conn, 3, "benches/throughput.rs", 11, "bench_helper", false);
        insert_panic_ref(&conn, 2, 10, 42, "unwrap");
        insert_panic_ref(&conn, 3, 11, 42, "unwrap");

        let findings = no_unwrap().run(&conn).expect("query should succeed");
        assert!(
            findings.is_empty(),
            "workspace-root tests/ and benches/ exempt; got {findings:?}"
        );
    }

    #[test]
    fn no_unwrap_skips_test_support_modules() {
        let conn = fresh_db();
        seed_function(
            &conn,
            2,
            "crates/core/src/service/test_support.rs",
            10,
            "MarketplaceService::stub",
            false,
        );
        insert_panic_ref(&conn, 2, 10, 42, "expect");

        let findings = no_unwrap().run(&conn).expect("query should succeed");
        assert!(
            findings.is_empty(),
            "test_support modules exempt; got {findings:?}"
        );
    }

    #[test]
    fn no_unwrap_does_not_flag_calls_outside_a_function() {
        // A ref with NULL in_symbol_id (e.g. unwrap inside a `const fn` body
        // that tethys can't yet attribute) shouldn't fail the JOIN-based
        // query — INNER JOIN excludes it naturally.
        let conn = fresh_db();
        conn.execute(
            "INSERT INTO refs (symbol_id, file_id, in_symbol_id, line, reference_name)
             VALUES (NULL, 1, NULL, 5, 'unwrap')",
            [],
        )
        .expect("ref insert");

        let findings = no_unwrap().run(&conn).expect("query should succeed");
        assert!(findings.is_empty());
    }

    // ─── format_path_line (line=0 sentinel) ─────────────────────────────

    #[test]
    fn format_path_line_renders_real_line_normally() {
        assert_eq!(
            format_path_line("crates/kiro-market-core/src/git.rs", 42),
            "crates/kiro-market-core/src/git.rs:42"
        );
    }

    #[test]
    fn format_path_line_substitutes_hint_for_zero_line_sentinel() {
        // The no-frontend-deps-in-core gate uses line=0 as a sentinel
        // (imports tethys table has no line column). Rendering as
        // `path:0` would mislead editors and CI parsers into following
        // a nonexistent line reference.
        assert_eq!(
            format_path_line("crates/kiro-market-core/src/foo.rs", 0),
            "crates/kiro-market-core/src/foo.rs (line unknown — grep for import)"
        );
    }

    // ─── --gate <name> validation ───────────────────────────────────────

    #[test]
    fn gate_filter_none_passes() {
        // Default invocation runs every gate.
        validate_gate_filter(None, ALL_GATES).expect("None should pass");
    }

    #[test]
    fn gate_filter_with_known_name_passes() {
        validate_gate_filter(Some("gate-4-external-error-boundary"), ALL_GATES)
            .expect("registered gate name should pass");
        validate_gate_filter(Some("no-unwrap-in-production"), ALL_GATES)
            .expect("registered gate name should pass");
    }

    #[test]
    fn gate_filter_with_unknown_name_fails_loud() {
        // The exact failure mode the broken plan-review-checklist grep had:
        // a typo silently skips everything. Must produce an error naming
        // the unknown gate AND the valid alternatives.
        let err =
            validate_gate_filter(Some("gate4-external-error-boundary"), ALL_GATES).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("gate4-external-error-boundary"),
            "error should echo the typo, got: {msg}"
        );
        assert!(
            msg.contains("gate-4-external-error-boundary"),
            "error should list valid gate names so the user can find the typo, got: {msg}"
        );
    }

    #[test]
    fn gate_filter_empty_string_fails_loud() {
        // `--gate ""` is also a typo, not a "match all" wildcard.
        let err = validate_gate_filter(Some(""), ALL_GATES).unwrap_err();
        assert!(
            err.to_string().contains("unknown gate"),
            "empty string should be treated as unknown, got: {err}"
        );
    }

    // ─── Allowlist mechanism ────────────────────────────────────────────

    #[test]
    fn is_allowed_matches_gate_path_and_line_exactly() {
        let f = Finding {
            path: "crates/kiro-control-center/src-tauri/src/lib.rs".to_string(),
            line: 55,
            qualified_name: "run".to_string(),
            signature: Some("expect".to_string()),
        };
        assert!(is_allowed("no-unwrap-in-production", &f));
    }

    #[test]
    fn is_allowed_is_per_gate() {
        // Same path/line registered under no-unwrap must NOT suppress a
        // gate-4 finding at the same location.
        let f = Finding {
            path: "crates/kiro-control-center/src-tauri/src/lib.rs".to_string(),
            line: 49,
            qualified_name: "run".to_string(),
            signature: Some("expect".to_string()),
        };
        assert!(!is_allowed("gate-4-external-error-boundary", &f));
    }

    #[test]
    fn is_allowed_rejects_unregistered_lines() {
        // One line off the registered exception is not exempted.
        let f = Finding {
            path: "crates/kiro-control-center/src-tauri/src/lib.rs".to_string(),
            line: 48,
            qualified_name: "run".to_string(),
            signature: Some("expect".to_string()),
        };
        assert!(!is_allowed("no-unwrap-in-production", &f));
    }

    #[test]
    fn is_allowed_rejects_unregistered_paths() {
        let f = Finding {
            path: "src/some_other_file.rs".to_string(),
            line: 49,
            qualified_name: "run".to_string(),
            signature: Some("expect".to_string()),
        };
        assert!(!is_allowed("no-unwrap-in-production", &f));
    }

    // ─── Stale-allowlist detection ──────────────────────────────────────

    /// Synthetic allowlist used by stale-detection tests. Decouples the
    /// tests from the production [`ALLOWED_SITES`] so adding/removing
    /// production entries doesn't break the regression suite.
    const SYNTHETIC_SITES: &[AllowedSite] = &[
        AllowedSite {
            gate: "no-unwrap-in-production",
            path: "src/a.rs",
            line: 10,
            reason: "test fixture A",
        },
        AllowedSite {
            gate: "no-unwrap-in-production",
            path: "src/b.rs",
            line: 20,
            reason: "test fixture B",
        },
        AllowedSite {
            gate: "gate-4-external-error-boundary",
            path: "src/c.rs",
            line: 30,
            reason: "test fixture C",
        },
    ];

    #[test]
    fn find_allowlist_index_returns_position_for_match() {
        let f = Finding {
            path: "src/b.rs".to_string(),
            line: 20,
            qualified_name: "x".to_string(),
            signature: None,
        };
        assert_eq!(
            find_allowlist_index(SYNTHETIC_SITES, "no-unwrap-in-production", &f),
            Some(1)
        );
    }

    #[test]
    fn find_allowlist_index_is_per_gate() {
        // (path, line) is registered under no-unwrap; querying as gate-4
        // must return None — the index lookup must be gate-scoped.
        let f = Finding {
            path: "src/a.rs".to_string(),
            line: 10,
            qualified_name: "x".to_string(),
            signature: None,
        };
        assert_eq!(
            find_allowlist_index(SYNTHETIC_SITES, "gate-4-external-error-boundary", &f),
            None
        );
    }

    #[test]
    fn stale_allowlist_entries_returns_unmatched_indices_for_run_gates() {
        // Nothing matched, but both no-unwrap entries should surface as
        // stale because their gate was actually run.
        let matched = std::collections::HashSet::new();
        let mut gates_run = std::collections::HashSet::new();
        gates_run.insert("no-unwrap-in-production");

        let stale = stale_allowlist_entries(SYNTHETIC_SITES, &matched, &gates_run);
        assert_eq!(stale.len(), 2);
        let stale_paths: Vec<&str> = stale.iter().map(|s| s.path).collect();
        assert!(stale_paths.contains(&"src/a.rs"));
        assert!(stale_paths.contains(&"src/b.rs"));
    }

    #[test]
    fn stale_allowlist_entries_skips_entries_for_unrun_gates() {
        // Only no-unwrap was run; gate-4 entries must NOT be reported as
        // stale because we have no observation of whether they would
        // have matched. A `--gate <name>` filter must not erode the
        // allowlist for unrun gates.
        let matched = std::collections::HashSet::new();
        let mut gates_run = std::collections::HashSet::new();
        gates_run.insert("no-unwrap-in-production");

        let stale = stale_allowlist_entries(SYNTHETIC_SITES, &matched, &gates_run);
        let stale_paths: Vec<&str> = stale.iter().map(|s| s.path).collect();
        assert!(
            !stale_paths.contains(&"src/c.rs"),
            "gate-4 entry must not be reported stale when only no-unwrap ran"
        );
    }

    #[test]
    fn stale_allowlist_entries_returns_empty_when_all_indices_matched() {
        let mut matched = std::collections::HashSet::new();
        matched.insert(0);
        matched.insert(1);
        matched.insert(2);
        let mut gates_run = std::collections::HashSet::new();
        gates_run.insert("no-unwrap-in-production");
        gates_run.insert("gate-4-external-error-boundary");

        let stale = stale_allowlist_entries(SYNTHETIC_SITES, &matched, &gates_run);
        assert!(
            stale.is_empty(),
            "no entries should be stale when every index matched"
        );
    }

    // ─── no-panic-in-production ─────────────────────────────────────────

    fn no_panic() -> &'static Gate {
        ALL_GATES
            .iter()
            .find(|g| g.name == "no-panic-in-production")
            .expect("no-panic gate registered")
    }

    #[test]
    fn no_panic_flags_panic_macro() {
        let conn = fresh_db();
        seed_function(&conn, 2, "crates/core/src/lib.rs", 10, "do_thing", false);
        insert_panic_ref(&conn, 2, 10, 42, "panic");

        let findings = no_panic().run(&conn).expect("query should succeed");
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].signature.as_deref(), Some("panic"));
    }

    #[test]
    fn no_panic_flags_todo_and_unimplemented() {
        let conn = fresh_db();
        seed_function(&conn, 2, "crates/core/src/lib.rs", 10, "do_thing", false);
        insert_panic_ref(&conn, 2, 10, 1, "todo");
        insert_panic_ref(&conn, 2, 10, 2, "unimplemented");

        let findings = no_panic().run(&conn).expect("query should succeed");
        assert_eq!(findings.len(), 2);
    }

    #[test]
    fn no_panic_does_not_flag_unreachable() {
        // unreachable!() is the canonical replacement when restructuring to
        // satisfy zero-tolerance — it must NOT be flagged by this gate or
        // the gate would defeat its own remediation path.
        let conn = fresh_db();
        seed_function(&conn, 2, "crates/core/src/lib.rs", 10, "do_thing", false);
        insert_panic_ref(&conn, 2, 10, 5, "unreachable");

        let findings = no_panic().run(&conn).expect("query should succeed");
        assert!(
            findings.is_empty(),
            "unreachable! is exempt; got {findings:?}"
        );
    }

    #[test]
    fn no_panic_skips_test_marked_functions() {
        let conn = fresh_db();
        seed_function(&conn, 2, "crates/core/src/lib.rs", 10, "test_thing", true);
        insert_panic_ref(&conn, 2, 10, 42, "panic");

        let findings = no_panic().run(&conn).expect("query should succeed");
        assert!(findings.is_empty());
    }

    #[test]
    fn no_panic_skips_files_under_tests_dir() {
        // Anything under `tests/` is test code by virtue of where it
        // lives, even if the function itself isn't `#[test]`-marked
        // (e.g. helpers in tests/common/). Mirrors the no-unwrap test
        // so a future SQL edit dropping the `'/' || f.path NOT LIKE
        // '%/tests/%'` clause regresses both gates symmetrically.
        let conn = fresh_db();
        seed_function(
            &conn,
            2,
            "crates/core/tests/common/fixtures.rs",
            10,
            "make_fixture",
            false,
        );
        insert_panic_ref(&conn, 2, 10, 42, "panic");

        let findings = no_panic().run(&conn).expect("query should succeed");
        assert!(
            findings.is_empty(),
            "files under tests/ are exempt; got {findings:?}"
        );
    }

    #[test]
    fn no_panic_skips_workspace_root_tests_and_benches() {
        // The `'/' || f.path` prepend in NO_PANIC_SQL must handle both
        // workspace-root (`tests/integration.rs`) and crate-nested
        // (`crates/foo/tests/...`) paths uniformly.
        let conn = fresh_db();
        seed_function(&conn, 2, "tests/integration.rs", 10, "test_helper", false);
        seed_function(&conn, 3, "benches/throughput.rs", 11, "bench_helper", false);
        insert_panic_ref(&conn, 2, 10, 42, "panic");
        insert_panic_ref(&conn, 3, 11, 42, "todo");

        let findings = no_panic().run(&conn).expect("query should succeed");
        assert!(
            findings.is_empty(),
            "workspace-root tests/ and benches/ exempt; got {findings:?}"
        );
    }

    #[test]
    fn no_panic_skips_test_support_modules() {
        let conn = fresh_db();
        seed_function(
            &conn,
            2,
            "crates/core/src/service/test_support.rs",
            10,
            "MarketplaceService::stub",
            false,
        );
        insert_panic_ref(&conn, 2, 10, 42, "unimplemented");

        let findings = no_panic().run(&conn).expect("query should succeed");
        assert!(
            findings.is_empty(),
            "test_support modules exempt; got {findings:?}"
        );
    }

    // ─── non-exhaustive-error-enum ──────────────────────────────────────

    fn non_exhaustive() -> &'static Gate {
        ALL_GATES
            .iter()
            .find(|g| g.name == "non-exhaustive-error-enum")
            .expect("non-exhaustive gate registered")
    }

    fn seed_error_enum(
        conn: &Connection,
        sym_id: i64,
        path: &str,
        name: &str,
        with_non_exhaustive: bool,
    ) {
        conn.execute(
            "INSERT OR IGNORE INTO files (id, path, language) VALUES (2, ?1, 'rust')",
            [path],
        )
        .expect("file");
        conn.execute(
            "INSERT INTO symbols (id, file_id, name, qualified_name, kind, line, visibility)
             VALUES (?1, 2, ?2, ?2, 'enum', 1, 'public')",
            rusqlite::params![sym_id, name],
        )
        .expect("enum");
        if with_non_exhaustive {
            conn.execute(
                "INSERT INTO attributes (symbol_id, name, args, line)
                 VALUES (?1, 'non_exhaustive', NULL, 1)",
                [sym_id],
            )
            .expect("attr");
        }
    }

    #[test]
    fn non_exhaustive_flags_pub_error_enum_without_attribute() {
        let conn = fresh_db();
        seed_error_enum(
            &conn,
            10,
            "crates/kiro-market-core/src/error.rs",
            "PluginError",
            false,
        );
        let findings = non_exhaustive().run(&conn).expect("query");
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].qualified_name, "PluginError");
    }

    #[test]
    fn non_exhaustive_passes_when_attribute_present() {
        let conn = fresh_db();
        seed_error_enum(
            &conn,
            10,
            "crates/kiro-market-core/src/error.rs",
            "PluginError",
            true,
        );
        let findings = non_exhaustive().run(&conn).expect("query");
        assert!(
            findings.is_empty(),
            "enum with #[non_exhaustive] is exempt; got {findings:?}"
        );
    }

    #[test]
    fn non_exhaustive_only_scopes_to_kiro_market_core() {
        // A pub Error enum in the Tauri crate or CLI binary doesn't carry
        // the same downstream-stability requirement; this gate only fires
        // on `kiro-market-core` symbols.
        let conn = fresh_db();
        seed_error_enum(
            &conn,
            10,
            "crates/kiro-control-center/src-tauri/src/error.rs",
            "CommandError",
            false,
        );
        let findings = non_exhaustive().run(&conn).expect("query");
        assert!(
            findings.is_empty(),
            "non-core crates exempt from non-exhaustive rule; got {findings:?}"
        );
    }

    #[test]
    fn non_exhaustive_ignores_non_error_enums() {
        // Heuristic: only enums whose name ends in `Error` are subject to
        // this rule. `Status`, `Kind`, etc. don't trigger.
        let conn = fresh_db();
        seed_error_enum(
            &conn,
            10,
            "crates/kiro-market-core/src/lib.rs",
            "Status",
            false,
        );
        let findings = non_exhaustive().run(&conn).expect("query");
        assert!(findings.is_empty());
    }

    // ─── no-frontend-deps-in-core ───────────────────────────────────────

    fn no_frontend_deps() -> &'static Gate {
        ALL_GATES
            .iter()
            .find(|g| g.name == "no-frontend-deps-in-core")
            .expect("no-frontend-deps gate registered")
    }

    fn seed_import(conn: &Connection, file_id: i64, path: &str, symbol: &str, source: &str) {
        conn.execute(
            "INSERT OR IGNORE INTO files (id, path, language) VALUES (?1, ?2, 'rust')",
            rusqlite::params![file_id, path],
        )
        .expect("file");
        conn.execute(
            "INSERT INTO imports (file_id, symbol_name, source_module) VALUES (?1, ?2, ?3)",
            rusqlite::params![file_id, symbol, source],
        )
        .expect("import");
    }

    #[test]
    fn no_frontend_deps_flags_tauri_in_core() {
        let conn = fresh_db();
        seed_import(
            &conn,
            2,
            "crates/kiro-market-core/src/foo.rs",
            "Manager",
            "tauri::Manager",
        );

        let findings = no_frontend_deps().run(&conn).expect("query");
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].signature.as_deref(), Some("tauri::Manager"));
    }

    #[test]
    fn no_frontend_deps_flags_tokio_in_core() {
        let conn = fresh_db();
        seed_import(
            &conn,
            2,
            "crates/kiro-market-core/src/foo.rs",
            "spawn",
            "tokio::spawn",
        );

        let findings = no_frontend_deps().run(&conn).expect("query");
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].signature.as_deref(), Some("tokio::spawn"));
    }

    #[test]
    fn no_frontend_deps_does_not_flag_tauri_in_other_crates() {
        // Tauri imports are FINE in the Tauri crate itself.
        let conn = fresh_db();
        seed_import(
            &conn,
            2,
            "crates/kiro-control-center/src-tauri/src/lib.rs",
            "Manager",
            "tauri::Manager",
        );

        let findings = no_frontend_deps().run(&conn).expect("query");
        assert!(
            findings.is_empty(),
            "tauri imports outside core are fine; got {findings:?}"
        );
    }

    #[test]
    fn no_frontend_deps_does_not_flag_unrelated_crate_with_similar_name() {
        // `tauri_anything` would not start with `tauri::` so the LIKE
        // pattern excludes it. Defensive test in case someone adds a
        // crate named `tauri_extra` etc. in the future.
        let conn = fresh_db();
        seed_import(
            &conn,
            2,
            "crates/kiro-market-core/src/foo.rs",
            "thing",
            "taurus::thing", // similar but not tauri::
        );

        let findings = no_frontend_deps().run(&conn).expect("query");
        assert!(findings.is_empty());
    }

    #[test]
    fn no_frontend_deps_flags_tauri_plugin_imports() {
        let conn = fresh_db();
        seed_import(
            &conn,
            2,
            "crates/kiro-market-core/src/foo.rs",
            "init",
            "tauri_plugin_dialog::init",
        );

        let findings = no_frontend_deps().run(&conn).expect("query");
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn no_frontend_deps_flags_bare_tauri_import() {
        // Covers the `OR i.source_module = 'tauri'` SQL branch — `use
        // tauri;` (bare crate import, no `::path`). Existing tests only
        // exercise the `LIKE 'tauri::%'` branch, so a regression that
        // accidentally changed `=` to `LIKE` would slip past untested.
        let conn = fresh_db();
        seed_import(
            &conn,
            2,
            "crates/kiro-market-core/src/foo.rs",
            "tauri",
            "tauri",
        );

        let findings = no_frontend_deps().run(&conn).expect("query");
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].signature.as_deref(), Some("tauri"));
    }

    #[test]
    fn no_frontend_deps_flags_bare_tokio_import() {
        // Covers the `OR i.source_module = 'tokio'` SQL branch.
        let conn = fresh_db();
        seed_import(
            &conn,
            2,
            "crates/kiro-market-core/src/foo.rs",
            "tokio",
            "tokio",
        );

        let findings = no_frontend_deps().run(&conn).expect("query");
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].signature.as_deref(), Some("tokio"));
    }

    // ─── ffi-enum-serde-tag ─────────────────────────────────────────────

    fn ffi_tag_gate() -> &'static Gate {
        ALL_GATES
            .iter()
            .find(|g| g.name == "ffi-enum-serde-tag")
            .expect("ffi-enum-serde-tag gate registered")
    }

    /// Seed a `pub enum` row, optionally tagged with Serialize / specta
    /// derives and a serde directive, then attach the requested variants.
    /// `serde_args` is `None` for "no serde attribute"; `Some("tag = ...")`
    /// or `Some("untagged")` for the two TS-friendly tagging modes.
    /// `variants` carries `(name, signature_or_none)` — `None` is a unit
    /// variant; `Some("{ ... }")` is a struct/tuple variant.
    #[expect(
        clippy::too_many_arguments,
        reason = "knobs map 1:1 to the gate's predicate inputs; collapsing into a struct \
                  would obscure the per-test intent at the call site"
    )]
    fn seed_ffi_candidate(
        conn: &Connection,
        sym_id: i64,
        name: &str,
        visibility: &str,
        with_serialize: bool,
        with_specta: bool,
        serde_args: Option<&str>,
        variants: &[(&str, Option<&str>)],
    ) {
        conn.execute(
            "INSERT INTO symbols (id, file_id, name, qualified_name, kind, line, visibility)
             VALUES (?1, 1, ?2, ?2, 'enum', 1, ?3)",
            rusqlite::params![sym_id, name, visibility],
        )
        .expect("enum insert");
        if with_serialize {
            conn.execute(
                "INSERT INTO attributes (symbol_id, name, args, line)
                 VALUES (?1, 'derive', 'Clone, Debug, Serialize', 1)",
                rusqlite::params![sym_id],
            )
            .expect("derive(Serialize) insert");
        }
        if with_specta {
            // Mirrors the canonical codebase shape:
            // `#[cfg_attr(feature = "specta", derive(specta::Type))]`.
            conn.execute(
                "INSERT INTO attributes (symbol_id, name, args, line)
                 VALUES (?1, 'cfg_attr', 'feature = \"specta\", derive(specta::Type)', 1)",
                rusqlite::params![sym_id],
            )
            .expect("cfg_attr(specta) insert");
        }
        if let Some(args) = serde_args {
            conn.execute(
                "INSERT INTO attributes (symbol_id, name, args, line)
                 VALUES (?1, 'serde', ?2, 1)",
                rusqlite::params![sym_id, args],
            )
            .expect("serde insert");
        }
        for (i, (vname, vsig)) in variants.iter().enumerate() {
            let qualified = format!("{name}::{vname}");
            let line = 10 + u32::try_from(i).expect("test fixture index fits u32");
            conn.execute(
                "INSERT INTO symbols (file_id, name, qualified_name, kind, line, signature)
                 VALUES (1, ?1, ?2, 'enum_variant', ?3, ?4)",
                rusqlite::params![vname, qualified, line, vsig],
            )
            .expect("variant insert");
        }
    }

    #[test]
    fn ffi_enum_tag_flags_untagged_pub_enum_with_payload() {
        // The canonical SteeringWarning-shape bug pre-PR-83: pub enum,
        // Serialize + specta, has struct variants, no serde tag.
        let conn = fresh_db();
        seed_ffi_candidate(
            &conn,
            1,
            "Warning",
            "public",
            true,
            true,
            None,
            &[
                ("ScanPathInvalid", Some("{ path: PathBuf, reason: String }")),
                (
                    "ScanDirUnreadable",
                    Some("{ path: PathBuf, reason: String }"),
                ),
            ],
        );

        let findings = ffi_tag_gate().run(&conn).expect("query");
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].qualified_name, "Warning");
    }

    #[test]
    fn ffi_enum_tag_passes_when_serde_tag_present() {
        let conn = fresh_db();
        seed_ffi_candidate(
            &conn,
            1,
            "Warning",
            "public",
            true,
            true,
            Some("tag = \"kind\", rename_all = \"snake_case\""),
            &[("ScanPathInvalid", Some("{ path: PathBuf, reason: String }"))],
        );

        let findings = ffi_tag_gate().run(&conn).expect("query");
        assert!(
            findings.is_empty(),
            "explicit serde(tag = ...) satisfies the gate; got: {findings:?}"
        );
    }

    #[test]
    fn ffi_enum_tag_passes_when_untagged() {
        // SettingValue uses #[serde(untagged)] — produces a TS union of
        // primitive shapes without a discriminant. Also acceptable.
        let conn = fresh_db();
        seed_ffi_candidate(
            &conn,
            1,
            "Value",
            "public",
            true,
            true,
            Some("untagged"),
            &[("Bool", Some("(bool)")), ("Number", Some("(f64)"))],
        );

        let findings = ffi_tag_gate().run(&conn).expect("query");
        assert!(
            findings.is_empty(),
            "explicit serde(untagged) satisfies the gate; got: {findings:?}"
        );
    }

    #[test]
    fn ffi_enum_tag_exempts_unit_only_enum() {
        // SourceType / ErrorType / GitProtocol / etc. — every variant is
        // a bare identifier. Default external tagging serializes each
        // variant as a plain string, producing a clean TS union of
        // string literals. No serde(tag) directive is needed.
        let conn = fresh_db();
        seed_ffi_candidate(
            &conn,
            1,
            "SourceType",
            "public",
            true,
            true,
            None,
            &[("GitHub", None), ("Git", None), ("Local", None)],
        );

        let findings = ffi_tag_gate().run(&conn).expect("query");
        assert!(
            findings.is_empty(),
            "unit-only enums are TS-friendly under default tagging; got: {findings:?}"
        );
    }

    #[test]
    fn ffi_enum_tag_exempts_non_serialize_enum() {
        // No `Serialize` derive means the type cannot be emitted by serde
        // at all — there is no wire format to worry about.
        let conn = fresh_db();
        seed_ffi_candidate(
            &conn,
            1,
            "Warning",
            "public",
            false,
            true,
            None,
            &[("Variant", Some("{ x: u32 }"))],
        );

        let findings = ffi_tag_gate().run(&conn).expect("query");
        assert!(
            findings.is_empty(),
            "non-Serialize enum doesn't cross FFI; got: {findings:?}"
        );
    }

    #[test]
    fn ffi_enum_tag_exempts_non_specta_enum() {
        // Serialize alone doesn't materialize the type in `bindings.ts`;
        // specta::Type is the FFI tell. An internal Serialize-only enum
        // is fine without serde tagging — JSON consumers in-process
        // tolerate the default shape.
        let conn = fresh_db();
        seed_ffi_candidate(
            &conn,
            1,
            "Internal",
            "public",
            true,
            false,
            None,
            &[("Variant", Some("{ x: u32 }"))],
        );

        let findings = ffi_tag_gate().run(&conn).expect("query");
        assert!(
            findings.is_empty(),
            "Serialize without specta doesn't cross FFI; got: {findings:?}"
        );
    }

    #[test]
    fn ffi_enum_tag_exempts_private_enum() {
        // The rule scopes to the *public* surface; pub(crate) and bare
        // (private) enums don't materialize in bindings even with the
        // derives present.
        let conn = fresh_db();
        seed_ffi_candidate(
            &conn,
            1,
            "Internal",
            "private",
            true,
            true,
            None,
            &[("Variant", Some("{ x: u32 }"))],
        );

        let findings = ffi_tag_gate().run(&conn).expect("query");
        assert!(
            findings.is_empty(),
            "private enum is not public API; got: {findings:?}"
        );
    }

    #[test]
    fn ffi_enum_tag_flags_unconditional_specta_derive() {
        // Less common shape: Serialize + specta::Type both inside one
        // unconditional `#[derive(...)]` (no cfg_attr feature gate).
        // Tethys stores this as a single `derive` attribute whose args
        // list includes `specta::Type`; the SQL must match this branch
        // as well as the `cfg_attr` form.
        let conn = fresh_db();
        conn.execute(
            "INSERT INTO symbols (id, file_id, name, qualified_name, kind, line, visibility)
             VALUES (1, 1, 'Foo', 'Foo', 'enum', 1, 'public')",
            [],
        )
        .expect("enum insert");
        conn.execute(
            "INSERT INTO attributes (symbol_id, name, args, line)
             VALUES (1, 'derive', 'Clone, Debug, Serialize, specta::Type', 1)",
            [],
        )
        .expect("derive insert");
        conn.execute(
            "INSERT INTO symbols (file_id, name, qualified_name, kind, line, signature)
             VALUES (1, 'V', 'Foo::V', 'enum_variant', 10, '{ x: u32 }')",
            [],
        )
        .expect("variant insert");

        let findings = ffi_tag_gate().run(&conn).expect("query");
        assert_eq!(
            findings.len(),
            1,
            "unconditional specta derive must also fire; got: {findings:?}"
        );
    }

    #[test]
    fn ffi_enum_tag_flags_enum_nested_in_module() {
        // gemini-code-assist (PR #91): for `mod foo { pub enum Bar { ... } }`,
        // `s.name = 'Bar'` but variants land at `qualified_name =
        // 'foo::Bar::Variant'`. Anchoring the variant→parent link on
        // `s.qualified_name || '::%'` instead of `s.name || '::%'` makes
        // nesting depth irrelevant. Without this fix the enum looks
        // unit-only to the gate (variant EXISTS clause never matches),
        // silently exempting nested-module FFI enums from the rule.
        let conn = fresh_db();
        conn.execute(
            "INSERT INTO symbols (id, file_id, name, qualified_name, kind, line, visibility)
             VALUES (1, 1, 'Inner', 'outer::Inner', 'enum', 1, 'public')",
            [],
        )
        .expect("nested enum insert");
        conn.execute(
            "INSERT INTO attributes (symbol_id, name, args, line)
             VALUES (1, 'derive', 'Clone, Debug, Serialize', 1),
                    (1, 'cfg_attr', 'feature = \"specta\", derive(specta::Type)', 1)",
            [],
        )
        .expect("derives insert");
        conn.execute(
            "INSERT INTO symbols (file_id, name, qualified_name, kind, line, signature)
             VALUES (1, 'PayloadVariant', 'outer::Inner::PayloadVariant', 'enum_variant', 10,
                     '{ value: u32 }')",
            [],
        )
        .expect("nested variant insert");

        let findings = ffi_tag_gate().run(&conn).expect("query");
        assert_eq!(
            findings.len(),
            1,
            "nested-module enum with payload must be flagged; got: {findings:?}"
        );
        assert_eq!(findings[0].qualified_name, "outer::Inner");
    }
}

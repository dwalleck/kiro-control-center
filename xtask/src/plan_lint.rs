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
        AND s.qualified_name LIKE parent.name || '::%'
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
];

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
    for gate in ALL_GATES {
        if let Some(name) = &opts.gate_filter
            && gate.name != name
        {
            continue;
        }
        let findings = gate.run(&conn)?;
        if findings.is_empty() {
            println!("{} OK", gate.name);
            continue;
        }
        total_findings += findings.len();
        println!(
            "{} — {} ({} finding{})",
            gate.name,
            gate.description,
            findings.len(),
            if findings.len() == 1 { "" } else { "s" },
        );
        for f in &findings {
            print_finding(f);
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
    println!("    {}:{}", f.path, f.line);
    println!("        {} : {}", f.qualified_name, signature);
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
}

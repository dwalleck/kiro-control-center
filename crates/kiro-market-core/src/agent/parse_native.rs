//! Parse native Kiro agent JSON files into [`NativeAgentBundle`] for the
//! validate-and-copy install path. This module deliberately does NOT
//! model the full Kiro agent schema — only the fields the install layer
//! acts on (`name`, `mcpServers`). The rest of the JSON is preserved as
//! `serde_json::Value` for read-only inspection, and the source bytes
//! are preserved verbatim for atomic copy-out at install time.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use thiserror::Error;

use crate::agent::types::McpServerConfig;
use crate::validation;

/// A parsed native Kiro agent ready for install. The only producer is
/// [`parse_native_kiro_agent_file`], which validates the JSON +
/// security-checks the source file before constructing this struct.
///
/// `#[non_exhaustive]` blocks external crates from forging instances
/// via struct literals — anyone outside this crate that needs a
/// `NativeAgentBundle` must go through the parser, getting the
/// validation guarantees with it.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct NativeAgentBundle {
    /// Absolute path to the source `.json` file.
    pub agent_json_source: PathBuf,
    /// The scan root (e.g. `<plugin>/agents/`) the JSON was discovered under.
    /// Used for computing destination-relative paths and for hashing.
    pub scan_root: PathBuf,
    /// Validated agent name (from JSON `name` field). Path-safe per
    /// [`validation::validate_name`].
    pub name: String,
    /// MCP server entries from the JSON's `mcpServers` field. Empty if the
    /// field is absent or empty. Drives the `--accept-mcp` install gate.
    pub mcp_servers: BTreeMap<String, McpServerConfig>,
    /// Parsed JSON, used for projection / validation only. Not the source
    /// of truth for what lands on disk.
    pub raw_json: serde_json::Value,
    /// Source bytes preserved exactly. The install path writes these to
    /// the destination so the installed file matches the source byte-for-byte
    /// (per design doc: v1 preserves verbatim).
    pub raw_bytes: Vec<u8>,
}

/// Maximum size in bytes of a native Kiro agent JSON file. 1 MiB is ~10x
/// the largest realistic agent JSON (multi-prompt embeds rarely exceed
/// 100 KB). The cap is enforced before [`std::fs::read`] allocates, so a
/// hostile multi-GB `agents/big.json` cannot OOM the parser.
pub const MAX_NATIVE_AGENT_BYTES: u64 = 1024 * 1024;

/// Failure modes for [`parse_native_kiro_agent_file`]. Mirrors the existing
/// [`super::ParseFailure`] for translated agents — structured variants
/// instead of free-form strings, so callers can branch on the semantic.
///
/// `IoError` and `InvalidJson` carry their underlying cause via `#[source]`
/// so [`crate::error::error_full_chain`] walks past the wrapper at terminal
/// rendering surfaces.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum NativeParseFailure {
    /// File could not be read (permission denied, racy delete, etc.).
    #[error("read failed")]
    IoError(#[source] io::Error),
    /// Source path resolved to a symlink. Following it could leak host
    /// files into the install pipeline (the bytes get written verbatim to
    /// `.kiro/agents/`), so the parser refuses. Discovery already filters
    /// these, but the re-check here closes the TOCTOU window between
    /// discovery's stat and parse's read.
    #[error("refusing to follow symlink at `{0}`")]
    SymlinkRefused(PathBuf),
    /// Source file is a hardlink (Unix `nlink > 1`). The other path(s)
    /// sharing the inode could be sensitive host files (`~/.ssh/id_rsa`,
    /// etc.); the parser refuses rather than write inode contents to
    /// `.kiro/agents/`. `symlink_metadata` doesn't catch this — symlinks
    /// redirect the path, hardlinks share the inode itself.
    ///
    /// This is the canonical statement of the hardlink threat model;
    /// the same defense fires at the steering and companion install
    /// staging boundaries (see `SteeringError::SourceHardlinked` and
    /// `stage_native_companion_files`) and refers back here.
    #[error("refusing hardlinked source at `{path}` (nlink={nlink})")]
    HardlinkRefused { path: PathBuf, nlink: u64 },
    /// Source file exceeds [`MAX_NATIVE_AGENT_BYTES`]. Refused before
    /// allocation to avoid OOM on hostile manifests.
    #[error("agent JSON exceeds size cap: {size} bytes (limit: {limit})")]
    FileTooLarge { size: u64, limit: u64 },
    /// File is not valid JSON.
    #[error("invalid JSON")]
    InvalidJson(#[source] serde_json::Error),
    /// JSON parsed but a string value contains a NUL byte (the JSON
    /// `\u0000` escape). Carries the JSON pointer of the offending field.
    /// NUL bytes break C-string boundaries in downstream tooling and have
    /// no legitimate use in Kiro agent JSON.
    #[error("NUL byte in JSON string at `{json_pointer}`")]
    NulByteInJsonString { json_pointer: String },
    /// JSON parsed but the required `name` field is missing.
    #[error("missing required `name` field")]
    MissingName,
    /// `name` field is present but failed [`validation::validate_name`].
    /// Carries the validator's reason.
    #[error("invalid `name`: {0}")]
    InvalidName(String),
}

/// The minimal projection we read out of the JSON to validate + classify
/// the agent. Everything else stays in `raw_json`.
#[derive(Deserialize)]
struct NativeAgentProjection {
    name: Option<String>,
    #[serde(default, rename = "mcpServers")]
    mcp_servers: BTreeMap<String, McpServerConfig>,
}

/// Parse a candidate native Kiro agent JSON file.
///
/// Hardening steps before allocation:
/// 1. `symlink_metadata` re-check refuses symlinks (narrows the TOCTOU
///    window between discovery's filter and this read).
/// 2. `nlink > 1` rejects hardlinked sources on Unix (see
///    [`NativeParseFailure::HardlinkRefused`] for the threat model).
/// 3. Size cap rejects files exceeding [`MAX_NATIVE_AGENT_BYTES`] before
///    `fs::read` allocates a `Vec<u8>` for them.
///
/// # Residual TOCTOU window
///
/// A microsecond-scale window exists between the `symlink_metadata`
/// re-check and `fs::read` where a sub-process attacker could swap a
/// regular file for a symlink and have the read follow it. Closing
/// this fully requires opening with `O_NOFOLLOW` on Unix and
/// `FILE_FLAG_OPEN_REPARSE_POINT` on Windows; neither has portable
/// std support, so a `libc` direct dep or hardcoded ABI constants
/// would be needed. Tracked at
/// <https://github.com/dwalleck/kiro-control-center/issues/65>: the
/// practical exploit requires filesystem-race timing precision
/// against a parse that completes in tens of microseconds, and the
/// upstream discovery filter
/// ([`crate::platform::is_reparse_or_symlink`] + `read_dir`) already
/// removes symlinks at scan time, so a successful attack would have
/// to pre-place a regular file AND time the swap inside the parse
/// window.
///
/// Then reads the bytes, parses into both `serde_json::Value` (preserved
/// for projection / install) and [`NativeAgentProjection`] (typed field
/// access). Walks the parsed value for NUL bytes inside string values —
/// `serde_json` permits the `\u0000` escape, but NUL has no legitimate use in
/// agent JSON and breaks C-string boundaries in downstream tooling.
///
/// # Errors
///
/// Returns [`NativeParseFailure`] for any failure. Callers route the
/// failure into a typed [`crate::error::AgentError`] variant at the
/// install boundary.
pub fn parse_native_kiro_agent_file(
    json_path: &Path,
    scan_root: &Path,
) -> Result<NativeAgentBundle, NativeParseFailure> {
    let md = std::fs::symlink_metadata(json_path).map_err(NativeParseFailure::IoError)?;
    if crate::platform::is_reparse_or_symlink(&md) {
        return Err(NativeParseFailure::SymlinkRefused(json_path.to_path_buf()));
    }
    // Refuse hardlinked sources on Unix; see `NativeParseFailure::HardlinkRefused`
    // for the threat model. Windows has no portable nlink accessor in std;
    // platform.rs's reparse-point check covers Windows hardlinks via
    // junction handling.
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if md.is_file() && md.nlink() > 1 {
            return Err(NativeParseFailure::HardlinkRefused {
                path: json_path.to_path_buf(),
                nlink: md.nlink(),
            });
        }
    }
    if md.len() > MAX_NATIVE_AGENT_BYTES {
        return Err(NativeParseFailure::FileTooLarge {
            size: md.len(),
            limit: MAX_NATIVE_AGENT_BYTES,
        });
    }

    let raw_bytes = std::fs::read(json_path).map_err(NativeParseFailure::IoError)?;
    let raw_json: serde_json::Value =
        serde_json::from_slice(&raw_bytes).map_err(NativeParseFailure::InvalidJson)?;
    if let Some(json_pointer) = first_nul_in_strings(&raw_json) {
        return Err(NativeParseFailure::NulByteInJsonString { json_pointer });
    }
    let projection: NativeAgentProjection =
        serde_json::from_slice(&raw_bytes).map_err(NativeParseFailure::InvalidJson)?;

    let name = projection.name.ok_or(NativeParseFailure::MissingName)?;
    validation::validate_name(&name).map_err(|e| NativeParseFailure::InvalidName(e.to_string()))?;

    Ok(NativeAgentBundle {
        agent_json_source: json_path.to_path_buf(),
        scan_root: scan_root.to_path_buf(),
        name,
        mcp_servers: projection.mcp_servers,
        raw_json,
        raw_bytes,
    })
}

/// Walk `value` and return the JSON pointer of the first string value
/// containing a NUL byte, if any. Returns `None` if all strings (values
/// AND object keys) are NUL-free. Recurses into objects and arrays;
/// non-string scalars are skipped.
///
/// Object keys are checked in addition to string values because
/// downstream tooling (MCP server name lookups, telemetry) may treat
/// keys as C-strings and truncate at the NUL — a key like
/// `"tool evil"` could match `"tool"` in some consumers.
fn first_nul_in_strings(value: &serde_json::Value) -> Option<String> {
    fn walk(value: &serde_json::Value, path: &mut String) -> Option<String> {
        match value {
            serde_json::Value::String(s) => {
                if s.as_bytes().contains(&0) {
                    Some(if path.is_empty() {
                        "/".to_string()
                    } else {
                        path.clone()
                    })
                } else {
                    None
                }
            }
            serde_json::Value::Array(arr) => {
                for (i, v) in arr.iter().enumerate() {
                    let saved_len = path.len();
                    path.push('/');
                    path.push_str(&i.to_string());
                    if let Some(p) = walk(v, path) {
                        return Some(p);
                    }
                    path.truncate(saved_len);
                }
                None
            }
            serde_json::Value::Object(map) => {
                for (k, v) in map {
                    // Reject NUL in the key itself before recursing.
                    // The pointer reported is the path UP TO this key's
                    // parent, with `/<key>` appended — accurate even
                    // though the failure isn't on a value.
                    if k.as_bytes().contains(&0) {
                        let saved_len = path.len();
                        path.push('/');
                        let escaped = k.replace('~', "~0").replace('/', "~1");
                        path.push_str(&escaped);
                        // path is non-empty here: we just pushed '/' + escaped key.
                        let result = path.clone();
                        path.truncate(saved_len);
                        return Some(result);
                    }
                    let saved_len = path.len();
                    path.push('/');
                    // RFC 6901: escape `~` and `/` in keys.
                    let escaped = k.replace('~', "~0").replace('/', "~1");
                    path.push_str(&escaped);
                    if let Some(p) = walk(v, path) {
                        return Some(p);
                    }
                    path.truncate(saved_len);
                }
                None
            }
            _ => None,
        }
    }
    let mut path = String::new();
    walk(value, &mut path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn write_json(dir: &Path, name: &str, body: &str) -> PathBuf {
        let p = dir.join(name);
        fs::write(&p, body).expect("write fixture");
        p
    }

    #[test]
    fn parses_minimal_valid_kiro_agent() {
        let tmp = tempdir().unwrap();
        let p = write_json(
            tmp.path(),
            "rev.json",
            r#"{"name": "rev", "prompt": "..."}"#,
        );
        let b = parse_native_kiro_agent_file(&p, tmp.path()).expect("parse");
        assert_eq!(b.name, "rev");
        assert!(b.mcp_servers.is_empty());
        assert_eq!(b.scan_root, tmp.path());
        assert_eq!(b.raw_bytes, br#"{"name": "rev", "prompt": "..."}"#);
    }

    #[test]
    fn missing_name_returns_missing_name_failure() {
        let tmp = tempdir().unwrap();
        let p = write_json(tmp.path(), "x.json", r#"{"prompt": "hi"}"#);
        let err = parse_native_kiro_agent_file(&p, tmp.path()).expect_err("must fail");
        assert!(matches!(err, NativeParseFailure::MissingName));
    }

    #[test]
    fn invalid_name_returns_invalid_name_failure() {
        let tmp = tempdir().unwrap();
        let p = write_json(tmp.path(), "x.json", r#"{"name": "../evil"}"#);
        let err = parse_native_kiro_agent_file(&p, tmp.path()).expect_err("must fail");
        match err {
            NativeParseFailure::InvalidName(reason) => {
                assert!(!reason.is_empty(), "reason must not be empty");
            }
            other => panic!("expected InvalidName, got {other:?}"),
        }
    }

    #[test]
    fn malformed_json_returns_invalid_json_failure() {
        let tmp = tempdir().unwrap();
        let p = write_json(tmp.path(), "x.json", r"{not json");
        let err = parse_native_kiro_agent_file(&p, tmp.path()).expect_err("must fail");
        assert!(matches!(err, NativeParseFailure::InvalidJson(_)));
    }

    #[test]
    fn extracts_mcp_servers_field() {
        let tmp = tempdir().unwrap();
        let p = write_json(
            tmp.path(),
            "with_mcp.json",
            r#"{
                "name": "x",
                "mcpServers": {
                    "tool": { "type": "stdio", "command": "echo", "args": ["hi"] }
                }
            }"#,
        );
        let b = parse_native_kiro_agent_file(&p, tmp.path()).expect("parse");
        assert_eq!(b.mcp_servers.len(), 1);
        assert!(b.mcp_servers["tool"].is_stdio());
    }

    #[test]
    fn missing_file_returns_io_error() {
        let tmp = tempdir().unwrap();
        let nonexistent = tmp.path().join("nope.json");
        let err = parse_native_kiro_agent_file(&nonexistent, tmp.path()).expect_err("must fail");
        assert!(matches!(err, NativeParseFailure::IoError(_)));
    }

    #[test]
    fn io_error_exposes_source_chain() {
        use std::error::Error as _;
        let tmp = tempdir().unwrap();
        let nonexistent = tmp.path().join("nope.json");
        let err = parse_native_kiro_agent_file(&nonexistent, tmp.path()).expect_err("must fail");
        assert_eq!(err.to_string(), "read failed");
        let source = err.source().expect("source chain populated by #[source]");
        assert!(source.to_string().to_lowercase().contains("file"));
    }

    #[test]
    fn invalid_json_exposes_source_chain() {
        use std::error::Error as _;
        let tmp = tempdir().unwrap();
        let p = write_json(tmp.path(), "x.json", r"{not json");
        let err = parse_native_kiro_agent_file(&p, tmp.path()).expect_err("must fail");
        assert_eq!(err.to_string(), "invalid JSON");
        let source = err.source().expect("source chain populated by #[source]");
        // serde_json error message references the line / column.
        assert!(!source.to_string().is_empty());
    }

    #[test]
    fn raw_bytes_preserves_source_verbatim() {
        let tmp = tempdir().unwrap();
        // Non-canonical whitespace + field ordering. raw_bytes must match.
        let body = b"{\n  \"name\":   \"rev\",\n     \"prompt\":\"x\"\n}\n";
        let p = tmp.path().join("rev.json");
        fs::write(&p, body).expect("write");
        let b = parse_native_kiro_agent_file(&p, tmp.path()).expect("parse");
        assert_eq!(b.raw_bytes.as_slice(), body.as_slice());
    }

    #[test]
    fn file_exceeding_size_cap_is_refused() {
        let tmp = tempdir().unwrap();
        let p = tmp.path().join("huge.json");
        // The size check runs before fs::read allocates, so the bytes
        // don't need to be valid JSON — just over the threshold.
        let cap_usize = usize::try_from(MAX_NATIVE_AGENT_BYTES)
            .expect("MAX_NATIVE_AGENT_BYTES fits in usize on test platforms");
        let oversized = vec![b' '; cap_usize + 1];
        fs::write(&p, &oversized).expect("write oversized fixture");
        let err = parse_native_kiro_agent_file(&p, tmp.path()).expect_err("must fail");
        match err {
            NativeParseFailure::FileTooLarge { size, limit } => {
                assert_eq!(limit, MAX_NATIVE_AGENT_BYTES);
                assert!(size > limit);
            }
            other => panic!("expected FileTooLarge, got {other:?}"),
        }
    }

    #[cfg(unix)]
    #[test]
    fn symlink_at_parse_time_is_refused() {
        use std::os::unix::fs::symlink;
        let tmp = tempdir().unwrap();
        let target = tmp.path().join("real.json");
        fs::write(&target, br#"{"name":"real"}"#).expect("write target");
        let link = tmp.path().join("link.json");
        symlink(&target, &link).expect("create symlink");

        let err = parse_native_kiro_agent_file(&link, tmp.path()).expect_err("must fail");
        assert!(matches!(err, NativeParseFailure::SymlinkRefused(_)));
    }

    #[cfg(unix)]
    #[test]
    fn hardlink_at_parse_time_is_refused() {
        // A hardlink shares an inode with another path — could be
        // ~/.ssh/id_rsa or any sensitive host file. symlink_metadata
        // returns "regular file" because the inode IS the data, not a
        // redirect; only the nlink count reveals the share. Defense-
        // in-depth gap flagged by marketplace-security-reviewer.
        let tmp = tempdir().unwrap();
        let original = tmp.path().join("real.json");
        fs::write(&original, br#"{"name":"real"}"#).expect("write original");
        let linked = tmp.path().join("linked.json");
        fs::hard_link(&original, &linked).expect("create hardlink");

        let err = parse_native_kiro_agent_file(&linked, tmp.path())
            .expect_err("hardlinked source must be refused");
        match err {
            NativeParseFailure::HardlinkRefused { path, nlink } => {
                assert_eq!(path, linked);
                assert!(nlink >= 2, "nlink must reflect the hardlink share");
            }
            other => panic!("expected HardlinkRefused, got {other:?}"),
        }
    }

    /// Six-char JSON escape sequence that parses to a NUL code point.
    /// Embedded via `format!` so the source file never contains literal
    /// raw NUL bytes — the threat vector is the JSON escape, not raw NUL
    /// (which `serde_json` rejects at parse time per RFC 8259).
    const NUL_ESC: &str = "\\u0000";

    #[test]
    fn nul_byte_in_top_level_string_field_is_refused() {
        let tmp = tempdir().unwrap();
        let body = format!(r#"{{"name":"rev","prompt":"hello{NUL_ESC}world"}}"#);
        let p = write_json(tmp.path(), "rev.json", &body);
        let err = parse_native_kiro_agent_file(&p, tmp.path()).expect_err("must fail");
        match err {
            NativeParseFailure::NulByteInJsonString { json_pointer } => {
                assert_eq!(json_pointer, "/prompt");
            }
            other => panic!("expected NulByteInJsonString, got {other:?}"),
        }
    }

    #[test]
    fn nul_byte_in_object_key_is_refused() {
        // Closes a gemini-bot review finding: the original NUL check
        // walked string VALUES only, leaving keys unguarded. Downstream
        // tooling (MCP server-name lookups, telemetry) may treat keys
        // as C-strings — `"tool\0evil"` could match `"tool"` after
        // truncation. The pointer reports the offending key path.
        let tmp = tempdir().unwrap();
        let body = format!(
            r#"{{"name":"rev","mcpServers":{{"tool{NUL_ESC}evil":{{"type":"stdio","command":"sh","args":[]}}}}}}"#
        );
        let p = write_json(tmp.path(), "rev.json", &body);
        let err = parse_native_kiro_agent_file(&p, tmp.path()).expect_err("must fail");
        match err {
            NativeParseFailure::NulByteInJsonString { json_pointer } => {
                // Pointer ends at the offending key inside mcpServers.
                assert!(
                    json_pointer.starts_with("/mcpServers/tool"),
                    "pointer must reference the offending key, got: {json_pointer}"
                );
            }
            other => panic!("expected NulByteInJsonString, got {other:?}"),
        }
    }

    #[test]
    fn nul_byte_in_mcp_command_is_refused() {
        let tmp = tempdir().unwrap();
        let body = format!(
            r#"{{"name":"rev","mcpServers":{{"x":{{"type":"stdio","command":"sh{NUL_ESC}evil","args":[]}}}}}}"#
        );
        let p = write_json(tmp.path(), "rev.json", &body);
        let err = parse_native_kiro_agent_file(&p, tmp.path()).expect_err("must fail");
        match err {
            NativeParseFailure::NulByteInJsonString { json_pointer } => {
                assert_eq!(json_pointer, "/mcpServers/x/command");
            }
            other => panic!("expected NulByteInJsonString, got {other:?}"),
        }
    }

    #[test]
    fn json_pointer_escapes_slash_and_tilde_per_rfc6901() {
        let tmp = tempdir().unwrap();
        // Key contains `/` and `~` — RFC 6901 requires them escaped to
        // `~1` and `~0` respectively in the JSON pointer.
        let body = format!(r#"{{"name":"rev","a/b~c":"bad{NUL_ESC}"}}"#);
        let p = write_json(tmp.path(), "rev.json", &body);
        let err = parse_native_kiro_agent_file(&p, tmp.path()).expect_err("must fail");
        match err {
            NativeParseFailure::NulByteInJsonString { json_pointer } => {
                assert_eq!(json_pointer, "/a~1b~0c");
            }
            other => panic!("expected NulByteInJsonString, got {other:?}"),
        }
    }

    #[test]
    fn raw_nul_byte_in_json_source_is_rejected_by_parser() {
        // Sanity check: a literal NUL byte (not escape) inside a JSON
        // string is invalid JSON per RFC 8259. serde_json rejects it
        // before our NUL walk runs. This test pins that contract — if
        // serde_json ever loosens its parser, our `first_nul_in_strings`
        // check becomes load-bearing rather than belt-and-suspenders.
        let tmp = tempdir().unwrap();
        let body: &[u8] = b"{\"name\":\"rev\",\"prompt\":\"\x00\"}";
        let p = tmp.path().join("rev.json");
        fs::write(&p, body).expect("write fixture");
        let err = parse_native_kiro_agent_file(&p, tmp.path()).expect_err("must fail");
        assert!(matches!(err, NativeParseFailure::InvalidJson(_)));
    }
}

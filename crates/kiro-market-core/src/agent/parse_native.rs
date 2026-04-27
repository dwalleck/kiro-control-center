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

/// A parsed native Kiro agent ready for install.
#[derive(Debug, Clone)]
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
    /// File is not valid JSON.
    #[error("invalid JSON")]
    InvalidJson(#[source] serde_json::Error),
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
/// Reads the file once, parses the JSON twice (into `serde_json::Value` for
/// preservation and into [`NativeAgentProjection`] for typed field access).
/// The two-parse cost is negligible vs. I/O and avoids manual `Value` walks.
///
/// # Errors
///
/// Returns [`NativeParseFailure`] for any failure: I/O, malformed JSON,
/// missing `name`, or invalid `name`. Callers route the failure into a
/// typed [`crate::error::AgentError`] variant at the install boundary.
pub fn parse_native_kiro_agent_file(
    json_path: &Path,
    scan_root: &Path,
) -> Result<NativeAgentBundle, NativeParseFailure> {
    let raw_bytes = std::fs::read(json_path).map_err(NativeParseFailure::IoError)?;
    let raw_json: serde_json::Value =
        serde_json::from_slice(&raw_bytes).map_err(NativeParseFailure::InvalidJson)?;
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
}

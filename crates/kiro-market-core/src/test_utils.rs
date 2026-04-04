//! Shared test utilities for crates that depend on `kiro-market-core`.
//!
//! Available when the `test-support` feature is enabled or when running
//! the crate's own tests.

use std::path::Path;

/// Convert a local filesystem path into a valid `file://` URL on all
/// platforms.
///
/// On Windows, `Path::display()` produces backslashes and
/// `format!("file://{}")` yields `file://C:\...` which git rejects.
/// This helper normalises to forward slashes with the triple-slash form
/// required by RFC 8089.
///
/// # Examples
///
/// ```
/// # use std::path::Path;
/// # use kiro_market_core::test_utils::path_to_file_url;
/// assert_eq!(path_to_file_url(Path::new("/tmp/repo")), "file:///tmp/repo");
/// ```
#[must_use]
pub fn path_to_file_url(path: &Path) -> String {
    let s = path.display().to_string().replace('\\', "/");
    if s.starts_with('/') {
        format!("file://{s}")
    } else {
        // Windows: C:/foo → file:///C:/foo
        format!("file:///{s}")
    }
}

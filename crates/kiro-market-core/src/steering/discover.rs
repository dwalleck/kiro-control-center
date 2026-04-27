//! Discovery for steering files.
//!
//! Plugins ship steering markdown at the root of their declared steering
//! scan paths (default `./steering/`). Each `.md` file there installs into
//! `.kiro/steering/<filename>`. Mirrors the security model of the native
//! agent discovery: scan paths are validated against path traversal,
//! symlinks and reparse points are refused, and README/CONTRIBUTING/
//! CHANGELOG are excluded by name so plugins can keep docs in their
//! `steering/` directory without surfacing them as steering files.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use tracing::{debug, warn};

use crate::agent::DiscoveredNativeFile;
use crate::steering::SteeringWarning;

/// Filenames excluded from steering discovery (case-insensitive).
const EXCLUDED_FILENAMES: &[&str] = &["README.md", "CONTRIBUTING.md", "CHANGELOG.md"];

/// Find steering markdown candidates: `.md` files at the root of each
/// scan path. Reuses [`DiscoveredNativeFile`] so the install layer can
/// compute destination-relative paths without re-doing the join.
///
/// Returns `(files, warnings)`. `warnings` carries actionable
/// discovery-time issues (invalid scan paths, unreadable scan dirs)
/// the service layer threads into `InstallSteeringResult.warnings`
/// so the CLI can surface them. By-design exclusions (README files,
/// refused symlinks) stay as `tracing::debug!` only — surfacing them
/// would just be CLI noise.
///
/// Mirrors the security primitives of
/// [`crate::agent::discover_native_kiro_agents_in_dirs`]:
/// - Each scan path is validated by [`crate::validation::validate_relative_path`].
/// - `read_dir` `NotFound` silently yields empty; other I/O errors emit
///   a `ScanDirUnreadable` warning.
/// - `symlink_metadata` + [`crate::platform::is_reparse_or_symlink`] refuse
///   symlinks and Windows directory junctions.
/// - README / CONTRIBUTING / CHANGELOG `.md` excluded case-insensitively.
/// - Non-recursive at the scan-path level.
#[must_use]
pub fn discover_steering_files_in_dirs(
    plugin_dir: &Path,
    scan_paths: &[String],
) -> (Vec<DiscoveredNativeFile>, Vec<SteeringWarning>) {
    let mut out = Vec::new();
    let mut warnings = Vec::new();
    for rel in scan_paths {
        if let Err(e) = crate::validation::validate_relative_path(rel) {
            warn!(
                path = %rel,
                error = %e,
                "skipping steering scan path that fails validation"
            );
            // Extract just the reason text — the full ValidationError
            // Display embeds the raw path, which would re-introduce
            // attacker-controlled bytes into the warning AFTER the
            // SafeForTerminal wrapper sanitized the standalone path
            // field. The variant's `reason` is plain English describing
            // why the path was rejected.
            let reason = match &e {
                crate::error::ValidationError::InvalidRelativePath { reason, .. }
                | crate::error::ValidationError::InvalidName { reason, .. } => reason.clone(),
            };
            warnings.push(SteeringWarning::ScanPathInvalid {
                path: PathBuf::from(rel),
                reason,
            });
            continue;
        }
        let dir = plugin_dir.join(rel.trim_start_matches("./"));
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
            Err(e) => {
                warn!(
                    path = %dir.display(),
                    error = %e,
                    "failed to read steering scan directory; skipping"
                );
                warnings.push(SteeringWarning::ScanDirUnreadable {
                    path: dir.clone(),
                    reason: e.to_string(),
                });
                continue;
            }
        };
        for entry in entries {
            let entry = match entry {
                Ok(e) => e,
                Err(e) => {
                    // Demoted from `warn!` to `debug!`: the structured
                    // warning below is the user-facing channel; a flaky FS
                    // or intermittent EIO on a single entry doesn't need
                    // to be in two places. Symlink/reparse skips below
                    // also use `debug!` for the same reason.
                    debug!(
                        dir = %dir.display(),
                        error = %e,
                        "failed to read directory entry; skipping"
                    );
                    // Phrase the reason at the entry level — the directory
                    // itself is readable (we got `entries` above); only one
                    // entry failed. Saying "could not read steering scan
                    // directory <dir>: entry read failed: ..." misled users
                    // into checking the directory rather than the entry.
                    warnings.push(SteeringWarning::ScanDirUnreadable {
                        path: dir.clone(),
                        reason: format!("failed to read an entry in this directory: {e}"),
                    });
                    continue;
                }
            };
            let path = entry.path();
            let metadata = match fs::symlink_metadata(&path) {
                Ok(m) => m,
                Err(e) => {
                    debug!(
                        path = %path.display(),
                        error = %e,
                        "failed to stat steering candidate; skipping"
                    );
                    // Per-candidate stat failure is system-level and
                    // user-actionable (perm, broken FS); the structured
                    // warning is the user-facing channel.
                    warnings.push(SteeringWarning::ScanDirUnreadable {
                        path: path.clone(),
                        reason: format!("stat failed: {e}"),
                    });
                    continue;
                }
            };
            if crate::platform::is_reparse_or_symlink(&metadata) {
                debug!(
                    path = %path.display(),
                    "skipping symlink or reparse point in steering scan directory"
                );
                continue;
            }
            if !metadata.file_type().is_file() {
                continue;
            }
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if EXCLUDED_FILENAMES
                .iter()
                .any(|excluded| excluded.eq_ignore_ascii_case(name))
            {
                continue;
            }
            if Path::new(name)
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("md"))
            {
                out.push(DiscoveredNativeFile {
                    source: path,
                    scan_root: dir.clone(),
                });
            }
        }
    }
    (out, warnings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn finds_md_files_at_steering_root() {
        let tmp = tempdir().unwrap();
        let steering = tmp.path().join("steering");
        fs::create_dir_all(&steering).unwrap();
        fs::write(steering.join("guide.md"), b"guide").unwrap();
        fs::write(steering.join("not.txt"), b"ignored").unwrap();

        let (found, _warnings) =
            discover_steering_files_in_dirs(tmp.path(), &["./steering/".to_string()]);
        let names: Vec<_> = found
            .iter()
            .map(|f| f.source.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["guide.md"]);
    }

    #[test]
    fn returns_empty_when_directory_missing() {
        // A plugin commonly declares `./steering/` without authoring
        // any files; missing-directory must be a silent no-op (no
        // warning), distinct from `ScanDirUnreadable`.
        let tmp = tempdir().unwrap();
        let (found, warnings) =
            discover_steering_files_in_dirs(tmp.path(), &["./missing/".to_string()]);
        assert!(found.is_empty());
        assert!(
            warnings.is_empty(),
            "missing scan dir must not emit a warning: {warnings:?}"
        );
    }

    #[test]
    fn excludes_readme_case_insensitive() {
        let tmp = tempdir().unwrap();
        let steering = tmp.path().join("steering");
        fs::create_dir_all(&steering).unwrap();
        fs::write(steering.join("README.md"), b"readme").unwrap();
        fs::write(steering.join("readme.md"), b"lowercase").unwrap();
        fs::write(steering.join("real.md"), b"real").unwrap();

        let (found, _warnings) =
            discover_steering_files_in_dirs(tmp.path(), &["./steering/".to_string()]);
        let names: Vec<_> = found
            .iter()
            .map(|f| f.source.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["real.md"]);
    }

    #[test]
    fn rejects_path_traversal_and_emits_scan_path_invalid_warning() {
        let tmp = tempdir().unwrap();
        let plugin = tmp.path().join("plugin");
        fs::create_dir_all(&plugin).unwrap();
        let escape = tmp.path().join("escape");
        fs::create_dir_all(&escape).unwrap();
        fs::write(escape.join("loot.md"), b"loot").unwrap();

        let (found, warnings) =
            discover_steering_files_in_dirs(&plugin, &["../escape/".to_string()]);
        assert!(found.is_empty());
        // Triple-flagged review finding: ScanPathInvalid must reach the
        // user via result.warnings rather than vanishing into tracing!.
        assert_eq!(warnings.len(), 1, "expected one warning, got: {warnings:?}");
        match &warnings[0] {
            SteeringWarning::ScanPathInvalid { path, reason } => {
                assert_eq!(path, std::path::Path::new("../escape/"));
                assert!(
                    !reason.is_empty(),
                    "validation reason must carry information"
                );
            }
            other => panic!("expected ScanPathInvalid, got {other:?}"),
        }
    }

    #[cfg(unix)]
    #[test]
    fn skips_symlinks() {
        use std::os::unix::fs::symlink;
        let tmp = tempdir().unwrap();
        let steering = tmp.path().join("steering");
        fs::create_dir_all(&steering).unwrap();
        fs::write(steering.join("real.md"), b"real").unwrap();

        let outside = tmp.path().join("outside.md");
        fs::write(&outside, b"outside").unwrap();
        symlink(&outside, steering.join("evil.md")).unwrap();

        let (found, _warnings) =
            discover_steering_files_in_dirs(tmp.path(), &["./steering/".to_string()]);
        let names: Vec<_> = found
            .iter()
            .map(|f| f.source.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["real.md"]);
    }

    #[test]
    fn carries_scan_root_for_destination_computation() {
        let tmp = tempdir().unwrap();
        let steering = tmp.path().join("steering");
        fs::create_dir_all(&steering).unwrap();
        fs::write(steering.join("a.md"), b"a").unwrap();

        let (found, _warnings) =
            discover_steering_files_in_dirs(tmp.path(), &["./steering/".to_string()]);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].scan_root, steering);
    }

    #[test]
    fn scans_multiple_paths_independently() {
        // S3-11: multi-scan-root steering is allowed (unlike companion
        // bundles). Each scan root contributes files independently;
        // same-name files across roots will surface as a normal collision
        // at install time, not a discovery rejection.
        let tmp = tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("a")).unwrap();
        fs::create_dir_all(tmp.path().join("b")).unwrap();
        fs::write(tmp.path().join("a/alpha.md"), b"alpha").unwrap();
        fs::write(tmp.path().join("b/beta.md"), b"beta").unwrap();

        let (found, _warnings) =
            discover_steering_files_in_dirs(tmp.path(), &["./a/".to_string(), "./b/".to_string()]);
        let names: Vec<_> = found
            .iter()
            .map(|f| f.source.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert!(names.contains(&"alpha.md".to_string()));
        assert!(names.contains(&"beta.md".to_string()));
    }

    #[cfg(unix)]
    #[test]
    fn unreadable_scan_dir_emits_scan_dir_unreadable_warning() {
        // Permission-denied on the scan dir is a system-level failure
        // the user can act on (chmod, ACL); surface it via warnings,
        // not just tracing logs. Unix-only because chmod doesn't
        // translate to Windows ACLs.
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempdir().unwrap();
        let steering = tmp.path().join("steering");
        fs::create_dir_all(&steering).unwrap();
        // 0o000 strips read+exec so read_dir errors with PermissionDenied.
        fs::set_permissions(&steering, fs::Permissions::from_mode(0o000)).unwrap();

        let (found, warnings) =
            discover_steering_files_in_dirs(tmp.path(), &["./steering/".to_string()]);

        // Restore permissions so the tempdir cleanup can recurse.
        fs::set_permissions(&steering, fs::Permissions::from_mode(0o700)).unwrap();

        assert!(found.is_empty());
        assert_eq!(warnings.len(), 1, "expected one warning, got: {warnings:?}");
        assert!(matches!(
            &warnings[0],
            SteeringWarning::ScanDirUnreadable { path, .. } if path == &steering
        ));
    }
}

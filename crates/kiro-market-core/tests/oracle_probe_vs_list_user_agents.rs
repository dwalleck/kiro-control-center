//! Cross-cutting oracle check for slice S3 of agents-view:
//! `KiroProject::list_user_agents` must agree with the prove-it-prototype
//! probe.py on the synthetic mixed-lineage fixture at
//! `.agents-view/probe/fixture/`.
//!
//! This is the regression form of the oracle from
//! `.agents-view/probe/README.md`: the Python probe and PowerShell oracle
//! already AGREE on the fixture; this test pins that the Rust binary
//! continues to agree with them as the implementation evolves.
//!
//! If this test fails, S3's design claim C1 (list output specification)
//! and C2 (untyped JSON, never `parse_native`) have drifted. Stop and
//! surface — don't paper over.

use std::path::PathBuf;

use kiro_market_core::project::KiroProject;

/// Locate the workspace-relative probe fixture from
/// `CARGO_MANIFEST_DIR` (which is the kiro-market-core crate root).
fn probe_fixture_root() -> PathBuf {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    PathBuf::from(manifest_dir)
        .join("..")
        .join("..")
        .join(".agents-view")
        .join("probe")
        .join("fixture")
}

#[test]
fn list_user_agents_against_probe_fixture_agrees_with_probe_py() {
    let fixture_root = probe_fixture_root();
    assert!(
        fixture_root.exists(),
        "probe fixture not found at {} — was the workspace re-rooted?",
        fixture_root.display()
    );

    let project = KiroProject::new(fixture_root.clone());
    let rows = project
        .list_user_agents()
        .expect("list_user_agents on probe fixture");

    // Expected from probe.py (captured at
    // .agents-view/probe/falsifier-runs/c2-no-name.log). Three rows:
    // marketplace-tracked, no-name, user-authored — orphan-tracking is
    // excluded by both probe and binary.
    assert_eq!(
        rows.len(),
        3,
        "row count diverged from probe.py — orphan included or files missed"
    );

    let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(
        names,
        vec!["marketplace-tracked", "no-name", "user-authored"],
        "sort order or row set diverged from probe.py"
    );

    // Spot-check the tracked row's lineage matches the fixture's tracking
    // file. probe.py emits the same shape: marketplace + plugin + version
    // from the agents map entry.
    let tracked = &rows[0];
    let lin = tracked
        .lineage
        .as_ref()
        .expect("tracked row must have lineage");
    assert_eq!(lin.marketplace, "fixture-market");
    assert_eq!(lin.plugin, "fixture-plugin");
    assert_eq!(lin.version.as_deref(), Some("1.2.3"));

    // The no-name file has no `name` field — probe.py uses filename
    // stem; binary must do the same.
    let no_name = &rows[1];
    assert_eq!(no_name.name, "no-name", "name fell back to filename stem");
    assert!(no_name.lineage.is_none());

    // Counts match what probe.py computed for the fixture content.
    assert_eq!(tracked.tools_count, 3);
    assert_eq!(tracked.mcp_count, 1);
    assert_eq!(tracked.resources_count, 2);
    assert_eq!(tracked.hooks_count, 1);

    let user_authored = &rows[2];
    assert!(user_authored.lineage.is_none());
    assert_eq!(user_authored.tools_count, 2);
}

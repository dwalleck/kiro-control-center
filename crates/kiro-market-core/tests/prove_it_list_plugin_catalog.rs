//! prove-it-prototype probe for slice 1 of the `BrowseTab` redesign.
//!
//! Feature: a bulk `list_plugin_catalog_for_marketplace` that returns
//! `PluginCatalogEntry { skills, steering, agents }` per plugin with an
//! `installed: bool` flag per item.
//!
//! Smallest factual question: for each category, can the proposed
//! `(name, installed)` tuples be assembled from existing primitives,
//! and does the resulting `installed` flag agree with a filesystem-up
//! oracle?
//!
//! Probe: command-driven assembly via existing primitives.
//! Oracle: independent walk of `.kiro/{skills,steering,agents}/`.
//!
//! This file is a probe artifact, not a regression test. It is allowed
//! to use `.expect()` on test infrastructure — the goal is evidence,
//! not production hardening.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use kiro_market_core::DEFAULT_AGENT_PATHS;
use kiro_market_core::DEFAULT_STEERING_PATHS;
use kiro_market_core::agent::discover::discover_agents_in_dirs;
use kiro_market_core::project::KiroProject;
use kiro_market_core::service::test_support::{
    make_kiro_project, make_plugin_with_skills, relative_path_entry,
    seed_marketplace_with_registry, temp_service,
};
use kiro_market_core::steering::discover::discover_steering_files_in_dirs;

/// Filesystem oracle: directory entries directly under `<project>/.kiro/<sub>/`.
fn oracle_kiro_subdir(project_path: &str, sub: &str) -> BTreeSet<String> {
    let dir = Path::new(project_path).join(".kiro").join(sub);
    let Ok(entries) = fs::read_dir(&dir) else {
        return BTreeSet::new();
    };
    entries
        .filter_map(Result::ok)
        .filter_map(|e| e.file_name().into_string().ok())
        .collect()
}

#[test]
fn probe_skills_consistent_empty_state_agrees() {
    // Slice: an empty project against a populated marketplace.
    // Both the command's `installed` set and the disk oracle should be
    // empty — non-trivial because they agree on the join-key (skill
    // name) AND on cardinality.
    let (dir, svc) = temp_service();
    let project_path = make_kiro_project(dir.path());
    let entries = vec![relative_path_entry("alpha", "plugins/alpha")];
    let mp_path = seed_marketplace_with_registry(dir.path(), &svc, "mp", &entries);
    make_plugin_with_skills(&mp_path, "alpha", &["s1", "s2", "s3"]);

    let project = KiroProject::new(Path::new(&project_path).to_path_buf());
    let installed = project.load_installed().expect("load_installed");

    let result = svc
        .list_skills_for_plugin("mp", "alpha", &installed)
        .expect("list_skills_for_plugin");

    let probe: BTreeSet<String> = result
        .skills
        .iter()
        .filter(|s| s.installed)
        .map(|s| s.name.clone())
        .collect();
    let oracle: BTreeSet<String> = oracle_kiro_subdir(&project_path, "skills");

    assert_eq!(result.skills.len(), 3, "all 3 skills enumerated");
    assert_eq!(probe, oracle, "consistent-empty agreement");
    eprintln!("[skills/empty] probe={probe:?} oracle={oracle:?}");
}

#[test]
fn probe_skills_orphan_disk_dir_diverges() {
    // Hand-create `.kiro/skills/s1/` with no tracking entry. The probe
    // (tracking-keyed) should NOT mark s1 installed; the oracle (disk
    // walk) WILL see s1. This pins the load-bearing contract for the
    // design: `SkillInfo.installed` is tracking-file membership, NOT
    // disk presence.
    let (dir, svc) = temp_service();
    let project_path = make_kiro_project(dir.path());
    let entries = vec![relative_path_entry("alpha", "plugins/alpha")];
    let mp_path = seed_marketplace_with_registry(dir.path(), &svc, "mp", &entries);
    make_plugin_with_skills(&mp_path, "alpha", &["s1", "s2"]);

    fs::create_dir_all(Path::new(&project_path).join(".kiro/skills/s1"))
        .expect("create orphan skill dir");

    let project = KiroProject::new(Path::new(&project_path).to_path_buf());
    let installed = project.load_installed().expect("load_installed");
    let result = svc
        .list_skills_for_plugin("mp", "alpha", &installed)
        .expect("list_skills_for_plugin");

    let probe: BTreeSet<String> = result
        .skills
        .iter()
        .filter(|s| s.installed)
        .map(|s| s.name.clone())
        .collect();
    let oracle = oracle_kiro_subdir(&project_path, "skills");

    assert!(
        probe.is_empty(),
        "tracking-keyed: nothing reported installed"
    );
    assert_eq!(oracle, BTreeSet::from(["s1".to_owned()]), "disk says s1");
    eprintln!(
        "[skills/orphan-disk] probe={probe:?} oracle={oracle:?} \
         => DIVERGENCE BY DESIGN: installed = tracking_membership, not disk_presence"
    );
}

#[test]
fn probe_steering_no_service_layer_enumeration_exists() {
    // Slice: the design needs a `(name, installed)` tuple per steering
    // file. There is no `list_steering_for_plugin`; the closest is
    // low-level `discover_steering_files_in_dirs` which yields paths.
    // Build the tuple manually and oracle-check against disk.
    let (dir, _svc) = temp_service();
    let project_path = make_kiro_project(dir.path());
    let plugin_dir = dir.path().join("plugins").join("alpha");
    let steering_src = plugin_dir.join("steering");
    fs::create_dir_all(&steering_src).expect("create steering src");
    fs::write(steering_src.join("rules.md"), "rules\n").expect("write rules.md");
    fs::write(steering_src.join("style.md"), "style\n").expect("write style.md");

    let scan_paths: Vec<String> = DEFAULT_STEERING_PATHS.iter().map(|s| (*s).into()).collect();
    let (discovered, warnings) = discover_steering_files_in_dirs(&plugin_dir, &scan_paths);
    assert!(warnings.is_empty(), "no warnings on a clean fixture");

    let project = KiroProject::new(Path::new(&project_path).to_path_buf());
    let installed = project
        .load_installed_steering()
        .expect("load_installed_steering");

    let probe: BTreeSet<String> = discovered
        .iter()
        .filter_map(|f| f.source.file_name()?.to_str().map(str::to_owned))
        .filter(|name| installed.files.contains_key(Path::new(name)))
        .collect();
    let oracle = oracle_kiro_subdir(&project_path, "steering");

    let names_enumerated: BTreeSet<String> = discovered
        .iter()
        .filter_map(|f| f.source.file_name()?.to_str().map(str::to_owned))
        .collect();
    assert_eq!(
        names_enumerated,
        BTreeSet::from(["rules.md".to_owned(), "style.md".to_owned()]),
        "manual enumeration recovers both files"
    );
    assert_eq!(probe, oracle, "consistent-empty agreement (no installs)");
    eprintln!("[steering/empty] enumerated={names_enumerated:?} probe={probe:?} oracle={oracle:?}");
}

#[test]
fn probe_agents_no_service_layer_enumeration_exists() {
    // Slice: same shape as steering. discover_agents_in_dirs yields
    // PathBufs; the agent name lives inside the file (frontmatter for
    // markdown agents, JSON for native). The probe can't even produce
    // `name` without parsing the file — additional infrastructure the
    // design needs to invoke or build.
    let (dir, _svc) = temp_service();
    let project_path = make_kiro_project(dir.path());
    let plugin_dir = dir.path().join("plugins").join("alpha");
    let agents_src = plugin_dir.join("agents");
    fs::create_dir_all(&agents_src).expect("create agents src");
    fs::write(
        agents_src.join("reviewer.md"),
        "---\nname: reviewer\ndescription: x\n---\nbody",
    )
    .expect("write agent");

    let scan_paths: Vec<String> = DEFAULT_AGENT_PATHS.iter().map(|s| (*s).into()).collect();
    let discovered = discover_agents_in_dirs(&plugin_dir, &scan_paths);
    assert_eq!(discovered.len(), 1, "one agent file discovered");

    let project = KiroProject::new(Path::new(&project_path).to_path_buf());
    let installed = project
        .load_installed_agents()
        .expect("load_installed_agents");

    let oracle = oracle_kiro_subdir(&project_path, "agents");

    // The probe CANNOT produce `(name, installed)` from `discovered`
    // alone — parsing is required. Empty installed-set is the only
    // case the probe can confirm without re-implementing the parser
    // chain. This IS the finding: the design's slice 1 must invoke
    // (or hoist) the agent parsing helpers to derive item names.
    assert!(installed.agents.is_empty(), "no installs");
    assert!(oracle.is_empty(), "no .kiro/agents dir on disk");
    eprintln!(
        "[agents/empty] discovered_paths={} installed_names={} oracle={oracle:?} \
         => GAP: name extraction requires invoking agent parser, not just discovery",
        discovered.len(),
        installed.agents.len()
    );
}

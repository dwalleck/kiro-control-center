# Plugin-First Install — Phase 1 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make plugins the first-class user-facing object in BrowseTab and InstalledTab. Add a backend coordinator (`install_plugin`) and aggregator (`list_installed_plugins`) so the UI can install / list / remove a plugin in one action that bundles its skills + steering + agents.

**Architecture:** Backend stays content-type-first (`installed-skills.json`, `installed-steering.json`, `installed-agents.json` remain separate). The new plugin-level Tauri commands are *coordinators* that call the existing per-content APIs and aggregate. The UI shifts to plugin cards as the primary surface; the existing skill grid is preserved as a secondary drill-down.

**Tech Stack:** Rust (`kiro-market-core`, `kiro-control-center` Tauri crate), TypeScript bindings via specta, Svelte 5 frontend, rstest, Playwright e2e.

**Companion design doc:** `2026-04-29-plugin-first-install-design.md`

---

## File structure

| File | Status | Responsibility |
|---|---|---|
| `crates/kiro-market-core/src/service/mod.rs` | Modify | `install_plugin` orchestrator, `InstallPluginResult` struct |
| `crates/kiro-market-core/src/project.rs` | Modify | `installed_plugins()`, `remove_plugin()`, `InstalledPluginInfo`, `RemovePluginResult` |
| `crates/kiro-control-center/src-tauri/src/commands/agents.rs` | **New** | `install_plugin_agents` Tauri command + `_impl` (mirrors `commands/steering.rs`) |
| `crates/kiro-control-center/src-tauri/src/commands/plugins.rs` | **New** | `install_plugin`, `list_installed_plugins`, `remove_plugin` Tauri commands |
| `crates/kiro-control-center/src-tauri/src/commands/mod.rs` | Modify | Register `agents` and `plugins` modules |
| `crates/kiro-control-center/src-tauri/src/lib.rs` | Modify | Add new commands to `invoke_handler!` |
| `crates/kiro-control-center/src/lib/components/PluginCard.svelte` | **New** | Reusable plugin card |
| `crates/kiro-control-center/src/lib/components/BrowseTab.svelte` | Modify | Plugin-cards primary view + skill-grid secondary view + view toggle |
| `crates/kiro-control-center/src/lib/components/InstalledTab.svelte` | Modify | Plugins-grouped section + collapsible flat skills |
| `crates/kiro-control-center/src/lib/bindings.ts` | Regenerate | Auto-generated; run via `cargo test -p kiro-control-center --lib -- --ignored` |
| `crates/kiro-control-center/tests/e2e/app.spec.ts` | Modify | Plugin-install happy-path |

---

## Task 1: Core `install_plugin` orchestrator

**Files:**
- Modify: `crates/kiro-market-core/src/service/mod.rs` (add `InstallPluginResult` near other result types ~line 387 region; add `install_plugin` method on `MarketplaceService` near other `install_plugin_*` methods)
- Test: same file, `mod tests` block

- [ ] **Step 1: Write the failing test**

In `crates/kiro-market-core/src/service/mod.rs::tests` (find the existing `mod tests` near line 1928), add:

```rust
#[test]
fn install_plugin_runs_skills_steering_agents_in_one_call() {
    use crate::project::KiroProject;
    use crate::service::test_support::{
        make_plugin_with_skills, relative_path_entry, seed_marketplace_with_registry,
        temp_service,
    };
    use std::fs;

    let (dir, svc) = temp_service();
    let entries = vec![relative_path_entry("p", "plugins/p")];
    let mp_path = seed_marketplace_with_registry(dir.path(), &svc, "mp", &entries);
    let plugin_dir = mp_path.join("plugins/p");
    fs::create_dir_all(&plugin_dir).expect("plugin dir");
    // Skill, steering file, and an agent — exercise all three install paths.
    make_plugin_with_skills(&mp_path, "p", &["alpha"]);
    fs::write(
        plugin_dir.join("plugin.json"),
        br#"{"name": "p", "version": "1.0.0"}"#,
    )
    .expect("write plugin.json");
    fs::create_dir_all(plugin_dir.join("steering")).expect("steering dir");
    fs::write(plugin_dir.join("steering/guide.md"), "# guide\n").expect("steering");
    fs::create_dir_all(plugin_dir.join("agents")).expect("agents dir");
    fs::write(
        plugin_dir.join("agents/reviewer.md"),
        "---\nname: reviewer\ndescription: Reviews\n---\nBody.\n",
    )
    .expect("agent");

    let project_dir = tempfile::tempdir().expect("project tempdir");
    let project = KiroProject::new(project_dir.path().to_path_buf());

    let result = svc
        .install_plugin(&project, "mp", "p", InstallMode::New, false)
        .expect("install_plugin happy path");

    assert_eq!(result.plugin, "p");
    assert_eq!(result.version.as_deref(), Some("1.0.0"));
    let skills = result.skills.expect("skills attempted");
    assert_eq!(skills.installed, vec!["alpha".to_string()]);
    let steering = result.steering.expect("steering attempted");
    assert_eq!(steering.installed.len(), 1);
    let agents = result.agents.expect("agents attempted");
    assert_eq!(agents.installed.len(), 1);
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test -p kiro-market-core install_plugin_runs_skills_steering_agents_in_one_call -- --nocapture 2>&1 | tail -10
```

Expected: FAIL with `error[E0599]: no method named install_plugin found for struct MarketplaceService`.

- [ ] **Step 3: Add the result type**

Insert near the existing `pub struct InstallAgentsResult` (approx. line 387):

```rust
/// Aggregate result of [`MarketplaceService::install_plugin`] — the
/// outcome of running every install path a plugin declares (skills,
/// steering, agents) in one coordinated call.
///
/// `Option<...>` distinguishes "this content type wasn't applicable"
/// (`None` — plugin declares no agents at all) from "this content type
/// was attempted and yielded zero installs" (`Some` with empty
/// `installed`). Encoding the distinction in the type avoids a magic
/// `len() == 0` ambiguity at the UI layer.
#[derive(Debug, Default, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct InstallPluginResult {
    pub plugin: String,
    pub version: Option<String>,
    pub skills: Option<InstallSkillsResult>,
    pub steering: Option<InstallSteeringResult>,
    pub agents: Option<InstallAgentsResult>,
}
```

- [ ] **Step 4: Add the method on `MarketplaceService`**

Insert near `install_plugin_steering` (search for `pub fn install_plugin_steering`):

```rust
/// Install everything a plugin declares — skills, steering, agents
/// — in a single call. Aggregates the three per-type results.
///
/// Errors from `resolve_plugin_install_context` propagate (the
/// caller can't recover from "plugin not found"). Per-content-type
/// partial failures land in the corresponding sub-result without
/// aborting the other content types — same policy each
/// `install_plugin_*` already follows individually.
///
/// `accept_mcp` is forwarded to the agent install path; defaults
/// `false` at the caller for safety (matches the existing CLI
/// `--accept-mcp` opt-in semantic).
pub fn install_plugin(
    &self,
    project: &crate::project::KiroProject,
    marketplace: &str,
    plugin: &str,
    mode: InstallMode,
    accept_mcp: bool,
) -> Result<InstallPluginResult, Error> {
    let ctx = self.resolve_plugin_install_context(marketplace, plugin)?;
    let mp_path = self.marketplace_path(marketplace);

    let skills = if ctx.skill_dirs.is_empty() {
        None
    } else {
        Some(self.install_skills(
            project,
            &ctx.skill_dirs,
            &InstallFilter::All,
            mode,
            marketplace,
            plugin,
            ctx.version.as_deref(),
        ))
    };

    let steering = if ctx.steering_scan_paths.is_empty() {
        None
    } else {
        Some(MarketplaceService::install_plugin_steering(
            project,
            &ctx.plugin_dir,
            &ctx.steering_scan_paths,
            crate::steering::SteeringInstallContext {
                mode,
                marketplace,
                plugin,
                version: ctx.version.as_deref(),
            },
        ))
    };

    let agents = if ctx.agent_scan_paths.is_empty() {
        None
    } else {
        Some(self.install_plugin_agents(
            project,
            &ctx.plugin_dir,
            &ctx.agent_scan_paths,
            ctx.format,
            AgentInstallContext {
                mode,
                accept_mcp,
                marketplace,
                plugin,
                version: ctx.version.as_deref(),
            },
        ))
    };

    let _ = mp_path; // suppress unused if future additions don't need it
    Ok(InstallPluginResult {
        plugin: plugin.to_string(),
        version: ctx.version,
        skills,
        steering,
        agents,
    })
}
```

- [ ] **Step 5: Run test to verify it passes**

```bash
cargo test -p kiro-market-core install_plugin_runs_skills_steering_agents_in_one_call -- --nocapture 2>&1 | tail -10
```

Expected: PASS.

- [ ] **Step 6: Add JSON wire-format lock**

Mirror the `steering_warning_variants_json_shape` precedent. Add in the same `mod tests` block:

```rust
#[test]
fn install_plugin_result_json_shape_locks_optional_subresults() {
    let result = InstallPluginResult {
        plugin: "p".into(),
        version: Some("1.0.0".into()),
        skills: None,
        steering: None,
        agents: None,
    };
    let json = serde_json::to_value(&result).expect("serialize");
    assert_eq!(
        json,
        serde_json::json!({
            "plugin": "p",
            "version": "1.0.0",
            "skills": null,
            "steering": null,
            "agents": null,
        }),
        "Option fields must serialize as JSON null when None — frontend \
         consumers branch on this; switching to a tagged or omitted shape \
         silently breaks them.",
    );
}
```

- [ ] **Step 7: Run + commit**

```bash
cargo test -p kiro-market-core install_plugin -- --nocapture 2>&1 | tail -10
cargo clippy -p kiro-market-core --tests -- -D warnings
cargo fmt --all
```

Expected: all green. Then:

```bash
git add crates/kiro-market-core/src/service/mod.rs
git commit -m "feat(core): add install_plugin orchestrator + InstallPluginResult"
```

---

## Task 2: Core `installed_plugins()` aggregator

**Files:**
- Modify: `crates/kiro-market-core/src/project.rs`

- [ ] **Step 1: Write the failing test**

In `crates/kiro-market-core/src/project.rs::tests`:

```rust
#[test]
fn installed_plugins_groups_skills_steering_agents_by_marketplace_plugin_pair() {
    use chrono::Utc;
    let dir = tempfile::tempdir().expect("tempdir");
    let project = KiroProject::new(dir.path().to_path_buf());
    std::fs::create_dir_all(project.kiro_dir()).expect("kiro dir");

    // Hand-write the three tracking files. Two plugins, mixed content.
    let now = Utc::now();
    let skills_json = serde_json::json!({
        "skills": {
            "alpha": {
                "marketplace": "mp",
                "plugin": "plug-a",
                "version": "1.0.0",
                "installed_at": now,
                "source_hash": "deadbeef"
            }
        }
    });
    std::fs::write(
        project.kiro_dir().join("installed-skills.json"),
        serde_json::to_vec_pretty(&skills_json).unwrap(),
    )
    .expect("skills tracking");

    let steering_json = serde_json::json!({
        "files": {
            "guide.md": {
                "marketplace": "mp",
                "plugin": "plug-a",
                "version": "1.0.0",
                "installed_at": now,
                "source_hash": "cafebabe",
                "installed_hash": "cafebabe"
            },
            "review.md": {
                "marketplace": "mp",
                "plugin": "plug-b",
                "version": "0.5.0",
                "installed_at": now,
                "source_hash": "feedface",
                "installed_hash": "feedface"
            }
        }
    });
    std::fs::write(
        project.kiro_dir().join("installed-steering.json"),
        serde_json::to_vec_pretty(&steering_json).unwrap(),
    )
    .expect("steering tracking");

    let result = project.installed_plugins().expect("installed_plugins");
    assert_eq!(result.len(), 2, "two plugins expected");

    let plug_a = result
        .iter()
        .find(|p| p.plugin == "plug-a")
        .expect("plug-a present");
    assert_eq!(plug_a.skill_count, 1);
    assert_eq!(plug_a.steering_count, 1);
    assert_eq!(plug_a.agent_count, 0);
    assert_eq!(plug_a.installed_skills, vec!["alpha".to_string()]);

    let plug_b = result
        .iter()
        .find(|p| p.plugin == "plug-b")
        .expect("plug-b present");
    assert_eq!(plug_b.skill_count, 0);
    assert_eq!(plug_b.steering_count, 1);
    assert_eq!(plug_b.agent_count, 0);
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test -p kiro-market-core installed_plugins_groups -- --nocapture 2>&1 | tail -10
```

Expected: FAIL — method doesn't exist.

- [ ] **Step 3: Add the result type**

Near the other `Installed*` types in `project.rs` (around the `InstalledSteering` definition):

```rust
/// Aggregated view of a single installed plugin — the union of
/// what's tracked across `installed-skills.json`,
/// `installed-steering.json`, and `installed-agents.json` for a
/// given `(marketplace, plugin)` pair.
///
/// Returned by [`KiroProject::installed_plugins`]. The frontend
/// renders one row per `InstalledPluginInfo`.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct InstalledPluginInfo {
    pub marketplace: String,
    pub plugin: String,
    /// Highest version across the three content types — they may
    /// differ if installed at different times. The latest wins for
    /// "what version do I have?".
    pub installed_version: Option<String>,
    pub skill_count: u32,
    pub steering_count: u32,
    pub agent_count: u32,
    pub installed_skills: Vec<String>,
    pub installed_steering: Vec<std::path::PathBuf>,
    pub installed_agents: Vec<String>,
    pub earliest_install: chrono::DateTime<chrono::Utc>,
    pub latest_install: chrono::DateTime<chrono::Utc>,
}
```

- [ ] **Step 4: Add the method**

Inside `impl KiroProject` (near `load_installed`):

```rust
/// Aggregate the three installed-* tracking files into a per-plugin
/// view. Returns one [`InstalledPluginInfo`] per `(marketplace,
/// plugin)` pair that has at least one tracked entry. Returns an
/// empty vec if no tracking files exist (a fresh project).
///
/// Used by the InstalledTab UI to show "what plugins are installed"
/// without forcing the frontend to make three round-trips and
/// stitch the results client-side.
pub fn installed_plugins(&self) -> crate::error::Result<Vec<InstalledPluginInfo>> {
    use std::collections::BTreeMap;

    #[derive(Default)]
    struct Acc {
        version: Option<String>,
        skills: Vec<String>,
        steering: Vec<std::path::PathBuf>,
        agents: Vec<String>,
        earliest: Option<chrono::DateTime<chrono::Utc>>,
        latest: Option<chrono::DateTime<chrono::Utc>>,
    }

    let mut by_pair: BTreeMap<(String, String), Acc> = BTreeMap::new();

    let skills = self.load_installed()?;
    for (name, meta) in &skills.skills {
        let acc = by_pair
            .entry((meta.marketplace.clone(), meta.plugin.clone()))
            .or_default();
        acc.skills.push(name.clone());
        update_version_and_dates(acc, meta.version.as_deref(), meta.installed_at);
    }

    let steering = self.load_installed_steering()?;
    for (rel, meta) in &steering.files {
        let acc = by_pair
            .entry((meta.marketplace.clone(), meta.plugin.clone()))
            .or_default();
        acc.steering.push(rel.clone());
        update_version_and_dates(acc, meta.version.as_deref(), meta.installed_at);
    }

    let agents = self.load_installed_agents()?;
    for (name, meta) in &agents.agents {
        let acc = by_pair
            .entry((meta.marketplace.clone(), meta.plugin.clone()))
            .or_default();
        acc.agents.push(name.clone());
        update_version_and_dates(acc, meta.version.as_deref(), meta.installed_at);
    }

    Ok(by_pair
        .into_iter()
        .map(|((marketplace, plugin), acc)| {
            let now = chrono::Utc::now();
            InstalledPluginInfo {
                marketplace,
                plugin,
                installed_version: acc.version,
                skill_count: u32::try_from(acc.skills.len()).unwrap_or(u32::MAX),
                steering_count: u32::try_from(acc.steering.len()).unwrap_or(u32::MAX),
                agent_count: u32::try_from(acc.agents.len()).unwrap_or(u32::MAX),
                installed_skills: acc.skills,
                installed_steering: acc.steering,
                installed_agents: acc.agents,
                earliest_install: acc.earliest.unwrap_or(now),
                latest_install: acc.latest.unwrap_or(now),
            }
        })
        .collect())
}

fn update_version_and_dates(
    acc: &mut Acc,
    version: Option<&str>,
    installed_at: chrono::DateTime<chrono::Utc>,
) {
    if let Some(v) = version {
        // Latest wins by string comparison (no semver).
        if acc.version.as_deref().map_or(true, |existing| existing < v) {
            acc.version = Some(v.to_string());
        }
    }
    acc.earliest = Some(acc.earliest.map_or(installed_at, |e| e.min(installed_at)));
    acc.latest = Some(acc.latest.map_or(installed_at, |l| l.max(installed_at)));
}
```

Note: `update_version_and_dates` takes `&mut Acc` where `Acc` is the local struct above. Move it next to the inner struct or into `installed_plugins`'s body — pick whichever the compiler accepts.

- [ ] **Step 5: Run + commit**

```bash
cargo test -p kiro-market-core installed_plugins -- --nocapture 2>&1 | tail -10
cargo clippy -p kiro-market-core --tests -- -D warnings
cargo fmt --all
git add crates/kiro-market-core/src/project.rs
git commit -m "feat(core): add installed_plugins aggregator"
```

---

## Task 3: Core `remove_plugin()` cascade

**Files:**
- Modify: `crates/kiro-market-core/src/project.rs`

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn remove_plugin_cascades_through_skills_steering_agents_tracking() {
    let dir = tempfile::tempdir().expect("tempdir");
    let project = KiroProject::new(dir.path().to_path_buf());
    std::fs::create_dir_all(project.kiro_dir().join("steering")).expect("dirs");

    // Pre-seed: tracking files + on-disk steering file.
    let now = chrono::Utc::now();
    std::fs::write(
        project.kiro_dir().join("installed-skills.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "skills": {
                "alpha": {
                    "marketplace": "mp", "plugin": "p",
                    "version": "1.0.0", "installed_at": now,
                    "source_hash": "deadbeef"
                }
            }
        })).unwrap(),
    ).expect("skills");
    std::fs::write(
        project.kiro_dir().join("installed-steering.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "files": {
                "guide.md": {
                    "marketplace": "mp", "plugin": "p",
                    "version": "1.0.0", "installed_at": now,
                    "source_hash": "feedface", "installed_hash": "feedface"
                }
            }
        })).unwrap(),
    ).expect("steering");
    std::fs::write(
        project.kiro_dir().join("steering/guide.md"),
        "# guide\n",
    ).expect("steering file");

    let result = project.remove_plugin("mp", "p").expect("remove_plugin");
    assert_eq!(result.skills_removed, 1);
    assert_eq!(result.steering_removed, 1);
    assert_eq!(result.agents_removed, 0);

    // Tracking files are now empty for this plugin.
    let post = project.installed_plugins().expect("installed_plugins");
    assert!(
        post.iter().all(|p| p.plugin != "p"),
        "plugin p must be gone from the aggregated view"
    );
    // On-disk steering file is gone.
    assert!(
        !project.kiro_dir().join("steering/guide.md").exists(),
        "on-disk steering file must be unlinked"
    );
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test -p kiro-market-core remove_plugin_cascades -- --nocapture 2>&1 | tail -10
```

- [ ] **Step 3: Add the result type and method**

```rust
#[derive(Debug, Clone, Default, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct RemovePluginResult {
    pub skills_removed: u32,
    pub steering_removed: u32,
    pub agents_removed: u32,
}

impl KiroProject {
    /// Cascade-remove every tracked entry from `(marketplace,
    /// plugin)` across all three content-type tracking files. Unlinks
    /// the on-disk files, updates the tracking JSON files atomically
    /// (each `with_file_lock`'d), and returns aggregated counts.
    ///
    /// Best-effort: if a sub-removal errors, returns early with the
    /// error. No rollback. CLAUDE.md "tracking files are user-owned"
    /// still applies — we never delete files we don't have a
    /// tracking entry for.
    pub fn remove_plugin(
        &self,
        marketplace: &str,
        plugin: &str,
    ) -> crate::error::Result<RemovePluginResult> {
        let mut result = RemovePluginResult::default();

        let skills = self.load_installed()?;
        let to_remove: Vec<String> = skills
            .skills
            .iter()
            .filter(|(_, meta)| meta.marketplace == marketplace && meta.plugin == plugin)
            .map(|(name, _)| name.clone())
            .collect();
        for name in &to_remove {
            self.remove_skill(name)?;
            result.skills_removed = result.skills_removed.saturating_add(1);
        }

        // For steering and agents, mirror the per-file pattern with
        // tracking-only updates plus on-disk unlink. (Adjust to match
        // each existing remove API or add minimal helpers as needed —
        // see project.rs near `remove_skill` for the precedent.)
        let steering = self.load_installed_steering()?;
        let steering_to_remove: Vec<std::path::PathBuf> = steering
            .files
            .iter()
            .filter(|(_, meta)| meta.marketplace == marketplace && meta.plugin == plugin)
            .map(|(rel, _)| rel.clone())
            .collect();
        for rel in &steering_to_remove {
            self.remove_steering_file(rel)?;
            result.steering_removed = result.steering_removed.saturating_add(1);
        }

        let agents = self.load_installed_agents()?;
        let agents_to_remove: Vec<String> = agents
            .agents
            .iter()
            .filter(|(_, meta)| meta.marketplace == marketplace && meta.plugin == plugin)
            .map(|(name, _)| name.clone())
            .collect();
        for name in &agents_to_remove {
            self.remove_agent(name)?;
            result.agents_removed = result.agents_removed.saturating_add(1);
        }

        Ok(result)
    }
}
```

- [ ] **Step 4: Add the supporting per-file remove methods if missing**

Confirm whether `remove_steering_file(&Path)` and `remove_agent(&str)` exist on `KiroProject`. If not, add them following `remove_skill`'s shape. **Sub-task:** if either is missing, write a separate failing test first (`remove_steering_file_unlinks_and_updates_tracking`) and add it before the cascade test passes.

- [ ] **Step 5: Run + commit**

```bash
cargo test -p kiro-market-core remove_plugin -- --nocapture 2>&1 | tail -10
cargo clippy -p kiro-market-core --tests -- -D warnings
cargo fmt --all
git add crates/kiro-market-core/src/project.rs
git commit -m "feat(core): add remove_plugin cascade across the three tracking files"
```

---

## Task 4: Tauri `install_plugin_agents` command

**Files:**
- Create: `crates/kiro-control-center/src-tauri/src/commands/agents.rs`
- Modify: `crates/kiro-control-center/src-tauri/src/commands/mod.rs` (add `pub mod agents;`)
- Modify: `crates/kiro-control-center/src-tauri/src/lib.rs` (add to `invoke_handler!`)

- [ ] **Step 1: Mirror `commands/steering.rs` exactly**

Copy `commands/steering.rs` to `commands/agents.rs` and rename: `install_plugin_steering` → `install_plugin_agents`, `SteeringInstallContext` → `AgentInstallContext`, etc. The agent install accepts an extra `accept_mcp: bool` and a `format: Option<PluginFormat>` (the latter from `PluginInstallContext`).

Reference shape (the existing `commands::steering::install_plugin_steering_impl` is the template):

```rust
#[tauri::command]
#[specta::specta]
pub async fn install_plugin_agents(
    marketplace: String,
    plugin: String,
    force: bool,
    accept_mcp: bool,
    project_path: String,
) -> Result<InstallAgentsResult, CommandError> {
    let svc = make_service()?;
    install_plugin_agents_impl(
        &svc,
        &marketplace,
        &plugin,
        InstallMode::from(force),
        accept_mcp,
        &project_path,
    )
}

fn install_plugin_agents_impl(
    svc: &MarketplaceService,
    marketplace: &str,
    plugin: &str,
    mode: InstallMode,
    accept_mcp: bool,
    project_path: &str,
) -> Result<InstallAgentsResult, CommandError> {
    validate_kiro_project_path(project_path)?;
    let ctx = svc
        .resolve_plugin_install_context(marketplace, plugin)
        .map_err(CommandError::from)?;
    let project = KiroProject::new(PathBuf::from(project_path));
    Ok(svc.install_plugin_agents(
        &project,
        &ctx.plugin_dir,
        &ctx.agent_scan_paths,
        ctx.format,
        AgentInstallContext {
            mode,
            accept_mcp,
            marketplace,
            plugin,
            version: ctx.version.as_deref(),
        },
    ))
}
```

- [ ] **Step 2: Add 5 `_impl`-level tests mirroring `commands/steering.rs::tests`**

The five tests for steering are:
1. `..._installs_default_path_files`
2. `..._returns_not_found_for_unknown_plugin`
3. `..._threads_resolved_version_into_install_ctx`
4. `..._force_mode_overwrites_changed_source`
5. `..._new_mode_surfaces_<failure>_in_failed`

Mirror each for agents. The agent equivalents are mechanical — replace fixture content (steering `.md` → agent `.md` with frontmatter) and assertion fields (`installed.len()` → `installed.len()` of agents).

- [ ] **Step 3: Register the module + command**

In `commands/mod.rs`:

```rust
pub mod agents;
```

In `lib.rs::create_builder()`'s `invoke_handler!`:

```rust
commands::agents::install_plugin_agents,
```

- [ ] **Step 4: Regenerate bindings**

```bash
cargo test -p kiro-control-center --lib -- --ignored 2>&1 | tail -5
```

Expected: `bindings.ts` now includes `installPluginAgents`.

- [ ] **Step 5: Run + commit**

```bash
cargo test --workspace 2>&1 | grep "test result\|FAILED" | tail -5
cargo clippy --workspace --tests -- -D warnings
cargo fmt --all
git add crates/kiro-control-center/src-tauri/src/commands/agents.rs \
        crates/kiro-control-center/src-tauri/src/commands/mod.rs \
        crates/kiro-control-center/src-tauri/src/lib.rs \
        crates/kiro-control-center/src/lib/bindings.ts
git commit -m "feat(tauri): install_plugin_agents command + impl-level tests"
```

---

## Task 5: Tauri `commands/plugins.rs` — install_plugin, list_installed_plugins, remove_plugin

**Files:**
- Create: `crates/kiro-control-center/src-tauri/src/commands/plugins.rs`
- Modify: `commands/mod.rs`, `lib.rs`

- [ ] **Step 1: Write the failing test**

Following the `_impl`-only test pattern, create `commands/plugins.rs::tests::install_plugin_impl_orchestrates_all_three_paths`:

```rust
#[test]
fn install_plugin_impl_orchestrates_all_three_paths() {
    use kiro_market_core::service::test_support::{
        make_plugin_with_skills, relative_path_entry, seed_marketplace_with_registry,
        temp_service,
    };
    use std::fs;

    let (dir, svc) = temp_service();
    let entries = vec![relative_path_entry("p", "plugins/p")];
    let mp_path = seed_marketplace_with_registry(dir.path(), &svc, "mp", &entries);
    let plugin_dir = mp_path.join("plugins/p");
    fs::create_dir_all(&plugin_dir).expect("plugin dir");
    make_plugin_with_skills(&mp_path, "p", &["alpha"]);
    fs::write(plugin_dir.join("plugin.json"), br#"{"name":"p","version":"1.0.0"}"#).unwrap();
    fs::create_dir_all(plugin_dir.join("steering")).unwrap();
    fs::write(plugin_dir.join("steering/guide.md"), "# g\n").unwrap();

    let project_path = make_kiro_project(dir.path());
    let result = install_plugin_impl(&svc, "mp", "p", InstallMode::New, false, &project_path)
        .expect("happy path");

    assert_eq!(result.plugin, "p");
    assert!(result.skills.is_some());
    assert!(result.steering.is_some());
}
```

- [ ] **Step 2: Add `install_plugin` Tauri command**

Same wrapper + `_impl` pattern as steering. Calls `svc.install_plugin(...)` from Task 1.

- [ ] **Step 3: Add `list_installed_plugins` Tauri command**

```rust
#[tauri::command]
#[specta::specta]
pub async fn list_installed_plugins(
    project_path: String,
) -> Result<Vec<InstalledPluginInfo>, CommandError> {
    list_installed_plugins_impl(&project_path)
}

fn list_installed_plugins_impl(
    project_path: &str,
) -> Result<Vec<InstalledPluginInfo>, CommandError> {
    validate_kiro_project_path(project_path)?;
    let project = KiroProject::new(PathBuf::from(project_path));
    project.installed_plugins().map_err(CommandError::from)
}
```

Plus 2 tests: empty project returns empty vec; project with mixed tracking returns aggregated.

- [ ] **Step 4: Add `remove_plugin` Tauri command**

```rust
#[tauri::command]
#[specta::specta]
pub async fn remove_plugin(
    marketplace: String,
    plugin: String,
    project_path: String,
) -> Result<RemovePluginResult, CommandError> {
    remove_plugin_impl(&marketplace, &plugin, &project_path)
}

fn remove_plugin_impl(
    marketplace: &str,
    plugin: &str,
    project_path: &str,
) -> Result<RemovePluginResult, CommandError> {
    validate_kiro_project_path(project_path)?;
    let project = KiroProject::new(PathBuf::from(project_path));
    project
        .remove_plugin(marketplace, plugin)
        .map_err(CommandError::from)
}
```

Plus 2 tests: removing-nonexistent returns zeros (not an error); removing-installed returns expected counts.

- [ ] **Step 5: Register, regenerate bindings, commit**

```bash
cargo test --workspace 2>&1 | grep "test result\|FAILED" | tail -5
cargo test -p kiro-control-center --lib -- --ignored 2>&1 | tail -3
cargo clippy --workspace --tests -- -D warnings
cargo fmt --all
git add -A
git commit -m "feat(tauri): plugins.rs — install_plugin / list_installed_plugins / remove_plugin"
```

---

## Task 6: Extract format helpers + create `PluginCard.svelte`

**Files:**
- Create: `crates/kiro-control-center/src/lib/format.ts`
- Modify: `crates/kiro-control-center/src/lib/components/BrowseTab.svelte` (replace 3 inlined functions with imports from `$lib/format`)
- Create: `crates/kiro-control-center/src/lib/components/PluginCard.svelte`

**Why this task starts with an extraction:** PR 92 left `formatSkippedReason`, `skillCountLabel`, and `skillCountTitle` as module-private functions inside `BrowseTab.svelte`. Both BrowseTab AND the new PluginCard need them. Lifting to `$lib/format.ts` first means PluginCard imports rather than re-implements — addresses the "duplicate the code" red flag from PR 92's review.

- [ ] **Step 1: Create `$lib/format.ts`**

```typescript
// crates/kiro-control-center/src/lib/format.ts

import type { SkillCount, SkippedReason, SkippedSkill } from "$lib/bindings";

/**
 * Render a SkippedReason as a one-line label suitable for tooltips and
 * banner bodies. Total over all variants — TypeScript's discriminated-
 * union exhaustiveness check forces full coverage.
 *
 * Lifted from BrowseTab.svelte (PR 92). Two consumers now: BrowseTab's
 * skill-count tooltip and PluginCard's matching tooltip.
 */
export function formatSkippedReason(r: SkippedReason): string {
  switch (r.kind) {
    case "directory_missing":
      return `plugin directory not found: ${r.path}`;
    case "not_a_directory":
      return `plugin path is not a directory: ${r.path}`;
    case "symlink_refused":
      return `plugin path is a symlink (refused): ${r.path}`;
    case "directory_unreadable":
      return `could not read ${r.path}: ${r.reason}`;
    case "invalid_manifest":
      return `malformed plugin.json at ${r.path}: ${r.reason}`;
    case "manifest_read_failed":
      return `could not read plugin.json at ${r.path}: ${r.reason}`;
    case "remote_source_not_local":
      return `plugin source is remote: ${r.plugin}`;
    case "no_skills":
      return `plugin declares no skills: ${r.path}`;
  }
}

export function skillCountLabel(sc: SkillCount): string {
  switch (sc.state) {
    case "known":
      return String(sc.count);
    case "remote_not_counted":
      return "–";
    case "manifest_failed":
      return "!";
  }
}

export function skillCountTitle(sc: SkillCount): string | undefined {
  switch (sc.state) {
    case "known":
      return undefined;
    case "remote_not_counted":
      return "Remote plugin — skills cannot be counted without cloning";
    case "manifest_failed":
      return formatSkippedReason(sc.reason);
  }
}

export function formatSkippedSkill(s: SkippedSkill): string {
  const label = s.name_hint ?? "<unnamed>";
  let reason: string;
  switch (s.reason.kind) {
    case "read_failed":
      reason = `could not read SKILL.md: ${s.reason.reason}`;
      break;
    case "frontmatter_invalid":
      reason = `malformed frontmatter: ${s.reason.reason}`;
      break;
    default:
      reason = "unreadable";
  }
  return `${label}: ${reason}`;
}

export function formatSkippedSkillsForPlugin(list: readonly SkippedSkill[]): string {
  const MAX = 5;
  const parts = list.slice(0, MAX).map(formatSkippedSkill);
  const overflow = list.length - parts.length;
  const joined = parts.join("; ");
  return overflow > 0
    ? `${list.length} skill(s) failed to load — ${joined}; +${overflow} more`
    : `${list.length} skill(s) failed to load — ${joined}`;
}
```

- [ ] **Step 2: Replace inlined functions in BrowseTab.svelte with imports**

In `BrowseTab.svelte` (currently lines 5-101), replace the inlined function bodies with:

```svelte
<script lang="ts">
  import { onMount } from "svelte";
  import { SvelteMap, SvelteSet } from "svelte/reactivity";
  import { commands } from "$lib/bindings";
  import {
    formatSkippedReason,
    skillCountLabel,
    skillCountTitle,
    formatSkippedSkill,
    formatSkippedSkillsForPlugin,
  } from "$lib/format";
  import type {
    MarketplaceInfo,
    PluginInfo,
    SkillInfo,
    SkillCount,
    SkippedReason,
    SkippedSkill,
    SteeringWarning,
  } from "$lib/bindings";
  import SkillCard from "./SkillCard.svelte";

  // formatSteeringWarning stays inline — it's specific to BrowseTab's
  // installSteering rendering. If a second consumer appears, lift it
  // to $lib/format.ts in a follow-up.
  function formatSteeringWarning(w: SteeringWarning): string {
    // ... existing body unchanged
  }
```

Delete the now-duplicated `formatSkippedReason`, `skillCountLabel`, `skillCountTitle`, `formatSkippedSkill`, and `formatSkippedSkillsForPlugin` definitions from BrowseTab.svelte.

- [ ] **Step 3: Run svelte-check after the extraction (catches accidental name drift)**

```bash
cd crates/kiro-control-center && npm run check 2>&1 | tail -5
```

Expected: `0 ERRORS 0 WARNINGS 0 FILES_WITH_PROBLEMS`. Then commit:

```bash
git add crates/kiro-control-center/src/lib/format.ts \
        crates/kiro-control-center/src/lib/components/BrowseTab.svelte
git commit -m "refactor(ui): lift format helpers to \$lib/format.ts"
```

- [ ] **Step 4: Create `PluginCard.svelte`**

```svelte
<!-- crates/kiro-control-center/src/lib/components/PluginCard.svelte -->
<script lang="ts">
  import type { PluginInfo } from "$lib/bindings";
  import { skillCountLabel, skillCountTitle } from "$lib/format";

  type Props = {
    /** The plugin to render. From `commands.listPlugins(...)` results. */
    plugin: PluginInfo;
    /** Owning marketplace name (not in `PluginInfo` itself). */
    marketplace: string;
    /** Whether the plugin appears in `commands.listInstalledPlugins(...)`. */
    installed: boolean;
    /** True while a per-plugin install is in flight for this card. */
    installing: boolean;
    /** True when project_path is empty — disables the install button. */
    projectPicked: boolean;
    onInstall: () => void;
  };

  let {
    plugin,
    marketplace,
    installed,
    installing,
    projectPicked,
    onInstall,
  }: Props = $props();

  const title = $derived(
    !projectPicked
      ? "Pick a project first"
      : installed
        ? `${plugin.name} is already installed in this project`
        : `Install ${plugin.name} (skills + steering + agents) into the active project`,
  );
</script>

<div class="flex items-start gap-3 px-3 py-3 rounded-md border border-kiro-muted bg-kiro-overlay">
  <div class="flex-1 min-w-0">
    <div class="flex items-center gap-2 flex-wrap">
      <span class="text-sm font-medium text-kiro-text truncate">{plugin.name}</span>
      <span
        class="text-[11px] {plugin.skill_count.state === 'manifest_failed'
          ? 'text-kiro-warning'
          : 'text-kiro-subtle'} flex-shrink-0"
        title={skillCountTitle(plugin.skill_count)}
        aria-label={skillCountTitle(plugin.skill_count)}
      >
        {skillCountLabel(plugin.skill_count)} skill{plugin.skill_count.state === "known" &&
        plugin.skill_count.count === 1
          ? ""
          : "s"}
      </span>
    </div>
    {#if plugin.description}
      <div class="mt-1 text-xs text-kiro-subtle">{plugin.description}</div>
    {/if}
    <div class="mt-1.5 text-[10px] uppercase tracking-wider text-kiro-subtle">
      {marketplace}
    </div>
  </div>

  <div class="flex flex-col items-end gap-1.5 flex-shrink-0">
    {#if installed}
      <span
        class="px-2 py-0.5 text-[11px] font-medium text-kiro-success border border-kiro-success/40 rounded"
      >
        Installed
      </span>
    {:else}
      <button
        type="button"
        onclick={onInstall}
        disabled={!projectPicked || installing}
        aria-busy={installing}
        {title}
        aria-label="Install {plugin.name}"
        class="px-3 py-1.5 text-xs font-medium rounded-md transition-colors
          {projectPicked && !installing
            ? 'bg-kiro-overlay border border-kiro-muted text-kiro-accent-300 hover:bg-kiro-muted hover:text-kiro-accent-200'
            : 'bg-kiro-muted text-kiro-subtle border border-transparent cursor-not-allowed'}"
      >
        {installing ? "Installing…" : "Install"}
      </button>
    {/if}
  </div>
</div>
```

- [ ] **Step 5: Run svelte-check + commit**

```bash
cd crates/kiro-control-center && npm run check 2>&1 | tail -5
git add crates/kiro-control-center/src/lib/components/PluginCard.svelte
git commit -m "feat(ui): PluginCard reusable component"
```

---

## Task 7: BrowseTab — plugins primary view

**Files:**
- Modify: `crates/kiro-control-center/src/lib/components/BrowseTab.svelte`

- [ ] **Step 1: Import the new component + add state**

In the script block (top of the file), add:

```svelte
<script lang="ts">
  // ... existing imports
  import PluginCard from "./PluginCard.svelte";
  import type { InstalledPluginInfo } from "$lib/bindings";

  // ... existing state declarations (selectedMarketplaces, selectedPlugins,
  // selectedSkills, installedOnly, filterText, forceInstall, popoverOpen,
  // popRef, loadingMarketplaces, pendingFetches, installing, fetchErrors,
  // installError, installMessage, pendingSteeringInstalls)

  type BrowseView = "plugins" | "skills";
  let browseView: BrowseView = $state("plugins");

  // Per-plugin in-flight tracker for plugin installs (parallels
  // pendingSteeringInstalls). Same key shape: pluginKey(marketplace,
  // plugin) so two plugins can install in parallel without colliding.
  let pendingPluginInstalls = new SvelteSet<string>();

  // Set of pluginKey()s currently installed in this project. Driven by
  // the listInstalledPlugins fetch below. Used by PluginCard to render
  // "Installed" vs. "Install" state.
  let installedPlugins: InstalledPluginInfo[] = $state([]);
  let installedPluginKeys = $derived(
    new Set(installedPlugins.map((p) => pluginKey(p.marketplace, p.plugin))),
  );
</script>
```

- [ ] **Step 2: Add `fetchInstalledPlugins` + wire it to project changes**

Below the existing `fetchSkillsFor` function, add:

```typescript
async function fetchInstalledPlugins() {
  if (!projectPath) {
    installedPlugins = [];
    return;
  }
  try {
    const result = await commands.listInstalledPlugins(projectPath);
    installedPlugins = result.status === "ok" ? result.data : [];
  } catch (e) {
    console.error("[BrowseTab] listInstalledPlugins rejected", e);
    installedPlugins = [];
  }
}

onMount(fetchInstalledPlugins);

// Refresh whenever projectPath changes — same pattern the existing
// `priorProjectPath` watcher uses for skill state.
$effect(() => {
  // Read projectPath to register the dependency.
  const _path = projectPath;
  fetchInstalledPlugins();
});
```

- [ ] **Step 3: Add `installWholePlugin` async function**

Below the existing `installSteering` function:

```typescript
async function installWholePlugin(marketplace: string, plugin: string) {
  const key = pluginKey(marketplace, plugin);
  if (pendingPluginInstalls.has(key)) return;
  pendingPluginInstalls.add(key);
  installError = null;
  installMessage = null;

  try {
    const result = await commands.installPlugin(
      marketplace,
      plugin,
      forceInstall,
      projectPath,
    );
    if (result.status === "ok") {
      const r = result.data;
      const parts: string[] = [];
      if (r.skills) {
        const installed = r.skills.installed.length;
        const failed = r.skills.failed.length;
        if (installed > 0) {
          parts.push(`${installed} skill${installed === 1 ? "" : "s"}`);
        }
        if (failed > 0) {
          parts.push(`${failed} skill${failed === 1 ? "" : "s"} failed`);
        }
      }
      if (r.steering) {
        const installed = r.steering.installed.length;
        const failed = r.steering.failed.length;
        if (installed > 0) {
          parts.push(`${installed} steering file${installed === 1 ? "" : "s"}`);
        }
        if (failed > 0) {
          parts.push(`${failed} steering failed`);
        }
      }
      if (r.agents) {
        const installed = r.agents.installed.length;
        const failed = r.agents.failed.length;
        if (installed > 0) {
          parts.push(`${installed} agent${installed === 1 ? "" : "s"}`);
        }
        if (failed > 0) {
          parts.push(`${failed} agent${failed === 1 ? "" : "s"} failed`);
        }
      }
      const anyFailed =
        (r.skills?.failed.length ?? 0) +
          (r.steering?.failed.length ?? 0) +
          (r.agents?.failed.length ?? 0) >
        0;
      const anyInstalled =
        (r.skills?.installed.length ?? 0) +
          (r.steering?.installed.length ?? 0) +
          (r.agents?.installed.length ?? 0) >
        0;
      const summary = parts.length > 0 ? parts.join(" · ") : "nothing to install";
      if (anyFailed && !anyInstalled) {
        installError = `Plugin install failed for ${plugin}: ${summary}`;
      } else {
        installMessage = `Plugin ${plugin}: ${summary}`;
      }
      // Refresh the installed-plugin set so the card flips to "Installed".
      await fetchInstalledPlugins();
    } else {
      installError = `Plugin install failed for ${plugin}: ${result.error.message}`;
    }
  } catch (e) {
    const reason = e instanceof Error ? e.message : String(e);
    installError = `Plugin install failed for ${plugin}: ${reason}`;
  } finally {
    pendingPluginInstalls.delete(key);
  }
}
```

- [ ] **Step 4: Add the view-toggle UI**

Above the main scrollable grid container (around line 985, just before `<div class="flex-1 overflow-y-auto p-4">`), insert:

```svelte
<div class="px-4 py-2 border-b border-kiro-muted bg-kiro-surface/40 flex items-center gap-2">
  <span class="text-xs text-kiro-subtle">Browse:</span>
  <div class="inline-flex gap-1 px-1 py-1 rounded-md bg-kiro-overlay border border-kiro-muted">
    <button
      type="button"
      onclick={() => (browseView = "plugins")}
      aria-pressed={browseView === "plugins"}
      class="px-2.5 py-0.5 text-xs font-medium rounded {browseView === 'plugins'
        ? 'bg-kiro-accent-900/40 text-kiro-accent-300'
        : 'text-kiro-text-secondary hover:text-kiro-text'}"
    >
      Plugins
    </button>
    <button
      type="button"
      onclick={() => (browseView = "skills")}
      aria-pressed={browseView === "skills"}
      class="px-2.5 py-0.5 text-xs font-medium rounded {browseView === 'skills'
        ? 'bg-kiro-accent-900/40 text-kiro-accent-300'
        : 'text-kiro-text-secondary hover:text-kiro-text'}"
    >
      Skills
    </button>
  </div>
</div>
```

- [ ] **Step 5: Conditionally render skill grid vs. plugin grid**

Find the existing skill-grid block (around line 1019-1028, the `{#each filteredSkills as skill ...}` block). Wrap it and add the plugin-grid else branch:

```svelte
{:else if browseView === "skills"}
  <div class="grid gap-3 grid-cols-1 lg:grid-cols-2">
    {#each filteredSkills as skill (skillKey(skill.marketplace, skill.plugin, skill.name))}
      {@const key = skillKey(skill.marketplace, skill.plugin, skill.name)}
      <SkillCard
        {skill}
        selected={selectedSkills.has(key)}
        onToggle={() => toggleSkill(key)}
      />
    {/each}
  </div>
{:else}
  <!-- Plugins view: every plugin in scope, install action per card. -->
  {#if availablePlugins.length === 0}
    <div class="flex flex-col items-center justify-center h-full text-kiro-subtle gap-3">
      <p class="text-sm">No plugins available — pick a marketplace from Filters.</p>
    </div>
  {:else}
    <div class="grid gap-3 grid-cols-1 lg:grid-cols-2">
      {#each availablePlugins as ap (pluginKey(ap.marketplace, ap.plugin.name))}
        {@const key = pluginKey(ap.marketplace, ap.plugin.name)}
        <PluginCard
          plugin={ap.plugin}
          marketplace={ap.marketplace}
          installed={installedPluginKeys.has(key)}
          installing={pendingPluginInstalls.has(key)}
          projectPicked={!!projectPath}
          onInstall={() => installWholePlugin(ap.marketplace, ap.plugin.name)}
        />
      {/each}
    </div>
  {/if}
{/if}
```

Note: the existing `{:else if filteredSkills.length === 0}` empty-state block (PR 92's per-plugin steering install in the empty state) needs to be removed in favor of the new Plugins view — Plugins is the new home for that flow. Confirm by reading the surrounding code; the `{:else if filteredSkills.length === 0}` branch should now ONLY fire when `browseView === "skills"`.

- [ ] **Step 6: Run svelte-check + commit**

```bash
cd crates/kiro-control-center && npm run check 2>&1 | tail -5
git add crates/kiro-control-center/src/lib/components/BrowseTab.svelte
git commit -m "feat(ui): plugin cards as primary BrowseTab view"
```

---

## Task 8: InstalledTab — plugins-grouped view

**Files:**
- Modify: `crates/kiro-control-center/src/lib/components/InstalledTab.svelte`

The existing InstalledTab is 215 lines, all skill-table rendering. After this task it has two sections: a primary "Installed plugins" grouped view, and a collapsible `<details>` "All installed skills" preserving the flat-table backward compat.

- [ ] **Step 1: Add state + fetchers**

In the `<script lang="ts">` block:

```svelte
<script lang="ts">
  import { onMount } from "svelte";
  import { commands } from "$lib/bindings";
  import type {
    InstalledSkillInfo,
    InstalledPluginInfo,
  } from "$lib/bindings";

  let { projectPath }: { projectPath: string } = $props();

  let plugins: InstalledPluginInfo[] = $state([]);
  let skills: InstalledSkillInfo[] = $state([]);
  let loading: boolean = $state(true);
  let loadError: string | null = $state(null);
  let removingKey: string | null = $state(null);

  function pluginKey(mp: string, plugin: string): string {
    return `${mp}${plugin}`;
  }

  async function refresh() {
    loading = true;
    loadError = null;
    try {
      const [pluginsResult, skillsResult] = await Promise.all([
        commands.listInstalledPlugins(projectPath),
        commands.listInstalledSkills(projectPath),
      ]);
      if (pluginsResult.status === "ok") {
        plugins = pluginsResult.data;
      } else {
        loadError = pluginsResult.error.message;
      }
      if (skillsResult.status === "ok") {
        skills = skillsResult.data;
      } else if (loadError === null) {
        loadError = skillsResult.error.message;
      }
    } catch (e) {
      const reason = e instanceof Error ? e.message : String(e);
      loadError = `Failed to load installed state: ${reason}`;
    } finally {
      loading = false;
    }
  }

  async function removePlugin(marketplace: string, plugin: string) {
    const key = pluginKey(marketplace, plugin);
    if (removingKey !== null) return;
    removingKey = key;
    try {
      const result = await commands.removePlugin(marketplace, plugin, projectPath);
      if (result.status === "ok") {
        await refresh();
      } else {
        loadError = `Remove failed for ${plugin}: ${result.error.message}`;
      }
    } catch (e) {
      const reason = e instanceof Error ? e.message : String(e);
      loadError = `Remove failed for ${plugin}: ${reason}`;
    } finally {
      removingKey = null;
    }
  }

  function formatDate(iso: string): string {
    const d = new Date(iso);
    return Number.isNaN(d.getTime()) ? iso : d.toLocaleString();
  }

  function contentSummary(p: InstalledPluginInfo): string {
    const parts: string[] = [];
    if (p.skill_count > 0) parts.push(`${p.skill_count} skill${p.skill_count === 1 ? "" : "s"}`);
    if (p.steering_count > 0)
      parts.push(`${p.steering_count} steering`);
    if (p.agent_count > 0)
      parts.push(`${p.agent_count} agent${p.agent_count === 1 ? "" : "s"}`);
    return parts.length > 0 ? parts.join(" · ") : "(empty)";
  }

  onMount(refresh);

  $effect(() => {
    const _path = projectPath;
    refresh();
  });
</script>
```

- [ ] **Step 2: Render the markup**

Replace the existing `<div>` body (lines ~135 onward) with:

```svelte
<div class="flex-1 overflow-y-auto p-4">
  {#if loading}
    <div class="flex flex-col items-center justify-center h-full text-kiro-subtle gap-3">
      <svg class="w-8 h-8 text-kiro-accent-800 animate-pulse" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="1.5"
          d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15" />
      </svg>
      <p class="text-sm">Loading installed state...</p>
    </div>
  {:else if loadError}
    <div class="px-4 py-3 rounded-md bg-kiro-error/10 border border-kiro-error/30">
      <p class="text-sm text-kiro-error">{loadError}</p>
    </div>
  {:else}
    <section class="mb-6">
      <h2 class="text-sm font-semibold text-kiro-text mb-3">Installed plugins</h2>
      {#if plugins.length === 0}
        <p class="text-sm text-kiro-subtle">No plugins installed in this project.</p>
      {:else}
        <table class="w-full text-sm">
          <thead>
            <tr class="text-left text-[11px] uppercase tracking-wider text-kiro-subtle border-b border-kiro-muted">
              <th class="px-4 py-2">Plugin</th>
              <th class="px-4 py-2">Marketplace</th>
              <th class="px-4 py-2">Version</th>
              <th class="px-4 py-2">Contents</th>
              <th class="px-4 py-2">Installed</th>
              <th class="px-4 py-2"></th>
            </tr>
          </thead>
          <tbody>
            {#each plugins as p (pluginKey(p.marketplace, p.plugin))}
              {@const key = pluginKey(p.marketplace, p.plugin)}
              <tr class="border-b border-kiro-muted/50">
                <td class="px-4 py-3 font-medium text-kiro-text">{p.plugin}</td>
                <td class="px-4 py-3 text-kiro-text-secondary">{p.marketplace}</td>
                <td class="px-4 py-3 text-kiro-text-secondary">{p.installed_version ?? "—"}</td>
                <td class="px-4 py-3 text-kiro-text-secondary">{contentSummary(p)}</td>
                <td class="px-4 py-3 text-kiro-text-secondary">{formatDate(p.latest_install)}</td>
                <td class="px-4 py-3 text-right">
                  <button
                    type="button"
                    onclick={() => removePlugin(p.marketplace, p.plugin)}
                    disabled={removingKey !== null}
                    aria-busy={removingKey === key}
                    class="px-2 py-0.5 text-[11px] text-kiro-subtle hover:text-kiro-error disabled:cursor-not-allowed"
                  >
                    {removingKey === key ? "Removing…" : "Remove"}
                  </button>
                </td>
              </tr>
            {/each}
          </tbody>
        </table>
      {/if}
    </section>

    <details class="mb-6">
      <summary class="cursor-pointer text-sm font-medium text-kiro-text-secondary hover:text-kiro-text">
        All installed skills (flat view)
      </summary>
      <div class="mt-3">
        {#if skills.length === 0}
          <p class="text-sm text-kiro-subtle">No skills installed.</p>
        {:else}
          <table class="w-full text-sm">
            <thead>
              <tr class="text-left text-[11px] uppercase tracking-wider text-kiro-subtle border-b border-kiro-muted">
                <th class="px-4 py-2">Name</th>
                <th class="px-4 py-2">Marketplace</th>
                <th class="px-4 py-2">Plugin</th>
                <th class="px-4 py-2">Version</th>
                <th class="px-4 py-2">Installed</th>
              </tr>
            </thead>
            <tbody>
              {#each skills as skill (skill.name)}
                <tr class="border-b border-kiro-muted/50">
                  <td class="px-4 py-3 text-kiro-text">{skill.name}</td>
                  <td class="px-4 py-3 text-kiro-text-secondary">{skill.marketplace}</td>
                  <td class="px-4 py-3 text-kiro-text-secondary">{skill.plugin}</td>
                  <td class="px-4 py-3 text-kiro-text-secondary">{skill.version ?? "—"}</td>
                  <td class="px-4 py-3 text-kiro-text-secondary">{formatDate(skill.installed_at)}</td>
                </tr>
              {/each}
            </tbody>
          </table>
        {/if}
      </div>
    </details>
  {/if}
</div>
```

The exact nav-rail wrapper / outer container should stay as the existing InstalledTab provides — only the inner body changes.

- [ ] **Step 3: Run svelte-check + commit**

```bash
cd crates/kiro-control-center && npm run check 2>&1 | tail -5
git add crates/kiro-control-center/src/lib/components/InstalledTab.svelte
git commit -m "feat(ui): plugins-grouped InstalledTab + collapsible flat skills"
```

---

## Task 9: Playwright e2e — plugin install happy path

**Files:**
- Modify: `crates/kiro-control-center/tests/e2e/app.spec.ts`

The existing `"install skill from browse tab"` test (around line 91) is the structural template. The new test exercises the plugin path: switch to Plugins view, click Install on a card, assert appearance in Installed.

- [ ] **Step 1: Add the test inside the `Marketplace workflow` describe block**

After the existing `"install skill from browse tab"` test, add:

```typescript
test("install plugin from browse tab and verify in installed tab", async ({ page }) => {
  await page.getByRole("button", { name: "Browse", exact: true }).click();

  // The earlier "add local marketplace" test seeds FIXTURE_MARKETPLACE_PATH.
  // Skip if the fixture isn't available (matches the skill-install pattern
  // at line ~95 of this file).
  const fixturePath = process.env.FIXTURE_MARKETPLACE_PATH;
  test.skip(!fixturePath, "FIXTURE_MARKETPLACE_PATH not set");

  // Switch to the Plugins view if the toggle isn't already on it. The
  // Plugins button is the default per Task 7's BrowseView state, but
  // switching defensively keeps the test robust to a future default
  // change.
  const pluginsToggle = page.getByRole("button", { name: "Plugins", exact: true });
  if (await pluginsToggle.isVisible({ timeout: 2_000 }).catch(() => false)) {
    await pluginsToggle.click();
  }

  // Find a plugin card with the test fixture's plugin name. The card
  // exposes an "Install plugin" button via aria-label="Install <name>".
  const testPlugin = page.getByText(/test-plugin/i).first();
  if (!(await testPlugin.isVisible({ timeout: 5_000 }).catch(() => false))) {
    test.skip(true, "No marketplace with test-plugin available");
  }

  const installButton = page
    .getByRole("button", { name: /install test-plugin/i })
    .first();
  await installButton.click();

  // Wait for the installMessage banner to land (matches the success
  // banner pattern from the skill-install test).
  await expect(page.getByText(/Plugin test-plugin/i)).toBeVisible({
    timeout: 30_000,
  });

  // Navigate to Installed tab and assert the plugin row appears.
  await page.getByRole("button", { name: "Installed", exact: true }).click();
  await expect(page.getByRole("heading", { name: /installed plugins/i })).toBeVisible();
  await expect(page.getByText(/test-plugin/i).first()).toBeVisible();
});
```

- [ ] **Step 2: Run + commit**

```bash
cd crates/kiro-control-center && npm run test:e2e 2>&1 | tail -10
git add crates/kiro-control-center/tests/e2e/app.spec.ts
git commit -m "test(e2e): plugin install happy-path"
```

If `FIXTURE_MARKETPLACE_PATH` is unset locally, the test will skip (the pattern matches the skill-install precedent). CI configuration determines whether the fixture is set in the test environment — that's a separate concern from this PR.

---

## Task 10: Final sweep + open PR

- [ ] **Step 1: Run all gates**

```bash
cargo fmt --all --check
cargo clippy --workspace --tests -- -D warnings
cargo test --workspace 2>&1 | grep "test result\|FAILED" | tail -10
cargo xtask plan-lint --no-reindex 2>&1 | tail -10
cd crates/kiro-control-center && npm run check 2>&1 | tail -3
```

All green.

- [ ] **Step 2: Open PR**

Title: `feat: plugin-first install (BrowseTab + InstalledTab + agents.rs)`

Body: reference design doc, list new commands, note out-of-scope items.

- [ ] **Step 3: Manual smoke**

Run `npm run tauri dev` against `dwalleck/kiro-starter-kit`. Click Install on the plugin card. Verify:
- Green banner with content-count breakdown
- Switch to InstalledTab, see the plugin listed
- Click Remove, watch it disappear, verify on-disk files cleaned up

---

## Self-review

### Spec coverage
Each design-doc bullet maps to a task: `install_plugin` → Task 1, `installed_plugins` → Task 2, `remove_plugin` → Task 3, `install_plugin_agents` Tauri → Task 4, the rest of the Tauri commands → Task 5, frontend → Tasks 6-8, e2e → Task 9. **Phase 2 is design-only** in the design doc — no Phase 2 tasks in this plan. Intentional.

### Placeholder scan
Spotted and resolved:
- Task 3 step 4 conditionally adds `remove_steering_file` and `remove_agent` if missing — flagged as a sub-task with TDD shape rather than left as "TODO." Reviewer-implementer should write the failing test first if either method is absent.
- Task 4 step 2 lists the 5 mirror tests as a checklist rather than reproducing 200+ lines. Acceptable because the steering tests are right there as the reference; the implementer reads them and renames.

### Type consistency
- `InstallPluginResult` shape consistent across Task 1 (definition) and Task 5 (Tauri command return type).
- `InstalledPluginInfo` shape consistent across Task 2 (definition) and Task 5 (`list_installed_plugins`) and Task 8 (frontend consumer).
- `RemovePluginResult` shape consistent.
- `pluginKey(marketplace, plugin)` reused from existing `BrowseTab.svelte` helper.
- `pendingPluginInstalls: SvelteSet<string>` follows the `pendingSteeringInstalls` precedent from PR 92.

No type-name drift detected.

### 5-Gates summary

See design doc's "5-Gates self-review" section. **Action items extracted into the plan above:**
- Every new Tauri `_impl` calls `validate_kiro_project_path` (Task 4 step 1, Task 5 steps 3 + 4).
- `install_plugin` plumbs `accept_mcp` (Task 1 step 4, Task 5 step 2).
- JSON wire-format lock for `InstallPluginResult` (Task 1 step 6).
- New struct types follow existing `Serialize + cfg_attr(specta::Type)` pattern (Tasks 1, 2, 3 — explicit in the code blocks).

---

**Plan complete.** Suggested execution: subagent-driven, one task per subagent, two-stage review between tasks. Tasks are independent enough that a fresh agent can pick up each one with the design doc as context.

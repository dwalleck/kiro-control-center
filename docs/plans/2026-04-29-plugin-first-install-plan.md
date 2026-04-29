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

## Task 6: `PluginCard.svelte` reusable component

**Files:**
- Create: `crates/kiro-control-center/src/lib/components/PluginCard.svelte`

- [ ] **Step 1: Define the component contract**

Props:
- `plugin: PluginInfo` (existing type from bindings)
- `marketplace: string`
- `installed: boolean` (whether this plugin appears in `installed_plugins`)
- `installing: boolean` (in-flight indicator)
- `onInstall: () => void`
- `onUpdate?: () => void` (Phase 2 hook; render disabled stub for v1)
- `onRemove?: () => void`

- [ ] **Step 2: Compose the visual**

Card with:
- Top row: plugin name (bold) + content-count chips (`3 skills · 1 steering · 0 agents`)
- Optional: description (truncated)
- Bottom row: action button (Install | Installed | Updating | Removing)

Use the existing tailwind theme tokens (`bg-kiro-overlay`, `border-kiro-muted`, `text-kiro-accent-300`, etc.) — match `BrowseTab.svelte`'s empty-state plugin cards as a starting point.

- [ ] **Step 3: Add a small story-like test in `app.spec.ts`** (optional — Playwright-only)

Skip this step for v1; covered by Task 9's e2e test.

- [ ] **Step 4: Commit**

```bash
git add crates/kiro-control-center/src/lib/components/PluginCard.svelte
git commit -m "feat(ui): PluginCard reusable component"
```

---

## Task 7: BrowseTab — plugins primary view

**Files:**
- Modify: `crates/kiro-control-center/src/lib/components/BrowseTab.svelte`

- [ ] **Step 1: Add view-toggle state**

Near the existing state declarations:

```typescript
type BrowseView = "plugins" | "skills";
let browseView: BrowseView = $state("plugins");
```

- [ ] **Step 2: Add view-toggle UI**

Above the current grid (around line 985), insert a 2-button toggle: `[Plugins] [Skills]`. Tailwind: `inline-flex gap-1 px-1 py-1 rounded-md bg-kiro-overlay border border-kiro-muted`. Each button styles based on `browseView`.

- [ ] **Step 3: Conditionally render**

Wrap the existing skill-grid block (lines 1019-1028) in `{#if browseView === "skills"}` ... `{/if}`. Add an else branch that renders `PluginCard` for each `availablePlugins[i]`:

```svelte
{:else}
  <div class="grid gap-3 grid-cols-1 lg:grid-cols-2">
    {#each availablePlugins as ap (pluginKey(ap.marketplace, ap.plugin.name))}
      {@const key = pluginKey(ap.marketplace, ap.plugin.name)}
      <PluginCard
        plugin={ap.plugin}
        marketplace={ap.marketplace}
        installed={installedPluginKeys.has(key)}
        installing={pendingPluginInstalls.has(key)}
        onInstall={() => installWholePlugin(ap.marketplace, ap.plugin.name)}
      />
    {/each}
  </div>
{/if}
```

- [ ] **Step 4: Add `installWholePlugin` async function**

Mirrors PR 92's `installSteering` but calls `commands.installPlugin(...)`. Renders aggregate result (counts across the three sub-results) into `installMessage`/`installError`.

- [ ] **Step 5: Add `installedPluginKeys` derived from `commands.listInstalledPlugins`**

Fetch on mount + when `projectPath` changes; mirror the existing `installed` lookup pattern for skills.

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

- [ ] **Step 1: Replace the skill-list fetch with `listInstalledPlugins`**

```typescript
const result = await commands.listInstalledPlugins(projectPath);
```

- [ ] **Step 2: Render plugin rows**

Each row shows: plugin name, version, content counts (`3 skills · 1 steering · 0 agents`), `installed_at`, `[Remove]` button.

- [ ] **Step 3: Wire `removePlugin`**

Call `commands.removePlugin(marketplace, plugin, projectPath)`. On success, refresh the list.

- [ ] **Step 4: Optional collapsible "All installed skills" sub-section**

Below the plugins list, add a `<details>` for the existing flat skill table. Preserves backward-compat for users who liked it.

- [ ] **Step 5: Run svelte-check + commit**

```bash
cd crates/kiro-control-center && npm run check 2>&1 | tail -5
git add crates/kiro-control-center/src/lib/components/InstalledTab.svelte
git commit -m "feat(ui): plugins-grouped InstalledTab"
```

---

## Task 9: Playwright e2e — plugin install happy path

**Files:**
- Modify: `crates/kiro-control-center/tests/e2e/app.spec.ts`

- [ ] **Step 1: Add the test**

```typescript
test("install plugin from BrowseTab and verify in InstalledTab", async ({ page }) => {
  // Skip if FIXTURE_MARKETPLACE_PATH not set (matches existing skill test).
  // Pick the test project, click Browse, switch to Plugins view if not default,
  // click Install on the test plugin's card, wait for success banner,
  // navigate to Installed, assert the plugin row appears.
});
```

The existing `"install skill from browse tab"` is the template; mirror its env-var skips and locator patterns.

- [ ] **Step 2: Run + commit**

```bash
cd crates/kiro-control-center && npm run test:e2e 2>&1 | tail -10
git add crates/kiro-control-center/tests/e2e/app.spec.ts
git commit -m "test(e2e): plugin install happy-path"
```

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

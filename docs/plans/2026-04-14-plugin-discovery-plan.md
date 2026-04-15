# Plugin Discovery Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Support importing skills from repos that have no `marketplace.json` or contain unlisted plugins, via depth-limited `plugin.json` scanning merged with marketplace entries.

**Architecture:** A new `discover_plugins()` function in `plugin.rs` walks a repo up to depth 3, finding `plugin.json` files. `MarketplaceService::add()` calls it after (optionally) reading `marketplace.json`, deduplicates, and merges both sets into the same `Vec<PluginBasicInfo>`. At install time, `find_plugin_entry` falls back to the same scan when a plugin isn't in the manifest. `MarketplaceSource` gains a `fallback_name()` method for repos without a manifest.

**Tech Stack:** Rust (edition 2024), `serde`/`serde_json`, `thiserror`, `tracing`, `rstest`, `tempfile`

---

### Task 1: Add `MarketplaceError::NoPluginsFound` variant

**Files:**
- Modify: `crates/kiro-market-core/src/error.rs:18-34` (add variant to `MarketplaceError`)

**Step 1: Write the failing test**

Add a display test case to the existing `marketplace_error_display` rstest in `error.rs`:

```rust
#[case::no_plugins_found(
    MarketplaceError::NoPluginsFound { path: PathBuf::from("/tmp/repo") },
    "no plugins found in /tmp/repo"
)]
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p kiro-market-core -- marketplace_error_display`
Expected: compile error — `NoPluginsFound` variant does not exist

**Step 3: Add the variant**

Add to the `MarketplaceError` enum after `ManifestNotFound`:

```rust
/// No `marketplace.json` and no `plugin.json` files found via scan.
#[error("no plugins found in {path}")]
NoPluginsFound { path: PathBuf },
```

**Step 4: Run test to verify it passes**

Run: `cargo test -p kiro-market-core -- marketplace_error_display`
Expected: PASS

**Step 5: Commit**

```bash
git add crates/kiro-market-core/src/error.rs
git commit -m "feat: add NoPluginsFound error variant for repos without marketplace or plugins"
```

---

### Task 2: Add `MarketplaceSource::fallback_name()`

**Files:**
- Modify: `crates/kiro-market-core/src/cache.rs:38-79` (add method to `impl MarketplaceSource`)

**Step 1: Write the failing tests**

Add to the existing `mod tests` in `cache.rs`:

```rust
#[rstest]
#[case::github("owner/skills", "skills")]
#[case::github_nested("org/sub-repo", "sub-repo")]
#[case::git_url_https("https://github.com/dotnet/skills.git", "skills")]
#[case::git_url_no_suffix("https://github.com/dotnet/skills", "skills")]
#[case::git_ssh("git@github.com:owner/repo.git", "repo")]
#[case::local_path("/home/user/my-plugins", "my-plugins")]
#[case::local_tilde("~/marketplaces/mine", "mine")]
#[case::local_relative("./my-market", "my-market")]
fn fallback_name_derives_from_source(#[case] source_str: &str, #[case] expected: &str) {
    let source = MarketplaceSource::detect(source_str);
    let name = source.fallback_name();
    assert_eq!(
        name.as_deref(),
        Some(expected),
        "fallback name for '{source_str}'"
    );
}

#[test]
fn fallback_name_returns_none_for_invalid_name() {
    // A source whose last segment is ".." should fail validation and return None.
    let source = MarketplaceSource::LocalPath {
        path: "/home/user/..".into(),
    };
    assert!(
        source.fallback_name().is_none(),
        "should return None for invalid name"
    );
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p kiro-market-core -- fallback_name`
Expected: compile error — `fallback_name` method does not exist

**Step 3: Implement `fallback_name()`**

Add to the existing `impl MarketplaceSource` block in `cache.rs`:

```rust
/// Derive a marketplace name from the source when no manifest provides one.
///
/// Extracts the last path/URL segment, strips a `.git` suffix if present,
/// and validates the result. Returns `None` if the derived name fails
/// validation.
#[must_use]
pub fn fallback_name(&self) -> Option<String> {
    let raw = match self {
        Self::GitHub { repo } => repo.rsplit('/').next(),
        Self::GitUrl { url } => url
            .rsplit('/')
            .next()
            .or_else(|| url.rsplit(':').next()),
        Self::LocalPath { path } => {
            let trimmed = path.trim_end_matches(['/', '\\']);
            trimmed.rsplit(['/', '\\']).next()
        }
    };

    let segment = raw?;
    let name = segment.strip_suffix(".git").unwrap_or(segment);

    if name.is_empty() {
        return None;
    }

    validation::validate_name(name).ok()?;
    Some(name.to_owned())
}
```

**Step 4: Run test to verify it passes**

Run: `cargo test -p kiro-market-core -- fallback_name`
Expected: PASS

**Step 5: Commit**

```bash
git add crates/kiro-market-core/src/cache.rs
git commit -m "feat: add fallback_name() to derive marketplace name from source"
```

---

### Task 3: Add `discover_plugins()` to `plugin.rs`

**Files:**
- Modify: `crates/kiro-market-core/src/plugin.rs` (add `DiscoveredPlugin` type and `discover_plugins()` function)

**Step 1: Write the failing tests**

Add to the existing `mod tests` in `plugin.rs`:

```rust
use std::path::Path;

/// Helper: create a minimal plugin.json in the given directory.
fn create_plugin_json(dir: &Path, name: &str, description: Option<&str>) {
    fs::create_dir_all(dir).expect("create_dir_all");
    let desc = description
        .map(|d| format!(r#","description":"{d}""#))
        .unwrap_or_default();
    fs::write(
        dir.join("plugin.json"),
        format!(r#"{{"name":"{name}"{desc},"skills":["./skills/"]}}"#),
    )
    .expect("write plugin.json");
}

#[test]
fn discover_plugins_finds_plugin_json_at_depth_1() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();

    create_plugin_json(&root.join("my-plugin"), "my-plugin", Some("A plugin"));

    let discovered = discover_plugins(root, 3);
    assert_eq!(discovered.len(), 1);
    assert_eq!(discovered[0].name, "my-plugin");
    assert_eq!(discovered[0].description.as_deref(), Some("A plugin"));
}

#[test]
fn discover_plugins_finds_plugin_json_at_depth_2() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();

    create_plugin_json(
        &root.join("plugins/dotnet-experimental"),
        "dotnet-experimental",
        Some("Experimental"),
    );

    let discovered = discover_plugins(root, 3);
    assert_eq!(discovered.len(), 1);
    assert_eq!(discovered[0].name, "dotnet-experimental");
}

#[test]
fn discover_plugins_finds_multiple_plugins() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();

    create_plugin_json(&root.join("plugins/alpha"), "alpha", None);
    create_plugin_json(&root.join("plugins/beta"), "beta", None);

    let mut discovered = discover_plugins(root, 3);
    discovered.sort_by(|a, b| a.name.cmp(&b.name));
    assert_eq!(discovered.len(), 2);
    assert_eq!(discovered[0].name, "alpha");
    assert_eq!(discovered[1].name, "beta");
}

#[test]
fn discover_plugins_respects_depth_limit() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();

    // Depth 4 — should NOT be found with max_depth 3.
    create_plugin_json(
        &root.join("a/b/c/deep-plugin"),
        "deep-plugin",
        None,
    );

    let discovered = discover_plugins(root, 3);
    assert!(discovered.is_empty(), "should not find plugin at depth 4");
}

#[test]
fn discover_plugins_skips_hidden_directories() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();

    create_plugin_json(&root.join(".git/hooks"), "git-hooks", None);
    create_plugin_json(&root.join(".claude-plugin"), "claude-internal", None);
    create_plugin_json(&root.join("plugins/visible"), "visible", None);

    let discovered = discover_plugins(root, 3);
    assert_eq!(discovered.len(), 1);
    assert_eq!(discovered[0].name, "visible");
}

#[test]
fn discover_plugins_skips_noise_directories() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();

    create_plugin_json(&root.join("node_modules/some-pkg"), "npm-thing", None);
    create_plugin_json(&root.join("target/debug"), "build-artifact", None);
    create_plugin_json(&root.join("plugins/real"), "real", None);

    let discovered = discover_plugins(root, 3);
    assert_eq!(discovered.len(), 1);
    assert_eq!(discovered[0].name, "real");
}

#[test]
fn discover_plugins_skips_malformed_plugin_json() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();

    let bad_dir = root.join("plugins/bad");
    fs::create_dir_all(&bad_dir).expect("mkdir");
    fs::write(bad_dir.join("plugin.json"), "not json").expect("write");

    create_plugin_json(&root.join("plugins/good"), "good", None);

    let discovered = discover_plugins(root, 3);
    assert_eq!(discovered.len(), 1);
    assert_eq!(discovered[0].name, "good");
}

#[test]
fn discover_plugins_returns_empty_for_no_plugins() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let discovered = discover_plugins(tmp.path(), 3);
    assert!(discovered.is_empty());
}

#[test]
fn discover_plugins_includes_relative_path() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();

    create_plugin_json(&root.join("plugins/my-plugin"), "my-plugin", None);

    let discovered = discover_plugins(root, 3);
    assert_eq!(discovered.len(), 1);
    assert_eq!(
        discovered[0].relative_path,
        Path::new("plugins/my-plugin")
    );
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p kiro-market-core -- discover_plugins`
Expected: compile error — `discover_plugins` and `DiscoveredPlugin` do not exist

**Step 3: Implement `DiscoveredPlugin` and `discover_plugins()`**

Add to `plugin.rs`, above the existing `discover_skill_dirs` function:

```rust
/// Name of the plugin manifest file.
const PLUGIN_JSON: &str = "plugin.json";

/// Directories to skip during recursive plugin scanning.
const SKIP_DIRS: &[&str] = &[
    ".git",
    ".claude-plugin",
    ".github",
    ".kiro",
    "node_modules",
    "target",
    "__pycache__",
    ".venv",
    "vendor",
];

/// A plugin discovered by scanning a repository for `plugin.json` files.
#[derive(Debug, Clone)]
pub struct DiscoveredPlugin {
    /// Plugin name from its `plugin.json`.
    pub name: String,
    /// Plugin description from its `plugin.json`.
    pub description: Option<String>,
    /// Path to the plugin directory, relative to the repo root.
    pub relative_path: PathBuf,
}

/// Scan a repository for `plugin.json` files up to `max_depth` levels deep.
///
/// Skips hidden directories and common noise directories (node_modules, target,
/// etc.). Returns a list of discovered plugins with their names, descriptions,
/// and relative paths.
///
/// Malformed `plugin.json` files are warned about and skipped.
#[must_use]
pub fn discover_plugins(repo_root: &Path, max_depth: usize) -> Vec<DiscoveredPlugin> {
    let mut results = Vec::new();
    scan_for_plugins(repo_root, repo_root, 0, max_depth, &mut results);
    results
}

fn scan_for_plugins(
    repo_root: &Path,
    dir: &Path,
    current_depth: usize,
    max_depth: usize,
    results: &mut Vec<DiscoveredPlugin>,
) {
    if current_depth > max_depth {
        return;
    }

    // Check for plugin.json in this directory (skip the root itself).
    if current_depth > 0 {
        let candidate = dir.join(PLUGIN_JSON);
        if candidate.is_file() {
            match fs::read(&candidate) {
                Ok(bytes) => match PluginManifest::from_json(&bytes) {
                    Ok(manifest) => {
                        let relative_path = dir
                            .strip_prefix(repo_root)
                            .unwrap_or(dir)
                            .to_path_buf();
                        debug!(
                            name = %manifest.name,
                            path = %relative_path.display(),
                            "discovered plugin"
                        );
                        results.push(DiscoveredPlugin {
                            name: manifest.name,
                            description: manifest.description,
                            relative_path,
                        });
                    }
                    Err(e) => {
                        warn!(
                            path = %candidate.display(),
                            error = %e,
                            "skipping malformed plugin.json"
                        );
                    }
                },
                Err(e) => {
                    warn!(
                        path = %candidate.display(),
                        error = %e,
                        "failed to read plugin.json, skipping"
                    );
                }
            }
            // Don't recurse into a plugin directory — it won't contain nested plugins.
            return;
        }
    }

    // Recurse into subdirectories.
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) => {
            debug!(
                path = %dir.display(),
                error = %e,
                "failed to read directory during plugin scan"
            );
            return;
        }
    };

    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                warn!(error = %e, "failed to read directory entry, skipping");
                continue;
            }
        };

        let entry_path = entry.path();
        if !entry_path.is_dir() {
            continue;
        }

        // Skip hidden and noise directories.
        let dir_name = match entry.file_name().to_str() {
            Some(name) => name.to_owned(),
            None => continue,
        };

        if dir_name.starts_with('.') || SKIP_DIRS.contains(&dir_name.as_str()) {
            continue;
        }

        scan_for_plugins(repo_root, &entry_path, current_depth + 1, max_depth, results);
    }
}
```

**Step 4: Run tests to verify they pass**

Run: `cargo test -p kiro-market-core -- discover_plugins`
Expected: all PASS

**Step 5: Run clippy**

Run: `cargo clippy -p kiro-market-core -- -D warnings`
Expected: no warnings

**Step 6: Commit**

```bash
git add crates/kiro-market-core/src/plugin.rs
git commit -m "feat: add discover_plugins() for depth-limited plugin.json scanning"
```

---

### Task 4: Modify `MarketplaceService::add()` to use scan-and-merge

**Files:**
- Modify: `crates/kiro-market-core/src/service.rs:132-199` (update `add()` method)
- Modify: `crates/kiro-market-core/src/service.rs:346-366` (update `read_manifest()`)

**Step 1: Write the failing tests**

Add to the existing `mod tests` in `service.rs`. First, update `MockGitBackend` to support repos without a marketplace manifest and repos with extra plugins.

Add a new mock that creates a repo with plugin.json files but no marketplace.json:

```rust
/// Mock git backend that creates a repo with plugin.json files but no marketplace.json.
#[derive(Debug, Default)]
struct NoManifestGitBackend;

impl GitBackend for NoManifestGitBackend {
    fn clone_repo(&self, _url: &str, dest: &Path, _opts: &CloneOptions) -> Result<(), GitError> {
        // Create two plugin directories with plugin.json but no marketplace.json.
        let plugin_a = dest.join("plugins/alpha");
        fs::create_dir_all(&plugin_a).unwrap();
        fs::write(
            plugin_a.join("plugin.json"),
            r#"{"name":"alpha","description":"Alpha plugin","skills":["./skills/"]}"#,
        )
        .unwrap();

        let plugin_b = dest.join("plugins/beta");
        fs::create_dir_all(&plugin_b).unwrap();
        fs::write(
            plugin_b.join("plugin.json"),
            r#"{"name":"beta","skills":["./skills/"]}"#,
        )
        .unwrap();

        Ok(())
    }

    fn pull_repo(&self, _path: &Path) -> Result<(), GitError> {
        Ok(())
    }

    fn verify_sha(&self, _path: &Path, _expected: &str) -> Result<(), GitError> {
        Ok(())
    }
}

/// Mock that creates a repo with a marketplace.json AND an unlisted plugin.
#[derive(Debug, Default)]
struct MixedGitBackend;

impl GitBackend for MixedGitBackend {
    fn clone_repo(&self, _url: &str, dest: &Path, _opts: &CloneOptions) -> Result<(), GitError> {
        // Create marketplace.json listing one plugin.
        let mp_dir = dest.join(".claude-plugin");
        fs::create_dir_all(&mp_dir).unwrap();
        fs::write(
            mp_dir.join("marketplace.json"),
            r#"{"name":"mixed-market","owner":{"name":"Test"},"plugins":[{"name":"listed","description":"A listed plugin","source":"./plugins/listed"}]}"#,
        )
        .unwrap();

        // Create the listed plugin directory with plugin.json.
        let listed = dest.join("plugins/listed");
        fs::create_dir_all(&listed).unwrap();
        fs::write(
            listed.join("plugin.json"),
            r#"{"name":"listed","description":"A listed plugin","skills":["./skills/"]}"#,
        )
        .unwrap();

        // Create an unlisted plugin directory with plugin.json.
        let unlisted = dest.join("plugins/unlisted");
        fs::create_dir_all(&unlisted).unwrap();
        fs::write(
            unlisted.join("plugin.json"),
            r#"{"name":"unlisted","description":"An unlisted plugin","skills":["./skills/"]}"#,
        )
        .unwrap();

        Ok(())
    }

    fn pull_repo(&self, _path: &Path) -> Result<(), GitError> {
        Ok(())
    }

    fn verify_sha(&self, _path: &Path, _expected: &str) -> Result<(), GitError> {
        Ok(())
    }
}

#[test]
fn add_repo_without_manifest_discovers_plugins_via_scan() {
    let dir = tempfile::tempdir().expect("tempdir");
    let cache = CacheDir::with_root(dir.path().to_path_buf());
    cache.ensure_dirs().expect("ensure_dirs");
    let svc = MarketplaceService::new(cache, NoManifestGitBackend);

    let result = svc
        .add("owner/skills", GitProtocol::Https)
        .expect("add should succeed");

    // Name derived from repo: "skills"
    assert_eq!(result.name, "skills");
    assert_eq!(result.plugins.len(), 2);

    let names: Vec<&str> = result.plugins.iter().map(|p| p.name.as_str()).collect();
    assert!(names.contains(&"alpha"), "should find alpha: {names:?}");
    assert!(names.contains(&"beta"), "should find beta: {names:?}");
}

#[test]
fn add_repo_with_manifest_and_unlisted_plugins_merges_both() {
    let dir = tempfile::tempdir().expect("tempdir");
    let cache = CacheDir::with_root(dir.path().to_path_buf());
    cache.ensure_dirs().expect("ensure_dirs");
    let svc = MarketplaceService::new(cache, MixedGitBackend);

    let result = svc
        .add("owner/repo", GitProtocol::Https)
        .expect("add should succeed");

    assert_eq!(result.name, "mixed-market");
    assert_eq!(result.plugins.len(), 2);

    let names: Vec<&str> = result.plugins.iter().map(|p| p.name.as_str()).collect();
    assert!(names.contains(&"listed"), "should find listed: {names:?}");
    assert!(names.contains(&"unlisted"), "should find unlisted: {names:?}");
}

#[test]
fn add_repo_with_manifest_deduplicates_listed_plugins() {
    let dir = tempfile::tempdir().expect("tempdir");
    let cache = CacheDir::with_root(dir.path().to_path_buf());
    cache.ensure_dirs().expect("ensure_dirs");
    let svc = MarketplaceService::new(cache, MixedGitBackend);

    let result = svc
        .add("owner/repo", GitProtocol::Https)
        .expect("add should succeed");

    // "listed" should appear only once, not duplicated from scan + manifest.
    let listed_count = result
        .plugins
        .iter()
        .filter(|p| p.name == "listed")
        .count();
    assert_eq!(listed_count, 1, "listed plugin should not be duplicated");
}

#[test]
fn add_empty_repo_returns_no_plugins_found_error() {
    let dir = tempfile::tempdir().expect("tempdir");
    let cache = CacheDir::with_root(dir.path().to_path_buf());
    cache.ensure_dirs().expect("ensure_dirs");

    // A backend that creates an empty repo — no marketplace.json, no plugin.json.
    #[derive(Debug)]
    struct EmptyRepoBackend;

    impl GitBackend for EmptyRepoBackend {
        fn clone_repo(
            &self,
            _url: &str,
            dest: &Path,
            _opts: &CloneOptions,
        ) -> Result<(), GitError> {
            fs::create_dir_all(dest).unwrap();
            Ok(())
        }

        fn pull_repo(&self, _path: &Path) -> Result<(), GitError> {
            Ok(())
        }

        fn verify_sha(&self, _path: &Path, _expected: &str) -> Result<(), GitError> {
            Ok(())
        }
    }

    let svc = MarketplaceService::new(cache, EmptyRepoBackend);
    let err = svc
        .add("owner/empty", GitProtocol::Https)
        .expect_err("should fail");

    assert!(
        err.to_string().contains("no plugins found"),
        "expected 'no plugins found' error, got: {err}"
    );
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test -p kiro-market-core -- add_repo_without_manifest add_repo_with_manifest add_empty_repo`
Expected: FAIL — current `add()` errors on missing marketplace.json

**Step 3: Modify `add()` to support scan-and-merge**

Update the `add()` method in `service.rs`. The key changes:

1. `read_manifest()` returns `Option<Marketplace>` instead of `Result<Marketplace, Error>` when the file is simply missing.
2. After reading the manifest (or noting its absence), call `discover_plugins()`.
3. Deduplicate by comparing scanned plugin paths against marketplace `RelativePath` entries.
4. If no manifest and no scanned plugins, return `NoPluginsFound` error.
5. Use manifest name if available, else `fallback_name()`.

Replace `service.rs` `add()` body (lines 132-199) with:

```rust
pub fn add(&self, source: &str, protocol: GitProtocol) -> Result<MarketplaceAddResult, Error> {
    let ms = MarketplaceSource::detect(source);
    self.cache.ensure_dirs()?;

    let temp_name = format!("_pending_{}", std::process::id());
    let temp_dir = self.cache.marketplace_path(&temp_name);

    if temp_dir.exists()
        && let Err(e) = fs::remove_dir_all(&temp_dir)
    {
        warn!(
            path = %temp_dir.display(),
            error = %e,
            "failed to clean up leftover temp directory"
        );
    }

    let mut guard = TempDirGuard::new(temp_dir.clone());

    let link_result = self.clone_or_link(&ms, protocol, &temp_dir)?;

    if link_result == LinkResult::Copied {
        warn!(
            source = %source,
            "marketplace was copied, not linked — local changes will NOT be live-tracked"
        );
    }

    // Try to read marketplace manifest (optional).
    let manifest = self.try_read_manifest(&temp_dir);

    // Scan for plugin.json files.
    let discovered = crate::plugin::discover_plugins(&temp_dir, 3);

    // Build the plugin list: manifest entries first, then discovered (deduplicated).
    let (name, plugins) = match manifest {
        Some(m) => {
            let manifest_name = m.name.clone();
            let mut plugins: Vec<PluginBasicInfo> = m
                .plugins
                .iter()
                .map(|p| PluginBasicInfo {
                    name: p.name.clone(),
                    description: p.description.clone(),
                })
                .collect();

            // Collect marketplace-listed relative paths for dedup.
            let listed_paths: Vec<PathBuf> = m
                .plugins
                .iter()
                .filter_map(|p| match &p.source {
                    crate::marketplace::PluginSource::RelativePath(rel) => {
                        Some(PathBuf::from(rel.trim_start_matches("./")))
                    }
                    _ => None,
                })
                .collect();

            // Collect listed names for dedup of structured sources.
            let listed_names: Vec<&str> =
                m.plugins.iter().map(|p| p.name.as_str()).collect();

            // Add discovered plugins that aren't already listed.
            for dp in &discovered {
                let path_match = listed_paths
                    .iter()
                    .any(|lp| lp == &dp.relative_path);
                let name_match = listed_names.contains(&dp.name.as_str());
                if !path_match && !name_match {
                    plugins.push(PluginBasicInfo {
                        name: dp.name.clone(),
                        description: dp.description.clone(),
                    });
                }
            }

            (manifest_name, plugins)
        }
        None => {
            if discovered.is_empty() {
                return Err(MarketplaceError::NoPluginsFound {
                    path: temp_dir.clone(),
                }
                .into());
            }

            let name = ms.fallback_name().ok_or_else(|| {
                MarketplaceError::InvalidManifest {
                    reason: "no marketplace.json found and could not derive a name from the source; use --name to specify one".into(),
                }
            })?;

            let plugins = discovered
                .iter()
                .map(|dp| PluginBasicInfo {
                    name: dp.name.clone(),
                    description: dp.description.clone(),
                })
                .collect();

            (name, plugins)
        }
    };

    validation::validate_name(&name)?;

    let final_dir = self.cache.marketplace_path(&name);
    if final_dir.exists() {
        return Err(MarketplaceError::AlreadyRegistered { name: name.clone() }.into());
    }

    fs::rename(&temp_dir, &final_dir)?;
    guard.defuse();

    let entry = KnownMarketplace {
        name: name.clone(),
        source: ms,
        protocol: Some(protocol),
        added_at: chrono::Utc::now(),
    };
    self.cache.add_known_marketplace(entry)?;

    debug!(marketplace = %name, "marketplace added");

    Ok(MarketplaceAddResult { name, plugins })
}
```

Add the `try_read_manifest` helper:

```rust
/// Try to read the marketplace manifest. Returns `None` if the file is missing,
/// `None` (with a warning) if malformed, or `Some(manifest)` on success.
fn try_read_manifest(&self, repo_dir: &Path) -> Option<Marketplace> {
    let manifest_path = repo_dir.join(crate::MARKETPLACE_MANIFEST_PATH);
    match fs::read(&manifest_path) {
        Ok(bytes) => match Marketplace::from_json(&bytes) {
            Ok(m) => Some(m),
            Err(e) => {
                warn!(
                    path = %manifest_path.display(),
                    error = %e,
                    "marketplace.json is malformed, falling back to plugin scan"
                );
                None
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            debug!(
                path = %manifest_path.display(),
                "no marketplace.json found, will discover plugins via scan"
            );
            None
        }
        Err(e) => {
            warn!(
                path = %manifest_path.display(),
                error = %e,
                "failed to read marketplace.json, falling back to plugin scan"
            );
            None
        }
    }
}
```

Remove the old `read_manifest` static method (it's replaced by `try_read_manifest`).

**Step 4: Run tests to verify they pass**

Run: `cargo test -p kiro-market-core -- add_`
Expected: all PASS (both old and new tests)

**Step 5: Run clippy**

Run: `cargo clippy -p kiro-market-core -- -D warnings`
Expected: no warnings

**Step 6: Commit**

```bash
git add crates/kiro-market-core/src/service.rs
git commit -m "feat: scan-and-merge in marketplace add — support repos without marketplace.json"
```

---

### Task 5: Update `find_plugin_entry` in CLI to fall back to scan

**Files:**
- Modify: `crates/kiro-market/src/commands/common.rs:12-34` (update `find_plugin_entry`)

**Step 1: Update `find_plugin_entry` to scan on miss**

Change the function to: try marketplace.json first, fall back to `discover_plugins` if the plugin isn't listed:

```rust
/// Read the marketplace manifest and find the matching plugin entry.
///
/// If the plugin is not listed in `marketplace.json` (or the manifest is absent),
/// falls back to a depth-limited scan for `plugin.json` files in the repo.
pub fn find_plugin_entry(
    marketplace_path: &Path,
    plugin_name: &str,
    marketplace_name: &str,
) -> Result<PluginEntry> {
    // Try marketplace.json first.
    let manifest_path = marketplace_path.join(kiro_market_core::MARKETPLACE_MANIFEST_PATH);
    if let Ok(manifest_bytes) = fs::read(&manifest_path) {
        if let Ok(manifest) = Marketplace::from_json(&manifest_bytes) {
            if let Some(entry) = manifest
                .plugins
                .into_iter()
                .find(|p| p.name == plugin_name)
            {
                return Ok(entry);
            }
        }
    }

    // Fall back to scanning for plugin.json.
    let discovered = kiro_market_core::plugin::discover_plugins(marketplace_path, 3);
    if let Some(dp) = discovered.into_iter().find(|dp| dp.name == plugin_name) {
        let relative = format!("./{}", dp.relative_path.display());
        return Ok(PluginEntry {
            name: dp.name,
            description: dp.description,
            source: kiro_market_core::marketplace::PluginSource::RelativePath(relative),
        });
    }

    anyhow::bail!("plugin '{plugin_name}' not found in marketplace '{marketplace_name}'")
}
```

**Step 2: Run the full test suite**

Run: `cargo test --workspace`
Expected: all PASS

**Step 3: Run clippy**

Run: `cargo clippy --workspace -- -D warnings`
Expected: no warnings

**Step 4: Commit**

```bash
git add crates/kiro-market/src/commands/common.rs
git commit -m "feat: install falls back to plugin.json scan when not in marketplace manifest"
```

---

### Task 6: Update CLI output to show discovered plugins on add

**Files:**
- Modify: `crates/kiro-market/src/commands/marketplace.rs` (the `add` subcommand output)

**Step 1: Check current add output formatting**

Read the marketplace add command handler to find where `MarketplaceAddResult.plugins` is printed. Update the output to show the total count naturally — since there's no `discovered` flag, the output just shows all plugins the same way. No code change may be needed if the current output already iterates over all plugins.

**Step 2: Verify the full workflow end-to-end**

Run manually with a real repo (if available) or verify via the test suite:

Run: `cargo test --workspace`
Expected: all PASS

**Step 3: Commit (only if changes were needed)**

```bash
git add crates/kiro-market/src/commands/marketplace.rs
git commit -m "chore: update add command output to show all discovered plugins"
```

---

### Task 7: Final verification and cleanup

**Step 1: Run full test suite**

Run: `cargo test --workspace`
Expected: all PASS

**Step 2: Run clippy on entire workspace**

Run: `cargo clippy --workspace -- -D warnings`
Expected: no warnings

**Step 3: Run cargo build**

Run: `cargo build`
Expected: clean build

**Step 4: Verify existing tests still pass (regression check)**

Run: `cargo test -p kiro-market-core -- add_marketplace add_duplicate remove_marketplace update_`
Expected: all existing service tests still PASS

**Step 5: Commit any final cleanup**

Only if needed.

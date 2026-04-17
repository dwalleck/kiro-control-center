//! `install` command: install a plugin or specific skill into a Kiro project.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use colored::Colorize;
use kiro_market_core::cache::CacheDir;
use kiro_market_core::git::{self, CloneOptions, GitBackend, GitProtocol, GitRef, GixCliBackend};
use kiro_market_core::marketplace::{PluginEntry, PluginSource, StructuredSource};
use kiro_market_core::plugin::{PluginManifest, discover_skill_dirs};
use kiro_market_core::project::KiroProject;
use kiro_market_core::service::{
    FailedSkill, InstallFilter, InstallSkillsResult, InstallWarning, MarketplaceService,
};
use tracing::{debug, warn};

use crate::cli;

/// Run the install command.
///
/// Resolves `plugin_ref` to a plugin, discovers skills, copies skill
/// directories, and installs into the current Kiro project.
pub fn run(plugin_ref: &str, skill_filter: Option<&str>, force: bool) -> Result<()> {
    let (plugin_name, marketplace_name) = cli::parse_plugin_ref(plugin_ref).with_context(|| {
        format!("invalid plugin reference '{plugin_ref}': expected plugin@marketplace")
    })?;

    let cache = CacheDir::default_location()
        .context("could not determine data directory; is $HOME set?")?;
    let marketplace_path = cache.marketplace_path(marketplace_name);
    if !marketplace_path.exists() {
        bail!(
            "marketplace '{}' not found. Run {} first.",
            marketplace_name,
            "kiro-market marketplace add".bold()
        );
    }

    // Look up the stored protocol preference for this marketplace.
    let protocol = match cache.load_known_marketplaces() {
        Ok(entries) => entries
            .into_iter()
            .find(|e| e.name == marketplace_name)
            .and_then(|e| e.protocol)
            .unwrap_or_default(),
        Err(e) => {
            warn!(
                marketplace = marketplace_name,
                error = %e,
                "failed to load marketplace registry; defaulting to HTTPS protocol"
            );
            GitProtocol::default()
        }
    };

    let plugin_entry =
        super::common::find_plugin_entry(&marketplace_path, plugin_name, marketplace_name)?;

    let plugin_dir = resolve_plugin_dir(
        &plugin_entry,
        &marketplace_path,
        &cache,
        marketplace_name,
        protocol,
    )
    .with_context(|| format!("failed to resolve plugin directory for '{plugin_name}'"))?;

    debug!(plugin_dir = %plugin_dir.display(), "resolved plugin directory");

    let plugin_manifest = load_plugin_manifest(&plugin_dir);
    let skill_dirs = discover_plugin_skills(&plugin_dir, plugin_manifest.as_ref());
    let agent_scan_paths = agent_scan_paths(plugin_manifest.as_ref());

    let cwd = std::env::current_dir().context("failed to determine current directory")?;
    let project = KiroProject::new(cwd);
    let version = plugin_manifest.as_ref().and_then(|m| m.version.clone());

    // Build a one-off service just to drive the install loops — the CLI only
    // needs the install calls here, not the full add/update lifecycle.
    let svc = MarketplaceService::new(cache.clone(), GixCliBackend::default());

    let skill_result = if skill_dirs.is_empty() {
        InstallSkillsResult::default()
    } else {
        let filter = match skill_filter {
            Some(name) => InstallFilter::SingleName(name),
            None => InstallFilter::All,
        };
        svc.install_skills(
            &project,
            &skill_dirs,
            &filter,
            force,
            marketplace_name,
            plugin_name,
            version.as_deref(),
        )
    };
    print_install_outcome(plugin_ref, &skill_result);

    // Agents: only run when the user did NOT pass `--skill <name>`. A skill
    // filter narrows the install to one skill and never includes agents.
    let (agents_installed, agent_warnings) = if skill_filter.is_none() {
        match svc.install_plugin_agents(
            &project,
            &plugin_dir,
            &agent_scan_paths,
            marketplace_name,
            plugin_name,
            version.as_deref(),
        ) {
            Ok(tup) => tup,
            Err(e) => {
                eprintln!(
                    "  {} Agent install failed: {}",
                    "✗".red().bold(),
                    format_args!("{e}")
                );
                (0, Vec::new())
            }
        }
    } else {
        (0, Vec::new())
    };
    print_agent_outcome(agents_installed, &agent_warnings);

    if skill_result.installed.is_empty() && skill_result.skipped.is_empty() && agents_installed == 0
    {
        if skill_filter.is_some() {
            bail!("no skills were installed from '{plugin_ref}'");
        }
        bail!("no skills or agents were installed from '{plugin_ref}'");
    }

    Ok(())
}

/// Resolve the list of agent scan paths for a plugin.
fn agent_scan_paths(plugin_manifest: Option<&PluginManifest>) -> Vec<String> {
    if let Some(m) = plugin_manifest.filter(|m| !m.agents.is_empty()) {
        m.agents.clone()
    } else {
        kiro_market_core::DEFAULT_AGENT_PATHS
            .iter()
            .map(|s| (*s).to_string())
            .collect()
    }
}

/// Render the agent install summary and any warnings. Warnings go to
/// stderr so they don't pollute stdout piping, matching the skill flow.
fn print_agent_outcome(installed: usize, warnings: &[InstallWarning]) {
    if installed > 0 {
        println!(
            "  {} Installed {} agent{}",
            "✓".green().bold(),
            installed,
            if installed == 1 { "" } else { "s" }
        );
    }
    for w in warnings {
        eprintln!("  {} {w}", "!".yellow().bold());
    }
}

/// Discover skill directories from a plugin, using its manifest or defaults.
fn discover_plugin_skills(
    plugin_dir: &Path,
    plugin_manifest: Option<&PluginManifest>,
) -> Vec<PathBuf> {
    let skill_paths: Vec<&str> =
        if let Some(manifest) = plugin_manifest.filter(|m| !m.skills.is_empty()) {
            manifest.skills.iter().map(String::as_str).collect()
        } else {
            kiro_market_core::DEFAULT_SKILL_PATHS.to_vec()
        };

    discover_skill_dirs(plugin_dir, &skill_paths)
}

/// Render a per-skill summary plus the rolled-up totals from a service-layer
/// install result. The previous per-line `eprintln!`s during the loop are
/// gone since the service now emits structured `warn!` events; if the user
/// wants more detail they can set `RUST_LOG`.
fn print_install_outcome(plugin_ref: &str, result: &InstallSkillsResult) {
    for name in &result.installed {
        println!("  {} Installed skill '{}'", "✓".green().bold(), name.bold());
    }
    for name in &result.skipped {
        println!(
            "  {} Skill '{}' already installed (use --force to overwrite)",
            "·".yellow().bold(),
            name.bold()
        );
    }
    for FailedSkill { name, error } in &result.failed {
        eprintln!(
            "  {} Failed to install skill '{}': {error}",
            "✗".red().bold(),
            name
        );
    }

    println!();
    if !result.installed.is_empty() {
        println!(
            "{} Installed {} skill{} from {}",
            "✓".green().bold(),
            result.installed.len(),
            if result.installed.len() == 1 { "" } else { "s" },
            plugin_ref.bold()
        );
    }
    if !result.skipped.is_empty() {
        println!(
            "{} Skipped {} already-installed skill{}",
            "·".yellow().bold(),
            result.skipped.len(),
            if result.skipped.len() == 1 { "" } else { "s" }
        );
    }
    if !result.failed.is_empty() {
        println!(
            "{} {} skill{} failed",
            "✗".red().bold(),
            result.failed.len(),
            if result.failed.len() == 1 { "" } else { "s" }
        );
    }
}

/// Resolve the on-disk directory for a plugin based on its source.
fn resolve_plugin_dir(
    entry: &PluginEntry,
    marketplace_path: &Path,
    cache: &CacheDir,
    marketplace_name: &str,
    protocol: GitProtocol,
) -> Result<PathBuf> {
    match &entry.source {
        PluginSource::RelativePath(rel) => {
            let resolved = marketplace_path.join(rel);
            if !resolved.exists() {
                bail!("plugin directory does not exist: {}", resolved.display());
            }
            Ok(resolved)
        }
        PluginSource::Structured(structured) => {
            resolve_structured_source(structured, cache, marketplace_name, &entry.name, protocol)
        }
    }
}

/// Clone a structured source into the cache plugins directory and return the path.
fn resolve_structured_source(
    source: &StructuredSource,
    cache: &CacheDir,
    marketplace_name: &str,
    plugin_name: &str,
    protocol: GitProtocol,
) -> Result<PathBuf> {
    cache
        .ensure_dirs()
        .context("failed to create cache directories")?;

    let dest = cache.plugin_path(marketplace_name, plugin_name);

    // Extract the varying parts from each source variant.
    let (url, subdir, git_ref, sha, label) = match source {
        StructuredSource::GitHub { repo, git_ref, sha } => (
            git::github_repo_to_url(repo, protocol),
            None,
            git_ref.as_deref(),
            sha.as_deref(),
            repo.as_str(),
        ),
        StructuredSource::GitUrl { url, git_ref, sha } => (
            url.clone(),
            None,
            git_ref.as_deref(),
            sha.as_deref(),
            url.as_str(),
        ),
        StructuredSource::GitSubdir {
            url,
            path,
            git_ref,
            sha,
        } => (
            url.clone(),
            Some(path.as_str()),
            git_ref.as_deref(),
            sha.as_deref(),
            url.as_str(),
        ),
    };

    let backend = GixCliBackend::default();

    // If already cloned, reuse — but re-verify SHA so a corrupt or stale
    // cache (e.g. the manifest's pinned SHA changed) cannot pass silently.
    if dest.exists() {
        debug!(dest = %dest.display(), "plugin already cached, reusing");
        if let Some(expected) = sha {
            backend.verify_sha(&dest, expected).with_context(|| {
                format!(
                    "cached plugin at {} fails SHA verification for '{label}' \
                     (expected {expected}); delete the cache directory and retry",
                    dest.display()
                )
            })?;
        }
        return Ok(match subdir {
            Some(path) => dest.join(path),
            None => dest,
        });
    }

    debug!(url = %url, dest = %dest.display(), "cloning plugin");
    print!("  Cloning {label}...");
    // Validate the manifest-supplied git ref shape before passing to the
    // backend. Treat failure as a clone-time error since this comes from
    // untrusted manifest data.
    let validated_ref = git_ref
        .map(GitRef::new)
        .transpose()
        .with_context(|| format!("invalid git ref in manifest for '{label}'"))?;
    let opts = CloneOptions {
        git_ref: validated_ref,
    };
    backend
        .clone_repo(&url, &dest, &opts)
        .with_context(|| format!("failed to clone plugin from '{label}'"))?;
    println!(" done");

    if let Some(expected) = sha {
        backend
            .verify_sha(&dest, expected)
            .with_context(|| format!("SHA verification failed for '{label}'"))?;
    }

    Ok(match subdir {
        Some(path) => dest.join(path),
        None => dest,
    })
}

/// Load a `plugin.json` from the given directory, returning `None` if missing.
fn load_plugin_manifest(plugin_dir: &Path) -> Option<PluginManifest> {
    let manifest_path = plugin_dir.join("plugin.json");
    match fs::read(&manifest_path) {
        Ok(bytes) => match PluginManifest::from_json(&bytes) {
            Ok(manifest) => {
                debug!(name = %manifest.name, "loaded plugin manifest");
                Some(manifest)
            }
            Err(e) => {
                warn!(
                    path = %manifest_path.display(),
                    error = %e,
                    "plugin.json is malformed, falling back to defaults"
                );
                None
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            debug!(
                path = %manifest_path.display(),
                "plugin.json not found, using defaults"
            );
            None
        }
        Err(e) => {
            warn!(
                path = %manifest_path.display(),
                error = %e,
                "failed to read plugin.json, falling back to defaults"
            );
            None
        }
    }
}

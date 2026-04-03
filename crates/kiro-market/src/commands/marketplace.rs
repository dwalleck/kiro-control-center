//! `marketplace` subcommand: add, list, update, and remove marketplace sources.

use std::fs;

use anyhow::{Context, Result, bail};
use colored::Colorize;
use kiro_market_core::cache::{CacheDir, KnownMarketplace, MarketplaceSource};
use kiro_market_core::git;
use kiro_market_core::marketplace::Marketplace;
use tracing::debug;

use crate::cli::MarketplaceAction;

/// Dispatch to the appropriate marketplace subcommand.
pub fn run(action: &MarketplaceAction) -> Result<()> {
    match action {
        MarketplaceAction::Add { source } => add(source),
        MarketplaceAction::List => list(),
        MarketplaceAction::Update { name } => update(name.as_deref()),
        MarketplaceAction::Remove { name } => remove(name),
    }
}

// ---------------------------------------------------------------------------
// Source detection
// ---------------------------------------------------------------------------

/// Classify a user-provided source string into a `MarketplaceSource`.
fn detect_source(source: &str) -> MarketplaceSource {
    if source.starts_with("http://") || source.starts_with("https://") || source.starts_with("git@")
    {
        MarketplaceSource::GitUrl {
            url: source.to_owned(),
        }
    } else if source.starts_with('/')
        || source.starts_with("./")
        || source.starts_with("../")
        || source.starts_with('~')
    {
        MarketplaceSource::LocalPath {
            path: source.to_owned(),
        }
    } else {
        // Treat as GitHub owner/repo shorthand.
        MarketplaceSource::GitHub {
            repo: source.to_owned(),
        }
    }
}

/// Return a human-readable label describing the source type.
fn source_label(source: &MarketplaceSource) -> &str {
    match source {
        MarketplaceSource::GitHub { .. } => "github",
        MarketplaceSource::GitUrl { .. } => "git",
        MarketplaceSource::LocalPath { .. } => "local",
    }
}

// ---------------------------------------------------------------------------
// add
// ---------------------------------------------------------------------------

/// Add a new marketplace source.
///
/// 1. Detect source type (GitHub, git URL, local path).
/// 2. Clone (or symlink for local paths) into the cache.
/// 3. Read the marketplace manifest to discover the real name.
/// 4. Rename the clone directory to the real marketplace name.
/// 5. Register in `known_marketplaces.json`.
fn add(source: &str) -> Result<()> {
    let ms = detect_source(source);
    let cache = CacheDir::default_location()
        .context("could not determine data directory; is $HOME set?")?;
    cache
        .ensure_dirs()
        .context("failed to create cache directories")?;

    // Clone or symlink into a temporary name first, then rename once we
    // know the real marketplace name from the manifest.
    let temp_name = format!("_pending_{}", std::process::id());
    let temp_dir = cache.marketplace_path(&temp_name);

    // Clean up any leftover temp directory from a prior interrupted run.
    if temp_dir.exists() {
        fs::remove_dir_all(&temp_dir)
            .with_context(|| format!("failed to clean up {}", temp_dir.display()))?;
    }

    match &ms {
        MarketplaceSource::GitHub { repo } => {
            let url = git::github_repo_to_url(repo);
            debug!(url = %url, dest = %temp_dir.display(), "cloning GitHub marketplace");
            print!("  Cloning {repo}...");
            git::clone_repo(&url, &temp_dir, None)
                .with_context(|| format!("failed to clone {repo}"))?;
            println!(" done");
        }
        MarketplaceSource::GitUrl { url } => {
            debug!(url = %url, dest = %temp_dir.display(), "cloning git marketplace");
            print!("  Cloning {url}...");
            git::clone_repo(url, &temp_dir, None)
                .with_context(|| format!("failed to clone {url}"))?;
            println!(" done");
        }
        MarketplaceSource::LocalPath { path } => {
            let src = resolve_local_path(path)?;
            debug!(src = %src.display(), dest = %temp_dir.display(), "symlinking local marketplace");
            #[cfg(unix)]
            std::os::unix::fs::symlink(&src, &temp_dir).with_context(|| {
                format!(
                    "failed to symlink {} -> {}",
                    src.display(),
                    temp_dir.display()
                )
            })?;
            #[cfg(not(unix))]
            bail!("local path marketplaces are only supported on Unix");
        }
    }

    // Read marketplace manifest to get the real name.
    let manifest_path = temp_dir.join(kiro_market_core::MARKETPLACE_MANIFEST_PATH);
    let manifest_bytes = fs::read(&manifest_path).with_context(|| {
        format!(
            "marketplace manifest not found at {}",
            manifest_path.display()
        )
    })?;
    let manifest =
        Marketplace::from_json(&manifest_bytes).context("failed to parse marketplace manifest")?;

    let name = manifest.name.clone();
    kiro_market_core::validation::validate_name(&name).with_context(|| {
        format!("marketplace manifest contains invalid name '{name}'")
    })?;
    let plugin_count = manifest.plugins.len();

    // Rename temp dir to the real marketplace name.
    let final_dir = cache.marketplace_path(&name);
    if final_dir.exists() {
        // Clean up the temp clone before reporting the error.
        let _ = fs::remove_dir_all(&temp_dir);
        bail!(
            "marketplace directory already exists at {}",
            final_dir.display()
        );
    }
    fs::rename(&temp_dir, &final_dir).with_context(|| {
        format!(
            "failed to rename {} -> {}",
            temp_dir.display(),
            final_dir.display()
        )
    })?;

    // Register in known_marketplaces.json.
    let entry = KnownMarketplace {
        name: name.clone(),
        source: ms,
        added_at: chrono::Utc::now(),
    };
    cache
        .add_known_marketplace(entry)
        .with_context(|| format!("failed to register marketplace '{name}'"))?;

    println!(
        "{} Added marketplace {} ({} plugin{})",
        "✓".green().bold(),
        name.bold(),
        plugin_count,
        if plugin_count == 1 { "" } else { "s" }
    );

    print_available_plugins(&manifest.plugins, &name);

    Ok(())
}

/// Resolve a local path string to an absolute path.
///
/// Handles `~` expansion and canonicalization.
fn resolve_local_path(path_str: &str) -> Result<std::path::PathBuf> {
    let expanded = if let Some(rest) = path_str.strip_prefix('~') {
        let home = dirs::home_dir().context("could not determine home directory")?;
        if rest.is_empty() {
            home
        } else {
            // Strip the leading '/' from rest.
            home.join(rest.trim_start_matches('/'))
        }
    } else {
        std::path::PathBuf::from(path_str)
    };

    expanded
        .canonicalize()
        .with_context(|| format!("failed to resolve path: {path_str}"))
}

/// Print the list of available plugins after adding a marketplace.
fn print_available_plugins(plugins: &[kiro_market_core::marketplace::PluginEntry], marketplace_name: &str) {
    if plugins.is_empty() {
        return;
    }

    println!();
    println!("  {}", "Available plugins:".bold());
    for plugin in plugins {
        let desc = plugin
            .description
            .as_deref()
            .unwrap_or("(no description)");
        println!("    {} - {}", plugin.name.green(), desc);
    }
    println!();
    println!(
        "  Install with: {}",
        format!("kiro-market install <plugin>@{marketplace_name}").bold()
    );
}

// ---------------------------------------------------------------------------
// list
// ---------------------------------------------------------------------------

/// List all registered marketplaces.
fn list() -> Result<()> {
    let cache = CacheDir::default_location()
        .context("could not determine data directory; is $HOME set?")?;
    let entries = cache
        .load_known_marketplaces()
        .context("failed to load known marketplaces")?;

    if entries.is_empty() {
        println!(
            "No marketplaces registered. Use {} to add one.",
            "kiro-market marketplace add".bold()
        );
        return Ok(());
    }

    println!("{}", "Registered marketplaces:".bold());
    for entry in &entries {
        println!(
            "  {} ({})",
            entry.name.green().bold(),
            source_label(&entry.source)
        );
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// update
// ---------------------------------------------------------------------------

/// Update marketplace clone(s) from remote.
fn update(name: Option<&str>) -> Result<()> {
    let cache = CacheDir::default_location()
        .context("could not determine data directory; is $HOME set?")?;
    let entries = cache
        .load_known_marketplaces()
        .context("failed to load known marketplaces")?;

    if entries.is_empty() {
        println!("No marketplaces registered.");
        return Ok(());
    }

    let targets: Vec<_> = if let Some(name) = name {
        let filtered: Vec<_> = entries.iter().filter(|e| e.name == name).collect();
        if filtered.is_empty() {
            bail!("marketplace '{name}' is not registered");
        }
        filtered
    } else {
        entries.iter().collect()
    };

    let mut failures = 0u32;

    for entry in &targets {
        let mp_path = cache.marketplace_path(&entry.name);

        // Skip symlinked (local) marketplaces -- they always reflect the
        // latest state on disk.
        if mp_path.is_symlink() {
            println!("  {} {} (local, skipped)", "·".bold(), entry.name.bold());
            continue;
        }

        print!("  Updating {}...", entry.name.bold());
        match git::pull_repo(&mp_path) {
            Ok(()) => {
                println!(" {}", "done".green());
            }
            Err(e) => {
                println!(" {}: {}", "failed".red(), e);
                failures += 1;
            }
        }
    }

    if failures > 0 {
        bail!(
            "{failures} marketplace{} failed to update",
            if failures == 1 { "" } else { "s" }
        );
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// remove
// ---------------------------------------------------------------------------

/// Remove a registered marketplace and its cached data.
fn remove(name: &str) -> Result<()> {
    let cache = CacheDir::default_location()
        .context("could not determine data directory; is $HOME set?")?;

    // Remove from the registry first.
    cache
        .remove_known_marketplace(name)
        .with_context(|| format!("failed to remove marketplace '{name}'"))?;

    // Remove the cloned directory or symlink.
    let mp_path = cache.marketplace_path(name);
    if mp_path.is_symlink() {
        fs::remove_file(&mp_path)
            .with_context(|| format!("failed to remove symlink {}", mp_path.display()))?;
    } else if mp_path.exists() {
        fs::remove_dir_all(&mp_path)
            .with_context(|| format!("failed to remove directory {}", mp_path.display()))?;
    }

    println!("{} Removed marketplace {}", "✓".green().bold(), name.bold());

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_source_github_shorthand() {
        let source = detect_source("microsoft/dotnet-skills");
        assert!(
            matches!(source, MarketplaceSource::GitHub { repo } if repo == "microsoft/dotnet-skills")
        );
    }

    #[test]
    fn detect_source_https_url() {
        let source = detect_source("https://github.com/owner/repo.git");
        assert!(
            matches!(source, MarketplaceSource::GitUrl { url } if url == "https://github.com/owner/repo.git")
        );
    }

    #[test]
    fn detect_source_git_ssh_url() {
        let source = detect_source("git@github.com:owner/repo.git");
        assert!(
            matches!(source, MarketplaceSource::GitUrl { url } if url == "git@github.com:owner/repo.git")
        );
    }

    #[test]
    fn detect_source_http_url() {
        let source = detect_source("http://example.com/repo.git");
        assert!(matches!(source, MarketplaceSource::GitUrl { .. }));
    }

    #[test]
    fn detect_source_absolute_path() {
        let source = detect_source("/home/user/marketplace");
        assert!(
            matches!(source, MarketplaceSource::LocalPath { path } if path == "/home/user/marketplace")
        );
    }

    #[test]
    fn detect_source_relative_dot() {
        let source = detect_source("./my-marketplace");
        assert!(matches!(source, MarketplaceSource::LocalPath { .. }));
    }

    #[test]
    fn detect_source_relative_dotdot() {
        let source = detect_source("../other/marketplace");
        assert!(matches!(source, MarketplaceSource::LocalPath { .. }));
    }

    #[test]
    fn detect_source_tilde() {
        let source = detect_source("~/marketplaces/mine");
        assert!(matches!(source, MarketplaceSource::LocalPath { .. }));
    }

    #[test]
    fn source_label_values() {
        assert_eq!(
            source_label(&MarketplaceSource::GitHub {
                repo: String::new()
            }),
            "github"
        );
        assert_eq!(
            source_label(&MarketplaceSource::GitUrl { url: String::new() }),
            "git"
        );
        assert_eq!(
            source_label(&MarketplaceSource::LocalPath {
                path: String::new()
            }),
            "local"
        );
    }
}

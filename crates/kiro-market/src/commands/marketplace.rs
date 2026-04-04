//! `marketplace` subcommand: add, list, update, and remove marketplace sources.

use anyhow::{Context, Result};
use colored::Colorize;
use kiro_market_core::cache::CacheDir;
use kiro_market_core::git::GixCliBackend;
use kiro_market_core::service::MarketplaceService;

use crate::cli::MarketplaceAction;

/// Dispatch to the appropriate marketplace subcommand.
pub fn run(action: &MarketplaceAction) -> Result<()> {
    let cache = CacheDir::default_location()
        .context("could not determine data directory; is $HOME set?")?;
    let git = GixCliBackend::default();
    let svc = MarketplaceService::new(cache, git);

    match action {
        MarketplaceAction::Add { source, protocol } => add(&svc, source, *protocol),
        MarketplaceAction::List => list(&svc),
        MarketplaceAction::Update { name } => update(&svc, name.as_deref()),
        MarketplaceAction::Remove { name } => remove(&svc, name),
    }
}

fn add(
    svc: &MarketplaceService,
    source: &str,
    protocol: kiro_market_core::git::GitProtocol,
) -> Result<()> {
    print!("  Adding marketplace...");
    let result = svc
        .add(source, protocol)
        .context("failed to add marketplace")?;

    println!(
        " {} Added {} ({} plugin{})",
        "✓".green().bold(),
        result.name.bold(),
        result.plugins.len(),
        if result.plugins.len() == 1 { "" } else { "s" }
    );

    if !result.plugins.is_empty() {
        println!();
        println!("  {}", "Available plugins:".bold());
        for plugin in &result.plugins {
            let desc = plugin.description.as_deref().unwrap_or("(no description)");
            println!("    {} - {}", plugin.name.green(), desc);
        }
        println!();
        println!(
            "  Install with: {}",
            format!("kiro-market install <plugin>@{}", result.name).bold()
        );
    }

    Ok(())
}

fn list(svc: &MarketplaceService) -> Result<()> {
    let entries = svc.list().context("failed to load marketplaces")?;

    if entries.is_empty() {
        println!(
            "No marketplaces registered. Use {} to add one.",
            "kiro-market marketplace add".bold()
        );
        return Ok(());
    }

    println!("{}", "Registered marketplaces:".bold());
    for entry in &entries {
        println!("  {} ({})", entry.name.green().bold(), entry.source.label());
    }

    Ok(())
}

fn update(svc: &MarketplaceService, name: Option<&str>) -> Result<()> {
    let result = svc.update(name).context("failed to update marketplaces")?;

    for name in &result.skipped {
        println!("  {} {} (local, skipped)", "·".bold(), name.bold());
    }
    for name in &result.updated {
        println!(
            "  {} {} {}",
            "✓".green().bold(),
            name.bold(),
            "done".green()
        );
    }
    for fail in &result.failed {
        println!(
            "  {} {} {}",
            "✗".red().bold(),
            fail.name.bold(),
            fail.error.red()
        );
    }

    if !result.failed.is_empty() {
        anyhow::bail!(
            "{} marketplace{} failed to update",
            result.failed.len(),
            if result.failed.len() == 1 { "" } else { "s" }
        );
    }

    Ok(())
}

fn remove(svc: &MarketplaceService, name: &str) -> Result<()> {
    svc.remove(name)
        .with_context(|| format!("failed to remove marketplace '{name}'"))?;
    println!("{} Removed marketplace {}", "✓".green().bold(), name.bold());
    Ok(())
}

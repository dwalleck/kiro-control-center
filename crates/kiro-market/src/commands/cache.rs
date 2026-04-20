//! `cache` subcommand: inspect and clean up the on-disk cache.

use anyhow::{Context, Result};
use colored::Colorize;
use kiro_market_core::cache::{CacheDir, PruneMode};

use crate::cli::CacheAction;

/// Dispatch to the appropriate cache subcommand.
pub fn run(action: &CacheAction) -> Result<()> {
    let cache = CacheDir::default_location()
        .context("could not determine data directory; is $HOME set?")?;

    match action {
        CacheAction::Prune { dry_run } => {
            let mode = if *dry_run {
                PruneMode::DryRun
            } else {
                PruneMode::Apply
            };
            prune(&cache, mode)
        }
    }
}

fn prune(cache: &CacheDir, mode: PruneMode) -> Result<()> {
    let report = cache.prune_orphans(mode).context("failed to prune cache")?;
    let dry_run = matches!(mode, PruneMode::DryRun);

    if report.targets.is_empty() && report.failed.is_empty() {
        println!("{} cache is clean — no orphaned entries found", "✓".green());
        return Ok(());
    }

    let verb = if dry_run { "Would remove" } else { "Removed" };
    if !report.targets.is_empty() {
        println!("{verb}:");
        for path in &report.targets {
            println!("  {} {}", "·".bold(), path.display());
        }
    }

    if !report.failed.is_empty() {
        println!("\n{}", "Failed:".red().bold());
        for failure in &report.failed {
            println!(
                "  {} {} — {}",
                "✗".red().bold(),
                failure.path.display(),
                failure.error.red()
            );
        }
    }

    if dry_run {
        println!(
            "\n{} run without --dry-run to actually delete",
            "hint:".yellow().bold()
        );
    } else {
        println!(
            "\n{} {} entr{} cleaned up",
            "✓".green().bold(),
            report.targets.len(),
            if report.targets.len() == 1 {
                "y"
            } else {
                "ies"
            }
        );
    }

    if !report.failed.is_empty() {
        anyhow::bail!(
            "{} entr{} failed to delete",
            report.failed.len(),
            if report.failed.len() == 1 { "y" } else { "ies" }
        );
    }

    Ok(())
}

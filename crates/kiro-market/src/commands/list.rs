//! `list` command: list installed skills in the current project.

use anyhow::{Context, Result};
use colored::Colorize;
use kiro_market_core::project::KiroProject;

/// Run the list command.
///
/// Loads the installed skills from the current project and prints each one
/// with its name, plugin, marketplace, and optional version.
pub fn run() -> Result<()> {
    let cwd = std::env::current_dir().context("failed to determine current directory")?;
    let project = KiroProject::new(cwd);

    let installed = project
        .load_installed()
        .context("failed to load installed skills")?;

    if installed.skills.is_empty() {
        println!(
            "No skills installed. Use {} to install skills.",
            "kiro-market install".bold()
        );
        return Ok(());
    }

    println!("{}", "Installed skills:".bold());
    println!();

    // Sort by name for deterministic output.
    let mut skills: Vec<_> = installed.skills.iter().collect();
    skills.sort_by_key(|(name, _)| name.as_str());

    for (name, meta) in &skills {
        let version_str = meta
            .version
            .as_deref()
            .map_or(String::new(), |v| format!(" v{v}"));

        println!(
            "  {} ({}@{}{})",
            name.green().bold(),
            meta.plugin,
            meta.marketplace,
            version_str
        );
    }

    println!();
    println!(
        "{} skill{} installed.",
        skills.len(),
        if skills.len() == 1 { "" } else { "s" }
    );

    Ok(())
}

//! `remove` command: remove an installed skill.

use anyhow::{Context, Result};
use colored::Colorize;
use kiro_market_core::project::KiroProject;

/// Run the remove command.
///
/// Removes the named skill from the current Kiro project.
pub fn run(skill_name: &str) -> Result<()> {
    let cwd = std::env::current_dir().context("failed to determine current directory")?;
    let project = KiroProject::new(cwd);

    project
        .remove_skill(skill_name)
        .with_context(|| format!("failed to remove skill '{skill_name}'"))?;

    println!(
        "{} Removed skill '{}'",
        "✓".green().bold(),
        skill_name.bold()
    );

    Ok(())
}

//! `update` command: update installed plugins.

use anyhow::Result;
use colored::Colorize;

/// Run the update command.
///
/// Currently advises users to use `remove` + `install --force` as a workaround.
/// A proper in-place update mechanism can be added later.
#[allow(clippy::unnecessary_wraps)]
pub fn run(plugin_ref: Option<&str>) -> Result<()> {
    let target = plugin_ref.map_or_else(|| "all plugins".to_owned(), |r| format!("'{r}'"));

    println!(
        "{} In-place update for {} is not yet supported.",
        "!".yellow().bold(),
        target
    );
    println!();
    println!("To update, use:");
    println!(
        "  1. {} to remove the skill",
        "kiro-market remove <skill-name>".bold()
    );
    println!(
        "  2. {} to reinstall",
        "kiro-market install <plugin@marketplace> --force".bold()
    );

    Ok(())
}

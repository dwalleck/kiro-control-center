//! `update` command: update installed plugins.

use anyhow::{Result, bail};
use colored::Colorize;

/// Run the update command.
///
/// Currently advises users to use `remove` + `install --force` as a workaround.
/// A proper in-place update mechanism can be added later.
///
/// Returns a non-zero exit code (via `Err`) so CI pipelines that invoke
/// `kiro-market update` can distinguish "updated successfully" from
/// "updating is not implemented yet."
pub fn run(plugin_ref: Option<&str>) -> Result<()> {
    let target = plugin_ref.map_or_else(|| "all plugins".to_owned(), |r| format!("'{r}'"));

    eprintln!(
        "{} In-place update for {} is not yet supported.",
        "!".yellow().bold(),
        target
    );
    eprintln!();
    eprintln!("To update, use:");
    eprintln!(
        "  1. {} to remove the skill",
        "kiro-market remove <skill-name>".bold()
    );
    eprintln!(
        "  2. {} to reinstall",
        "kiro-market install <plugin@marketplace> --force".bold()
    );

    bail!("update command is not yet implemented");
}

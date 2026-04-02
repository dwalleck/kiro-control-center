//! `update` command: update installed plugins.

use anyhow::Result;

/// Run the update command.
#[allow(clippy::unnecessary_wraps)]
pub fn run(plugin_ref: Option<&str>) -> Result<()> {
    let label = plugin_ref.unwrap_or("all");
    println!("update: {label}");
    Ok(())
}

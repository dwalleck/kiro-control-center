//! `info` command: show detailed information about a plugin.

use anyhow::Result;

/// Run the info command.
#[allow(clippy::unnecessary_wraps)]
pub fn run(plugin_ref: &str) -> Result<()> {
    println!("info: {plugin_ref}");
    Ok(())
}

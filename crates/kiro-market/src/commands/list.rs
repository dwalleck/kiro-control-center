//! `list` command: list installed skills in the current project.

use anyhow::Result;

/// Run the list command.
#[allow(clippy::unnecessary_wraps)]
pub fn run() -> Result<()> {
    println!("list installed skills");
    Ok(())
}

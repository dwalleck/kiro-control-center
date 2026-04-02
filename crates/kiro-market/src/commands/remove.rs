//! `remove` command: remove an installed skill.

use anyhow::Result;

/// Run the remove command.
#[allow(clippy::unnecessary_wraps)]
pub fn run(skill_name: &str) -> Result<()> {
    println!("remove: {skill_name}");
    Ok(())
}

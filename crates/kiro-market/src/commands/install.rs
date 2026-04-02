//! `install` command: install a plugin or specific skill into a Kiro project.

use anyhow::Result;

/// Run the install command.
#[allow(clippy::unnecessary_wraps)]
pub fn run(plugin_ref: &str, skill: Option<&str>, force: bool) -> Result<()> {
    println!("install: plugin_ref={plugin_ref}, skill={skill:?}, force={force}");
    Ok(())
}

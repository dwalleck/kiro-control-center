//! `search` command: search plugins across registered marketplaces.

use anyhow::Result;

/// Run the search command.
#[allow(clippy::unnecessary_wraps)]
pub fn run(query: &str) -> Result<()> {
    println!("search: {query}");
    Ok(())
}

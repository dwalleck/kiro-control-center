//! `marketplace` subcommand: add, list, update, and remove marketplace sources.

use anyhow::Result;

use crate::cli::MarketplaceAction;

/// Dispatch to the appropriate marketplace subcommand.
#[allow(clippy::unnecessary_wraps)]
pub fn run(action: &MarketplaceAction) -> Result<()> {
    match action {
        MarketplaceAction::Add { source } => {
            println!("marketplace add: {source}");
            Ok(())
        }
        MarketplaceAction::List => {
            println!("marketplace list");
            Ok(())
        }
        MarketplaceAction::Update { name } => {
            let label = name.as_deref().unwrap_or("all");
            println!("marketplace update: {label}");
            Ok(())
        }
        MarketplaceAction::Remove { name } => {
            println!("marketplace remove: {name}");
            Ok(())
        }
    }
}

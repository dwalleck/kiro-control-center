//! kiro-market CLI binary.

mod cli;
mod commands;

use anyhow::Result;
use clap::Parser;
use tracing_subscriber::EnvFilter;

fn main() -> Result<()> {
    let cli = cli::Cli::parse();

    let default_filter = match cli.verbose {
        0 => "kiro_market=info",
        1 => "kiro_market=debug",
        _ => "kiro_market=trace",
    };

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_filter)),
        )
        .with_target(false)
        .init();

    match &cli.command {
        cli::Command::Marketplace { action } => commands::marketplace::run(action),
        cli::Command::Search { query } => commands::search::run(query.as_deref()),
        cli::Command::Install {
            plugin_ref,
            skill,
            force,
        } => commands::install::run(plugin_ref, skill.as_deref(), *force),
        cli::Command::List => commands::list::run(),
        cli::Command::Update { plugin_ref } => commands::update::run(plugin_ref.as_deref()),
        cli::Command::Remove { skill_name } => commands::remove::run(skill_name),
        cli::Command::Info { plugin_ref } => commands::info::run(plugin_ref),
    }
}

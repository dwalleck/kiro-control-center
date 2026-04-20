//! kiro-market CLI binary.

mod cli;
mod commands;

use std::io::IsTerminal;

use anyhow::Result;
use clap::Parser;
use tracing_subscriber::EnvFilter;

fn main() -> Result<()> {
    let cli = cli::Cli::parse();

    // Disable ANSI color codes when stdout is redirected to a file/pipe
    // (ls > out.log, kiro-market list | less) or when the user has set
    // NO_COLOR (https://no-color.org/). The `colored` crate does a best
    // effort itself, but setting this explicitly is less fragile than
    // relying on auto-detection and keeps behaviour testable.
    let force_no_color = std::env::var_os("NO_COLOR").is_some() || !std::io::stdout().is_terminal();
    if force_no_color {
        colored::control::set_override(false);
    }

    // Default filter explicitly admits `kiro_market_core=warn` because
    // several security-critical "skip + warn" paths live in the core crate
    // (e.g. project::copy_dir_recursive logging a hardlink/symlink skip,
    // platform::sys::StagingGuard logging a Windows staging-cleanup
    // failure). Without the core crate listed here, the default
    // `kiro_market=info` filter would silently drop those warnings, and
    // a user whose install was incomplete because of a hardlink in the
    // source tree would see a successful "✓ installed" with no signal
    // that files were dropped. Bumping verbosity escalates both crates
    // in lockstep so `-v` and `-vv` continue to be the way to see
    // everything.
    let default_filter = match cli.verbose {
        0 => "kiro_market=info,kiro_market_core=warn",
        1 => "kiro_market=debug,kiro_market_core=debug",
        _ => "kiro_market=trace,kiro_market_core=trace",
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
            accept_mcp,
        } => commands::install::run(plugin_ref, skill.as_deref(), *force, *accept_mcp),
        cli::Command::List => commands::list::run(),
        cli::Command::Update { plugin_ref } => commands::update::run(plugin_ref.as_deref()),
        cli::Command::Remove { skill_name } => commands::remove::run(skill_name),
        cli::Command::Info { plugin_ref } => commands::info::run(plugin_ref),
        cli::Command::Cache { action } => commands::cache::run(action),
    }
}

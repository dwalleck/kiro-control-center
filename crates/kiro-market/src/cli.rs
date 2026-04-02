//! Command-line argument definitions using `clap` derive API.

use clap::{Parser, Subcommand};

/// Install Claude Code marketplace skills into Kiro CLI projects.
#[derive(Parser, Debug)]
#[command(name = "kiro-market", about, version)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,

    /// Increase verbosity (-v, -vv, -vvv).
    #[arg(short, long, action = clap::ArgAction::Count)]
    pub verbose: u8,
}

/// Top-level subcommands.
#[derive(Subcommand, Debug)]
pub enum Command {
    /// Manage marketplace sources (add, list, update, remove).
    Marketplace {
        #[command(subcommand)]
        action: MarketplaceAction,
    },
    /// Search plugins across all registered marketplaces.
    Search {
        /// Search query string.
        query: String,
    },
    /// Install a plugin (or a specific skill from a plugin).
    Install {
        /// Plugin reference in the form `plugin@marketplace`.
        plugin_ref: String,
        /// Install only the named skill instead of the entire plugin.
        #[arg(long)]
        skill: Option<String>,
        /// Overwrite existing skills without prompting.
        #[arg(long)]
        force: bool,
    },
    /// List all installed skills in the current project.
    List,
    /// Update installed plugins (or a specific one).
    Update {
        /// Plugin reference to update; updates all if omitted.
        plugin_ref: Option<String>,
    },
    /// Remove an installed skill from the current project.
    Remove {
        /// Name of the skill to remove.
        skill_name: String,
    },
    /// Show detailed information about a plugin.
    Info {
        /// Plugin reference in the form `plugin@marketplace`.
        plugin_ref: String,
    },
}

/// Subcommands for `marketplace` management.
#[derive(Subcommand, Debug)]
pub enum MarketplaceAction {
    /// Add a new marketplace source (GitHub owner/repo, git URL, or local path).
    Add {
        /// Source string: `owner/repo`, a git URL, or a local path.
        source: String,
    },
    /// List all registered marketplaces.
    List,
    /// Update marketplace clone(s) from remote.
    Update {
        /// Marketplace name to update; updates all if omitted.
        name: Option<String>,
    },
    /// Remove a registered marketplace.
    Remove {
        /// Name of the marketplace to remove.
        name: String,
    },
}

/// Parse a `"plugin@marketplace"` reference into `(plugin, marketplace)`.
///
/// Returns `None` if the string does not contain exactly one `@`.
#[must_use]
#[allow(dead_code)]
pub fn parse_plugin_ref(plugin_ref: &str) -> Option<(&str, &str)> {
    plugin_ref.split_once('@')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_plugin_ref_valid() {
        assert_eq!(
            parse_plugin_ref("dotnet@microsoft"),
            Some(("dotnet", "microsoft"))
        );
    }

    #[test]
    fn parse_plugin_ref_no_at() {
        assert_eq!(parse_plugin_ref("dotnet"), None);
    }

    #[test]
    fn parse_plugin_ref_multiple_at() {
        // split_once splits on the first '@' only.
        assert_eq!(parse_plugin_ref("a@b@c"), Some(("a", "b@c")));
    }

    #[test]
    fn parse_plugin_ref_empty_parts() {
        assert_eq!(parse_plugin_ref("@marketplace"), Some(("", "marketplace")));
        assert_eq!(parse_plugin_ref("plugin@"), Some(("plugin", "")));
    }
}

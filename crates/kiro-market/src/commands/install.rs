//! `install` command: install a plugin or specific skill into a Kiro project.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use colored::Colorize;
use kiro_market_core::cache::CacheDir;
use kiro_market_core::git::{GitProtocol, GixCliBackend};
use kiro_market_core::project::KiroProject;
use kiro_market_core::service::{
    InstallAgentsResult, InstallFilter, InstallMode, InstallSkillsResult, MarketplaceService,
};
use kiro_market_core::steering::InstallSteeringResult;
use tracing::{debug, warn};

use crate::cli;

/// Run the install command.
///
/// Resolves `plugin_ref` to a plugin, discovers skills, copies skill
/// directories, and installs into the current Kiro project. `accept_mcp`
/// gates installation of agents that bring MCP servers — without the
/// opt-in flag, those agents are skipped with a warning so the user can
/// see the risk surface (subprocess execution / external network calls)
/// before re-running with `--accept-mcp`.
pub fn run(
    plugin_ref: &str,
    skill_filter: Option<&str>,
    force: bool,
    accept_mcp: bool,
) -> Result<()> {
    let mode = InstallMode::from(force);
    let (plugin_name, marketplace_name) = cli::parse_plugin_ref(plugin_ref).with_context(|| {
        format!("invalid plugin reference '{plugin_ref}': expected plugin@marketplace")
    })?;

    let cache = CacheDir::default_location()
        .context("could not determine data directory; is $HOME set?")?;
    let marketplace_path = cache.marketplace_path(marketplace_name);
    if !marketplace_path.exists() {
        bail!(
            "marketplace '{}' not found. Run {} first.",
            marketplace_name,
            "kiro-market marketplace add".bold()
        );
    }

    let protocol = load_protocol(&cache, marketplace_name);
    let plugin_entry =
        super::common::find_plugin_entry(&marketplace_path, plugin_name, marketplace_name)?;

    // One service instance drives plugin-dir resolution plus the install
    // loops; no second `GixCliBackend` needed.
    let svc = MarketplaceService::new(cache.clone(), GixCliBackend::default());
    let plugin_dir = fetch_plugin_dir(
        &svc,
        &plugin_entry,
        &marketplace_path,
        marketplace_name,
        plugin_name,
        protocol,
    )?;

    let ctx = MarketplaceService::resolve_plugin_install_context_from_dir(&plugin_dir)
        .with_context(|| format!("failed to resolve install context for '{plugin_name}'"))?;

    let cwd = std::env::current_dir().context("failed to determine current directory")?;
    let project = KiroProject::new(cwd);

    let skill_result = run_skill_install(
        &svc,
        &project,
        &ctx.skill_dirs,
        skill_filter,
        mode,
        marketplace_name,
        plugin_name,
        ctx.version.as_deref(),
    );
    print_install_outcome(plugin_ref, &skill_result);

    let install_ctx = kiro_market_core::service::AgentInstallContext {
        mode,
        accept_mcp,
        marketplace: marketplace_name,
        plugin: plugin_name,
        version: ctx.version.as_deref(),
    };
    let agent_result = run_agent_install(
        &svc,
        &project,
        &plugin_dir,
        &ctx.agent_scan_paths,
        skill_filter,
        ctx.format,
        install_ctx,
    );
    print_agent_outcome(&agent_result);

    let steering_ctx = kiro_market_core::steering::SteeringInstallContext {
        mode,
        marketplace: marketplace_name,
        plugin: plugin_name,
        version: ctx.version.as_deref(),
    };
    let steering_result = run_steering_install(
        &svc,
        &project,
        &plugin_dir,
        &ctx.steering_scan_paths,
        skill_filter,
        steering_ctx,
    );
    print_steering_outcome(&steering_result, &project);

    summarize_outcome(
        plugin_ref,
        skill_filter,
        &skill_result,
        &agent_result,
        &steering_result,
    )
}

/// Look up the stored git protocol preference for a marketplace, falling back
/// to the default if the registry is unreadable.
fn load_protocol(cache: &CacheDir, marketplace_name: &str) -> GitProtocol {
    match cache.load_known_marketplaces() {
        Ok(entries) => entries
            .into_iter()
            .find(|e| e.name == marketplace_name)
            .and_then(|e| e.protocol)
            .unwrap_or_default(),
        Err(e) => {
            warn!(
                marketplace = marketplace_name,
                error = %e,
                "failed to load marketplace registry; defaulting to HTTPS protocol"
            );
            GitProtocol::default()
        }
    }
}

/// Resolve the plugin's on-disk directory, printing a progress message so the
/// user sees something during the clone (which can block on network I/O).
fn fetch_plugin_dir(
    svc: &MarketplaceService,
    entry: &kiro_market_core::marketplace::PluginEntry,
    marketplace_path: &Path,
    marketplace_name: &str,
    plugin_name: &str,
    protocol: GitProtocol,
) -> Result<PathBuf> {
    print!("  Fetching plugin '{plugin_name}'...");
    if let Err(e) = std::io::Write::flush(&mut std::io::stdout()) {
        warn!(error = %e, "failed to flush stdout before fetch progress message");
    }
    let dir = svc
        .resolve_plugin_dir(entry, marketplace_path, marketplace_name, protocol)
        .with_context(|| format!("failed to resolve plugin directory for '{plugin_name}'"))?;
    println!(" done");
    debug!(plugin_dir = %dir.display(), "resolved plugin directory");
    Ok(dir)
}

#[allow(clippy::too_many_arguments)] // each arg is an independent piece of upstream context
fn run_skill_install(
    svc: &MarketplaceService,
    project: &KiroProject,
    skill_dirs: &[PathBuf],
    skill_filter: Option<&str>,
    mode: InstallMode,
    marketplace_name: &str,
    plugin_name: &str,
    version: Option<&str>,
) -> InstallSkillsResult {
    if skill_dirs.is_empty() {
        return InstallSkillsResult::default();
    }
    let filter = match skill_filter {
        Some(name) => InstallFilter::SingleName(name),
        None => InstallFilter::All,
    };
    svc.install_skills(
        project,
        skill_dirs,
        &filter,
        mode,
        marketplace_name,
        plugin_name,
        version,
    )
}

fn run_agent_install(
    svc: &MarketplaceService,
    project: &KiroProject,
    plugin_dir: &Path,
    agent_scan_paths: &[String],
    skill_filter: Option<&str>,
    format: Option<kiro_market_core::plugin::PluginFormat>,
    ctx: kiro_market_core::service::AgentInstallContext<'_>,
) -> InstallAgentsResult {
    // A `--skill <name>` filter narrows the install to one skill and never
    // includes agents.
    if skill_filter.is_some() {
        return InstallAgentsResult::default();
    }
    svc.install_plugin_agents(project, plugin_dir, agent_scan_paths, format, ctx)
}

fn run_steering_install(
    svc: &MarketplaceService,
    project: &KiroProject,
    plugin_dir: &Path,
    steering_scan_paths: &[String],
    skill_filter: Option<&str>,
    ctx: kiro_market_core::steering::SteeringInstallContext<'_>,
) -> InstallSteeringResult {
    // A `--skill <name>` filter narrows the install to one skill and never
    // touches steering files.
    if skill_filter.is_some() {
        return InstallSteeringResult::default();
    }
    svc.install_plugin_steering(project, plugin_dir, steering_scan_paths, ctx)
}

/// Decide whether the command exits zero or non-zero based on the
/// accumulated skill + agent + steering results. Any per-item failure
/// becomes a non-zero exit so CI catches partial-success regressions.
fn summarize_outcome(
    plugin_ref: &str,
    skill_filter: Option<&str>,
    skill_result: &InstallSkillsResult,
    agent_result: &InstallAgentsResult,
    steering_result: &InstallSteeringResult,
) -> Result<()> {
    let nothing_installed = skill_result.installed.is_empty()
        && skill_result.skipped.is_empty()
        && agent_result.installed.is_empty()
        && agent_result.skipped.is_empty()
        && steering_result.installed.is_empty();
    if nothing_installed
        && agent_result.failed.is_empty()
        && skill_result.failed.is_empty()
        && steering_result.failed.is_empty()
    {
        let kind = if skill_filter.is_some() {
            "skills"
        } else {
            "skills, agents, or steering files"
        };
        bail!("no {kind} were installed from '{plugin_ref}'");
    }
    if !agent_result.failed.is_empty()
        || !skill_result.failed.is_empty()
        || !steering_result.failed.is_empty()
    {
        let fail_count =
            agent_result.failed.len() + skill_result.failed.len() + steering_result.failed.len();
        bail!(
            "{fail_count} item{s} failed during install from '{plugin_ref}'",
            s = if fail_count == 1 { "" } else { "s" }
        );
    }
    Ok(())
}

/// Render the agent install summary plus any warnings and per-agent
/// failures. Warnings and failures go to stderr so they don't pollute
/// stdout piping, matching the skill flow.
fn print_agent_outcome(result: &InstallAgentsResult) {
    // Build a lookup from agent name to its rich native outcome so the
    // legacy `installed: Vec<String>` rendering can append a `(forced)`
    // suffix where the native install path overwrote a tracked path.
    // Translated installs leave installed_native empty, so the lookup is
    // a no-op for that path.
    let native_by_name: std::collections::HashMap<&str, &_> = result
        .installed_native
        .iter()
        .map(|o| (o.name.as_str(), o))
        .collect();

    for name in &result.installed {
        let suffix = native_by_name
            .get(name.as_str())
            .filter(|o| o.kind == kiro_market_core::project::InstallOutcomeKind::ForceOverwrote)
            .map_or("", |_| " (forced)");
        println!(
            "  {} Installed agent '{}'{}",
            "✓".green().bold(),
            name.bold(),
            suffix.yellow()
        );
    }
    for name in &result.skipped {
        // Native idempotent reinstalls land here with a typed outcome
        // already in installed_native — render "(unchanged)" so the
        // user can tell the difference from a translated already-installed
        // skip.
        let suffix = native_by_name
            .get(name.as_str())
            .filter(|o| o.kind == kiro_market_core::project::InstallOutcomeKind::Idempotent)
            .map_or("", |_| " (unchanged)");
        println!(
            "  {} Agent '{}' already installed{}",
            "·".yellow().bold(),
            name.bold(),
            suffix.dimmed()
        );
    }
    if let Some(companions) = &result.installed_companions {
        // Wildcard arm required by `InstallOutcomeKind`'s `#[non_exhaustive]`
        // (cross-crate matches must accept future variants). New variants
        // land with neutral rendering until the CLI explicitly handles them.
        let suffix = match companions.kind {
            kiro_market_core::project::InstallOutcomeKind::Idempotent => " (unchanged)".dimmed(),
            kiro_market_core::project::InstallOutcomeKind::ForceOverwrote => " (forced)".yellow(),
            _ => "".normal(),
        };
        let plural = if companions.files.len() == 1 { "" } else { "s" };
        println!(
            "  {} Installed {} companion file{plural} for '{}'{}",
            "✓".green().bold(),
            companions.files.len(),
            companions.plugin.bold(),
            suffix
        );
    }
    for failed in &result.failed {
        let label = failed
            .name
            .as_deref()
            .map_or_else(|| failed.source_path.display().to_string(), str::to_owned);
        let rendered = kiro_market_core::error::error_full_chain(&failed.error);
        eprintln!(
            "  {} Failed to install agent '{}': {rendered}",
            "✗".red().bold(),
            label
        );
    }
    for w in &result.warnings {
        eprintln!("  {} {w}", "!".yellow().bold());
    }
}

/// Render the steering install summary plus per-file failures and
/// warnings. Failures use [`error_full_chain`](kiro_market_core::error::error_full_chain)
/// per S3-13 so the underlying `io::Error` reason (e.g. which file
/// couldn't be read, what OS error fired) reaches the user — a bare
/// `to_string()` would drop the `#[source]` chain and produce
/// "tracking I/O failed" with no actionable detail.
fn print_steering_outcome(result: &InstallSteeringResult, project: &KiroProject) {
    let steering_root = project.steering_dir();
    for outcome in &result.installed {
        // Match the workspace-shared `InstallOutcomeKind` (3 variants
        // today). Wildcard arm required by `#[non_exhaustive]` —
        // future variants render neutrally until explicitly handled.
        let suffix = match outcome.kind {
            kiro_market_core::project::InstallOutcomeKind::Idempotent => " (unchanged)".dimmed(),
            kiro_market_core::project::InstallOutcomeKind::ForceOverwrote => " (forced)".yellow(),
            _ => "".normal(),
        };
        let rel = outcome
            .destination
            .strip_prefix(&steering_root)
            .unwrap_or(&outcome.destination);
        println!(
            "  {} Installed steering '{}'{}",
            "✓".green().bold(),
            rel.display(),
            suffix
        );
    }
    for failed in &result.failed {
        let rendered = kiro_market_core::error::error_full_chain(&failed.error);
        eprintln!(
            "  {} Failed to install steering '{}': {rendered}",
            "✗".red().bold(),
            failed.source.display()
        );
    }
    for w in &result.warnings {
        eprintln!("  {} {w}", "!".yellow().bold());
    }
}

/// Render a per-skill summary plus the rolled-up totals from a service-layer
/// install result. The previous per-line `eprintln!`s during the loop are
/// gone since the service now emits structured `warn!` events; if the user
/// wants more detail they can set `RUST_LOG`.
fn print_install_outcome(plugin_ref: &str, result: &InstallSkillsResult) {
    for name in &result.installed {
        println!("  {} Installed skill '{}'", "✓".green().bold(), name.bold());
    }
    for name in &result.skipped {
        println!(
            "  {} Skill '{}' already installed (use --force to overwrite)",
            "·".yellow().bold(),
            name.bold()
        );
    }
    for failure in &result.failed {
        // `FailedSkill`'s fields are `pub(crate)` — go through the
        // accessors so the `error`/`kind` invariant (populated together
        // via `FailedSkill::install_failed` or ::requested_but_not_found)
        // stays enforced at the type boundary.
        eprintln!(
            "  {} Failed to install skill '{}': {}",
            "✗".red().bold(),
            failure.name(),
            failure.error()
        );
    }
    // Per-skill read/parse failures used to vanish into `warn!` logs;
    // the service now surfaces them as structured SkippedSkill entries.
    // Render them so users see *why* the install count is smaller than
    // the skill directory count.
    for sk in &result.skipped_skills {
        // SkippedSkillReason is #[non_exhaustive]; a wildcard arm is
        // required. Future variants render via their Debug form until
        // an explicit arm lands — better than a bare "unreadable"
        // label because it at least surfaces the variant payload in
        // terminal output. Safer than a compile error in the downstream
        // binary every time core gains a variant.
        let reason = match &sk.reason {
            kiro_market_core::service::SkippedSkillReason::ReadFailed { reason } => {
                format!("could not read SKILL.md: {reason}")
            }
            kiro_market_core::service::SkippedSkillReason::FrontmatterInvalid { reason } => {
                format!("malformed frontmatter: {reason}")
            }
            other => format!("unreadable: {other:?}"),
        };
        // `name_hint` is None when the skill directory's file_name()
        // can't be extracted (degenerate path — empty, root, or `..`-
        // terminated). Fall back to a `<unnamed>` placeholder; the
        // skill's file path is still printed on the same line below,
        // so the user retains a locator even when the label is empty.
        let label = sk.name_hint.as_deref().unwrap_or("<unnamed>");
        eprintln!(
            "  {} Skipped unreadable skill '{label}' ({}): {reason}",
            "⚠".yellow().bold(),
            sk.path.display()
        );
    }

    println!();
    if !result.installed.is_empty() {
        println!(
            "{} Installed {} skill{} from {}",
            "✓".green().bold(),
            result.installed.len(),
            if result.installed.len() == 1 { "" } else { "s" },
            plugin_ref.bold()
        );
    }
    if !result.skipped.is_empty() {
        println!(
            "{} Skipped {} already-installed skill{}",
            "·".yellow().bold(),
            result.skipped.len(),
            if result.skipped.len() == 1 { "" } else { "s" }
        );
    }
    if !result.failed.is_empty() {
        println!(
            "{} {} skill{} failed",
            "✗".red().bold(),
            result.failed.len(),
            if result.failed.len() == 1 { "" } else { "s" }
        );
    }
    if !result.skipped_skills.is_empty() {
        println!(
            "{} {} skill{} could not be read",
            "⚠".yellow().bold(),
            result.skipped_skills.len(),
            if result.skipped_skills.len() == 1 {
                ""
            } else {
                "s"
            }
        );
    }
}

//! `install` command: install a plugin or specific skill into a Kiro project.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use colored::Colorize;
use kiro_market_core::cache::CacheDir;
use kiro_market_core::git::{GitProtocol, GixCliBackend};
use kiro_market_core::plugin::{PluginManifest, discover_skill_dirs};
use kiro_market_core::project::KiroProject;
use kiro_market_core::service::{
    FailedAgent, InstallAgentsResult, InstallFilter, InstallMode, InstallSkillsResult,
    MarketplaceService,
};
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

    let plugin_manifest = load_plugin_manifest(&plugin_dir)?;
    let skill_dirs = discover_plugin_skills(&plugin_dir, plugin_manifest.as_ref());
    let agent_scan_paths = agent_scan_paths(plugin_manifest.as_ref());

    let cwd = std::env::current_dir().context("failed to determine current directory")?;
    let project = KiroProject::new(cwd);
    let version = plugin_manifest.as_ref().and_then(|m| m.version.clone());

    let skill_result = run_skill_install(
        &svc,
        &project,
        &skill_dirs,
        skill_filter,
        mode,
        marketplace_name,
        plugin_name,
        version.as_deref(),
    );
    print_install_outcome(plugin_ref, &skill_result);

    let agent_result = run_agent_install(
        &svc,
        &project,
        &plugin_dir,
        &agent_scan_paths,
        skill_filter,
        mode,
        accept_mcp,
        marketplace_name,
        plugin_name,
        version.as_deref(),
    );
    print_agent_outcome(&agent_result);

    summarize_outcome(plugin_ref, skill_filter, &skill_result, &agent_result)
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
    let _ = std::io::Write::flush(&mut std::io::stdout());
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

#[allow(clippy::too_many_arguments)] // each arg is an independent piece of upstream context
fn run_agent_install(
    svc: &MarketplaceService,
    project: &KiroProject,
    plugin_dir: &Path,
    agent_scan_paths: &[String],
    skill_filter: Option<&str>,
    mode: InstallMode,
    accept_mcp: bool,
    marketplace_name: &str,
    plugin_name: &str,
    version: Option<&str>,
) -> InstallAgentsResult {
    // A `--skill <name>` filter narrows the install to one skill and never
    // includes agents.
    if skill_filter.is_some() {
        return InstallAgentsResult::default();
    }
    svc.install_plugin_agents(
        project,
        plugin_dir,
        agent_scan_paths,
        mode,
        accept_mcp,
        marketplace_name,
        plugin_name,
        version,
    )
}

/// Decide whether the command exits zero or non-zero based on the accumulated
/// skill + agent results. Any per-item failure becomes a non-zero exit so CI
/// catches partial-success regressions.
fn summarize_outcome(
    plugin_ref: &str,
    skill_filter: Option<&str>,
    skill_result: &InstallSkillsResult,
    agent_result: &InstallAgentsResult,
) -> Result<()> {
    let nothing_installed = skill_result.installed.is_empty()
        && skill_result.skipped.is_empty()
        && agent_result.installed.is_empty()
        && agent_result.skipped.is_empty();
    if nothing_installed && agent_result.failed.is_empty() && skill_result.failed.is_empty() {
        let kind = if skill_filter.is_some() {
            "skills"
        } else {
            "skills or agents"
        };
        bail!("no {kind} were installed from '{plugin_ref}'");
    }
    if !agent_result.failed.is_empty() || !skill_result.failed.is_empty() {
        let fail_count = agent_result.failed.len() + skill_result.failed.len();
        bail!(
            "{fail_count} item{s} failed during install from '{plugin_ref}'",
            s = if fail_count == 1 { "" } else { "s" }
        );
    }
    Ok(())
}

/// Resolve the list of agent scan paths for a plugin.
fn agent_scan_paths(plugin_manifest: Option<&PluginManifest>) -> Vec<String> {
    if let Some(m) = plugin_manifest.filter(|m| !m.agents.is_empty()) {
        m.agents.clone()
    } else {
        kiro_market_core::DEFAULT_AGENT_PATHS
            .iter()
            .map(|s| (*s).to_string())
            .collect()
    }
}

/// Render the agent install summary plus any warnings and per-agent
/// failures. Warnings and failures go to stderr so they don't pollute
/// stdout piping, matching the skill flow.
fn print_agent_outcome(result: &InstallAgentsResult) {
    for name in &result.installed {
        println!("  {} Installed agent '{}'", "✓".green().bold(), name.bold());
    }
    for name in &result.skipped {
        println!(
            "  {} Agent '{}' already installed",
            "·".yellow().bold(),
            name.bold()
        );
    }
    for FailedAgent { name, error } in &result.failed {
        eprintln!(
            "  {} Failed to install agent '{}': {error}",
            "✗".red().bold(),
            name
        );
    }
    for w in &result.warnings {
        eprintln!("  {} {w}", "!".yellow().bold());
    }
}

/// Discover skill directories from a plugin, using its manifest or defaults.
fn discover_plugin_skills(
    plugin_dir: &Path,
    plugin_manifest: Option<&PluginManifest>,
) -> Vec<PathBuf> {
    let skill_paths: Vec<&str> =
        if let Some(manifest) = plugin_manifest.filter(|m| !m.skills.is_empty()) {
            manifest.skills.iter().map(String::as_str).collect()
        } else {
            kiro_market_core::DEFAULT_SKILL_PATHS.to_vec()
        };

    discover_skill_dirs(plugin_dir, &skill_paths)
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

/// Load a `plugin.json` from the given directory.
///
/// Returns:
/// - `Ok(Some(manifest))` on success.
/// - `Ok(None)` when the file is genuinely absent (`NotFound`) or when it is
///   a symlink — a symlinked `plugin.json` inside a cloned repo could point
///   at arbitrary host files, so it is treated as absent with a `warn!`.
/// - `Err` for every other condition: permission denied, EIO, interrupted,
///   malformed JSON, etc. Matches the allowlist-style error handling in
///   `find_plugin_entry` — never mask a broken cache as "missing manifest."
fn load_plugin_manifest(plugin_dir: &Path) -> Result<Option<PluginManifest>> {
    let manifest_path = plugin_dir.join("plugin.json");

    // Refuse to follow symlinks. plugin_dir lives inside a cloned (untrusted)
    // repository; matches project::copy_dir_recursive and
    // agent::discover_agents_in_dirs.
    match fs::symlink_metadata(&manifest_path) {
        Ok(m) if m.file_type().is_symlink() => {
            warn!(
                path = %manifest_path.display(),
                "plugin.json is a symlink, refusing to follow; treating as missing"
            );
            return Ok(None);
        }
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            debug!(
                path = %manifest_path.display(),
                "plugin.json not found, using defaults"
            );
            return Ok(None);
        }
        Err(e) => {
            return Err(anyhow::Error::new(e).context(format!(
                "failed to stat plugin.json at {}",
                manifest_path.display()
            )));
        }
    }

    let bytes = fs::read(&manifest_path)
        .with_context(|| format!("failed to read plugin.json at {}", manifest_path.display()))?;
    let manifest = PluginManifest::from_json(&bytes)
        .with_context(|| format!("plugin.json at {} is malformed", manifest_path.display()))?;
    debug!(name = %manifest.name, "loaded plugin manifest");
    Ok(Some(manifest))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_plugin_manifest_reads_regular_file() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(
            tmp.path().join("plugin.json"),
            r#"{"name":"ok","version":"1.0.0"}"#,
        )
        .unwrap();
        let m = load_plugin_manifest(tmp.path())
            .expect("ok result")
            .expect("some manifest");
        assert_eq!(m.name, "ok");
    }

    #[test]
    fn load_plugin_manifest_returns_ok_none_when_absent() {
        // Genuine absence is expected — NotFound is part of the contract,
        // not an error. Regression guard for the allowlist split.
        let tmp = tempfile::tempdir().unwrap();
        let result = load_plugin_manifest(tmp.path()).expect("NotFound must be Ok(None)");
        assert!(result.is_none());
    }

    #[test]
    fn load_plugin_manifest_errors_on_malformed_json() {
        // Regression: previously malformed plugin.json silently fell back
        // to defaults. Allowlist-style handling requires it to surface.
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("plugin.json"), b"{ not json").unwrap();
        let err = load_plugin_manifest(tmp.path()).expect_err("malformed must error");
        let msg = format!("{err:#}");
        assert!(
            msg.contains("malformed") || msg.contains("plugin.json"),
            "error chain should mention the manifest: {msg}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn load_plugin_manifest_refuses_symlinked_manifest() {
        // A malicious cloned repo could include a symlink
        // `plugin.json -> /etc/passwd`. We must not follow it. Symlink is
        // treated as absent (Ok(None)) with a warn!, not as an error —
        // the install degrades to "no skills" rather than aborting.
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("elsewhere.json");
        fs::write(&target, r#"{"name":"smuggled"}"#).unwrap();
        std::os::unix::fs::symlink(&target, tmp.path().join("plugin.json")).unwrap();

        let result = load_plugin_manifest(tmp.path()).expect("symlink should not error");
        assert!(
            result.is_none(),
            "symlinked plugin.json must be treated as absent, got: {result:?}"
        );
    }
}

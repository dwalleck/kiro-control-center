import type {
  InstallPluginResult_Serialize,
  InstallWarning,
  ParseFailure,
  SkillCount,
  SkippedReason,
  SkippedSkill,
  SteeringWarning,
} from "$lib/bindings";

// Render a structured SkippedReason as a one-line string. Total over
// all eight variants (not just the six reachable via SkillCount) —
// TypeScript's exhaustiveness check forces full coverage. The two
// currently-unreachable variants (remote_source_not_local, no_skills)
// are reserved for future reuse with SkippedPlugin banners.
export function formatSkippedReason(r: SkippedReason): string {
  switch (r.kind) {
    case "directory_missing":
      return `plugin directory not found: ${r.path}`;
    case "not_a_directory":
      return `plugin path is not a directory: ${r.path}`;
    case "symlink_refused":
      return `plugin path is a symlink (refused): ${r.path}`;
    case "directory_unreadable":
      return `could not read ${r.path}: ${r.reason}`;
    case "invalid_manifest":
      return `malformed plugin.json at ${r.path}: ${r.reason}`;
    case "manifest_read_failed":
      return `could not read plugin.json at ${r.path}: ${r.reason}`;
    case "remote_source_not_local":
      return `plugin source is remote: ${r.plugin}`;
    case "no_skills":
      return `plugin declares no skills: ${r.path}`;
  }
}

export function skillCountLabel(sc: SkillCount): string {
  switch (sc.state) {
    case "known": return String(sc.count);
    case "remote_not_counted": return "–";
    case "manifest_failed": return "!";
  }
}

export function skillCountTitle(sc: SkillCount): string | undefined {
  switch (sc.state) {
    case "known":
      return undefined;
    case "remote_not_counted":
      return "Remote plugin — skills cannot be counted without cloning";
    case "manifest_failed":
      return formatSkippedReason(sc.reason);
  }
}

// Render a structured SkippedSkill as a one-line label for warning
// banners. Uses name_hint (Option<String> in core → string | null on
// the wire) with "<unnamed>" fallback so a skill whose directory name
// could not be extracted still shows up in the UI rather than being
// silently dropped — that silent drop is exactly the class of bug the
// SkippedSkill surfacing pattern is fighting across all three call
// sites that consume SkippedSkill.
export function formatSkippedSkill(s: SkippedSkill): string {
  const label = s.name_hint ?? "<unnamed>";
  // SkippedSkillReason is a discriminated union on `kind`. The default
  // arm uses an assertNever binding so a new variant lands as a
  // compile-time error here rather than a silent "unreadable"
  // collapse — `#[non_exhaustive]` on the Rust side is for runtime
  // forward-compat, but the UI side benefits from the type system
  // forcing a renderer update when bindings.ts grows.
  let reason: string;
  switch (s.reason.kind) {
    case "read_failed":
      reason = `could not read SKILL.md: ${s.reason.reason}`;
      break;
    case "frontmatter_invalid":
      reason = `malformed frontmatter: ${s.reason.reason}`;
      break;
    default: {
      const _exhaustive: never = s.reason;
      throw new Error(
        `unhandled SkippedSkillReason variant: ${JSON.stringify(_exhaustive)}`,
      );
    }
  }
  return `${label}: ${reason}`;
}

// Render a SteeringWarning as a one-line label. Lifted from BrowseTab
// once a second consumer (installWholePlugin) appeared — keeping it
// inline would have meant two drift-prone copies of the assertNever
// guard.
export function formatSteeringWarning(w: SteeringWarning): string {
  switch (w.kind) {
    case "scan_path_invalid":
      return `invalid scan path '${w.path}': ${w.reason}`;
    case "scan_dir_unreadable":
      return `could not read steering dir '${w.path}': ${w.reason}`;
    default: {
      const _exhaustive: never = w;
      throw new Error(
        `unhandled SteeringWarning variant: ${JSON.stringify(_exhaustive)}`,
      );
    }
  }
}

// Render an `InstallWarning` (from agent installs) as a one-line
// label. The `mcp_servers_require_opt_in` variant is the
// security-sensitive one — an agent declaring MCP servers was
// refused because `accept_mcp` is false. Surfacing the listed
// transports lets the user understand the risk surface before
// re-running with the opt-in.
export function formatInstallWarning(w: InstallWarning): string {
  switch (w.kind) {
    case "unmapped_tool":
      return `agent '${w.agent}' dropped unmapped tool '${w.tool}' (${w.reason})`;
    case "agent_parse_failed":
      return `agent file '${w.path}' could not be parsed: ${formatParseFailure(w.failure)}`;
    case "mcp_servers_require_opt_in": {
      const transports = w.transports.length > 0
        ? ` [${w.transports.join(", ")}]`
        : "";
      return `agent '${w.agent}' declares MCP servers${transports} — re-run with --accept-mcp to install`;
    }
    default: {
      const _exhaustive: never = w;
      throw new Error(
        `unhandled InstallWarning variant: ${JSON.stringify(_exhaustive)}`,
      );
    }
  }
}

// Helper for `agent_parse_failed`. Kept module-private — the only
// consumer today is `formatInstallWarning`, but the shape is the same
// assertNever-guarded discriminated-union switch the other formatters
// use, so a future caller can lift it without rework.
function formatParseFailure(f: ParseFailure): string {
  switch (f.kind) {
    case "missing_frontmatter":
      return "missing frontmatter fence";
    case "unclosed_frontmatter":
      return "unclosed frontmatter fence";
    case "invalid_yaml":
      return `invalid YAML: ${f.reason}`;
    case "missing_name":
      return "missing 'name' in frontmatter";
    case "invalid_name":
      return `invalid name: ${f.reason}`;
    case "io_error":
      return `I/O error: ${f.reason}`;
    case "unsupported_dialect":
      return "unsupported dialect for this code path";
    default: {
      const _exhaustive: never = f;
      throw new Error(
        `unhandled ParseFailure variant: ${JSON.stringify(_exhaustive)}`,
      );
    }
  }
}

export function formatSkippedSkillsForPlugin(list: readonly SkippedSkill[]): string {
  // Caller already filtered to entries for one plugin; compose a
  // compact single-line banner body. Truncate at MAX entries and
  // surface the remainder as a "+N more" count — a plugin with
  // dozens of malformed SKILL.md files is a real failure mode, but
  // the banner isn't the right place to dump the whole list.
  const MAX = 5;
  const parts = list.slice(0, MAX).map(formatSkippedSkill);
  const overflow = list.length - parts.length;
  const joined = parts.join("; ");
  return overflow > 0
    ? `${list.length} skill(s) failed to load — ${joined}; +${overflow} more`
    : `${list.length} skill(s) failed to load — ${joined}`;
}

/**
 *  Summarized view of an `InstallPluginResult_Serialize` for banner
 *  rendering. Extracted from `BrowseTab.installWholePlugin` so the new
 *  Update flow (which also calls `installPlugin`, just with `force=true`)
 *  reuses the same summarization rather than duplicating it.
 *
 *  - `summary`: human-readable mid-dot-separated count phrase
 *    (e.g. "2 skills · 1 steering · 1 agent"). Reads "nothing to install"
 *    when nothing happened.
 *  - `warnings`: pipe-separated warning lines (steering-scan warnings,
 *    MCP-gated agents, per-skill skipped_skills) or `null` when empty.
 *  - `anyInstalled` / `anyFailed`: caller uses these to decide which
 *    banner channel (success vs. error vs. warning) to route to.
 */
export type FormattedInstallPluginResult = {
  summary: string;
  warnings: string | null;
  anyInstalled: boolean;
  anyFailed: boolean;
};

export function formatInstallPluginResult(
  r: InstallPluginResult_Serialize,
  _plugin: string,
): FormattedInstallPluginResult {
  const summaryParts: string[] = [];
  const warningParts: string[] = [];

  // Skills sub-result.
  {
    const skills = r.skills;
    if (skills.installed.length > 0) {
      const noun = skills.installed.length === 1 ? "skill" : "skills";
      summaryParts.push(`${skills.installed.length} ${noun}`);
    }
    if (skills.failed.length > 0) {
      const noun = skills.failed.length === 1 ? "skill" : "skills";
      summaryParts.push(`${skills.failed.length} ${noun} failed`);
    }
    if (skills.skipped.length > 0) {
      const noun = skills.skipped.length === 1 ? "skill" : "skills";
      summaryParts.push(`${skills.skipped.length} ${noun} already installed`);
    }
    if (skills.skipped_skills.length > 0) {
      warningParts.push(formatSkippedSkillsForPlugin(skills.skipped_skills));
    }
  }

  // Steering sub-result. Idempotent reinstalls land in `installed` with
  // `kind: idempotent` (not a separate field) — the current banner shape
  // counts them as installed; per-content breakdown is future scope.
  {
    const steering = r.steering;
    if (steering.installed.length > 0) {
      const noun = steering.installed.length === 1 ? "file" : "files";
      summaryParts.push(`${steering.installed.length} steering ${noun}`);
    }
    if (steering.failed.length > 0) {
      summaryParts.push(`${steering.failed.length} steering failed`);
    }
    for (const w of steering.warnings) {
      warningParts.push(formatSteeringWarning(w));
    }
  }

  // Agents sub-result.
  {
    const agents = r.agents;
    if (agents.installed.length > 0) {
      const noun = agents.installed.length === 1 ? "agent" : "agents";
      summaryParts.push(`${agents.installed.length} ${noun}`);
    }
    if (agents.failed.length > 0) {
      const noun = agents.failed.length === 1 ? "agent" : "agents";
      summaryParts.push(`${agents.failed.length} ${noun} failed`);
    }
    if (agents.skipped.length > 0) {
      const noun = agents.skipped.length === 1 ? "agent" : "agents";
      summaryParts.push(`${agents.skipped.length} ${noun} already installed`);
    }
    for (const w of agents.warnings) {
      warningParts.push(formatInstallWarning(w));
    }
  }

  const anyFailed =
    r.skills.failed.length + r.steering.failed.length + r.agents.failed.length > 0;
  const anyInstalled =
    r.skills.installed.length +
      r.steering.installed.length +
      r.agents.installed.length >
    0;
  const summary = summaryParts.length > 0 ? summaryParts.join(" · ") : "nothing to install";
  const warnings = warningParts.length > 0 ? warningParts.join(" | ") : null;

  return { summary, warnings, anyInstalled, anyFailed };
}

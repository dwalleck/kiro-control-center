import type {
  FailedSkill,
  FailedSkillReason,
  FailedSteeringFile_Serialize,
  InstallPluginResult_Serialize,
  InstallWarning,
  ParseFailure,
  RemovePluginResult,
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

// Render a FailedSkill as a one-line label. Both FailedSkillReason variants
// share the same `${name} — ${error}` render shape — the `kind` discriminator
// lets the surrounding context (panel section heading) convey "install failed"
// vs. "not found", so the rendered string itself is uniform. The switch is
// still required: the default assertNever arm ensures a new FailedSkillReason
// variant (added on the Rust side and regenerated into bindings.ts) becomes
// a compile-time error here rather than a silent runtime fallthrough.
// Paired with value-position exhaustiveness asserts below per CLAUDE.md
// discriminator-pushdown discipline.
export function formatFailedSkill(f: FailedSkill): string {
  switch (f.kind.kind) {
    case "install_failed":
      return `${f.name} — ${f.error}`;
    case "requested_but_not_found":
      return `${f.name} — ${f.error}`;
    default: {
      const _exhaustive: never = f.kind;
      throw new Error(
        `unhandled FailedSkillReason variant: ${JSON.stringify(_exhaustive)}`,
      );
    }
  }
}

// Value-position exhaustiveness asserts for FailedSkillReason["kind"].
// The `satisfies` arm catches shape changes (a literal becomes an object arm).
// The `Exclude<>` arm catches additions (a new variant added to the type that
// isn't listed here). The trailing const assignment is what makes the tripwire
// fire — an unused type alias resolving to `never` is valid TS, so the const
// is the active gate. Precedent: _PLUGIN_ACTION_VALUES at stores/plugin-updates.ts:135-137.
const _FAILED_SKILL_REASON_KINDS = ["install_failed", "requested_but_not_found"] as const satisfies readonly FailedSkillReason["kind"][];
type _AssertFailedSkillReasonExhaustive = Exclude<FailedSkillReason["kind"], (typeof _FAILED_SKILL_REASON_KINDS)[number]> extends never ? true : never;
const _assertFailedSkillReasonExhaustive: _AssertFailedSkillReasonExhaustive = true;

// Render a FailedSteeringFile as a one-line label. Single-shape type today
// (source + error), so no switch is needed. If FailedSteeringFile grows
// discriminated variants in the future (per docs/plans/2026-05-09-failed-agent-discriminator-design.md),
// revisit with the same discriminator-pushdown pattern used by formatFailedAgent.
export function formatFailedSteeringFile(f: FailedSteeringFile_Serialize): string {
  return `${f.source} — ${f.error}`;
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

export type FormattedInstallPluginResult = {
  summary: string;
  warnings: string | null;
  anyInstalled: boolean;
  anyFailed: boolean;
};

export function formatInstallPluginResult(
  r: InstallPluginResult_Serialize,
): FormattedInstallPluginResult {
  const summaryParts: string[] = [];
  const warningParts: string[] = [];

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

export type FormattedRemovePluginResult = {
  summary: string;
  hasItems: boolean;
  hasFailures: boolean;
};

export function formatRemovePluginResult(
  r: RemovePluginResult,
): FormattedRemovePluginResult {
  const skillsRemoved = r.skills.removed ?? [];
  const skillsFailures = r.skills.failures ?? [];
  const steeringRemoved = r.steering.removed ?? [];
  const steeringFailures = r.steering.failures ?? [];
  const agentsRemoved = r.agents.removed ?? [];
  const agentsFailures = r.agents.failures ?? [];

  const summaryParts: string[] = [];

  if (skillsRemoved.length > 0) {
    const noun = skillsRemoved.length === 1 ? "skill" : "skills";
    summaryParts.push(`${skillsRemoved.length} ${noun}`);
  }
  if (steeringRemoved.length > 0) {
    const noun = steeringRemoved.length === 1 ? "file" : "files";
    summaryParts.push(`${steeringRemoved.length} steering ${noun}`);
  }
  if (agentsRemoved.length > 0) {
    const noun = agentsRemoved.length === 1 ? "agent" : "agents";
    summaryParts.push(`${agentsRemoved.length} ${noun}`);
  }
  if (skillsFailures.length > 0) {
    const noun = skillsFailures.length === 1 ? "skill" : "skills";
    summaryParts.push(`${skillsFailures.length} ${noun} failed`);
  }
  if (steeringFailures.length > 0) {
    summaryParts.push(`${steeringFailures.length} steering failed`);
  }
  if (agentsFailures.length > 0) {
    const noun = agentsFailures.length === 1 ? "agent" : "agents";
    summaryParts.push(`${agentsFailures.length} ${noun} failed`);
  }

  const hasItems =
    skillsRemoved.length + steeringRemoved.length + agentsRemoved.length > 0;
  const hasFailures =
    skillsFailures.length + steeringFailures.length + agentsFailures.length > 0;
  const summary = summaryParts.length > 0 ? summaryParts.join(" · ") : "nothing to remove";

  return { summary, hasItems, hasFailures };
}

import type {
  FailedAgent,
  FailedSkill,
  FailedSkillReason,
  FailedSteeringFile_Serialize,
  InstallPluginResult_Serialize,
  InstallWarning,
  ParseFailure,
  RemovePluginResult,
  SkillCount,
  SkippedItem,
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
    default: {
      const _exhaustive: never = r;
      throw new Error(
        `unhandled SkippedReason variant: ${JSON.stringify(_exhaustive)}`,
      );
    }
  }
}

export function skillCountLabel(sc: SkillCount): string {
  switch (sc.state) {
    case "known": return String(sc.count);
    case "remote_not_counted": return "–";
    case "manifest_failed": return "!";
    default: {
      const _exhaustive: never = sc;
      throw new Error(
        `unhandled SkillCount variant: ${JSON.stringify(_exhaustive)}`,
      );
    }
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
    default: {
      const _exhaustive: never = sc;
      throw new Error(
        `unhandled SkillCount variant: ${JSON.stringify(_exhaustive)}`,
      );
    }
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
    case "duplicate_name":
      // Two SKILL.md files in the plugin's scan paths declared the same
      // frontmatter `name`. The catalog kept the first; the second's
      // dir is surfaced here so the plugin author can see the conflict.
      reason = `duplicate skill name (kept ${s.reason.existing_dir}; dropped ${s.reason.conflict_dir})`;
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
    case "source_not_utf8":
      return `steering source '${w.path}' is not valid UTF-8; installed bytes verbatim`;
    case "unclosed_frontmatter":
      return `steering source '${w.path}' has an unclosed YAML frontmatter fence; installed bytes verbatim`;
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

// Both FailedSkillReason variants share the `${name} — ${error}` shape;
// the switch is retained so a new variant becomes a compile-time error
// at the assertNever arm rather than a silent fallthrough.
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

// Value-position exhaustiveness asserts; see _PLUGIN_ACTION_VALUES at stores/plugin-updates.ts:135-137.
const _FAILED_SKILL_REASON_KINDS = ["install_failed", "requested_but_not_found"] as const satisfies readonly FailedSkillReason["kind"][];
type _AssertFailedSkillReasonExhaustive = Exclude<FailedSkillReason["kind"], (typeof _FAILED_SKILL_REASON_KINDS)[number]> extends never ? true : never;
const _assertFailedSkillReasonExhaustive: _AssertFailedSkillReasonExhaustive = true;

// `entry.error` is opaque pre-rendered text from Rust's error_full_chain —
// render directly. The companion_bundle `|| "no enumeration"` fallback
// covers MultipleScanRootsNotSupported, where the engine bails before
// enumerating any conflict paths.
export function formatFailedAgent(entry: FailedAgent): string {
  switch (entry.kind) {
    case "agent":
      return `${entry.name} (${entry.source_path}) — ${entry.error}`;
    case "unparseable_agent":
      return `${entry.source_path} (unparseable) — ${entry.error}`;
    case "companion_bundle":
      return `${entry.plugin} bundle [${entry.conflicts.join(", ") || "no enumeration"}] — ${entry.error}`;
    case "requested_but_not_found":
      // No file was attempted — the request itself failed (typo or
      // stale catalog reference). Compose the user-facing string
      // from name + plugin since there's no underlying AgentError.
      return `agent '${entry.name}' not found in plugin '${entry.plugin}'`;
    default: {
      const _exhaustive: never = entry;
      throw new Error(
        `unhandled FailedAgent variant: ${JSON.stringify(_exhaustive)}`,
      );
    }
  }
}

// Value-position exhaustiveness asserts; see _PLUGIN_ACTION_VALUES at stores/plugin-updates.ts:135-137.
const _FAILED_AGENT_KINDS = ["agent", "unparseable_agent", "companion_bundle", "requested_but_not_found"] as const satisfies readonly FailedAgent["kind"][];
type _AssertFailedAgentKindExhaustive = Exclude<FailedAgent["kind"], (typeof _FAILED_AGENT_KINDS)[number]> extends never ? true : never;
const _assertFailedAgentKindExhaustive: _AssertFailedAgentKindExhaustive = true;

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
  const joined = list.slice(0, MAX).map(formatSkippedSkill).join("; ");
  const overflow = Math.max(0, list.length - MAX);
  const suffix = overflow > 0 ? `; +${overflow} more` : "";
  return `${list.length} skill(s) failed to load — ${joined}${suffix}`;
}

// Render a plugin's `entry.skipped_items` as a compact banner body —
// the catalog-side counterpart to `formatSkippedSkillsForPlugin`,
// covering all three categories (skills, steering, agents) the bulk
// catalog can surface. The switch on `item.kind` is exhaustive (with
// the standard `_exhaustive: never` guard) so a future SkippedItem
// variant lands as a compile error here rather than a silent miss
// in the banner output.
export function formatSkippedItemsForPlugin(items: readonly SkippedItem[]): string {
  // Bucket by category first so the per-category formatters
  // (formatSkippedSkill, formatSteeringWarning) get the same
  // single-shape input they were designed for. The reconstruction
  // overhead is trivial — each plugin's skipped_items is typically
  // 0–5 entries and pathologically ≤30.
  const skills: SkippedSkill[] = [];
  const steering: SteeringWarning[] = [];
  const agents: { source_path: string; reason: string }[] = [];
  for (const item of items) {
    switch (item.kind) {
      case "skill":
        skills.push(item.skill);
        break;
      case "steering_discovery":
        steering.push(item.warning);
        break;
      case "agent_parse":
        agents.push({ source_path: item.source_path, reason: item.reason });
        break;
      default: {
        const _exhaustive: never = item;
        throw new Error(
          `unhandled SkippedItem variant in formatSkippedItemsForPlugin: ${JSON.stringify(_exhaustive)}`,
        );
      }
    }
  }
  const parts: string[] = [];
  if (skills.length > 0) parts.push(formatSkippedSkillsForPlugin(skills));
  if (steering.length > 0) {
    const MAX = 3;
    const detail = steering.slice(0, MAX).map(formatSteeringWarning).join("; ");
    const overflow = Math.max(0, steering.length - MAX);
    const suffix = overflow > 0 ? `; +${overflow} more` : "";
    parts.push(`${steering.length} steering warning(s) — ${detail}${suffix}`);
  }
  if (agents.length > 0) {
    const MAX = 3;
    const detail = agents
      .slice(0, MAX)
      .map((a) => `${a.source_path}: ${a.reason}`)
      .join("; ");
    const overflow = Math.max(0, agents.length - MAX);
    const suffix = overflow > 0 ? `; +${overflow} more` : "";
    parts.push(`${agents.length} agent(s) failed to parse — ${detail}${suffix}`);
  }
  return parts.join(" | ");
}

// Value-position exhaustiveness assert mirroring _FAILED_AGENT_KINDS above.
// The `satisfies` catches arm-shape changes; `Exclude<...> extends never`
// catches arm additions; the value-position `const _assert: T = true` is
// what makes the guard active — an unused type alias resolving to `never`
// is valid TS, so without the value-assignment the tripwire is dead.
const _SKIPPED_ITEM_KINDS = ["skill", "steering_discovery", "agent_parse"] as const satisfies readonly SkippedItem["kind"][];
type _AssertSkippedItemKindExhaustive = Exclude<SkippedItem["kind"], (typeof _SKIPPED_ITEM_KINDS)[number]> extends never ? true : never;
const _assertSkippedItemKindExhaustive: _AssertSkippedItemKindExhaustive = true;

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

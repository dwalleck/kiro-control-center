import type {
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

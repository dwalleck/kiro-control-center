import type { SkillCount, SkippedReason, SkippedSkill } from "$lib/bindings";

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
  // SkippedSkillReason is a discriminated union on `kind`. A future
  // variant would land here as an unknown kind with a generic
  // "unreadable" label rather than a compile error — consistent with
  // the Rust #[non_exhaustive] attribute on the enum.
  let reason: string;
  switch (s.reason.kind) {
    case "read_failed":
      reason = `could not read SKILL.md: ${s.reason.reason}`;
      break;
    case "frontmatter_invalid":
      reason = `malformed frontmatter: ${s.reason.reason}`;
      break;
    default:
      reason = "unreadable";
  }
  return `${label}: ${reason}`;
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

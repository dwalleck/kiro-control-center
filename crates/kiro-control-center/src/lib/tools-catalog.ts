// Native-tool catalog for the Workflows > Agents editor's Tools section.
//
// Source-of-truth is the design bundle's `agents-data.js` window.AGENT_TOOLS
// (vendored at Kiro Control Center Design System/design_handoff_agents/
// source/agents-data.js). The probe at .agents-view/probe-s2/ locked the
// answer to 15 tools across 9 categories on 2026-05-26; tools-catalog.test.ts
// asserts byte-equality against probe.out so a future regression that drops
// a category during a re-port surfaces as a unit-test failure.
//
// Tools are listed in source order (NOT alphabetical) so a side-by-side
// visual diff against agents-data.js matches line-by-line during review.

/** The nine categories surfaced in the Tools-section by-category grid. */
export type ToolCategory =
  | "Cloud"
  | "Code"
  | "Filesystem"
  | "Meta"
  | "Orchestration"
  | "Planning"
  | "Reasoning"
  | "Shell"
  | "Web";

export type Tool = {
  readonly name: string;
  readonly category: ToolCategory;
  readonly summary: string;
};

export const TOOLS_CATALOG: readonly Tool[] = Object.freeze([
  { name: "fs_read",       category: "Filesystem",    summary: "Read files, directories, and images" },
  { name: "fs_write",      category: "Filesystem",    summary: "Create, edit, insert into files" },
  { name: "execute_bash",  category: "Shell",         summary: "Run shell commands" },
  { name: "code",          category: "Code",          summary: "AST-based symbol search and rewrite" },
  { name: "grep",          category: "Code",          summary: "Regex search across files" },
  { name: "glob",          category: "Code",          summary: "Find paths by glob pattern" },
  { name: "use_aws",       category: "Cloud",         summary: "Invoke AWS CLI operations" },
  { name: "use_subagent",  category: "Orchestration", summary: "Delegate to specialized subagents" },
  { name: "web_search",    category: "Web",           summary: "Search the web for current information" },
  { name: "web_fetch",     category: "Web",           summary: "Fetch and extract URL content" },
  { name: "introspect",    category: "Meta",          summary: "Inspect Kiro features and commands" },
  { name: "session",       category: "Meta",          summary: "Adjust temporary session settings" },
  { name: "todo_list",     category: "Planning",      summary: "Track multi-step task plans" },
  { name: "thinking",      category: "Reasoning",     summary: "Internal step-by-step reasoning" },
  { name: "report_issue",  category: "Meta",          summary: "File a GitHub issue with context" },
]) as readonly Tool[];

/**
 * Render the by-category grid in this fixed visual order. Anchored as a
 * separate constant (rather than derived from `TOOLS_CATALOG`) so the
 * grid's display order does not silently drift if `TOOLS_CATALOG` is
 * re-sorted alphabetically by name.
 */
export const CATEGORY_ORDER: readonly ToolCategory[] = Object.freeze([
  "Filesystem",
  "Code",
  "Shell",
  "Cloud",
  "Web",
  "Orchestration",
  "Planning",
  "Reasoning",
  "Meta",
]) as readonly ToolCategory[];

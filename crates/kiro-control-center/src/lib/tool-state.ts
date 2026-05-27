// Pure-logic reducers for the Workflows > Agents editor's Tools section.
//
// The editor's `Draft` type is intentionally open (`Record<string,
// unknown>`) so panels for unimplemented slices can round-trip fields
// they don't surface. This module narrows that to just the three
// fields the Tools section owns — `tools`, `allowedTools`,
// `toolAliases` — so the reducers can be reasoned about (and unit-
// tested) in isolation.
//
// Every reducer is total: takes a draft, returns a (possibly new)
// draft. None mutate the input. Vitest assertions in
// `tool-state.test.ts` lock the falsifiers for claims C2, C3, C4, C6
// of design-slice-2.md.

/**
 * Tools-section slice of the editor draft. Maps directly onto the
 * three fields of `agent-spec.json#/properties` that the Tools UI
 * controls: `tools` (string[]), `allowedTools` (string[]),
 * `toolAliases` (Record<string, string>).
 */
export type ToolsDraft = {
  readonly tools: readonly string[];
  readonly allowedTools: readonly string[];
  readonly toolAliases: Readonly<Record<string, string>>;
};

/**
 * Toggle a tool's enabled status.
 *
 * - **Enable** (`name ∉ tools`): append to `tools`. `allowedTools`
 *   and `toolAliases` unchanged.
 * - **Disable** (`name ∈ tools`): scrub `name` from ALL THREE fields.
 *   This is the cascade invariant from the spec's "Things to watch
 *   out for" item 8 — without the alias and allowed-list cleanup,
 *   re-enabling the tool later surfaces stale state.
 *
 * Idempotent on the enable path: calling twice with the same name
 * leaves `tools` unchanged (no duplicates).
 */
export function toggleTool(draft: ToolsDraft, name: string): ToolsDraft {
  if (draft.tools.includes(name)) {
    // Disable: scrub all three. The watch-out invariant.
    const { [name]: _alias, ...remainingAliases } = draft.toolAliases;
    return {
      tools: draft.tools.filter((t) => t !== name),
      allowedTools: draft.allowedTools.filter((t) => t !== name),
      toolAliases: remainingAliases,
    };
  }
  return { ...draft, tools: [...draft.tools, name] };
}

/**
 * Append `name` to `allowedTools` if not already present. Mutates
 * ONLY `allowedTools`; `tools` and `toolAliases` are unchanged.
 *
 * The independence is bidirectional: a tool can be allowed without
 * being in `tools[]` (the design's yellow "NOT VISIBLE" chip state),
 * and a tool can be in `tools[]` without being allowed (the default
 * for a freshly-enabled tool, which requires per-call permission).
 *
 * Whitespace-only names are rejected — returns the draft unchanged.
 */
export function addAllowed(draft: ToolsDraft, name: string): ToolsDraft {
  const trimmed = name.trim();
  if (!trimmed) return draft;
  if (draft.allowedTools.includes(trimmed)) return draft;
  return { ...draft, allowedTools: [...draft.allowedTools, trimmed] };
}

/**
 * Drop `name` from `allowedTools`. Idempotent: removing a name that
 * isn't in the list returns the draft unchanged. Never touches
 * `tools` or `toolAliases`.
 */
export function removeAllowed(draft: ToolsDraft, name: string): ToolsDraft {
  if (!draft.allowedTools.includes(name)) return draft;
  return {
    ...draft,
    allowedTools: draft.allowedTools.filter((t) => t !== name),
  };
}

/**
 * Split `tools` into native and external (MCP) groups. An entry routes
 * to `external` iff `name.startsWith("@")`; otherwise `native`. Source
 * order is preserved within each group.
 *
 * Anchored on `.startsWith` (not `.includes("@")`) so a hypothetical
 * name like `"weird@embedded"` lands in `native`. The substring-
 * match alternative would silently misroute it.
 */
export function partitionTools(
  tools: readonly string[],
): { native: string[]; external: string[] } {
  const native: string[] = [];
  const external: string[] = [];
  for (const t of tools) {
    if (t.startsWith("@")) external.push(t);
    else native.push(t);
  }
  return { native, external };
}

/**
 * Outcome of an `+Add external tool` request from the External (MCP)
 * sub-region. Discriminated union so the panel can route the failure
 * reason to a user-facing message rather than silently no-op'ing.
 *
 * Reasons (per the React reference `ExternalToolList`, AgentEditor.jsx:461):
 * - `empty` — input was whitespace-only.
 * - `not-mcp` — input doesn't start with `@`; external MCP entries
 *   require the `@server/tool` or `@server` shape.
 * - `duplicate` — input already exists in `tools[]`.
 */
export type AddExternalResult =
  | { ok: true; draft: ToolsDraft }
  | { ok: false; reason: "empty" | "not-mcp" | "duplicate" };

/**
 * Validate and apply an `+Add` request from the External (MCP)
 * sub-region. On success appends to `tools[]` only — `allowedTools[]`
 * and `toolAliases{}` are unchanged.
 *
 * Visibility (`tools[]`) and auto-allow (`allowedTools[]`) are
 * orthogonal concepts in the agent spec. The user adds an MCP tool
 * to `tools[]` here; if they also want it auto-allowed, the
 * AllowedToolsList sub-region's `+Add custom` is the separate path.
 * Mirrors the React reference at `ExternalToolList` (AgentEditor.jsx:461).
 */
export function addExternalTool(
  draft: ToolsDraft,
  raw: string,
): AddExternalResult {
  const trimmed = raw.trim();
  if (!trimmed) return { ok: false, reason: "empty" };
  if (!trimmed.startsWith("@")) return { ok: false, reason: "not-mcp" };
  if (draft.tools.includes(trimmed)) return { ok: false, reason: "duplicate" };
  return {
    ok: true,
    draft: { ...draft, tools: [...draft.tools, trimmed] },
  };
}

/**
 * Rail-count badge value for the Tools section. Returns `null` when
 * `tools[]` is empty (the badge is hidden, matching slice-1's
 * convention for `mcp` / `resources` / `hooks`); otherwise returns
 * the count. Lives here (rather than inline in the editor) so the
 * empty-vs-populated edge case is unit-testable.
 */
export function toolsRailBadge(
  draft: Pick<ToolsDraft, "tools">,
): number | null {
  return draft.tools.length === 0 ? null : draft.tools.length;
}

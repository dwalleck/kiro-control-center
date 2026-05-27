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
 * Toggle a tool's enabled status. NOT idempotent — each call flips
 * membership, so a second call with the same name disables what the
 * first call enabled (and vice-versa).
 *
 * - **Enable** (`name ∉ tools`): append to `tools`. `allowedTools`
 *   and `toolAliases` unchanged.
 * - **Disable** (`name ∈ tools`): scrub `name` from ALL THREE fields.
 *   This is the cascade invariant from the spec's "Things to watch
 *   out for" item 8 — without the alias and allowed-list cleanup,
 *   re-enabling the tool later surfaces stale state.
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
 * Set or clear the rename alias for `name`. The trimmed-empty case
 * removes the key entirely (an empty alias is the same state as no
 * alias — distinguishing them would create a third state the schema
 * doesn't model). Never touches `tools` or `allowedTools`.
 */
export function setAlias(
  draft: ToolsDraft,
  name: string,
  value: string,
): ToolsDraft {
  const trimmed = value.trim();
  const { [name]: _existing, ...rest } = draft.toolAliases;
  return {
    ...draft,
    toolAliases: trimmed === "" ? rest : { ...rest, [name]: trimmed },
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
 * Failure reasons for the `+Add external tool` request from the
 * External (MCP) sub-region. Single source of truth for the failure
 * union — consumers import `AddExternalReason` and switch on it with
 * a `_exhaustive: never` default arm.
 *
 * Reasons (per the React reference `ExternalToolList`):
 * - `empty` — input was whitespace-only.
 * - `not-mcp` — input doesn't start with `@`; external MCP entries
 *   require the `@server/tool` or `@server` shape.
 * - `duplicate` — input already exists in `tools[]`.
 */
export type AddExternalReason = "empty" | "not-mcp" | "duplicate";

/**
 * Outcome of an `+Add external tool` request. Discriminated union so
 * the panel can route the failure reason to a user-facing message
 * rather than silently no-op'ing.
 */
export type AddExternalResult =
  | { ok: true; draft: ToolsDraft }
  | { ok: false; reason: AddExternalReason };

// Compile-time exhaustiveness tripwire for `AddExternalReason`.
// Bidirectional guard mirroring `_PLUGIN_ACTION_VALUES` in
// `stores/plugin-updates.ts`:
//   - `satisfies readonly AddExternalReason[]` fails if an element of
//     the array is not a valid reason.
//   - `Exclude<AddExternalReason, …>` fails if `AddExternalReason`
//     gains a new arm not listed in `_ADD_EXTERNAL_REASON_VALUES`.
// The value-position `const _assert: … = true` forces type-check
// evaluation; an unused type alias resolving to `never` is valid TS,
// so the const is what makes the tripwire fire.
const _ADD_EXTERNAL_REASON_VALUES = [
  "empty",
  "not-mcp",
  "duplicate",
] as const satisfies readonly AddExternalReason[];
type _AssertAddExternalReasonExhaustive =
  Exclude<AddExternalReason, (typeof _ADD_EXTERNAL_REASON_VALUES)[number]> extends never
    ? true
    : never;
const _assertAddExternalReasonExhaustive: _AssertAddExternalReasonExhaustive = true;
void _assertAddExternalReasonExhaustive;

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
  // Bare `@` fails the @server/@server/tool shape requirement — there
  // must be at least one character after the prefix. Folds under
  // `not-mcp` so the user gets the shape-of-a-valid-name error message.
  if (!trimmed.startsWith("@") || trimmed === "@") {
    return { ok: false, reason: "not-mcp" };
  }
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

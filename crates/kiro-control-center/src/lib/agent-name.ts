// Agent-name validation for the Workflows > Agents editor.
//
// Two layers:
//   1. `isValidAgentName` — pure regex check, the building block.
//      Used by the IdentityPanel for inline feedback as the user
//      types in the Name input.
//   2. `validateAgentNameForSave` — the editor's save-time check
//      with the split-policy from kiro-k9ok: strict regex applies
//      to renames, but a user editing an existing agent whose name
//      doesn't match the regex (e.g. a marketplace-installed
//      "Terraform Agent") can save without renaming.

// The strict frontend regex is **NOT** parity with the backend's
// `validate_name` (validation.rs:550). The backend accepts a wider
// set: uppercase, underscores, internal whitespace, leading dots
// (e.g. "Terraform Agent", "my_plugin", ".hidden"). The frontend
// regex enforces kebab-case for authoring-side UX hygiene — encourages
// the convention used by other Kiro plugin assets.
//
// **Maintenance burden:** the regex syntax differs from the Rust
// validator's logic, so a parity test is impractical. Instead, this
// regex is a documented **subset** of what the backend accepts —
// every name that passes here also passes the backend, but the
// reverse is false. The split-policy below is what makes this
// asymmetry safe.
//
// Per kiro-k9ok decision (S14): split-policy resolves the asymmetry.
const AGENT_NAME_REGEX = /^[a-z0-9][a-z0-9-]*$/;

/**
 * Pure regex check — does `name` match the strict frontend
 * convention `^[a-z0-9][a-z0-9-]*$`?
 *
 * This does NOT mirror the backend's `validate_name`; see the
 * module-level comment for the asymmetry rationale. Use this for
 * inline UI feedback as the user types; use `validateAgentNameForSave`
 * for the save-time gate.
 */
export function isValidAgentName(name: string): boolean {
  return AGENT_NAME_REGEX.test(name);
}

/**
 * Editor-side save validation with the split-policy escape hatch.
 *
 * Returns `null` when the name is acceptable to save, or a
 * human-readable error string suitable for inline display when not.
 *
 * **Split-policy** (kiro-k9ok resolution):
 * - If `name` is empty: rejected ("Name is required.").
 * - If `name === originalName` and `originalName` is non-empty:
 *   accepted, regardless of regex. This is the escape hatch for
 *   marketplace-installed agents whose names predate (or violate)
 *   the kebab-case convention. The user can save edits without
 *   renaming; the backend's `validate_name` will accept the name
 *   as-is.
 * - Otherwise (new agent OR rename): the regex applies.
 *
 * `originalName` is the filename stem the editor opened with —
 * the empty string for new-agent mode.
 */
export function validateAgentNameForSave(
  name: string,
  originalName: string,
): string | null {
  if (!name) return "Name is required.";
  // Unchanged-name escape hatch. Only fires for edit mode
  // (originalName non-empty) — new-agent mode has originalName === ""
  // and falls through to the regex check.
  if (name === originalName && originalName !== "") return null;
  if (!isValidAgentName(name)) {
    return "Name must be lowercase letters, digits, or hyphens, and start with a letter or digit.";
  }
  return null;
}

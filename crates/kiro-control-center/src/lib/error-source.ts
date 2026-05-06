import { DELIM } from "$lib/keys";

export type RemediationClass = "stale_cache" | "manifest_invalid" | "unknown";

export const UPDATE_CHECK_PREFIX = "update-check" as const;
export const ERR_INSTALLED_PLUGINS = "installed-plugins" as const;
export const ERR_UPDATE_FETCH = "update-fetch" as const;

export type UpdateCheckKey =
  `${typeof UPDATE_CHECK_PREFIX}${typeof DELIM}${string}${typeof DELIM}${string}`;

// Compile-time guard: fails if UPDATE_CHECK_PREFIX loses `as const` and
// UpdateCheckKey silently widens to `string` (defeating typo protection
// on `fetchErrors.get/set/delete` with zero compile errors). The
// `const _assertNarrow = true` below forces evaluation — an unused type
// alias resolving to `never` is valid TS, so the value-position assignment
// is what makes the tripwire actually fire.
type _AssertNarrow = string extends UpdateCheckKey ? never : true;
const _assertNarrow: _AssertNarrow = true;

export const updateCheckErrKey = (
  remediation: RemediationClass,
  marketplace: string,
): UpdateCheckKey =>
  `${UPDATE_CHECK_PREFIX}${DELIM}${remediation}${DELIM}${marketplace}` as UpdateCheckKey;

// Type guard: narrows `s` to `UpdateCheckKey` when shape, emptiness, and
// prefix all check out. Use at trust boundaries (e.g. iterating SvelteMap
// keys typed as `string`) to recover the brand without an `as` cast.
export const isUpdateCheckKey = (s: string): s is UpdateCheckKey => {
  const parts = s.split(DELIM);
  return (
    parts.length === 3 &&
    parts.every((p) => p !== "") &&
    parts[0] === UPDATE_CHECK_PREFIX
  );
};

// Throw on malformed input rather than returning `{ remediation: undefined }`
// dressed up as `{ remediation: string }`. Every call site round-trips through
// `updateCheckErrKey`, so a malformed key is a programmer bug — throwing makes
// the bug loud instead of silent.
export const parseUpdateCheckKey = (
  key: string,
): { remediation: string; marketplace: string } => {
  if (!isUpdateCheckKey(key)) {
    throw new Error(
      `parseUpdateCheckKey: malformed key (expected UPDATE_CHECK_PREFIX + DELIM + remediation + DELIM + marketplace): ${JSON.stringify(key)}`,
    );
  }
  const parts = key.split(DELIM);
  return { remediation: parts[1], marketplace: parts[2] };
};

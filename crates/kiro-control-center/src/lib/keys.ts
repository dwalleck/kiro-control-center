// Composite-key helpers shared across BrowseTab and InstalledTab.
//
// The ASCII Unit Separator (\u001f) is reserved for exactly this purpose
// and cannot occur in marketplace/plugin/skill names, so it never collides
// the way "/" or ":" would. A marketplace named "foo" + plugin "bar"
// produces a distinct key from marketplace "fooba" + plugin "r".
//
// Exported (not just module-private) so BrowseTab can build its typed
// `ErrorSource` literal-union prefixes (`plugins${DELIM}`, `skills${DELIM}`,
// `bulk-skills${DELIM}`) off the same constant — drift would silently
// re-widen the ErrorSource type back to `string`.
export const DELIM = "\u001f";

export const pluginKey = (marketplace: string, plugin: string): string =>
  `${marketplace}${DELIM}${plugin}`;

export const skillKey = (
  marketplace: string,
  plugin: string,
  name: string,
): string => `${marketplace}${DELIM}${plugin}${DELIM}${name}`;

// Throw on malformed input rather than returning `{ plugin: undefined }`
// dressed up as `{ plugin: string }`. Every call site round-trips through
// `pluginKey` / `skillKey`, so a malformed key is a programmer bug —
// throwing makes the bug loud instead of silent (e.g. `fetchErrors.keys()`
// later produces an `undefined` field that flows into a banner).
export const parsePluginKey = (
  key: string,
): { marketplace: string; plugin: string } => {
  const parts = key.split(DELIM);
  if (parts.length !== 2 || parts[0] === "" || parts[1] === "") {
    throw new Error(
      `parsePluginKey: malformed key (expected exactly one DELIM separator): ${JSON.stringify(key)}`,
    );
  }
  return { marketplace: parts[0], plugin: parts[1] };
};

export const parseSkillKey = (
  key: string,
): { marketplace: string; plugin: string; name: string } => {
  const parts = key.split(DELIM);
  if (parts.length !== 3 || parts.some((p) => p === "")) {
    throw new Error(
      `parseSkillKey: malformed key (expected exactly two DELIM separators): ${JSON.stringify(key)}`,
    );
  }
  return { marketplace: parts[0], plugin: parts[1], name: parts[2] };
};

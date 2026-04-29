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

export const parsePluginKey = (
  key: string,
): { marketplace: string; plugin: string } => {
  const [marketplace, plugin] = key.split(DELIM);
  return { marketplace, plugin };
};

export const parseSkillKey = (
  key: string,
): { marketplace: string; plugin: string; name: string } => {
  const [marketplace, plugin, name] = key.split(DELIM);
  return { marketplace, plugin, name };
};

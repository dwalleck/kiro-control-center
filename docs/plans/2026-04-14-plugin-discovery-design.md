# Plugin Discovery: Scan-and-Merge for Unlisted Plugins

## Problem

Some skill repositories contain plugins that are not listed in their `marketplace.json`. For example, `dotnet/skills` has `plugins/dotnet-experimental/plugin.json` but it does not appear in the marketplace manifest. Additionally, some repositories have no `marketplace.json` at all but still contain valid `plugin.json` files. The current tool requires a marketplace manifest to function and cannot see unlisted plugins.

## Approach

Scan-and-merge on `marketplace add`. After cloning a repo, the tool reads `marketplace.json` if present, then performs a depth-limited directory scan for `plugin.json` files not covered by the manifest. The two sources are deduplicated and merged into a single plugin list. Repos without `marketplace.json` are supported — the scan is the sole discovery mechanism.

## Discovery Mechanism

New function `discover_plugins(repo_root, max_depth) -> Vec<DiscoveredPlugin>`:

- Walks the directory tree from `repo_root` up to `max_depth` levels (default: 3)
- At each directory, checks for `plugin.json`
- Parses it into a `PluginManifest` to get name and description
- Skips hidden directories (`.git`, `.claude-plugin`) and noise directories (`node_modules`, `target`, etc.)
- Returns discovered plugins with their relative paths

Deduplication compares resolved paths of scanned plugins against resolved paths of marketplace entries. If they overlap, the marketplace entry wins (it may carry a richer description).

**Location:** `crates/kiro-market-core/src/plugin.rs` alongside the existing `discover_skill_dirs` function.

## Type Changes

### New internal type (not user-facing)

```rust
/// A plugin found by scanning a repo for `plugin.json` files.
pub struct DiscoveredPlugin {
    pub name: String,
    pub description: Option<String>,
    /// Path to the directory containing plugin.json, relative to repo root.
    pub relative_path: PathBuf,
}
```

### No changes to existing types

`PluginBasicInfo`, `Marketplace`, `PluginEntry`, `PluginManifest`, and `PluginSource` are unchanged. Discovered plugins are converted to `PluginBasicInfo` values and merged into the same list — no `discovered` flag, no distinction visible to the user.

## Changes to `MarketplaceService::add()`

```
1. Clone/link repo                              (unchanged)
2. Try to read marketplace.json
   - If found -> parse, collect listed plugins
   - If not found -> empty list (NOT an error)
3. Scan repo for plugin.json files up to depth 3
4. Deduplicate: remove scanned plugins whose paths overlap with marketplace entries
5. Merge: marketplace plugins ++ remaining discovered plugins -> Vec<PluginBasicInfo>
6. Validate name, rename, register              (unchanged)
```

`ManifestNotFound` is no longer a terminal error. The only new error case: no `marketplace.json` AND the scan finds zero plugins -> `MarketplaceError::NoPluginsFound`.

## Name Derivation

When `marketplace.json` is absent, the marketplace name is derived from the source:

| Source type | Input | Derived name |
|---|---|---|
| `GitHub { repo }` | `"dotnet/skills"` | `"skills"` |
| `GitUrl { url }` | `"https://github.com/dotnet/skills.git"` | `"skills"` |
| `LocalPath { path }` | `"~/my-plugins"` | `"my-plugins"` |

New method `MarketplaceSource::fallback_name() -> Option<String>`: takes the last path/URL segment, strips `.git` suffix, validates with `validate_name()`. Returns `None` if validation fails — `add()` errors with a message suggesting `--name`.

## Install Fallback

No structural changes to the install flow. When `install` cannot find a plugin in `marketplace.json`, it re-runs the depth-limited scan on the cached clone and looks for a matching `plugin.json` by name. The scan is fast (depth 3) and avoids staleness — if `update` pulls new content, the next `install` picks it up automatically.

No sidecar files, no persistence of scan results.

## Error Handling

- `marketplace.json` missing -> fall through to scan (not an error)
- Scan finds zero plugins AND no marketplace.json -> `MarketplaceError::NoPluginsFound`
- Name derivation fails validation -> error suggesting `--name` override
- Plugin not in marketplace.json, scan finds no match -> existing `PluginError::NotFound`

## Testing Strategy

1. Repo with marketplace.json + unlisted plugins: merged list contains both
2. Repo with no marketplace.json, has plugin.json files: scan is sole source, name derived from repo
3. Repo with neither: `NoPluginsFound` error
4. Deduplication: marketplace entry and scan overlap -> single entry
5. Depth limit respected: plugin.json at depth 4 is not found
6. Hidden/noise directories skipped: `.git/plugin.json` is ignored
7. Install fallback: plugin not in marketplace.json but discoverable by scan
8. Name derivation: GitHub, git URL, local path all produce correct names

# Kiro-CLI Native Plugin Import (with Steering and Content Hashes)

## Problem

`kiro-market-cli` today imports two flavors of agent — Claude markdown and
Copilot markdown — by parsing their YAML frontmatter, normalizing into
`AgentDefinition`, and emitting Kiro JSON. That round-trip is intentionally
lossy: `AgentDefinition` omits Kiro-specific fields (`resources`, `hooks`,
`toolsSettings`, `useLegacyMcpJson`, `keyboardShortcut`, `welcomeMessage`,
`toolAliases`) so it can be the union of two *foreign* dialects.

Plugins like [`dwalleck/kiro-starter-kit`](https://github.com/dwalleck/kiro-starter-kit)
already author their agents directly in Kiro's native JSON format, with
sibling `prompts/` directories referenced by `file://` URIs and project-level
`steering/` files referenced from the agents' `resources` array. The current
pipeline cannot import these without silently dropping fields.

Three gaps to close:

1. **Native Kiro agent import** — validate-and-copy native JSON bundles
   instead of parse-and-translate, so author intent is preserved verbatim.
2. **Steering files** — a peer install target alongside skills and agents,
   landing at `.kiro/steering/`. The core has zero awareness of this concept
   today.
3. **Content-hash tracking** — folds in
   [kiro-control-center#27](https://github.com/dwalleck/kiro-control-center/issues/27)
   so reinstall is idempotent on unchanged content, and so future drift checks
   can answer "has upstream changed?" / "has the user edited locally?" without
   relying on author-supplied version strings.

## Approach

A plugin declares `"format": "kiro-cli"` in its `plugin.json`. When set, the
agent install path skips parse-and-translate entirely: it discovers `.json`
files at the agent scan-path roots, validates each as a Kiro agent, and copies
each JSON into `.kiro/agents/`. Companion files (everything in subdirectories
of the agent scan paths, one level deep) are copied as a single plugin-wide
bundle into `.kiro/agents/`, preserving the relative layout. Translated plugins
(no `format` field, or a future non-`kiro-cli` value) keep their existing flow
unchanged.

Steering becomes a third install target with its own manifest field
(`steering: Vec<String>`), default scan path (`./steering/`), discovery
function, project-layer install method, tracking, and service orchestrator.
It sits alongside skills and agents — independent, parallel, never coupled.

Every install path (skills, native agents, native companion bundles, steering,
and the existing translated agents) populates two content hashes per artifact:
`source_hash` (what was in the marketplace) and `installed_hash` (what landed
in the project). Reinstall logic compares `source_hash` to short-circuit
unchanged content into a no-op; future drift checks compare `installed_hash`
against on-disk bytes to detect local edits.

## Manifest Schema

Two additions to `PluginManifest` in `crates/kiro-market-core/src/plugin.rs`,
both backward-compatible:

```rust
#[derive(Debug, Clone, Deserialize)]
pub struct PluginManifest {
    pub name: String,
    pub version: Option<String>,
    pub description: Option<String>,
    #[serde(default)]
    pub skills: Vec<String>,
    #[serde(default)]
    pub agents: Vec<String>,

    // NEW
    #[serde(default)]
    pub format: Option<PluginFormat>,
    #[serde(default)]
    pub steering: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum PluginFormat {
    KiroCli,
    // Future variants (KiroIde, etc.) land additively without breaking
    // external consumers thanks to non_exhaustive.
}
```

`#[serde(rename_all = "kebab-case")]` gives `KiroCli → "kiro-cli"`. Strict
deserialization means an unknown variant (typo `"format": "Kiro"` or
forward-looking `"format": "kiro-ide"`) fails manifest parsing loudly with a
message naming the unknown variant — surfaces author errors instead of silently
treating them as the default.

**Default scan paths** in `crates/kiro-market-core/src/lib.rs`, alongside the
existing `DEFAULT_AGENT_PATHS`:

```rust
pub const DEFAULT_STEERING_PATHS: &[&str] = &["./steering/"];
```

**A starter-kit-style manifest after this change:**

```json
{
  "name": "kiro-code-reviewer",
  "version": "0.1.0",
  "format": "kiro-cli"
}
```

No `skills`, `agents`, or `steering` declarations needed — defaults match the
directory layout the author already uses.

## Layer Contracts

Four layers, each with explicit input/output contracts. Project-layer
operations are added as methods on the existing `KiroProject` struct, matching
the codebase's current pattern (`install_skill_dir`, `install_agent`). When
`ProjectService` is later extracted from `MarketplaceService` per the broader
core-split refactor, all `KiroProject` methods migrate together — the new
methods don't need to be free functions to be split-friendly.

### Layer 1 — Discovery (`agent/discover.rs`, `steering/discover.rs`)

Two thin functions, each returning candidate paths only. Parsing happens in a
separate step (Layer 1.5) so discovery can never fail per-file — it returns
the candidates, the parser decides what's a real agent and what's a broken
file.

```rust
/// Native Kiro agent JSON candidates: `.json` files at scan-path roots.
pub fn discover_native_kiro_agents_in_dirs(
    plugin_dir: &Path,
    scan_paths: &[String],
) -> Vec<DiscoveredNativeFile>;

/// Companion file candidates: any file in subdirectories of scan paths,
/// one level deep, excluding the README/CONTRIBUTING/CHANGELOG conventions.
/// Plugin-wide — not attributed to any specific agent.
pub fn discover_native_companion_files(
    plugin_dir: &Path,
    scan_paths: &[String],
) -> Vec<DiscoveredNativeFile>;

/// Steering candidates: `.md` files at scan-path roots.
pub fn discover_steering_files_in_dirs(
    plugin_dir: &Path,
    scan_paths: &[String],
) -> Vec<DiscoveredNativeFile>;

#[derive(Debug, Clone)]
pub struct DiscoveredNativeFile {
    /// Absolute path to the source file.
    pub source: PathBuf,
    /// The resolved scan-path directory (e.g. `<plugin>/agents/`).
    /// Used to compute destination-relative paths at install time.
    pub scan_root: PathBuf,
}
```

**Inputs:** `(plugin_dir: &Path, scan_paths: &[String])` where `scan_paths`
are plugin-relative.

**Outputs:** `Vec<DiscoveredNativeFile>`. Never returns errors — invalid scan
paths produce `warn!` and an empty contribution.

**Guarantees:**
- Every returned `source` is inside `plugin_dir` (validated via
  `validate_relative_path`, refusing absolute paths and `..` traversal).
- Every returned `source` was a regular file at discovery time, **not a
  symlink** (checked via `symlink_metadata`).
- README / CONTRIBUTING / CHANGELOG (case-insensitive) excluded.
- Top-level scan only — `agents/foo.json` in scope, `agents/nested/foo.json`
  out of scope (for `discover_native_kiro_agents_in_dirs`).
- Companion scan is one level deep — `agents/prompts/x.md` in scope,
  `agents/prompts/nested/y.md` out of scope (for
  `discover_native_companion_files`).

The project layer re-validates everything at install time. Discovery is
best-effort filtering, not a security boundary (TOCTOU disclaimer).

### Layer 1.5 — Parsing (`agent/parse_native.rs`)

```rust
/// Parses a native Kiro agent JSON candidate, validates the `name` field
/// via the path-safe newtype, extracts `mcpServers` for the install gate,
/// and returns a bundle ready for the project layer to install.
pub fn parse_native_kiro_agent_file(
    json_path: &Path,
    scan_root: &Path,
) -> Result<NativeAgentBundle, NativeParseFailure>;

#[derive(Debug, Clone)]
pub struct NativeAgentBundle {
    /// Absolute path to the source `.json` file.
    pub agent_json_source: PathBuf,
    /// The scan root (e.g. `<plugin>/agents/`) the JSON was discovered under.
    pub scan_root: PathBuf,
    /// Validated agent name (from JSON `name` field).
    pub name: String,
    /// MCP server entries from the JSON's `mcpServers` field. Empty if
    /// the field is absent or empty. Drives the `--accept-mcp` install gate.
    pub mcp_servers: BTreeMap<String, McpServerConfig>,
    /// The full parsed JSON, preserved verbatim for atomic copy-out at
    /// install time. Avoids re-reading the file from disk during install.
    pub raw_json: serde_json::Value,
}
```

**Failures:** `NativeParseFailure` enumerates parse-time outcomes
(`MissingFrontmatter` analog), mirroring the existing `ParseFailure` for
translated agents:

```rust
#[derive(Debug, Clone)]
pub enum NativeParseFailure {
    IoError(io::Error),
    InvalidJson(serde_json::Error),
    MissingName,
    InvalidName(String),  // newtype validation reason
}
```

The existing companion file scan does NOT need a separate parse layer —
companion files are copied as opaque bytes. They're identified by directory
walking (Layer 1) and consumed directly by the project layer.

### Layer 2 — Project (`project.rs`)

Methods on the existing `KiroProject` struct:

```rust
impl KiroProject {
    /// Install one native Kiro agent JSON. Idempotent if `source_hash`
    /// matches the tracked entry; otherwise requires `force == true` for
    /// content-changed reinstall, and returns a typed error for cross-plugin
    /// name clashes or orphan-on-disk files.
    pub fn install_native_agent(
        &self,
        bundle: &NativeAgentBundle,
        marketplace: &str,
        plugin: &str,
        version: Option<&str>,
        source_hash: &str,
        force: bool,
    ) -> Result<InstalledNativeAgentOutcome, AgentError>;

    /// Install a plugin's companion file bundle as a single atomic unit.
    /// Companion ownership is plugin-scoped; intra-plugin file overlap is
    /// fine, cross-plugin file overlap is a `PathOwnedByOtherPlugin` error.
    pub fn install_native_companions(
        &self,
        files: &[DiscoveredNativeFile],
        marketplace: &str,
        plugin: &str,
        version: Option<&str>,
        source_hash: &str,
        force: bool,
    ) -> Result<InstalledNativeCompanionsOutcome, AgentError>;

    /// Install one steering file. Same idempotency / force / collision rules
    /// as native agent install.
    pub fn install_steering_file(
        &self,
        source: &DiscoveredNativeFile,
        marketplace: &str,
        plugin: &str,
        version: Option<&str>,
        source_hash: &str,
        force: bool,
    ) -> Result<InstalledSteeringOutcome, SteeringError>;
}
```

**Atomicity guarantee:** for a single agent / companion bundle / steering
file, either all destination files exist + tracking is updated, or nothing
changed. Implementation: stage to a temp dir under the project tracking dir,
validate every destination against tracking BEFORE any rename, then
rename-into-place under the existing `with_file_lock` primitive. Failure at
any step removes the staging dir and returns without touching the project
tree. This mirrors the existing `install_agent_inner` (`project.rs:470`)
pattern.

**Collision detection** runs against the relevant tracking file. Per
destination path, four outcomes:

- Path absent + no tracking → write, add tracking entry.
- Path present + tracking owned by **same plugin** + **same `source_hash`** →
  no-op (idempotent reinstall). Returns success outcome with
  `forced_overwrite: false` and `was_idempotent: true`.
- Path present + tracking owned by **same plugin** + **different `source_hash`**
  → `ContentChangedRequiresForce` unless `force == true`.
- Path present + tracking owned by **different plugin** →
  `PathOwnedByOtherPlugin` (or `NameClashWithOtherPlugin` for the agent JSON
  itself) unless `force == true` (transfers ownership, sets
  `forced_overwrite: true` on outcome).
- Path present + no tracking (orphan) → `OrphanFileAtDestination` unless
  `force == true`.

**No silent overwrites:** every conflict produces a typed error. There is no
path through these methods that overwrites a tracked or orphaned file without
`force == true`.

### Layer 3 — Service (`service/mod.rs`)

```rust
pub fn install_plugin_agents(
    &self,
    ctx: &PluginInstallContext,
    options: AgentInstallOptions,
) -> InstallAgentsResult;

pub fn install_plugin_steering(
    &self,
    ctx: &PluginInstallContext,
    options: SteeringInstallOptions,
) -> InstallSteeringResult;

#[derive(Debug, Clone, Copy, Default)]
pub struct AgentInstallOptions {
    pub force: bool,
    pub accept_mcp: bool,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SteeringInstallOptions {
    pub force: bool,
}
```

`install_plugin_agents` becomes a thin dispatcher on `ctx.format`:

```rust
match ctx.format {
    Some(PluginFormat::KiroCli) => self.install_native_kiro_cli_agents_inner(ctx, opts),
    None => self.install_translated_agents_inner(ctx, opts),  // existing body, renamed
}
```

The native inner:
1. Discovers agent JSON candidates via `discover_native_kiro_agents_in_dirs`.
2. Discovers companion file candidates via `discover_native_companion_files`.
3. Parses each agent JSON candidate via `parse_native_kiro_agent_file`.
4. For each parsed bundle: applies the MCP gate (skip + warn if Stdio servers
   present without `--accept-mcp`), computes `source_hash`, calls
   `project.install_native_agent`, accumulates outcomes.
5. After all agents: computes the companion bundle's `source_hash` over the
   discovered companion files, calls `project.install_native_companions`
   ONCE per plugin, accumulates the outcome.
6. Returns aggregated `InstallAgentsResult`.

**Guarantees:**
- Per-item failure isolation — one failed bundle does not abort the rest.
- All discovery + parse warnings reach the caller in the result's `warnings`
  field.
- Top-level `Result::Err` is reserved for catastrophic problems (tracking-file
  deserialization failure, manifest unreadable). Per-item failures live inside
  the result struct.
- Companion bundle install runs after all agent installs so that a
  per-agent failure does not block companion-file install for the surviving
  agents (and so the user can still uninstall via the plugin's tracking even
  when some agents failed).

### Layer 4 — Frontend (CLI / Tauri)

Consumes `Install*Result` types, renders to user, sets exit code based on
whether `failed` is non-empty. Tauri-side wire-format translation uses
`error_full_chain(&err)` per CLAUDE.md, never `.to_string()`. Tauri commands
follow the existing `_impl` pattern from `crates/kiro-control-center/src-tauri/
src/commands/browse.rs::install_skills_impl`.

CLI install order in `crates/kiro-market/src/commands/install.rs`:

```
1. Resolve & cache plugin source     (existing)
2. install_plugin_skills             (existing)
3. install_plugin_agents             (existing call site, internal dispatch)
4. install_plugin_steering           (NEW)
5. Render combined per-plugin summary (presenter grows steering rows)
```

Steering install is independent — a plugin with steering and no agents
installs fine, and vice versa.

## Type Changes

### Discovery / parsing layer

```rust
// agent/discover.rs
#[derive(Debug, Clone)]
pub struct DiscoveredNativeFile {
    pub source: PathBuf,
    pub scan_root: PathBuf,
}

// agent/parse_native.rs
#[derive(Debug, Clone)]
pub struct NativeAgentBundle {
    pub agent_json_source: PathBuf,
    pub scan_root: PathBuf,
    pub name: String,
    pub mcp_servers: BTreeMap<String, McpServerConfig>,
    pub raw_json: serde_json::Value,
}

#[derive(Debug, Clone)]
pub enum NativeParseFailure {
    IoError(io::Error),
    InvalidJson(serde_json::Error),
    MissingName,
    InvalidName(String),
}
```

### Project boundary (success outcomes)

```rust
#[derive(Debug, Clone)]
pub struct InstalledNativeAgentOutcome {
    pub name: String,
    pub json_path: PathBuf,                    // absolute destination
    pub forced_overwrite: bool,
    pub was_idempotent: bool,
    pub source_hash: String,
    pub installed_hash: String,
}

#[derive(Debug, Clone)]
pub struct InstalledNativeCompanionsOutcome {
    pub plugin: String,
    pub files: Vec<PathBuf>,                   // absolute destinations
    pub forced_overwrite: bool,
    pub was_idempotent: bool,
    pub source_hash: String,
    pub installed_hash: String,
}

#[derive(Debug, Clone)]
pub struct InstalledSteeringOutcome {
    pub source: PathBuf,
    pub destination: PathBuf,
    pub forced_overwrite: bool,
    pub was_idempotent: bool,
    pub source_hash: String,
    pub installed_hash: String,
}
```

### Service-layer aggregate results

```rust
pub struct InstallAgentsResult {
    pub installed_agents: Vec<InstalledNativeAgentOutcome>,
    pub installed_companions: Option<InstalledNativeCompanionsOutcome>,  // None if zero companions
    pub skipped: Vec<SkippedAgent>,            // existing; translated path only
    pub failed: Vec<FailedAgent>,              // grows new variants from native
    pub warnings: Vec<DiscoveryWarning>,
}

// For translated plugins, installed_agents holds the existing per-bundle
// outcome (with companion_files populated for the prompts/<name>.md file
// the translator emits — see Tracking Schema below). installed_companions
// is None for translated plugins.

pub struct FailedAgent {
    pub name: Option<String>,           // None when failure is pre-name-extraction
    pub source_path: PathBuf,
    pub error: AgentError,
}

pub struct InstallSteeringResult {
    pub installed: Vec<InstalledSteeringOutcome>,
    pub failed: Vec<FailedSteeringFile>,
    pub warnings: Vec<DiscoveryWarning>,
    // No `skipped` — steering has no idempotent-skip semantic distinct from
    // success: idempotent reinstall is a successful outcome with
    // `was_idempotent: true`.
}

pub struct FailedSteeringFile {
    pub source: PathBuf,
    pub error: SteeringError,
}
```

### Extended `PluginInstallContext`

Lives where `resolve_plugin_install_context_from_dir` already lives
(`service/browse.rs`). Three new fields:

```rust
pub struct PluginInstallContext {
    pub plugin_dir: PathBuf,
    pub plugin_name: String,
    pub plugin_version: Option<String>,
    pub skill_scan_paths: Vec<String>,
    pub agent_scan_paths: Vec<String>,
    pub steering_scan_paths: Vec<String>,    // NEW
    pub format: Option<PluginFormat>,        // NEW
    pub manifest_warnings: Vec<ManifestWarning>,
}
```

The resolver function grows two lines: read `manifest.steering` (defaulting
to `DEFAULT_STEERING_PATHS`) and `manifest.format` (defaulting to `None`).

## Tracking Schema and Content Hashes

Type names match the existing convention (`InstalledSkillMeta`,
`InstalledAgentMeta`, `InstalledSkills`, `InstalledAgents`). All schema
additions use `Option<T>` with `#[serde(default)]` so existing tracking files
load unchanged.

### Existing tracking, extended

```rust
// project.rs (existing types, with new fields marked NEW)

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledSkillMeta {
    pub marketplace: String,
    pub plugin: String,
    pub version: Option<String>,
    pub installed_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_hash: Option<String>,        // NEW (per #27)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installed_hash: Option<String>,     // NEW (per #27)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledAgentMeta {
    pub marketplace: String,
    pub plugin: String,
    pub version: Option<String>,
    pub installed_at: DateTime<Utc>,
    pub dialect: AgentDialect,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_hash: Option<String>,        // NEW
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installed_hash: Option<String>,     // NEW
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InstalledAgents {
    pub agents: HashMap<String, InstalledAgentMeta>,
    #[serde(default)]
    pub native_companions: HashMap<String, InstalledNativeCompanionsMeta>,  // NEW
}
```

`AgentDialect` (in `agent/types.rs`) gains a third variant:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum AgentDialect {
    Claude,
    Copilot,
    Native,    // NEW: serializes to "native". Marks agents installed via
               // the kiro-cli native path (not parse-and-translate).
}
```

`#[non_exhaustive]` was already in place to enable exactly this kind of
extension.

### New tracking types

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledNativeCompanionsMeta {
    pub marketplace: String,
    pub plugin: String,
    pub version: Option<String>,
    pub installed_at: DateTime<Utc>,
    /// Relative paths under `.kiro/agents/` of every companion file owned
    /// by this plugin. Used for collision detection and uninstall.
    pub files: Vec<PathBuf>,
    pub source_hash: String,
    pub installed_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledSteeringMeta {
    pub marketplace: String,
    pub plugin: String,
    pub version: Option<String>,
    pub installed_at: DateTime<Utc>,
    pub source_hash: String,
    pub installed_hash: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InstalledSteering {
    /// Map from steering file's relative path under `.kiro/steering/` to
    /// its installation metadata. Mirrors the InstalledAgents shape.
    pub files: HashMap<PathBuf, InstalledSteeringMeta>,
}
```

### Tracking file layout on disk

```
.kiro/
├── installed-skills.json           (existing, schema extended with hashes)
├── installed-agents.json           (existing, schema extended with hashes
│                                    + native_companions map)
├── installed-steering.json         (NEW)
└── ... (project content)
```

**Native companions sample entry in `installed-agents.json`:**

```jsonc
{
  "agents": {
    "code-reviewer": {
      "marketplace": "kiro-starter-kit",
      "plugin": "kiro-code-reviewer",
      "version": "0.1.0",
      "installed_at": "2026-04-23T10:00:00Z",
      "dialect": "native",
      "source_hash": "blake3:8f9a...",
      "installed_hash": "blake3:8f9a..."
    }
  },
  "native_companions": {
    "kiro-code-reviewer": {
      "marketplace": "kiro-starter-kit",
      "plugin": "kiro-code-reviewer",
      "version": "0.1.0",
      "installed_at": "2026-04-23T10:00:00Z",
      "files": [
        "prompts/code-reviewer.md",
        "prompts/code-simplifier.md",
        "prompts/comment-analyzer.md"
      ],
      "source_hash": "blake3:e2c4...",
      "installed_hash": "blake3:e2c4..."
    }
  }
}
```

**`installed-steering.json` shape:**

```jsonc
{
  "files": {
    "review-process.md": {
      "marketplace": "kiro-starter-kit",
      "plugin": "kiro-code-reviewer",
      "version": "0.1.0",
      "installed_at": "2026-04-23T10:00:00Z",
      "source_hash": "blake3:a1b2...",
      "installed_hash": "blake3:a1b2..."
    }
  }
}
```

### Translated-agent path also gets hashes (and a companion entry)

The existing `install_agent_inner` (`project.rs:470`) writes
`<name>.json` plus `prompts/<name>.md`. To keep tracking uniform, the
translated path also:
1. Computes `source_hash` over the agent's parsed source (the .md file's bytes).
2. Computes `installed_hash` over the emitted JSON + prompt body.
3. For each translated plugin, writes a synthesized `native_companions` entry
   listing its `prompts/<name>.md` files. Even though they didn't come from
   a "native" plugin, treating them uniformly means `--force` cross-plugin
   transfers and uninstall both work the same way regardless of source format.

This keeps a single tracking model — the `dialect` field discriminates HOW
the agent was installed, but tracking semantics are dialect-agnostic.

### Hash primitive

New module `crates/kiro-market-core/src/hash.rs`:

```rust
use blake3::Hasher;

#[derive(Debug, thiserror::Error)]
pub enum HashError {
    #[error("failed to read `{path}` while hashing")]
    ReadFailed {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

/// Deterministic tree-hash over `relative_paths` rooted at `base`.
///
/// Sorts paths internally for determinism. For each path, feeds
/// `path_bytes || 0x00 || file_bytes || 0x00` into blake3.
/// The NUL separators prevent file-rename collisions
/// (`a/b` + content `XY` would otherwise collide with `a` + content `b\0XY`).
///
/// Returns `"blake3:" + hex_digest`.
pub fn hash_artifact(
    base: &Path,
    relative_paths: &[PathBuf],
) -> Result<String, HashError>;

/// Convenience: hash an entire directory tree by walking it.
/// Used by skill install (which doesn't know its content list ahead of time).
pub fn hash_dir_tree(root: &Path) -> Result<String, HashError>;
```

**Algorithm:** blake3 (~10× faster than SHA-256 on modern CPUs, supports
keyed-MAC for future signed manifests). Output is hex-encoded with the
algorithm prefix (`"blake3:"`) so a future migration to a different algorithm
is schema-compatible.

**No content normalization:** no whitespace stripping, no line-ending
normalization. A file that differs only by CRLF vs LF is legitimately
different — downstream tools may treat them differently. Hash truth, not
"probably the same."

### What hashes gate

- `source_hash` gates **install decisions**: same-plugin reinstall with
  matching `source_hash` is a no-op; mismatch requires `--force`.
- `installed_hash` gates **nothing in this design**. It exists for future
  drift detection (a `kiro-market check` command, or a "you have local edits"
  warning before reinstall) and is populated at install time so the future
  feature has data to compare against. Read-side use is out of scope here.

## Collision Policy

Fail-loudly default, `--force` overrides. Per-item failures isolated — one
collision in a batch does not block the rest.

| Scenario | Default | `--force` |
|---|---|---|
| Cross-plugin agent name clash (native) | `NameClashWithOtherPlugin` | Transfer ownership |
| Cross-plugin companion-file path clash (any file in the new plugin's bundle conflicts with a file in another plugin's tracked bundle) | `PathOwnedByOtherPlugin` | The entire new bundle installs; conflicting files' ownership transfers from prior plugin's bundle entry to the new plugin's bundle entry. If the prior plugin's bundle entry loses all its files, the entry is removed. |
| Cross-plugin steering-file path clash | `PathOwnedByOtherPlugin` | Transfer ownership |
| Same plugin, same name, **same `source_hash`** | No-op (idempotent reinstall) | Same: no-op |
| Same plugin, same name, **different `source_hash`** | `ContentChangedRequiresForce` | Overwrite |
| Same plugin, same companion bundle, **same `source_hash`** | No-op | Same: no-op |
| Same plugin, same companion bundle, **different `source_hash`** | `ContentChangedRequiresForce` | Overwrite the entire bundle (atomic) |
| Same plugin, same steering path, **same `source_hash`** | No-op | Same: no-op |
| Same plugin, same steering path, **different `source_hash`** | `ContentChangedRequiresForce` | Overwrite |
| Orphan file at destination | `OrphanFileAtDestination` | Overwrite, take ownership |

**Companion bundles are plugin-scoped, not agent-scoped.** Two agents within
the same plugin can reference the same companion file — that's a deliberate
authoring choice and the install path treats the companion bundle as one
unit. Cross-plugin companion file overlap is still a clash because the file
ownership model is per-plugin.

**Existing translated-agent flow's collision behavior is unchanged.** A
translated-agent name clash today produces `AgentError::AlreadyInstalled` and
the service routes it to the `skipped` bucket. That behavior stays — this
design only changes new install paths and the new collision types. Aligning
translated and native is a separate decision.

**`--force` warning surface.** Cross-plugin ownership transfers are loud —
a `warn!` at install time naming the prior owner and the new owner, and
`forced_overwrite: true` on the success outcome so the CLI can render a
transfer notice in the per-plugin summary.

**Forward-compat:** future "auto-namespace on conflict" (e.g. install as
`<plugin>__<agent>.json` when name clashes) needs no schema changes — the
tracking already carries `plugin: String` per file, so the rename logic is
purely a service-layer addition.

## Error Handling

Three new typed errors, following CLAUDE.md conventions (typed variants, no
`reason: String`, `#[source]` for inner errors, `error_full_chain` at
boundaries).

### `AgentError` gains five native-only variants

```rust
#[derive(Debug, thiserror::Error)]
pub enum AgentError {
    // ... existing variants ...

    #[error("native agent JSON `{path}` failed to parse")]
    NativeManifestParseFailed {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },

    #[error("native agent at `{path}` is missing the required `name` field")]
    NativeManifestMissingName { path: PathBuf },

    #[error("native agent at `{path}` has an invalid `name`: {reason}")]
    NativeManifestInvalidName { path: PathBuf, reason: String },

    #[error(
        "native agent name `{name}` would clobber an agent owned by plugin \
         `{owner}`; pass --force to transfer ownership"
    )]
    NameClashWithOtherPlugin { name: String, owner: String },

    #[error(
        "native agent `{name}` content has changed since last install; \
         pass --force to overwrite"
    )]
    ContentChangedRequiresForce { name: String },
}
```

### `SteeringError` is new

```rust
#[derive(Debug, thiserror::Error)]
pub enum SteeringError {
    #[error("steering source `{path}` could not be read")]
    SourceReadFailed {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error(
        "steering file `{rel}` would clobber a file owned by plugin `{owner}`; \
         pass --force to transfer ownership"
    )]
    PathOwnedByOtherPlugin { rel: PathBuf, owner: String },

    #[error(
        "steering file exists at `{path}` but has no tracking entry; \
         remove it manually or pass --force"
    )]
    OrphanFileAtDestination { path: PathBuf },

    #[error(
        "steering file `{rel}` content has changed since last install; \
         pass --force to overwrite"
    )]
    ContentChangedRequiresForce { rel: PathBuf },

    #[error("steering tracking I/O failed at `{path}`")]
    TrackingIoFailed {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}
```

### Companion-bundle collisions reuse existing shapes

Companion-bundle collisions during native-agent install reuse
`AgentError::PathOwnedByOtherPlugin` and `AgentError::OrphanFileAtDestination`
— same shape as steering's, scoped under the agent error type. The bundle
install method walks each destination path and produces these per-path errors
before any file write happens, so a partial-bundle-installed state is
impossible.

### Top-level wiring

Both new types get added as `#[error(transparent)]` arms on
`kiro_market_core::Error`:

```rust
pub enum Error {
    // ... existing ...
    #[error(transparent)]
    Agent(#[from] AgentError),
    #[error(transparent)]
    Steering(#[from] SteeringError),
}
```

`.source()` walks through cleanly. Tauri/log boundaries use
`error_full_chain(&err)` per CLAUDE.md.

### Newtype validation at parse boundary

The native agent JSON's `name` field gets wrapped in the existing path-safe
newtype convention (`RelativePath`-style, see
`crates/kiro-market-core/src/validation.rs:28`) at deserialization, not at use.
Bad input fails at parse time, not later. The full agent JSON body remains a
`serde_json::Value` (we don't model every Kiro agent field) — only the fields
we *act on* (`name`, `mcpServers`) get typed extraction.

### Classifier discipline

`SkippedReason::from_plugin_error`, `PluginError::remediation_hint`, and any
sibling classifier touching these enums must enumerate every new variant
explicitly per CLAUDE.md ("classifier functions over error enums enumerate
every variant"). No `_ =>` defaults. Audit during implementation:
`SkippedReason::from_plugin_error` and `remediation_hint` both gain explicit
arms for `NameClashWithOtherPlugin`, `ContentChangedRequiresForce`,
`NativeManifestParseFailed`, etc.

## Module Map

```
crates/kiro-market-core/src/
├── plugin.rs                    [+] format: Option<PluginFormat>
│                                [+] steering: Vec<String>
│                                [+] PluginFormat enum
├── lib.rs                       [+] DEFAULT_STEERING_PATHS
│                                [+] pub mod hash; pub mod steering;
├── hash.rs                      [NEW] hash_artifact, hash_dir_tree, HashError
├── agent/
│   ├── discover.rs              [+] discover_native_kiro_agents_in_dirs
│   │                            [+] discover_native_companion_files
│   │                            [+] DiscoveredNativeFile
│   ├── parse_native.rs          [NEW] parse_native_kiro_agent_file
│   │                                  NativeAgentBundle, NativeParseFailure
│   ├── mod.rs                   [+] pub mod parse_native;
│   │                            [+] re-exports
│   └── types.rs                 [+] AgentDialect::Native variant
├── steering/                    [NEW]
│   ├── mod.rs
│   ├── discover.rs              [+] discover_steering_files_in_dirs
│   └── types.rs                 [+] SteeringSource, InstallSteeringResult,
│                                    InstalledSteeringOutcome,
│                                    FailedSteeringFile
├── project.rs                   [+] install_native_agent (KiroProject method)
│                                [+] install_native_companions (method)
│                                [+] install_steering_file (method)
│                                [+] InstalledSkillMeta: source_hash,
│                                    installed_hash
│                                [+] InstalledAgentMeta: source_hash,
│                                    installed_hash
│                                [+] InstalledAgents: native_companions map
│                                [+] InstalledNativeCompanionsMeta
│                                [+] InstalledSteering, InstalledSteeringMeta
│                                [+] INSTALLED_STEERING_FILE constant
├── service/
│   ├── mod.rs                   [+] install_plugin_steering orchestrator
│   │                            [+] dispatch in install_plugin_agents on
│   │                                PluginFormat
│   │                            [+] install_native_kiro_cli_agents_inner
│   └── browse.rs                [+] PluginInstallContext.steering_scan_paths,
│                                    .format
└── error.rs                     [+] AgentError native variants
                                 [+] SteeringError
                                 [+] HashError
                                 [+] Error::Agent, Error::Steering arms
```

CLI side:
```
crates/kiro-market/src/commands/install.rs
                                 [+] call svc.install_plugin_steering(...)
                                 [+] presenter rows for steering
```

Tauri side: explicitly out of scope. New Tauri commands for native install /
steering land when the Tauri side adopts the feature, following the existing
`_impl` pattern (`crates/kiro-control-center/src-tauri/src/commands/browse.rs::install_skills_impl`).

## Discovery and Install Pipeline

```
plugin.json ──► load_plugin_manifest ──► PluginInstallContext
                                              │
                ┌─────────────────────────────┼─────────────────────────────┐
                ▼                             ▼                             ▼
        install_plugin_skills      install_plugin_agents       install_plugin_steering
                │                             │                             │
                │                             ▼                             │
                │                ┌─ format == KiroCli? ─┐                   │
                │                │           │           │                  │
                │                ▼           ▼           ▼                  │
                │       install_native_   ...inner    install_translated_   │
                │       kiro_cli_agents_              agents_inner          │
                │       inner               (existing, renamed)             │
                │                │                     │                    │
                ▼                ▼                     ▼                    ▼
         InstallSkillsResult         InstallAgentsResult              InstallSteeringResult
```

### Native agent install flow (per plugin)

1. **Discovery:** call `discover_native_kiro_agents_in_dirs` and
   `discover_native_companion_files` — get two `Vec<DiscoveredNativeFile>`
   lists.
2. **Per agent:** for each agent JSON candidate:
   - Call `parse_native_kiro_agent_file` → `Result<NativeAgentBundle, NativeParseFailure>`.
   - Parse failures → `failed` entry, continue.
   - Apply MCP gate: if `bundle.mcp_servers` contains a Stdio entry and
     `accept_mcp == false` → `failed` entry naming the agent, continue.
   - Compute `source_hash = hash_artifact(&bundle.scan_root, &[bundle.relative_path()])`
     — single-file hash for the agent JSON.
   - Call `project.install_native_agent(...)`. Outcome → `installed_agents`
     or `failed`.
3. **Per plugin (after all agents):** for the companion files (if any):
   - Compute `source_hash = hash_artifact(&companion_scan_root, &companion_relative_paths)`
     over the discovered companion file set.
   - Call `project.install_native_companions(...)`. Outcome →
     `installed_companions: Some(...)` or `failed`.
4. Return `InstallAgentsResult`.

### Steering install flow (per plugin)

1. Call `discover_steering_files_in_dirs` → `Vec<DiscoveredNativeFile>`.
2. For each file:
   - Compute `source_hash = hash_artifact(&file.scan_root, &[file.relative_path()])`.
   - Call `project.install_steering_file(...)`. Outcome → `installed`
     or `failed`.
3. Return `InstallSteeringResult`.

## Testing Strategy

Three tiers, mirroring the existing test layout. CLAUDE.md's "tests must cover
branches not patterns" applies — assertions target distinct code paths, not
just the happy case.

### Tier 1: Discovery and parsing (unit, no project dir)

For `discover_native_kiro_agents_in_dirs`,
`discover_native_companion_files`, and `discover_steering_files_in_dirs`,
port the entire test set from `discover_agents_in_dirs`
(`agent/discover.rs:120-301`) — same security surface, same bugs to prevent:

- Finds `.json` (or `.md`) files in default paths.
- Honors custom scan paths from manifest.
- Rejects path traversal (`../secrets/`).
- Rejects absolute paths (`/etc/`).
- Skips symlinks (uses `symlink_metadata`, never follows).
- Excludes `README.md` / `CONTRIBUTING.md` / `CHANGELOG.md` case-insensitively.
- Native agent JSON discovery is non-recursive at scan-path level.
- Companion file discovery is one level deep: `agents/prompts/x.md` is
  found, `agents/prompts/nested/y.md` is not.
- Mixed valid + invalid scan paths: valid still works.

For `parse_native_kiro_agent_file`:
- Valid Kiro JSON parses → bundle has correct `name`, populated
  `mcp_servers`, raw_json preserved.
- Missing `name` field → `NativeParseFailure::MissingName`.
- Unsafe `name` (`../`, leading `.`, slashes) →
  `NativeParseFailure::InvalidName` with newtype validator's reason.
- Malformed JSON → `NativeParseFailure::InvalidJson` with serde source
  preserved.
- I/O failure (e.g., permission denied) → `NativeParseFailure::IoError`.
- `mcpServers` with `Stdio` entry sets the `mcp_servers` field correctly
  (drives the install-layer MCP gate).
- `mcpServers` absent → empty `mcp_servers` field.

For `hash_artifact`:
- Same input → same hash (determinism).
- Different file order in input → same hash (sort-internal property).
- Identical content at different relative paths → different hashes (NUL
  separator prevents rename collisions).
- CRLF vs LF in same file → different hashes (no content normalization).
- Missing file in `relative_paths` list → `HashError::ReadFailed` with path.

### Tier 2: Project-layer install (integration with tempfile project dir)

For `KiroProject::install_native_agent`:
- Happy path: `.kiro/agents/<name>.json` lands. Both hash fields populated.
  Tracking entry has `dialect: "native"`.
- Idempotent reinstall (same plugin, same source_hash) → no file writes,
  tracking unchanged, success outcome with `was_idempotent: true`,
  `forced_overwrite: false`.
- Content-changed reinstall (same plugin, different source_hash) →
  `ContentChangedRequiresForce`, no file writes.
- Cross-plugin name collision → `NameClashWithOtherPlugin` with prior
  owner named, no files written.
- Orphan on disk → `OrphanFileAtDestination`, no files written.
- `--force` cross-plugin: ownership transfers, prior plugin's tracking entry
  removed, new plugin's tracking entry added. **Explicit assertion that
  installed-agents.json reflects the transfer.**
- `--force` orphan: file gets owned by the installing plugin, hashes
  populated.
- Atomicity under fault injection: if the prompt-write fails after the JSON
  is staged, no files appear at the destination.
- MCP gate: bundle with `mcpServers` containing Stdio is rejected without
  `--accept-mcp`.

For `KiroProject::install_native_companions`:
- Happy path: every companion file lands at its destination preserving the
  relative subdirectory structure. Tracking entry under
  `installed-agents.json`'s `native_companions` map lists every file.
- Idempotent bundle reinstall (same plugin, same bundle source_hash) →
  no file writes, success outcome with `was_idempotent: true`.
- Content-changed bundle (e.g., one prompt edited upstream) →
  `ContentChangedRequiresForce` for the entire bundle, no files written
  (atomic — all-or-nothing per bundle).
- Cross-plugin file overlap (Plugin A and Plugin B both ship
  `prompts/shared.md` but with different content) →
  `PathOwnedByOtherPlugin` for the conflicting file, no files written.
- Cross-plugin file overlap with `--force`: ownership transfers, prior
  plugin's `native_companions` entry loses the file (or the entire bundle
  entry if it had only that file).
- Orphan file at destination → `OrphanFileAtDestination`, no files written.
- Empty bundle (zero companion files): no tracking entry written, no error.

For `KiroProject::install_steering_file`:
- Same matrix (idempotent, content-changed, cross-plugin, orphan,
  `--force` ownership transfer).
- Tracking file (`installed-steering.json`) round-trips through serde.

### Tier 3: Service-layer dispatch and end-to-end

For `MarketplaceService`:
- `format: "kiro-cli"` routes to `install_native_kiro_cli_agents_inner`.
- `format` absent routes to existing translated path (existing tests still
  pass — backward compat assertion).
- `format: "kiro-ide"` (or other unknown) → manifest parse error with
  message naming the unknown variant.
- Mixed plugin (skills + native agents + steering): all three install,
  results aggregate per target.
- Native plugin with agents and companions: per-agent installs run first,
  then companions install once. Companion install runs even if some agents
  failed.
- Steering-only plugin (no agents declared) installs cleanly.
- Agents-only plugin (no steering declared) installs cleanly — no spurious
  `installed-steering.json` creation if zero steering files installed.

For `PluginManifest`:
- `format` absent / `"kiro-cli"` / unknown variant cases.
- `steering` absent → falls back to `DEFAULT_STEERING_PATHS`.
- `steering: []` → falls back to `DEFAULT_STEERING_PATHS` (mirrors existing
  `agents: []` semantics — confirm against
  `resolve_plugin_install_context_from_dir`).
- `steering: ["./custom/"]` → uses custom path.

### Adversarial cases

Per the project's `adversarial-tests` skill — drop-in tests for:
- Symlink at `agents/prompts/code-reviewer.md` pointing to `/etc/passwd` →
  discovery skips it, no copy.
- Concurrent install of two different plugins racing on the same companion
  path → `with_file_lock` serializes, second fails with
  `PathOwnedByOtherPlugin`.
- Native agent with `mcpServers` containing typo (`{ "type": "stdoi" }`) →
  parse error at the typed `McpServerConfig` boundary, not at install time.
- Hash primitive race: file modified between `hash_artifact` call and the
  rename — the post-stage `installed_hash` differs from the pre-stage
  `source_hash`, install proceeds (the file the user got is what they got),
  but the test asserts both hashes are populated correctly.

### Out of scope for this feature's tests

- Tauri command tests for steering / native agents — deferred until those
  commands land.
- `ProjectService`-extraction tests — deferred to the broader refactor.

## Implementation Phasing

The bundled design has three natural seams. Each is a complete, independently
shippable deliverable that doesn't leave the codebase in a half-state. If
implementation lands as one PR, great; if split, these are the cleanest cuts.

### Stage 1 — Hash primitive and tracking schema (the #27 scope)

**What lands:** `hash` module, `hash_artifact` + `hash_dir_tree`, hash field
additions to `InstalledSkillMeta` and `InstalledAgentMeta` schemas
(translated-agent path only at this stage), updated `install_skills` and
`install_translated_agents_inner` to populate hashes.

**What works after:** drift detection becomes possible for existing skills +
translated agents. No user-facing behavior change yet (no UI consuming the
hashes), but the foundation is in place. Existing tracking files keep loading
via `#[serde(default)]` defaults.

**Blocks:** Stages 2 and 3 both want `hash_artifact` to compute `source_hash`
for their new artifact types.

### Stage 2 — Native kiro-cli agent import

**What lands:** `PluginFormat::KiroCli` enum, manifest deserialization,
`AgentDialect::Native` variant, `discover_native_kiro_agents_in_dirs`,
`discover_native_companion_files`, `parse_native_kiro_agent_file`,
`NativeAgentBundle`, `KiroProject::install_native_agent`,
`KiroProject::install_native_companions`,
`InstalledNativeCompanionsMeta`, the new `AgentError` variants, dispatch in
`install_plugin_agents`, CLI install path tested against
`dwalleck/kiro-starter-kit`.

**What works after:** native Kiro plugins (without steering) install
end-to-end. The starter-kit's six reviewer agents land in `.kiro/agents/`
with their `prompts/` companion files at `.kiro/agents/prompts/`. Hash-based
idempotency works on reinstall.

**Missing:** the starter-kit's `steering/` directory is silently ignored.

### Stage 3 — Steering import

**What lands:** `steering/` module, `PluginManifest::steering` field,
`DEFAULT_STEERING_PATHS`, `installed-steering.json` tracking,
`SteeringError`, `KiroProject::install_steering_file`,
`MarketplaceService::install_plugin_steering`, CLI install command extended.

**What works after:** the starter-kit installs in full — agents AND
steering. Native plugins reach feature parity with translated plugins.

**Missing:** Tauri-side commands for native install / steering. Out of scope
per the broader Tauri-domain-grouping refactor decision.

### Why this ordering

- **Stage 1 first** because both Stages 2 and 3 want `hash_artifact` to
  populate `source_hash`. Without it, they'd write `None` everywhere and lose
  idempotency until Stage 1 retroactively turned it on.
- **Stage 2 before Stage 3** because the starter-kit's primary value is the
  agents (six specialist reviewers); steering is a layer on top. After Stage 2
  alone, a user installing the starter-kit gets working agents minus steering
  (degraded but useful). After Stage 3 alone (without 2), steering files
  would install with no agents to consume them.
- **Stages 2 and 3 are code-independent** — ordering is purely UX-driven.

If implementation pressure forces a smaller scope, **Stage 1 alone is
shippable as just-#27** — pure additive, zero risk to existing flows.

## Out of Scope

Explicitly **not** addressed by this design (each is its own future work):

- **Splitting `kiro-market-core` into ingestion + project layers.** Separate
  refactor. This design adds new methods to `KiroProject` (matching existing
  pattern), so the future extraction lifts them alongside the existing
  methods with no per-method redesign.
- **Introducing `ProjectService` / `SettingsService`.** Same reasoning.
- **Tauri commands for native agent / steering install.** Land when the Tauri
  side adopts the feature, following the existing `_impl` pattern.
- **CLI vs. Tauri command grouping harmonization.** Steering slots into both
  frontends following each one's current convention.
- **Reference-driven companion file copying.** v1 copies sibling
  subdirectories wholesale (filtered to one level deep, README/etc.
  excluded). A future version could parse each agent JSON's `file://`
  references and copy only what's referenced. Not needed for the starter-kit
  layout. The choice was deliberate: the user's stated preference was "just
  copy everything in the agents directory" and the plugin-scoped tracking
  model handles cross-plugin collisions cleanly without per-reference
  bookkeeping.
- **Rewriting `file://` URIs in agent JSON.** v1 preserves them verbatim.
  The bundle layout is preserved (`agents/<name>.json` references
  `file://./prompts/<name>.md` which lands at
  `.kiro/agents/prompts/<name>.md`, resolving correctly relative to the
  JSON's location).
- **Auto-namespacing on name collision** (e.g. install as
  `<plugin>__<agent>.json`). Tracking already carries `plugin: String`, so
  the rename logic is a future service-layer addition with no schema change.
- **Aligning translated-agent collision behavior with native** (today: skip;
  proposed for native: fail loudly). Translated path stays as-is; alignment
  is a separate decision.
- **Drift-check command consuming `installed_hash`.** Hash is populated for
  future use; the read-side command is its own design.
- **`format` values other than `"kiro-cli"`.** `KiroIde` reserved as a future
  variant; lands when the IDE-flavored agent format is defined.
- **Per-agent uninstall removing only that agent's referenced companions.**
  Companions are plugin-scoped; the unit of uninstall for companions is the
  whole plugin, not individual agents. Per-agent companion uninstall would
  require switching to the reference-driven model above.

## References

- Example native plugin: [`dwalleck/kiro-starter-kit`](https://github.com/dwalleck/kiro-starter-kit)
- Folded-in proposal: [kiro-control-center#27](https://github.com/dwalleck/kiro-control-center/issues/27)
- Existing translated-agent install: `crates/kiro-market-core/src/project.rs:470` (`install_agent_inner`)
- Existing tracking types: `crates/kiro-market-core/src/project.rs:32-71` (`InstalledSkillMeta`, `InstalledAgentMeta`, `InstalledSkills`, `InstalledAgents`)
- Discovery security primitives: `crates/kiro-market-core/src/agent/discover.rs:38` (`discover_agents_in_dirs`)
- Validation newtype precedent: `crates/kiro-market-core/src/validation.rs:28` (`RelativePath`)
- Tauri `_impl` pattern exemplar: `crates/kiro-control-center/src-tauri/src/commands/browse.rs::install_skills_impl`
- Plugin install context: `crates/kiro-market-core/src/service/browse.rs::resolve_plugin_install_context_from_dir`
- CLI install entry: `crates/kiro-market/src/commands/install.rs:188`

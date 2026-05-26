# Data Models

<!-- tags: types, models, json, data-structures -->

## Validation Newtypes (`validation.rs`)

All newtypes are only constructible via validated constructors. They serialize/deserialize transparently as strings.

| Type | Rejects | Used for |
|---|---|---|
| `PluginName` | empty, traversal (`..`), NUL, backslash, forward slash | Plugin identifiers |
| `MarketplaceName` | empty, traversal, NUL, backslash, forward slash | Marketplace identifiers |
| `AgentName` | empty, traversal, NUL, path separators | Agent identifiers |
| `RelativePath` | absolute paths, `..` components, backslashes, NUL, empty | Safe relative file paths |

`validate_name()` additionally rejects: Windows reserved names (`CON`, `NUL`, etc.), trailing dots, leading/trailing spaces, other control characters.

---

## Tracking Files (`.kiro/`)

### `installed-skills.json`

```json
{
  "skills": {
    "<skill-name>": {
      "marketplace": "string",
      "plugin": "string",
      "skill_dir": "relative/path",
      "scan_root": "relative/path",
      "source_hash": "blake3:hex...",
      "installed_hash": "blake3:hex...",
      "installed_at": "2026-01-01T00:00:00Z",
      "version": "string | null"
    }
  }
}
```

### `installed-agents.json`

```json
{
  "agents": {
    "<agent-name>": {
      "marketplace": "string",
      "plugin": "string",
      "dialect": "claude" | "copilot" | "native",
      "source_path": "relative/path",
      "source_hash": "blake3:hex...",
      "installed_hash": "blake3:hex...",
      "installed_at": "2026-01-01T00:00:00Z",
      "version": "string | null",
      "companion_files": ["relative/path", ...]
    }
  },
  "native_companions": {
    "<plugin@marketplace>": {
      "files": ["relative/path", ...],
      "source_scan_root": "relative/path",
      "source_hash": "blake3:hex...",
      "installed_hash": "blake3:hex..."
    }
  }
}
```

### `installed-steering.json`

```json
{
  "files": {
    "relative/path/to/file.md": {
      "marketplace": "string",
      "plugin": "string",
      "source_scan_root": "relative/path",
      "source_hash": "blake3:hex...",
      "installed_hash": "blake3:hex...",
      "installed_at": "2026-01-01T00:00:00Z",
      "version": "string | null"
    }
  }
}
```

---

## Cache Files (`~/.cache/kiro-market/`)

### Known Marketplaces Registry

```json
{
  "marketplaces": {
    "<name>": {
      "source": "github:owner/repo" | "https://..." | "local:/path",
      "protocol": "https" | "ssh"
    }
  }
}
```

### Plugin Registry (`registries/<marketplace>.json`)

```json
{
  "plugins": [
    {
      "name": "string",
      "path": "./relative/path",
      "source": { "type": "github", "owner": "...", "repo": "..." } | null
    }
  ]
}
```

---

## Plugin Manifest (`plugin.json`)

```json
{
  "name": "string",
  "description": "string | null",
  "version": "string | null",
  "format": "translated" | "kiro-cli",
  "skills": ["./path/to/skill/dir", ...],
  "agents": ["./path/to/agent.md", ...],
  "steering": ["./path/to/steering.md", ...]
}
```

`format` defaults to `"translated"` when absent. `skills`/`agents`/`steering` default to empty (triggering default scan paths).

---

## Marketplace Manifest (`.claude-plugin/marketplace.json`)

```json
{
  "plugins": [
    {
      "name": "string",
      "path": "./relative/path",
      "source": {
        "type": "github" | "git_url" | "git_subdir",
        ...
      }
    }
  ]
}
```

---

## SKILL.md Frontmatter

```yaml
---
name: skill-name
description: "Human-readable description"
invocable: true | false   # optional
---
```

---

## Agent Types (`agent/types.rs`)

```rust
pub struct AgentDefinition {
    pub name: AgentName,
    pub description: Option<String>,
    pub model: Option<String>,
    pub tools: Vec<MappedTool>,
    pub mcp_servers: Vec<McpServerConfig>,
    pub dialect: AgentDialect,
    pub body: String,
}

pub enum AgentDialect { Claude, Copilot, Native }

pub struct McpServerConfig {
    pub name: String,
    pub transport: McpTransport,  // Stdio { command, args, env } | Http { url }
}
```

---

## Kiro Agent JSON (installed at `.kiro/agents/<name>.json`)

```json
{
  "name": "string",
  "description": "string",
  "model": "string | null",
  "prompt": "file:///relative/path/to/prompt.md",
  "tools": ["tool-name", ...],
  "allowedTools": ["tool-name", ...],
  "mcpServers": {
    "<server-name>": {
      "type": "stdio",
      "command": "string",
      "args": ["..."],
      "env": {}
    }
  }
}
```

---

## BlakeHash (`hash.rs`)

Validated hex string with `blake3:` prefix. Format: `blake3:<64-hex-chars>`. Normalized to lowercase. Constructed via `BlakeHash::new(s)` or `BlakeHash::from_blake3_digest(digest)`. A placeholder value exists for cases where hashing is deferred.

---

## Error Hierarchy (`error.rs`)

```
Error
├── ValidationError (path traversal, invalid name, etc.)
├── GitError
│   ├── CloneFailed { url, gix_err, cli_err }
│   ├── PullFailed { source }
│   ├── OpenFailed { source }
│   ├── CommandFailed { stderr }
│   ├── NotFound
│   └── ShaMismatch { expected, actual, reason: InvalidShaReason }
├── PluginError
│   ├── NotFound
│   ├── ManifestReadFailed
│   ├── InstallFailed
│   ├── OrphanFileAtDestination
│   ├── ContentChangedRequiresForce
│   ├── PathOwnedByOtherPlugin
│   └── NameClashWithOtherPlugin
├── MarketplaceError
│   ├── NotFound
│   ├── AlreadyExists
│   ├── RemoteSourceNotLocal
│   └── ...
├── AgentError
├── SkillError
└── (serde_json, io wrapping variants)
```

`remediation_hint()` returns an optional actionable string for CLI display (e.g., "use --force to overwrite").

---

## CommandError (Tauri, `src-tauri/src/error.rs`)

```rust
pub struct CommandError {
    pub r#type: ErrorType,
    pub message: String,
}

pub enum ErrorType {
    Validation,
    NotFound,
    ParseError,
    Internal,
    Unknown,
}
```

Serializes with snake_case discriminants. `From` impls for `Error`, `ValidationError`, `serde_json::Error`, `String`.

---

## Settings Types

### App Settings (`commands/settings.rs`)

```rust
pub struct Settings {
    pub scan_roots: Vec<String>,
    pub active_project: Option<String>,
}
```

### Kiro Setting Entry (`kiro_settings.rs`)

```rust
pub struct SettingEntry {
    pub key: String,
    pub category: SettingCategory,
    pub label: String,
    pub value_type: SettingType,   // Bool, String, Number, Array
    pub current_value: Option<serde_json::Value>,
    pub default_value: Option<serde_json::Value>,
}
```

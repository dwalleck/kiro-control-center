# Documentation Index

> **For AI Assistants**: This file is your primary entry point. Read it first to understand what documentation is available and which file to consult for specific questions. Each entry below includes a summary so you can determine relevance without reading every file.

## How to Use This Documentation

1. **Start here** — scan the summaries below to find the right file for your question
2. **Go deeper** — read the specific file for detailed information
3. **Cross-reference** — files link to each other where topics overlap

## Documentation Files

| File | Purpose | Consult When... |
|------|---------|-----------------|
| [codebase_info.md](codebase_info.md) | Project identity, tech stack, workspace layout | You need versions, crate names, or build targets |
| [architecture.md](architecture.md) | System design, layering, data flow | You need to understand how components interact |
| [components.md](components.md) | Major modules and their responsibilities | You need to find where specific logic lives |
| [interfaces.md](interfaces.md) | Public APIs, IPC commands, CLI surface | You need to understand how frontends talk to core |
| [data_models.md](data_models.md) | Key data structures and serialization | You need to understand types, JSON schemas, or state |
| [workflows.md](workflows.md) | End-to-end processes (install, add, etc.) | You need to trace a user action through the system |
| [dependencies.md](dependencies.md) | External crates and their roles | You need to understand why a dependency exists |
| [review_notes.md](review_notes.md) | Documentation gaps and inconsistencies | You need to know what's underdocumented |

## Quick Reference

### Where does business logic live?
All in `crates/kiro-market-core/src/`. CLI and desktop app are thin wrappers. See [components.md](components.md).

### How do I add a new Tauri command?
Add the handler in `src-tauri/src/commands/`, register in `lib.rs`, regenerate bindings. See [interfaces.md](interfaces.md).

### How does skill installation work?
`MarketplaceService` → `KiroProject` → disk writes with file locking. See [workflows.md](workflows.md).

### What are the security constraints?
Path traversal prevention, MCP opt-in, TLS-by-default, no unsafe code. See [architecture.md](architecture.md).

### What external services does this depend on?
Git repositories only (no databases, no cloud APIs). See [dependencies.md](dependencies.md).

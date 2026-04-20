# Documentation Index

> **For AI Assistants**: This file is your primary entry point. Read this first to understand what documentation is available and where to find specific information. Each section below describes a documentation file, its purpose, and when to consult it.

## How to Use This Documentation

1. **Start here** — scan the summaries below to identify which file(s) are relevant to your task
2. **Read targeted files** — open only the files that match your needs
3. **Cross-reference** — files link to each other where topics overlap

## Documentation Files

### [codebase_info.md](./codebase_info.md)
**Purpose**: Project identity, technology stack, workspace structure, build commands, and configuration files.

**Consult when**: You need to know what language/framework is used, how to build/test, what the project does at a high level, or where configuration lives.

**Key content**: Project name and repo URL, tech stack table, workspace member list, build/test/lint commands, configuration file inventory.

---

### [architecture.md](./architecture.md)
**Purpose**: System design, architectural patterns, security model, and cross-cutting concerns.

**Consult when**: You need to understand how components relate, why something is designed a certain way, what security invariants exist, or how errors flow through the system.

**Key content**: Layered architecture diagram, shared-core pattern, service layer design, file-based state with locking, RAII cleanup guards, dual git backend, typed IPC via specta, feature flags, path traversal prevention, MCP gating, TLS enforcement, platform abstraction.

---

### [components.md](./components.md)
**Purpose**: Detailed breakdown of every major component, its responsibilities, and key types.

**Consult when**: You need to understand what a specific module does, what types it exposes, or how the CLI/GUI commands map to backend logic.

**Key content**: Core library modules (service, cache, project, git, plugin, skill, agent, validation, settings, platform, file_lock, raii), CLI command modules, Tauri command modules, Svelte frontend components and state management.

---

### [interfaces.md](./interfaces.md)
**Purpose**: All APIs, IPC contracts, file format specifications, and integration points.

**Consult when**: You need to know the shape of a CLI command, Tauri IPC function signature, file format schema, or how data flows between layers.

**Key content**: CLI command reference, Tauri TypeScript API signatures, core library public API (Rust), marketplace/plugin/skill/agent file format specs, cache directory structure, tracking file schemas.

---

### [data_models.md](./data_models.md)
**Purpose**: All data structures, their relationships, and type hierarchies.

**Consult when**: You need to understand a specific type, its fields, or how types relate to each other across the system.

**Key content**: Mermaid class diagrams for marketplace layer, cache layer, project layer, plugin/skill layer, agent layer, service result types, error types, settings types, and frontend TypeScript interfaces.

---

### [workflows.md](./workflows.md)
**Purpose**: Step-by-step process flows for all major operations.

**Consult when**: You need to understand what happens during a specific operation (install, add marketplace, update, prune), the CI pipeline structure, or the development workflow with Claude hooks.

**Key content**: Sequence diagrams for marketplace registration, skill installation, agent installation, marketplace update, cache pruning, desktop app initialization, git clone dual-backend, CI pipeline, and Claude hook workflow.

---

### [dependencies.md](./dependencies.md)
**Purpose**: All external dependencies, their versions, purposes, and notable decisions.

**Consult when**: You need to know why a dependency exists, what version is used, or understand dependency-related design decisions (curl shim, gix vs git2, Svelte 5 runes).

**Key content**: Workspace Rust deps table, Tauri crate deps, npm production/dev deps, dependency relationship diagram, notable decisions (curl TLS shim, gix choice, Svelte 5 runes, static adapter).

---

### [review_notes.md](./review_notes.md)
**Purpose**: Documentation quality assessment — inconsistencies found and completeness gaps.

**Consult when**: You want to know what areas of the codebase are under-documented or where documentation may be inaccurate.

## Quick Reference

| Question | File |
|----------|------|
| How do I build/test this project? | codebase_info.md |
| What does module X do? | components.md |
| What's the API for function Y? | interfaces.md |
| What type is field Z? | data_models.md |
| What happens when I run command W? | workflows.md |
| Why is dependency D used? | dependencies.md |
| How are components connected? | architecture.md |
| What's missing from the docs? | review_notes.md |

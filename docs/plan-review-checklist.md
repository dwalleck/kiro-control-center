# Plan Review Checklist

A standing checklist applied at plan-review-self time (between writing
a plan and starting implementation) and at PR-review time (against any
plan that produced the PR).

Originated from the PR #64 retrospective. The April 23 plan-review pass
caught 17 grounding issues before any code was written — the gates here
codify *what* that pass was looking for so future plan-reviews don't
have to rediscover the categories. Reading order: this doc complements
the upstream `superpowers:writing-plans` skill; apply these gates after
the skill's own self-review step.

## When to apply

- After writing a plan, before opening it for code: run all five gates
  yourself and patch the plan with an amendments doc (see
  `docs/plans/2026-04-24-stage2-3-plan-amendments.md` for the precedent
  format).
- During PR review on any change touching the public API of
  `kiro-market-core`: the gates also fire as code-review questions.
- When refactoring an existing plan because implementation surfaced
  drift: rewrite the plan section that failed a gate and link forward
  to the corrected version.

Each gate has a **what to check**, a **where to look**, and a
**fail signature** — what a violation looks like in plan text or in
code.

---

## Gate 1 — Grounding

**What to check.** Every API the plan references actually exists at
the SHA the plan was written against. The plan's "use existing X"
claims are real, not aspirational.

**Where to look.**
- `grep` every fully-qualified path the plan names against the current
  source tree.
- Open every "see existing function `foo`" reference and confirm the
  signature matches what the plan assumes.
- Check that imports / module paths in plan code samples resolve.

**Fail signature.** The April 23 review caught these (they appear in
`docs/plans/2026-04-23-plan-review-findings.md`):

- Plan referenced `crate::service::DiscoveryWarning` — type didn't exist.
- Plan referenced `test_marketplace_service()` — actual helper was
  `temp_service()`.
- Plan referenced `uuid_or_pid()` — never defined anywhere.
- Plan said `install_plugin_agents` takes one signature; actual
  signature was different.

The mitigation is mechanical: a `grep` pass against the current SHA
before declaring the plan reviewed.

---

## Gate 2 — Threat Model

**What to check.** For every byte that enters the system from outside
the trust boundary, the plan names the source, the attacker
capabilities, and the per-source defenses. "Out of scope: security" is
a fail unless paired with an explicit threat-model statement.

**Where to look.**
- Enumerate untrusted byte sources: manifest JSON / YAML, plugin file
  trees on local disk, cloned git repositories, downloaded archives,
  tracking files written by prior installs (treat as
  potentially-tampered).
- For each source, list the attacker capabilities: can they write to
  the source dir? race a file replacement? supply arbitrary file modes
  / hardlinks / symlinks?
- For each (source × capability), list the defense: pre-allocation
  size cap, control-byte rejection, hardlink refusal, symlink refusal,
  TOCTOU re-check, name validation, path-traversal validation,
  reparse-point check.

**Fail signature.** PR #64 plans listed only "skip symlinks at
discovery" and an explicit "TOCTOU is documented as deferred" line.
Post-merge security review found:

- No hardlink refusal anywhere — `nlink > 1` would let a `LocalPath`
  attacker exfiltrate `~/.ssh/id_rsa` into `.kiro/agents/`.
- No size cap — a hostile multi-GB `agents/big.json` could OOM the
  parser.
- No NUL-byte rejection in JSON — keys like `"tool\0evil"` would
  truncate to `"tool"` in C-string-consuming downstream tooling.
- No tracking-file path validation — a tampered
  `installed-agents.json` with `prompts/../../etc/passwd` entries
  would escape the base join at hash-recompute or removal time.
- Staging-path symlink TOCTOU at two install sites (the
  `md.is_file()` gate skips symlinks and `fs::read` follows them).

Each of these shipped as a follow-up commit AFTER the planned work
landed. Catching them at plan-review time is cheaper.

---

## Gate 3 — Wire Format / FFI Shape

**What to check.** Every type the plan adds with `#[derive(Serialize)]`
plus `#[cfg_attr(feature = "specta", derive(specta::Type))]` crosses
the Tauri FFI. Each field's *meaning* needs to be explicit, not just
its type. Specifically: when the field is constructed in two paths,
both paths must produce semantically equivalent values.

**Where to look.**
- List every type touched by the plan that derives `Serialize` (or
  could become serializable downstream).
- For each field, name what the frontend sees and confirm that's what
  the plan intends.
- Audit any classifier / factory that constructs the type from
  multiple branches — does each branch fill every field correctly, or
  does one branch fall back to a default that's semantically wrong?

**Fail signature.** PR #64 had `InstalledSteeringOutcome { source,
destination, kind, source_hash, installed_hash }`. The classifier
`classify_steering_collision` constructed this but didn't receive the
source path as input — it filled `source: dest.to_path_buf()`. The
non-idempotent path constructed the same type one function up,
correctly using `source.source.clone()`. The wire format silently
served the destination path on idempotent reinstalls.

The mitigation: when adding a serializable type, do a
"who-constructs-this" inventory. If two construction sites have
different inputs in scope, split the type — return a minimal echo
from the data-poor site and assemble the full type at the data-rich
site. PR #64 ended up doing this with `SteeringIdempotentEcho`.

---

## Gate 4 — External Type Boundary

**What to check.** No field of any `pub` type in `kiro-market-core`
carries an external crate's error type via `#[source]`. CLAUDE.md's
"map external errors at the adapter boundary" rule is tested by a
SQL gate at plan-review time, not by waiting for a reviewer agent to
flag it.

**Where to look.**
- `cargo xtask plan-lint --gate gate-4-external-error-boundary` —
  this queries the tethys index for any `pub` enum variant carrying
  an external crate's error type (`serde_json`, `gix`, `reqwest`,
  `toml`) via `#[source]`. The earlier shape of this gate was a
  `grep` against source files, but standard `grep` doesn't match
  multi-line patterns and silently produced zero matches even when
  violations existed; the SQL gate is the supported form.
- Any planned variant matching this pattern needs the
  `#[non_exhaustive]` enum + `reason: String` field +
  `pub(crate) fn` constructor recipe (CLAUDE.md "Map external errors
  at the adapter boundary" — canonical examples
  `error::native_manifest_parse_failed` and
  `steering::tracking_malformed`).
- The corresponding test must assert `err.source().is_none()` to lock
  the contract.

**Fail signature.** PR #64 planned `AgentError::NativeManifestParseFailed
{ #[source] source: serde_json::Error }` and
`SteeringError::TrackingMalformed { #[source] source: serde_json::Error
}`. Both leaked `serde_json` through the public API. Both shipped
that way and were fixed in follow-up commits with the
constructor-pattern recipe. A plan-lint Gate 4 run would have caught
both at plan-review time.

---

## Gate 5 — Type Design

**What to check.**

1. **Parse, don't validate** — applied to every untrusted string
   field in a manifest. A plan that says "validate the name field"
   is a fail; the correct shape is a newtype with a fallible `new` and
   a `Deserialize` routing through `new`. Templates: `RelativePath`,
   `GitRef`, `AgentName` — note `GitRef` is a parse-don't-validate
   template only and lacks the specta derive; for the specta cfg-attr
   in item 2, follow `RelativePath` instead.

   *Exception:* a transient projection struct may keep a raw
   `Option<String>` field when post-parse routing needs to split
   failures across distinct error variants — e.g.
   `NativeAgentProjection.name: Option<String>` so `MissingName` /
   `InvalidName(reason)` / `InvalidJson` route to three distinct
   `AgentError` variants instead of collapsing into
   `InvalidJson(serde_json::Error)`. The type-level guarantee must
   still land at the *bundle* boundary
   (`NativeAgentBundle.name: AgentName`). See the parse-don't-validate
   exception in CLAUDE.md.
2. **Specta cfg-attr on validation newtypes** — every newtype that
   *could* end up in a Tauri-reachable type needs
   `#[cfg_attr(feature = "specta", derive(specta::Type))]` from day
   one. Adding it later is fine but a missed-on-creation case becomes
   a latent break.
3. **Classifier exhaustiveness** — every classifier function over an
   error enum matches every variant explicitly (no `_ => default`).
   When the plan adds a new variant, it must list which classifiers
   need a new arm.
4. **Classifier idempotent-payload rule** — when a classifier returns
   `CollisionDecision::Idempotent(Box<T>)`, every field of `T` must be
   data the classifier *actually receives as input*, not data the
   caller has but didn't pass in. If a field can't be filled from the
   classifier's inputs, split the type: return a minimal echo from the
   classifier (e.g. `SteeringIdempotentEcho { prior_installed_hash:
   String }`) and have the caller assemble the full outcome where the
   missing data is in scope. PR #64's steering classifier shipped with
   `T = InstalledSteeringOutcome` and substituted `dest` for the
   missing `source` path, leaking the destination into the wire-format
   `source` field on idempotent reinstalls — this rule is named in
   CLAUDE.md ("Classifier idempotent-payload rule") because the bug
   shape is repeatable. Cross-references Gate 3 (wire-format payload
   correctness) but lives here because the fix is type-design.
5. **`InstallOutcomeKind`-style enum vs. boolean pair** — when the
   plan introduces multiple boolean flags that describe the same axis
   (e.g. `was_idempotent` + `forced_overwrite`), check whether the
   `(true, true)` state is meaningful. If not, replace with a 3- or
   4-variant enum so unrepresentable states are unrepresentable.

**Where to look.**
- For every untrusted string field in a planned schema: is there a
  newtype, and does the type the field is declared as have a fallible
  constructor?
- For every planned error enum variant: which existing classifier
  functions need an arm?
- For every planned `classify_*_collision` returning
  `Idempotent(Box<T>)`: do a who-constructs-this inventory and confirm
  every field of `T` is data the classifier actually receives as
  input. If two construction sites of `T` have different inputs in
  scope, split the type.
- For every planned outcome / status struct: are there ≥2 boolean
  fields that could collapse into an enum?

**Fail signature.** PR #64 plans had:

- `NativeAgentBundle.name: String` validated post-parse via free
  `validate_name` — fixed in review by introducing the `AgentName`
  newtype.
- `classify_steering_collision` returning
  `Idempotent(Box<InstalledSteeringOutcome>)` and substituting `dest`
  for the missing `source` path — leaked the destination into the
  wire-format `source` field on idempotent reinstalls, fixed in
  follow-up by splitting the type into `SteeringIdempotentEcho` (echo
  from the classifier) plus full assembly at the data-rich caller.
- `InstalledSteeringOutcome { was_idempotent: bool, forced_overwrite:
  bool }` with a meaningless `(true, true)` state — fixed mid-plan
  (Issue #59) with `InstallOutcomeKind` 3-variant enum.

---

## Output of a gate failure

When a gate fires during plan review, write an amendments doc following
the precedent of `docs/plans/2026-04-24-stage2-3-plan-amendments.md`:

- Date-stamped (`YYYY-MM-DD-<feature>-plan-amendments.md`).
- One section per amendment, numbered (S2-1, S3-1, ...) so commits
  can reference them.
- Each amendment cites: the gate that fired, the original plan text,
  the amended plan text, the rationale.
- Save next to the original plans so they're discoverable together.

When a gate fires during PR review, file the fix as a follow-up commit
referencing the gate by name in the commit message
(`review(core): apply gate-3 wire-format audit — fix
InstalledSteeringOutcome.source bug`).

---

## Cross-references

- Upstream skill: `superpowers:writing-plans` — invoke FIRST; this
  checklist is the project-specific addendum.
- Pre-implementation precedent:
  `docs/plans/2026-04-23-plan-review-findings.md` — the original
  17-finding pass that motivated this checklist.
- Mid-plan amendment precedent:
  `docs/plans/2026-04-24-stage2-3-plan-amendments.md` — the format
  for writing amendments when gates fire.
- Patterns referenced in plans (P-1 through P-6): see the amendments
  doc; project-conventions (newtype recipe, classifier exhaustiveness,
  external-error boundary recipe, etc.) live in `CLAUDE.md`.

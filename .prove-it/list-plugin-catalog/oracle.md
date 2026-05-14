# Oracle — list_plugin_catalog_for_marketplace

## Probe
`crates/kiro-market-core/tests/prove_it_list_plugin_catalog.rs` —
command-driven assembly of `PluginCatalogEntry { skills, steering, agents }`
using `MarketplaceService::list_skills_for_plugin`,
`steering::discover::discover_steering_files_in_dirs`,
`agent::discover::discover_agents_in_dirs`, and the project's
tracking-file loaders (`KiroProject::load_installed{,_steering,_agents}`).

## Oracle
Independent filesystem walk of `<project>/.kiro/{skills,steering,agents}/`
via `fs::read_dir` (`oracle_kiro_subdir`). The oracle uses no service-layer
machinery, no manifest parsing, and no tracking-file knowledge — it answers
the orthogonal question "what is physically present in the install
destination?"

## Agreement slices (all 4 tests pass)

1. **`probe_skills_consistent_empty_state_agrees`** — empty project,
   populated marketplace. Probe enumerates 3 skills, all `installed=false`.
   Oracle returns `{}`. Both installed-sets equal `{}`. **Agreement on
   join-key (skill name) and cardinality.**

2. **`probe_skills_orphan_disk_dir_diverges`** — same setup, plus a
   manually-created `.kiro/skills/s1/` with no tracking entry.
   Probe says installed-set = `{}`. Oracle says `{s1}`.
   **Intentional divergence — pins the contract.**

3. **`probe_steering_no_service_layer_enumeration_exists`** — assembled
   `(name, installed)` manually by joining `discover_steering_files_in_dirs`
   output with `InstalledSteering.files`. Both files enumerated; consistent-
   empty agreement.

4. **`probe_agents_no_service_layer_enumeration_exists`** — `discover_agents_in_dirs`
   returns paths only. The probe **cannot** produce `(name, installed)` from
   discovery alone — agent names live inside the file, requiring the parser
   pipeline. Empty case still agrees, but the gap is documented.

## What this validates / falsifies

**Validated** — for skills, `list_skills_for_plugin` returns exactly the
shape the design needs. The bulk version is a structural lift, not a
new computation.

**Falsified** — the assumption that "steering and agents work the same way"
is wrong in degree. Steering needs only a thin service-layer wrapper around
existing primitives. Agents need a wrapper that *also* invokes the agent
parser to extract names from each discovered file.

**Pinned** — `installed: bool` on every category is **tracking-file
membership only**. A user who hand-deletes `.kiro/skills/<name>/` will see
"Installed" in the UI until tracking is updated. The drawer's UX should
either accept this contract (cheap) or add an explicit cross-check (truth).

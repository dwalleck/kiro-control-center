# What I learned (one sentence per finding — all unknown before the probe)

1. **`SkillInfo.installed` is tracking-file membership, not disk presence**
   (`browse.rs:896`: `installed.skills.contains_key(&frontmatter.name)`) —
   so the new bulk command will silently disagree with the filesystem in
   orphan-tracking and orphan-disk states, and the drawer's button matrix
   inherits that.

2. **The "drop in a single bulk command" framing of slice 1 is wrong** —
   no service-layer enumeration exists for steering or agents; the design
   needs three new methods (`list_steering_for_plugin`, `list_agents_for_plugin`,
   plus the bulk wrapper) before the Tauri command can be a thin shim, and
   the agents one must invoke the parser to extract names from
   `discover_agents_in_dirs`'s path output.

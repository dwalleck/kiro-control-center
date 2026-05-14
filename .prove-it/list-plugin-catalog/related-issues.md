# Tracker scan — list_plugin_catalog_for_marketplace

Searched rivets (18 issues, all listed) for: catalog, bulk, list_plugin, tracking,
browse, drawer, installed flag, fan-out, steering enum, agent enum, BrowseTab
redesign, three-state.

**No prior art.** Closest adjacent issues all relate to install-time error
structuring (kiro-19zq family — F1-F5 follow-ups from PR #113), not the
read-side bulk-catalog work this probe is investigating.

Bug-discovery rule: if the probe surfaces a real divergence (tracking-file ↔
disk ↔ command-reported `installed` flag), file a fresh rivets issue rather
than papering over.

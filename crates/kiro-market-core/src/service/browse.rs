//! Browse-side service methods: enumerate skills across marketplaces and
//! plugins, cross-referenced with the target project's installed set.
//!
//! Frontends (CLI, Tauri) remain thin wrappers — they decide how to
//! construct the [`MarketplaceService`] and how to frame errors, but
//! they do not duplicate the enumeration loop or the per-skill
//! frontmatter-parsing logic.

use serde::Serialize;

// ---------------------------------------------------------------------------
// Response types
// ---------------------------------------------------------------------------

/// Information about a single skill, cross-referenced with the target
/// project's installed set.
///
/// `installed` is a point-in-time snapshot — the project's
/// `.kiro/installed.json` at the moment the listing was built. Callers
/// that want a live view must re-query.
#[derive(Clone, Debug, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct SkillInfo {
    pub name: String,
    pub description: String,
    pub plugin: String,
    pub marketplace: String,
    pub installed: bool,
}

/// Result of a marketplace-wide skill listing. The bulk path continues
/// past per-plugin errors (missing directory, malformed manifest) to
/// preserve the partial listing; `skipped` records those errors so the
/// frontend can show a warning rather than silently dropping plugins.
#[derive(Clone, Debug, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct BulkSkillsResult {
    pub skills: Vec<SkillInfo>,
    pub skipped: Vec<SkippedPlugin>,
}

/// A plugin that was excluded from a bulk listing, with the reason.
#[derive(Clone, Debug, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct SkippedPlugin {
    pub name: String,
    pub reason: String,
}

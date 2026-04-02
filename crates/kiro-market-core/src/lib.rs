//! Core library for kiro-market.
//!
//! Provides types and logic for discovering and installing Claude Code
//! marketplace skills into Kiro CLI projects.

pub mod cache;
pub mod error;
pub mod git;
pub mod marketplace;
pub mod plugin;
pub mod project;
pub mod skill;
pub mod validation;

/// Path to the marketplace manifest within a marketplace repository.
pub const MARKETPLACE_MANIFEST_PATH: &str = ".claude-plugin/marketplace.json";

/// Default skill scan paths when a plugin has no `plugin.json` or its skills
/// list is empty.
pub const DEFAULT_SKILL_PATHS: &[&str] = &["./skills/"];

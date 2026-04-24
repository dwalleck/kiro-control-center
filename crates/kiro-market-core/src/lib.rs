//! Core library for kiro-market.
//!
//! Provides types and logic for discovering and installing Claude Code
//! marketplace skills into Kiro CLI projects.

pub mod agent;
pub mod cache;
pub mod error;
pub mod file_lock;
pub mod git;
pub mod hash;
pub mod kiro_settings;
pub mod marketplace;
pub mod platform;
pub mod plugin;
pub mod project;
pub(crate) mod raii;
pub mod service;
pub mod skill;
#[cfg(any(test, feature = "test-support"))]
pub mod test_utils;
pub mod validation;

/// Path to the marketplace manifest within a marketplace repository.
pub const MARKETPLACE_MANIFEST_PATH: &str = ".claude-plugin/marketplace.json";

/// Default skill scan paths when a plugin has no `plugin.json` or its skills
/// list is empty.
pub const DEFAULT_SKILL_PATHS: &[&str] = &["./skills/"];

/// Default agent scan paths when a plugin has no `plugin.json` or its
/// `agents` list is empty.
pub const DEFAULT_AGENT_PATHS: &[&str] = &["./agents/"];

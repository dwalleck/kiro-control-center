//! Tauri commands for reading and writing Kiro CLI settings.

use std::path::PathBuf;

use kiro_market_core::kiro_settings::{
    default_kiro_dir, load_kiro_settings_from, registry, remove_nested, resolve_settings,
    save_kiro_settings_to, set_nested, SettingEntry,
};
use serde_json::Value as JsonValue;

use crate::error::{CommandError, ErrorType};

// ---------------------------------------------------------------------------
// Helper
// ---------------------------------------------------------------------------

/// Resolve the Kiro home directory (`~/.kiro`), returning a [`CommandError`]
/// if the home directory cannot be determined.
fn kiro_dir() -> Result<PathBuf, CommandError>
{
    default_kiro_dir().ok_or_else(|| {
        CommandError::new(
            "could not determine Kiro home directory",
            ErrorType::IoError,
        )
    })
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// Load all Kiro CLI settings, merging the stored JSON with the registry defaults.
#[tauri::command]
#[specta::specta]
#[allow(clippy::unused_async)]
pub async fn get_kiro_settings() -> Result<Vec<SettingEntry>, CommandError>
{
    let dir = kiro_dir()?;
    let json = load_kiro_settings_from(&dir);
    Ok(resolve_settings(&json))
}

/// Update a single Kiro CLI setting by key and return the updated entry.
#[tauri::command]
#[specta::specta]
#[allow(clippy::unused_async)]
pub async fn set_kiro_setting(key: String, value: JsonValue) -> Result<SettingEntry, CommandError>
{
    let reg = registry();
    if !reg.iter().any(|def| def.key == key)
    {
        return Err(CommandError::new(
            format!("unknown setting key: {key}"),
            ErrorType::Validation,
        ));
    }

    let dir = kiro_dir()?;
    let mut json = load_kiro_settings_from(&dir);
    set_nested(&mut json, &key, value);
    save_kiro_settings_to(&dir, &json).map_err(|e| {
        CommandError::new(format!("failed to save settings: {e}"), ErrorType::IoError)
    })?;

    let entry = resolve_settings(&json)
        .into_iter()
        .find(|e| e.key == key)
        .expect("key was validated against registry above");

    Ok(entry)
}

/// Remove a single Kiro CLI setting by key, reverting it to its default.
#[tauri::command]
#[specta::specta]
#[allow(clippy::unused_async)]
pub async fn reset_kiro_setting(key: String) -> Result<(), CommandError>
{
    let reg = registry();
    if !reg.iter().any(|def| def.key == key)
    {
        return Err(CommandError::new(
            format!("unknown setting key: {key}"),
            ErrorType::Validation,
        ));
    }

    let dir = kiro_dir()?;
    let mut json = load_kiro_settings_from(&dir);
    remove_nested(&mut json, &key);
    save_kiro_settings_to(&dir, &json).map_err(|e| {
        CommandError::new(format!("failed to save settings: {e}"), ErrorType::IoError)
    })?;

    Ok(())
}

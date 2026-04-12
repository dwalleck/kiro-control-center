//! Tauri commands for reading and writing Kiro CLI settings.

use std::path::PathBuf;
use std::sync::Mutex;

use kiro_market_core::kiro_settings::{
    default_kiro_dir, load_kiro_settings_from, registry, remove_nested, resolve_settings,
    save_kiro_settings_to, set_nested, LoadSettingsError, SettingEntry,
};
use serde_json::Value as JsonValue;

use crate::error::{CommandError, ErrorType};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Serialize settings writes to prevent lost-update races from rapid UI
/// interactions (e.g. toggling multiple booleans quickly).
static SETTINGS_WRITE_LOCK: Mutex<()> = Mutex::new(());

/// Resolve the Kiro home directory (`~/.kiro`), returning a [`CommandError`]
/// if the home directory cannot be determined.
fn kiro_dir() -> Result<PathBuf, CommandError> {
    default_kiro_dir().ok_or_else(|| {
        CommandError::new(
            "could not determine Kiro home directory",
            ErrorType::IoError,
        )
    })
}

/// Load the settings JSON, treating a missing file as empty defaults but
/// propagating corrupt-file and I/O errors to the caller.
fn load_settings(dir: &std::path::Path) -> Result<JsonValue, CommandError> {
    match load_kiro_settings_from(dir) {
        Ok(json) => Ok(json),
        Err(LoadSettingsError::NotFound) => Ok(serde_json::json!({})),
        Err(LoadSettingsError::InvalidJson(e)) => Err(CommandError::new(
            format!(
                "settings file contains invalid JSON and cannot be safely updated: {e}. \
                 Back up or delete ~/.kiro/settings/cli.json and try again."
            ),
            ErrorType::ParseError,
        )),
        Err(LoadSettingsError::Io(e)) => Err(CommandError::new(
            format!("could not read settings file: {e}"),
            ErrorType::IoError,
        )),
    }
}

/// Validate that a setting key is non-empty and exists in the registry.
/// Returns the index into the registry for the matched definition.
fn validate_key(key: &str) -> Result<usize, CommandError> {
    if key.is_empty() {
        return Err(CommandError::new(
            "setting key must not be empty",
            ErrorType::Validation,
        ));
    }

    let reg = registry();
    reg.iter()
        .position(|def| def.key == key)
        .ok_or_else(|| {
            CommandError::new(
                format!("unknown setting key: {key}"),
                ErrorType::Validation,
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
pub async fn get_kiro_settings() -> Result<Vec<SettingEntry>, CommandError> {
    let dir = kiro_dir()?;
    let json = load_settings(&dir)?;
    Ok(resolve_settings(&json))
}

/// Update a single Kiro CLI setting by key and return the updated entry.
#[tauri::command]
#[specta::specta]
#[allow(clippy::unused_async)]
pub async fn set_kiro_setting(
    key: String,
    value: JsonValue,
) -> Result<SettingEntry, CommandError> {
    let reg_idx = validate_key(&key)?;

    // Validate that the value matches the setting's declared type.
    let reg = registry();
    let def = &reg[reg_idx];
    if !def.value_type.is_compatible_value(&value) {
        return Err(CommandError::new(
            format!(
                "invalid value for '{}': expected {}, got {}",
                key,
                def.value_type.type_name(),
                value_type_label(&value),
            ),
            ErrorType::Validation,
        ));
    }

    let dir = kiro_dir()?;

    // Serialize load-modify-save to prevent lost updates from rapid changes.
    let _guard = SETTINGS_WRITE_LOCK.lock().unwrap_or_else(|poisoned| {
        // A prior panic poisoned the lock. Recover rather than cascade panics —
        // a lost-update is preferable to permanently bricking settings writes.
        poisoned.into_inner()
    });

    let mut json = load_settings(&dir)?;
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
pub async fn reset_kiro_setting(key: String) -> Result<(), CommandError> {
    validate_key(&key)?;

    let dir = kiro_dir()?;

    // Serialize load-modify-save to prevent lost updates from rapid changes.
    let _guard = SETTINGS_WRITE_LOCK.lock().unwrap_or_else(|poisoned| {
        // A prior panic poisoned the lock. Recover rather than cascade panics —
        // a lost-update is preferable to permanently bricking settings writes.
        poisoned.into_inner()
    });

    let mut json = load_settings(&dir)?;
    remove_nested(&mut json, &key);
    save_kiro_settings_to(&dir, &json).map_err(|e| {
        CommandError::new(format!("failed to save settings: {e}"), ErrorType::IoError)
    })?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Utilities
// ---------------------------------------------------------------------------

/// Human-readable label for a JSON value's type, used in validation errors.
fn value_type_label(value: &JsonValue) -> &'static str {
    match value {
        JsonValue::Null => "null",
        JsonValue::Bool(_) => "boolean",
        JsonValue::Number(_) => "number",
        JsonValue::String(_) => "string",
        JsonValue::Array(_) => "array",
        JsonValue::Object(_) => "object",
    }
}

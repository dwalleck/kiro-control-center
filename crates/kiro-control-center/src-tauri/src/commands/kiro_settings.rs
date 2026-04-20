//! Tauri commands for reading and writing Kiro CLI settings.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use kiro_market_core::file_lock::with_file_lock;
use kiro_market_core::kiro_settings::{
    apply_registered_setting, default_kiro_dir, kiro_settings_path, load_kiro_settings_from,
    registry, remove_nested, resolve_settings, save_kiro_settings_to, LoadSettingsError,
    SettingEntry,
};
use serde_json::Value as JsonValue;

use crate::error::{CommandError, ErrorType};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Serialize settings writes to prevent lost-update races from rapid UI
/// interactions (e.g. toggling multiple booleans quickly).
static SETTINGS_WRITE_LOCK: Mutex<()> = Mutex::new(());

/// Acquire the settings write lock, recovering from poisoning.
fn acquire_settings_lock() -> std::sync::MutexGuard<'static, ()> {
    SETTINGS_WRITE_LOCK.lock().unwrap_or_else(|poisoned| {
        // A prior panic poisoned the lock. Recover rather than cascade
        // panics — a lost-update is preferable to permanently bricking
        // settings writes.
        poisoned.into_inner()
    })
}

/// Save the settings JSON to the Kiro directory, wrapping I/O errors
/// into [`CommandError`].
fn save_settings(dir: &std::path::Path, json: &serde_json::Value) -> Result<(), CommandError> {
    save_kiro_settings_to(dir, json)
        .map_err(|e| CommandError::new(format!("failed to save settings: {e}"), ErrorType::IoError))
}

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
    reg.iter().position(|def| def.key == key).ok_or_else(|| {
        CommandError::new(format!("unknown setting key: {key}"), ErrorType::Validation)
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
///
/// All validation runs inside `apply_registered_setting` so the
/// "key in registry?" / "value type matches?" / "write" triple stays
/// atomic at the source of truth (kiro-market-core). Earlier this
/// command did the registry check here and the type check inline,
/// then handed the validated triple to the unchecked `set_nested`;
/// the new helper keeps the three steps inseparable so a future
/// caller can't drop one by accident.
#[tauri::command]
#[specta::specta]
#[allow(clippy::unused_async)]
pub async fn set_kiro_setting(key: String, value: JsonValue) -> Result<SettingEntry, CommandError> {
    if key.is_empty() {
        return Err(CommandError::new(
            "setting key must not be empty",
            ErrorType::Validation,
        ));
    }

    let dir = kiro_dir()?;
    locked_modify(&dir, |json| {
        // All current reject reasons (unknown key, type mismatch) surface
        // as Validation errors — same severity, same UI treatment. The
        // catch-all is required because ApplySettingError is
        // `#[non_exhaustive]`: a future variant added in core (e.g.
        // ReadOnly, Deprecated) compiles here without forcing a frontend
        // edit, and its `Display` impl provides the user-facing string.
        apply_registered_setting(json, &key, value)
            .map_err(|e| CommandError::new(e.to_string(), ErrorType::Validation))
    })?;

    let json = load_settings(&dir)?;
    let entry = resolve_settings(&json)
        .into_iter()
        .find(|e| e.key == key)
        .expect("key was validated against registry inside apply_registered_setting");

    Ok(entry)
}

/// Remove a single Kiro CLI setting by key, reverting it to its default.
#[tauri::command]
#[specta::specta]
#[allow(clippy::unused_async)]
pub async fn reset_kiro_setting(key: String) -> Result<(), CommandError> {
    validate_key(&key)?;

    let dir = kiro_dir()?;
    locked_modify(&dir, |json| {
        remove_nested(json, &key);
        Ok(())
    })
}

/// Run a load-modify-save cycle on the Kiro settings file under both an
/// in-process [`Mutex`] and a cross-process file lock. The in-process
/// mutex absorbs rapid UI clicks; the file lock prevents two Tauri/CLI
/// processes from clobbering each other's writes on the same `~/.kiro/`.
fn locked_modify(
    dir: &Path,
    modify: impl FnOnce(&mut JsonValue) -> Result<(), CommandError>,
) -> Result<(), CommandError> {
    let _guard = acquire_settings_lock();
    let lock_target = kiro_settings_path(dir);
    with_file_lock(&lock_target, || -> Result<(), CommandError> {
        let mut json = load_settings(dir)?;
        modify(&mut json)?;
        save_settings(dir, &json)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    // Tests reach into set_nested to seed the file directly without
    // going through the registry-validation path that the production
    // command uses. Imported in the test module only so it doesn't
    // appear unused in non-test builds.
    use kiro_market_core::kiro_settings::set_nested;

    #[test]
    fn validate_key_rejects_empty_key() {
        let err = validate_key("").expect_err("empty key should be rejected");
        assert_eq!(err.error_type, ErrorType::Validation);
        assert!(err.message.contains("must not be empty"));
    }

    #[test]
    fn validate_key_rejects_unknown_key() {
        let err = validate_key("totally.bogus.setting").expect_err("unknown key");
        assert_eq!(err.error_type, ErrorType::Validation);
        assert!(
            err.message.contains("unknown setting key"),
            "expected unknown-key wording: {}",
            err.message
        );
    }

    #[test]
    fn validate_key_accepts_known_key() {
        // Pick the first registered key so this test stays valid as the
        // registry evolves.
        let reg = registry();
        let known = reg.first().expect("registry has at least one entry").key;
        let idx = validate_key(known).expect("known key should validate");
        assert_eq!(reg[idx].key, known);
    }

    #[test]
    fn load_settings_returns_empty_object_when_file_missing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let json = load_settings(dir.path()).expect("missing → defaults");
        assert!(json.is_object(), "should be an empty JSON object");
        assert_eq!(json.as_object().unwrap().len(), 0);
    }

    #[test]
    fn load_settings_propagates_invalid_json_with_recovery_hint() {
        // The error message must give the user a clear next step: where
        // the file lives and what to do with it. Without this they have
        // no way to recover from a corrupt settings file.
        let dir = tempfile::tempdir().expect("tempdir");
        let settings_subdir = dir.path().join("settings");
        std::fs::create_dir_all(&settings_subdir).expect("mkdir settings");
        std::fs::write(settings_subdir.join("cli.json"), "{not valid json").expect("write");

        let err = load_settings(dir.path()).expect_err("corrupt JSON should error");
        assert_eq!(err.error_type, ErrorType::ParseError);
        assert!(
            err.message.contains("invalid JSON"),
            "missing 'invalid JSON' hint in: {}",
            err.message
        );
        assert!(
            err.message.contains("Back up or delete"),
            "missing recovery action in: {}",
            err.message
        );
    }

    #[test]
    fn locked_modify_persists_changes_under_lock() {
        let dir = tempfile::tempdir().expect("tempdir");
        let key = "chat.defaultModel";

        locked_modify(dir.path(), |json| {
            set_nested(json, key, JsonValue::from("test-model"));
            Ok(())
        })
        .expect("modify should succeed");

        // Reload via the same code path the Tauri command uses.
        let after = load_settings(dir.path()).expect("load");
        assert_eq!(
            after.pointer("/chat/defaultModel"),
            Some(&JsonValue::from("test-model"))
        );
    }

    #[test]
    fn acquire_settings_lock_recovers_from_poisoned_mutex() {
        // Poison the lock by panicking inside it on a separate thread, then
        // confirm acquire_settings_lock still hands back a usable guard.
        let poisoner = std::thread::spawn(|| {
            let _guard = acquire_settings_lock();
            panic!("poison the mutex");
        });
        // Expect the spawned thread to panic; we don't propagate that here.
        let _ = poisoner.join();

        // Should succeed (and not panic) even though the mutex is poisoned.
        let _guard = acquire_settings_lock();
    }
}

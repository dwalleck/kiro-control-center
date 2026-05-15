// Tauri commands are public for the invoke handler but are internal to this app.
// Pedantic doc lints don't add value here.
#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::cast_possible_truncation,
    clippy::doc_markdown,
    clippy::too_many_lines
)]

use tauri_specta::{Builder, collect_commands};

pub mod commands;
pub mod error;

fn create_builder() -> Builder<tauri::Wry> {
    Builder::<tauri::Wry>::new().commands(collect_commands![
        commands::browse::list_marketplaces,
        commands::browse::list_plugins,
        commands::browse::list_available_skills,
        commands::browse::list_all_skills_for_marketplace,
        commands::browse::list_plugin_catalog_for_marketplace,
        commands::browse::install_skills,
        commands::browse::get_project_info,
        commands::installed::list_installed_skills,
        commands::installed::remove_skill,
        commands::marketplaces::add_marketplace,
        commands::marketplaces::remove_marketplace,
        commands::marketplaces::update_marketplace,
        commands::settings::get_settings,
        commands::settings::save_scan_roots,
        commands::settings::discover_projects,
        commands::settings::set_active_project,
        commands::kiro_settings::get_kiro_settings,
        commands::kiro_settings::set_kiro_setting,
        commands::kiro_settings::reset_kiro_setting,
        commands::steering::install_plugin_steering,
        commands::steering::install_steering_files,
        commands::steering::remove_steering_file,
        commands::agents::install_plugin_agents,
        commands::agents::install_agents,
        commands::agents::remove_agent,
        commands::plugins::install_plugin,
        commands::plugins::list_installed_plugins,
        commands::plugins::remove_plugin,
        commands::plugins::detect_plugin_updates,
    ])
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = create_builder();

    #[cfg(debug_assertions)]
    builder
        .export(
            specta_typescript::Typescript::default(),
            "../src/lib/bindings.ts",
        )
        .expect("Failed to export typescript bindings");

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(builder.invoke_handler())
        .setup(move |app| {
            builder.mount_events(app);
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Generate TypeScript bindings.
    /// Run with: cargo test -p kiro-control-center generate_types -- --exact --ignored
    #[test]
    #[ignore = "build-only: regenerates bindings, not a regression test"]
    fn generate_types() {
        let builder = create_builder();
        builder
            .export(
                specta_typescript::Typescript::default(),
                "../src/lib/bindings.ts",
            )
            .expect("Failed to export TypeScript bindings");

        println!("TypeScript bindings generated successfully!");
    }

    /// C12 regression fence: the new BrowseTab catalog wire types
    /// must surface as `export type ...` in `bindings.ts`, AND none
    /// of them may carry a `chrono::DateTime` (specta doesn't have
    /// the chrono feature enabled in this crate, so a stray
    /// `DateTime<Utc>` field on a `specta::Type`-derived struct
    /// would fail compilation — the grep below is a belt-and-braces
    /// check against future drift).
    ///
    /// Reads the *committed* bindings file (the same path
    /// `generate_types` writes to). If `bindings.ts` is stale, this
    /// test fails — the fix is to run the regen test, not paper
    /// over the assertion. The tracker referenced in the design
    /// (kiro-zx73, kiro-3ivx) covers separate follow-ups, not this
    /// fence.
    #[test]
    fn bindings_export_plugin_catalog_view() {
        // The test runs from kiro-control-center/src-tauri, so the
        // bindings file is one level up.
        let bindings = std::fs::read_to_string("../src/lib/bindings.ts")
            .expect("bindings.ts should exist; run generate_types first if it doesn't");

        for ty in &[
            "PluginCatalogResponseView",
            "PluginCatalogEntryView",
            "SteeringItemInfo",
            "AgentItemInfo",
            "SkippedItem",
        ] {
            let needle = format!("export type {ty}");
            assert!(
                bindings.contains(&needle),
                "bindings.ts must export `{ty}` — missing `{needle}`. \
                 If this test fails after type changes, run \
                 `cargo test -p kiro-control-center --lib generate_types -- --exact --ignored` \
                 to regenerate, then re-run this test."
            );
        }

        // The wire surface MUST NOT carry chrono types (specta's
        // chrono feature is off in this crate; one accidental
        // `DateTime<Utc>` on a derived struct would fail to compile,
        // but the grep guards against future config changes).
        let chrono_count = bindings
            .lines()
            .filter(|line| {
                // Skip doc-comments; they may mention chrono
                // descriptively (e.g. "specta's chrono feature
                // isn't enabled here").
                let trimmed = line.trim_start();
                if trimmed.starts_with('*') || trimmed.starts_with("//") {
                    return false;
                }
                trimmed.contains("chrono") || trimmed.contains("DateTime<")
            })
            .count();
        assert_eq!(
            chrono_count, 0,
            "bindings.ts contains chrono / DateTime types in non-doc lines; \
             specta has no chrono feature in this crate, so these would have \
             failed to compile — investigate"
        );
    }
}

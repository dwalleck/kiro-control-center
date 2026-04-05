// Tauri commands are public for the invoke handler but are internal to this app.
// Pedantic doc lints don't add value here.
#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::cast_possible_truncation,
    clippy::doc_markdown,
    clippy::too_many_lines
)]

use tauri_specta::{collect_commands, Builder};

pub mod commands;
pub mod error;

fn create_builder() -> Builder<tauri::Wry> {
    Builder::<tauri::Wry>::new().commands(collect_commands![
        commands::browse::list_marketplaces,
        commands::browse::list_plugins,
        commands::browse::list_available_skills,
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
    #[ignore]
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
}

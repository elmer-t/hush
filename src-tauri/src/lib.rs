pub mod commands;
pub mod config;
pub mod db;
pub mod discover;
pub mod embed;
pub mod import;
pub mod markdown;
pub mod session;

use std::time::Duration;

use tauri::{Emitter, Manager, RunEvent, WindowEvent};

use crate::commands::discover::DiscoverJob;
use crate::commands::embeddings::Indexer;
use crate::commands::import::ImportJob;
use crate::commands::{CLOSE_EVENT, LOCKED_EVENT};
use crate::config::Config;
use crate::session::Session;

const IDLE_CHECK_INTERVAL: Duration = Duration::from_secs(5);
/// How long the frontend gets to save pending edits before the app exits anyway.
const CLOSE_GRACE: Duration = Duration::from_secs(3);

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(Session::default())
        .manage(ImportJob::default())
        .manage(DiscoverJob::default())
        .setup(|app| {
            let config_path = app.path().app_config_dir()?.join("config.json");
            app.manage(Config::new(config_path));
            app.manage(Indexer::new(app.path().resource_dir()?.join("models")));

            // Auto-lock runs here rather than in the webview, whose timers are throttled
            // while the window is hidden.
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                loop {
                    std::thread::sleep(IDLE_CHECK_INTERVAL);
                    if handle.state::<Session>().lock_if_idle() {
                        let _ = handle.emit(LOCKED_EVENT, "idle");
                    }
                }
            });
            Ok(())
        })
        // Closing is handled here rather than with a webview close listener, which would
        // keep the window open forever if the page reloaded (as it does on lock).
        .on_window_event(|window, event| {
            let WindowEvent::CloseRequested { api, .. } = event else { return };
            let app = window.app_handle();
            if app.state::<Session>().path().is_none() {
                return;
            }
            api.prevent_close();
            let _ = window.emit(CLOSE_EVENT, ());
            let app = app.clone();
            std::thread::spawn(move || {
                std::thread::sleep(CLOSE_GRACE);
                app.exit(0);
            });
        })
        .invoke_handler(tauri::generate_handler![
            commands::vault::recent_vaults,
            commands::vault::forget_recent_vault,
            commands::vault::create_vault,
            commands::vault::open_vault,
            commands::vault::lock_vault,
            commands::vault::vault_status,
            commands::vault::touch,
            commands::vault::auto_lock_minutes,
            commands::vault::set_auto_lock_minutes,
            commands::vault::change_passphrase,
            commands::vault::exit_app,
            commands::notes::folders,
            commands::notes::folder_notes,
            commands::notes::sidebar_view,
            commands::notes::set_sidebar_view,
            commands::notes::confirm_trash,
            commands::notes::set_confirm_trash,
            commands::notes::create_note,
            commands::notes::open_note,
            commands::notes::save_note,
            commands::notes::rename_note,
            commands::notes::delete_note,
            commands::notes::trash_note,
            commands::notes::trash_notes,
            commands::notes::move_notes,
            commands::notes::move_folder,
            commands::notes::edit_notes_metadata,
            commands::notes::trashed_notes,
            commands::notes::restore_note,
            commands::notes::empty_trash,
            commands::notes::trash_days,
            commands::notes::set_trash_days,
            commands::notes::edit_frontmatter,
            commands::notes::default_author,
            commands::notes::set_default_author,
            commands::search::search_notes,
            commands::search::search_facets,
            commands::search::attribute_values,
            commands::import::start_import,
            commands::import::cancel_import,
            commands::import::suggest_import_target,
            commands::import::import_target_notes,
            commands::import::import_rules,
            commands::import::set_import_rules,
            commands::import::default_import_rules,
            commands::import::test_import_rules,
            commands::embeddings::index_status,
            commands::tags::tag_list,
            commands::tags::set_tag_description,
            commands::tags::tag_suggestions,
            commands::tags::accept_tag_suggestion,
            commands::tags::reject_tag_suggestion,
            commands::tags::suggestion_threshold,
            commands::tags::set_suggestion_threshold,
            commands::embeddings::related_notes,
            commands::discover::discover_tags,
            commands::discover::cancel_discover_tags,
            commands::discover::create_tag_from_group,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            if let RunEvent::Exit = event {
                app.state::<Session>().lock();
            }
        });
}

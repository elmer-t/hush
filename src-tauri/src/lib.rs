pub mod activity;
pub mod classify;
pub mod commands;
pub mod config;
pub mod db;
pub mod discover;
pub mod embed;
pub mod graph;
pub mod import;
pub mod markdown;
pub mod retag;
pub mod session;

use std::time::Duration;

use tauri::{Emitter, Manager, RunEvent, WindowEvent};
use tauri_plugin_window_state::StateFlags;

use crate::commands::activity::ActivityPanel;
use crate::commands::discover::DiscoverJob;
use crate::commands::embeddings::Indexer;
use crate::commands::import::ImportJob;
use crate::commands::models::Downloads;
use crate::commands::{CLOSE_EVENT, LOCKED_EVENT};
use crate::config::Config;
use crate::embed::models::ModelDirs;
use crate::session::Session;

const IDLE_CHECK_INTERVAL: Duration = Duration::from_secs(5);
/// How long the frontend gets to save pending edits before the app exits anyway.
const CLOSE_GRACE: Duration = Duration::from_secs(3);

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        // Geometry only, in plain JSON in the app config dir: no note data, readable before
        // unlock. The position is only restored when it is on a connected monitor.
        .plugin(
            tauri_plugin_window_state::Builder::default()
                .with_state_flags(StateFlags::SIZE | StateFlags::POSITION | StateFlags::MAXIMIZED)
                .build(),
        )
        .manage(Session::default())
        .manage(ImportJob::default())
        .manage(DiscoverJob::default())
        .manage(ActivityPanel::default())
        .manage(Downloads::default())
        .setup(|app| {
            let config_path = app.path().app_config_dir()?.join("config.json");
            let config = Config::new(config_path);
            let (all_cores, gpu_off) = {
                let c = config.get();
                (c.index_all_cores, c.index_gpu_off)
            };
            app.manage(config);
            // Downloaded and imported models go to the local (not roaming) app data
            // folder: they are large, and per machine.
            let dirs = ModelDirs {
                bundled: app.path().resource_dir()?.join("models"),
                user: app.path().app_local_data_dir()?.join("models"),
            };
            app.manage(Indexer::new(app.handle().clone(), dirs, all_cores, gpu_off));
            commands::activity::start_emitter(app.handle());

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
            commands::notes::recent_opened_notes,
            commands::notes::recent_opened_count,
            commands::notes::set_recent_opened_count,
            commands::notes::clear_recent_opened,
            commands::notes::recent_opened_collapsed,
            commands::notes::set_recent_opened_collapsed,
            commands::layout::last_opened_note,
            commands::layout::graph_scope,
            commands::layout::set_graph_scope,
            commands::layout::sidebar_folder,
            commands::layout::set_sidebar_folder,
            commands::notes::trash_note,
            commands::notes::trash_notes,
            commands::notes::move_notes,
            commands::notes::move_folder,
            commands::notes::edit_notes_metadata,
            commands::notes::set_read_status,
            commands::notes::trash_entries,
            commands::notes::trash_count,
            commands::notes::folder_contents,
            commands::notes::trash_folder,
            commands::notes::restore_folder,
            commands::notes::delete_trash_folder,
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
            commands::embeddings::index_all_cores,
            commands::embeddings::set_index_all_cores,
            commands::embeddings::gpu_status,
            commands::embeddings::set_index_gpu,
            commands::embeddings::model_status,
            commands::embeddings::model_overview,
            commands::embeddings::set_embedding_model,
            commands::embeddings::recheck_models,
            commands::models::model_downloads,
            commands::models::download_model,
            commands::models::cancel_model_download,
            commands::models::import_model_files,
            commands::models::remove_model,
            commands::tags::tag_list,
            commands::tags::set_tag_description,
            commands::tags::tag_suggestions,
            commands::tags::accept_tag_suggestion,
            commands::tags::reject_tag_suggestion,
            commands::tags::suggestion_threshold,
            commands::tags::set_suggestion_threshold,
            commands::embeddings::related_notes,
            commands::links::backlinks,
            commands::links::outgoing_links,
            commands::links::resolve_link,
            commands::links::create_linked_note,
            commands::links::collapsed_sections,
            commands::links::set_section_collapsed,
            commands::links::graph,
            commands::discover::discover_tags,
            commands::discover::cancel_discover_tags,
            commands::discover::create_tag_from_group,
            commands::training::tag_training,
            commands::review::review_tag_suggestions,
            commands::review::accept_tag_suggestions,
            commands::review::reject_tag_suggestions,
            commands::review::retrain_tags,
            commands::activity::activity_status,
            commands::activity::set_activity_panel_open,
            commands::activity::note_internals,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            if let RunEvent::Exit = event {
                app.state::<Session>().lock();
            }
        });
}

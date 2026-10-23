pub mod commands;
pub mod config;
pub mod db;
pub mod markdown;
pub mod session;

use std::time::Duration;

use tauri::{Emitter, Manager};

use crate::commands::LOCKED_EVENT;
use crate::config::Config;
use crate::session::Session;

const IDLE_CHECK_INTERVAL: Duration = Duration::from_secs(5);

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(Session::default())
        .setup(|app| {
            let config_path = app.path().app_config_dir()?.join("config.json");
            app.manage(Config::load(config_path));

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
        .invoke_handler(tauri::generate_handler![
            commands::recent_vaults,
            commands::forget_recent_vault,
            commands::create_vault,
            commands::open_vault,
            commands::lock_vault,
            commands::vault_status,
            commands::touch,
            commands::auto_lock_minutes,
            commands::set_auto_lock_minutes,
            commands::change_passphrase,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

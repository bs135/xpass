mod commands;
mod error;
mod paths;
mod session;
mod state;
mod storage;
mod throttle;

use tauri::Manager;
use xpass_core::crypto::KdfParams;

use crate::{paths::Paths, state::AppState};

pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let paths = Paths::from_app(app.handle())?;
            app.manage(AppState::new(paths, KdfParams::DEFAULT));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::app_info,
            commands::session::vault_status,
            commands::session::create_vault,
            commands::session::unlock_password,
            commands::session::unlock_recovery,
            commands::session::lock,
        ])
        .run(tauri::generate_context!())
        .expect("error while running xpass");
}

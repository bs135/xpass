mod commands;
mod error;
mod paths;
mod session;
mod state;
mod storage;
mod strength;
mod throttle;

use tauri::Manager;
use xpass_core::crypto::KdfParams;

use crate::{paths::Paths, state::AppState};

pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let paths = data_dir_override().map_or_else(|| Paths::from_app(app.handle()), |dir| Ok(Paths::new(dir)))?;
            app.manage(AppState::new(paths, KdfParams::DEFAULT));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::app_info,
            commands::session::vault_status,
            commands::session::create_vault,
            commands::session::unlock_password,
            commands::session::unlock_recovery,
            commands::session::change_password,
            commands::session::new_recovery_key,
            commands::session::password_strength,
            commands::session::lock,
        ])
        .run(tauri::generate_context!())
        .expect("error while running xpass");
}

/// Debug builds only: `XPASS_DATA_DIR` points the app at a throwaway directory so
/// development and manual tests never touch the real vault.
fn data_dir_override() -> Option<std::path::PathBuf> {
    if cfg!(debug_assertions) {
        std::env::var_os("XPASS_DATA_DIR").map(Into::into)
    } else {
        None
    }
}

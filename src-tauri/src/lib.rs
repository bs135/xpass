mod clipboard;
mod commands;
mod entries;
mod error;
mod file_dialog;
mod generator;
mod github;
mod hello;
mod lock_watch;
mod paths;
mod secrets;
mod session;
mod settings;
mod shell;
mod state;
mod storage;
mod strength;
mod sync_state;
mod sync_worker;
mod throttle;
mod tray;
mod update;

use tauri::{Manager, RunEvent};
use xpass_core::crypto::KdfParams;

use crate::{clipboard::SecureClipboard, lock_watch::LockReason, paths::Paths, state::AppState};

pub fn run() {
    install_panic_hook();
    tauri::Builder::default()
        // Must be the first plugin: a second launch focuses the running app instead.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            tray::show_main(app)
        }))
        .setup(|app| {
            let paths = data_dir_override().map_or_else(|| Paths::from_app(app.handle()), |dir| Ok(Paths::new(dir)))?;
            let overridden = data_dir_override().is_some();
            let tokens = secrets::CredentialManager::new(secrets::target_for(&paths, overridden));
            let state = AppState::new(paths, KdfParams::DEFAULT, SecureClipboard::system(), hello::system())
                .with_token_store(Box::new(tokens));
            if let Some(timeout) = idle_timeout_override() {
                state.set_idle_timeout(Some(timeout));
            }
            app.manage(state);
            app.manage(update::UpdateState::default());
            lock_watch::start(app.handle().clone());
            tray::setup(app)?;
            sync_worker::start(app.handle().clone());
            update::start(app.handle().clone());
            Ok(())
        })
        .on_window_event(tray::on_window_event)
        .invoke_handler(tauri::generate_handler![
            commands::app_info,
            commands::open_project_page,
            update::check_update,
            update::update_status,
            update::open_release_page,
            commands::session::vault_status,
            commands::session::create_vault,
            commands::session::unlock_password,
            commands::session::unlock_recovery,
            commands::session::change_password,
            commands::session::new_recovery_key,
            commands::session::rotate_vault_key,
            commands::hello::hello_status,
            commands::hello::unlock_hello,
            commands::hello::hello_enable,
            commands::hello::hello_disable,
            commands::hello::hello_dismiss_reenroll,
            commands::settings::get_settings,
            commands::sync::sync_get_config,
            commands::sync::sync_configure,
            commands::sync::sync_set_token,
            commands::sync::sync_disconnect,
            commands::sync::sync_test_connection,
            commands::sync::sync_open_token_page,
            commands::sync::sync_status,
            commands::sync::sync_now,
            commands::sync::sync_dismiss_password_changed,
            commands::sync::sync_join,
            commands::sync::sync_resolve_key_change,
            commands::sync::sync_replace_local,
            commands::backups::list_backups,
            commands::backups::restore_backup,
            commands::import::import_open,
            commands::import::import_set_mapping,
            commands::import::import_commit,
            commands::import::import_cancel,
            commands::import::import_delete_source,
            commands::export::export_vault,
            commands::settings::update_settings,
            commands::session::password_strength,
            commands::session::lock,
            commands::entries::list_entries,
            commands::entries::get_entry,
            commands::entries::save_entry,
            commands::entries::list_history,
            commands::entries::use_history_password,
            commands::entries::delete_entry,
            commands::entries::set_favorite,
            commands::entries::open_url,
            commands::entries::copy_field,
            commands::entries::reveal_secret,
            commands::tools::generate_password,
            commands::tools::copy_text,
        ])
        .build(tauri::generate_context!())
        .expect("error while building xpass")
        .run(|app, event| {
            if let RunEvent::Exit = event {
                // Wipe the session and our clipboard copy before the process ends.
                lock_watch::lock_app(app, LockReason::Exit);
                // S8: push unsynced changes without the key (bounded wait).
                sync_worker::push_on_exit(app);
            }
        });
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

/// Debug builds only: `XPASS_IDLE_SECS` shortens the inactivity timeout for manual tests.
fn idle_timeout_override() -> Option<std::time::Duration> {
    if !cfg!(debug_assertions) {
        return None;
    }
    let secs: u64 = std::env::var("XPASS_IDLE_SECS").ok()?.parse().ok()?;
    Some(std::time::Duration::from_secs(secs))
}

/// Panic messages could in theory include data; log only where it happened.
fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        let location = info
            .location()
            .map(|l| format!("{}:{}", l.file(), l.line()))
            .unwrap_or_else(|| "unknown location".into());
        eprintln!("xpass: internal error at {location}");
    }));
}

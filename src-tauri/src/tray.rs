//! Tray icon (Open / Lock / Quit), close-to-tray and lock-on-minimize.

use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    App, AppHandle, Manager, Window, WindowEvent, Wry,
};

use crate::{
    lock_watch::{self, LockReason},
    settings::Language,
    state::{AppState, SyncPhase, SyncStatus, VaultStatus},
    sync_worker::Trigger,
};

const MAIN_WINDOW: &str = "main";

/// Shows, restores and focuses the main window (tray "Open", second app launch).
pub fn show_main(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(MAIN_WINDOW) {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

struct Labels {
    open: &'static str,
    lock: &'static str,
    sync: &'static str,
    quit: &'static str,
    synced: &'static str,
    syncing: &'static str,
    pending: &'static str,
    offline: &'static str,
    sync_problem: &'static str,
}

/// The tray's "Sync now" item and the language of the tooltips, for updates.
struct TraySync {
    item: MenuItem<Wry>,
    labels: Labels,
}

/// Tray labels follow the language setting (or Windows) at startup.
fn labels(language: Option<Language>) -> Labels {
    match language.unwrap_or_else(os_language) {
        Language::Vi => Labels {
            open: "Mở xPass",
            lock: "Khoá",
            sync: "Đồng bộ ngay",
            quit: "Thoát",
            synced: "Đã đồng bộ",
            syncing: "Đang đồng bộ…",
            pending: "Có thay đổi chưa đồng bộ",
            offline: "Offline, thay đổi được giữ trên máy này",
            sync_problem: "Đồng bộ cần bạn xử lý",
        },
        Language::En => Labels {
            open: "Open xPass",
            lock: "Lock",
            sync: "Sync now",
            quit: "Quit",
            synced: "Synced",
            syncing: "Syncing…",
            pending: "Changes not synced yet",
            offline: "Offline, changes kept on this PC",
            sync_problem: "Sync needs your attention",
        },
    }
}

#[cfg(windows)]
fn os_language() -> Language {
    // SAFETY: no arguments, no preconditions.
    let lang = unsafe { windows_sys::Win32::Globalization::GetUserDefaultUILanguage() };
    // Primary language id is the low 10 bits; 0x2A = LANG_VIETNAMESE.
    if lang & 0x3ff == 0x2a {
        Language::Vi
    } else {
        Language::En
    }
}

#[cfg(not(windows))]
fn os_language() -> Language {
    Language::En
}

pub fn setup(app: &App) -> tauri::Result<()> {
    let language = app.state::<AppState>().settings().language;
    let l = labels(language);
    let open = MenuItem::with_id(app, "open", l.open, true, None::<&str>)?;
    let lock = MenuItem::with_id(app, "lock", l.lock, true, None::<&str>)?;
    let sync = MenuItem::with_id(app, "sync", l.sync, false, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", l.quit, true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(app, &[&open, &lock, &sync, &separator, &quit])?;
    app.manage(TraySync { item: sync, labels: l });

    let mut tray = TrayIconBuilder::with_id("main")
        .tooltip("xPass")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "open" => show_main(app),
            "lock" => {
                lock_watch::lock_app(app, LockReason::Manual);
            }
            "sync" => app.state::<AppState>().request_sync(Trigger::Manual),
            // RunEvent::Exit locks the vault and clears our clipboard copy.
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(())
}

/// Tooltip and "Sync now" follow the sync status (called by the sync worker).
pub fn show_sync_status(app: &AppHandle, status: &SyncStatus) {
    let Some(tray_sync) = app.try_state::<TraySync>() else {
        return;
    };
    let unlocked = app
        .try_state::<AppState>()
        .is_some_and(|s| s.status() == VaultStatus::Unlocked);
    let _ = tray_sync
        .item
        .set_enabled(unlocked && !matches!(status.phase, SyncPhase::Off | SyncPhase::NoToken | SyncPhase::Syncing));
    if let Some(tray) = app.tray_by_id("main") {
        let _ = tray.set_tooltip(Some(tooltip(&tray_sync.labels, status)));
    }
}

fn tooltip(l: &Labels, status: &SyncStatus) -> String {
    let detail = match status.phase {
        SyncPhase::Off => return "xPass".to_owned(),
        SyncPhase::Syncing => l.syncing,
        SyncPhase::Idle if status.dirty => l.pending,
        SyncPhase::Idle => l.synced,
        SyncPhase::Offline => l.offline,
        _ => l.sync_problem,
    };
    format!("xPass – {detail}")
}

/// Close button hides to the tray (setting); optional lock when hidden or minimized.
pub fn on_window_event(window: &Window, event: &WindowEvent) {
    if window.label() != MAIN_WINDOW {
        return;
    }
    let app = window.app_handle();
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    let settings = state.settings();
    match event {
        WindowEvent::CloseRequested { api, .. } if settings.close_to_tray => {
            api.prevent_close();
            let _ = window.hide();
            if settings.lock_on_minimize {
                lock_watch::lock_app(app, LockReason::Minimized);
            }
        }
        WindowEvent::Resized(_) if settings.lock_on_minimize && window.is_minimized().unwrap_or(false) => {
            lock_watch::lock_app(app, LockReason::Minimized);
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_follow_the_language_setting() {
        assert_eq!(labels(Some(Language::Vi)).quit, "Thoát");
        assert_eq!(labels(Some(Language::En)).open, "Open xPass");
    }

    #[test]
    fn tooltip_follows_the_sync_status() {
        let l = labels(Some(Language::En));
        let status = |phase, dirty| SyncStatus {
            phase,
            dirty,
            ..SyncStatus::default()
        };
        assert_eq!(tooltip(&l, &status(SyncPhase::Off, true)), "xPass");
        assert_eq!(tooltip(&l, &status(SyncPhase::Idle, false)), "xPass – Synced");
        assert_eq!(
            tooltip(&l, &status(SyncPhase::Idle, true)),
            "xPass – Changes not synced yet"
        );
        assert_eq!(
            tooltip(&l, &status(SyncPhase::TokenInvalid, false)),
            "xPass – Sync needs your attention"
        );
    }
}

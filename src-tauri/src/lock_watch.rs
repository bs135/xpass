//! Automatic lock (T8): on Windows session lock (`Win+L`, screen lock), before sleep,
//! and after system-wide inactivity.
//!
//! - A dedicated thread owns a message-only window (`HWND_MESSAGE`) registered with
//!   `WTSRegisterSessionNotification` and `RegisterSuspendResumeNotification`; it
//!   reacts to `WTS_SESSION_LOCK` and `PBT_APMSUSPEND`.
//! - Another thread checks `GetLastInputInfo` every 10 s against the idle timeout.
//!
//! Locking drops the session (zeroizing keys and secrets), clears our clipboard copy
//! and emits `vault://locked` with the reason.

use std::{sync::mpsc, thread, time::Duration};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::state::{AppState, VaultStatus};

/// Event emitted to all windows when the vault locks.
pub const EVENT_LOCKED: &str = "vault://locked";

/// How often the idle timer is checked.
const IDLE_POLL: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum LockReason {
    Manual,
    Idle,
    SessionLocked,
    Suspend,
    Exit,
}

#[derive(Clone, Serialize)]
struct LockedPayload {
    reason: LockReason,
}

/// Locks the vault and notifies the frontend. Returns whether it was unlocked.
pub fn lock_app(app: &AppHandle, reason: LockReason) -> bool {
    let Some(state) = app.try_state::<AppState>() else {
        return false;
    };
    let was_unlocked = state.lock();
    if was_unlocked {
        // Best effort: the session is gone even if no window receives this.
        let _ = app.emit(EVENT_LOCKED, LockedPayload { reason });
    }
    was_unlocked
}

/// True when the vault should lock for inactivity.
pub fn idle_exceeded(idle: Duration, timeout: Option<Duration>) -> bool {
    timeout.is_some_and(|t| idle >= t)
}

/// Starts the watcher threads. They live as long as the process.
pub fn start(app: AppHandle) {
    let (tx, rx) = mpsc::channel::<LockReason>();

    let handler_app = app.clone();
    spawn("xpass-lock-handler", move || {
        for reason in rx {
            lock_app(&handler_app, reason);
        }
    });

    let os_tx = tx.clone();
    spawn("xpass-lock-watch", move || os::run_message_window(os_tx));

    spawn("xpass-idle-watch", move || loop {
        thread::sleep(IDLE_POLL);
        let Some(state) = app.try_state::<AppState>() else {
            continue;
        };
        if state.status() != VaultStatus::Unlocked {
            continue;
        }
        if idle_exceeded(os::idle_time(), state.idle_timeout()) && tx.send(LockReason::Idle).is_err() {
            return;
        }
    });
}

fn spawn(name: &str, f: impl FnOnce() + Send + 'static) {
    if let Err(e) = thread::Builder::new().name(name.into()).spawn(f) {
        eprintln!("xpass: could not start {name}: {e}");
    }
}

#[cfg(windows)]
mod os {
    use std::{
        mem::{size_of, zeroed},
        ptr::{null, null_mut},
        sync::{mpsc::Sender, Mutex, OnceLock},
        time::Duration,
    };

    use windows_sys::Win32::{
        Foundation::{HWND, LPARAM, LRESULT, WPARAM},
        System::{
            LibraryLoader::GetModuleHandleW, Power::RegisterSuspendResumeNotification,
            RemoteDesktop::WTSRegisterSessionNotification, SystemInformation::GetTickCount,
        },
        UI::{
            Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO},
            WindowsAndMessaging::{
                CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, RegisterClassW, TranslateMessage,
                DEVICE_NOTIFY_WINDOW_HANDLE, HWND_MESSAGE, MSG, PBT_APMSUSPEND, WM_POWERBROADCAST,
                WM_WTSSESSION_CHANGE, WNDCLASSW, WTS_SESSION_LOCK,
            },
        },
    };

    use super::LockReason;

    /// `NOTIFY_FOR_THIS_SESSION` (wtsapi32.h).
    const NOTIFY_FOR_THIS_SESSION: u32 = 0;

    /// Window class of the hidden message-only window (also used by manual tests).
    pub const CLASS_NAME: &str = "xpass-lock-watch";

    static SENDER: OnceLock<Mutex<Sender<LockReason>>> = OnceLock::new();

    fn send(reason: LockReason) {
        if let Some(tx) = SENDER.get() {
            let _ = tx.lock().map(|tx| tx.send(reason));
        }
    }

    unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        match msg {
            WM_WTSSESSION_CHANGE if wparam as u32 == WTS_SESSION_LOCK => send(LockReason::SessionLocked),
            WM_POWERBROADCAST if wparam as u32 == PBT_APMSUSPEND => send(LockReason::Suspend),
            _ => {}
        }
        // SAFETY: forwarding the unmodified message to the default handler.
        unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
    }

    /// Creates the message-only window and pumps its messages (blocks forever).
    pub fn run_message_window(tx: Sender<LockReason>) {
        if SENDER.set(Mutex::new(tx)).is_err() {
            return;
        }
        let class: Vec<u16> = CLASS_NAME.encode_utf16().chain(Some(0)).collect();
        // SAFETY: plain Win32 calls; `class` outlives the window, the class struct is
        // fully initialized (zeroed fields are valid "none" values), and the window
        // and message loop stay on this thread.
        unsafe {
            let hinstance = GetModuleHandleW(null());
            let mut wc: WNDCLASSW = zeroed();
            wc.lpfnWndProc = Some(wndproc);
            wc.hInstance = hinstance;
            wc.lpszClassName = class.as_ptr();
            if RegisterClassW(&wc) == 0 {
                eprintln!("xpass: lock watcher: RegisterClassW failed");
                return;
            }
            let hwnd = CreateWindowExW(
                0,
                class.as_ptr(),
                class.as_ptr(),
                0,
                0,
                0,
                0,
                0,
                HWND_MESSAGE,
                null_mut(),
                hinstance,
                null(),
            );
            if hwnd.is_null() {
                eprintln!("xpass: lock watcher: CreateWindowExW failed");
                return;
            }
            if WTSRegisterSessionNotification(hwnd, NOTIFY_FOR_THIS_SESSION) == 0 {
                eprintln!("xpass: lock watcher: WTSRegisterSessionNotification failed");
            }
            if RegisterSuspendResumeNotification(hwnd, DEVICE_NOTIFY_WINDOW_HANDLE) == 0 {
                eprintln!("xpass: lock watcher: RegisterSuspendResumeNotification failed");
            }
            #[cfg(debug_assertions)]
            eprintln!("xpass: lock watcher window hwnd={hwnd:?}");
            let mut msg: MSG = zeroed();
            while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }

    /// Time since the last keyboard or mouse input in this Windows session.
    pub fn idle_time() -> Duration {
        let mut info = LASTINPUTINFO {
            cbSize: size_of::<LASTINPUTINFO>() as u32,
            dwTime: 0,
        };
        // SAFETY: `info` is a valid, correctly sized LASTINPUTINFO.
        let ok = unsafe { GetLastInputInfo(&mut info) } != 0;
        if !ok {
            return Duration::ZERO;
        }
        // SAFETY: no arguments. Both values wrap every ~49.7 days; wrapping_sub handles it.
        let now = unsafe { GetTickCount() };
        Duration::from_millis(u64::from(now.wrapping_sub(info.dwTime)))
    }
}

#[cfg(not(windows))]
mod os {
    use std::{sync::mpsc::Sender, time::Duration};

    use super::LockReason;

    pub fn run_message_window(_tx: Sender<LockReason>) {}

    pub fn idle_time() -> Duration {
        Duration::ZERO
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_timeout_rules() {
        let five = Duration::from_secs(300);
        assert!(!idle_exceeded(Duration::from_secs(299), Some(five)));
        assert!(idle_exceeded(Duration::from_secs(300), Some(five)));
        assert!(!idle_exceeded(Duration::from_secs(100_000), None), "never");
    }

    #[test]
    fn reasons_serialize_for_the_frontend() {
        let json = serde_json::to_string(&LockedPayload {
            reason: LockReason::SessionLocked,
        })
        .unwrap();
        assert_eq!(json, r#"{"reason":"sessionLocked"}"#);
    }

    #[cfg(windows)]
    #[test]
    fn idle_time_is_readable() {
        // Any value is fine; the call must not fail or panic.
        let _ = os::idle_time();
    }
}

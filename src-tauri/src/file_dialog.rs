//! Native open / save dialogs called from Rust (v0.5 plan I1), without the `dialog`
//! or `fs` plugins: the chosen path stays in Rust and the WebView never sees it.
//!
//! Uses the shell's `IFileOpenDialog` / `IFileSaveDialog` on a dedicated COM STA
//! thread. The dialog is modal to the owner window (the main window's HWND) and does
//! not add the file to the "Recent" lists. Cancel → `Ok(None)`.

use std::path::PathBuf;

use crate::error::{AppError, AppResult};

/// One entry of the dialog's file-type list, e.g. `("CSV files", "*.csv")`.
#[derive(Debug, Clone, Copy)]
pub struct Filter {
    pub name: &'static str,
    pub pattern: &'static str,
}

pub const CSV: Filter = Filter {
    name: "CSV files (*.csv)",
    pattern: "*.csv",
};

pub const KDBX: Filter = Filter {
    name: "KeePass database (*.kdbx)",
    pattern: "*.kdbx",
};

pub const ALL: Filter = Filter {
    name: "All files (*.*)",
    pattern: "*.*",
};

/// What the dialog is for.
#[derive(Debug, Clone)]
pub enum Request {
    /// Pick an existing file.
    Open { filters: Vec<Filter> },
    /// Pick where to write a new file; `file_name` is proposed in Documents and
    /// `extension` (without dot) is appended when the user types a bare name.
    Save {
        file_name: String,
        extension: &'static str,
        filters: Vec<Filter>,
    },
}

/// Raw HWND of the window that owns the dialog (`0` = no owner). A plain integer so it
/// can cross to the dialog thread.
pub type Owner = isize;

/// The main window's HWND for [`show`].
pub fn owner_of(window: &tauri::WebviewWindow) -> Owner {
    #[cfg(windows)]
    {
        window.hwnd().map_or(0, |hwnd| hwnd.0 as isize)
    }
    #[cfg(not(windows))]
    {
        let _ = window;
        0
    }
}

/// Shows the dialog and blocks until the user answers: run on a blocking thread
/// (`spawn_blocking`), never on the UI / IPC thread.
pub fn show(owner: Owner, request: Request) -> AppResult<Option<PathBuf>> {
    std::thread::Builder::new()
        .name("xpass-file-dialog".into())
        .spawn(move || imp::show(owner, &request))
        .map_err(|_| AppError::DialogFailed)?
        .join()
        .map_err(|_| AppError::DialogFailed)?
}

#[cfg(windows)]
mod imp {
    use std::path::PathBuf;

    use windows::{
        core::{Interface, HRESULT, HSTRING, PCWSTR},
        Win32::{
            Foundation::{ERROR_CANCELLED, HWND},
            System::Com::{
                CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_INPROC_SERVER,
                COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE,
            },
            UI::Shell::{
                Common::COMDLG_FILTERSPEC, FOLDERID_Documents, FileOpenDialog, FileSaveDialog, IFileDialog,
                IFileOpenDialog, IFileSaveDialog, IShellItem, SHGetKnownFolderItem, FOS_DONTADDTORECENT,
                FOS_FILEMUSTEXIST, FOS_FORCEFILESYSTEM, FOS_NOCHANGEDIR, FOS_OVERWRITEPROMPT, FOS_PATHMUSTEXIST,
                KF_FLAG_DEFAULT, SIGDN_FILESYSPATH,
            },
        },
    };

    use super::{Filter, Owner, Request};
    use crate::error::{AppError, AppResult};

    /// Balances `CoInitializeEx` on this thread.
    struct Com;

    impl Drop for Com {
        fn drop(&mut self) {
            // SAFETY: only created after a successful `CoInitializeEx` on this thread.
            unsafe { CoUninitialize() };
        }
    }

    pub fn show(owner: Owner, request: &Request) -> AppResult<Option<PathBuf>> {
        // SAFETY: called once on this fresh thread; balanced by `Com::drop`.
        unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) }
            .ok()
            .map_err(|_| AppError::DialogFailed)?;
        let _com = Com;
        // SAFETY: COM is initialized on this thread; every interface below lives
        // and is released on it.
        match unsafe { run(owner, request) } {
            Ok(path) => Ok(Some(path)),
            Err(e) if e.code() == HRESULT::from_win32(ERROR_CANCELLED.0) => Ok(None),
            Err(_) => Err(AppError::DialogFailed),
        }
    }

    unsafe fn run(owner: Owner, request: &Request) -> windows::core::Result<PathBuf> {
        let (dialog, filters): (IFileDialog, &[Filter]) = match request {
            Request::Open { filters } => {
                let d: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)?;
                d.SetOptions(d.GetOptions()? | FOS_FILEMUSTEXIST | FOS_PATHMUSTEXIST)?;
                (d.cast()?, filters)
            }
            Request::Save {
                file_name,
                extension,
                filters,
            } => {
                let d: IFileSaveDialog = CoCreateInstance(&FileSaveDialog, None, CLSCTX_INPROC_SERVER)?;
                d.SetOptions(d.GetOptions()? | FOS_OVERWRITEPROMPT | FOS_PATHMUSTEXIST)?;
                d.SetFileName(&HSTRING::from(file_name.as_str()))?;
                d.SetDefaultExtension(&HSTRING::from(*extension))?;
                // Best effort: without it the dialog opens in its last folder.
                if let Ok(documents) = SHGetKnownFolderItem::<IShellItem>(&FOLDERID_Documents, KF_FLAG_DEFAULT, None) {
                    let _ = d.SetDefaultFolder(&documents);
                }
                (d.cast()?, filters)
            }
        };
        dialog.SetOptions(dialog.GetOptions()? | FOS_FORCEFILESYSTEM | FOS_NOCHANGEDIR | FOS_DONTADDTORECENT)?;

        // The HSTRINGs must outlive `SetFileTypes`.
        let strings: Vec<(HSTRING, HSTRING)> = filters
            .iter()
            .map(|f| (HSTRING::from(f.name), HSTRING::from(f.pattern)))
            .collect();
        let specs: Vec<COMDLG_FILTERSPEC> = strings
            .iter()
            .map(|(name, pattern)| COMDLG_FILTERSPEC {
                pszName: PCWSTR(name.as_ptr()),
                pszSpec: PCWSTR(pattern.as_ptr()),
            })
            .collect();
        if !specs.is_empty() {
            dialog.SetFileTypes(&specs)?;
            dialog.SetFileTypeIndex(1)?;
        }

        let owner = (owner != 0).then_some(HWND(owner as *mut core::ffi::c_void));
        dialog.Show(owner)?;
        let item = dialog.GetResult()?;
        let raw = item.GetDisplayName(SIGDN_FILESYSPATH)?;
        let path = raw.to_string();
        CoTaskMemFree(Some(raw.0 as *const core::ffi::c_void));
        Ok(PathBuf::from(
            path.map_err(|_| windows::core::Error::from(HRESULT(-1)))?,
        ))
    }
}

#[cfg(not(windows))]
mod imp {
    use std::path::PathBuf;

    use super::{Owner, Request};
    use crate::error::{AppError, AppResult};

    pub fn show(_owner: Owner, _request: &Request) -> AppResult<Option<PathBuf>> {
        Err(AppError::DialogFailed)
    }
}

#[cfg(all(test, windows))]
mod tests {
    use std::{thread, time::Duration};

    use windows_sys::Win32::{
        Foundation::{HWND, LPARAM},
        System::Threading::GetCurrentProcessId,
        UI::WindowsAndMessaging::{
            EnumWindows, GetClassNameW, GetWindowThreadProcessId, IsWindowVisible, PostMessageW, WM_CLOSE,
        },
    };

    use super::*;

    /// Visible standard dialog (`#32770`) of this test process, if any.
    fn our_dialog() -> Option<HWND> {
        unsafe extern "system" fn visit(hwnd: HWND, found: LPARAM) -> i32 {
            let mut pid = 0;
            let mut class = [0u16; 16];
            // SAFETY: `hwnd` comes from EnumWindows; the buffers outlive the calls and
            // `found` points at the `Option<HWND>` below.
            unsafe {
                GetWindowThreadProcessId(hwnd, &mut pid);
                let len = GetClassNameW(hwnd, class.as_mut_ptr(), class.len() as i32);
                if pid == GetCurrentProcessId()
                    && IsWindowVisible(hwnd) != 0
                    && String::from_utf16_lossy(&class[..len as usize]) == "#32770"
                {
                    *(found as *mut Option<HWND>) = Some(hwnd);
                    return 0;
                }
            }
            1
        }
        let mut found: Option<HWND> = None;
        // SAFETY: the callback only writes through the pointer to `found`, which lives
        // until EnumWindows returns.
        unsafe { EnumWindows(Some(visit), &mut found as *mut _ as LPARAM) };
        found
    }

    /// Opens the real dialog, closes it from another thread (only a dialog of this
    /// process), and expects "canceled". Needs a desktop session: run with
    /// `cargo test -p xpass --lib file_dialog -- --ignored`.
    fn cancel_test(request: Request) {
        // One dialog at a time: the closer cannot tell two dialogs of this process apart.
        static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _serial = SERIAL.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let closer = {
            let done = done.clone();
            thread::spawn(move || {
                let mut closed = false;
                // Keep closing until `show` returns, so a close posted too early does not
                // leave the dialog waiting for a person.
                for _ in 0..300 {
                    if done.load(std::sync::atomic::Ordering::SeqCst) {
                        break;
                    }
                    thread::sleep(Duration::from_millis(100));
                    if let Some(hwnd) = our_dialog() {
                        // SAFETY: posting WM_CLOSE to a window handle has no memory effects.
                        unsafe { PostMessageW(hwnd, WM_CLOSE, 0, 0) };
                        closed = true;
                    }
                }
                closed
            })
        };
        let result = show(0, request);
        done.store(true, std::sync::atomic::Ordering::SeqCst);
        assert!(closer.join().unwrap(), "the dialog did not appear");
        assert_eq!(result, Ok(None));
    }

    #[test]
    #[ignore = "shows a real Windows dialog"]
    fn open_dialog_cancel_returns_none() {
        cancel_test(Request::Open {
            filters: vec![CSV, ALL],
        });
    }

    #[test]
    #[ignore = "shows a real Windows dialog"]
    fn save_dialog_cancel_returns_none() {
        cancel_test(Request::Save {
            file_name: "xpass-export-test.csv".into(),
            extension: "csv",
            filters: vec![CSV],
        });
    }
}

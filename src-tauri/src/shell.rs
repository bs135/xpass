//! Opening links with the OS default browser, without the `opener`/`shell` plugins.
//! Callers must pass a URL already validated as http(s) (`entries::normalize_url`).

use url::Url;

use crate::error::{AppError, AppResult};

#[cfg(windows)]
pub fn open_url(url: &Url) -> AppResult<()> {
    use std::ptr::null;

    use windows_sys::Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL};

    debug_assert!(matches!(url.scheme(), "http" | "https"));
    let verb: Vec<u16> = "open\0".encode_utf16().collect();
    let target: Vec<u16> = url.as_str().encode_utf16().chain(Some(0)).collect();
    // SAFETY: both strings are NUL-terminated UTF-16 buffers that outlive the call;
    // the other pointer arguments may be null per the ShellExecuteW contract.
    let result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            target.as_ptr(),
            null(),
            null(),
            SW_SHOWNORMAL,
        )
    };
    // ShellExecuteW returns a value greater than 32 on success.
    if result as usize > 32 {
        Ok(())
    } else {
        Err(AppError::OpenFailed)
    }
}

#[cfg(not(windows))]
pub fn open_url(_url: &Url) -> AppResult<()> {
    Err(AppError::OpenFailed)
}

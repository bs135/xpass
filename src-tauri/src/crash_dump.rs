//! Keeps secrets out of Windows Error Reporting (v0.7 plan U9).
//!
//! A crash of an unlocked xPass would otherwise let WER write a report or a dump
//! with process memory (VaultKey, decrypted entries) to disk and maybe send it
//! to Microsoft. At startup, for the whole life of the process:
//!
//! - `WerSetFlags(WER_FAULT_REPORTING_FLAG_NOHEAP)`: reports carry no heap;
//! - `WerRegisterExcludedMemoryBlock` for every VaultKey page
//!   ([`xpass_core::secret::LockedKey`], through its page hooks);
//! - `WerAddExcludedApplication("xpass.exe", per user)`: no WER report for this
//!   executable at all. That writes a value under
//!   `HKCU\Software\Microsoft\Windows\Windows Error Reporting\ExcludedApplications`,
//!   which the NSIS uninstaller removes again (`nsis-hooks.nsh`).
//!
//! Not covered: a debugger or "Create dump file" (malware as the user, A5), an
//! administrator's dump policy that ignores the exclusion, and WebView2's own
//! crash reports (THREAT_MODEL §5). Every call is best effort: a failure is
//! logged and the app runs.

/// Applies the WER settings and installs the page hooks. Call once, early.
pub fn install() {
    #[cfg(windows)]
    imp::install();
}

/// Debug builds only: with `XPASS_CRASH_TEST=1`, crash with an access violation
/// (a real fault that reaches WER, unlike a Rust panic) once the vault has been
/// unlocked, to check by hand that no report or dump is written.
pub fn start_crash_test(app: tauri::AppHandle) {
    if !cfg!(debug_assertions) || std::env::var_os("XPASS_CRASH_TEST").is_none_or(|v| v != "1") {
        return;
    }
    use tauri::Manager;
    eprintln!("xpass: XPASS_CRASH_TEST=1, the app crashes 5 s after the vault is unlocked");
    std::thread::spawn(move || {
        let state = app.state::<crate::state::AppState>();
        while state.status() != crate::state::VaultStatus::Unlocked {
            std::thread::sleep(std::time::Duration::from_secs(1));
        }
        std::thread::sleep(std::time::Duration::from_secs(5));
        eprintln!("xpass: crash test: faulting now");
        // SAFETY: deliberately invalid; this is the crash under test.
        unsafe { std::ptr::null_mut::<u8>().write_volatile(1) };
    });
}

#[cfg(windows)]
mod imp {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;

    use windows_sys::Win32::System::ErrorReporting::{
        WerAddExcludedApplication, WerRegisterExcludedMemoryBlock, WerSetFlags, WerUnregisterExcludedMemoryBlock,
        WER_FAULT_REPORTING_FLAG_NOHEAP,
    };
    use xpass_core::secret::{set_page_hooks, PageHooks};

    pub fn install() {
        // SAFETY: plain flag call for the current process.
        let hr = unsafe { WerSetFlags(WER_FAULT_REPORTING_FLAG_NOHEAP) };
        if hr < 0 {
            eprintln!("xpass: WerSetFlags failed ({hr:#010x})");
        }

        if let Some(name) = exe_name() {
            // SAFETY: `name` is a NUL-terminated UTF-16 string that outlives the call.
            let hr = unsafe { WerAddExcludedApplication(name.as_ptr(), 0) };
            if hr < 0 {
                eprintln!("xpass: WerAddExcludedApplication failed ({hr:#010x})");
            }
        }

        set_page_hooks(PageHooks {
            on_create: exclude_block,
            on_drop: include_block,
        });
    }

    /// File name of this executable (`xpass.exe`), NUL-terminated UTF-16.
    fn exe_name() -> Option<Vec<u16>> {
        let exe = std::env::current_exe().ok()?;
        let name: &OsStr = exe.file_name()?;
        Some(name.encode_wide().chain(Some(0)).collect())
    }

    fn exclude_block(address: *const u8, len: usize) {
        let Ok(len) = u32::try_from(len) else { return };
        // SAFETY: the LockedKey page is a live allocation of `len` bytes; WER only
        // records the range. More than WER's 512 entries fails, which is harmless.
        let hr = unsafe { WerRegisterExcludedMemoryBlock(address.cast(), len) };
        if hr < 0 {
            log_block_failure_once(hr);
        }
    }

    fn include_block(address: *const u8) {
        // SAFETY: unregisters a range registered in `exclude_block` (or none: the
        // call then fails, which is ignored).
        unsafe { WerUnregisterExcludedMemoryBlock(address.cast()) };
    }

    fn log_block_failure_once(hr: i32) {
        static LOGGED: std::sync::Once = std::sync::Once::new();
        LOGGED.call_once(|| eprintln!("xpass: WerRegisterExcludedMemoryBlock failed ({hr:#010x})"));
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn exe_name_is_a_file_name() {
            let name = exe_name().unwrap();
            assert_eq!(name.last(), Some(&0));
            let text = String::from_utf16(&name[..name.len() - 1]).unwrap();
            assert!(text.ends_with(".exe") && !text.contains('\\'), "{text}");
        }

        #[test]
        fn excluding_a_page_sized_block_succeeds() {
            let block = vec![0u8; 4096];
            // SAFETY: a live allocation of 4096 bytes, unregistered before it is freed.
            let hr = unsafe { WerRegisterExcludedMemoryBlock(block.as_ptr().cast(), 4096) };
            assert!(hr >= 0, "{hr:#010x}");
            // SAFETY: registered just above.
            let hr = unsafe { WerUnregisterExcludedMemoryBlock(block.as_ptr().cast()) };
            assert!(hr >= 0, "{hr:#010x}");
        }
    }
}

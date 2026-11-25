; NSIS installer hooks (bundle.windows.nsis.installerHooks in tauri.conf.json).

; The app excludes itself from Windows Error Reporting at startup (v0.7 plan U9,
; src/crash_dump.rs), which writes this per-user value. Remove it on a real
; uninstall; an update also runs the old uninstaller (with /UPDATE) and the new
; version adds the value again on its first start, so leave it then.
!macro NSIS_HOOK_POSTUNINSTALL
  ${If} $UpdateMode <> 1
    DeleteRegValue HKCU "Software\Microsoft\Windows\Windows Error Reporting\ExcludedApplications" "${MAINBINARYNAME}.exe"
  ${EndIf}
!macroend

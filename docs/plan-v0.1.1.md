# v0.1.1 Plan: Custom title bar

Status: **in progress** (2026-11-16). Parent: [PLAN.md §4 v0.1.1](PLAN.md#v011--custom-title-bar-size-s).

## Goal

Replace the native Windows title bar with a self-drawn one that follows the app theme (light / dark / system),
without weakening the security baseline from v0.1 (strict CSP, minimal capabilities, no new plugins).

## Decisions

- The title bar **is** the existing `Header`: one row with app icon + name, drag area, app actions, window
  controls. App actions (Generator, Settings, Lock, theme, language) become icon-only buttons with tooltips.
- No `tauri-plugin-decorum`: no third-party code in the window message loop of a password manager. Accepted
  trade-off: no Windows 11 snap layouts flyout when hovering the maximize button. `Win+Z`, `Win+arrows` and
  drag-to-edge snapping still work.
- No Rust changes: the close button calls `window.close()`, which raises `CloseRequested` and goes through
  `tray.rs::on_window_event` (close-to-tray, lock-on-minimize) exactly like the native button did.

## Tasks

### T11 — Custom title bar

**Change**
- `src-tauri/tauri.conf.json`: window `main` → `decorations: false`, `shadow: true` (keeps the Windows 11 shadow,
  rounded corners and edge resizing on an undecorated window).
- `src/components/Header.tsx`: `data-tauri-drag-region="deep"` on the header (buttons opt out automatically);
  double-click on empty space toggles maximize. Icon buttons with `title` + `aria-label`; theme and language
  buttons cycle through their options and the tooltip shows the current and next value.
  The app icon is `src-tauri/icons/source.svg` (same artwork as the installer and tray); the system theme uses
  the `SunMoon` icon.
- `src/components/WindowControls.tsx`: minimize, maximize/restore (icon follows the maximized state via
  `onResized` + `isMaximized`), close (red on hover), drawn like the Windows caption buttons and kept above
  modal backdrops. `BackdropDragStrip` keeps the window draggable while a modal or confirm dialog is open.
- `src-tauri/capabilities/default.json`: add only `core:window:allow-start-dragging`,
  `allow-internal-toggle-maximize`, `allow-minimize`, `allow-toggle-maximize`, `allow-is-maximized`,
  `allow-close`.
- i18n (en, vi): `window.*` (minimize, maximize, restore, close), `titleBar.theme`, `titleBar.language`.

**Done when:** the manual checklist below passes and CI is green.

## Manual test checklist (v0.1.1)

- [ ] No native title bar; the header is the title bar and matches light, dark and system themes (also when the
      theme changes while the app runs).
- [ ] Drag the window by the header's empty space and by the app name; clicking header buttons never starts a drag.
- [ ] Double-click the header → maximize; again → restore. The maximize button icon follows (also after
      `Win+↑` / `Win+↓` and drag-to-edge snapping).
- [ ] Minimize → restore from the taskbar and from the tray.
- [ ] Close with close-to-tray on → hidden to tray (and locked when lock-on-minimize is on); close-to-tray off → app
      exits.
- [ ] With the generator modal or a confirm dialog open: the window can still be dragged and the window buttons work.
- [ ] Windows 11: shadow and rounded corners present; resize from all four edges and corners; `Win+←/→/↑` and
      `Win+Z` work.
- [ ] 125 % and 150 % display scaling: buttons aligned and crisp; maximized window does not overflow the screen
      or cover the taskbar.
- [ ] Tooltips (theme, language, Generator, Settings, Lock, window buttons) in English and Vietnamese.
- [ ] Release build: devtools, context menu and reload still disabled.

## Out of scope

Windows 11 snap layouts flyout on the maximize button, menu bar, tabs in the title bar, macOS / Linux styling.

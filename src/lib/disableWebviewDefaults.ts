/**
 * Disables browser-like WebView defaults that hurt a desktop app (from logledge):
 * - the native context menu (Back, Reload, Save as, Inspect, ...); the app's own
 *   `onContextMenu` handlers still run because only the default action is cancelled;
 * - reload shortcuts (F5, Ctrl+R, Ctrl+Shift+R), which would reset app state;
 * - Ctrl+P (print dialog of the whole window) and Ctrl+S (save page).
 */
export function disableWebviewDefaults(): void {
    window.addEventListener('contextmenu', (e) => e.preventDefault(), {capture: true})

    window.addEventListener('keydown', (e) => {
        const key = e.key.toLowerCase()
        const ctrl = e.ctrlKey || e.metaKey
        if (key === 'f5' || (ctrl && (key === 'r' || key === 'p' || key === 's'))) {
            e.preventDefault()
        }
    })
}

import {useEffect, useState} from 'react'
import {Copy, Minus, Square, X} from 'lucide-react'
import clsx from 'clsx'
import {useTranslation} from 'react-i18next'
import {getCurrentWindow} from '@tauri-apps/api/window'

/** Height of the self-drawn title bar (Tailwind `h-10`); modal backdrops keep a drag strip of this height. */
export const TITLE_BAR_HEIGHT = 'h-10'

const buttonClass =
    'flex h-full w-[46px] items-center justify-center text-neutral-600 outline-none hover:bg-neutral-200 focus-visible:bg-neutral-200 dark:text-neutral-300 dark:hover:bg-neutral-800 dark:focus-visible:bg-neutral-800'

/**
 * Minimize / maximize-restore / close, drawn like the Windows caption buttons. Close goes through
 * `CloseRequested` in `tray.rs`, so close-to-tray and lock-on-minimize still apply.
 * `relative z-[60]` keeps them usable above modal backdrops (`z-50`).
 */
export function WindowControls() {
    const {t} = useTranslation()
    const maximized = useMaximized()
    const appWindow = getCurrentWindow()
    const run = (action: () => Promise<void>, name: string) => () =>
        void action().catch((e) => console.error(`window ${name} failed`, e))

    return (
        <div className="relative z-[60] flex h-full self-stretch">
            <button
                type="button"
                onClick={run(() => appWindow.minimize(), 'minimize')}
                title={t('window.minimize')}
                aria-label={t('window.minimize')}
                className={buttonClass}
            >
                <Minus className="size-4" strokeWidth={1.5} aria-hidden/>
            </button>
            <button
                type="button"
                onClick={run(() => appWindow.toggleMaximize(), 'toggleMaximize')}
                title={t(maximized ? 'window.restore' : 'window.maximize')}
                aria-label={t(maximized ? 'window.restore' : 'window.maximize')}
                className={buttonClass}
            >
                {maximized ? (
                    <Copy className="size-3.5 -scale-x-100" strokeWidth={1.5} aria-hidden/>
                ) : (
                    <Square className="size-3.5" strokeWidth={1.5} aria-hidden/>
                )}
            </button>
            <button
                type="button"
                onClick={run(() => appWindow.close(), 'close')}
                title={t('window.close')}
                aria-label={t('window.close')}
                className={clsx(
                    buttonClass,
                    'hover:bg-red-600 hover:text-white focus-visible:bg-red-600 focus-visible:text-white dark:hover:bg-red-600 dark:hover:text-white dark:focus-visible:bg-red-600',
                )}
            >
                <X className="size-4" strokeWidth={1.5} aria-hidden/>
            </button>
        </div>
    )
}

/** Tracks the maximized state (button icon); resize events cover maximize, restore, snap and double-click. */
function useMaximized(): boolean {
    const [maximized, setMaximized] = useState(false)
    useEffect(() => {
        const appWindow = getCurrentWindow()
        let active = true
        const update = () =>
            void appWindow
                .isMaximized()
                .then((value) => active && setMaximized(value))
                .catch((e) => console.error('window isMaximized failed', e))
        update()
        const unlisten = appWindow.onResized(update)
        return () => {
            active = false
            void unlisten.then((off) => off())
        }
    }, [])
    return maximized
}

/**
 * Invisible strip over the title bar inside a modal backdrop: the window can still be dragged
 * (and maximized by double-click) while a dialog is open, as with the native title bar.
 */
export function BackdropDragStrip() {
    return (
        <div
            data-tauri-drag-region
            className={clsx('absolute inset-x-0 top-0', TITLE_BAR_HEIGHT)}
            onClick={(e) => e.stopPropagation()}
        />
    )
}

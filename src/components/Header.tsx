import {type ComponentProps, useState} from 'react'
import {Languages, Lock, Moon, Settings, Sun, SunMoon, Wand2} from 'lucide-react'
import clsx from 'clsx'
import {useTranslation} from 'react-i18next'
import {LANGUAGES} from '../i18n'
import {THEMES, type Theme} from '../lib/theme'
import {useSession} from '../stores/session'
import {Generator} from './Generator'
import {Modal} from './Modal'
import {SyncIndicator} from './SyncIndicator'
import {useUi} from '../stores/ui'
import {TITLE_BAR_HEIGHT, WindowControls} from './WindowControls'
// Same artwork as the installer / tray icon (single source in src-tauri/icons).
import appIcon from '../../src-tauri/icons/source.svg'

const THEME_ICONS: Record<Theme, typeof Sun> = {system: SunMoon, light: Sun, dark: Moon}

/** Next value in a list, wrapping around (theme and language buttons cycle through their options). */
function next<T>(values: readonly T[], current: T): T {
    return values[(values.indexOf(current) + 1) % values.length]
}

/**
 * Self-drawn title bar (the window has no native decorations): app name, app actions as icon
 * buttons, window controls. Empty space drags the window and double-click maximizes it
 * (`data-tauri-drag-region="deep"`; buttons opt out automatically).
 */
export function Header() {
    const {t} = useTranslation()
    const {theme, language, setTheme, setLanguage} = useUi()
    const screen = useSession((s) => s.screen)
    const lock = useSession((s) => s.lock)
    const go = useSession((s) => s.go)
    const unlocked = screen === 'unlocked' || screen === 'settings'
    const [generator, setGenerator] = useState(false)
    const ThemeIcon = THEME_ICONS[theme]
    const nextTheme = next(THEMES, theme)
    const nextLanguage = next(LANGUAGES, language)

    return (
        <header
            data-tauri-drag-region="deep"
            className={clsx(
                'flex shrink-0 items-center border-b border-neutral-200 bg-neutral-100 pl-3 print:hidden dark:border-neutral-800 dark:bg-neutral-900',
                TITLE_BAR_HEIGHT,
            )}
        >
            <div className="flex items-center gap-2 text-sm font-semibold">
                <img src={appIcon} alt="" className="size-4" draggable={false}/>
                {t('app.name')}
            </div>
            <div className="flex-1 self-stretch"/>
            <div className="flex items-center gap-0.5 pr-2">
                {unlocked && <SyncIndicator/>}
                {unlocked && (
                    <IconButton label={t('generator.title')} onClick={() => setGenerator(true)}>
                        <Wand2 className="size-4" aria-hidden/>
                    </IconButton>
                )}
                {screen === 'unlocked' && (
                    <IconButton label={t('settings.title')} onClick={() => go('settings')}>
                        <Settings className="size-4" aria-hidden/>
                    </IconButton>
                )}
                {unlocked && (
                    <IconButton
                        label={t('actions.lock')}
                        onClick={() => void lock().catch((e) => console.error('lock failed', e))}
                    >
                        <Lock className="size-4" aria-hidden/>
                    </IconButton>
                )}
                {unlocked && <div className="mx-1 h-4 w-px bg-neutral-300 dark:bg-neutral-700" aria-hidden/>}
                <IconButton
                    label={t('titleBar.theme', {current: t(`theme.${theme}`), next: t(`theme.${nextTheme}`)})}
                    onClick={() => setTheme(nextTheme)}
                >
                    <ThemeIcon className="size-4" aria-hidden/>
                </IconButton>
                <IconButton
                    label={t('titleBar.language', {
                        current: t(`language.${language}`),
                        next: t(`language.${nextLanguage}`),
                    })}
                    onClick={() => setLanguage(nextLanguage)}
                >
                    <Languages className="size-4" aria-hidden/>
                </IconButton>
            </div>
            <WindowControls/>
            {generator && (
                <Modal title={t('generator.title')} onClose={() => setGenerator(false)}>
                    <Generator/>
                </Modal>
            )}
        </header>
    )
}

function IconButton({label, className, children, ...rest}: ComponentProps<'button'> & {label: string}) {
    return (
        <button
            type="button"
            title={label}
            aria-label={label}
            className={clsx(
                'rounded-md p-1.5 text-neutral-600 hover:bg-neutral-200 hover:text-neutral-900 dark:text-neutral-300 dark:hover:bg-neutral-800 dark:hover:text-neutral-100',
                className,
            )}
            {...rest}
        >
            {children}
        </button>
    )
}

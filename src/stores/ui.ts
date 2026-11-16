import {create} from 'zustand'
import i18n, {isLanguage, type Language} from '../i18n'
import {api, type Settings} from '../lib/api'
import {readPref} from '../lib/prefs'
import {applyTheme, isTheme, resolveTheme, systemPrefersDark, type Theme} from '../lib/theme'

/**
 * App settings, loaded from Rust (`settings.json`) before the first render and saved
 * back through `update_settings`. Theme and language are applied here.
 */
interface UiState {
    settings: Settings
    theme: Theme
    language: Language
    setTheme: (theme: Theme) => void
    setLanguage: (language: Language | null) => void
    /** Saves a settings change; throws the command error (the store keeps the old value). */
    update: (patch: Partial<Settings>) => Promise<void>
    /** Re-reads settings (e.g. after a new Emergency Kit updates its date). */
    refresh: () => Promise<void>
}

export const DEFAULT_SETTINGS: Settings = {
    theme: 'system',
    language: null,
    autoLockMinutes: 5,
    clipboardClearSeconds: 20,
    closeToTray: true,
    lockOnMinimize: false,
    lastKitAt: null,
}

function osLanguage(): Language {
    return navigator.language.toLowerCase().startsWith('vi') ? 'vi' : 'en'
}

function applyLanguage(language: Language): void {
    void i18n.changeLanguage(language)
    document.documentElement.lang = language
}

/**
 * Loads settings before the app renders. Theme and language chosen in earlier builds
 * (localStorage) are moved into settings.json once.
 */
export async function loadSettings(): Promise<Settings> {
    let settings: Settings
    try {
        settings = await api.getSettings()
    } catch (e) {
        console.error('get_settings failed', e)
        return DEFAULT_SETTINGS
    }
    const oldTheme = readPref('theme')
    const oldLanguage = readPref('language')
    // Each value moves over only while settings.json still has its default.
    const theme = settings.theme === 'system' && isTheme(oldTheme) ? oldTheme : settings.theme
    const language = settings.language === null && isLanguage(oldLanguage) ? oldLanguage : settings.language
    if (theme !== settings.theme || language !== settings.language) {
        try {
            settings = await api.updateSettings({...settings, theme, language})
        } catch (e) {
            console.error('settings migration failed', e)
        }
    }
    try {
        window.localStorage.removeItem('xpass.theme')
        window.localStorage.removeItem('xpass.language')
    } catch {
        // Storage may be unavailable; nothing to clean up then.
    }
    return settings
}

export function initialState(settings: Settings): Pick<UiState, 'settings' | 'theme' | 'language'> {
    return {settings, theme: settings.theme, language: settings.language ?? osLanguage()}
}

export const useUi = create<UiState>((set, get) => ({
    ...initialState(DEFAULT_SETTINGS),
    setTheme: (theme) => {
        applyTheme(resolveTheme(theme, systemPrefersDark()))
        set({theme})
        get().update({theme}).catch((e) => console.error('saving theme failed', e))
    },
    setLanguage: (language) => {
        const effective = language ?? osLanguage()
        applyLanguage(effective)
        set({language: effective})
        get().update({language}).catch((e) => console.error('saving language failed', e))
    },
    update: async (patch) => {
        const saved = await api.updateSettings({...get().settings, ...patch})
        set({settings: saved})
    },
    refresh: async () => {
        set({settings: await api.getSettings()})
    },
}))

import {create} from 'zustand'
import i18n, {isLanguage, type Language} from '../i18n'
import {readPref, writePref} from '../lib/prefs'
import {applyTheme, isTheme, resolveTheme, systemPrefersDark, type Theme} from '../lib/theme'

interface UiState {
    theme: Theme
    language: Language
    setTheme: (theme: Theme) => void
    setLanguage: (language: Language) => void
}

export function initialTheme(): Theme {
    const saved = readPref('theme')
    return isTheme(saved) ? saved : 'system'
}

export function initialLanguage(): Language {
    const saved = readPref('language')
    if (isLanguage(saved)) return saved
    return navigator.language.toLowerCase().startsWith('vi') ? 'vi' : 'en'
}

export const useUi = create<UiState>((set) => ({
    theme: initialTheme(),
    language: initialLanguage(),
    setTheme: (theme) => {
        writePref('theme', theme)
        applyTheme(resolveTheme(theme, systemPrefersDark()))
        set({theme})
    },
    setLanguage: (language) => {
        writePref('language', language)
        void i18n.changeLanguage(language)
        document.documentElement.lang = language
        set({language})
    },
}))

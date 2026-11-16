import i18n from 'i18next'
import {initReactI18next} from 'react-i18next'
import en from './locales/en.json'
import vi from './locales/vi.json'

export type Language = 'en' | 'vi'
export const LANGUAGES: readonly Language[] = ['en', 'vi']

export function isLanguage(value: unknown): value is Language {
    return typeof value === 'string' && (LANGUAGES as readonly string[]).includes(value)
}

export function initI18n(language: Language): void {
    void i18n.use(initReactI18next).init({
        resources: {en: {translation: en}, vi: {translation: vi}},
        lng: language,
        fallbackLng: 'en',
        interpolation: {escapeValue: false}, // React already escapes
    })
}

export default i18n

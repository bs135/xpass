import {KeyRound, Monitor, Moon, Sun} from 'lucide-react'
import clsx from 'clsx'
import {useTranslation} from 'react-i18next'
import {LANGUAGES} from '../i18n'
import {THEMES, type Theme} from '../lib/theme'
import {useUi} from '../stores/ui'

const THEME_ICONS: Record<Theme, typeof Sun> = {system: Monitor, light: Sun, dark: Moon}

export function Header() {
    const {t} = useTranslation()
    const {theme, language, setTheme, setLanguage} = useUi()

    return (
        <header className="flex items-center justify-between border-b border-neutral-200 px-4 py-2 dark:border-neutral-800">
            <div className="flex items-center gap-2 font-semibold">
                <KeyRound className="size-5 text-emerald-600 dark:text-emerald-400" aria-hidden/>
                {t('app.name')}
            </div>
            <div className="flex items-center gap-3">
                <div role="radiogroup" aria-label={t('settings.theme')} className="flex rounded-md border border-neutral-200 p-0.5 dark:border-neutral-700">
                    {THEMES.map((value) => {
                        const Icon = THEME_ICONS[value]
                        return (
                            <button
                                key={value}
                                type="button"
                                role="radio"
                                aria-checked={theme === value}
                                title={t(`theme.${value}`)}
                                onClick={() => setTheme(value)}
                                className={clsx(
                                    'rounded p-1.5',
                                    theme === value
                                        ? 'bg-neutral-200 dark:bg-neutral-700'
                                        : 'text-neutral-500 hover:bg-neutral-100 dark:hover:bg-neutral-800',
                                )}
                            >
                                <Icon className="size-4" aria-hidden/>
                            </button>
                        )
                    })}
                </div>
                <select
                    aria-label={t('settings.language')}
                    value={language}
                    onChange={(e) => setLanguage(e.target.value as typeof language)}
                    className="rounded-md border border-neutral-200 bg-transparent px-2 py-1 text-sm dark:border-neutral-700"
                >
                    {LANGUAGES.map((lang) => (
                        <option key={lang} value={lang} className="text-neutral-900">
                            {t(`language.${lang}`)}
                        </option>
                    ))}
                </select>
            </div>
        </header>
    )
}

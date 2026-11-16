import {useEffect, useState} from 'react'
import {ShieldCheck} from 'lucide-react'
import {useTranslation} from 'react-i18next'
import {getCurrentWindow} from '@tauri-apps/api/window'
import {Header} from './components/Header'
import {api, type AppInfo} from './lib/api'
import {applyTheme, resolveTheme} from './lib/theme'
import {useUi} from './stores/ui'

export default function App() {
    const {t} = useTranslation()
    const theme = useUi((s) => s.theme)
    const [info, setInfo] = useState<AppInfo | null>(null)

    // Follow OS theme changes while the "system" theme is selected.
    useEffect(() => {
        const media = window.matchMedia('(prefers-color-scheme: dark)')
        const update = () => applyTheme(resolveTheme(theme, media.matches))
        update()
        media.addEventListener('change', update)
        return () => media.removeEventListener('change', update)
    }, [theme])

    // The window starts hidden (tauri.conf.json) to avoid a white flash; show it once rendered.
    useEffect(() => {
        void getCurrentWindow().show()
        api.appInfo().then(setInfo).catch((e) => console.error('app_info failed', e))
    }, [])

    return (
        <div className="flex h-full flex-col">
            <Header/>
            <main className="flex flex-1 flex-col items-center justify-center gap-3 p-8 text-center">
                <ShieldCheck className="size-12 text-emerald-600 dark:text-emerald-400" aria-hidden/>
                <h1 className="text-xl font-semibold">{t('placeholder.title')}</h1>
                <p className="max-w-md text-neutral-500 dark:text-neutral-400">{t('placeholder.body')}</p>
            </main>
            <footer className="px-4 py-2 text-xs text-neutral-500 dark:text-neutral-500">
                {info && t('about.version', {version: info.version, format: info.formatVersion})}
            </footer>
        </div>
    )
}

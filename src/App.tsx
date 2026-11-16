import {useEffect, useState} from 'react'
import {useTranslation} from 'react-i18next'
import {listen} from '@tauri-apps/api/event'
import {getCurrentWindow} from '@tauri-apps/api/window'
import {Header} from './components/Header'
import {Toast} from './components/Toast'
import {api, type AppInfo, EVENT_LOCKED} from './lib/api'
import {applyTheme, resolveTheme} from './lib/theme'
import {EmergencyKitScreen} from './screens/EmergencyKitScreen'
import {EntriesScreen} from './screens/entries/EntriesScreen'
import {NewPasswordScreen} from './screens/NewPasswordScreen'
import {SetupScreen} from './screens/SetupScreen'
import {UnlockScreen} from './screens/UnlockScreen'
import {type Screen, useSession} from './stores/session'
import {useUi} from './stores/ui'

export default function App() {
    const {t} = useTranslation()
    const theme = useUi((s) => s.theme)
    const screen = useSession((s) => s.screen)
    const [info, setInfo] = useState<AppInfo | null>(null)

    // Follow OS theme changes while the "system" theme is selected.
    useEffect(() => {
        const media = window.matchMedia('(prefers-color-scheme: dark)')
        const update = () => applyTheme(resolveTheme(theme, media.matches))
        update()
        media.addEventListener('change', update)
        return () => media.removeEventListener('change', update)
    }, [theme])

    useEffect(() => {
        const {init, onLocked} = useSession.getState()
        // The window starts hidden (tauri.conf.json) to avoid a white flash; show it once the first screen is known.
        init()
            .catch((e) => console.error('vault_status failed', e))
            .finally(() => void getCurrentWindow().show())
        api.appInfo().then(setInfo).catch((e) => console.error('app_info failed', e))
        const unlisten = listen(EVENT_LOCKED, onLocked)
        return () => void unlisten.then((off) => off())
    }, [])

    return (
        <div className="flex h-full flex-col">
            <Header/>
            <ScreenView screen={screen}/>
            <Toast/>
            <footer className="px-4 py-2 text-xs text-neutral-500 print:hidden">
                {info && t('about.version', {version: info.version, format: info.formatVersion})}
            </footer>
        </div>
    )
}

function ScreenView({screen}: {screen: Screen}) {
    switch (screen) {
        case 'loading':
            return <div className="flex-1"/>
        case 'setup':
            return <SetupScreen/>
        case 'kit':
            return <EmergencyKitScreen/>
        case 'unlock':
            // `key` remounts the form so a typed password never carries over.
            return <UnlockScreen key="password" mode="password"/>
        case 'recovery':
            return <UnlockScreen key="recovery" mode="recovery"/>
        case 'newPassword':
            return <NewPasswordScreen/>
        case 'unlocked':
            return <EntriesScreen/>
    }
}

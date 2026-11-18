import {useEffect, useState} from 'react'
import {useTranslation} from 'react-i18next'
import {listen} from '@tauri-apps/api/event'
import {getCurrentWindow} from '@tauri-apps/api/window'
import {Header} from './components/Header'
import {Toast} from './components/Toast'
import {
    api,
    type AppInfo,
    EVENT_LOCKED,
    EVENT_SYNC_STATUS,
    EVENT_VAULT_CHANGED,
    type LockReason,
    type SyncStatus,
} from './lib/api'
import {applyTheme, resolveTheme} from './lib/theme'
import {EmergencyKitScreen} from './screens/EmergencyKitScreen'
import {EntriesScreen} from './screens/entries/EntriesScreen'
import {NewPasswordScreen} from './screens/NewPasswordScreen'
import {SettingsScreen} from './screens/settings/SettingsScreen'
import {SetupScreen} from './screens/SetupScreen'
import {UnlockScreen} from './screens/UnlockScreen'
import {useEntries} from './stores/entries'
import {type Screen, useSession} from './stores/session'
import {useSync} from './stores/sync'
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
        const unlisten = listen<{reason: LockReason}>(EVENT_LOCKED, (e) => onLocked(e.payload?.reason))
        const unlistenSync = listen<SyncStatus>(EVENT_SYNC_STATUS, (e) => useSync.getState().set(e.payload))
        const unlistenChanged = listen(EVENT_VAULT_CHANGED, () => {
            if (useSession.getState().screen === 'unlocked') {
                useEntries.getState().refresh().catch((e) => console.error('reload after sync failed', e))
            }
        })
        useSync.getState().refresh()
        // Events can be lost while the WebView is frozen (sleep): re-check when shown again.
        const resync = () => {
            if (document.visibilityState === 'visible') {
                useSession.getState().sync().catch((e) => console.error('vault_status failed', e))
                useSync.getState().refresh()
            }
        }
        document.addEventListener('visibilitychange', resync)
        window.addEventListener('focus', resync)
        return () => {
            void unlisten.then((off) => off())
            void unlistenSync.then((off) => off())
            void unlistenChanged.then((off) => off())
            document.removeEventListener('visibilitychange', resync)
            window.removeEventListener('focus', resync)
        }
    }, [])

    return (
        <div className="flex h-full flex-col">
            <Header/>
            <ScreenView screen={screen}/>
            <Toast/>
            <footer className="shrink-0 border-t border-neutral-200 px-4 py-2 text-xs text-neutral-500 print:hidden dark:border-neutral-800">
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
        case 'settings':
            return <SettingsScreen/>
    }
}

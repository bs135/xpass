import {AlertTriangle, CloudCheck, CloudOff, CloudUpload, RefreshCw} from 'lucide-react'
import clsx from 'clsx'
import {useTranslation} from 'react-i18next'
import {errorMessage} from '../lib/errors'
import {syncText, syncView, type SyncView} from '../lib/syncStatus'
import {useNow} from '../lib/useNow'
import {useSession} from '../stores/session'
import {useSync} from '../stores/sync'
import {useToast} from '../stores/toast'

const ICONS: Record<SyncView['key'], typeof CloudCheck> = {
    syncing: RefreshCw,
    synced: CloudCheck,
    pending: CloudUpload,
    offline: CloudOff,
    tokenInvalid: AlertTriangle,
    needsPassword: AlertTriangle,
    differentVault: AlertTriangle,
    error: AlertTriangle,
}

/** Header button: sync state icon; click syncs now, or opens Settings for problems only the user can fix. */
export function SyncIndicator() {
    const {t} = useTranslation()
    const status = useSync((s) => s.status)
    const syncNow = useSync((s) => s.syncNow)
    const go = useSession((s) => s.go)
    const show = useToast((s) => s.show)
    const now = useNow()
    const view = syncView(status)
    if (!view) return null
    const Icon = ICONS[view.key]
    const label = syncText(t, status, now)

    const onClick = () => {
        if (view.action === 'openSettings') go('settings')
        else if (status.phase !== 'syncing') syncNow().catch((e) => show(errorMessage(t, e), 4000))
    }

    return (
        <button
            type="button"
            title={label}
            aria-label={label}
            onClick={onClick}
            className={clsx(
                'rounded-md p-1.5 hover:bg-neutral-200 dark:hover:bg-neutral-800',
                view.warn ? 'text-amber-600 dark:text-amber-400' : 'text-neutral-600 dark:text-neutral-300',
            )}
        >
            <Icon className={clsx('size-4', view.key === 'syncing' && 'animate-spin')} aria-hidden/>
        </button>
    )
}
